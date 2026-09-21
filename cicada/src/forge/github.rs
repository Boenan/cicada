//! The GitHub adapter — REST v3, plus one GraphQL mutation it cannot avoid.
//!
//! GHE is reached by pointing `forge.api_url` at `https://host/api/v3`.
//!
//! ## Drafting is not a REST field, and #54 is optimistic about it
//!
//! The design says GitHub drafting is "a real API field (`draft: true`)". That is
//! true when a pull request is **created** and false afterwards: `PATCH
//! /repos/{o}/{r}/pulls/{n}` accepts `title`, `body`, `state`, `base` and
//! `maintainer_can_modify`, and nothing else. Converting an existing PR to draft
//! is `convertPullRequestToDraft`, a **GraphQL** mutation, and coming back is
//! `markPullRequestReadyForReview`.
//!
//! That matters because staleness layer 2 (#33) drafts a PR that already exists —
//! which is precisely the case REST cannot express. The alternative #54 offers
//! (title prefix plus label plus a failing status) is the fallback for forges
//! where drafting is genuinely unavailable; on GitHub it *is* available, so using
//! the fallback here would be a visible degradation on the main supported forge.
//! Hence one GraphQL call, addressed by the node id every REST response already
//! carries.

use serde_json::{Value, json};

use super::{ForgeError, Forge, PrRequest, PullRequest, Release, ReleaseRequest, backoff, is_retryable};

pub struct GitHub {
    api: String,
    repo: String,
    token: String,
    agent: ureq::Agent,
}

/// Enough attempts to ride out a deploy or a rate-limit window, few enough that a
/// genuinely broken forge fails the job rather than hanging it.
const MAX_ATTEMPTS: u32 = 4;

impl GitHub {
    pub fn new(api_url: &str, repo: &str, token: String) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // cicada reads statuses itself: a 404 from `find_pull_request` is an
            // answer, not an error, and the retry policy needs the code anyway.
            .http_status_as_error(false)
            .user_agent(concat!("cicada/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            api: api_url.trim_end_matches('/').to_string(),
            repo: repo.trim_matches('/').to_string(),
            token,
            agent,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/repos/{}{}", self.api, self.repo, path)
    }

    /// The headers every request carries.
    ///
    /// Generic over ureq's body typestate because `GET` and `POST` produce
    /// different builder types — `RequestBuilder<WithoutBody>` and
    /// `RequestBuilder<WithBody>` — and there is no uniform `request(method, url)`
    /// to hang them off.
    fn auth<B>(&self, rb: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        rb.header("Authorization", &format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// One request, with the retry policy applied (#58).
    fn send(
        &self,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> Result<(u16, String), ForgeError> {
        let mut attempt = 0u32;
        loop {
            let json = body.as_ref().map(ToString::to_string).unwrap_or_default();
            let sent = match method {
                // `call()` is the no-body send; `send()` is the with-body one.
                "GET" => self.auth(self.agent.get(url)).call(),
                "POST" => self
                    .auth(self.agent.post(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                "PATCH" => self
                    .auth(self.agent.patch(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                "PUT" => self
                    .auth(self.agent.put(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                other => {
                    return Err(ForgeError::Decode {
                        context: "github",
                        detail: format!("unsupported method {other}"),
                    });
                }
            };

            let mut resp = match sent {
                Ok(r) => r,
                Err(e) => {
                    return Err(ForgeError::Network { url: url.to_string(), source: Box::new(e) });
                }
            };

            let status = resp.status().as_u16();
            let retry_after =
                resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
            let text = resp.body_mut().read_to_string().unwrap_or_default();

            if is_retryable(status) && attempt + 1 < MAX_ATTEMPTS {
                std::thread::sleep(backoff(attempt, retry_after.as_deref()));
                attempt += 1;
                continue;
            }
            return Ok((status, text));
        }
    }

    /// Send, and turn anything that is not a success into an actionable error.
    fn ok(&self, method: &str, url: &str, body: Option<Value>) -> Result<Value, ForgeError> {
        let (status, text) = self.send(method, url, body)?;
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text)
                .map_err(|e| ForgeError::Decode { context: "github", detail: e.to_string() });
        }
        Err(map_error(status, &text, &self.repo))
    }

    fn graphql(&self, query: &str, variables: Value) -> Result<Value, ForgeError> {
        let url = format!("{}/graphql", self.api);
        let (status, text) =
            self.send("POST", &url, Some(json!({ "query": query, "variables": variables })))?;
        if !(200..300).contains(&status) {
            return Err(map_error(status, &text, &self.repo));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| ForgeError::Decode { context: "github graphql", detail: e.to_string() })?;
        // GraphQL answers 200 with an `errors` array, so a failure here is silent
        // unless it is looked for.
        if let Some(errs) = v.get("errors").and_then(|e| e.as_array())
            && !errs.is_empty()
        {
            let msg = errs
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ForgeError::Http { status: 200, message: format!("GitHub GraphQL: {msg}") });
        }
        Ok(v)
    }
}

/// Map a status to text that names the fix, not the code (#58).
fn map_error(status: u16, body: &str, repo: &str) -> ForgeError {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect());

    let message = match status {
        401 => format!(
            "GitHub rejected the token (401 {detail})\n\
             it is invalid, expired, or not a token for this host"
        ),
        403 => format!(
            "GitHub refused the request (403 {detail})\n\
             the token needs `contents: write` and `pull-requests: write` on {repo}, and the repo \
             needs\n*Allow GitHub Actions to create and approve pull requests* enabled"
        ),
        404 => format!(
            "GitHub has no {repo}, or this token cannot see it (404)\n\
             a private repo returns 404 rather than 403 when the token lacks access, so check the \
             scope before the spelling"
        ),
        422 => format!(
            "GitHub rejected the request as invalid (422 {detail})\n\
             the usual cause is a head branch that was never pushed, or a base branch that does \
             not exist"
        ),
        429 => format!("GitHub rate-limited the request and it did not recover (429 {detail})"),
        s if (500..600).contains(&s) => {
            format!("GitHub returned {s} and did not recover after {MAX_ATTEMPTS} attempts ({detail})")
        }
        s => format!("GitHub returned {s}: {detail}"),
    };
    ForgeError::Http { status, message }
}

fn pr_from(v: &Value) -> PullRequest {
    PullRequest {
        number: v.get("number").and_then(Value::as_u64).unwrap_or_default(),
        url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        draft: v.get("draft").and_then(Value::as_bool).unwrap_or(false),
        node_id: v.get("node_id").and_then(Value::as_str).map(str::to_string),
    }
}

impl Forge for GitHub {
    fn find_pull_request(&self, head: &str) -> Result<Option<PullRequest>, ForgeError> {
        // `state=open` is the whole of #77: a closed release PR is never reopened,
        // and idempotency rests on the head branch rather than on PR identity.
        // The `owner:branch` qualifier is required by the API even for same-repo
        // branches.
        let owner = self.repo.split('/').next().unwrap_or_default();
        let url = self.url(&format!(
            "/pulls?state=open&head={}",
            urlencode(&format!("{owner}:{head}"))
        ));
        let v = self.ok("GET", &url, None)?;
        Ok(v.as_array().and_then(|a| a.first()).map(pr_from))
    }

    fn create_pull_request(&self, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        let v = self.ok(
            "POST",
            &self.url("/pulls"),
            Some(json!({
                "head": req.head,
                "base": req.base,
                "title": req.title,
                "body": req.body,
            })),
        )?;
        Ok(pr_from(&v))
    }

    fn update_pull_request(&self, number: u64, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        // Deliberately not sending `head`: GitHub will not move it, and the head
        // branch is how this PR was found in the first place.
        let v = self.ok(
            "PATCH",
            &self.url(&format!("/pulls/{number}")),
            Some(json!({ "title": req.title, "body": req.body, "base": req.base })),
        )?;
        Ok(pr_from(&v))
    }

    fn close_pull_request(&self, number: u64) -> Result<(), ForgeError> {
        self.ok("PATCH", &self.url(&format!("/pulls/{number}")), Some(json!({ "state": "closed" })))?;
        Ok(())
    }

    fn set_draft(&self, pr: &PullRequest, draft: bool) -> Result<(), ForgeError> {
        let Some(id) = &pr.node_id else {
            return Err(ForgeError::Http {
                status: 0,
                message: format!(
                    "cannot change the draft state of #{}: GitHub exposes it only through GraphQL, \
                     and this pull request came back without a node id",
                    pr.number
                ),
            });
        };
        let (mutation, field) = if draft {
            ("convertPullRequestToDraft", "pullRequestId")
        } else {
            ("markPullRequestReadyForReview", "pullRequestId")
        };
        let query = format!(
            "mutation($id: ID!) {{ {mutation}(input: {{ {field}: $id }}) \
             {{ pullRequest {{ isDraft }} }} }}"
        );
        self.graphql(&query, json!({ "id": id }))?;
        Ok(())
    }

    fn comment(&self, number: u64, body: &str) -> Result<(), ForgeError> {
        // Comments live on the *issue* resource; a PR is an issue with a branch.
        self.ok(
            "POST",
            &self.url(&format!("/issues/{number}/comments")),
            Some(json!({ "body": body })),
        )?;
        Ok(())
    }

    fn set_labels(&self, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        self.ok(
            "PUT",
            &self.url(&format!("/issues/{number}/labels")),
            Some(json!({ "labels": labels })),
        )?;
        Ok(())
    }

    fn find_release(&self, tag: &str) -> Result<Option<Release>, ForgeError> {
        let url = self.url(&format!("/releases/tags/{}", urlencode(tag)));
        let (status, text) = self.send("GET", &url, None)?;
        if status == 404 {
            // Absent is an answer, not a failure — Phase B reconciles the release
            // object independently of the tag (#56).
            return Ok(None);
        }
        if !(200..300).contains(&status) {
            return Err(map_error(status, &text, &self.repo));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| ForgeError::Decode { context: "github release", detail: e.to_string() })?;
        Ok(Some(Release {
            id: v.get("id").and_then(Value::as_u64).unwrap_or_default(),
            url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
            tag: tag.to_string(),
        }))
    }

    fn create_release(&self, req: &ReleaseRequest) -> Result<Release, ForgeError> {
        let v = self.ok(
            "POST",
            &self.url("/releases"),
            Some(json!({
                "tag_name": req.tag,
                "target_commitish": req.target,
                "name": req.name,
                "body": req.body,
                "prerelease": req.prerelease,
            })),
        )?;
        Ok(Release {
            id: v.get("id").and_then(Value::as_u64).unwrap_or_default(),
            url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
            tag: req.tag.clone(),
        })
    }
}

/// Percent-encode a path or query segment.
///
/// Hand-rolled rather than a dependency: the inputs are branch names, tags and
/// `org/repo` slugs, and the set of bytes that need escaping in them is small and
/// unchanging. GitLab subgroups make this matter more there than here (M6).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_the_fix_rather_than_the_status_code() {
        let e = map_error(403, r#"{"message":"Resource not accessible by integration"}"#, "acme/app");
        let s = e.to_string();
        assert!(s.contains("pull-requests: write"), "{s}");
        assert!(s.contains("acme/app"), "{s}");

        // A private repo answers 404 when the token cannot see it, which sends
        // people hunting for a typo that is not there.
        let s = map_error(404, "{}", "acme/app").to_string();
        assert!(s.contains("scope before the spelling"), "{s}");

        // A body that is not JSON still produces something readable.
        let s = map_error(500, "<html>502 Bad Gateway</html>", "acme/app").to_string();
        assert!(s.contains("500"), "{s}");
    }

    #[test]
    fn a_pull_request_carries_the_node_id_drafting_needs() {
        let v = json!({"number": 7, "html_url": "u", "draft": false, "node_id": "PR_abc"});
        assert_eq!(
            pr_from(&v),
            PullRequest {
                number: 7,
                url: "u".into(),
                draft: false,
                node_id: Some("PR_abc".into())
            }
        );
    }

    #[test]
    fn encoding_covers_the_characters_a_branch_or_tag_actually_contains() {
        assert_eq!(urlencode("cicada/release/1.4"), "cicada%2Frelease%2F1.4");
        assert_eq!(urlencode("api-v1.5.0"), "api-v1.5.0");
        assert_eq!(urlencode("acme:cicada/main"), "acme%3Acicada%2Fmain");
        assert_eq!(urlencode("v1.0.0+build.1"), "v1.0.0%2Bbuild.1");
    }
}

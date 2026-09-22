//! The GitLab adapter — REST v4.
//!
//! Self-hosted instances are reached by pointing `forge.api_url` at
//! `https://host/api/v4`. A project is addressed by its **URL-encoded full path**
//! (`group%2Fsub%2Fproj`), so a subgroup needs nothing special — it is one more
//! slash that the encoder already handles.
//!
//! ## Two asymmetries with GitHub, both from #54
//!
//! **Draft state is the title.** GitLab has no draft flag: an MR is a draft
//! because its title starts with `Draft: `, and un-drafting is removing that
//! prefix. So `set_draft` is a title edit, and the adapter is careful to strip
//! before it prefixes — otherwise a second refusal produces `Draft: Draft: …`.
//! This is not a degradation: GitLab genuinely refuses to merge a draft MR, so
//! staleness layer 2 is as enforced here as it is on GitHub.
//!
//! **Releases have no prerelease flag.** GitHub carries `prerelease: true`;
//! GitLab has nothing equivalent, so the adapter marks it in the release **name**
//! instead. A tester reading the releases list can still tell, which is the point
//! of the flag.
//!
//! The token goes in `PRIVATE-TOKEN`, the header GitLab documents for personal,
//! project and group access tokens. Cicada wants a **Project Access Token with
//! Maintainer role and the `api` + `write_repository` scopes** (§Permissions).

use serde_json::{Value, json};

use super::http::{Client, MAX_ATTEMPTS, api_message, urlencode};
use super::{Forge, ForgeError, PrRequest, PullRequest, Release, ReleaseRequest};

/// The prefix that *is* GitLab's draft state (#54).
const DRAFT_PREFIX: &str = "Draft: ";

pub struct GitLab {
    api: String,
    project: String,
    repo: String,
    http: Client,
}

impl GitLab {
    pub fn new(api_url: &str, repo: &str, token: String) -> Self {
        let repo = repo.trim_matches('/').to_string();
        Self {
            api: api_url.trim_end_matches('/').to_string(),
            project: urlencode(&repo),
            repo,
            http: Client::new(vec![("PRIVATE-TOKEN".into(), token)]),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/projects/{}{}", self.api, self.project, path)
    }

    fn ok(&self, method: &str, url: &str, body: Option<Value>) -> Result<Value, ForgeError> {
        let (status, text) = self.http.send(method, url, body.as_ref())?;
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text)
                .map_err(|e| ForgeError::Decode { context: "gitlab", detail: e.to_string() });
        }
        Err(map_error(status, &text, &self.repo))
    }

    /// One PUT against an MR, which is how GitLab expresses nearly every change
    /// to one — including closing it and toggling its draft state.
    fn edit_mr(&self, iid: u64, body: Value) -> Result<Value, ForgeError> {
        self.ok("PUT", &self.url(&format!("/merge_requests/{iid}")), Some(body))
    }
}

/// Map a status to text that names the fix, not the code (#58).
fn map_error(status: u16, body: &str, repo: &str) -> ForgeError {
    let detail = api_message(body);
    let message = match status {
        401 => format!(
            "GitLab rejected the token (401 {detail})\n\
             it is invalid, expired, or revoked"
        ),
        403 => format!(
            "GitLab refused the request (403 {detail})\n\
             the token needs the `api` and `write_repository` scopes and at least Maintainer role \
             on {repo}"
        ),
        404 => format!(
            "GitLab has no project {repo}, or this token cannot see it (404)\n\
             `repo` is the project's full path including any subgroups, e.g. `group/sub/proj` — \
             and a\nprivate project returns 404 rather than 403 when the token lacks access, so \
             check the scope\nbefore the spelling"
        ),
        409 => format!(
            "GitLab reported a conflict (409 {detail})\n\
             the usual cause is a merge request that already exists for this source branch"
        ),
        422 => format!(
            "GitLab rejected the request as invalid (422 {detail})\n\
             the usual cause is a source branch that was never pushed, or a target branch that \
             does not exist"
        ),
        429 => format!("GitLab rate-limited the request and it did not recover (429 {detail})"),
        s if (500..600).contains(&s) => {
            format!("GitLab returned {s} and did not recover after {MAX_ATTEMPTS} attempts ({detail})")
        }
        s => format!("GitLab returned {s}: {detail}"),
    };
    ForgeError::Http { status, message }
}

/// Strip the draft prefix, however it was spelled.
///
/// GitLab has used `WIP: ` historically and `Draft: ` since 13.2, and both are
/// still recognised on existing merge requests. Stripping only the modern one
/// would leave a `WIP: ` MR permanently undraftable-looking.
fn strip_draft(title: &str) -> &str {
    for p in ["Draft: ", "draft: ", "WIP: ", "wip: "] {
        if let Some(rest) = title.strip_prefix(p) {
            return rest.trim_start();
        }
    }
    title
}

fn is_draft(v: &Value) -> bool {
    // The modern field, the legacy field, then the title — in that order, because
    // an old instance may report neither and the title is always authoritative.
    v.get("draft")
        .and_then(Value::as_bool)
        .or_else(|| v.get("work_in_progress").and_then(Value::as_bool))
        .unwrap_or_else(|| {
            let t = v.get("title").and_then(Value::as_str).unwrap_or_default();
            strip_draft(t) != t
        })
}

fn mr_from(v: &Value) -> PullRequest {
    PullRequest {
        // `iid` is the per-project number a human sees; `id` is global and is not
        // what any other endpoint wants.
        number: v.get("iid").and_then(Value::as_u64).unwrap_or_default(),
        url: v.get("web_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        draft: is_draft(v),
        title: v.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        // GitHub-only; GitLab addresses merge requests by iid throughout.
        node_id: None,
    }
}

impl Forge for GitLab {
    fn find_pull_request(&self, head: &str) -> Result<Option<PullRequest>, ForgeError> {
        // `state=opened` is the whole of #77: a closed release MR is never
        // reopened, and idempotency rests on the source branch rather than on MR
        // identity.
        let url = self.url(&format!(
            "/merge_requests?state=opened&source_branch={}",
            urlencode(head)
        ));
        let v = self.ok("GET", &url, None)?;
        Ok(v.as_array().and_then(|a| a.first()).map(mr_from))
    }

    fn create_pull_request(&self, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        let v = self.ok(
            "POST",
            &self.url("/merge_requests"),
            Some(json!({
                "source_branch": req.head,
                "target_branch": req.base,
                "title": req.title,
                "description": req.body,
            })),
        )?;
        Ok(mr_from(&v))
    }

    fn update_pull_request(&self, number: u64, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        // Deliberately not sending `source_branch`: GitLab will not move it, and
        // it is how this MR was found in the first place.
        //
        // Sending the plain title also un-drafts an MR that was drafted by a
        // previous refusal — which is the recovery `flow::upsert` wants (D3), and
        // why the `set_draft(false)` it then makes is a confirming no-op here
        // rather than the operative call.
        let v = self.edit_mr(
            number,
            json!({ "title": req.title, "description": req.body, "target_branch": req.base }),
        )?;
        Ok(mr_from(&v))
    }

    fn close_pull_request(&self, number: u64) -> Result<(), ForgeError> {
        self.edit_mr(number, json!({ "state_event": "close" }))?;
        Ok(())
    }

    fn set_draft(&self, pr: &PullRequest, draft: bool) -> Result<(), ForgeError> {
        // Strip first, always. Without it a second refusal produces
        // `Draft: Draft: chore(release): …`, which is both ugly and, after
        // enough of them, longer than GitLab's title limit.
        let base = strip_draft(&pr.title);
        let title = if draft { format!("{DRAFT_PREFIX}{base}") } else { base.to_string() };
        self.edit_mr(pr.number, json!({ "title": title }))?;
        Ok(())
    }

    fn comment(&self, number: u64, body: &str) -> Result<(), ForgeError> {
        self.ok(
            "POST",
            &self.url(&format!("/merge_requests/{number}/notes")),
            Some(json!({ "body": body })),
        )?;
        Ok(())
    }

    fn set_labels(&self, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        // `labels` replaces the whole set, which is what cicada means: the empty
        // list clears `cicada:stale` on recovery.
        self.edit_mr(number, json!({ "labels": labels.join(",") }))?;
        Ok(())
    }

    fn find_release(&self, tag: &str) -> Result<Option<Release>, ForgeError> {
        let url = self.url(&format!("/releases/{}", urlencode(tag)));
        let (status, text) = self.http.send("GET", &url, None)?;
        if status == 404 {
            // Absent is an answer, not a failure — Phase B reconciles the release
            // object independently of the tag (#56).
            return Ok(None);
        }
        if !(200..300).contains(&status) {
            return Err(map_error(status, &text, &self.repo));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| ForgeError::Decode { context: "gitlab release", detail: e.to_string() })?;
        Ok(Some(Release {
            // GitLab releases have no numeric id of their own; the tag is the key.
            id: 0,
            url: v
                .get("_links")
                .and_then(|l| l.get("self"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tag: tag.to_string(),
        }))
    }

    fn create_release(&self, req: &ReleaseRequest) -> Result<Release, ForgeError> {
        // No prerelease flag exists, so it goes in the name (#54). Same class of
        // gap as draft state, and the same answer: put it where a human will see
        // it rather than pretend the distinction was not made.
        let name =
            if req.prerelease { format!("{} (prerelease)", req.name) } else { req.name.clone() };
        let v = self.ok(
            "POST",
            &self.url("/releases"),
            Some(json!({
                "tag_name": req.tag,
                "ref": req.target,
                "name": name,
                "description": req.body,
            })),
        )?;
        Ok(Release {
            id: 0,
            url: v
                .get("_links")
                .and_then(|l| l.get("self"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tag: req.tag.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gl() -> GitLab {
        GitLab::new("https://gitlab.com/api/v4", "group/sub/proj", "t".into())
    }

    #[test]
    fn a_project_is_addressed_by_its_url_encoded_full_path() {
        // Subgroups need nothing special — they are one more slash.
        assert_eq!(
            gl().url("/merge_requests"),
            "https://gitlab.com/api/v4/projects/group%2Fsub%2Fproj/merge_requests"
        );
    }

    #[test]
    fn a_self_hosted_api_url_is_used_verbatim_bar_a_trailing_slash() {
        let gl = GitLab::new("https://gitlab.internal/api/v4/", "a/b", "t".into());
        assert_eq!(gl.url("/releases"), "https://gitlab.internal/api/v4/projects/a%2Fb/releases");
    }

    #[test]
    fn the_draft_prefix_is_stripped_before_it_is_added() {
        // Without this a second refusal produces `Draft: Draft: …`.
        assert_eq!(strip_draft("Draft: chore(release): prepare release"), "chore(release): prepare release");
        assert_eq!(strip_draft("chore(release): prepare release"), "chore(release): prepare release");
        // The legacy spelling is still on merge requests in the wild.
        assert_eq!(strip_draft("WIP: thing"), "thing");
        assert_eq!(strip_draft("wip: thing"), "thing");
        // A title that merely mentions the word is not a prefix.
        assert_eq!(strip_draft("chore: update the Draft: section"), "chore: update the Draft: section");
    }

    #[test]
    fn draft_state_is_read_from_the_flag_the_legacy_flag_or_the_title() {
        assert!(is_draft(&json!({"draft": true, "title": "x"})));
        assert!(!is_draft(&json!({"draft": false, "title": "Draft: x"})));
        // An older instance reports only `work_in_progress`.
        assert!(is_draft(&json!({"work_in_progress": true, "title": "x"})));
        // And an instance that reports neither leaves the title authoritative.
        assert!(is_draft(&json!({"title": "Draft: x"})));
        assert!(!is_draft(&json!({"title": "x"})));
    }

    #[test]
    fn a_merge_request_is_identified_by_iid_not_id() {
        // `id` is global; every other endpoint wants the per-project `iid`, and
        // using the wrong one addresses somebody else's merge request.
        let v = json!({"id": 91234, "iid": 7, "web_url": "u", "title": "t", "draft": false});
        let pr = mr_from(&v);
        assert_eq!(pr.number, 7);
        assert_eq!(pr.url, "u");
        assert_eq!(pr.title, "t");
        assert_eq!(pr.node_id, None);
    }

    #[test]
    fn errors_name_the_scopes_and_the_subgroup_trap() {
        let s = map_error(403, r#"{"message":"403 Forbidden"}"#, "group/sub/proj").to_string();
        assert!(s.contains("write_repository"), "{s}");
        assert!(s.contains("Maintainer"), "{s}");

        let s = map_error(404, "{}", "group/sub/proj").to_string();
        assert!(s.contains("subgroups"), "{s}");
        assert!(s.contains("scope"), "{s}");

        // GitLab's `error` shape is read as well as `message`.
        let s = map_error(401, r#"{"error":"invalid_token"}"#, "a/b").to_string();
        assert!(s.contains("invalid_token"), "{s}");
    }
}

#[cfg(test)]
mod asymmetry_tests {
    use super::*;

    /// The release name GitLab gets, since it has no prerelease flag (#54).
    fn release_name(name: &str, prerelease: bool) -> String {
        if prerelease { format!("{name} (prerelease)") } else { name.to_string() }
    }

    #[test]
    fn a_prerelease_is_marked_in_the_name_because_there_is_no_flag() {
        // GitHub carries `prerelease: true`; GitLab has nothing equivalent, so a
        // tester reading the releases list would otherwise have no way to tell an
        // rc from a stable at a glance.
        assert_eq!(release_name("app 1.5.0-rc.1", true), "app 1.5.0-rc.1 (prerelease)");
        assert_eq!(release_name("app 1.5.0", false), "app 1.5.0");
    }

    #[test]
    fn drafting_twice_does_not_stack_prefixes() {
        // The failure this prevents is cosmetic once and cumulative after that:
        // every refusal would add another `Draft: ` until the title is rejected
        // for length.
        let mut title = "chore(release): prepare release".to_string();
        for _ in 0..5 {
            title = format!("{DRAFT_PREFIX}{}", strip_draft(&title));
        }
        assert_eq!(title, "Draft: chore(release): prepare release");
        assert_eq!(strip_draft(&title), "chore(release): prepare release");
    }
}

//! The HTTP plumbing both adapters share.
//!
//! Extracted rather than duplicated because the retry policy is a **decision**
//! (#58), not an implementation detail: retry on 5xx and 429, honour
//! `Retry-After`, never retry a 4xx. Two copies of that would drift, and the one
//! that drifted would be the one nobody was looking at.
//!
//! What the adapters keep for themselves is everything forge-specific: the auth
//! header, the URL shapes, and the mapping from a status to text that names the
//! fix.

use serde_json::Value;

use super::{ForgeError, backoff, is_retryable};

/// Enough attempts to ride out a deploy or a rate-limit window, few enough that a
/// genuinely broken forge fails the job rather than hanging it.
pub const MAX_ATTEMPTS: u32 = 4;

pub struct Client {
    agent: ureq::Agent,
    /// Sent on every request. Holds the credential, so this type has no `Debug`.
    headers: Vec<(String, String)>,
}

impl Client {
    pub fn new(headers: Vec<(String, String)>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            // cicada reads statuses itself: a 404 from a "find" call is an answer,
            // not an error, and the retry policy needs the code anyway.
            .http_status_as_error(false)
            .user_agent(concat!("cicada/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self { agent, headers }
    }

    fn with_headers<B>(&self, rb: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        let mut rb = rb;
        for (k, v) in &self.headers {
            rb = rb.header(k, v);
        }
        rb
    }

    /// One request, retried per #58. Returns the status and the body; deciding
    /// what a status *means* is the adapter's job.
    pub fn send(
        &self,
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<(u16, String), ForgeError> {
        let mut attempt = 0u32;
        loop {
            let json = body.map(ToString::to_string).unwrap_or_default();
            let sent = match method {
                // `call()` is the no-body send; `send()` is the with-body one.
                "GET" => self.with_headers(self.agent.get(url)).call(),
                "POST" => self
                    .with_headers(self.agent.post(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                "PATCH" => self
                    .with_headers(self.agent.patch(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                "PUT" => self
                    .with_headers(self.agent.put(url))
                    .header("Content-Type", "application/json")
                    .send(&json),
                other => {
                    return Err(ForgeError::Decode {
                        context: "http",
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
}

/// Percent-encode a path or query segment.
///
/// Hand-rolled rather than a dependency: the inputs are branch names, tags and
/// project paths, and the set of bytes needing escaping in them is small and
/// unchanging. It matters most on GitLab, where a project is addressed by its
/// **URL-encoded full path** — `group/sub/proj` becomes `group%2Fsub%2Fproj`,
/// and a subgroup is just another slash.
pub fn urlencode(s: &str) -> String {
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

/// The `message` an API returned, when it returned one shaped like an error.
pub fn api_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            // GitHub uses `message`; GitLab uses `message` or `error` and
            // sometimes an object of field errors.
            v.get("message")
                .or_else(|| v.get("error"))
                .map(|m| match m.as_str() {
                    Some(s) => s.to_string(),
                    None => m.to_string(),
                })
        })
        .unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_covers_what_a_branch_tag_or_project_path_contains() {
        assert_eq!(urlencode("cicada/release/1.4"), "cicada%2Frelease%2F1.4");
        assert_eq!(urlencode("api-v1.5.0"), "api-v1.5.0");
        // A GitLab subgroup is just another slash.
        assert_eq!(urlencode("group/sub/proj"), "group%2Fsub%2Fproj");
        assert_eq!(urlencode("v1.0.0+build.1"), "v1.0.0%2Bbuild.1");
    }

    #[test]
    fn an_api_message_is_lifted_out_of_either_forges_error_shape() {
        assert_eq!(api_message(r#"{"message":"Not Found"}"#), "Not Found");
        assert_eq!(api_message(r#"{"error":"insufficient_scope"}"#), "insufficient_scope");
        // A field-error object is stringified rather than dropped.
        assert!(api_message(r#"{"message":{"base":["nope"]}}"#).contains("nope"));
        // And a body that is not JSON at all still yields something readable.
        assert_eq!(api_message("<html>502</html>"), "<html>502</html>");
    }
}

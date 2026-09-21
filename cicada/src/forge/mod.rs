//! The forge abstraction — the thin edge of an otherwise all-git tool.
//!
//! release-please is GitHub-only because its entire I/O layer is Octokit: it
//! reads history over GraphQL and writes commits over REST. cicada inverts that.
//! History reading, version anchoring, tagging and release detection are pure git
//! (#9), and the forge API is used for exactly two things — opening or updating a
//! pull request, and creating a release object. That is what makes a GitLab or
//! Gitea adapter cheap instead of a rewrite.
//!
//! **Nine methods, not three** (#53). The original sketch had find/create/update;
//! the staleness model (#32–#34) then needed drafting, commenting, labelling and
//! closing, because refusing to clobber a branch is only safe if the resulting
//! stale PR is *unmergeable*.
//!
//! Two invariants worth stating where they cannot be missed:
//!
//! * [`Forge::find_pull_request`] keys on the **head branch name**, never the
//!   title (#77). That is what makes the upsert idempotent by construction, so
//!   two concurrent Phase A runs cannot open two PRs (D6).
//! * It searches **open** pull requests only. A closed release PR is never
//!   reopened; idempotency rests on the head branch, not on PR identity (D5c).

pub mod fake;
pub mod flow;
pub mod github;

use cicada_core::config::{Config, ForgeKind};

/// A pull request as cicada needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub draft: bool,
    /// GitHub's GraphQL node id, when the adapter has one.
    ///
    /// Present because converting an existing PR to draft is **not** a REST
    /// operation on GitHub — see [`github`] — and the mutation that does it
    /// addresses the PR by node id.
    pub node_id: Option<String>,
}

/// Everything needed to open or refresh a release PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRequest {
    /// `cicada/{branch}` (#27).
    pub head: String,
    /// The branch cicada ran on — never a fixed default.
    pub base: String,
    /// `release_pr_title`, a plain string with no interpolation (#86).
    pub title: String,
    /// The same notes the changelog entry carries (#43, D7).
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub id: u64,
    pub url: String,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRequest {
    pub tag: String,
    pub name: String,
    pub body: String,
    /// GitHub has a real flag; GitLab has none and marks it in the name (#54).
    pub prerelease: bool,
    /// The commit the tag points at.
    pub target: String,
}

/// The label cicada applies to a release PR it could not refresh (#33 layer 2).
pub const STALE_LABEL: &str = "cicada:stale";

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// A mapped HTTP failure. The `message` is written to be *actionable*: a 403
    /// caused by a token lacking `pull-requests: write` must say that, not print
    /// a status code (#58).
    #[error("{message}")]
    Http { status: u16, message: String },

    #[error("could not reach {url}: {source}")]
    Network {
        url: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("{context}: the forge returned something cicada could not read ({detail})")]
    Decode { context: &'static str, detail: String },

    #[error(
        "no token: pass `--token` or set `CICADA_TOKEN`\n\
         `plan`, `check` and `--dry-run`/`--no-push` need no token; opening a pull request does"
    )]
    NoToken,
}

pub trait Forge {
    /// Open pull requests only, keyed on the head branch (#77).
    fn find_pull_request(&self, head: &str) -> Result<Option<PullRequest>, ForgeError>;
    fn create_pull_request(&self, req: &PrRequest) -> Result<PullRequest, ForgeError>;
    fn update_pull_request(&self, number: u64, req: &PrRequest) -> Result<PullRequest, ForgeError>;
    /// #34 — an empty plan closes an open release PR, because leaving it open
    /// advertises a release that is no longer justified.
    fn close_pull_request(&self, number: u64) -> Result<(), ForgeError>;
    /// #33 layer 2. Draft PRs are unmergeable on both forges, enforced by the
    /// forge itself and needing no repo configuration.
    fn set_draft(&self, pr: &PullRequest, draft: bool) -> Result<(), ForgeError>;
    fn comment(&self, number: u64, body: &str) -> Result<(), ForgeError>;
    fn set_labels(&self, number: u64, labels: &[String]) -> Result<(), ForgeError>;
    fn find_release(&self, tag: &str) -> Result<Option<Release>, ForgeError>;
    fn create_release(&self, req: &ReleaseRequest) -> Result<Release, ForgeError>;
}

/// Build the adapter named by `forge.kind`.
///
/// **Never inferred from the remote URL** (#66). Detection is a diagnostic that
/// `check` offers when the key is missing; the runtime reads what a human
/// approved, because a wrong guess at release time is silent.
pub fn build(
    config: &Config,
    repo: &str,
    token: String,
) -> Result<Box<dyn Forge>, ForgeError> {
    match config.forge.kind {
        ForgeKind::Github => Ok(Box::new(github::GitHub::new(
            config.forge.api_url.as_deref().unwrap_or("https://api.github.com"),
            repo,
            token,
        ))),
        // M6. Refusing by name beats a half-adapter that fails mid-release.
        ForgeKind::Gitlab => Err(ForgeError::Http {
            status: 0,
            message: "the GitLab adapter lands in M6; `--no-push` works on any forge today".into(),
        }),
    }
}

// --- retry policy (#58) --------------------------------------------------

/// Retry on 5xx and 429, **never** on 4xx.
///
/// A 4xx is cicada's fault or the token's, and repeating it just delays an error
/// the caller needs to read. 429 is the forge asking for patience and is the one
/// 4xx that means "try again".
pub(crate) fn is_retryable(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

/// How long to wait before attempt `n` (0-based), honouring `Retry-After`.
///
/// `Retry-After` wins when present and parseable: the forge knows when its own
/// rate window reopens, and guessing shorter just burns the remaining budget.
pub(crate) fn backoff(attempt: u32, retry_after: Option<&str>) -> std::time::Duration {
    if let Some(secs) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return std::time::Duration::from_secs(secs.min(60));
    }
    std::time::Duration::from_secs(1u64 << attempt.min(4))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_5xx_and_429_are_worth_repeating() {
        assert!(is_retryable(500));
        assert!(is_retryable(502));
        assert!(is_retryable(503));
        assert!(is_retryable(429));
        // A 4xx is cicada's fault or the token's; repeating it delays the error.
        for s in [400, 401, 403, 404, 409, 422] {
            assert!(!is_retryable(s), "{s}");
        }
        assert!(!is_retryable(200));
    }

    #[test]
    fn retry_after_wins_over_the_exponential_guess() {
        assert_eq!(backoff(0, Some("30")).as_secs(), 30);
        assert_eq!(backoff(3, Some("2")).as_secs(), 2);
        // Capped, so a hostile or mistaken header cannot hang a CI job for an hour.
        assert_eq!(backoff(0, Some("86400")).as_secs(), 60);
        // Unparseable falls through to the guess rather than to zero.
        assert_eq!(backoff(0, Some("Wed, 21 Oct 2026 07:28:00 GMT")).as_secs(), 1);
    }

    #[test]
    fn the_guess_doubles_and_then_stops() {
        assert_eq!(backoff(0, None).as_secs(), 1);
        assert_eq!(backoff(1, None).as_secs(), 2);
        assert_eq!(backoff(2, None).as_secs(), 4);
        assert_eq!(backoff(9, None).as_secs(), 16);
    }
}

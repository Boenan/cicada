//! Git access.
//!
//! Git is the substrate (design §How cicada differs from release-please), so this
//! is the widest I/O surface in the tool. It sits behind a trait for two reasons:
//! tests get a fake with no repo, and a `gix` read path can be swapped in later
//! without touching callers (future-work §5).
//!
//! **Subprocess discipline, from the first line** (§Tech choices):
//!
//! * machine-readable formats only — `--format=%H%x00%s%x00%b`, `-z`. Never parse
//!   human-facing output.
//! * pinned environment — `LC_ALL=C`, `GIT_CONFIG_NOSYSTEM=1`,
//!   `GIT_TERMINAL_PROMPT=0`. A CI job must fail rather than hang on a prompt.
//! * `git --version` checked at startup with a clear error if missing or too old.

mod subprocess;

pub use subprocess::GitCli;

use cicada_core::{CommitSource, RawCommit};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("`git` was not found on PATH — cicada shells out to git, so it is a required runtime dependency")]
    NotFound,
    #[error("`git` {found} is too old; cicada needs {needed} or newer")]
    TooOld { found: String, needed: String },
    #[error(
        "this is a shallow clone, so history is truncated and a changelog computed from it would be silently incomplete\n\
         set `fetch-depth: 0` (GitHub Actions) or `GIT_DEPTH: 0` (GitLab CI)"
    )]
    ShallowClone,
    #[error("not inside a git repository")]
    NotARepo,
    #[error("git {args} failed with status {status}:\n{stderr}")]
    Command { args: String, status: String, stderr: String },
    #[error("could not run git: {0}")]
    Io(#[from] std::io::Error),
    #[error("git emitted output that is not valid UTF-8 ({0})")]
    Encoding(&'static str),
}

/// The git operations cicada needs. M0 uses the read half; the write half lands
/// in M3.
pub trait GitBackend {
    /// The branch HEAD is on. `None` when detached — which is not an error here,
    /// because the branch gate (#26) is what decides whether that matters.
    fn current_branch(&self) -> Result<Option<String>, GitError>;

    /// Commits in `base..head`, oldest first, read according to `source` (#40).
    /// An unresolvable ref is an error; an empty range is not.
    fn log(
        &self,
        base: Option<&str>,
        head: &str,
        source: CommitSource,
    ) -> Result<Vec<RawCommit>, GitError>;

    /// Does this ref resolve? Used for anchor resolution (#2) — a missing anchor
    /// tag is a hard error naming the tag, never a fallback.
    fn rev_parse(&self, rev: &str) -> Result<Option<String>, GitError>;

    /// Is `ancestor` reachable from `descendant`? The primitive behind both the
    /// anchor reachability rule (#3) and collision detection (#9).
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, GitError>;

    /// Every tag in the repo, for collision detection.
    fn tags(&self) -> Result<Vec<String>, GitError>;

    /// Checked up front so a truncated history is never mistaken for a short one.
    fn assert_not_shallow(&self) -> Result<(), GitError>;
}

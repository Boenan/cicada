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

/// One release commit, to be built without disturbing anything.
///
/// The branch cicada writes is **not** the branch it is standing on, and the
/// obvious implementation — check out the release PR branch, write the files,
/// commit, check back — is wrong for this tool in three separate ways (#100):
///
/// * `--no-push` is the *local rehearsal* mode (§Goals 6), so a developer running
///   it on their own machine must get their tree back exactly as it was. A
///   command that leaves you on another branch is hostile precisely where it is
///   meant to be safe.
/// * a failure between the two checkouts strands the run — and the caller — on
///   the wrong branch, with edits applied.
/// * two overlapping Phase A runs in one workspace (D6) would corrupt each
///   other's checkout. Built through a private index they cannot interact at all.
///
/// So the commit is assembled with plumbing: a temporary index seeded from
/// `base`, the edits hashed into it, a tree written, a commit created, and the
/// ref moved. Nothing observable changes except the branch.
pub struct CommitRequest<'a> {
    /// The ref to move, e.g. `cicada/main` — a short branch name, not `refs/…`.
    pub branch: &'a str,
    /// The commit whose tree the edits are applied to, and the new commit's
    /// parent. Always HEAD in Phase A: the branch is **replaced, never updated**
    /// (#27, D1), so nothing is merged or rebased and there is never a conflict.
    pub base: &'a str,
    pub edits: &'a [cicada_core::edit::FileEdit],
    pub message: &'a str,
    pub identity: &'a cicada_core::Identity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    pub sha: String,
    /// `false` when the branch already pointed at a commit with this exact tree
    /// and parent, so nothing was created.
    ///
    /// Re-running Phase A on an unchanged repo would otherwise mint a new commit
    /// every time — identical content under a new sha, because the timestamp
    /// moved — and force-push it. Harmless but noisy, and it makes "did anything
    /// change?" unanswerable from the outside.
    pub created: bool,
}

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

    /// `--force-with-lease` refused: the remote moved between the read and the
    /// push. Self-correcting — the next run replans against the new tip — so it
    /// is reported as its own condition rather than as a generic git failure.
    #[error(
        "the remote `{branch}` moved while cicada was working, so the push was refused\n\
         nothing was lost; run again and cicada will recompute against the new tip"
    )]
    LeaseLost { branch: String },
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

    /// The commit a branch points at, or `None` when the branch does not exist.
    fn branch_tip(&self, branch: &str) -> Result<Option<String>, GitError>;

    /// The `Name <email>` that committed `rev` — what #32's guard compares
    /// against to decide whether a release PR branch is cicada's to replace.
    fn committer_of(&self, rev: &str) -> Result<Option<String>, GitError>;

    /// Build a commit containing `base`'s tree plus `edits`, parented on `base`,
    /// and point `branch` at it.
    ///
    /// **The working tree and the real index are never touched** (#100). See
    /// [`CommitRequest`] for why that matters more than it looks.
    fn commit_edits(&self, req: &CommitRequest<'_>) -> Result<CommitOutcome, GitError>;

    /// Create an **annotated** tag at `sha`, tagged by `identity`.
    ///
    /// Annotated rather than lightweight: the tag is the anchor (#2) and the
    /// thing a release object hangs off, so it is worth carrying who made it and
    /// when. `rev_parse` peels `^{commit}`, so nothing downstream can tell the
    /// difference.
    ///
    /// Does not touch the working tree or the index.
    fn create_tag(
        &self,
        tag: &str,
        sha: &str,
        message: &str,
        identity: &cicada_core::Identity,
    ) -> Result<(), GitError>;

    /// Push one tag. **Never forced**: a tag that already exists on the remote at
    /// a different commit is a version collision (#9), and quietly overwriting it
    /// would replace a published release's contents.
    fn push_tag(&self, url: &str, tag: &str, env: &[(String, String)]) -> Result<(), GitError>;

    /// Fetch one branch from `url` and return the sha it points at there, or
    /// `None` when the remote does not have it.
    ///
    /// **This is what makes #32's guard real.** `branch_tip` reads
    /// `refs/heads/…`, which in CI does not exist: a fresh clone has no local
    /// `cicada/main`, so a guard that consulted only the local ref would see
    /// nothing, find no human commit, and force-push over one. The branch that is
    /// about to be clobbered is the **remote** one, so that is the branch the
    /// guard has to look at.
    fn fetch_branch(
        &self,
        url: &str,
        branch: &str,
        env: &[(String, String)],
    ) -> Result<Option<String>, GitError>;

    /// Force-push `sha` to `branch` on `url`, refusing if the remote has moved
    /// away from `expected` since it was read.
    ///
    /// `--force-with-lease` rather than `--force`: cicada's own guard closes the
    /// window it can see, and the lease closes the one it cannot — a second Phase
    /// A run, or a human, landing a commit between the fetch and the push (D6).
    fn push_branch(
        &self,
        url: &str,
        sha: &str,
        branch: &str,
        expected: Option<&str>,
        env: &[(String, String)],
    ) -> Result<(), GitError>;

    /// A remote's URL, or `None` when the remote is not configured.
    ///
    /// Used for one thing: deriving `org/repo` for changelog links when config
    /// does not state it. It is **never** how `forge.kind` is decided (#66) —
    /// a wrong link costs a 404, a wrong forge costs a release.
    fn remote_url(&self, name: &str) -> Result<Option<String>, GitError>;
}

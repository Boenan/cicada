//! `GitBackend` against a real repository.
//!
//! The subprocess impl is the widest I/O surface in the tool, so it is tested
//! against real git rather than mocked. Each test builds a throwaway repo from a
//! commit script, the same convention the scenario files use.

use std::path::{Path, PathBuf};
use std::process::Command;

use cicada::git::{GitBackend, GitCli};
use cicada_core::CommitSource;

struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cicada-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = TempRepo { dir };
        r.git(&["init", "--initial-branch=main"]);
        r.git(&["config", "user.email", "test@example.invalid"]);
        r.git(&["config", "user.name", "Test"]);
        r
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn commit(&self, subject: &str) {
        self.commit_with_body(subject, "");
    }

    fn commit_with_body(&self, subject: &str, body: &str) {
        let n = self.git(&["rev-list", "--count", "--all"]).trim().to_string();
        std::fs::write(self.dir.join(format!("f{n}.txt")), subject).unwrap();
        self.git(&["add", "-A"]);
        if body.is_empty() {
            self.git(&["commit", "-m", subject]);
        } else {
            self.git(&["commit", "-m", subject, "-m", body]);
        }
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn reads_a_range_oldest_first() {
    let r = TempRepo::new("range");
    r.commit("feat: one");
    let first = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.commit("fix: two");
    r.commit("chore: three");

    let g = GitCli::discover(r.path()).unwrap();
    let commits = g.log(Some(&first), "HEAD", CommitSource::Commits).unwrap();
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["fix: two", "chore: three"]);
}

#[test]
fn an_unanchored_range_is_all_reachable_history() {
    let r = TempRepo::new("unanchored");
    r.commit("feat: one");
    r.commit("fix: two");
    let g = GitCli::discover(r.path()).unwrap();
    assert_eq!(g.log(None, "HEAD", CommitSource::Commits).unwrap().len(), 2);
}

#[test]
fn an_empty_range_is_not_an_error() {
    let r = TempRepo::new("empty");
    r.commit("feat: one");
    let g = GitCli::discover(r.path()).unwrap();
    assert!(g.log(Some("HEAD"), "HEAD", CommitSource::Commits).unwrap().is_empty());
}

#[test]
fn bodies_survive_the_round_trip() {
    let r = TempRepo::new("bodies");
    r.commit("feat: one");
    let first = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.commit_with_body("fix: two", "BREAKING CHANGE: it changed\nRefs: #42");

    let g = GitCli::discover(r.path()).unwrap();
    let commits = g.log(Some(&first), "HEAD", CommitSource::Commits).unwrap();
    assert_eq!(commits.len(), 1);
    assert!(commits[0].body.contains("BREAKING CHANGE: it changed"));
    assert!(commits[0].body.contains("Refs: #42"));
}

#[test]
fn commit_source_agrees_on_a_linear_repo() {
    // J6: on a squash/rebase repo main is linear, so all three traversals return
    // the identical set. This is the claim that lets cicada work out of the box
    // regardless of merge strategy.
    let r = TempRepo::new("linear");
    r.commit("feat: one");
    let first = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.commit("fix: two");
    r.commit("fix: three");

    let g = GitCli::discover(r.path()).unwrap();
    let by = |s| {
        g.log(Some(&first), "HEAD", s)
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect::<Vec<_>>()
    };
    let commits = by(CommitSource::Commits);
    assert_eq!(commits, by(CommitSource::Merges));
    assert_eq!(commits, by(CommitSource::Both));
}

#[test]
fn commit_source_differs_on_a_merge_commit_repo() {
    // J5/J7: the only shape where the setting matters. `commits` sees the work,
    // `merges` sees the merge commit's own message.
    let r = TempRepo::new("merges");
    r.commit("feat: base");
    let base = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "-b", "feature"]);
    r.commit("feat: add export");
    r.commit("chore: tidy");
    r.git(&["checkout", "main"]);
    r.git(&["merge", "--no-ff", "feature", "-m", "Merge pull request #41 from acme/feature"]);

    let g = GitCli::discover(r.path()).unwrap();
    let subjects = |s| {
        g.log(Some(&base), "HEAD", s)
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect::<Vec<_>>()
    };

    let commits = subjects(CommitSource::Commits);
    assert!(commits.contains(&"feat: add export".to_string()));
    assert!(!commits.iter().any(|s| s.starts_with("Merge pull request")));

    let merges = subjects(CommitSource::Merges);
    assert_eq!(merges, ["Merge pull request #41 from acme/feature"]);

    let both = subjects(CommitSource::Both);
    assert!(both.len() > commits.len());
}

#[test]
fn rev_parse_resolves_tags_and_reports_absence() {
    let r = TempRepo::new("revparse");
    r.commit("feat: one");
    r.git(&["tag", "v1.0.0"]);
    r.git(&["tag", "-a", "v1.0.1", "-m", "annotated"]);

    let g = GitCli::discover(r.path()).unwrap();
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(g.rev_parse("v1.0.0").unwrap().as_deref(), Some(head.as_str()));
    // An annotated tag peels to the commit, so both spellings compare equal.
    assert_eq!(g.rev_parse("v1.0.1").unwrap().as_deref(), Some(head.as_str()));
    // A missing tag is `None`, not an error — the caller decides that it is fatal.
    assert_eq!(g.rev_parse("v9.9.9").unwrap(), None);
}

#[test]
fn is_ancestor_is_the_reachability_primitive() {
    // The primitive behind both the anchor rule (#3) and collision detection (#9).
    let r = TempRepo::new("ancestor");
    r.commit("feat: one");
    let first = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "-b", "other"]);
    r.commit("feat: elsewhere");
    let elsewhere = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "main"]);
    r.commit("fix: on main");

    let g = GitCli::discover(r.path()).unwrap();
    assert!(g.is_ancestor(&first, "HEAD").unwrap());
    // A tag that exists elsewhere in the repo is NOT reachable — the case that
    // means two branches are converging on one version.
    assert!(!g.is_ancestor(&elsewhere, "HEAD").unwrap());
}

#[test]
fn tags_are_listed() {
    let r = TempRepo::new("tags");
    r.commit("feat: one");
    r.git(&["tag", "v1.0.0"]);
    r.git(&["tag", "app-v1.0.0"]);
    let g = GitCli::discover(r.path()).unwrap();
    let mut tags = g.tags().unwrap();
    tags.sort();
    assert_eq!(tags, ["app-v1.0.0", "v1.0.0"]);
}

#[test]
fn current_branch_is_reported_and_detached_head_is_not_an_error() {
    let r = TempRepo::new("branch");
    r.commit("feat: one");
    let g = GitCli::discover(r.path()).unwrap();
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));

    let sha = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.git(&["checkout", "--detach", &sha]);
    let g = GitCli::discover(r.path()).unwrap();
    assert_eq!(g.current_branch().unwrap(), None);
}

#[test]
fn a_full_clone_is_not_shallow() {
    let r = TempRepo::new("shallow");
    r.commit("feat: one");
    let g = GitCli::discover(r.path()).unwrap();
    assert!(g.assert_not_shallow().is_ok());
}

#[test]
fn discover_finds_the_root_from_a_subdirectory() {
    let r = TempRepo::new("subdir");
    r.commit("feat: one");
    let sub = r.path().join("services/api");
    std::fs::create_dir_all(&sub).unwrap();
    let g = GitCli::discover(&sub).unwrap();
    assert_eq!(
        g.root().canonicalize().unwrap(),
        r.path().canonicalize().unwrap(),
        "config and paths resolve from the repo root, not the cwd"
    );
}

// --- the write half (M3) -------------------------------------------------

use cicada::git::CommitRequest;
use cicada_core::Identity;
use cicada_core::edit::{EditSource, FileEdit};

fn bot() -> Identity {
    Identity::parse("cicada[bot] <cicada@users.noreply.github.com>").unwrap()
}

fn edit(path: &str, content: &str) -> FileEdit {
    FileEdit { path: path.into(), content: content.into(), source: EditSource::VersionsFile }
}

/// One release commit onto `cicada/main` from HEAD, which is what every case
/// below varies one thing about.
fn commit_one(
    git: &GitCli,
    base: &str,
    edits: &[FileEdit],
) -> Result<cicada::git::CommitOutcome, cicada::git::GitError> {
    git.commit_edits(&CommitRequest {
        branch: "cicada/main",
        base,
        edits,
        message: "chore(release): prepare release",
        identity: &bot(),
    })
}

#[test]
fn commit_edits_creates_the_branch_and_leaves_the_working_tree_alone() {
    let r = TempRepo::new("write-basic");
    r.commit("chore: init");
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    let before = r.git(&["status", "--porcelain"]);

    let git = GitCli::discover(&r.dir).unwrap();
    let out = commit_one(&git, "HEAD", &[edit("VERSION", "1.5.0\n")]).unwrap();

    assert!(out.created);
    assert_eq!(git.branch_tip("cicada/main").unwrap().as_deref(), Some(out.sha.as_str()));
    assert_eq!(r.git(&["show", &format!("{}:VERSION", out.sha)]), "1.5.0\n");
    assert_eq!(r.git(&["log", "-1", "--format=%P", &out.sha]).trim(), head);

    // Nothing observable changed except the branch (#100).
    assert_eq!(r.git(&["status", "--porcelain"]), before);
    assert_eq!(r.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main");
    assert!(!r.dir.join("VERSION").exists(), "the file exists in the commit, not on disk");
}

#[test]
fn uncommitted_work_in_the_tree_never_leaks_into_the_release_commit() {
    // The property the private index buys, and the one that would be silently
    // wrong under a checkout-and-commit implementation: a developer's half-finished
    // edits must not be swept into a release.
    let r = TempRepo::new("write-dirty");
    std::fs::write(r.dir.join("src.txt"), "committed\n").unwrap();
    r.git(&["add", "-A"]);
    r.git(&["commit", "-m", "chore: init"]);

    // Dirty the tree, and stage part of it, the way a developer mid-task would.
    std::fs::write(r.dir.join("src.txt"), "HALF FINISHED\n").unwrap();
    std::fs::write(r.dir.join("scratch.txt"), "notes\n").unwrap();
    r.git(&["add", "src.txt"]);

    let git = GitCli::discover(&r.dir).unwrap();
    let out = commit_one(&git, "HEAD", &[edit("VERSION", "1.5.0\n")]).unwrap();

    // The release commit holds HEAD's tree plus the edits — not the tree.
    assert_eq!(r.git(&["show", &format!("{}:src.txt", out.sha)]), "committed\n");
    assert_eq!(r.git(&["show", &format!("{}:VERSION", out.sha)]), "1.5.0\n");
    assert!(
        r.git(&["ls-tree", "-r", "--name-only", &out.sha]).lines().all(|l| l != "scratch.txt"),
        "an untracked file was swept in"
    );

    // And the developer's staging area is exactly as they left it.
    assert_eq!(std::fs::read_to_string(r.dir.join("src.txt")).unwrap(), "HALF FINISHED\n");
    let staged = r.git(&["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "src.txt", "the real index was disturbed");
}

#[test]
fn a_second_call_with_the_same_content_creates_nothing() {
    let r = TempRepo::new("write-idem");
    r.commit("chore: init");
    let git = GitCli::discover(&r.dir).unwrap();

    let first = commit_one(&git, "HEAD", &[edit("VERSION", "1.5.0\n")]).unwrap();
    assert!(first.created);

    let second = commit_one(&git, "HEAD", &[edit("VERSION", "1.5.0\n")]).unwrap();
    assert!(!second.created, "identical content should not mint a new sha");
    assert_eq!(second.sha, first.sha);

    // Different content does create one.
    let third = commit_one(&git, "HEAD", &[edit("VERSION", "1.6.0\n")]).unwrap();
    assert!(third.created);
    assert_ne!(third.sha, first.sha);
}

#[test]
fn branch_tip_and_committer_of_answer_the_staleness_guards_question() {
    let r = TempRepo::new("write-guard");
    r.commit("chore: init");
    let git = GitCli::discover(&r.dir).unwrap();

    assert_eq!(git.branch_tip("cicada/main").unwrap(), None);
    let out = commit_one(&git, "HEAD", &[edit("VERSION", "1.5.0\n")]).unwrap();

    assert_eq!(git.branch_tip("cicada/main").unwrap().as_deref(), Some(out.sha.as_str()));
    // Exactly the string #32 compares against, so `Identity::to_git` round-trips
    // through git's own `%cn <%ce>`.
    assert_eq!(git.committer_of(&out.sha).unwrap().as_deref(), Some(bot().to_git().as_str()));
    // The human commit on `main` is not cicada's.
    assert_eq!(git.committer_of("main").unwrap().as_deref(), Some("Test <test@example.invalid>"));
}

#[test]
fn a_base_that_does_not_resolve_is_an_error_rather_than_an_empty_commit() {
    let r = TempRepo::new("write-nobase");
    r.commit("chore: init");
    let git = GitCli::discover(&r.dir).unwrap();

    let err = commit_one(&git, "refs/heads/nope", &[edit("VERSION", "1.5.0\n")]).unwrap_err();
    assert!(err.to_string().contains("does not resolve"), "{err}");
    assert_eq!(git.branch_tip("cicada/main").unwrap(), None);
}

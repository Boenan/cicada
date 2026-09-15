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

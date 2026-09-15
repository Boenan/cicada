use std::path::{Path, PathBuf};
use std::process::Command;

use cicada_core::{CommitSource, RawCommit};

use super::{GitBackend, GitError};

/// `--format` is NUL-delimited so a subject containing a newline, a tab or any
/// other plausible character cannot break the parse. The record separator is a
/// double NUL, which cannot occur inside any field.
const LOG_FORMAT: &str = "--format=%H%x00%s%x00%b%x00%x00";

/// Oldest supported git. `--first-parent` with `--format` and `%x00` placeholders
/// have been stable far longer than this; 2.20 is a conservative floor that every
/// CI image in circulation clears.
const MIN_GIT: (u32, u32) = (2, 20);

pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    /// Discover the repository root from `cwd`, checking the git version first so
    /// "git is too old" never surfaces as a confusing parse failure downstream.
    pub fn discover(cwd: &Path) -> Result<Self, GitError> {
        check_version(cwd)?;
        let out = run(cwd, &["rev-parse", "--show-toplevel"])?;
        let root = out.trim();
        if root.is_empty() {
            return Err(GitError::NotARepo);
        }
        Ok(Self { root: PathBuf::from(root) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn git(&self, args: &[&str]) -> Result<String, GitError> {
        run(&self.root, args)
    }
}

fn check_version(cwd: &Path) -> Result<(), GitError> {
    let out = match run(cwd, &["--version"]) {
        Ok(o) => o,
        Err(GitError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GitError::NotFound);
        }
        Err(e) => return Err(e),
    };
    // "git version 2.43.0" — the one place a human-shaped string is unavoidable,
    // so it is parsed defensively and a failure to parse is not fatal.
    let Some(v) = out.split_whitespace().nth(2) else { return Ok(()) };
    let mut parts = v.split('.').filter_map(|p| p.parse::<u32>().ok());
    let (Some(maj), Some(min)) = (parts.next(), parts.next()) else { return Ok(()) };
    if (maj, min) < MIN_GIT {
        return Err(GitError::TooOld {
            found: v.to_string(),
            needed: format!("{}.{}", MIN_GIT.0, MIN_GIT.1),
        });
    }
    Ok(())
}

fn run(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        // Pinned environment: identical output regardless of the developer's
        // locale, global config, or credential helpers.
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;

    if !out.status.success() {
        return Err(GitError::Command {
            args: args.join(" "),
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    String::from_utf8(out.stdout).map_err(|_| GitError::Encoding("stdout"))
}

/// Same as [`run`], but a non-zero exit is reported as `Ok(None)` rather than an
/// error — for the questions where "no" is an answer (`rev-parse` of a tag that
/// does not exist, `merge-base --is-ancestor`).
fn run_opt(cwd: &Path, args: &[&str]) -> Result<Option<String>, GitError> {
    match run(cwd, args) {
        Ok(s) => Ok(Some(s)),
        Err(GitError::Command { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

impl GitBackend for GitCli {
    fn current_branch(&self) -> Result<Option<String>, GitError> {
        let out = self.git(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
        match out {
            Ok(s) => Ok(Some(s.trim().to_string())),
            // Detached HEAD: `symbolic-ref` exits non-zero, which is an answer.
            Err(GitError::Command { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn log(
        &self,
        base: Option<&str>,
        head: &str,
        source: CommitSource,
    ) -> Result<Vec<RawCommit>, GitError> {
        let range = match base {
            Some(b) => format!("{b}..{head}"),
            None => head.to_string(),
        };
        let mut args: Vec<&str> = vec!["log", LOG_FORMAT];
        match source {
            // `--no-merges`: every real commit; merge commits skipped.
            CommitSource::Commits => args.push("--no-merges"),
            // `--first-parent`: one entry per PR, from the merge commit's message.
            CommitSource::Merges => args.push("--first-parent"),
            // Everything; can double-count a change (#40, J5).
            CommitSource::Both => {}
        }
        args.push(&range);

        let out = self.git(&args)?;
        Ok(parse_log(&out))
    }

    fn rev_parse(&self, rev: &str) -> Result<Option<String>, GitError> {
        // `^{commit}` peels annotated tags, so a tag and a lightweight tag on the
        // same commit compare equal.
        let spec = format!("{rev}^{{commit}}");
        Ok(run_opt(&self.root, &["rev-parse", "--verify", "--quiet", &spec])?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool, GitError> {
        Ok(run_opt(&self.root, &["merge-base", "--is-ancestor", ancestor, descendant])?.is_some())
    }

    fn tags(&self) -> Result<Vec<String>, GitError> {
        let out = self.git(&["tag", "--list"])?;
        Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
    }

    fn assert_not_shallow(&self) -> Result<(), GitError> {
        if self.git(&["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
            return Err(GitError::ShallowClone);
        }
        Ok(())
    }
}

/// Split the NUL-delimited log stream. Kept free-standing so it is testable
/// without a repo.
fn parse_log(out: &str) -> Vec<RawCommit> {
    let mut commits: Vec<RawCommit> = Vec::new();
    for record in out.split("\0\0") {
        // git separates entries with a newline, and the trailing separator leaves
        // a final fragment of pure padding. NUL is not whitespace, so it has to be
        // named explicitly or an empty record parses as a commit with no sha.
        let record = record.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(3, '\0');
        let (Some(sha), Some(subject)) = (fields.next(), fields.next()) else { continue };
        let body = fields.next().unwrap_or("");
        commits.push(RawCommit::new(sha.trim(), subject, body));
    }
    // git log is newest-first; every consumer wants oldest-first.
    commits.reverse();
    commits
}

#[cfg(test)]
mod tests {
    use super::parse_log;

    #[test]
    fn splits_records_and_reverses_to_oldest_first() {
        // Exactly what `--format=%H%x00%s%x00%b%x00%x00` produces: NUL-separated
        // fields, a double-NUL record separator, and git's own newline between
        // entries.
        let out = "b2\0feat: two\0body two\n\0\0\na1\0fix: one\0\0\0\n";
        let got = parse_log(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].sha, "a1");
        assert_eq!(got[0].subject, "fix: one");
        assert_eq!(got[1].sha, "b2");
        assert_eq!(got[1].body.trim(), "body two");
    }

    #[test]
    fn a_subject_containing_a_newline_survives() {
        // The reason the format is NUL-delimited rather than line-oriented.
        let out = "a1\0fix: one\nstill the subject\0\0\0\n";
        let got = parse_log(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].subject, "fix: one\nstill the subject");
    }

    #[test]
    fn empty_output_is_an_empty_range_not_an_error() {
        assert!(parse_log("").is_empty());
    }
}

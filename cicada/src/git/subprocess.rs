use std::path::{Path, PathBuf};
use std::process::Command;

use cicada_core::{CommitSource, RawCommit};

use super::{CommitOutcome, CommitRequest, GitBackend, GitError};

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
    run_full(cwd, args, &[], None)
}

/// The one place a git subprocess is spawned.
///
/// `env` adds to the pinned base rather than replacing it, and `stdin` is bytes
/// because `hash-object --stdin` is fed file content that cicada must not
/// re-encode on the way past.
fn run_full(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, String)],
    stdin: Option<&[u8]>,
) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        // Pinned environment: identical output regardless of the developer's
        // locale, global config, or credential helpers.
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    for (k, v) in env {
        cmd.env(k, v);
    }

    let out = if let Some(bytes) = stdin {
        use std::io::Write;
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;
        child.stdin.take().expect("stdin was piped").write_all(bytes)?;
        child.wait_with_output()?
    } else {
        cmd.output()?
    };

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

    fn remote_url(&self, name: &str) -> Result<Option<String>, GitError> {
        Ok(run_opt(&self.root, &["remote", "get-url", name])?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    fn branch_tip(&self, branch: &str) -> Result<Option<String>, GitError> {
        // `refs/heads/` explicitly: a tag and a branch may share a name, and a
        // question about a branch must not be answered by a tag.
        self.rev_parse(&format!("refs/heads/{branch}"))
    }

    fn committer_of(&self, rev: &str) -> Result<Option<String>, GitError> {
        Ok(run_opt(&self.root, &["log", "-1", "--format=%cn <%ce>", rev])?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    fn commit_edits(&self, req: &CommitRequest<'_>) -> Result<CommitOutcome, GitError> {
        let base = self
            .rev_parse(req.base)?
            .ok_or_else(|| GitError::Command {
                args: format!("rev-parse {}", req.base),
                status: "no such commit".into(),
                stderr: format!("`{}` does not resolve", req.base),
            })?;

        // A private index, so the real one — and whatever the developer had
        // staged in it — is untouched (#100). Named by pid so two runs in one
        // workspace cannot collide (D6).
        let index = self.root.join(".git").join(format!("cicada-index-{}", std::process::id()));
        let _guard = TempIndex(index.clone());
        let env = [("GIT_INDEX_FILE", index.to_string_lossy().to_string())];

        run_full(&self.root, &["read-tree", &base], &env, None)?;

        for e in req.edits {
            // `--path` applies the same gitattributes clean filters `git add`
            // would, so a repo with `* text=auto` gets the content git expects to
            // store rather than the bytes cicada happened to compute.
            let blob = run_full(
                &self.root,
                &["hash-object", "-w", "--path", &e.path, "--stdin"],
                &[],
                Some(e.content.as_bytes()),
            )?
            .trim()
            .to_string();

            // Preserve the file's existing mode. A Makefile or shell script
            // carrying a marker comment (#50) may well be executable, and
            // hard-coding 100644 would silently un-execute it.
            let mode = self.mode_of(&base, &e.path)?.unwrap_or_else(|| "100644".to_string());
            run_full(
                &self.root,
                &["update-index", "--add", "--cacheinfo", &format!("{mode},{blob},{}", e.path)],
                &env,
                None,
            )?;
        }

        let tree = run_full(&self.root, &["write-tree"], &env, None)?.trim().to_string();

        // Idempotency: if the branch already holds this exact tree on this exact
        // parent, there is nothing to do. Without this, a re-run mints a new sha
        // for identical content purely because the clock moved.
        if let Some(tip) = self.branch_tip(req.branch)? {
            // Not via `rev_parse`: that peels `^{commit}`, so asking it for a
            // *tree* resolves nothing and the comparison silently never matches.
            // The first version of this had exactly that bug, and it passed its
            // test whenever two runs landed in the same second.
            let same_tree = self.tree_of(&tip)?.as_deref() == Some(tree.as_str());
            let same_parent = self.parents_of(&tip)? == vec![base.clone()];
            if same_tree && same_parent {
                return Ok(CommitOutcome { sha: tip, created: false });
            }
        }

        // Author *and* committer, both cicada: the guard reads `%cn <%ce>` (#32),
        // and an author that differed would make `git log` misleading for no gain.
        let commit_env = [
            ("GIT_AUTHOR_NAME", req.identity.name.clone()),
            ("GIT_AUTHOR_EMAIL", req.identity.email.clone()),
            ("GIT_COMMITTER_NAME", req.identity.name.clone()),
            ("GIT_COMMITTER_EMAIL", req.identity.email.clone()),
        ];

        let sha = run_full(
            &self.root,
            &["commit-tree", &tree, "-p", &base, "-m", req.message],
            &commit_env,
            None,
        )?
        .trim()
        .to_string();

        run_full(
            &self.root,
            &["update-ref", &format!("refs/heads/{}", req.branch), &sha],
            &[],
            None,
        )?;

        Ok(CommitOutcome { sha, created: true })
    }

    fn create_tag(
        &self,
        tag: &str,
        sha: &str,
        message: &str,
        identity: &cicada_core::Identity,
    ) -> Result<(), GitError> {
        let env = [
            ("GIT_COMMITTER_NAME", identity.name.clone()),
            ("GIT_COMMITTER_EMAIL", identity.email.clone()),
        ];
        run_full(&self.root, &["tag", "-a", tag, "-m", message, sha], &env, None)?;
        Ok(())
    }

    fn push_tag(&self, url: &str, tag: &str, env: &[(String, String)]) -> Result<(), GitError> {
        let env: Vec<(&str, String)> = env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let refspec = format!("refs/tags/{tag}:refs/tags/{tag}");
        run_full(&self.root, &["push", "--quiet", url, &refspec], &env, None)?;
        Ok(())
    }

    fn fetch_branch(
        &self,
        url: &str,
        branch: &str,
        env: &[(String, String)],
    ) -> Result<Option<String>, GitError> {
        // Into a dedicated ref rather than `refs/remotes/origin/…`: cicada must
        // not quietly rewrite the remote-tracking refs of the workspace it was
        // handed, and the answer it needs is this one branch.
        let local = format!("refs/cicada/head/{branch}");
        let refspec = format!("+refs/heads/{branch}:{local}");
        let env: Vec<(&str, String)> = env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();

        // A branch the remote does not have is an answer, not an error: it is the
        // ordinary state before cicada has ever run on this line.
        match run_full(&self.root, &["fetch", "--no-tags", "--quiet", url, &refspec], &env, None) {
            Ok(_) => {}
            Err(GitError::Command { stderr, .. })
                if stderr.contains("couldn't find remote ref")
                    || stderr.contains("does not appear to be a git repository") =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
        self.rev_parse(&local)
    }

    fn push_branch(
        &self,
        url: &str,
        sha: &str,
        branch: &str,
        expected: Option<&str>,
        env: &[(String, String)],
    ) -> Result<(), GitError> {
        let lease = match expected {
            // The precise form: refuse unless the remote is still exactly where
            // it was when cicada read it.
            Some(e) => format!("--force-with-lease={branch}:{e}"),
            // No remote branch yet, so there is nothing to lease against — but
            // `--force-if-includes` is wrong here too, since the branch is
            // replaced rather than advanced. Requiring creation is the check.
            None => "--force-with-lease".to_string(),
        };
        let refspec = format!("{sha}:refs/heads/{branch}");
        let env: Vec<(&str, String)> = env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();

        match run_full(&self.root, &["push", "--quiet", &lease, url, &refspec], &env, None) {
            Ok(_) => Ok(()),
            Err(GitError::Command { stderr, .. })
                if stderr.contains("stale info") || stderr.contains("fetch first") =>
            {
                Err(GitError::LeaseLost { branch: branch.to_string() })
            }
            Err(e) => Err(e),
        }
    }

    fn assert_not_shallow(&self) -> Result<(), GitError> {
        if self.git(&["rev-parse", "--is-shallow-repository"])?.trim() == "true" {
            return Err(GitError::ShallowClone);
        }
        Ok(())
    }
}

/// Removes the private index whatever happens — including on the error paths,
/// where leaving a stale index file in `.git` would be a small, confusing mess.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl GitCli {
    /// The mode a path has in `rev`, if it is tracked there.
    fn mode_of(&self, rev: &str, path: &str) -> Result<Option<String>, GitError> {
        let out = run_opt(&self.root, &["ls-tree", "-z", rev, "--", path])?.unwrap_or_default();
        // `<mode> <type> <sha>\t<path>\0`
        Ok(out
            .split('\0')
            .find(|r| !r.trim().is_empty())
            .and_then(|r| r.split_whitespace().next())
            .map(str::to_string))
    }

    /// The tree a commit points at.
    fn tree_of(&self, rev: &str) -> Result<Option<String>, GitError> {
        Ok(run_opt(&self.root, &["rev-parse", "--verify", "--quiet", &format!("{rev}^{{tree}}")])?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    fn parents_of(&self, rev: &str) -> Result<Vec<String>, GitError> {
        Ok(run_opt(&self.root, &["log", "-1", "--format=%P", rev])?
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect())
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

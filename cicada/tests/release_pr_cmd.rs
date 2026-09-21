//! `cicada release-pr --dry-run` end to end — M2's exit criterion.
//!
//! *"prints exactly the diff it would write"*, so these assert on the diff and,
//! just as importantly, on the working tree being untouched afterwards: a dry run
//! that writes is not a dry run, and #30's guarantee is the reason `--dry-run` is
//! trustworthy at all.

use std::path::PathBuf;
use std::process::Command;

use assert_cmd::prelude::*;

const SHAPE_1: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"
repo = "acme/app"

[packages.app]
path       = "."
tag_format = "v{version}"
changelog  = "CHANGELOG.md"

[[packages.app.updaters]]
kind = "cargo"
path = "Cargo.toml"
"#;

const CARGO: &str = "[package]\nname    = \"app\"\nversion = \"1.4.2\"   # managed by cicada\nedition = \"2024\"\n";

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, config: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cicada-rpr-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = Fixture { dir };
        f.git(&["init", "--initial-branch=main"]);
        f.git(&["config", "user.email", "t@example.invalid"]);
        f.git(&["config", "user.name", "T"]);
        f.write("cicada.toml", config);
        f.commit("chore: init");
        f
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn write(&self, rel: &str, content: &str) {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.join(rel)).unwrap()
    }

    fn commit(&self, subject: &str) {
        let n = self.git(&["rev-list", "--count", "--all"]).trim().to_string();
        self.write(&format!("f{n}.txt"), subject);
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", subject]);
    }

    fn cicada(&self, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("cicada")
            .unwrap()
            .args(args)
            .current_dir(&self.dir)
            .env("NO_COLOR", "1")
            .env_remove("CICADA_TOKEN")
            .output()
            .unwrap()
    }

    fn dry_run(&self) -> std::process::Output {
        self.cicada(&["release-pr", "--dry-run", "--date", "2026-09-21"])
    }

    /// Everything git can see as changed, including untracked files.
    fn dirty(&self) -> String {
        self.git(&["status", "--porcelain"])
    }

    fn no_push(&self) -> std::process::Output {
        self.cicada(&["release-pr", "--no-push", "--date", "2026-09-21"])
    }

    fn branch_tip(&self, branch: &str) -> Option<String> {
        let out = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")])
            .current_dir(&self.dir)
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !s.is_empty() { Some(s) } else { None }
    }

    /// The file's content as of a given ref, straight out of the object store —
    /// the only honest way to assert what a commit contains when the working tree
    /// was deliberately never touched.
    fn show(&self, rev: &str, path: &str) -> String {
        self.git(&["show", &format!("{rev}:{path}")])
    }

    fn log_format(&self, rev: &str, fmt: &str) -> String {
        self.git(&["log", "-1", &format!("--format={fmt}"), rev]).trim().to_string()
    }

    fn count_commits(&self, rev: &str) -> usize {
        self.git(&["rev-list", "--count", rev]).trim().parse().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn code(out: &std::process::Output) -> i32 {
    out.status.code().unwrap()
}
fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}
fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// Put the fixture in the "already released 1.4.2" state.
///
/// Returns nothing on purpose: an earlier version handed back a second `Fixture`
/// over the same directory, and its `Drop` deleted the tree the moment the
/// statement ended.
fn released(f: &Fixture) {
    f.git(&["tag", "v1.4.2"]);
    f.write(".cicada-versions.toml", "[packages.app]\nversion = \"1.4.2\"\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: record"]);
}

// --- the diff ------------------------------------------------------------

#[test]
fn k1_the_diff_shows_the_version_moving_and_nothing_else_in_that_file() {
    let f = Fixture::new("cargo", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    released(&f);
    f.commit("feat: add export");

    let out = f.dry_run();
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let s = stdout(&out);
    assert!(s.contains("--- Cargo.toml"), "{s}");
    assert!(s.contains(r#"- version = "1.4.2"   # managed by cicada"#), "{s}");
    assert!(s.contains(r#"+ version = "1.5.0"   # managed by cicada"#), "{s}");
    // The changelog and versions file are part of the same release.
    assert!(s.contains("+++ new CHANGELOG.md"), "created, because there was none: {s}");
    // Modified rather than created: this fixture already has a versions file.
    assert!(s.contains("--- .cicada-versions.toml"), "{s}");
    assert!(s.contains(r#"- version = "1.4.2""#), "{s}");
    assert!(s.contains("3 files would be written"), "{s}");
}

#[test]
fn h5_a_dry_run_leaves_the_working_tree_exactly_as_it_found_it() {
    // The assertion that makes `--dry-run` worth having. #30's claim is that it is
    // the same computation with the write skipped — so the write must be skipped.
    let f = Fixture::new("clean", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");

    let before = f.dirty();
    let out = f.dry_run();
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(f.dirty(), before, "a dry run wrote something");
    assert!(!f.dir.join("CHANGELOG.md").exists(), "a dry run created CHANGELOG.md");
    assert!(!f.dir.join(".cicada-versions.toml").exists());
    assert_eq!(f.read("Cargo.toml"), CARGO);
}

#[test]
fn a5_an_empty_plan_writes_nothing_and_succeeds() {
    let f = Fixture::new("empty", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    released(&f);
    f.commit("chore: tidy");

    let out = f.dry_run();
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("nothing to release"), "{}", stdout(&out));
}

#[test]
fn h1_a_branch_outside_the_gate_exits_zero_and_says_why() {
    // The gate applies to every command that writes: a push to a feature branch
    // is not a failure, and a CI job must not go red for it (#26).
    //
    // But it must not be *silent*. Doing nothing without a word is
    // indistinguishable from a broken tool, and it is the first thing a human
    // running this by hand hits. The note goes to stderr, so H1's real assertion
    // — stdout stays empty, `--json` and pipelines unaffected — still holds.
    let f = Fixture::new("gate", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.git(&["checkout", "-b", "feature/foo"]);
    f.commit("feat: add export");

    for args in [
        vec!["release-pr", "--dry-run"],
        vec!["release-pr", "--no-push"],
        // The full form too: the gate is checked before the mode is.
        vec!["release-pr"],
    ] {
        let out = f.cicada(&args);
        assert_eq!(code(&out), 0, "{args:?}");
        assert_eq!(stdout(&out), "", "nothing on stdout: {args:?}");

        let e = stderr(&out);
        assert!(e.contains("feature/foo"), "it names the branch: {e}");
        assert!(e.contains("release_branches"), "and the setting: {e}");
        assert!(e.contains("cicada plan"), "and the way to rehearse anyway: {e}");
        // Nothing was written on the way to saying so.
        assert!(f.branch_tip("cicada/feature/foo").is_none());
    }
}

#[test]
fn quiet_suppresses_the_gate_note_and_v_still_logs_it() {
    let f = Fixture::new("gate-quiet", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.git(&["checkout", "-b", "feature/foo"]);
    f.commit("feat: add export");

    let quiet = f.cicada(&["release-pr", "--no-push", "--quiet"]);
    assert_eq!(code(&quiet), 0);
    assert_eq!(stdout(&quiet), "");
    assert_eq!(stderr(&quiet), "", "--quiet means quiet");

    let verbose = f.cicada(&["release-pr", "--no-push", "-v"]);
    assert!(stderr(&verbose).contains("not a release branch"), "{}", stderr(&verbose));
}

// --- #24 · the workspace-inheritance redirect ---------------------------

const WORKSPACE_CONFIG: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"
repo = "acme/app"

[packages.app]
path       = "app"
tag_format = "v{version}"
changelog  = "CHANGELOG.md"

[[packages.app.updaters]]
kind = "cargo"
path = "app/Cargo.toml"
"#;

#[test]
fn an_inheriting_crate_moves_the_number_in_the_workspace_root() {
    // cicada's own shape, and the reason it is here: the member manifest says
    // `version.workspace = true`, so the number lives in the *root* manifest — a
    // different file, found by walking up. The first build read only the declared
    // path and failed with "no value at the file itself".
    let f = Fixture::new("ws", WORKSPACE_CONFIG);
    f.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"app\"]\n\n[workspace.package]\nversion = \"1.4.2\"\nedition = \"2024\"\n",
    );
    f.write("app/Cargo.toml", "[package]\nname = \"app\"\nversion.workspace = true\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: workspace"]);
    f.git(&["tag", "v1.4.2"]);
    f.write(".cicada-versions.toml", "[packages.app]\nversion = \"1.4.2\"\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: record"]);
    f.commit("feat: add export");

    let out = f.dry_run();
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let s = stdout(&out);
    // The root manifest is the file that changes...
    assert!(s.contains("--- Cargo.toml"), "{s}");
    assert!(s.contains(r#"+ version = "1.5.0""#), "{s}");
    // ...and the member manifest is not touched, because it holds no number.
    assert!(!s.contains("app/Cargo.toml"), "the member manifest has no version to move:\n{s}");
    assert!(s.contains("edition = \"2024\"") || !s.contains("- edition"), "{s}");
}

// --- L6 · the printed default round-trips --------------------------------

#[test]
fn l6_printing_a_built_in_and_using_it_as_an_override_changes_nothing() {
    let f = Fixture::new("tmpl", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");

    let plain = stdout(&f.dry_run());

    let printed = f.cicada(&["template", "print", "notes"]);
    assert_eq!(code(&printed), 0);
    assert!(!stdout(&printed).is_empty());
    f.write(".cicada/templates/notes.md.j2", &stdout(&printed));

    // Picked up by convention, with no config change (#47).
    let overridden = stdout(&f.dry_run());
    assert_eq!(plain, overridden, "the printed default is the real default (#48)");
}

#[test]
fn an_override_actually_takes_effect_so_the_round_trip_is_not_vacuous() {
    let f = Fixture::new("tmpl2", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");
    f.write(".cicada/templates/notes.md.j2", "ENTIRELY DIFFERENT {{ version }}\n");

    assert!(stdout(&f.dry_run()).contains("ENTIRELY DIFFERENT 0.1.0"));
}

#[test]
fn template_print_names_the_alternatives_when_asked_for_one_that_does_not_exist() {
    let f = Fixture::new("tmpl3", SHAPE_1);
    let out = f.cicada(&["template", "print", "nope"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("notes"), "{}", stderr(&out));
}

// --- the push and the pull request are not here yet ----------------------

#[test]
fn a_full_run_says_what_is_missing_rather_than_half_doing_it() {
    let f = Fixture::new("m4", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");

    let out = f.cicada(&["release-pr"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("M4"), "{}", stderr(&out));
    // It names the branch `--no-push` would write, so the way forward is in the
    // message rather than in the docs.
    assert!(stderr(&out).contains("cicada/main"), "{}", stderr(&out));
    // And it created nothing on the way to saying so.
    assert!(f.branch_tip("cicada/main").is_none());
}

// --- determinism ---------------------------------------------------------

#[test]
fn the_same_inputs_produce_a_byte_identical_diff_across_processes() {
    let f = Fixture::new("det", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");
    f.commit("fix: handle nil");

    let first = stdout(&f.dry_run());
    assert!(!first.is_empty());
    for _ in 0..3 {
        assert_eq!(stdout(&f.dry_run()), first);
    }
}

// =========================================================================
// M3 · the git write path
//
// Exit criterion: `release-pr --no-push` produces a real local release commit,
// inspectable with `git show`. No token, no network, nothing pushed.
// =========================================================================

/// A fixture already at 1.4.2 with one unreleased `feat:` — the state almost
/// every write-path assertion starts from.
fn ready(name: &str) -> Fixture {
    let f = Fixture::new(name, SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    released(&f);
    f.commit("feat: add export");
    f
}

#[test]
fn h6_no_push_produces_a_real_local_release_commit() {
    let f = ready("nopush");
    let main_before = f.branch_tip("main").unwrap();

    let out = f.no_push();
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));

    let tip = f.branch_tip("cicada/main").expect("the release PR branch exists");

    // `git show cicada/main` displays the version bumps and the changelog entry.
    assert!(f.show(&tip, "Cargo.toml").contains(r#"version = "1.5.0""#));
    assert!(f.show(&tip, "CHANGELOG.md").contains("## 1.5.0 (2026-09-21)"));
    assert!(f.show(&tip, "CHANGELOG.md").contains("* add export"));
    assert!(f.show(&tip, ".cicada-versions.toml").contains(r#"version = "1.5.0""#));

    // Exactly one commit on top of main, which has not moved.
    assert_eq!(f.log_format(&tip, "%P"), main_before);
    assert_eq!(f.branch_tip("main").unwrap(), main_before);
    assert_eq!(f.count_commits(&tip), f.count_commits("main") + 1);
}

#[test]
fn the_working_tree_is_never_touched_even_though_a_commit_was_made() {
    // #100 — the whole reason the commit is built through a private index rather
    // than by checking the branch out. `--no-push` is the *local rehearsal* mode,
    // so a developer must get their tree back exactly as it was.
    let f = ready("untouched");
    let before = f.dirty();
    let head_before = f.branch_tip("main").unwrap();

    assert_eq!(code(&f.no_push()), 0);

    assert_eq!(f.dirty(), before, "the working tree changed");
    assert_eq!(f.git(&["rev-parse", "--abbrev-ref", "HEAD"]).trim(), "main", "still on main");
    assert_eq!(f.branch_tip("main").unwrap(), head_before);
    // The edits exist in the commit and *only* there.
    assert!(!f.dir.join("CHANGELOG.md").exists());
    assert_eq!(f.read("Cargo.toml"), CARGO);
}

#[test]
fn hash_55_the_commit_carries_the_configured_identity_as_author_and_committer() {
    // #32's guard reads `%cn <%ce>` back, so this is not cosmetic.
    let f = ready("identity");
    assert_eq!(code(&f.no_push()), 0);
    let tip = f.branch_tip("cicada/main").unwrap();

    assert_eq!(f.log_format(&tip, "%cn <%ce>"), "cicada[bot] <cicada@users.noreply.github.com>");
    assert_eq!(f.log_format(&tip, "%an <%ae>"), "cicada[bot] <cicada@users.noreply.github.com>");
    // The subject is conventional and non-releasable (#41, #36), so cicada's own
    // release commit cannot trigger a release on the next run.
    assert_eq!(f.log_format(&tip, "%s"), "chore(release): prepare release");
    assert!(f.log_format(&tip, "%b").contains("app 1.4.2 → 1.5.0"), "the body says what moved");
}

#[test]
fn a_configured_identity_is_used_instead_of_the_default() {
    let f = Fixture::new(
        "identity2",
        &SHAPE_1.replace(
            "default_scope    = \"app\"",
            "default_scope    = \"app\"\ncommitter        = \"Release Bot <bot@acme.test>\"",
        ),
    );
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");

    assert_eq!(code(&f.no_push()), 0);
    let tip = f.branch_tip("cicada/main").unwrap();
    assert_eq!(f.log_format(&tip, "%cn <%ce>"), "Release Bot <bot@acme.test>");
}

#[test]
fn a_committer_that_is_not_a_name_email_pair_is_a_config_finding() {
    let f = Fixture::new(
        "identity3",
        &SHAPE_1.replace(
            "default_scope    = \"app\"",
            "default_scope    = \"app\"\ncommitter        = \"just a name\"",
        ),
    );
    let out = f.cicada(&["check"]);
    assert_eq!(code(&out), 3);
    assert!(stdout(&out).contains("settings.committer"), "{}", stdout(&out));
}

#[test]
fn d1_the_branch_is_replaced_not_updated() {
    let f = ready("replace");
    assert_eq!(code(&f.no_push()), 0);
    let first = f.branch_tip("cicada/main").unwrap();
    assert!(f.show(&first, "CHANGELOG.md").contains("## 1.5.0"));

    // A breaking change lands on main; the next run recomputes from scratch.
    f.commit("feat!: drop the v1 endpoints");
    assert_eq!(code(&f.no_push()), 0);
    let second = f.branch_tip("cicada/main").unwrap();

    assert_ne!(first, second);
    assert!(f.show(&second, "CHANGELOG.md").contains("## 2.0.0"), "recomputed, not amended");
    // Still exactly one commit, parented on main — nothing was merged or rebased,
    // and the first release commit is unreachable from the second.
    assert_eq!(f.log_format(&second, "%P"), f.branch_tip("main").unwrap());
    assert_eq!(f.count_commits(&second), f.count_commits("main") + 1);
    let reachable = f.git(&["rev-list", &second]);
    assert!(!reachable.contains(&first), "the discarded commit must not be an ancestor");
}

#[test]
fn re_running_with_nothing_changed_creates_no_new_commit() {
    // Without the tree-and-parent check, the timestamp alone mints a fresh sha for
    // identical content on every run, and "did anything change?" stops being
    // answerable from the outside.
    let f = ready("idem");
    let out = f.no_push();
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("committed"), "{}", stdout(&out));
    let first = f.branch_tip("cicada/main").unwrap();

    let again = f.no_push();
    assert_eq!(code(&again), 0);
    // Asserted on the word as well as the sha: two runs inside one second would
    // produce the same sha anyway, so the sha alone is not evidence.
    assert!(stdout(&again).contains("unchanged"), "{}", stdout(&again));
    assert_eq!(f.branch_tip("cicada/main").unwrap(), first);
}

/// The `+` lines the diff printed for one file, reassembled into content.
///
/// For a file the run *creates*, every line is an addition — so this is the whole
/// file, and comparing it against the committed blob is a genuine byte-level
/// check rather than a substring spot-check.
fn added_lines(diff: &str, path: &str) -> String {
    let start = diff
        .find(&format!("+++ new {path}\n"))
        .unwrap_or_else(|| panic!("no creation section for {path} in:\n{diff}"));
    let body = &diff[start..];
    // Sections are separated by a blank line before the next header.
    let end = body[1..].find("\n---").map(|i| i + 1).unwrap_or(body.len());
    let end = body[1..].find("\n+++").map(|i| (i + 1).min(end)).unwrap_or(end);
    body[..end]
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("+ ").map(str::to_string))
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn h5_dry_run_and_no_push_agree_byte_for_byte() {
    // #30's actual guarantee, asserted the way the design asks: the two modes are
    // the same computation with the write skipped, so the bytes must match.
    let f = ready("agree");
    let dry = stdout(&f.dry_run());
    assert!(dry.contains("+ version = \"1.5.0\""), "{dry}");

    assert_eq!(code(&f.no_push()), 0);
    let tip = f.branch_tip("cicada/main").unwrap();

    // `CHANGELOG.md` is created, so the diff's additions are its entire content.
    let from_diff = added_lines(&dry, "CHANGELOG.md");
    assert!(from_diff.contains("## 1.5.0 (2026-09-21)"), "{from_diff}");
    assert_eq!(
        f.show(&tip, "CHANGELOG.md"),
        from_diff,
        "the diff and the commit disagree — `--dry-run` is describing a different computation"
    );
}

// --- #32 · refusing to clobber -------------------------------------------

#[test]
fn d2_a_human_commit_on_the_release_branch_is_refused_and_the_branch_is_untouched() {
    let f = ready("stale");
    assert_eq!(code(&f.no_push()), 0);

    // A human adds a commit on cicada's branch. It exists nowhere else, so
    // discarding it would lose it.
    f.git(&["checkout", "-q", "cicada/main"]);
    f.write("HOTFIX.txt", "by hand");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: tweak the release by hand"]);
    f.git(&["checkout", "-q", "main"]);
    let tip_before = f.branch_tip("cicada/main").unwrap();

    f.commit("fix: something else");
    let out = f.no_push();

    assert_eq!(code(&out), 4, "refused, not broken (#68): {}", stderr(&out));
    assert_eq!(f.branch_tip("cicada/main").unwrap(), tip_before, "the branch must be untouched");
    let e = stderr(&out);
    assert!(e.contains("cicada did not write"), "{e}");
    assert!(e.contains("exists nowhere else"), "it says why, not just that: {e}");
}

#[test]
fn d2_the_refusal_also_fires_under_dry_run() {
    // A rehearsal that reports success where the real run would refuse is a
    // rehearsal that lies.
    let f = ready("stale2");
    assert_eq!(code(&f.no_push()), 0);
    f.git(&["checkout", "-q", "cicada/main"]);
    f.write("HOTFIX.txt", "by hand");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: by hand"]);
    f.git(&["checkout", "-q", "main"]);
    f.commit("fix: something else");

    assert_eq!(code(&f.dry_run()), 4);
}

#[test]
fn d3_removing_the_human_commit_lets_the_next_run_proceed() {
    let f = ready("recover");
    assert_eq!(code(&f.no_push()), 0);
    let cicadas = f.branch_tip("cicada/main").unwrap();

    f.git(&["checkout", "-q", "cicada/main"]);
    f.write("HOTFIX.txt", "by hand");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: by hand"]);
    f.git(&["checkout", "-q", "main"]);
    assert_eq!(code(&f.no_push()), 4);

    // Reset the branch back to cicada's own commit and it recovers.
    f.git(&["branch", "-f", "cicada/main", &cicadas]);
    assert_eq!(code(&f.no_push()), 0);
}

// --- #80 / G4 · the collision check, in Phase A --------------------------

#[test]
fn g4_a_version_whose_tag_exists_unreachably_refuses_before_any_branch_is_written() {
    let f = ready("collide");
    // `v1.5.0` was cut on another line, so it is not reachable from main.
    f.git(&["checkout", "-q", "-b", "other"]);
    f.commit("feat: released over there");
    f.git(&["tag", "v1.5.0"]);
    f.git(&["checkout", "-q", "main"]);

    let out = f.no_push();
    assert_eq!(code(&out), 4, "stderr: {}", stderr(&out));
    let e = stderr(&out);
    assert!(e.contains("v1.5.0"), "{e}");
    assert!(e.contains("converging"), "{e}");
    // Discovering this in Phase B would mean the PR was already reviewed and
    // merged; here, no branch is written at all.
    assert!(f.branch_tip("cicada/main").is_none());
}

#[test]
fn a_tag_that_is_reachable_is_not_a_collision() {
    // The rule is *unreachable* (#80). A reachable tag is this branch's own
    // history, and refusing on it would refuse every ordinary release.
    let f = ready("noncollide");
    f.git(&["tag", "v1.4.2-something-else"]);
    assert_eq!(code(&f.no_push()), 0);
}

// --- G3 · the templated head branch --------------------------------------

#[test]
fn g3_a_slashed_branch_keeps_its_slash_in_the_rendered_head() {
    // `cicada/release/1.4`, not `cicada/release-1.4`: flattening would map
    // `release/1.4` and a hypothetical `release-1.4` onto one head branch (#27).
    let config = SHAPE_1.replace(r#"release_branches = ["main"]"#, r#"release_branches = ["main", "release/*"]"#);
    let f = Fixture::new("slash", &config);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.git(&["checkout", "-q", "-b", "release/1.4"]);
    f.commit("fix: handle nil");

    assert_eq!(code(&f.no_push()), 0);
    assert!(f.branch_tip("cicada/release/1.4").is_some());
    // And the rendered head is outside every release pattern, which is what stops
    // cicada's own push re-triggering it.
    assert!(f.branch_tip("cicada/main").is_none());
}

// --- file modes ----------------------------------------------------------

#[test]
fn an_executable_file_stays_executable_through_a_marker_edit() {
    // A Makefile or shell script carrying `# x-cicada-version` (#50) is exactly
    // the file most likely to be executable, and hard-coding 100644 in the index
    // would silently un-execute it.
    let config = format!(
        "{SHAPE_1}\n[[packages.app.updaters]]\nkind = \"generic\"\npath = \"release.sh\"\nedit = {{ marker = \"x-cicada-version\" }}\n"
    );
    let f = Fixture::new("mode", &config);
    f.write("Cargo.toml", CARGO);
    f.write("release.sh", "#!/bin/sh\nVERSION=1.4.2 # x-cicada-version\n");
    // Set the bit on disk rather than in the index: `released()` runs another
    // `git add -A`, which re-reads the mode from the file and would undo an
    // index-only `--chmod=+x`.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(f.dir.join("release.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: script"]);
    released(&f);
    f.commit("feat: add export");

    assert_eq!(code(&f.no_push()), 0);
    let tip = f.branch_tip("cicada/main").unwrap();
    assert!(f.show(&tip, "release.sh").contains("VERSION=1.5.0"));

    let entry = f.git(&["ls-tree", &tip, "--", "release.sh"]);
    assert!(entry.starts_with("100755"), "the executable bit was dropped: {entry}");
}

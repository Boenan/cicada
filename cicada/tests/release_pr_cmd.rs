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
fn h1_a_branch_outside_the_gate_exits_zero_silently() {
    // Unlike `plan`, this command writes — so the gate applies, and a non-matching
    // branch is not an error, a warning, or output (#26).
    let f = Fixture::new("gate", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.git(&["checkout", "-b", "feature/foo"]);
    f.commit("feat: add export");

    let out = f.dry_run();
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "", "nothing on stdout except at -v");
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

// --- the write path is not here yet --------------------------------------

#[test]
fn without_dry_run_the_command_says_what_is_missing_rather_than_half_doing_it() {
    let f = Fixture::new("m3", SHAPE_1);
    f.write("Cargo.toml", CARGO);
    f.git(&["add", "-A"]);
    f.git(&["commit", "-m", "chore: manifest"]);
    f.commit("feat: add export");

    let out = f.cicada(&["release-pr", "--no-push"]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("M3"), "{}", stderr(&out));
    // And it wrote nothing on the way to saying so.
    assert!(!f.dir.join("CHANGELOG.md").exists());
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

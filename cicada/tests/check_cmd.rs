//! `cicada check` end to end — M0's exit criterion.
//!
//! Exit codes are a contract (#68), so they are asserted rather than the output
//! alone: `0` clean, `3` findings, `1` a config that will not parse.

use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::prelude::*;

const GOOD_CONFIG: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"
repo = "acme/app"

[packages.app]
path       = "."
tag_format = "v{version}"
"#;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, config: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cicada-check-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = Fixture { dir };
        f.git(&["init", "--initial-branch=main"]);
        f.git(&["config", "user.email", "t@example.invalid"]);
        f.git(&["config", "user.name", "T"]);
        std::fs::write(f.dir.join("cicada.toml"), config).unwrap();
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

    fn commit(&self, subject: &str) {
        let n = self.git(&["rev-list", "--count", "--all"]).trim().to_string();
        std::fs::write(self.dir.join(format!("f{n}.txt")), subject).unwrap();
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", subject]);
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn check(&self, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("cicada")
            .unwrap()
            .arg("check")
            .args(args)
            .current_dir(&self.dir)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
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

#[test]
fn i1_clean_range_exits_zero() {
    let f = Fixture::new("i1", GOOD_CONFIG);
    let base = f.head();
    f.commit("feat(app): add login");
    f.commit("fix: handle nil");

    let out = f.check(&["--base", &base, "--head", "HEAD"]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    let s = stdout(&out);
    assert!(s.contains("✓ feat(app): add login"), "{s}");
    assert!(s.contains("✓ fix: handle nil"), "{s}");
}

#[test]
fn i2_non_conventional_commit_is_a_finding() {
    let f = Fixture::new("i2", GOOD_CONFIG);
    let base = f.head();
    f.commit("update deps");

    let out = f.check(&["--base", &base]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("not a conventional commit"), "{s}");
    assert!(s.contains("1 problem in 1 commit"), "{s}");
}

#[test]
fn i3_unknown_scope_suggests_the_right_package() {
    // The did-you-mean is what turns #11's hard error from an obstacle into help.
    let f = Fixture::new("i3", GOOD_CONFIG);
    let base = f.head();
    f.commit("feat(aip): tune pool size");

    let out = f.check(&["--base", &base]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("unknown scope 'aip'"), "{s}");
    assert!(s.contains("did you mean 'app'?"), "{s}");
}

#[test]
fn a_scope_with_no_near_match_lists_the_alternatives_instead_of_guessing() {
    let f = Fixture::new("nomatch", GOOD_CONFIG);
    let base = f.head();
    f.commit("feat(database): add index");

    let out = f.check(&["--base", &base]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("unknown scope 'database'"), "{s}");
    assert!(s.contains("known scopes: app"), "{s}");
    assert!(!s.contains("did you mean"), "a misleading suggestion is worse than none: {s}");
}

#[test]
fn i4_missing_package_path_is_a_finding() {
    let cfg = GOOD_CONFIG.replace(r#"path       = ".""#, r#"path       = "services/nope""#);
    let f = Fixture::new("i4", &cfg);
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    assert!(stdout(&out).contains("does not exist in the repository"), "{}", stdout(&out));
}

#[test]
fn i6_default_scope_naming_nothing_is_a_finding() {
    let cfg = GOOD_CONFIG.replace(r#"default_scope    = "app""#, r#"default_scope    = "platform""#);
    let f = Fixture::new("i6", &cfg);
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("names no package or umbrella"), "{s}");
    assert!(s.contains("valid targets: app"), "{s}");
}

#[test]
fn i6b_release_pr_branch_inside_the_allowlist_is_a_finding() {
    // #27: the prefix is a namespace, not a label. `release/{branch}` renders
    // `release/main`, which matches `release/*` — so cicada's own push would
    // re-trigger it.
    let cfg = GOOD_CONFIG.replace(
        r#"release_branches = ["main"]"#,
        "release_branches  = [\"main\", \"release/*\"]\nrelease_pr_branch = \"release/{branch}\"",
    );
    let f = Fixture::new("i6b", &cfg);
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("renders to `release/main`"), "{s}");
}

#[test]
fn i7_unknown_config_key_is_exit_one_not_a_finding() {
    // #67 and #79: a config that will not parse is `1`. It is not a "finding" —
    // cicada could not load its own configuration.
    let cfg = GOOD_CONFIG.replace("tag_format = ", "tag_fromat = ");
    let f = Fixture::new("i7", &cfg);
    let out = f.check(&[]);
    assert_eq!(code(&out), 1);
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("tag_fromat"), "the error must name the key: {err}");
}

#[test]
fn i8_malformed_toml_is_exit_one() {
    let f = Fixture::new("i8", "[settings\nrelease_branches = [");
    let out = f.check(&[]);
    assert_eq!(code(&out), 1);
}

#[test]
fn a_helm_package_without_initial_version_is_a_finding() {
    // #88: Helm mandates `version` in Chart.yaml, so the number already exists
    // and cicada must not invent one.
    let cfg = format!(
        "{GOOD_CONFIG}\n[[packages.app.updaters]]\nkind = \"helm\"\npath = \"Chart.yaml\"\n"
    );
    let f = Fixture::new("helm", &cfg);
    std::fs::write(f.path().join("Chart.yaml"), "name: app\nversion: 0.1.0\n").unwrap();
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    assert!(stdout(&out).contains("required on a package with a `helm` updater"), "{}", stdout(&out));
}

#[test]
fn duplicate_tag_formats_are_a_finding() {
    let cfg = format!(
        "{GOOD_CONFIG}\n[packages.worker]\npath = \".\"\ntag_format = \"v{{version}}\"\n"
    );
    let f = Fixture::new("dupfmt", &cfg);
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("duplicate tag format"), "{s}");
    assert!(s.contains("duplicate path"), "{s}");
}

#[test]
fn a_release_as_footer_in_history_is_ignored_not_rejected() {
    // #39: forced versions left v1. A repo migrating from release-please carries
    // these footers, so they must be inert — neither honoured nor an error.
    let f = Fixture::new("releaseas", GOOD_CONFIG);
    let base = f.head();
    f.git(&["commit", "--allow-empty", "-m", "feat(app): rewrite storage", "-m", "Release-As: 2.0.0"]);

    let out = f.check(&["--base", &base]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
}

#[test]
fn json_output_carries_a_schema_version() {
    // #69: a machine interface with a compatibility promise.
    let f = Fixture::new("json", GOOD_CONFIG);
    let base = f.head();
    f.commit("update deps");

    let out = f.check(&["--base", &base, "--json"]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains(r#""schema":1"#), "{s}");
    assert!(s.contains(r#""ok":false"#), "{s}");
    assert!(s.contains("not a conventional commit"), "{s}");
}

#[test]
fn config_only_check_needs_no_range() {
    let f = Fixture::new("configonly", GOOD_CONFIG);
    let out = f.check(&[]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
}

#[test]
fn the_workspace_inheritance_trap_is_reported() {
    // #24: two crates with `version.workspace = true` resolve to one field and
    // cannot be bumped independently.
    let cfg = r#"
[settings]
release_branches = ["main"]
default_scope    = "api"

[forge]
kind = "github"

[packages.api]
path       = "services/api"
tag_format = "api-v{version}"
[[packages.api.updaters]]
kind = "cargo"
path = "services/api/Cargo.toml"

[packages.worker]
path       = "services/worker"
tag_format = "worker-v{version}"
[[packages.worker.updaters]]
kind = "cargo"
path = "services/worker/Cargo.toml"
"#;
    let f = Fixture::new("wsinherit", cfg);
    for p in ["services/api", "services/worker"] {
        std::fs::create_dir_all(f.path().join(p)).unwrap();
        std::fs::write(
            f.path().join(p).join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion.workspace = true\n",
        )
        .unwrap();
    }
    let out = f.check(&[]);
    assert_eq!(code(&out), 3);
    let s = stdout(&out);
    assert!(s.contains("resolve to the same Cargo version field"), "{s}");
    assert!(s.contains("'api'") && s.contains("'worker'"), "must name both: {s}");
}

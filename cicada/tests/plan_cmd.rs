//! `cicada plan` end to end — M1's exit criterion.
//!
//! The criterion is specific: *correct on a fixture **monorepo and a
//! single-package repo through the same code path**, fully offline*. So both
//! shapes are here, and both are driven through the real binary against a real
//! git repository — anchor resolution, reachability and per-package ranges are
//! precisely the parts a pure test cannot reach.
//!
//! Offline is asserted structurally where it can be: no token is ever set, and
//! `plan` has no forge code to reach for.

use std::path::PathBuf;
use std::process::Command;

use assert_cmd::prelude::*;

// --- fixtures ------------------------------------------------------------

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
"#;

/// Deliberately not alphabetical, so config order is distinguishable from a sort
/// (O6). No `updaters` on any package: M1 plans, it does not write files.
const MONOREPO: &str = r#"
[settings]
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]
default_scope        = "platform"

[forge]
kind = "github"
repo = "acme/platform"

[umbrella]
name       = "platform"
tag_format = "platform-v{version}"
changelog  = "CHANGELOG.md"

[packages.api]
path       = "services/api"
tag_format = "api-v{version}"

[packages.worker]
path       = "services/worker"
tag_format = "worker-v{version}"

[packages.auth]
path       = "libs/auth"
tag_format = "auth-v{version}"
"#;

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str, config: &str, dirs: &[&str]) -> Self {
        // Tests share a process, so the pid alone is not unique — two fixtures
        // with the same name would land in one directory and race.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("cicada-plan-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = Fixture { dir };
        f.git(&["init", "--initial-branch=main"]);
        f.git(&["config", "user.email", "t@example.invalid"]);
        f.git(&["config", "user.name", "T"]);
        for d in dirs {
            std::fs::create_dir_all(f.dir.join(d)).unwrap();
            std::fs::write(f.dir.join(d).join(".keep"), "").unwrap();
        }
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

    fn versions(&self, body: &str) {
        std::fs::write(self.dir.join(".cicada-versions.toml"), body).unwrap();
    }

    fn plan(&self, args: &[&str]) -> std::process::Output {
        Command::cargo_bin("cicada")
            .unwrap()
            .arg("plan")
            .args(args)
            .current_dir(&self.dir)
            .env("NO_COLOR", "1")
            // Offline is a stated goal (§Goals 6): nothing here supplies a token,
            // and a `plan` that needed one would fail rather than quietly work.
            .env_remove("CICADA_TOKEN")
            .output()
            .unwrap()
    }

    /// `plan --json`, parsed into the handful of fields the scenarios assert.
    fn json(&self, args: &[&str]) -> Json {
        let out = self.plan(&[&["--json"], args].concat());
        assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
        Json::parse(&stdout(&out))
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

/// A deliberately tiny reader for the one payload shape this suite asserts.
///
/// Pulling in a JSON parser to test a hand-rolled emitter would mostly test the
/// parser; what matters here is that the **documented fields** carry the
/// documented values, in order (#69).
struct Json {
    raw: String,
}

struct Rel {
    package: String,
    from: Option<String>,
    to: String,
    bump: String,
    reason: String,
    tag: String,
}

impl Json {
    fn parse(raw: &str) -> Self {
        Json { raw: raw.trim().to_string() }
    }

    fn field(&self, key: &str) -> String {
        let pat = format!("\"{key}\":\"");
        let i = self.raw.find(&pat).unwrap_or_else(|| panic!("no `{key}` in {}", self.raw));
        let rest = &self.raw[i + pat.len()..];
        rest[..rest.find('"').unwrap()].to_string()
    }

    /// Releases in emitted order — which is the assertion, not an incidental.
    fn releases(&self) -> Vec<Rel> {
        let start = self.raw.find("\"releases\":[").expect("releases key") + "\"releases\":[".len();
        let body = &self.raw[start..self.raw.rfind(']').unwrap()];
        body.split("{\"package\":\"")
            .skip(1)
            .map(|chunk| {
                let take = |key: &str| -> String {
                    let pat = format!("\"{key}\":");
                    let i = chunk.find(&pat).unwrap() + pat.len();
                    let rest = &chunk[i..];
                    if let Some(r) = rest.strip_prefix('"') {
                        r[..r.find('"').unwrap()].to_string()
                    } else {
                        rest[..rest.find([',', '}']).unwrap()].to_string()
                    }
                };
                let from = take("from");
                Rel {
                    package: chunk[..chunk.find('"').unwrap()].to_string(),
                    from: if from == "null" { None } else { Some(from) },
                    to: take("to"),
                    bump: take("bump"),
                    reason: take("reason"),
                    tag: take("tag"),
                }
            })
            .collect()
    }
}

fn shape_1() -> Fixture {
    Fixture::new("shape1", SHAPE_1, &[])
}

fn monorepo() -> Fixture {
    Fixture::new("mono", MONOREPO, &["services/api", "services/worker", "libs/auth"])
}

// --- the single-package repo ---------------------------------------------

#[test]
fn a1_a_first_release_reports_a_null_from() {
    let f = shape_1();
    f.commit("feat: add login");

    let j = f.json(&[]);
    let rs = j.releases();
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].package, "app");
    // Present and explicit (#89): `"0.0.0"` names a release that never existed,
    // and omitting the key is ambiguous with an older schema.
    assert_eq!(rs[0].from, None);
    assert_eq!(rs[0].to, "0.1.0");
    assert_eq!(rs[0].bump, "minor");
    assert_eq!(rs[0].reason, "first-release");
    assert_eq!(rs[0].tag, "v0.1.0");
    assert!(j.raw.contains("\"from\":null"), "literal null, not a string: {}", j.raw);
}

#[test]
fn a8_a_first_release_covers_all_reachable_history() {
    // Surprising to adopters, and asserted for that reason: the range is
    // unanchored, so every conventional commit in the repo is in it.
    let f = shape_1();
    for i in 0..5 {
        f.commit(&format!("fix: thing {i}"));
    }
    f.commit("feat: add login");

    let j = f.json(&[]);
    let rs = j.releases();
    assert_eq!(rs[0].to, "0.1.0");
    // Five fixes, the feat, *and* the fixture's `chore: init` — seven. A release
    // carries every commit routed to it, not only the ones that moved the number:
    // `bump` and *renders in the changelog* are two questions (#36), and `docs` is
    // `bump = "none"` while still having a section title. Dropping non-releasable
    // commits here would silently empty the Documentation section.
    assert_eq!(j.raw.matches("\"sha\":").count(), 7);
}

#[test]
fn b2_an_anchored_range_starts_at_the_tag() {
    let f = shape_1();
    f.commit("feat: released already");
    f.git(&["tag", "v1.4.2"]);
    f.versions("[packages.app]\nversion = \"1.4.2\"\n");
    f.commit("feat: add export");
    f.commit("fix: handle nil");

    let j = f.json(&[]);
    let rs = j.releases();
    assert_eq!((rs[0].from.as_deref(), rs[0].to.as_str()), (Some("1.4.2"), "1.5.0"));
    // The already-released `feat` is behind the anchor and is not counted again.
    assert_eq!(j.raw.matches("\"sha\":").count(), 2);
}

#[test]
fn a5_nothing_to_release_is_exit_zero_and_says_so() {
    let f = shape_1();
    f.commit("chore: tidy");
    f.commit("docs: readme");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 0, "an empty plan is a normal outcome, not a failure (#68)");
    assert!(stdout(&out).contains("nothing to release"), "{}", stdout(&out));

    let j = f.json(&[]);
    assert!(j.releases().is_empty());
}

#[test]
fn j1_a_missing_anchor_tag_is_a_hard_error_naming_the_repair() {
    // No SHA fallback (#2): a resolvable-but-wrong range produces a changelog that
    // looks fine and is incomplete, which is the failure mode the tool exists to
    // avoid.
    let f = shape_1();
    f.versions("[packages.app]\nversion = \"1.4.2\"\n");
    f.commit("fix: handle nil");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("v1.4.2"), "{e}");
    assert!(e.contains("git tag v1.4.2"), "the repair is spelled out: {e}");
}

#[test]
fn j2_a_tag_that_exists_elsewhere_is_a_hard_error() {
    // The same primitive as the collision check (#9), reported from the other end.
    let f = shape_1();
    f.commit("feat: base");
    f.git(&["checkout", "-b", "other"]);
    f.commit("feat: on the other line");
    f.git(&["tag", "v1.4.2"]);
    f.git(&["checkout", "main"]);
    f.versions("[packages.app]\nversion = \"1.4.2\"\n");
    f.commit("fix: handle nil");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("not reachable"), "{e}");
    assert!(e.contains("v1.4.2"), "{e}");
}

#[test]
fn b13_an_unknown_scope_hard_errors_where_check_only_reported_it() {
    // The same problem at two exit codes: `check` finds it at review time (`3`),
    // `plan` refuses to compute (`1`). Both name the scope.
    let f = shape_1();
    f.commit("feat(parser): tune the pool");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("unknown scope 'parser'"), "{}", stderr(&out));
}

#[test]
fn b9_a_dependency_bot_scope_does_not_block_the_repo() {
    // `chore(deps):` is what Dependabot writes and what the design uses as its
    // canonical non-releasable example — it must not be a hard error.
    let f = shape_1();
    f.commit("chore(deps): bump serde");
    let out = f.plan(&[]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("nothing to release"));

    f.commit("feat: add export");
    let j = f.json(&[]);
    assert_eq!(j.releases()[0].to, "0.1.0");
}

// --- the maintenance guard, on a real branch -----------------------------

#[test]
fn b11_the_guard_exits_four_on_a_real_maintenance_branch() {
    let f = Fixture::new("guard", MONOREPO, &["services/api", "services/worker", "libs/auth"]);
    f.commit("feat(api): base");
    f.git(&["tag", "api-v1.4.7"]);
    f.versions("[packages.api]\nversion = \"1.4.7\"\n");
    f.git(&["checkout", "-b", "release/1.4"]);
    f.commit("feat(api): cherry-picked by mistake");

    let out = f.plan(&[]);
    // Refused, not broken (#68) — a pipeline may handle the two differently.
    assert_eq!(code(&out), 4, "stderr: {}", stderr(&out));
    let e = stderr(&out);
    assert!(e.contains("release/1.4"), "{e}");
    assert!(e.contains("cherry-picked by mistake"), "it names the offending commit: {e}");
    assert!(e.contains("no override"), "G2b — the message states the way forward: {e}");
}

#[test]
fn b15_the_same_branch_releases_a_patch_without_complaint() {
    let f = Fixture::new("guardok", MONOREPO, &["services/api", "services/worker", "libs/auth"]);
    f.commit("feat(api): base");
    f.git(&["tag", "api-v1.4.7"]);
    f.versions("[packages.api]\nversion = \"1.4.7\"\n");
    f.git(&["checkout", "-b", "release/1.4"]);
    f.commit("fix(api): handle nil");

    let j = f.json(&[]);
    let rs = j.releases();
    assert_eq!(rs[0].package, "api");
    assert_eq!(rs[0].to, "1.4.8");
    assert_eq!(j.field("branch"), "release/1.4");
}

// --- the monorepo, through the same code path ----------------------------

#[test]
fn o6_p1_per_package_anchors_and_config_order() {
    let f = monorepo();
    f.commit("feat(api): released already");
    f.git(&["tag", "api-v1.4.2"]);
    f.commit("feat(worker): released already");
    f.git(&["tag", "worker-v0.9.1"]);
    f.git(&["tag", "platform-v1.7.0"]);
    f.versions(
        r#"
        [umbrella]
        version = "1.7.0"
        [packages.api]
        version = "1.4.2"
        [packages.worker]
        version = "0.9.1"
        "#,
    );

    f.commit("fix(api): handle nil");
    f.commit("feat(worker): parallel drain");
    f.commit("feat(auth): add token rotation");

    let j = f.json(&[]);
    let rs = j.releases();
    let names: Vec<&str> = rs.iter().map(|r| r.package.as_str()).collect();
    // Config order, umbrella last (#31, #96). Alphabetical would be
    // api, auth, platform, worker.
    assert_eq!(names, vec!["api", "worker", "auth", "platform"]);

    assert_eq!((rs[0].from.as_deref(), rs[0].to.as_str()), (Some("1.4.2"), "1.4.3"));
    assert_eq!((rs[1].from.as_deref(), rs[1].to.as_str()), (Some("0.9.1"), "0.10.0"));
    // `auth` has never released, so its range is all reachable history (P3).
    assert_eq!((rs[2].from.as_deref(), rs[2].to.as_str()), (None, "0.1.0"));
    assert_eq!(rs[2].reason, "first-release");
    // The umbrella aggregates the highest child bump onto its own line (#10).
    assert_eq!((rs[3].from.as_deref(), rs[3].to.as_str()), (Some("1.7.0"), "1.8.0"));
    assert_eq!(rs[3].reason, "aggregate");
    assert_eq!(rs[3].tag, "platform-v1.8.0");
}

#[test]
fn p1_each_package_is_anchored_at_its_own_tag_not_the_newest_one() {
    // The assertion that per-package ranges are real: `api` released *after*
    // worker's last release, so a shared anchor would hide worker's commit.
    let f = monorepo();
    f.commit("feat(worker): released already");
    f.git(&["tag", "worker-v0.9.1"]);
    f.commit("feat(worker): parallel drain");
    f.commit("feat(api): add caching");
    f.git(&["tag", "api-v1.5.0"]);
    f.commit("fix(api): handle nil");
    f.versions(
        r#"
        [packages.api]
        version = "1.5.0"
        [packages.worker]
        version = "0.9.1"
        "#,
    );

    let j = f.json(&[]);
    let rs = j.releases();
    // api's range is api-v1.5.0..HEAD — one fix.
    assert_eq!(rs[0].package, "api");
    assert_eq!(rs[0].to, "1.5.1");
    // worker's range is worker-v0.9.1..HEAD, which *spans* the api release
    // commit — the range decides what is read, the scope what is attributed (P2).
    assert_eq!(rs[1].package, "worker");
    assert_eq!(rs[1].to, "0.10.0");

    // `platform` is declared as an umbrella and has never released, so it takes
    // the highest child bump onto a first release (#10, #37) — minor, from
    // worker. Three releases, not two: `auth` earns nothing, the umbrella does.
    assert_eq!(rs.len(), 3, "{:?}", rs.iter().map(|r| &r.package).collect::<Vec<_>>());
    assert_eq!(rs[2].package, "platform");
    assert_eq!((rs[2].from.as_deref(), rs[2].to.as_str()), (None, "0.1.0"));
    assert_eq!(rs[2].reason, "first-release");
}

#[test]
fn o5_one_bad_scope_refuses_the_whole_run() {
    let f = monorepo();
    f.commit("feat(api): good");
    f.commit("feat(api,wrker): bad");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 1);
    let e = stderr(&out);
    assert!(e.contains("wrker"), "{e}");
    assert!(e.contains("did you mean 'worker'?"), "{e}");
    assert!(!stdout(&out).contains("api"), "no partial plan is emitted: {}", stdout(&out));
}

// --- the CLI contract ----------------------------------------------------

#[test]
fn the_json_payload_carries_its_schema_and_its_date() {
    let f = shape_1();
    f.commit("feat: add login");

    // The date is an input, not a clock read (#95), so a plan is reproducible
    // from its own output.
    let j = f.json(&["--date", "2026-09-21"]);
    assert!(j.raw.starts_with("{\"schema\":1,"), "{}", j.raw);
    assert_eq!(j.field("date"), "2026-09-21");
    assert_eq!(j.field("branch"), "main");
}

#[test]
fn determinism_is_observable_across_processes() {
    // Goal 7, and the reason ordered maps are a constraint (#31): iteration order
    // of a `HashMap` is randomised per process, so this is the test that would
    // catch one creeping in.
    let f = monorepo();
    f.commit("feat(api): a");
    f.commit("fix(worker): b");
    f.commit("feat(auth): c");

    let first = stdout(&f.plan(&["--json", "--date", "2026-09-21"]));
    for _ in 0..3 {
        assert_eq!(stdout(&f.plan(&["--json", "--date", "2026-09-21"])), first);
    }
}

#[test]
fn h1_plan_answers_on_a_branch_outside_the_gate_and_says_that_it_is_outside() {
    // `plan` is a question, not a release: gating it would make the rehearsal
    // useless exactly where it is wanted. But `run` here would exit 0 without
    // acting (#26), and discovering that from a silent pipeline is worse.
    let f = shape_1();
    f.git(&["checkout", "-b", "feature/foo"]);
    f.commit("feat: add login");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("0.1.0"), "{}", stdout(&out));
    assert!(stderr(&out).contains("would exit 0 here"), "{}", stderr(&out));

    let j = f.json(&[]);
    assert!(j.raw.contains("\"release_branch\":false"), "{}", j.raw);
}

#[test]
fn i4_a_config_problem_hard_errors_rather_than_reporting_a_finding() {
    // `check` reports these at exit `3`; every other command cannot proceed and
    // exits `1` (I4–I6c).
    let f = Fixture::new("badcfg", SHAPE_1, &[]);
    std::fs::write(
        f.dir.join("cicada.toml"),
        SHAPE_1.replace("default_scope    = \"app\"", "default_scope    = \"nope\""),
    )
    .unwrap();
    f.commit("feat: add login");

    let out = f.plan(&[]);
    assert_eq!(code(&out), 1);
    assert!(stderr(&out).contains("names no package"), "{}", stderr(&out));
}

#[test]
fn f8_plan_on_a_prerelease_branch_reports_the_rc_version_and_the_skipped_changelog() {
    // The default `prerelease_branches` pattern picks this up with no config
    // stanza at all, and `prerelease_identifier` defaults to `rc` (#83).
    let f = shape_1();
    f.git(&["checkout", "-b", "prerelease-v1.5-rc"]);
    f.commit("feat: add login");

    let j = f.json(&[]);
    let rs = j.releases();
    assert_eq!(rs[0].to, "0.1.0-rc.1");
    assert_eq!(rs[0].tag, "v0.1.0-rc.1");

    // F8 — it says the changelog will not be written, because a silently skipped
    // file write reads as a bug.
    let out = f.plan(&[]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("CHANGELOG.md is not"), "{}", stdout(&out));
}

#[test]
fn f6_the_ad_hoc_flag_cuts_a_prerelease_from_an_ordinary_branch() {
    let f = shape_1();
    f.commit("feat: add login");
    assert_eq!(f.json(&["--prerelease", "beta"]).releases()[0].to, "0.1.0-beta.1");
}


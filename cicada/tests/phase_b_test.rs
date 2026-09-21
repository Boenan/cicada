//! Phase B — M5's exit criterion.
//!
//! *merge → tag → release; re-run is a no-op; a simulated failed release call
//! self-heals on the next run; a maintenance branch releases independently of
//! `main`.*
//!
//! Driven against a **real git repository** and the **recording fake** forge, so
//! the assertions can be about the ordered list of calls — including the ones
//! that must not happen (C3).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use cicada::forge::fake::FakeForge;
use cicada::git::GitCli;
use cicada::phase_b::{self, PhaseBError, PhaseBInput, TagState};
use cicada_core::config::Config;
use cicada_core::notes::LinkStyle;
use cicada_core::versions::VersionsFile;
use cicada_core::Identity;

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
"#;

struct Repo {
    dir: PathBuf,
}

impl Repo {
    fn new(name: &str, dirs: &[&str]) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cicada-pb-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Repo { dir };
        r.git(&["init", "--initial-branch=main"]);
        r.git(&["config", "user.email", "t@example.invalid"]);
        r.git(&["config", "user.name", "T"]);
        for d in dirs {
            std::fs::create_dir_all(r.dir.join(d)).unwrap();
        }
        r.commit("chore: init");
        r
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

    fn tag_exists(&self, tag: &str) -> bool {
        Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", &format!("refs/tags/{tag}")])
            .current_dir(&self.dir)
            .output()
            .unwrap()
            .status
            .success()
    }

    fn tag_sha(&self, tag: &str) -> String {
        self.git(&["rev-parse", &format!("{tag}^{{commit}}")]).trim().to_string()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn bot() -> Identity {
    Identity::parse("cicada[bot] <cicada@users.noreply.github.com>").unwrap()
}

/// Run Phase B with no push and a fake forge — the shape every test below uses.
fn run(
    repo: &Repo,
    config: &str,
    versions: &str,
    forge: &FakeForge,
) -> Result<phase_b::Outcome, PhaseBError> {
    let cfg = Config::from_toml(config).expect("config parses");
    let v = VersionsFile::from_toml(versions).expect("versions parse");
    let git = GitCli::discover(&repo.dir).unwrap();
    let overrides = BTreeMap::new();
    let links = LinkStyle::default();
    let id = bot();
    phase_b::run(
        &git,
        &PhaseBInput {
            config: &cfg,
            versions: &v,
            date: "2026-09-21",
            links: &links,
            identity: &id,
            overrides: &overrides,
            push: None,
            forge: Some(forge),
        },
    )
}

// --- C2 · merge → tag → release ------------------------------------------

#[test]
fn c2_a_merged_release_is_tagged_and_published() {
    let r = Repo::new("c2", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");
    // The release PR has been merged: the versions file now names 1.5.0, and the
    // tag does not exist yet.
    r.commit("chore(release): prepare release");
    let head = r.head();

    let f = FakeForge::new();
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap();

    assert_eq!(out.tagged, vec![("app".to_string(), "v1.5.0".to_string())]);
    assert!(r.tag_exists("v1.5.0"));
    assert_eq!(r.tag_sha("v1.5.0"), head, "the tag lands on HEAD");
    assert_eq!(out.released, vec![("app".to_string(), "v1.5.0".to_string())]);

    // Tag first, then the release: a release object for a tag that does not exist
    // is not a thing either forge will accept.
    assert_eq!(f.call_names(), vec!["find_release", "create_release"]);

    let cicada::forge::fake::Call::CreateRelease(req) = &f.calls()[1] else { panic!() };
    assert_eq!(req.tag, "v1.5.0");
    assert_eq!(req.target, head);
    assert!(!req.prerelease);
    // The notes are the same content the changelog entry carries (#43).
    assert!(req.body.contains("add export"), "{}", req.body);
    // And the manifest rides along, collapsed (#42, #57).
    assert!(req.body.contains("<summary>Release manifest</summary>"), "{}", req.body);
    let json = req.body.split("```json").nth(1).unwrap().split("```").next().unwrap();
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["releases"][0]["tag"], "v1.5.0");
    assert_eq!(v["releases"][0]["sha"], head);
}

// --- C3 · a re-run is a total no-op --------------------------------------

#[test]
fn c3_re_running_changes_nothing_at_all() {
    let r = Repo::new("c3", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");
    r.commit("chore(release): prepare release");

    let f = FakeForge::new();
    run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap();
    let tag_sha = r.tag_sha("v1.5.0");

    // Second run: everything is already in place.
    let f2 = FakeForge::new();
    // The fake starts empty, so seed it with the release the first run created.
    f2.create_release_for("v1.5.0");
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f2).unwrap();

    assert!(out.is_no_op(), "{out:?}");
    assert_eq!(out.already_tagged, vec!["v1.5.0".to_string()]);
    assert_eq!(r.tag_sha("v1.5.0"), tag_sha, "the tag was not recreated");
    // One read, no writes. `create_release_for` is the seeding call, not cicada's.
    assert_eq!(out.released.len(), 0);
    assert!(
        f2.calls().iter().skip(1).all(|c| !c.is_mutating()),
        "{:?}",
        f2.call_names()
    );
}

// --- C4 · self-healing (#56) ---------------------------------------------

#[test]
fn c4_a_tag_with_no_release_object_creates_only_the_release() {
    // The shape produced by a `create_release` that 500s after the tag push.
    // Skipping the package on "tag exists" would strand the release forever,
    // which is why the two artifacts reconcile independently.
    let r = Repo::new("c4", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");
    r.commit("chore(release): prepare release");
    r.git(&["tag", "v1.5.0"]);
    let before = r.tag_sha("v1.5.0");

    let f = FakeForge::new();
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap();

    assert!(out.tagged.is_empty(), "no tag was created");
    assert_eq!(out.already_tagged, vec!["v1.5.0".to_string()]);
    assert_eq!(out.released, vec![("app".to_string(), "v1.5.0".to_string())]);
    assert_eq!(r.tag_sha("v1.5.0"), before, "the existing tag is untouched");
    assert_eq!(f.call_names(), vec!["find_release", "create_release"]);
}

// --- D4 / C5 · the stale-merge guard (#33 layer 3) -----------------------

#[test]
fn d4_a_release_pr_merged_while_stale_refuses_and_tags_nothing() {
    let r = Repo::new("d4", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("fix: small");
    r.commit("chore(release): prepare release"); // proposed 1.4.3
    // Someone pushed a breaking change and merged the stale PR anyway.
    r.commit("feat!: drop the v1 endpoints");

    let f = FakeForge::new();
    let err = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.4.3\"\n", &f).unwrap_err();

    let PhaseBError::StaleMerge { recorded, expected, .. } = &err else { panic!("{err:?}") };
    assert_eq!(recorded, "1.4.3");
    assert_eq!(expected, "2.0.0");
    // A visible unreleased state beats an invisible mis-versioned release.
    assert!(!r.tag_exists("v1.4.3"), "no tag was created");
    assert!(f.mutating_calls().is_empty(), "and nothing was published");
    // The error names both numbers and the repair.
    let s = err.to_string();
    assert!(s.contains("1.4.3") && s.contains("2.0.0"), "{s}");
    assert!(s.contains(".cicada-versions.toml"), "{s}");
}

#[test]
fn d4b_the_guard_is_one_sided_so_an_over_bump_is_allowed() {
    // A human hand-editing the versions file upwards to cut a release by hand is
    // a supported workflow (J4). Under-bumping is the harm; over-bumping is not.
    let r = Repo::new("d4b", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("fix: small");

    let f = FakeForge::new();
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"3.0.0\"\n", &f).unwrap();
    assert_eq!(out.tagged, vec![("app".to_string(), "v3.0.0".to_string())]);
    assert!(r.tag_exists("v3.0.0"));
}

#[test]
fn j4b_editing_the_versions_file_downwards_is_refused() {
    // J4b claims the staleness guard catches this "with no extra machinery". It
    // does not, and cannot: #90 picks `prev` from tags *below* the recorded
    // version, so editing 1.4.2 down to 1.0.0 leaves no eligible `prev`, the
    // recomputation becomes a *first release*, and 1.0.0 comfortably clears it.
    //
    // Monotonicity is therefore checked directly — which is what #76 says anyway.
    let r = Repo::new("j4b", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");

    let f = FakeForge::new();
    let err = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.0.0\"\n", &f).unwrap_err();
    let PhaseBError::WentBackwards { recorded, published, .. } = &err else { panic!("{err:?}") };
    assert_eq!(recorded, "1.0.0");
    assert_eq!(published, "1.4.2");
    assert!(!r.tag_exists("v1.0.0"));
    assert!(f.mutating_calls().is_empty());
}

#[test]
fn the_monotonicity_check_does_not_reach_across_to_another_line() {
    // Reachability is what keeps it safe on parallel lines: `release/1.4` records
    // 1.4.8 while `main` has tagged v2.0.0, and that tag is not this line's
    // business (G6, G10).
    let r = Repo::new("mono-lines", &[]);
    r.commit("feat: base");
    r.git(&["tag", "v1.4.7"]);
    let cut = r.head();
    r.commit("feat!: main moves on");
    r.git(&["tag", "v2.0.0"]);

    r.git(&["checkout", "-q", "-b", "release/1.4", &cut]);
    r.commit("fix(app): handle nil");

    let f = FakeForge::new();
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.4.8\"\n", &f).unwrap();
    assert_eq!(out.tagged, vec![("app".to_string(), "v1.4.8".to_string())]);
}

#[test]
fn hash_78_the_guard_does_not_fire_on_an_ordinary_push_after_a_release() {
    // Run unconditionally it would fire on every push: once `v1.5.0` is tagged,
    // the next `fix:` legitimately makes the recomputation say `1.5.1` while the
    // versions file still says `1.5.0`. Only at the moment a tag is about to be
    // created is the versions file claiming to describe HEAD.
    let r = Repo::new("gate78", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");
    r.git(&["tag", "v1.5.0"]);
    r.commit("fix: lands after the release");

    let f = FakeForge::new();
    f.create_release_for("v1.5.0");
    let out = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap();
    assert!(out.is_no_op(), "{out:?}");
}

// --- #9 · the collision check --------------------------------------------

#[test]
fn a_tag_that_exists_unreachably_refuses_rather_than_swallowing_a_release() {
    let r = Repo::new("collide", &[]);
    r.commit("feat: base");
    r.git(&["tag", "v1.4.2"]);
    r.git(&["checkout", "-q", "-b", "other"]);
    r.commit("feat: over there");
    r.git(&["tag", "v1.5.0"]);
    r.git(&["checkout", "-q", "main"]);
    r.commit("feat: add export");

    let f = FakeForge::new();
    let err = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap_err();
    assert!(matches!(err, PhaseBError::Collision { .. }), "{err:?}");
    assert!(f.mutating_calls().is_empty());
}

// --- A7 · a virgin repo ---------------------------------------------------

#[test]
fn a7_phase_b_on_a_repo_that_has_never_released_does_nothing_at_all() {
    // No versions-file entry means no version to derive a tag from, so the
    // package is skipped — no tag, no release, no error.
    let r = Repo::new("a7", &[]);
    r.commit("feat: add login");

    let f = FakeForge::new();
    let out = run(&r, SHAPE_1, "", &f).unwrap();
    assert!(out.is_no_op());
    assert!(out.already_tagged.is_empty());
    assert!(f.calls().is_empty(), "zero forge calls: {:?}", f.call_names());
}

// --- #96 · ordering, and #90 · stable-only `prev` ------------------------

#[test]
fn hash_96_releases_are_created_in_config_order_with_the_umbrella_last() {
    let r = Repo::new("order", &["services/api", "services/worker"]);
    r.commit("feat(api): base");
    r.git(&["tag", "api-v1.4.2"]);
    r.git(&["tag", "worker-v0.9.1"]);
    r.git(&["tag", "platform-v1.7.0"]);
    r.commit("feat(api): add caching");
    r.commit("fix(worker): handle nil");

    let f = FakeForge::new();
    let versions = r#"
    [umbrella]
    version = "1.8.0"
    [packages.api]
    version = "1.5.0"
    [packages.worker]
    version = "0.9.2"
    "#;
    let out = run(&r, MONOREPO, versions, &f).unwrap();

    let order: Vec<&str> = out.tagged.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(order, vec!["api", "worker", "platform"], "config order, umbrella last");

    // The umbrella's embedded manifest describes work already done, which is the
    // reason for the ordering.
    let last = f.calls().last().cloned().unwrap();
    let cicada::forge::fake::Call::CreateRelease(req) = last else { panic!() };
    assert_eq!(req.tag, "platform-v1.8.0");
    let json = req.body.split("```json").nth(1).unwrap().split("```").next().unwrap();
    let v: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["releases"].as_array().unwrap().len(), 3);
}

#[test]
fn hash_90_a_prerelease_tag_is_never_chosen_as_the_previous_stable() {
    // `v1.5.0-rc.2` matches `v{version}` and sorts below `1.5.0`, so an
    // unqualified "highest tag" would anchor the recomputation on an rc and
    // replan over a range that may be empty. The guard is one-sided, so it would
    // pass either way and the bug would never surface.
    let r = Repo::new("prev90", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");
    r.git(&["tag", "v1.5.0-rc.1"]);
    r.commit("feat!: and a breaking one");

    let f = FakeForge::new();
    // Anchored on `v1.4.2` the range holds a `feat!:`, so 1.5.0 is too low and
    // the guard fires. Anchored on the rc it would hold only the breaking commit
    // — still too low, but for the wrong reason. Assert the number it names.
    let err = run(&r, SHAPE_1, "[packages.app]\nversion = \"1.5.0\"\n", &f).unwrap_err();
    let PhaseBError::StaleMerge { expected, .. } = &err else { panic!("{err:?}") };
    assert_eq!(expected, "2.0.0", "computed from the last stable tag, not the rc");
}

// --- G1 / G6 · a maintenance branch releases independently ---------------

#[test]
fn g6_a_maintenance_branch_releases_without_touching_main() {
    let r = Repo::new("maint", &["services/api", "services/worker"]);
    r.commit("feat(api): base");
    r.git(&["tag", "api-v1.4.7"]);
    let main_head = r.head();

    // Main moves on to 1.5.x.
    r.commit("feat(api): main moves on");
    r.git(&["tag", "api-v1.5.0"]);

    // The maintenance line is cut behind it, and takes a fix.
    r.git(&["checkout", "-q", "-b", "release/1.4", &main_head]);
    r.commit("fix(api): handle nil");

    let f = FakeForge::new();
    let out = run(&r, MONOREPO, "[packages.api]\nversion = \"1.4.8\"\n", &f).unwrap();

    assert_eq!(out.tagged, vec![("api".to_string(), "api-v1.4.8".to_string())]);
    assert!(r.tag_exists("api-v1.4.8"));
    // Independently: main's tags are untouched and its line is not consulted.
    assert!(r.tag_exists("api-v1.5.0"));
    assert_eq!(r.tag_sha("api-v1.4.8"), r.head());

    // And the maintenance guard does not fire here — it gates *opening* a
    // release, and this one is already merged.
    assert!(out.released.len() == 1);
}

#[test]
fn a_maintenance_line_does_not_anchor_on_a_tag_from_the_newer_line() {
    // `api-v1.5.0` is not reachable from `release/1.4`, so it must not be chosen
    // as `prev` — the same reachability rule the collision check uses, from the
    // other end.
    let r = Repo::new("maint2", &["services/api", "services/worker"]);
    r.commit("feat(api): base");
    r.git(&["tag", "api-v1.4.7"]);
    let cut = r.head();
    r.commit("feat(api): main moves on");
    r.git(&["tag", "api-v1.5.0"]);

    r.git(&["checkout", "-q", "-b", "release/1.4", &cut]);
    r.commit("fix(api): handle nil");

    let f = FakeForge::new();
    let out = run(&r, MONOREPO, "[packages.api]\nversion = \"1.4.8\"\n", &f).unwrap();
    assert_eq!(out.tagged.len(), 1);

    let cicada::forge::fake::Call::CreateRelease(req) = &f.calls()[1] else { panic!() };
    // The notes cover `api-v1.4.7..HEAD` — the fix, and not main's feature.
    assert!(req.body.contains("handle nil"), "{}", req.body);
    assert!(!req.body.contains("main moves on"), "{}", req.body);
}

// --- the dry-run view -----------------------------------------------------

#[test]
fn the_dry_run_reports_the_two_questions_without_answering_them_destructively() {
    let r = Repo::new("dry", &[]);
    r.commit("feat: released already");
    r.git(&["tag", "v1.4.2"]);
    r.commit("feat: add export");

    let cfg = Config::from_toml(SHAPE_1).unwrap();
    let v = VersionsFile::from_toml("[packages.app]\nversion = \"1.5.0\"\n").unwrap();
    let git = GitCli::discover(&r.dir).unwrap();

    let pending = phase_b::pending(&git, &cfg, &v).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tag, "v1.5.0");
    assert_eq!(pending[0].state, TagState::WouldTag);
    assert!(!r.tag_exists("v1.5.0"), "a dry run creates nothing");
}

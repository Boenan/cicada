//! The planner, as an acceptance suite.
//!
//! Every test here is a scenario from `docs/scenarios/` — the id in each name is
//! the row it pins. Those files exist so that *a scenario whose expected outcome
//! cannot be derived from the design is a hole in the design*, and this is where
//! they stop being prose.
//!
//! All of it is pure: no repo, no network, no filesystem. Anchor resolution is the
//! binary's job, so a test states the range's contents directly.

use std::collections::BTreeMap;

use cicada_core::commit::{ParsedCommit, RawCommit, parse};
use cicada_core::config::{Bump, Config, PackageId, Target};
use cicada_core::plan::{Plan, PlanError, PlanInput, Reason, plan};
use cicada_core::versions::VersionsFile;

// --- fixtures ------------------------------------------------------------

/// Shape 1 — one package (single-service.md §The two shapes). `default_scope`
/// names it, so unscoped commits route to it and length 1 is not a mode (#16).
const SHAPE_1: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"
"#;

/// The monorepo shape (monorepo.md §The shape): three Rust packages, a chart that
/// tracks one of them, and an umbrella. Declared `api`, `worker`, `auth`,
/// `api-chart` — deliberately *not* alphabetical, so O6 can tell config order
/// from a sort.
const MONOREPO: &str = r#"
[settings]
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]
default_scope        = "platform"

[forge]
kind = "github"

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

[packages.api-chart]
path            = "deploy/charts/api"
tag_format      = "api-chart-v{version}"
initial_version = "0.1.0"

[[packages.api-chart.updaters]]
kind            = "helm"
path            = "deploy/charts/api/Chart.yaml"
app_version     = { track = "api" }
on_tracked_bump = "patch"
"#;

const MONOREPO_VERSIONS: &str = r#"
[umbrella]
version = "1.7.0"

[packages.api]
version = "1.4.2"
[packages.worker]
version = "0.9.1"
[packages.api-chart]
version = "0.9.0"
# `auth` absent — never released (#4)
"#;

fn cfg(src: &str) -> Config {
    Config::from_toml(src).expect("fixture config parses")
}

fn versions(src: &str) -> VersionsFile {
    VersionsFile::from_toml(src).expect("fixture versions file parses")
}

/// Commits are `c1`, `c2`, … per the fixture conventions.
fn commits(subjects: &[&str]) -> Vec<ParsedCommit> {
    subjects
        .iter()
        .enumerate()
        .map(|(i, s)| parse(&RawCommit::new(format!("c{}", i + 1), *s, "")).expect(s))
        .collect()
}

/// One commit with a body, for the `BREAKING CHANGE:` footer (B4).
fn commit_with_body(sha: &str, subject: &str, body: &str) -> ParsedCommit {
    parse(&RawCommit::new(sha, subject, body)).expect(subject)
}

/// Every target's range holds the same commits — the ordinary case, where nothing
/// has released since the last time everything released.
fn same_range(config: &Config, cs: &[ParsedCommit]) -> BTreeMap<Target, Vec<ParsedCommit>> {
    let mut h = BTreeMap::new();
    for i in 0..config.packages.len() {
        h.insert(Target::Package(PackageId(i)), cs.to_vec());
    }
    if config.umbrella.is_some() {
        h.insert(Target::Umbrella, cs.to_vec());
    }
    h
}

fn run(config: &Config, v: &VersionsFile, cs: &[ParsedCommit]) -> Result<Plan, PlanError> {
    run_on(config, v, cs, "main", false)
}

fn run_on(
    config: &Config,
    v: &VersionsFile,
    cs: &[ParsedCommit],
    branch: &str,
    maintenance: bool,
) -> Result<Plan, PlanError> {
    let history = same_range(config, cs);
    plan(&PlanInput {
        config,
        versions: v,
        history: &history,
        maintenance,
        branch: branch.to_string(),
        prerelease: None,
    })
}

/// `(name, from, to, bump, reason)` for every release, in plan order.
fn summary(p: &Plan) -> Vec<(String, Option<String>, String, Bump, Reason)> {
    p.releases
        .iter()
        .map(|r| {
            (
                r.name.clone(),
                r.from.as_ref().map(ToString::to_string),
                r.to.to_string(),
                r.bump,
                r.reason,
            )
        })
        .collect()
}

fn only(p: &Plan) -> (String, String, Bump) {
    assert_eq!(p.releases.len(), 1, "expected exactly one release: {:#?}", summary(p));
    let r = &p.releases[0];
    (r.name.clone(), r.to.to_string(), r.bump)
}

// --- A · bootstrap: the repo has never released --------------------------

#[test]
fn a1_a2_a3_the_first_release_table() {
    let c = cfg(SHAPE_1);
    let none = VersionsFile::default();

    // A1 · feat → 0.1.0
    let p = run(&c, &none, &commits(&["feat: add login"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "0.1.0".into(), Bump::Minor));
    // `from` is None, not 0.0.0 — a version that never existed (#89).
    assert_eq!(p.releases[0].from, None);
    assert_eq!(p.releases[0].reason, Reason::FirstRelease);
    assert_eq!(p.releases[0].tag, "v0.1.0");

    // A2 · fix → 0.0.1
    let p = run(&c, &none, &commits(&["fix: handle empty body"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "0.0.1".into(), Bump::Patch));

    // A3 · breaking → 1.0.0, even on a first release. This is the row that makes
    // the first release a *table* rather than arithmetic from 0.0.0.
    let p = run(&c, &none, &commits(&["feat!: initial public API"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "1.0.0".into(), Bump::Major));
}

#[test]
fn a4_initial_version_overrides_the_table_outright() {
    let c = cfg(&format!("{SHAPE_1}initial_version = \"1.0.0\"\n"));
    let p = run(&c, &VersionsFile::default(), &commits(&["fix: typo"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "1.0.0".into(), Bump::Patch));
}

#[test]
fn a5_a6_a_range_with_nothing_releasable_is_an_empty_plan() {
    let c = cfg(SHAPE_1);
    let none = VersionsFile::default();

    // A5 · chore and docs are `bump = "none"` (#36).
    assert!(run(&c, &none, &commits(&["chore: tidy", "docs: readme"])).unwrap().is_empty());
    // A6 · an unknown type is ignored, not an error.
    assert!(run(&c, &none, &commits(&["wibble: something"])).unwrap().is_empty());
    // And the two together, which is what a quiet week actually looks like.
    assert!(
        run(&c, &none, &commits(&["chore: tidy", "wibble: x", "docs: readme"])).unwrap().is_empty()
    );
}

// --- B · version arithmetic ----------------------------------------------

fn shape_1_at(version: &str) -> VersionsFile {
    versions(&format!("[packages.app]\nversion = \"{version}\"\n"))
}

#[test]
fn b1_to_b10_the_arithmetic_table() {
    let c = cfg(SHAPE_1);
    let at = shape_1_at("1.4.2");

    let cases: &[(&[&str], &str, Bump)] = &[
        // B1 · section bump = patch
        (&["fix: a"], "1.4.3", Bump::Patch),
        // B2 · highest wins
        (&["feat: a", "fix: b"], "1.5.0", Bump::Minor),
        // B3 · `!` overrides the section bump
        (&["feat!: a"], "2.0.0", Bump::Major),
        // B7 · perf is patch
        (&["perf: a"], "1.4.3", Bump::Patch),
        // B8 · chore contributes nothing alongside a feat
        (&["feat: a", "chore(deps): bump serde"], "1.5.0", Bump::Minor),
    ];
    for (subjects, expected, bump) in cases {
        let p = run(&c, &at, &commits(subjects)).unwrap();
        assert_eq!(only(&p), ("app".into(), (*expected).into(), *bump), "for {subjects:?}");
    }

    // B9 · the dependency-bump case: chore alone releases nothing (#36).
    assert!(run(&c, &at, &commits(&["chore(deps): bump serde"])).unwrap().is_empty());
    // B10 · docs alone releases nothing.
    assert!(run(&c, &at, &commits(&["docs: tidy the readme"])).unwrap().is_empty());
}

#[test]
fn b4_a_breaking_change_footer_is_equivalent_to_the_bang() {
    let c = cfg(SHAPE_1);
    let cs = vec![commit_with_body(
        "c1",
        "fix: retry on 429",
        "BREAKING CHANGE: retries are now on by default",
    )];
    let p = run(&c, &shape_1_at("1.4.2"), &cs).unwrap();
    assert_eq!(only(&p), ("app".into(), "2.0.0".into(), Bump::Major));
}

#[test]
fn b5_b6_pre_major_decides_whether_zero_x_escapes() {
    // B5 · the default keeps 0.x inside 0.x — never 1.0.0 by accident.
    let c = cfg(SHAPE_1);
    let p = run(&c, &shape_1_at("0.3.0"), &commits(&["feat!: rework"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "0.4.0".into(), Bump::Major));

    // B6 · the opt-in escape is a statement about the package in config, not an
    // instruction buried in one commit message (#39).
    let c = cfg(&format!("{SHAPE_1}pre_major = \"major\"\n"));
    let p = run(&c, &shape_1_at("0.3.0"), &commits(&["feat!: rework"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "1.0.0".into(), Bump::Major));
}

#[test]
fn b12_an_explicit_scope_is_identical_to_an_unscoped_commit_in_shape_1() {
    let c = cfg(SHAPE_1);
    let at = shape_1_at("1.4.2");
    let unscoped = run(&c, &at, &commits(&["feat: add export"])).unwrap();
    let scoped = run(&c, &at, &commits(&["feat(app): add export"])).unwrap();
    assert_eq!(summary(&unscoped), summary(&scoped));
}

#[test]
fn b13_an_unknown_scope_is_fatal_and_suggests() {
    let c = cfg(SHAPE_1);
    let err = run(&c, &shape_1_at("1.4.2"), &commits(&["feat(parser): tune"])).unwrap_err();
    let PlanError::UnknownScopes(scopes) = &err else { panic!("{err:?}") };
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].scope, "parser");
    // No near match for `parser` among `["app"]`, so the message falls back to the
    // full list rather than inventing a misleading guess.
    assert_eq!(scopes[0].suggestion, None);

    // The design's own worked example is a transposition, which is why the
    // suggester is Damerau-Levenshtein rather than plain Levenshtein.
    let c = cfg(MONOREPO);
    let err = run(&c, &versions(MONOREPO_VERSIONS), &commits(&["feat(aip): tune pool size"]))
        .unwrap_err();
    let PlanError::UnknownScopes(scopes) = &err else { panic!("{err:?}") };
    assert_eq!(scopes[0].suggestion.as_deref(), Some("api"));
}

// --- B11 / B14 / B15 / G2 · the maintenance guard ------------------------

#[test]
fn b11_b14_the_guard_refuses_rather_than_clamping() {
    let c = cfg(SHAPE_1);
    let at = shape_1_at("1.4.2");

    for subject in ["feat: add export", "feat!: drop v1"] {
        let err = run_on(&c, &at, &commits(&[subject]), "release/1.4", true).unwrap_err();
        let PlanError::MaintenanceGuard { branch, offenders } = &err else { panic!("{err:?}") };
        assert_eq!(branch, "release/1.4");
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders[0].package, "app");
        // B11 detail: the error names the offending *commits*. On a cherry-pick
        // batch of twenty, "a feat: is present" is not actionable.
        assert_eq!(offenders[0].commits.len(), 1);
        assert!(offenders[0].commits[0].1.contains("add export") || subject.contains("drop v1"));
    }
}

#[test]
fn b15_the_guard_is_silent_in_the_normal_case() {
    // The assertion that matters most in practice: a maintenance branch doing its
    // ordinary job must never see the guard.
    let c = cfg(SHAPE_1);
    let p = run_on(&c, &shape_1_at("1.4.2"), &commits(&["fix: handle nil"]), "release/1.4", true)
        .unwrap();
    assert_eq!(only(&p), ("app".into(), "1.4.3".into(), Bump::Patch));
}

#[test]
fn b11_the_guard_names_every_offending_commit_not_just_the_first() {
    let c = cfg(SHAPE_1);
    let cs = commits(&[
        "fix: one",
        "feat: two",
        "chore: three",
        "feat: four",
    ]);
    let err = run_on(&c, &shape_1_at("1.4.2"), &cs, "release/1.4", true).unwrap_err();
    let PlanError::MaintenanceGuard { offenders, .. } = &err else { panic!("{err:?}") };
    assert_eq!(offenders[0].commits.len(), 2, "both feats, and neither the fix nor the chore");

    // G2b · the message states the two real answers. Asserted on the text rather
    // than the behaviour, because a blocked pipeline with no stated way forward is
    // how an escape hatch gets reintroduced.
    let text = err.to_string();
    assert!(text.contains("no override"), "{text}");
    assert!(text.contains("newer line"), "{text}");
    assert!(text.contains("cherry-pick"), "{text}");
}

// --- O · scope routing and attribution -----------------------------------

#[test]
fn o1_the_reference_monorepo_scenario() {
    let c = cfg(MONOREPO);
    let p = run(&c, &versions(MONOREPO_VERSIONS), &commits(&["feat(api): add caching"])).unwrap();

    assert_eq!(
        summary(&p),
        vec![
            ("api".into(), Some("1.4.2".into()), "1.5.0".into(), Bump::Minor, Reason::Commits),
            // worker and auth are absent — a package that does not release is not
            // in the plan at all.
            (
                "api-chart".into(),
                Some("0.9.0".into()),
                "0.9.1".into(),
                Bump::Patch,
                Reason::Derived
            ),
            (
                "platform".into(),
                Some("1.7.0".into()),
                "1.8.0".into(),
                Bump::Minor,
                Reason::Aggregate
            ),
        ]
    );

    // The chart has no commits of its own, so its entry explains itself from
    // `derived_from` rather than rendering empty (#97).
    let chart = p.releases.iter().find(|r| r.name == "api-chart").unwrap();
    assert!(chart.commits.is_empty());
    let d = chart.derived_from.as_ref().expect("a derived release carries its cause");
    assert_eq!(d.package, "api");
    assert_eq!(d.version.to_string(), "1.5.0");

    // Tags come from each package's own `tag_format`.
    assert_eq!(p.releases[0].tag, "api-v1.5.0");
    assert_eq!(p.releases[2].tag, "platform-v1.8.0");
}

#[test]
fn o2_multi_scope_is_the_whole_blast_radius_mechanism() {
    let c = cfg(MONOREPO);
    let p = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api,worker): shared retry backoff"]),
    )
    .unwrap();

    let names: Vec<&str> = p.releases.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["api", "worker", "api-chart", "platform"]);
    assert_eq!(p.releases[1].to.to_string(), "0.10.0");

    // The one commit appears under *both* packages — which is how it lands in two
    // changelogs (R3).
    assert_eq!(p.releases[0].commits.len(), 1);
    assert_eq!(p.releases[1].commits.len(), 1);
    assert_eq!(p.releases[0].commits[0].sha, p.releases[1].commits[0].sha);
}

#[test]
fn o3_o7_an_unscoped_commit_releases_the_umbrella_alone() {
    let c = cfg(MONOREPO);
    let v = versions(MONOREPO_VERSIONS);

    // O3 · unscoped → `default_scope`, which is the umbrella here (#38).
    let p = run(&c, &v, &commits(&["fix: correct the deploy script"])).unwrap();
    assert_eq!(only(&p), ("platform".into(), "1.7.1".into(), Bump::Patch));
    // It is earned, not aggregated: the umbrella has a commit of its own.
    assert_eq!(p.releases[0].reason, Reason::Commits);

    // O7 · naming the umbrella explicitly is the same commit.
    let explicit = run(&c, &v, &commits(&["fix(platform): correct the deploy script"])).unwrap();
    assert_eq!(summary(&p), summary(&explicit));
}

#[test]
fn o8_bump_none_applies_per_package_not_just_globally() {
    let c = cfg(MONOREPO);
    let p = run(&c, &versions(MONOREPO_VERSIONS), &commits(&["chore(api): bump serde"])).unwrap();
    assert!(p.is_empty(), "{:#?}", summary(&p));
}

#[test]
fn o9_a_never_released_package_takes_the_first_release_table() {
    let c = cfg(MONOREPO);
    let p =
        run(&c, &versions(MONOREPO_VERSIONS), &commits(&["feat(auth): add token rotation"])).unwrap();

    assert_eq!(
        summary(&p),
        vec![
            ("auth".into(), None, "0.1.0".into(), Bump::Minor, Reason::FirstRelease),
            (
                "platform".into(),
                Some("1.7.0".into()),
                "1.8.0".into(),
                Bump::Minor,
                Reason::Aggregate
            ),
        ]
    );
    // api and worker are untouched, and the chart does not fire: it tracks `api`,
    // which did not release.
    assert!(p.releases.iter().all(|r| r.name != "api-chart"));
}

#[test]
fn o5_one_bad_scope_fails_the_whole_run() {
    let c = cfg(MONOREPO);
    let err = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api): good", "feat(api,wrker): bad"]),
    )
    .unwrap_err();

    // No partial plan: releasing the half that parsed would ship a version whose
    // changelog is missing the change that motivated it.
    let PlanError::UnknownScopes(scopes) = &err else { panic!("{err:?}") };
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].scope, "wrker");
    assert_eq!(scopes[0].suggestion.as_deref(), Some("worker"));
}

#[test]
fn o6_config_order_is_the_output_order_and_it_is_not_alphabetical() {
    let c = cfg(MONOREPO);
    let p = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api): a", "fix(worker): b", "feat(auth): c"]),
    )
    .unwrap();

    let names: Vec<&str> = p.releases.iter().map(|r| r.name.as_str()).collect();
    // Config order, umbrella last (#31, #96) — alphabetical would be
    // api, api-chart, auth, worker.
    assert_eq!(names, vec!["api", "worker", "auth", "api-chart", "platform"]);

    // Determinism is observable (Goal 7): the same inputs twice, byte-identical.
    let again = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api): a", "fix(worker): b", "feat(auth): c"]),
    )
    .unwrap();
    assert_eq!(summary(&p), summary(&again));
}

// --- P · independent version lines ---------------------------------------

#[test]
fn p2_the_range_decides_what_is_read_the_scope_decides_what_is_attributed() {
    // After `api` released, worker is still anchored further back — so worker's
    // range *spans* the api release commit. That is correct and must not be
    // filtered; the api commits in it simply route elsewhere.
    let c = cfg(MONOREPO);
    let v = versions(MONOREPO_VERSIONS);

    let api_range = commits(&["fix(worker): handle nil"]);
    let worker_range = {
        let mut cs = commits(&["feat(api): add caching"]);
        cs.push(parse(&RawCommit::new("c2", "fix(worker): handle nil", "")).unwrap());
        cs
    };

    let mut history = BTreeMap::new();
    history.insert(Target::Package(PackageId(0)), api_range.clone()); // api
    history.insert(Target::Package(PackageId(1)), worker_range.clone()); // worker
    history.insert(Target::Package(PackageId(2)), Vec::new()); // auth
    history.insert(Target::Package(PackageId(3)), api_range.clone()); // api-chart
    history.insert(Target::Umbrella, worker_range.clone());

    let p = plan(&PlanInput {
        config: &c,
        versions: &v,
        history: &history,
        maintenance: false,
        branch: "main".into(),
        prerelease: None,
    })
    .unwrap();

    // api's own range holds no api commits, so api does not release — even though
    // a `feat(api):` is visible in worker's range.
    assert!(p.releases.iter().all(|r| r.name != "api"));
    let worker = p.releases.iter().find(|r| r.name == "worker").unwrap();
    assert_eq!(worker.to.to_string(), "0.9.2");
    assert_eq!(worker.commits.len(), 1, "only worker-scoped commits are attributed");
    assert_eq!(worker.commits[0].scopes, vec!["worker"]);
}

// --- Q · umbrella aggregation --------------------------------------------

#[test]
fn q1_the_umbrella_aggregates_the_bump_kind_never_the_number() {
    // The fixture normally hides the distinction, because both lines land on
    // 2.0.0. Separate them: umbrella at 0.4.0, api at 1.4.2, one `feat(api)!:`.
    let c = cfg(MONOREPO);
    let v = versions(
        r#"
        [umbrella]
        version = "0.4.0"
        [packages.api]
        version = "1.4.2"
        "#,
    );
    let p = run(&c, &v, &commits(&["feat(api)!: drop v1 endpoints"])).unwrap();

    let api = p.releases.iter().find(|r| r.name == "api").unwrap();
    assert_eq!(api.to.to_string(), "2.0.0");

    let platform = p.releases.iter().find(|r| r.name == "platform").unwrap();
    // A *major bump* applied to the umbrella's own line, which under the default
    // `pre_major = "minor"` does not escape 0.x. "Copy the child" would say 2.0.0
    // and looks right in every other scenario, which is why this row exists.
    assert_eq!(platform.to.to_string(), "0.5.0");
    assert_eq!(platform.bump, Bump::Major);
}

#[test]
fn q3_the_highest_child_bump_wins() {
    let c = cfg(MONOREPO);
    let p = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api): a", "fix(worker): b"]),
    )
    .unwrap();
    let platform = p.releases.iter().find(|r| r.name == "platform").unwrap();
    assert_eq!(platform.to.to_string(), "1.8.0");
}

#[test]
fn q5_the_umbrella_takes_patch_from_two_independent_patch_sources() {
    let c = cfg(MONOREPO);
    let p = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["fix(worker): a", "fix: correct the deploy script"]),
    )
    .unwrap();
    let platform = p.releases.iter().find(|r| r.name == "platform").unwrap();
    assert_eq!(platform.to.to_string(), "1.7.1");
    assert_eq!(platform.bump, Bump::Patch);
}

#[test]
fn q6_no_release_at_all_rather_than_an_empty_umbrella_release() {
    let c = cfg(MONOREPO);
    assert!(run(&c, &versions(MONOREPO_VERSIONS), &commits(&["chore(api): x"])).unwrap().is_empty());
}

/// The sharpest form of #22 meeting #10. Q7 and Q8 differ only in whether the
/// chart's bump was earned or derived, and nothing else in the repo changes.
fn monorepo_tracking_minor() -> Config {
    cfg(&MONOREPO.replace(r#"on_tracked_bump = "patch""#, r#"on_tracked_bump = "minor""#))
}

#[test]
fn q7_a_derived_chart_bump_never_feeds_the_aggregate() {
    let c = monorepo_tracking_minor();
    let p = run(&c, &versions(MONOREPO_VERSIONS), &commits(&["fix(api): handle nil"])).unwrap();

    assert_eq!(
        summary(&p),
        vec![
            ("api".into(), Some("1.4.2".into()), "1.4.3".into(), Bump::Patch, Reason::Commits),
            (
                "api-chart".into(),
                Some("0.9.0".into()),
                "0.10.0".into(),
                Bump::Minor,
                Reason::Derived
            ),
            // 1.7.1, NOT 1.8.0. If the chart's derived minor fed the aggregate,
            // one `fix:` in one service would announce a minor release of the
            // whole platform.
            (
                "platform".into(),
                Some("1.7.0".into()),
                "1.7.1".into(),
                Bump::Patch,
                Reason::Aggregate
            ),
        ]
    );
}

#[test]
fn q8_the_charts_own_bump_does_feed_the_aggregate() {
    let c = monorepo_tracking_minor();
    let p = run(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["fix(api): handle nil", "feat(api-chart): add a PodDisruptionBudget"]),
    )
    .unwrap();

    let chart = p.releases.iter().find(|r| r.name == "api-chart").unwrap();
    assert_eq!(chart.to.to_string(), "0.10.0");
    // Earned *and* derived at once: the commits are its own, and the cause is
    // still carried so a template can mention it (§Templates).
    assert_eq!(chart.reason, Reason::Commits);
    assert_eq!(chart.commits.len(), 1);
    assert!(chart.derived_from.is_some());

    let platform = p.releases.iter().find(|r| r.name == "platform").unwrap();
    assert_eq!(platform.to.to_string(), "1.8.0", "the chart's earned minor is a real child bump");
}

#[test]
fn q9_the_guard_names_the_package_that_caused_the_bump_not_the_umbrella() {
    let c = cfg(MONOREPO);
    let err = run_on(
        &c,
        &versions(MONOREPO_VERSIONS),
        &commits(&["feat(api): add caching"]),
        "release/1.4",
        true,
    )
    .unwrap_err();

    let PlanError::MaintenanceGuard { offenders, .. } = &err else { panic!("{err:?}") };
    // The umbrella's minor is a *consequence*; reporting it as the cause would
    // send the reader to the wrong place.
    assert_eq!(offenders.len(), 1);
    assert_eq!(offenders[0].package, "api");
    assert_eq!(offenders[0].commits.len(), 1);
}

// --- T · a monorepo with no umbrella -------------------------------------

const NO_UMBRELLA: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "api"

[forge]
kind = "github"

[packages.api]
path       = "services/api"
tag_format = "api-v{version}"

[packages.worker]
path       = "services/worker"
tag_format = "worker-v{version}"
"#;

#[test]
fn t1_t3_aggregation_is_absent_not_empty() {
    let c = cfg(NO_UMBRELLA);
    let v = versions("[packages.api]\nversion = \"1.4.2\"\n[packages.worker]\nversion = \"0.9.1\"\n");

    // T1 · unscoped commits land on the named package.
    let p = run(&c, &v, &commits(&["fix: handle nil"])).unwrap();
    assert_eq!(only(&p), ("api".into(), "1.4.3".into(), Bump::Patch));

    // T3 · no umbrella entry at all, rather than one with a null version.
    assert!(p.releases.iter().all(|r| r.target != Target::Umbrella));
}

// --- shape 2 · an app and its chart are two packages (#74) ---------------

const APP_AND_CHART: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"

[packages.chart]
path            = "deploy/chart"
tag_format      = "chart-v{version}"
initial_version = "0.1.0"

[[packages.chart.updaters]]
kind        = "helm"
path        = "deploy/chart/Chart.yaml"
app_version = { track = "app" }
"#;

#[test]
fn m_shape_2_the_chart_is_dragged_but_is_never_a_child() {
    let c = cfg(APP_AND_CHART);
    let v = versions("[packages.app]\nversion = \"1.4.2\"\n[packages.chart]\nversion = \"0.9.0\"\n");
    let p = run(&c, &v, &commits(&["fix: handle nil"])).unwrap();

    assert_eq!(
        summary(&p),
        vec![
            ("app".into(), Some("1.4.2".into()), "1.4.3".into(), Bump::Patch, Reason::Commits),
            // `on_tracked_bump` defaults to patch (#22).
            ("chart".into(), Some("0.9.0".into()), "0.9.1".into(), Bump::Patch, Reason::Derived),
        ]
    );
    // Two independent version lines, never an umbrella and a child (#74): there is
    // no aggregate here, and removing or adding one would not change this pair.
    assert!(p.releases.iter().all(|r| r.target != Target::Umbrella));
}

#[test]
fn a_tracking_chart_that_has_never_released_takes_its_declared_initial_version() {
    // #88: `initial_version` is required on a helm package precisely so a derived
    // bump on a never-released chart cannot land on the first-release table, whose
    // size would have come from `on_tracked_bump` — a default that says nothing
    // about the chart at all.
    let c = cfg(APP_AND_CHART);
    let v = versions("[packages.app]\nversion = \"1.4.2\"\n");
    let p = run(&c, &v, &commits(&["fix: handle nil"])).unwrap();

    let chart = p.releases.iter().find(|r| r.name == "chart").unwrap();
    assert_eq!(chart.from, None);
    assert_eq!(chart.to.to_string(), "0.1.0");
    assert_eq!(chart.reason, Reason::FirstRelease);
}

#[test]
fn the_chart_does_not_fire_when_the_app_it_tracks_does_not_release() {
    let c = cfg(APP_AND_CHART);
    let v = versions("[packages.app]\nversion = \"1.4.2\"\n[packages.chart]\nversion = \"0.9.0\"\n");
    assert!(run(&c, &v, &commits(&["chore: tidy"])).unwrap().is_empty());
}

// --- the scope rule, narrowed to releasable commits ----------------------

#[test]
fn a_non_releasable_commit_may_carry_a_scope_that_names_no_package() {
    // B8/B9 use `chore(deps): bump serde` as *the* non-releasable example, and
    // `deps` is not a package. Under a blanket reading of #11 the design's own
    // worked example would be a hard error, and every repo with a dependency bot
    // would be unable to release at all.
    //
    // #11 exists so a `feat` is never silently misfiled. A commit that cannot move
    // a version number has no attribution to get wrong, so it is ignored — and
    // `check` reports it as a warning rather than dropping it silently.
    let c = cfg(SHAPE_1);
    let at = shape_1_at("1.4.2");

    assert!(run(&c, &at, &commits(&["chore(deps): bump serde"])).unwrap().is_empty());

    let p = run(&c, &at, &commits(&["feat: add export", "chore(deps): bump serde"])).unwrap();
    assert_eq!(only(&p), ("app".into(), "1.5.0".into(), Bump::Minor));
    // The chore is not attributed to `app` either — it routes nowhere, which is
    // what "ignored" means.
    assert_eq!(p.releases[0].commits.len(), 1);
    assert_eq!(p.releases[0].commits[0].kind, "feat");
}

#[test]
fn the_narrowing_does_not_reach_a_commit_that_can_release() {
    let c = cfg(SHAPE_1);
    let at = shape_1_at("1.4.2");

    // A typo'd scope on a `feat` is exactly what #11 is for, and still fatal.
    assert!(matches!(
        run(&c, &at, &commits(&["feat(deps): add export"])),
        Err(PlanError::UnknownScopes(_))
    ));
    // As is a breaking `chore`, whose `!` overrides the section's `bump = "none"`.
    assert!(matches!(
        run(&c, &at, &commits(&["chore(deps)!: drop the old client"])),
        Err(PlanError::UnknownScopes(_))
    ));
}

#[test]
fn an_initial_version_that_is_not_semver_is_a_config_finding_not_a_silent_fallback() {
    // The planner takes `initial_version` verbatim (A4), so a value it cannot
    // parse would otherwise fall back to the first-release table — releasing at a
    // number nobody asked for, with nothing saying why. `check` is where that
    // becomes legible, which is what lets the planner assume it parses.
    use cicada_core::check::validate_config;

    let c = cfg(&format!("{SHAPE_1}initial_version = \"1.0\"\n"));
    let findings = validate_config(&c);
    assert!(
        findings.iter().any(|f| f.subject == "packages.app.initial_version"),
        "{findings:#?}"
    );

    let c = cfg(&format!("{SHAPE_1}initial_version = \"1.0.0\"\n"));
    assert!(validate_config(&c).is_empty());
}

// --- reading a version back out of a tag ---------------------------------

#[test]
fn a_tag_round_trips_through_its_format() {
    use cicada_core::plan::{parse_tag, render_tag};
    use semver::Version;

    for (format, version) in [
        ("v{version}", "1.5.0"),
        ("api-v{version}", "1.5.0"),
        ("{version}", "0.0.1"),
        ("release/{version}", "2.0.0"),
        ("v{version}-release", "1.0.0"),
    ] {
        let v: Version = version.parse().unwrap();
        let tag = render_tag(format, &v);
        assert_eq!(parse_tag(format, &tag), Some(v), "{format} / {tag}");
    }
}

#[test]
fn a_prefix_relationship_between_two_packages_does_not_confuse_them() {
    // `api-` and `api-chart-` are legal together and common. Parsing
    // `api-chart-v0.9.0` against `api-v{version}` leaves `chart-v0.9.0`, which is
    // not semver — so the parse is what keeps the two lines apart.
    use cicada_core::plan::parse_tag;
    assert_eq!(parse_tag("api-v{version}", "api-chart-v0.9.0"), None);
    assert_eq!(parse_tag("api-v{version}", "api-v1.5.0").unwrap().to_string(), "1.5.0");
    assert_eq!(parse_tag("api-chart-v{version}", "api-chart-v0.9.0").unwrap().to_string(), "0.9.0");
}

#[test]
fn a_prerelease_tag_parses_and_is_distinguishable_from_a_stable_one() {
    // #90 — the staleness guard's `prev` must never select one of these, so it
    // has to be able to tell.
    use cicada_core::plan::parse_tag;
    let v = parse_tag("v{version}", "v1.5.0-rc.2").unwrap();
    assert_eq!(v.to_string(), "1.5.0-rc.2");
    assert!(!v.pre.is_empty());
    assert!(parse_tag("v{version}", "v1.5.0").unwrap().pre.is_empty());
}

#[test]
fn something_that_is_not_this_packages_tag_is_rejected() {
    use cicada_core::plan::parse_tag;
    assert_eq!(parse_tag("v{version}", "nightly"), None);
    assert_eq!(parse_tag("v{version}", "v1.5"), None);
    assert_eq!(parse_tag("v{version}", "1.5.0"), None);
    assert_eq!(parse_tag("v{version}", "vx.y.z"), None);
}

//! Prereleases, as an acceptance suite — scenario group F.
//!
//! The whole mechanism is *one extra field* (#60): `version` keeps meaning **last
//! stable release**, so the anchor, the range and the computed base are untouched
//! and every release — rc.1, rc.2, and the final stable — computes from the same
//! place. These tests exist to hold that property down, because the moment it
//! slips the stable changelog stops being complete.

use std::collections::BTreeMap;

use cicada_core::commit::{ParsedCommit, RawCommit, parse};
use cicada_core::config::{Config, PackageId, Target};
use cicada_core::plan::{Plan, PlanError, PlanInput, plan, split_prerelease};
use cicada_core::versions::VersionsFile;

const SHAPE_1: &str = r#"
[settings]
release_branches      = ["main", "prerelease-v*-rc", "release-*"]
prerelease_branches   = ["prerelease-v*-rc"]
prerelease_identifier = "rc"
default_scope         = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"
changelog  = "CHANGELOG.md"
"#;

fn cfg() -> Config {
    Config::from_toml(SHAPE_1).expect("fixture config parses")
}

fn commits(subjects: &[&str]) -> Vec<ParsedCommit> {
    subjects
        .iter()
        .enumerate()
        .map(|(i, s)| parse(&RawCommit::new(format!("c{}", i + 1), *s, "")).expect(s))
        .collect()
}

fn run(versions: &str, subjects: &[&str], identifier: Option<&str>) -> Result<Plan, PlanError> {
    let c = cfg();
    let v = VersionsFile::from_toml(versions).expect("versions parse");
    let cs = commits(subjects);
    let mut history = BTreeMap::new();
    history.insert(Target::Package(PackageId(0)), cs);
    plan(&PlanInput {
        config: &c,
        versions: &v,
        history: &history,
        maintenance: false,
        branch: "prerelease-v1.5-rc".into(),
        prerelease: identifier.map(str::to_string),
    })
}

fn to(versions: &str, subjects: &[&str], id: Option<&str>) -> String {
    let p = run(versions, subjects, id).unwrap_or_else(|e| panic!("{e}"));
    p.releases[0].to.to_string()
}

const AT_1_4_2: &str = "[packages.app]\nversion = \"1.4.2\"\n";

// --- F1–F3 · the counter rule (#63) --------------------------------------

#[test]
fn f1_the_first_rc_decorates_the_computed_stable_base() {
    let p = run(AT_1_4_2, &["feat: add export"], Some("rc")).unwrap();
    let r = &p.releases[0];
    assert_eq!(r.to.to_string(), "1.5.0-rc.1");
    assert_eq!(r.tag, "v1.5.0-rc.1");
    // `from` is the last stable, because that is what the range was anchored on.
    assert_eq!(r.from.as_ref().unwrap().to_string(), "1.4.2");
}

#[test]
fn f2_an_unchanged_base_and_identifier_increments_the_counter() {
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc.1\"\n";
    assert_eq!(to(open, &["feat: add export", "fix: handle nil"], Some("rc")), "1.5.0-rc.2");
}

#[test]
fn f3_a_changed_base_resets_the_counter() {
    // `2.0.0-rc.1`, not `2.0.0-rc.3`: the counter belongs to the base it
    // decorates, and this is a different base.
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc.2\"\n";
    assert_eq!(to(open, &["feat: a", "feat!: drop v1"], Some("rc")), "2.0.0-rc.1");
}

#[test]
fn f10d_a_changed_identifier_resets_the_counter_too() {
    // **The assertion the whole two-axis rule exists for.** With only the base
    // considered, an `alpha.3` line moving to `beta` computes an unchanged base,
    // increments, and publishes `1.5.0-beta.4` — a counter continuing across a
    // stage it never belonged to. #85 does not catch it either, because `beta.4`
    // sorts above `alpha.3`.
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-alpha.3\"\n";
    assert_eq!(to(open, &["feat: add export"], Some("beta")), "1.5.0-beta.1");
}

#[test]
fn f10d_a_line_walks_the_ladder_with_the_base_fixed_throughout() {
    // The k8s/Node shape, and the reason "one identifier at a time" is not "one
    // forever" (#93). The base is `1.5.0` at every step, so the notes stay
    // cumulative from `v1.4.2` and the eventual stable entry is complete.
    let steps: &[(&str, &str, &[&str])] = &[
        ("alpha", "1.5.0-alpha.1", &["feat: c1"]),
        ("alpha", "1.5.0-alpha.2", &["feat: c1", "fix: c2"]),
        ("beta", "1.5.0-beta.1", &["feat: c1", "fix: c2", "fix: c3"]),
        ("rc", "1.5.0-rc.1", &["feat: c1", "fix: c2", "fix: c3", "fix: c4"]),
    ];
    let mut open = AT_1_4_2.to_string();
    for (id, expected, subjects) in steps {
        let got = to(&open, subjects, Some(id));
        assert_eq!(got, *expected, "at stage {id}");
        open = format!("[packages.app]\nversion = \"1.4.2\"\nprerelease = \"{got}\"\n");
    }

    // F5 — promotion. No identifier, so the stable cuts from the same anchor and
    // its entry contains every commit since 1.4.2.
    let p = run(&open, &["feat: c1", "fix: c2", "fix: c3", "fix: c4"], None).unwrap();
    assert_eq!(p.releases[0].to.to_string(), "1.5.0");
    assert_eq!(p.releases[0].commits.len(), 4, "cumulative, not just since the last rc");
}

// --- #85 · monotonicity is the ladder's only guarantee -------------------

#[test]
fn hash_85_a_stage_that_sorts_downwards_is_refused() {
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc.2\"\n";

    // `rc` → `beta` sorts down.
    let err = run(open, &["feat: add export"], Some("beta")).unwrap_err();
    let PlanError::PrereleaseWentBackwards { open: o, computed, .. } = &err else {
        panic!("{err:?}")
    };
    assert_eq!(o.to_string(), "1.5.0-rc.2");
    assert_eq!(computed.to_string(), "1.5.0-beta.1");

    // The message says why there is no ladder to appeal to (#94).
    let s = err.to_string();
    assert!(s.contains("sorts below"), "{s}");
    assert!(s.contains("Semver's ordering"), "{s}");
}

#[test]
fn hash_94_a_legitimate_looking_ladder_that_semver_disagrees_with_is_still_refused() {
    // `preview` → `beta` reads like progress and sorts *down* (`p` > `b`). A
    // declared order would bless it while consumers resolving `>=1.5.0-preview`
    // do so with semver, not with cicada's config.
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-preview.1\"\n";
    assert!(matches!(
        run(open, &["feat: a"], Some("beta")),
        Err(PlanError::PrereleaseWentBackwards { .. })
    ));
    // And upwards is fine, whatever the words mean.
    assert_eq!(to(open, &["feat: a"], Some("rc")), "1.5.0-rc.1");
}

#[test]
fn hash_94_cicada_distinguishes_identifiers_in_no_way_at_all() {
    // F10 — what makes "one at a time" costless: the anchor, the base, the
    // counter and the skipped changelog are identical, and only the string
    // differs.
    for id in ["alpha", "beta", "rc", "preview", "wibble"] {
        let p = run(AT_1_4_2, &["feat: add export"], Some(id)).unwrap();
        let r = &p.releases[0];
        assert_eq!(r.to.to_string(), format!("1.5.0-{id}.1"));
        assert_eq!(r.from.as_ref().unwrap().to_string(), "1.4.2");
        assert_eq!(r.bump, cicada_core::Bump::Minor);
    }
}

// --- F9 · the line that outlives its own promotion (#91) ----------------

#[test]
fn f9_a_promoted_line_that_nobody_reset_keeps_cutting_rcs_and_that_is_accepted() {
    // Recorded behaviour, not a bug. The branch says an rc line is open at
    // `1.5.0-rc.2`, and cicada acts on what the branch and the config say.
    // Refusing would mean extending the collision check to the *computed base*,
    // which is machinery to detect a branch its operator stopped maintaining —
    // and the fix costs a fast-forward (F9b).
    let stale = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc.2\"\n";
    assert_eq!(to(stale, &["feat: c1", "fix: c2", "feat: c4"], Some("rc")), "1.5.0-rc.3");
}

#[test]
fn f9b_merging_main_back_resets_the_line() {
    // Afterwards the line reads `version = "1.5.0"` with no `prerelease` key, so
    // the base moves and the counter starts again.
    let reset = "[packages.app]\nversion = \"1.5.0\"\n";
    assert_eq!(to(reset, &["feat: c4"], Some("rc")), "1.6.0-rc.1");
}

// --- a first release cut as a prerelease ---------------------------------

#[test]
fn a_line_whose_first_release_is_an_rc_records_no_stable_version() {
    // There is no last stable, and writing `0.0.0` would be the sentinel #4
    // refuses — a version that never existed, which anchor resolution would then
    // go looking for a tag for.
    let p = run("", &["feat: add login"], Some("rc")).unwrap();
    assert_eq!(p.releases[0].to.to_string(), "0.1.0-rc.1");
    assert_eq!(p.releases[0].from, None);
}

#[test]
fn such_an_entry_round_trips_through_the_versions_file() {
    let v = VersionsFile::from_toml("[packages.app]\nprerelease = \"0.1.0-rc.1\"\n").unwrap();
    let e = v.package("app").unwrap();
    assert_eq!(e.version, None, "no stable yet");
    assert_eq!(e.prerelease.as_ref().unwrap().to_string(), "0.1.0-rc.1");
    assert_eq!(VersionsFile::from_toml(&v.to_toml()).unwrap(), v);
}

#[test]
fn an_entry_with_neither_field_is_refused_rather_than_read_as_released_at_nothing() {
    let err = VersionsFile::from_toml("[packages.app]\n").unwrap_err();
    assert!(err.to_string().contains("remove the entry"), "{err}");
}

// --- the splitter --------------------------------------------------------

#[test]
fn a_prerelease_splits_into_base_identifier_and_counter() {
    let v = |s: &str| s.parse::<semver::Version>().unwrap();
    assert_eq!(
        split_prerelease(&v("1.5.0-rc.2")),
        Some((v("1.5.0"), "rc".into(), 2))
    );
    // A dotted identifier keeps its dots; only a numeric tail is a counter (#64).
    assert_eq!(
        split_prerelease(&v("1.5.0-alpha.beta.3")),
        Some((v("1.5.0"), "alpha.beta".into(), 3))
    );
    // No numeric tail reports 0, so the next computed one sorts above it.
    assert_eq!(split_prerelease(&v("1.5.0-rc")), Some((v("1.5.0"), "rc".into(), 0)));
    assert_eq!(split_prerelease(&v("1.5.0")), None);
}

#[test]
fn an_identifier_with_no_counter_is_superseded_rather_than_matched() {
    // `1.5.0-rc` reports counter 0, so the next is `rc.1` — which sorts above it.
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc\"\n";
    assert_eq!(to(open, &["feat: a"], Some("rc")), "1.5.0-rc.1");
}

// --- #83 · the lint that closes the quiet gap ----------------------------

#[test]
fn i6d_i6e_the_default_pattern_is_linted_against_the_identifier() {
    use cicada_core::check::validate_prerelease_identifier;
    let c = Config::from_toml(
        "[settings]\nrelease_branches = [\"main\"]\ndefault_scope = \"app\"\n\
         [forge]\nkind = \"github\"\n[packages.app]\npath = \".\"\ntag_format = \"v{version}\"\n",
    )
    .unwrap();

    // I6d — the config the defaults exist to serve is not a finding.
    assert!(validate_prerelease_identifier(&c, "prerelease-v1.5-rc", true).is_none());
    // A branch the pattern does not match is none of its business.
    assert!(validate_prerelease_identifier(&c, "main", true).is_none());
    // A trailing segment that names no stage is not second-guessed.
    assert!(validate_prerelease_identifier(&c, "prerelease-wibble", true).is_none());

    // I6e — the branch says `beta`, the identifier says `rc`. Names both.
    let f = validate_prerelease_identifier(&c, "prerelease-v1.5-beta", true).unwrap();
    assert!(f.message.contains("beta") && f.message.contains("rc"), "{f:?}");
    assert!(f.help.as_ref().unwrap().contains("prerelease_identifier = \"beta\""), "{f:?}");

    // A repo that wrote its own patterns has said what it means.
    assert!(validate_prerelease_identifier(&c, "prerelease-v1.5-beta", false).is_none());
}

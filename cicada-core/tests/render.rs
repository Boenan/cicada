//! `Plan` → `Vec<FileEdit>`, as an acceptance suite.
//!
//! Scenario groups K (updaters and file edits) and L (changelog rendering) from
//! `docs/scenarios/single-service.md`, plus the composition rule that group K
//! leaves implicit. All pure: sources are supplied as strings, nothing is read.

use std::collections::BTreeMap;

use cicada_core::commit::{ParsedCommit, RawCommit, parse};
use cicada_core::config::{Config, PackageId, Target};
use cicada_core::edit::FileEdit;
use cicada_core::notes::LinkStyle;
use cicada_core::plan::{Plan, PlanInput, plan};
use cicada_core::render::{RenderInput, Rendered, render};
use cicada_core::versions::VersionsFile;

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
changelog  = "CHANGELOG.md"

[[packages.app.updaters]]
kind = "cargo"
path = "Cargo.toml"
"#;

const CARGO: &str = r#"[package]
name    = "app"
version = "1.4.2"   # managed by cicada
edition = "2024"

[dependencies]
serde = "1"
"#;

fn cfg(src: &str) -> Config {
    Config::from_toml(src).expect("fixture config parses")
}

fn commits(subjects: &[&str]) -> Vec<ParsedCommit> {
    subjects
        .iter()
        .enumerate()
        .map(|(i, s)| parse(&RawCommit::new(format!("c{}deadbeef", i + 1), *s, "")).expect(s))
        .collect()
}

fn make_plan(config: &Config, versions: &VersionsFile, cs: &[ParsedCommit]) -> Plan {
    make_plan_pre(config, versions, cs, None)
}

fn make_plan_pre(
    config: &Config,
    versions: &VersionsFile,
    cs: &[ParsedCommit],
    identifier: Option<&str>,
) -> Plan {
    let mut history = BTreeMap::new();
    for i in 0..config.packages.len() {
        history.insert(Target::Package(PackageId(i)), cs.to_vec());
    }
    if config.umbrella.is_some() {
        history.insert(Target::Umbrella, cs.to_vec());
    }
    plan(&PlanInput {
        config,
        versions,
        history: &history,
        maintenance: false,
        branch: "main".into(),
        prerelease: identifier.map(str::to_string),
    })
    .expect("fixture plans")
}

struct Case {
    config: Config,
    versions: VersionsFile,
    sources: BTreeMap<String, String>,
    overrides: BTreeMap<String, String>,
    workspace_roots: BTreeMap<String, String>,
    links: LinkStyle,
    date: String,
}

impl Case {
    fn new(config: &str) -> Self {
        Case {
            config: cfg(config),
            versions: VersionsFile::from_toml("[packages.app]\nversion = \"1.4.2\"\n").unwrap(),
            sources: BTreeMap::new(),
            overrides: BTreeMap::new(),
            workspace_roots: BTreeMap::new(),
            links: LinkStyle::default(),
            date: "2026-09-21".into(),
        }
    }

    fn source(mut self, path: &str, content: &str) -> Self {
        self.sources.insert(path.into(), content.into());
        self
    }

    fn versions(mut self, toml: &str) -> Self {
        self.versions = VersionsFile::from_toml(toml).unwrap();
        self
    }

    fn run(&self, subjects: &[&str]) -> Rendered {
        self.run_pre(subjects, None)
    }

    fn run_pre(&self, subjects: &[&str], identifier: Option<&str>) -> Rendered {
        let p = make_plan_pre(&self.config, &self.versions, &commits(subjects), identifier);
        render(&RenderInput {
            config: &self.config,
            plan: &p,
            versions: &self.versions,
            date: &self.date,
            links: &self.links,
            sources: &self.sources,
            workspace_roots: &self.workspace_roots,
            overrides: &self.overrides,
        })
        .unwrap_or_else(|e| panic!("render failed: {e}"))
    }
}

fn edit<'a>(r: &'a Rendered, path: &str) -> &'a FileEdit {
    r.edits
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| {
            panic!("no edit for {path}; got {:?}", r.edits.iter().map(|e| &e.path).collect::<Vec<_>>())
        })
}

fn paths(r: &Rendered) -> Vec<&str> {
    r.edits.iter().map(|e| e.path.as_str()).collect()
}

// --- K · updaters and file edits -----------------------------------------

#[test]
fn k1_cargo_changes_the_version_and_not_one_other_byte() {
    let r = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);
    let got = &edit(&r, "Cargo.toml").content;
    assert_eq!(*got, CARGO.replace("1.4.2", "1.5.0"));
    // Spelled out, because "preserves formatting" is the whole claim (#49).
    assert!(got.contains(r#"version = "1.5.0"   # managed by cicada"#), "{got}");
    assert!(got.ends_with("serde = \"1\"\n"), "{got}");
}

#[test]
fn k9_a_tag_only_package_writes_its_changelog_and_the_versions_file_and_nothing_else() {
    let config = SHAPE_1.split("[[packages.app.updaters]]").next().unwrap();
    let r = Case::new(config).run(&["fix: handle nil"]);
    assert_eq!(paths(&r), vec!["CHANGELOG.md", ".cicada-versions.toml"]);
}

#[test]
fn the_versions_file_records_the_new_version_and_keeps_every_other_line() {
    let config = format!(
        "{SHAPE_1}\n[packages.other]\npath = \"other\"\ntag_format = \"other-v{{version}}\"\n"
    );
    let r = Case::new(&config)
        .versions("[packages.app]\nversion = \"1.4.2\"\n[packages.other]\nversion = \"9.9.9\"\n")
        .source("Cargo.toml", CARGO)
        .run(&["feat(app): add export"]);

    let got = &edit(&r, ".cicada-versions.toml").content;
    assert!(got.contains("[packages.app]\nversion = \"1.5.0\""), "{got}");
    // `other` is not releasing, and this file is the sole source of truth (#75) —
    // dropping its line would lose the only record of where that line stands.
    assert!(got.contains("[packages.other]\nversion = \"9.9.9\""), "{got}");
}

// --- the composition rule ------------------------------------------------

const TWO_EDITS: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"
changelog  = "CHANGELOG.md"

[[packages.app.updaters]]
kind = "generic"
path = "deploy/values.yaml"
edit = { yaml = "image.tag" }

[[packages.app.updaters]]
kind = "generic"
path = "deploy/values.yaml"
edit = { yaml = "labels.version" }
"#;

#[test]
fn two_surgical_updaters_on_one_file_fold_rather_than_clobbering() {
    // `FileEdit` carries final content, so a naive implementation emits two edits
    // for one path and the second wins — losing the first silently. They must fold.
    let values = "image:\n  repo: acme/app\n  tag: 1.4.2      # keep this comment\nlabels:\n  version: 1.4.2\n";
    let r = Case::new(TWO_EDITS).source("deploy/values.yaml", values).run(&["feat: add export"]);

    let got = &edit(&r, "deploy/values.yaml").content;
    assert_eq!(
        *got,
        "image:\n  repo: acme/app\n  tag: 1.5.0      # keep this comment\nlabels:\n  version: 1.5.0\n"
    );
    // One path, one edit.
    assert_eq!(r.edits.iter().filter(|e| e.path == "deploy/values.yaml").count(), 1);
}

#[test]
fn k7_a_template_sharing_a_path_with_another_updater_is_refused_by_check() {
    // The collision #44 actually means: a whole-file writer cannot fold.
    use cicada_core::check::{Severity, validate_config};
    let c = cfg(&format!(
        "{SHAPE_1}\n[[packages.app.updaters]]\nkind = \"template\"\ntemplate = \"t.j2\"\noutput = \"Cargo.toml\"\n"
    ));
    let findings = validate_config(&c);
    assert!(
        findings.iter().any(|f| f.severity == Severity::Problem && f.message.contains("also writes")),
        "{findings:#?}"
    );

    // And two surgical updaters on one path are *not* a finding — they fold.
    assert!(validate_config(&cfg(TWO_EDITS)).is_empty());
}

#[test]
fn a_generic_updater_with_no_edit_is_a_finding_rather_than_a_silent_no_op() {
    use cicada_core::check::validate_config;
    let c = cfg(&format!(
        "{SHAPE_1}\n[[packages.app.updaters]]\nkind = \"generic\"\npath = \"x.yaml\"\n"
    ));
    assert!(validate_config(&c).iter().any(|f| f.message.contains("needs an `edit`")));
}

// --- K5 · the marker escape hatch ----------------------------------------

#[test]
fn k5_a_marker_reaches_a_file_no_dedicated_kind_covers() {
    let config = format!(
        "{SHAPE_1}\n[[packages.app.updaters]]\nkind = \"generic\"\npath = \"Dockerfile\"\nedit = {{ marker = \"x-cicada-version\" }}\n"
    );
    let r = Case::new(&config)
        .source("Cargo.toml", CARGO)
        .source("Dockerfile", "FROM acme/app:1.4.2 # x-cicada-version\nRUN true\n")
        .run(&["feat: add export"]);
    assert_eq!(
        edit(&r, "Dockerfile").content,
        "FROM acme/app:1.5.0 # x-cicada-version\nRUN true\n"
    );
}

// --- K8 · the post-write re-parse ----------------------------------------

#[test]
fn k8_an_edit_that_produces_an_unparseable_file_aborts_the_whole_render() {
    // A `| yaml`-less template writing a hostile value is the realistic route to
    // this; here the same outcome is reached directly. What matters is that the
    // failure is an `Err` from `render` — so it lands before anything is written,
    // not after (#52).
    use cicada_core::edit::EditError;
    use cicada_core::updater::yaml;
    let broken = yaml::splice("a: 1.0.0\n", "a", "x\n  - not: a scalar\n").unwrap();
    assert!(yaml::verify(&broken).is_err(), "the fixture must actually be broken");

    // And the error type carries the file, so a human knows which one.
    let e = EditError::Reparse { path: "f.yaml".into(), kind: "generic (yaml)", message: "x".into() };
    assert!(e.to_string().contains("nothing has been written"), "{e}");
}

// --- L · changelog rendering ---------------------------------------------

#[test]
fn l1_an_absent_changelog_is_created_with_a_header_and_the_entry() {
    let r = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);
    let got = &edit(&r, "CHANGELOG.md").content;
    assert!(got.starts_with("# Changelog\n"), "{got}");
    assert!(got.contains("## 1.5.0 (2026-09-21)"), "{got}");
    assert!(got.contains("### Features"), "{got}");
    assert!(got.contains("* add export"), "{got}");
}

#[test]
fn l2_l4_a_second_entry_stacks_above_the_first_without_touching_it() {
    let first = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);
    let after_first = edit(&first, "CHANGELOG.md").content.clone();

    let second = Case::new(SHAPE_1)
        .versions("[packages.app]\nversion = \"1.5.0\"\n")
        .source("Cargo.toml", &CARGO.replace("1.4.2", "1.5.0"))
        .source("CHANGELOG.md", &after_first)
        .run(&["fix: handle nil"]);
    let got = &edit(&second, "CHANGELOG.md").content;

    assert!(got.find("1.5.1").unwrap() < got.find("1.5.0").unwrap(), "newest first:\n{got}");
    // The previous entry survives byte-for-byte — changelogs are append-at-top.
    assert!(got.contains("## 1.5.0 (2026-09-21)\n\n### Features\n\n* add export"), "{got}");
}

#[test]
fn l9_the_date_is_an_input_so_the_same_run_twice_is_byte_identical() {
    let mut case = Case::new(SHAPE_1).source("Cargo.toml", CARGO);
    case.date = "2026-09-11".into();
    let a = case.run(&["feat: add export"]);
    let b = case.run(&["feat: add export"]);

    assert!(edit(&a, "CHANGELOG.md").content.contains("## 1.5.0 (2026-09-11)"));
    // #30 — the whole dry-run guarantee is that this holds.
    assert_eq!(a.edits, b.edits);
}

#[test]
fn l8_one_render_feeds_both_destinations() {
    // #43 — the changelog entry and the forge release body are the same content in
    // different frames. Two renders would drift within a month.
    let r = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);
    let body = r.notes.get(&Target::Package(PackageId(0))).expect("notes for the release");
    assert!(edit(&r, "CHANGELOG.md").content.contains(body.trim()), "body:\n{body}");
    // And the release body does not repeat the version, which the frame adds.
    assert!(!body.contains("## 1.5.0"), "{body}");
}

#[test]
fn l5_l6_a_printed_default_used_as_an_override_changes_nothing() {
    // The property that makes customisation start from the real thing (#48): copy
    // the built-in to `.cicada/templates/notes.md.j2` and the output is identical.
    let plain = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);

    let mut overridden = Case::new(SHAPE_1).source("Cargo.toml", CARGO);
    overridden
        .overrides
        .insert("notes".into(), cicada_core::template::NOTES.to_string());
    let overridden = overridden.run(&["feat: add export"]);

    assert_eq!(edit(&plain, "CHANGELOG.md").content, edit(&overridden, "CHANGELOG.md").content);
}

#[test]
fn l5_an_override_is_picked_up_by_convention_with_no_config_change() {
    let mut case = Case::new(SHAPE_1).source("Cargo.toml", CARGO);
    case.overrides.insert("notes".into(), "ENTIRELY DIFFERENT".into());
    let r = case.run(&["feat: add export"]);
    assert!(edit(&r, "CHANGELOG.md").content.contains("ENTIRELY DIFFERENT"));
}

#[test]
fn a_breaking_change_gets_its_own_section_with_the_footers_explanation() {
    let cs = vec![
        parse(&RawCommit::new(
            "c1deadbeef",
            "feat!: drop the v1 endpoints",
            "BREAKING CHANGE: /v1 is gone; use /v2",
        ))
        .unwrap(),
    ];
    let case = Case::new(SHAPE_1).source("Cargo.toml", CARGO);
    let p = make_plan(&case.config, &case.versions, &cs);
    let r = render(&RenderInput {
        config: &case.config,
        plan: &p,
        versions: &case.versions,
        date: &case.date,
        links: &case.links,
        sources: &case.sources,
        workspace_roots: &case.workspace_roots,
        overrides: &case.overrides,
    })
    .unwrap();

    let got = &edit(&r, "CHANGELOG.md").content;
    assert!(got.contains("## 2.0.0 (2026-09-21)"), "{got}");
    assert!(got.contains("BREAKING CHANGES"), "{got}");
    assert!(got.contains("/v1 is gone; use /v2"), "{got}");
}

#[test]
fn commit_links_appear_only_when_the_forge_supplied_a_style() {
    // A2 — links are a value, so `plan`/`--no-push` with no forge renders notes
    // without links rather than with broken ones.
    let plain = Case::new(SHAPE_1).source("Cargo.toml", CARGO).run(&["feat: add export"]);
    // Asserted on the entry body rather than the whole file, so this stays a
    // statement about commit links whatever the header happens to contain.
    let body = plain.notes.get(&Target::Package(PackageId(0))).unwrap();
    assert!(!body.contains(']'), "no links without a style:\n{body}");

    let mut linked = Case::new(SHAPE_1).source("Cargo.toml", CARGO);
    linked.links = LinkStyle {
        commit: Some("https://github.com/acme/app/commit/{sha}".into()),
        compare: Some("https://github.com/acme/app/compare/{from}...{to}".into()),
    };
    let linked = linked.run(&["feat: add export"]);
    let got = &edit(&linked, "CHANGELOG.md").content;
    assert!(got.contains("https://github.com/acme/app/commit/c1deadbeef"), "{got}");
    // The compare URL spells the *previous* tag through the same `tag_format`.
    assert!(got.contains("compare/v1.4.2...v1.5.0"), "{got}");
}

#[test]
fn a_first_release_has_no_compare_url_because_there_is_nothing_to_compare_with() {
    let mut case = Case::new(SHAPE_1).source("Cargo.toml", CARGO).versions("");
    case.links = LinkStyle {
        commit: None,
        compare: Some("https://github.com/acme/app/compare/{from}...{to}".into()),
    };
    let r = case.run(&["feat: add export"]);
    // `0.0.0` would produce a URL that 404s (#89).
    assert!(!edit(&r, "CHANGELOG.md").content.contains("compare/"), "{}", edit(&r, "CHANGELOG.md").content);
}

// --- the monorepo shape: umbrella changelog grouped by service -----------

const MONOREPO: &str = r#"
[settings]
release_branches = ["main"]
default_scope    = "platform"

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
"#;

#[test]
fn r1_the_umbrella_entry_groups_by_service_in_config_order() {
    let mut case = Case::new(MONOREPO);
    case.versions = VersionsFile::from_toml(
        "[umbrella]\nversion = \"1.7.0\"\n[packages.api]\nversion = \"1.4.2\"\n[packages.worker]\nversion = \"0.9.1\"\n",
    )
    .unwrap();
    let r = case.run(&["feat(api): add caching", "fix(worker): handle nil"]);

    let root = &edit(&r, "CHANGELOG.md").content;
    assert!(root.contains("## 1.8.0 (2026-09-21)"), "{root}");
    assert!(root.contains("### api 1.5.0"), "{root}");
    assert!(root.contains("### worker 0.9.2"), "{root}");
    assert!(root.find("### api").unwrap() < root.find("### worker").unwrap(), "config order");

    // #15 — each service's own changelog contains only its own.
    let api = &edit(&r, "services/api/CHANGELOG.md").content;
    assert!(api.contains("add caching"), "{api}");
    assert!(!api.contains("handle nil"), "a service changelog holds only its own:\n{api}");
}

#[test]
fn a_derived_release_explains_itself_rather_than_rendering_an_empty_entry() {
    // #97 — a chart dragged by its tracked app has no commits at all, and a
    // changelog holds only its own package's commits. Without `derived_from` the
    // entry would be blank, which reads as a rendering bug.
    let config = r#"
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"
changelog  = "CHANGELOG.md"

[packages.chart]
path            = "chart"
tag_format      = "chart-v{version}"
changelog       = "chart/CHANGELOG.md"
initial_version = "0.1.0"

[[packages.chart.updaters]]
kind        = "helm"
path        = "chart/Chart.yaml"
app_version = { track = "app" }
"#;
    let r = Case::new(config)
        .versions("[packages.app]\nversion = \"1.4.2\"\n[packages.chart]\nversion = \"0.9.0\"\n")
        .source("chart/Chart.yaml", "name: app\nversion: 0.9.0\nappVersion: \"1.4.2\"\n")
        .run(&["fix: handle nil"]);

    let chart_log = &edit(&r, "chart/CHANGELOG.md").content;
    assert!(chart_log.contains("## 0.9.1 (2026-09-21)"), "{chart_log}");
    assert!(chart_log.contains("Released to track app 1.4.3"), "{chart_log}");

    // And the chart's appVersion followed the app, quoted (#51).
    let chart = &edit(&r, "chart/Chart.yaml").content;
    assert_eq!(*chart, "name: app\nversion: 0.9.1\nappVersion: \"1.4.3\"\n");
}

// --- F · prereleases at the file level -----------------------------------

#[test]
fn f1_an_rc_writes_the_version_file_and_leaves_the_changelog_untouched() {
    // #62 — if every release wrote a cumulative entry the file would accumulate
    // rc.1, then rc.2 repeating it, then the stable repeating it again. Version
    // files *are* written, because the rc artifact has to build.
    let existing = "# Changelog\n\n<!-- cicada:insert -->\n\n## 1.4.2 (2026-01-01)\n\n* old\n";
    let r = Case::new(SHAPE_1)
        .source("Cargo.toml", CARGO)
        .source("CHANGELOG.md", existing)
        .run_pre(&["feat: add export"], Some("rc"));

    assert_eq!(edit(&r, "Cargo.toml").content, CARGO.replace("1.4.2", "1.5.0-rc.1"));
    assert!(
        r.edits.iter().all(|e| e.path != "CHANGELOG.md"),
        "the changelog must be byte-identical: {:?}",
        paths(&r)
    );

    // The versions file records the rc and leaves `version` exactly where it was
    // — `version` means *last stable release*, and no stable release happened.
    let v = &edit(&r, ".cicada-versions.toml").content;
    assert!(v.contains("version = \"1.4.2\""), "{v}");
    assert!(v.contains("prerelease = \"1.5.0-rc.1\""), "{v}");
}

#[test]
fn f5_promotion_writes_one_complete_entry_and_deletes_the_prerelease_key() {
    // The scenario the whole `version`/`prerelease` split exists to make work.
    let open = "[packages.app]\nversion = \"1.4.2\"\nprerelease = \"1.5.0-rc.2\"\n";
    let r = Case::new(SHAPE_1)
        .versions(open)
        .source("Cargo.toml", CARGO)
        .run(&["feat: add export", "fix: handle nil"]);

    let log = &edit(&r, "CHANGELOG.md").content;
    assert!(log.contains("## 1.5.0 (2026-09-21)"), "{log}");
    // Everything since the last stable, in one entry, with no rc entries and no
    // duplication.
    assert!(log.contains("add export") && log.contains("handle nil"), "{log}");
    assert!(!log.contains("-rc."), "no rc string anywhere: {log}");

    let v = &edit(&r, ".cicada-versions.toml").content;
    assert!(v.contains("version = \"1.5.0\""), "{v}");
    assert!(!v.contains("prerelease"), "the key is deleted, not emptied: {v}");
}

#[test]
fn f7_the_notes_are_cumulative_even_though_no_changelog_was_written() {
    // Reviewers and testers see the content; the file stays linear.
    let r = Case::new(SHAPE_1)
        .source("Cargo.toml", CARGO)
        .run_pre(&["feat: add export", "fix: handle nil"], Some("rc"));
    let body = r.notes.get(&Target::Package(PackageId(0))).expect("notes for the rc");
    assert!(body.contains("add export"), "{body}");
    assert!(body.contains("handle nil"), "{body}");
}

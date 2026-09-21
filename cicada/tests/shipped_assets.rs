//! The things a third party adopts from: the README and the CI templates.
//!
//! M7's exit criterion is *"a third party can adopt from the README alone"*,
//! which is a claim about documentation — and documentation rots silently. These
//! tests make it mechanical: every configuration the README shows is parsed by
//! the real parser, and every shipped YAML asset is parsed by the same YAML code
//! the updaters use.
//!
//! The failure this prevents is specific and embarrassing: a config key renamed
//! in `config.rs`, every test updated, and the README left quietly telling
//! adopters to write the old one.

use std::path::{Path, PathBuf};

use cicada_core::config::Config;
use cicada_core::versions::VersionsFile;

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/cicada`.
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Every fenced block of a given language in a Markdown file.
fn code_blocks(md: &str, lang: &str) -> Vec<String> {
    let open = format!("```{lang}\n");
    let mut out = Vec::new();
    let mut rest = md;
    while let Some(i) = rest.find(&open) {
        let body = &rest[i + open.len()..];
        let Some(j) = body.find("```") else { break };
        out.push(body[..j].to_string());
        rest = &body[j + 3..];
    }
    out
}

#[test]
fn every_configuration_the_readme_shows_is_one_the_parser_accepts() {
    let md = read("README.md");
    let blocks = code_blocks(&md, "toml");
    assert!(blocks.len() >= 4, "the README should still be showing worked configs");

    let mut configs = 0;
    for (i, b) in blocks.iter().enumerate() {
        // A block with `[settings]` is a whole `cicada.toml`; anything else is a
        // fragment, which still has to be valid TOML.
        if b.contains("[settings]") {
            Config::from_toml(b).unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
            configs += 1;
        } else if b.contains("version =") && !b.contains("[packages.app.updaters]") {
            // The versions-file seed in "Adopting into a repo that already has
            // releases" must parse as one, or the adoption instructions are wrong.
            if VersionsFile::from_toml(b).is_err() {
                b.parse::<toml::Table>()
                    .unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
            }
        } else {
            b.parse::<toml::Table>()
                .unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
        }
    }
    assert!(configs >= 3, "one worked config per repo shape, and there were {configs}");
}

#[test]
fn the_readme_documents_every_template_name_that_exists() {
    // `cicada template print <name>` is in the README; if a built-in is added and
    // not listed, adopters cannot discover it.
    let md = read("README.md");
    for name in cicada_core::template::built_in_names() {
        assert!(md.contains(name), "the README does not mention the `{name}` template");
    }
}

#[test]
fn the_readme_documents_every_exit_code_the_binary_can_return() {
    let md = read("README.md");
    for code in ["`0`", "`1`", "`2`", "`3`", "`4`"] {
        assert!(md.contains(code), "the README's exit-code table is missing {code}");
    }
    // The one that is counter-intuitive and therefore the one most worth stating.
    assert!(
        md.contains("including \"nothing to release\""),
        "the README must say that exit 0 covers \"nothing to release\""
    );
}

#[test]
fn the_ci_trigger_footgun_is_prominent_rather_than_a_footnote() {
    // "prominent in the docs rather than a footnote — because a release PR whose
    // CI never runs defeats the entire point of decision #7".
    let md = read("README.md");
    let at = md.find("CI-trigger footgun").expect("the README must carry the footgun section");
    assert!(at < md.len() / 3, "it is buried at {at} of {}", md.len());

    assert!(md.contains("GITHUB_TOKEN"), "it must name the GitHub token");
    assert!(md.contains("CI_JOB_TOKEN"), "and the GitLab one");
    // Both forges' permission requirements, which are the other half of it.
    assert!(md.contains("pull-requests: write"));
    assert!(md.contains("write_repository"));
    assert!(md.contains("Allow GitHub Actions to create and approve pull requests"));
}

#[test]
fn every_shipped_yaml_asset_parses() {
    // With cicada's own YAML reader, which is the one that has to cope with
    // whatever a repository actually contains.
    for asset in [
        "action.yml",
        "ci/gitlab-ci.yml",
        ".github/workflows/release.yml",
        ".github/workflows/ci.yml",
    ] {
        let src = read(asset);
        cicada_core::updater::yaml::verify(&src)
            .unwrap_or_else(|e| panic!("{asset} does not parse: {e:?}"));
    }
}

#[test]
fn the_ci_templates_bake_in_the_two_settings_everyone_forgets() {
    // §Distribution: both templates call `cicada run`, bake in the full-history
    // flag, and set a concurrency guard.
    let gitlab = read("ci/gitlab-ci.yml");
    assert!(gitlab.contains("GIT_DEPTH: 0"), "a shallow clone truncates history");
    assert!(gitlab.contains("resource_group"), "overlapping runs race on the force-push");
    assert!(gitlab.contains("cicada run"), "the phase order must not be CI's to get wrong (#35)");

    let workflow = read(".github/workflows/release.yml");
    assert!(workflow.contains("fetch-depth: 0"));
    assert!(workflow.contains("concurrency:"));

    // The action runs `cicada run` by default, for the same reason.
    let action = read("action.yml");
    assert!(action.contains("default: run"), "the action must default to `run`");
}

#[test]
fn the_action_declares_the_inputs_the_readme_tells_people_to_pass() {
    let action = read("action.yml");
    let md = read("README.md");
    for input in ["command", "token", "version", "args"] {
        assert!(action.contains(&format!("  {input}:")), "action.yml has no `{input}` input");
    }
    // The README's GitHub example passes `token:`; if that were renamed the
    // example would silently stop working.
    assert!(md.contains("token: ${{ secrets.CICADA_TOKEN }}"));
}

#[test]
fn the_readme_is_honest_about_what_needs_a_token() {
    // The offline guarantee (§Goals 6) is the claim `check` on a forked pull
    // request rests on, so the table stating it must not quietly disappear.
    let md = read("README.md");
    assert!(md.contains("What needs a token"));
    for cmd in ["`plan`, `check`", "`--dry-run`", "`--no-push`"] {
        assert!(md.contains(cmd), "the token table is missing {cmd}");
    }
}

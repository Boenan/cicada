//! Gathering everything `cicada_core::render` needs from the world.
//!
//! Core is pure (#29), so every file it reasons about is read here first and
//! handed in as data. That inversion is what makes `--dry-run` honest: the render
//! is the same function call either way, and the only difference is whether
//! [`apply`] is called afterwards.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cicada_core::config::{Config, ForgeKind, UpdaterKind};
use cicada_core::edit::FileEdit;
use cicada_core::notes::LinkStyle;
use cicada_core::plan::Plan;
use cicada_core::render::required_sources;
use cicada_core::template;
use cicada_core::updater::cargo;

/// Where template overrides live, by convention and with no configuration (#47).
pub const TEMPLATE_DIR: &str = ".cicada/templates";

/// Read every file the render may consult. A path that does not exist is simply
/// absent from the map — a changelog is created when missing (#5), and an updater
/// reports its own missing file by name.
pub fn sources(
    root: &Path,
    config: &Config,
    plan: &Plan,
    workspace_roots: &BTreeMap<String, String>,
) -> std::io::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut wanted = required_sources(config, plan);
    // A cargo updater whose crate inherits writes the *workspace root* manifest
    // rather than its own (#24), and core addresses it by that path — so the
    // redirect target has to be read too. Missing it makes the edit fail with
    // "no value at the file itself", which is the least helpful error in the tool.
    for target in workspace_roots.values() {
        if !wanted.contains(target) {
            wanted.push(target.clone());
        }
    }
    for rel in wanted {
        if let Ok(s) = std::fs::read_to_string(root.join(&rel)) {
            out.insert(rel, s);
        }
    }
    // The versions file is rewritten wholesale rather than edited, but it is read
    // so an unchanged one produces no edit at all.
    if let Ok(s) = std::fs::read_to_string(root.join(cicada_core::render::VERSIONS_FILE)) {
        out.insert(cicada_core::render::VERSIONS_FILE.to_string(), s);
    }
    Ok(out)
}

/// Template overrides found at `.cicada/templates/<name>.j2`, by built-in name.
///
/// Picked up by convention with no config change (#47, L5). The header is a plain
/// `.md` rather than a template, so it is looked for under both spellings.
pub fn overrides(root: &Path) -> BTreeMap<String, String> {
    let dir = root.join(TEMPLATE_DIR);
    let mut out = BTreeMap::new();
    for name in template::built_in_names() {
        for file in [format!("{name}.md.j2"), format!("{name}.j2"), format!("{name}.md")] {
            if let Ok(s) = std::fs::read_to_string(dir.join(&file)) {
                out.insert(name.to_string(), s);
                break;
            }
        }
    }
    out
}

/// For every cargo updater whose manifest inherits its version, the workspace root
/// manifest that actually holds the number (#24).
///
/// The map is keyed by the *declared* path and contains an entry **only** when the
/// redirect applies, so core's lookup is the whole rule rather than a condition it
/// would need the filesystem to evaluate.
pub fn workspace_roots(root: &Path, config: &Config, plan: &Plan) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in &plan.releases {
        let cicada_core::config::Target::Package(id) = r.target else { continue };
        let Some((_, pkg)) = config.packages.get_index(id.0) else { continue };
        for u in &pkg.updaters {
            if u.kind != UpdaterKind::Cargo {
                continue;
            }
            let Some(declared) = &u.path else { continue };
            let Ok(src) = std::fs::read_to_string(root.join(declared)) else { continue };
            if !cargo::inherits_workspace_version(&src) {
                continue;
            }
            if let Some(ws) = find_workspace_root(root, Path::new(declared)) {
                out.insert(declared.clone(), ws);
            }
        }
    }
    out
}

/// Walk up from a member manifest looking for the `[workspace]` that owns it.
fn find_workspace_root(root: &Path, member: &Path) -> Option<String> {
    let mut dir = member.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate != member
            && let Ok(src) = std::fs::read_to_string(root.join(&candidate))
            && src.parse::<toml::Table>().is_ok_and(|t| t.contains_key("workspace"))
        {
            return Some(candidate.to_string_lossy().replace('\\', "/"));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Compare and commit URL templates for the configured forge (A2).
///
/// Built here rather than in core, which must not know what a forge is. In M4 the
/// adapter supplies this instead; the *value* it supplies has this shape, which is
/// why core takes a value and never a trait object.
pub fn link_style(config: &Config, repo: Option<&str>) -> LinkStyle {
    let Some(repo) = repo.or(config.forge.repo.as_deref()) else { return LinkStyle::default() };
    let host = web_host(config);
    match config.forge.kind {
        ForgeKind::Github => LinkStyle {
            commit: Some(format!("{host}/{repo}/commit/{{sha}}")),
            compare: Some(format!("{host}/{repo}/compare/{{from}}...{{to}}")),
        },
        ForgeKind::Gitlab => LinkStyle {
            commit: Some(format!("{host}/{repo}/-/commit/{{sha}}")),
            compare: Some(format!("{host}/{repo}/-/compare/{{from}}...{{to}}")),
        },
    }
}

/// The *web* host, which is not the API host. `api_url` points at the API — on
/// GitHub Enterprise that is `https://host/api/v3`, and linking a human there
/// gives them JSON.
fn web_host(config: &Config) -> String {
    match &config.forge.api_url {
        None => match config.forge.kind {
            ForgeKind::Github => "https://github.com".into(),
            ForgeKind::Gitlab => "https://gitlab.com".into(),
        },
        Some(api) => {
            let trimmed = api.trim_end_matches('/');
            let trimmed = trimmed.strip_suffix("/api/v3").unwrap_or(trimmed);
            let trimmed = trimmed.strip_suffix("/api/v4").unwrap_or(trimmed);
            // `api.github.com` is the one host whose web form is a different name
            // rather than a shorter path.
            trimmed.replace("https://api.github.com", "https://github.com")
        }
    }
}

/// `org/repo` from a remote URL, in either the SSH or the HTTPS spelling.
///
/// Derivation is fine *here* — a link that is slightly wrong costs a 404 in a
/// changelog. It is never how `forge.kind` is decided (#66), which is a question
/// about where cicada may write.
pub fn repo_from_remote(url: &str) -> Option<String> {
    let s = url.trim().trim_end_matches(".git");
    let path = if let Some(rest) = s.split_once("://").map(|(_, r)| r) {
        // https://host/org/repo  → org/repo
        rest.split_once('/').map(|(_, p)| p)?
    } else if let Some((_, rest)) = s.split_once(':') {
        // git@host:org/repo → org/repo
        rest
    } else {
        return None;
    };
    let path = path.trim_matches('/');
    if path.contains('/') { Some(path.to_string()) } else { None }
}

/// Write the edits. The **only** thing that distinguishes a dry run from a real
/// one (#30).
pub fn apply(root: &Path, edits: &[FileEdit]) -> std::io::Result<()> {
    for e in edits {
        let path: PathBuf = root.join(&e.path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, &e.content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_is_read_out_of_either_remote_spelling() {
        assert_eq!(repo_from_remote("git@github.com:acme/app.git").as_deref(), Some("acme/app"));
        assert_eq!(repo_from_remote("https://github.com/acme/app.git").as_deref(), Some("acme/app"));
        assert_eq!(repo_from_remote("https://github.com/acme/app").as_deref(), Some("acme/app"));
        // GitLab subgroups are ordinary path segments.
        assert_eq!(
            repo_from_remote("git@gitlab.com:group/sub/proj.git").as_deref(),
            Some("group/sub/proj")
        );
        assert_eq!(repo_from_remote("/a/local/path"), None);
    }

    fn cfg(extra: &str) -> Config {
        Config::from_toml(&format!(
            "[settings]\nrelease_branches=[\"main\"]\ndefault_scope=\"a\"\n[forge]\nkind=\"github\"\n{extra}\n[packages.a]\npath=\".\"\ntag_format=\"v{{version}}\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn links_point_at_the_web_host_not_the_api_host() {
        let ls = link_style(&cfg("repo=\"acme/app\""), None);
        assert_eq!(ls.commit_url("abc").unwrap(), "https://github.com/acme/app/commit/abc");

        // GitHub Enterprise: linking a human at `/api/v3` gives them JSON.
        let ls = link_style(&cfg("repo=\"acme/app\"\napi_url=\"https://ghe.example/api/v3\""), None);
        assert_eq!(ls.commit_url("abc").unwrap(), "https://ghe.example/acme/app/commit/abc");
    }

    #[test]
    fn no_repo_means_no_links_rather_than_broken_ones() {
        assert_eq!(link_style(&cfg(""), None), LinkStyle::default());
    }
}

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
use cicada_core::render::{LockTarget, required_sources};
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
    lock_targets: &BTreeMap<String, LockTarget>,
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
    // Same reason, one layer down: the lockfile is edited, so it has to be read.
    for lock in lock_targets.values() {
        if !wanted.contains(&lock.path) {
            wanted.push(lock.path.clone());
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

/// For every cargo updater, the `Cargo.lock` its bump makes stale and the crates
/// in it that move (#24 again, one layer down).
///
/// Cargo records each workspace member's own version in the lock, so bumping a
/// manifest without the lock leaves a repo that fails `cargo build --locked` —
/// and a release artifact build is precisely where `--locked` gets used, so the
/// failure surfaces *after* the tag exists.
///
/// Two cases, and the second is the one that bites: a crate with its own
/// `[package] version` moves alone, but a crate that inherits moves every other
/// member that inherits too, because they all read the one
/// `[workspace.package] version`. cicada's own repo is the second kind —
/// `cicada` and `cicada-core` both inherit — so fixing only the declared crate
/// would still not build.
pub fn lock_targets(
    root: &Path,
    config: &Config,
    plan: &Plan,
    workspace_roots: &BTreeMap<String, String>,
) -> BTreeMap<String, LockTarget> {
    let mut out = BTreeMap::new();
    for r in &plan.releases {
        let cicada_core::config::Target::Package(id) = r.target else { continue };
        let Some((_, pkg)) = config.packages.get_index(id.0) else { continue };
        for u in &pkg.updaters {
            if u.kind != UpdaterKind::Cargo {
                continue;
            }
            let Some(declared) = u.path.as_deref() else { continue };

            // The lock lives beside the workspace root when there is one, and
            // beside the manifest otherwise.
            let anchor = workspace_roots
                .get(declared)
                .cloned()
                .or_else(|| find_workspace_root(root, Path::new(declared)))
                .unwrap_or_else(|| declared.to_string());
            let dir = Path::new(&anchor).parent().unwrap_or(Path::new(""));
            let lock = dir.join("Cargo.lock");
            if !root.join(&lock).is_file() {
                // No committed lockfile. Ordinary for a library, and nothing to
                // keep in step.
                continue;
            }

            let crates = if workspace_roots.contains_key(declared) {
                inheriting_members(root, dir)
            } else {
                crate_name(root, Path::new(declared)).into_iter().collect()
            };
            if crates.is_empty() {
                continue;
            }
            out.insert(
                declared.to_string(),
                LockTarget { path: lock.to_string_lossy().to_string(), crates },
            );
        }
    }
    out
}

/// `[package] name` from a manifest.
fn crate_name(root: &Path, manifest: &Path) -> Option<String> {
    let src = std::fs::read_to_string(root.join(manifest)).ok()?;
    let t: toml::Table = src.parse().ok()?;
    t.get("package")?.get("name")?.as_str().map(str::to_string)
}

/// Every crate under a workspace root that takes its version from it.
///
/// Found by walking rather than by resolving the `members` globs, because a
/// manifest saying `version.workspace = true` is already the answer — cargo
/// rejects it in a crate that is not a member, so the property is self-proving
/// and needs no glob engine.
///
/// The walk stops at a nested `[workspace]`: `spikes/` in this very repo carries
/// its own, and its crates belong to it rather than to the root.
fn inheriting_members(root: &Path, dir: &Path) -> Vec<String> {
    fn walk(root: &Path, rel: &Path, depth: usize, out: &mut Vec<String>) {
        if depth > 8 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(root.join(rel)) else { return };
        let mut dirs = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy().to_string();
            if e.path().is_dir() {
                // Build output and VCS internals hold no manifests worth reading,
                // and `target/` holds thousands.
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    dirs.push(rel.join(&name));
                }
            }
        }
        if let Ok(src) = std::fs::read_to_string(root.join(rel).join("Cargo.toml"))
            && cargo::inherits_workspace_version(&src)
            && let Some(n) = src
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| Some(t.get("package")?.get("name")?.as_str()?.to_string()))
        {
            out.push(n);
        }
        for d in dirs {
            // A nested workspace owns what is under it.
            let nested = std::fs::read_to_string(root.join(&d).join("Cargo.toml"))
                .ok()
                .and_then(|s| s.parse::<toml::Table>().ok())
                .is_some_and(|t| t.contains_key("workspace"));
            if nested {
                continue;
            }
            walk(root, &d, depth + 1, out);
        }
    }

    let mut out = Vec::new();
    walk(root, dir, 0, &mut out);
    out.sort();
    out.dedup();
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

/// Where the release manifest lands on disk (#42). Gitignored, **never
/// committed** — it describes a run, not a state of the repository.
pub const MANIFEST_PATH: &str = "target/cicada/release-manifest.json";

/// Write the manifest beside the build output.
///
/// On disk *as well as* embedded in every release body, for two reasons: under
/// `--no-push` there is no release object at all, so the file is the only way to
/// inspect a rehearsal (§Goals 6); and the deploy job that runs after the release
/// job can read a file in the workspace without a token and an API round trip.
pub fn write_manifest(
    root: &Path,
    manifest: &cicada_core::manifest::Manifest,
) -> std::io::Result<PathBuf> {
    let path = root.join(MANIFEST_PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, manifest.to_json())?;
    Ok(path)
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

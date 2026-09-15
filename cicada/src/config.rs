//! Loading `cicada.toml` from disk, and the validations that need a filesystem.
//!
//! The types and every pure validation live in `cicada-core` (#29). What is here
//! is what that crate cannot do: open files. The two halves append to the same
//! `Vec<Finding>`, so a caller sees one list.

use std::path::{Path, PathBuf};

use cicada_core::check::Finding;
use cicada_core::config::{Config, UpdaterKind};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("no cicada.toml at {}", .0.display())]
    Missing(PathBuf),
    #[error("could not read {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A config that will not parse is exit `1`, never a finding (#79).
    #[error("cicada.toml: {0}")]
    Parse(String),
}

pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
}

/// Read and parse. `--config` overrides the default location, which is beside
/// `.cicada-versions.toml` at the repo root.
pub fn load(repo_root: &Path, override_path: Option<&Path>) -> Result<Loaded, LoadError> {
    let path = match override_path {
        Some(p) => p.to_path_buf(),
        None => repo_root.join("cicada.toml"),
    };
    if !path.exists() {
        return Err(LoadError::Missing(path));
    }
    let src = std::fs::read_to_string(&path)
        .map_err(|source| LoadError::Io { path: path.clone(), source })?;
    let config = Config::from_toml(&src).map_err(|e| LoadError::Parse(e.to_string()))?;
    Ok(Loaded { config, path })
}

/// Validations that need to look at the tree (design §Loading and validation).
pub fn validate_on_disk(config: &Config, repo_root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();

    for (name, pkg) in &config.packages {
        let dir = repo_root.join(&pkg.path);
        if !dir.is_dir() {
            out.push(Finding::problem(
                format!("packages.{name}.path"),
                format!("`{}` does not exist in the repository", pkg.path),
            ));
        }
        for (i, u) in pkg.updaters.iter().enumerate() {
            let key = format!("packages.{name}.updaters[{i}]");
            if let Some(p) = &u.path
                && !repo_root.join(p).is_file()
            {
                out.push(Finding::problem(format!("{key}.path"), format!("`{p}` does not exist")));
            }
            // Template and output paths resolve from the **repo root**, never the
            // package's path (#44) — packages already declare their own.
            if let Some(t) = &u.template
                && !repo_root.join(t).is_file()
            {
                out.push(
                    Finding::problem(format!("{key}.template"), format!("`{t}` does not exist"))
                        .with_help("template paths resolve from the repo root, not the package path"),
                );
            }
        }
    }

    out.extend(check_cargo_workspace_inheritance(config, repo_root));
    out
}

/// The Cargo workspace-inheritance trap (#24).
///
/// Two crates with `version.workspace = true` resolve to a single field —
/// `[workspace.package] version` in the workspace root — so they cannot be bumped
/// independently. Releasing one would silently move the other. `check` is where
/// that unreleasable arrangement becomes a legible error rather than a surprise
/// at the first release.
fn check_cargo_workspace_inheritance(config: &Config, repo_root: &Path) -> Vec<Finding> {
    let mut inheriting: Vec<&str> = Vec::new();
    let mut out = Vec::new();

    for (name, pkg) in &config.packages {
        for u in &pkg.updaters {
            if u.kind != UpdaterKind::Cargo {
                continue;
            }
            let Some(rel) = &u.path else { continue };
            let Ok(src) = std::fs::read_to_string(repo_root.join(rel)) else { continue };
            let Ok(doc) = src.parse::<toml::Table>() else { continue };
            if inherits_workspace_version(&doc) {
                inheriting.push(name.as_str());
            }
        }
    }

    if inheriting.len() > 1 {
        out.push(
            Finding::problem(
                "packages",
                format!(
                    "{} resolve to the same Cargo version field",
                    inheriting
                        .iter()
                        .map(|n| format!("'{n}'"))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            )
            .with_help(
                "each sets `version.workspace = true`, so all of them read `[workspace.package] version` \
                 in the workspace root and cannot be bumped independently",
            ),
        );
    }
    out
}

/// Both spellings Cargo accepts: `version.workspace = true` and
/// `version = { workspace = true }` are the same key written two ways.
fn inherits_workspace_version(doc: &toml::Table) -> bool {
    doc.get("package")
        .and_then(|p| p.as_table())
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("workspace"))
        .and_then(|w| w.as_bool())
        .unwrap_or(false)
}

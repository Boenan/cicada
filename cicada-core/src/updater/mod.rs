//! Updaters — the pure `&str -> String` transforms behind every file cicada writes.
//!
//! A package owns a **list** of updaters (#13), which collapses what were three
//! separate concepts (`type`, `extra_files`, `templates`) into one uniform
//! mechanism and extends to Go, Java and Python later by adding `kind` values with
//! no schema change.
//!
//! | kind | Writes |
//! |---|---|
//! | `cargo` | `Cargo.toml` `version`, redirected to `[workspace.package]` when the crate inherits |
//! | `helm` | `Chart.yaml` `version`, and `appVersion` when it tracks an app |
//! | `generic` | one value in any file, by path or by marker comment (#50) |
//! | `template` | a whole file — handled in [`crate::render`], where the template environment lives |
//!
//! **Everything here is pure** (A5), which is what makes the comment-preserving
//! editing — the fiddliest code in the project — also the easiest to test. Reading
//! and writing the file is three lines outside it.

pub mod cargo;
pub mod json;
pub mod marker;
pub mod toml_path;
pub mod yaml;

mod helm;

pub use helm::{HELM_APP_VERSION_KEY, HELM_VERSION_KEY};

use semver::Version;

use crate::config::{Edit, Updater, UpdaterKind};
use crate::edit::EditError;

/// What one updater produced.
pub struct Outcome {
    pub content: String,
    /// Reported to the human, never fatal. The one case today is an unquoted
    /// `appVersion` found in place (#51).
    pub warnings: Vec<String>,
}

/// Apply one updater to the content currently destined for its path.
///
/// `current` is the content *as the run has it so far*, not necessarily what is on
/// disk — two updaters may legitimately edit different keys of one `values.yaml`,
/// and folding them in order is what lets that work without last-writer-wins.
///
/// `template` is absent from this dispatch: it renders a whole file and needs the
/// template environment, so it lives in [`crate::render`].
pub fn apply(
    u: &Updater,
    path: &str,
    current: &str,
    version: &Version,
    tracked: Option<&Version>,
    cargo_workspace: bool,
) -> Result<Outcome, EditError> {
    match u.kind {
        UpdaterKind::Cargo => {
            // `cargo_workspace` says this edit was *redirected* here because the
            // declared crate inherits (#24), so the number to move is
            // `[workspace.package] version` rather than `[package] version`.
            //
            // It cannot be re-derived from `current`: a workspace root may be a
            // real package as well, carrying both keys, and the two answers
            // differ. Whether the redirect happened is knowledge the caller has
            // and the content does not.
            let content = if cargo_workspace {
                cargo::set_workspace_version(path, current, version)?
            } else {
                cargo::set_package_version(path, current, version)?
            };
            cargo::verify(path, &content)?;
            Ok(Outcome { content, warnings: Vec::new() })
        }

        UpdaterKind::Helm => {
            let (content, warnings) = helm::update(path, current, version, tracked)?;
            yaml_verify(path, &content, "helm")?;
            Ok(Outcome { content, warnings })
        }

        UpdaterKind::Generic => {
            let edit = u.edit.as_ref().ok_or_else(|| EditError::Invalid {
                path: path.to_string(),
                message: "a generic updater needs an `edit`, e.g. `edit = { yaml = \"image.tag\" }`"
                    .into(),
            })?;
            let v = version.to_string();
            let content = match edit {
                Edit::Toml(addr) => {
                    let out = toml_path::splice(path, current, addr, &v)?;
                    toml_path::verify(path, &out)?;
                    out
                }
                Edit::Yaml(addr) => {
                    let out = yaml::splice(current, addr, &v)
                        .map_err(|e| yaml_error(path, addr, e))?;
                    yaml_verify(path, &out, "generic (yaml)")?;
                    out
                }
                Edit::Json(addr) => {
                    let out = json::splice(current, addr, &v).map_err(|e| match e {
                        json::JsonError::Parse(m) => {
                            EditError::Parse { path: path.to_string(), message: m.to_string() }
                        }
                        json::JsonError::NotFound => EditError::NotFound {
                            path: path.to_string(),
                            address: addr.clone(),
                        },
                        json::JsonError::NotAString => EditError::Unaddressable {
                            path: path.to_string(),
                            address: addr.clone(),
                            message: "is not a string, and cicada will not change a value's type",
                        },
                    })?;
                    json::verify(path, &out)?;
                    out
                }
                Edit::Marker(m) => marker::splice(path, current, m, &v)?,
            };
            Ok(Outcome { content, warnings: Vec::new() })
        }

        // Rendered in `render`, which owns the template environment.
        UpdaterKind::Template => Err(EditError::Invalid {
            path: path.to_string(),
            message: "a template updater is rendered rather than spliced".into(),
        }),
    }
}

fn yaml_error(path: &str, address: &str, e: yaml::YamlError) -> EditError {
    match e {
        yaml::YamlError::Parse(m) => EditError::Parse { path: path.to_string(), message: m },
        yaml::YamlError::NotFound => {
            EditError::NotFound { path: path.to_string(), address: address.to_string() }
        }
        yaml::YamlError::Unaddressable(message) => EditError::Unaddressable {
            path: path.to_string(),
            address: address.to_string(),
            message,
        },
    }
}

/// #52 — every written file is re-parsed before anything is committed. A splice
/// that produces an unparseable file aborts the run, which is cheap insurance
/// against exactly the class of bug byte-splicing can have.
fn yaml_verify(path: &str, content: &str, kind: &'static str) -> Result<(), EditError> {
    yaml::verify(content).map_err(|e| EditError::Reparse {
        path: path.to_string(),
        kind,
        message: match e {
            yaml::YamlError::Parse(m) => m,
            other => format!("{other:?}"),
        },
    })
}

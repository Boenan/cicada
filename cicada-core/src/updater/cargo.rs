//! The `cargo` updater — `Cargo.toml` `version`.
//!
//! `toml_edit` already does for TOML what the YAML module has to do by hand
//! (#49): it preserves formatting, comments, key order and whitespace because it
//! edits a document model rather than re-emitting one.
//!
//! **Workspace inheritance is a path decision, not a transform** (#24). A crate
//! with `version.workspace = true` has no version of its own — the number lives in
//! `[workspace.package]` in the workspace root manifest, a *different file*.
//! Finding that file means walking the tree, which is I/O, so the split is:
//! [`inherits_workspace_version`] answers the question here, purely, and the
//! binary decides which file to hand to which of the two setters.

use semver::Version;
use toml_edit::DocumentMut;

use crate::edit::EditError;

fn parse(path: &str, src: &str) -> Result<DocumentMut, EditError> {
    src.parse::<DocumentMut>()
        .map_err(|e| EditError::Parse { path: path.to_string(), message: e.to_string() })
}

/// Does this manifest take its version from the workspace root?
///
/// Both spellings Cargo accepts, because they are the same key written two ways:
/// `version.workspace = true` and `version = { workspace = true }`.
pub fn inherits_workspace_version(src: &str) -> bool {
    let Ok(doc) = src.parse::<DocumentMut>() else { return false };
    doc.get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.get("workspace"))
        .and_then(|w| w.as_bool())
        .unwrap_or(false)
}

/// Rewrite `[package] version`.
pub fn set_package_version(path: &str, src: &str, v: &Version) -> Result<String, EditError> {
    let mut doc = parse(path, src)?;
    let pkg = doc.get_mut("package").and_then(|p| p.as_table_like_mut()).ok_or_else(|| {
        EditError::Invalid {
            path: path.to_string(),
            message: "no `[package]` table — a cargo updater needs a real manifest, not a \
                      virtual workspace root"
                .into(),
        }
    })?;
    let item = pkg.get_mut("version").ok_or_else(|| EditError::NotFound {
        path: path.to_string(),
        address: "package.version".into(),
    })?;
    // Preserves the trailing comment on the edited line (#49) — see
    // `toml_path::replace_preserving_decor` for why the obvious spelling loses it.
    super::toml_path::replace_preserving_decor(item, &v.to_string());
    Ok(doc.to_string())
}

/// Rewrite `[workspace.package] version` — where an inheriting crate's number
/// actually lives.
pub fn set_workspace_version(path: &str, src: &str, v: &Version) -> Result<String, EditError> {
    let mut doc = parse(path, src)?;
    let item = doc
        .get_mut("workspace")
        .and_then(|w| w.get_mut("package"))
        .and_then(|p| p.as_table_like_mut())
        .and_then(|p| p.get_mut("version"))
        .ok_or_else(|| EditError::NotFound {
            path: path.to_string(),
            address: "workspace.package.version".into(),
        })?;
    super::toml_path::replace_preserving_decor(item, &v.to_string());
    Ok(doc.to_string())
}

/// #52 — every written file is re-parsed before anything is committed.
pub fn verify(path: &str, src: &str) -> Result<(), EditError> {
    src.parse::<DocumentMut>().map(|_| ()).map_err(|e| EditError::Reparse {
        path: path.to_string(),
        kind: "cargo",
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"# a comment that must survive
[package]
name    = "api"          # aligned, and it stays aligned
version = "1.4.2"        # the comment on the edited line is the one most likely lost
edition = "2024"

[dependencies]
serde = "1"
"#;

    #[test]
    fn k1_every_byte_outside_the_version_is_identical() {
        let out = set_package_version("Cargo.toml", MANIFEST, &Version::new(1, 5, 0)).unwrap();
        assert_eq!(out, MANIFEST.replace(r#"version = "1.4.2""#, r#"version = "1.5.0""#));
        // Spelled out, because "preserves formatting" is the whole claim.
        assert!(out.starts_with("# a comment that must survive\n"));
        assert!(out.contains(r#"name    = "api"          # aligned, and it stays aligned"#));
        // The edited line keeps its own trailing comment — the case `*item = value(..)`
        // silently drops.
        assert!(
            out.contains(r#"version = "1.5.0"        # the comment on the edited line is the one most likely lost"#),
            "{out}"
        );
        assert!(out.ends_with("serde = \"1\"\n"));
    }

    #[test]
    fn workspace_inheritance_is_detected_in_both_spellings() {
        assert!(inherits_workspace_version("[package]\nversion.workspace = true\n"));
        assert!(inherits_workspace_version("[package]\nversion = { workspace = true }\n"));
        assert!(!inherits_workspace_version("[package]\nversion = \"1.0.0\"\n"));
        assert!(!inherits_workspace_version("[package]\nname = \"x\"\n"));
    }

    #[test]
    fn the_workspace_setter_writes_where_an_inheriting_crate_actually_reads() {
        let root = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n";
        let out = set_workspace_version("Cargo.toml", root, &Version::new(0, 2, 0)).unwrap();
        assert!(out.contains("version = \"0.2.0\""), "{out}");
        assert!(out.contains("edition = \"2024\""), "{out}");
        assert!(out.contains("members = [\"a\"]"), "{out}");
    }

    #[test]
    fn a_virtual_manifest_named_by_a_cargo_updater_says_so() {
        // The likeliest misconfiguration: pointing a cargo updater at a workspace
        // root that declares no package of its own.
        let err = set_package_version("Cargo.toml", "[workspace]\nmembers = []\n", &Version::new(1, 0, 0))
            .unwrap_err();
        assert!(err.to_string().contains("virtual workspace root"), "{err}");
    }

    #[test]
    fn a_manifest_that_will_not_parse_names_the_file() {
        let err = set_package_version("a/Cargo.toml", "[package\n", &Version::new(1, 0, 0)).unwrap_err();
        assert!(matches!(err, EditError::Parse { .. }));
        assert!(err.to_string().starts_with("a/Cargo.toml:"), "{err}");
    }
}

//! `.cicada-versions.toml` — the tool-owned file (design §Configuration).
//!
//! **A version and nothing else** (#1). No commit SHA is recorded here or
//! anywhere else: the tag is the only anchor (#2), so there is no degraded state
//! to fall back into when one is missing.
//!
//! It is also the **sole source of truth** for versions (#75). `Cargo.toml` and
//! `Chart.yaml` are outputs, and are never read back to seed a computation — which
//! is why a first release computed as `0.0.1` rewrites a `cargo new` manifest from
//! `0.1.0` *downwards* and that is correct rather than a bug (A9).
//!
//! "Never released" is an **absent entry** (#4). There is no sentinel version, so
//! `Option` is the entire representation and `0.0.0` keeps its ordinary meaning.
//!
//! Reading the file is the binary's job (#29); this module only knows its shape.

use indexmap::IndexMap;
use semver::Version;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VersionsFile {
    /// Present only if the repo declares an umbrella (#73).
    #[serde(default)]
    pub umbrella: Option<Entry>,
    #[serde(default)]
    pub packages: IndexMap<String, Entry>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The last **stable** release — and therefore the anchor (#60). The presence
    /// of `prerelease` does not change what this field means, which is the whole
    /// reason prereleases cost one extra field rather than a new anchor model.
    pub version: Version,
    /// Present only while a prerelease line is open; deleted when the stable
    /// release cuts (#60). Every release computes from `version` regardless, so
    /// notes stay cumulative (#61) — this tracks only which counter is current.
    #[serde(default)]
    pub prerelease: Option<Version>,
}

/// A versions file that would not parse. Distinct from a *finding*: cicada cannot
/// proceed without knowing where each line stands, so this is exit `1` (#79).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(String);

impl VersionsFile {
    /// Parse from a string. An **absent file** is not this function's problem —
    /// the binary maps that to [`VersionsFile::default`], because absent and empty
    /// mean the same thing: nothing has been released (#4).
    pub fn from_toml(src: &str) -> Result<Self, ParseError> {
        toml::from_str(src).map_err(|e| ParseError(e.to_string()))
    }

    pub fn package(&self, name: &str) -> Option<&Entry> {
        self.packages.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_means_nothing_has_been_released() {
        let v = VersionsFile::from_toml("").unwrap();
        assert!(v.umbrella.is_none());
        assert!(v.packages.is_empty());
    }

    #[test]
    fn a_version_and_nothing_else() {
        let v = VersionsFile::from_toml(
            r#"
            [umbrella]
            version = "1.7.0"

            [packages.api]
            version = "1.4.2"
            "#,
        )
        .unwrap();
        assert_eq!(v.umbrella.as_ref().unwrap().version, Version::new(1, 7, 0));
        assert_eq!(v.package("api").unwrap().version, Version::new(1, 4, 2));
        assert!(v.package("api").unwrap().prerelease.is_none());
        // Absent entry, not a sentinel (#4).
        assert!(v.package("worker").is_none());
    }

    #[test]
    fn an_open_prerelease_line_leaves_version_meaning_the_last_stable() {
        let v = VersionsFile::from_toml(
            r#"
            [packages.api]
            version    = "1.4.2"
            prerelease = "1.5.0-rc.2"
            "#,
        )
        .unwrap();
        let e = v.package("api").unwrap();
        assert_eq!(e.version, Version::new(1, 4, 2));
        assert_eq!(e.prerelease.as_ref().unwrap().to_string(), "1.5.0-rc.2");
    }

    #[test]
    fn a_stray_key_is_an_error_rather_than_a_silent_no_op() {
        // Same rule as cicada.toml (#67). This file is written by cicada, so a
        // stray key here means a human edited it — exactly when a typo must be loud.
        let err = VersionsFile::from_toml("[packages.api]\nversion = \"1.0.0\"\nsha = \"deadbeef\"\n")
            .unwrap_err();
        assert!(err.to_string().contains("sha"), "{err}");
    }

    #[test]
    fn a_version_that_is_not_semver_names_itself() {
        let err = VersionsFile::from_toml("[packages.api]\nversion = \"1.4\"\n").unwrap_err();
        assert!(err.to_string().contains("version"), "{err}");
    }
}

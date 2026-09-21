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

    /// Serialise. Hand-written rather than via serde for two reasons: this file is
    /// read by humans during a manual release (J4) and by other tooling, so its
    /// shape is a small contract; and **a version and nothing else** (#1) is
    /// easier to keep true when the writer says so in ten lines than when it is a
    /// derive that would happily emit any field added later.
    ///
    /// Emission order is the map's order, and the caller builds the map in config
    /// order — so the output is deterministic (#31) and a re-run produces byte
    /// identical content.
    pub fn to_toml(&self) -> String {
        let mut s = String::from(
            "# Managed by cicada. A version and nothing else (#1).\n\
             # Hand-edit upwards to cut a release by hand; editing downwards is refused (#76).\n",
        );
        if let Some(u) = &self.umbrella {
            s.push_str("\n[umbrella]\n");
            push_entry(&mut s, u);
        }
        for (name, e) in &self.packages {
            s.push_str(&format!("\n[packages.{}]\n", quote_key(name)));
            push_entry(&mut s, e);
        }
        s
    }
}

fn push_entry(s: &mut String, e: &Entry) {
    s.push_str(&format!("version = \"{}\"\n", e.version));
    // Written only while a line is open, and deleted when the stable cuts (#60) —
    // so the key's *absence* is meaningful and must not become `prerelease = \"\"`.
    if let Some(p) = &e.prerelease {
        s.push_str(&format!("prerelease = \"{p}\"\n"));
    }
}

/// Package names are free strings in `cicada.toml`, so one may need quoting as a
/// TOML key. Bare keys are the common case and stay bare, because this file is
/// read by humans.
fn quote_key(name: &str) -> String {
    let bare = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if bare { name.to_string() } else { format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\"")) }
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

#[cfg(test)]
mod write_tests {
    use super::*;

    fn round_trip(src: &str) -> VersionsFile {
        let once = VersionsFile::from_toml(src).unwrap();
        let text = once.to_toml();
        let twice = VersionsFile::from_toml(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(once, twice, "what cicada writes, cicada reads back:\n{text}");
        twice
    }

    #[test]
    fn what_is_written_parses_back_identically() {
        round_trip(
            r#"
            [umbrella]
            version = "1.7.0"

            [packages.api]
            version    = "1.4.2"
            prerelease = "1.5.0-rc.2"

            [packages.api-chart]
            version = "0.9.0"
            "#,
        );
    }

    #[test]
    fn an_absent_prerelease_stays_absent_rather_than_becoming_empty() {
        // The key's absence is what says the line is closed (#60), so it must not
        // round-trip into `prerelease = ""`.
        let v = VersionsFile::from_toml("[packages.api]\nversion = \"1.4.2\"\n").unwrap();
        let text = v.to_toml();
        assert!(!text.contains("prerelease"), "{text}");
        assert!(text.contains("version = \"1.4.2\""), "{text}");
    }

    #[test]
    fn an_empty_file_is_still_valid_toml() {
        let text = VersionsFile::default().to_toml();
        assert_eq!(VersionsFile::from_toml(&text).unwrap(), VersionsFile::default());
    }

    #[test]
    fn writing_is_deterministic_and_order_preserving() {
        let v = VersionsFile::from_toml(
            "[packages.zebra]\nversion = \"1.0.0\"\n[packages.apple]\nversion = \"2.0.0\"\n",
        )
        .unwrap();
        let text = v.to_toml();
        // Map order, not alphabetical — the caller builds the map in config order
        // and that is the repo's one observable ordering (#31).
        assert!(text.find("zebra").unwrap() < text.find("apple").unwrap(), "{text}");
        assert_eq!(text, v.to_toml());
    }

    #[test]
    fn a_package_name_needing_quotes_gets_them() {
        let mut v = VersionsFile::default();
        v.packages.insert(
            "my.pkg".into(),
            Entry { version: Version::new(1, 0, 0), prerelease: None },
        );
        let text = v.to_toml();
        assert!(text.contains("[packages.\"my.pkg\"]"), "{text}");
        assert_eq!(VersionsFile::from_toml(&text).unwrap().package("my.pkg").unwrap().version,
                   Version::new(1, 0, 0));
    }
}

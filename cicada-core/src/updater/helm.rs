//! The `helm` updater — `Chart.yaml` `version` and `appVersion`.
//!
//! **`appVersion` is always written quoted** (#51). Helm treats it as a string,
//! but YAML does not:
//!
//! ```yaml
//! appVersion: 1.10      # unquoted → YAML parses this as the float 1.1
//! ```
//!
//! Byte-splicing faithfully preserves whatever quoting it finds, *including none*
//! — so faithfulness is the wrong default here, and the updater overrides the
//! style rather than inheriting it. An unquoted value found in place is warned
//! about, because the file was already wrong before cicada touched it.
//!
//! `version` keeps whatever style it has: a chart version is `0.9.1`, which YAML
//! reads as a string either way.

use saphyr_parser::ScalarStyle;
use semver::Version;

use crate::edit::EditError;

use super::yaml;

pub const HELM_VERSION_KEY: &str = "version";
pub const HELM_APP_VERSION_KEY: &str = "appVersion";

pub fn update(
    path: &str,
    src: &str,
    version: &Version,
    tracked: Option<&Version>,
) -> Result<(String, Vec<String>), EditError> {
    let mut warnings = Vec::new();

    let mut out = yaml::splice(src, HELM_VERSION_KEY, &version.to_string())
        .map_err(|e| super::yaml_error(path, HELM_VERSION_KEY, e))?;

    if let Some(app) = tracked {
        // Re-located against `out` rather than `src`: the first splice moved every
        // byte after it, so a span taken from the original would land wrong.
        let loc = yaml::locate(&out, HELM_APP_VERSION_KEY)
            .map_err(|e| super::yaml_error(path, HELM_APP_VERSION_KEY, e))?;
        if loc.style == ScalarStyle::Plain {
            warnings.push(format!(
                "{path}: `appVersion` was unquoted; YAML reads that as a number, so cicada is \
                 writing it quoted"
            ));
        }
        out = yaml::splice_at(&out, &loc, &app.to_string(), ScalarStyle::DoubleQuoted)
            .map_err(|e| super::yaml_error(path, HELM_APP_VERSION_KEY, e))?;
    }

    Ok((out, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHART: &str = r#"apiVersion: v2
name: api
description: the api chart   # keep me
version: 0.9.0
appVersion: "1.4.2"
maintainers:
  - name: platform
"#;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    #[test]
    fn both_versions_move_and_nothing_else_does() {
        let (out, warns) = update("Chart.yaml", CHART, &v("0.9.1"), Some(&v("1.5.0"))).unwrap();
        assert_eq!(
            out,
            CHART.replace("version: 0.9.0", "version: 0.9.1").replace(r#""1.4.2""#, r#""1.5.0""#)
        );
        assert!(warns.is_empty());
        assert!(out.contains("description: the api chart   # keep me"));
    }

    #[test]
    fn app_version_is_written_quoted_even_when_it_was_not() {
        // `appVersion: 1.10` is the float 1.1 to YAML — the bug #51 exists for.
        let src = "name: api\nversion: 0.9.0\nappVersion: 1.10\n";
        let (out, warns) = update("Chart.yaml", src, &v("0.9.1"), Some(&v("1.11.0"))).unwrap();
        assert!(out.contains(r#"appVersion: "1.11.0""#), "{out}");
        assert_eq!(warns.len(), 1, "the file was already wrong, and that is worth saying");
        assert!(warns[0].contains("unquoted"), "{}", warns[0]);
    }

    #[test]
    fn a_chart_that_tracks_nothing_leaves_app_version_alone() {
        let (out, warns) = update("Chart.yaml", CHART, &v("0.9.1"), None).unwrap();
        assert!(out.contains(r#"appVersion: "1.4.2""#), "{out}");
        assert!(warns.is_empty());
    }

    #[test]
    fn the_second_splice_is_relocated_against_the_first_ones_output() {
        // The trap: `version` grows from 5 to 6 bytes, shifting `appVersion`'s span
        // by one. A span taken from the original would corrupt the second edit.
        let src = "version: 9.9.9\nappVersion: \"1.0.0\"\n";
        let (out, _) = update("Chart.yaml", src, &v("10.10.10"), Some(&v("2.0.0"))).unwrap();
        assert_eq!(out, "version: 10.10.10\nappVersion: \"2.0.0\"\n");
    }

    #[test]
    fn non_ascii_before_the_target_does_not_shift_the_splice() {
        // K4 / #71, carried from the spike into the real updater.
        let src = "# naïve — … ✅\nversion: 1.0.0\nappVersion: \"1.0.0\"\n";
        let (out, _) = update("Chart.yaml", src, &v("2.0.0"), Some(&v("3.0.0"))).unwrap();
        assert_eq!(out, "# naïve — … ✅\nversion: 2.0.0\nappVersion: \"3.0.0\"\n");
    }

    #[test]
    fn a_chart_with_no_version_key_names_it() {
        let err = update("Chart.yaml", "name: api\n", &v("1.0.0"), None).unwrap_err();
        assert!(matches!(err, EditError::NotFound { .. }), "{err}");
        assert!(err.to_string().contains("version"), "{err}");
    }
}

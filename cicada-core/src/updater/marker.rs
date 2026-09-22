//! Marker-comment editing (#50) — the escape hatch that works on any text file.
//!
//! ```text
//! image: acme/app:1.4.2   # x-cicada-version
//!                 ^^^^^     the first semver-shaped token on the line
//! ```
//!
//! Line-oriented and deliberately dumb: find the lines carrying the marker, and
//! on each replace the first semver-shaped token. That is what lets it serve a
//! Dockerfile, a Makefile, a `version.go` or a shell script without cicada
//! knowing anything about those formats — and it is why the ecosystems deferred
//! out of v1 (future-work §3) have an answer today rather than a wait.
//!
//! The marker itself is never touched, so the file stays annotated and the edit
//! is repeatable.

use crate::edit::EditError;

/// Replace the first semver-shaped token on every line carrying `marker`.
///
/// Every other byte — including the marker comment, the rest of the line, and
/// every unmarked line — is preserved exactly. A file with no marked line is an
/// error rather than a silent no-op: the config says a value lives here, and a
/// version that quietly failed to update is the failure mode #52 exists for.
pub fn splice(path: &str, src: &str, marker: &str, new_value: &str) -> Result<String, EditError> {
    let mut out = String::with_capacity(src.len() + 8);
    let mut hits = 0usize;

    // Split on '\n' rather than `lines()`, so CRLF endings and the presence or
    // absence of a trailing newline both survive untouched.
    for (i, line) in src.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.contains(marker) {
            match replace_first_semver(line, new_value) {
                Some(edited) => {
                    hits += 1;
                    out.push_str(&edited);
                }
                // Marked, but nothing on it looks like a version. Loud, because a
                // marker on the wrong line is exactly the mistake worth catching.
                None => {
                    return Err(EditError::Unaddressable {
                        path: path.to_string(),
                        address: marker.to_string(),
                        message: "marks a line with no version-shaped token on it",
                    });
                }
            }
        } else {
            out.push_str(line);
        }
    }

    if hits == 0 {
        return Err(EditError::NotFound { path: path.to_string(), address: marker.to_string() });
    }
    Ok(out)
}

/// Replace the first `MAJOR.MINOR.PATCH[-pre][+build]` run in a line.
fn replace_first_semver(line: &str, new_value: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            // Only start at the beginning of a number: `app2.0.0` must not match
            // at the `2`, and `1.4.2` must not match again at the `4`. A leading
            // `v` is fine — `v1.4.2` becomes `v1.5.0`, which is what an author
            // who wrote it meant.
            let fresh = i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.');
            if fresh && let Some(end) = semver_end(b, i) {
                return Some(format!("{}{new_value}{}", &line[..i], &line[end..]));
            }
        }
        i += 1;
    }
    None
}

/// The end offset of a semver-shaped token starting at `i`, if there is one.
fn semver_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    for seg in 0..3 {
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == start {
            return None;
        }
        if seg < 2 {
            if j >= b.len() || b[j] != b'.' {
                return None;
            }
            j += 1;
        }
    }
    // Optional `-prerelease` and `+build`, which are dot-separated alphanumerics.
    for lead in *b"-+" {
        if j < b.len() && b[j] == lead {
            let mut k = j + 1;
            while k < b.len() && (b[k].is_ascii_alphanumeric() || b[k] == b'.' || b[k] == b'-') {
                k += 1;
            }
            // A lone `-` with nothing after it is a separator, not a prerelease.
            if k > j + 1 {
                j = k;
            }
        }
    }
    Some(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(src: &str, new: &str) -> String {
        splice("f", src, "x-cicada-version", new).unwrap()
    }

    #[test]
    fn k5_the_rest_of_the_line_including_the_marker_is_untouched() {
        let src = "FROM alpine\nimage: acme/app:1.4.2 # x-cicada-version\nRUN true\n";
        assert_eq!(
            s(src, "1.5.0"),
            "FROM alpine\nimage: acme/app:1.5.0 # x-cicada-version\nRUN true\n"
        );
    }

    #[test]
    fn a_leading_v_is_kept_and_a_trailing_prerelease_is_replaced_whole() {
        assert_eq!(s("VERSION = v1.4.2 # x-cicada-version\n", "1.5.0"), "VERSION = v1.5.0 # x-cicada-version\n");
        assert_eq!(s("v = 1.5.0-rc.2 # x-cicada-version\n", "1.5.0"), "v = 1.5.0 # x-cicada-version\n");
        assert_eq!(s("v = 1.5.0+build.7 # x-cicada-version\n", "1.6.0"), "v = 1.6.0 # x-cicada-version\n");
    }

    #[test]
    fn a_number_inside_a_word_is_not_a_version() {
        // `app2.0.0` would otherwise match at the `2`, rewriting a name.
        assert_eq!(
            s("image: app2 tagged 1.4.2 # x-cicada-version\n", "1.5.0"),
            "image: app2 tagged 1.5.0 # x-cicada-version\n"
        );
    }

    #[test]
    fn every_marked_line_is_edited_and_no_unmarked_line_is() {
        let src = "a: 1.0.0 # x-cicada-version\nb: 1.0.0\nc: 1.0.0 # x-cicada-version\n";
        assert_eq!(s(src, "2.0.0"), "a: 2.0.0 # x-cicada-version\nb: 1.0.0\nc: 2.0.0 # x-cicada-version\n");
    }

    #[test]
    fn crlf_and_a_missing_trailing_newline_both_survive() {
        assert_eq!(s("a: 1.0.0 # x-cicada-version\r\n", "2.0.0"), "a: 2.0.0 # x-cicada-version\r\n");
        assert_eq!(s("a: 1.0.0 # x-cicada-version", "2.0.0"), "a: 2.0.0 # x-cicada-version");
    }

    #[test]
    fn a_file_with_no_marker_is_an_error_rather_than_a_silent_no_op() {
        let err = splice("f", "nothing here\n", "x-cicada-version", "1.0.0").unwrap_err();
        assert!(matches!(err, EditError::NotFound { .. }), "{err}");
    }

    #[test]
    fn a_marker_on_a_line_with_no_version_is_an_error() {
        let err = splice("f", "# x-cicada-version\nfoo\n", "x-cicada-version", "1.0.0").unwrap_err();
        assert!(err.to_string().contains("no version-shaped token"), "{err}");
    }

    #[test]
    fn non_ascii_earlier_in_the_line_does_not_shift_the_splice() {
        assert_eq!(
            s("naïve — 1.4.2 # x-cicada-version\n", "1.5.0"),
            "naïve — 1.5.0 # x-cicada-version\n"
        );
    }
}

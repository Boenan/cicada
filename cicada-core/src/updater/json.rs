//! Span-preserving JSON editing (#49).
//!
//! Same rule as YAML: locate the value's byte span and splice, never
//! parse-and-reserialize. JSON has no comments to lose, but it has key order,
//! indentation and trailing-newline conventions that a round-trip through
//! `serde_json::Value` would silently normalise — and `package.json` is exactly
//! the file where that produces a diff nobody asked for.
//!
//! `serde_json` gives no byte offsets, so this is a small hand-written scanner.
//! It is a *locator*, not a validator: the re-parse afterwards (#52) is what
//! confirms the result is still JSON.

use crate::edit::EditError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    Parse(&'static str),
    NotFound,
    NotAString,
}

struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
    target: Vec<String>,
    /// Byte span of the located value, quotes included.
    found: Option<(usize, usize)>,
}

impl<'a> Scanner<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn byte(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// A JSON string, starting at the opening quote. Returns its span.
    fn string(&mut self) -> Result<(usize, usize), JsonError> {
        let start = self.i;
        if self.byte() != Some(b'"') {
            return Err(JsonError::Parse("expected a string"));
        }
        self.i += 1;
        while let Some(c) = self.byte() {
            match c {
                b'\\' => self.i += 2,
                b'"' => {
                    self.i += 1;
                    return Ok((start, self.i));
                }
                _ => self.i += 1,
            }
        }
        Err(JsonError::Parse("unterminated string"))
    }

    fn value(&mut self, path: &[String]) -> Result<(), JsonError> {
        self.ws();
        let start = self.i;
        match self.byte().ok_or(JsonError::Parse("unexpected end of input"))? {
            b'{' => {
                self.i += 1;
                loop {
                    self.ws();
                    match self.byte() {
                        Some(b'}') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                            continue;
                        }
                        None => return Err(JsonError::Parse("unterminated object")),
                        _ => {}
                    }
                    let (ks, ke) = self.string()?;
                    let key = unescape(&self.s[ks + 1..ke - 1]);
                    self.ws();
                    if self.byte() != Some(b':') {
                        return Err(JsonError::Parse("expected `:` after an object key"));
                    }
                    self.i += 1;
                    let mut child = path.to_vec();
                    child.push(key);
                    self.value(&child)?;
                }
            }
            b'[' => {
                self.i += 1;
                let mut idx = 0usize;
                loop {
                    self.ws();
                    match self.byte() {
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                            continue;
                        }
                        None => return Err(JsonError::Parse("unterminated array")),
                        _ => {}
                    }
                    let mut child = path.to_vec();
                    child.push(idx.to_string());
                    self.value(&child)?;
                    idx += 1;
                }
            }
            b'"' => {
                self.string()?;
            }
            // A number, `true`, `false` or `null`: everything up to the next
            // structural byte. The re-parse (#52) is what judges whether it was
            // well formed, so the scanner only needs its extent.
            _ => {
                while let Some(c) = self.byte() {
                    if c == b',' || c == b'}' || c == b']' || c.is_ascii_whitespace() {
                        break;
                    }
                    self.i += 1;
                }
            }
        }
        if path == self.target.as_slice() && self.found.is_none() {
            self.found = Some((start, self.i));
        }
        Ok(())
    }
}

/// Only the escapes a key can realistically carry. A key needing `\uXXXX` cannot
/// be addressed by a dotted path anyway, so it simply never matches.
fn unescape(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    s.replace("\\\"", "\"").replace("\\\\", "\\").replace("\\/", "/")
}

/// Accepts `$.a.b`, `a.b`, and `a.0.b` for array elements — the shapes the design
/// writes (`json = "$.version"`).
fn parse_path(address: &str) -> Vec<String> {
    address
        .trim_start_matches("$.")
        .trim_start_matches('$')
        .split('.')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Replace the **string** at `address`.
///
/// A non-string target is refused rather than coerced: `{"version": 1}` rewritten
/// to a bare `1.5.0` is not JSON, and silently quoting it would change the
/// document's type in a file cicada does not own.
pub fn splice(src: &str, address: &str, new_value: &str) -> Result<String, JsonError> {
    let target = parse_path(address);
    let mut sc = Scanner { s: src.as_bytes(), i: 0, target, found: None };
    sc.value(&[])?;
    let (start, end) = sc.found.ok_or(JsonError::NotFound)?;
    if src.as_bytes()[start] != b'"' {
        return Err(JsonError::NotAString);
    }
    let escaped = new_value.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("{}\"{escaped}\"{}", &src[..start], &src[end..]))
}

/// #52 — re-parse after editing.
pub fn verify(path: &str, src: &str) -> Result<(), EditError> {
    serde_json::from_str::<serde_json::Value>(src).map(|_| ()).map_err(|e| EditError::Reparse {
        path: path.to_string(),
        kind: "generic (json)",
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = r#"{
  "name": "app",
  "version": "1.4.2",
  "scripts": { "build": "tsc" },
  "workspaces": ["packages/a", "packages/b"]
}
"#;

    #[test]
    fn only_the_addressed_span_changes() {
        let out = splice(PKG, "$.version", "1.5.0").unwrap();
        assert_eq!(out, PKG.replace(r#""1.4.2""#, r#""1.5.0""#));
        // Indentation, key order and the trailing newline all survive, because
        // the document was never re-emitted.
        assert!(out.ends_with("}\n"));
        assert!(out.contains(r#""scripts": { "build": "tsc" }"#));
    }

    #[test]
    fn nested_and_indexed_paths_resolve() {
        assert!(splice(PKG, "scripts.build", "vite").unwrap().contains(r#""build": "vite""#));
        let out = splice(PKG, "workspaces.1", "packages/c").unwrap();
        assert!(out.contains(r#"["packages/a", "packages/c"]"#), "{out}");
    }

    #[test]
    fn a_key_that_appears_deeper_does_not_shadow_the_addressed_one() {
        let src = r#"{"deps":{"version":"9.9.9"},"version":"1.0.0"}"#;
        let out = splice(src, "version", "2.0.0").unwrap();
        assert_eq!(out, r#"{"deps":{"version":"9.9.9"},"version":"2.0.0"}"#);
    }

    #[test]
    fn a_non_string_target_is_refused_rather_than_coerced() {
        assert_eq!(splice(r#"{"version": 1}"#, "version", "1.5.0"), Err(JsonError::NotAString));
    }

    #[test]
    fn a_missing_path_is_named_rather_than_silently_skipped() {
        assert_eq!(splice(PKG, "nope.nope", "1.5.0"), Err(JsonError::NotFound));
    }

    #[test]
    fn non_ascii_before_the_target_does_not_shift_the_splice() {
        // The same class of bug as the YAML char-vs-byte trap (#71). This scanner
        // works in bytes throughout, and this is the test that keeps it that way.
        let src = r#"{"note":"naïve — … ✅","version":"1.0.0"}"#;
        let out = splice(src, "version", "2.0.0").unwrap();
        assert_eq!(out, r#"{"note":"naïve — … ✅","version":"2.0.0"}"#);
    }

    #[test]
    fn a_value_containing_a_quote_is_escaped() {
        let out = splice(r#"{"v":"x"}"#, "v", r#"a"b"#).unwrap();
        assert_eq!(out, r#"{"v":"a\"b"}"#);
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
    }
}

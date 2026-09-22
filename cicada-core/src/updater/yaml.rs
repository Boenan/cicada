//! Span-preserving YAML editing (#49, #71) — promoted from `spikes/yaml-splice`.
//!
//! **Parse with a span-preserving parser, then splice bytes.** Locate the scalar
//! at the addressed path, take its byte range, replace exactly those bytes.
//! Everything outside the span is byte-identical *by construction*, because the
//! document is never re-emitted:
//!
//! ```text
//! before:  tag: 1.4.2   # managed by cicada
//!                ^^^^^  only this span is replaced
//! after:   tag: 1.5.0   # managed by cicada
//! ```
//!
//! The naive options both fail. `serde_yaml` round-trips destroy comments and
//! formatting; hand-rolled line-walking means reimplementing quoting, trailing
//! comments, flow mappings, anchors, multi-document files and indentation
//! tracking — a YAML parser, written badly.
//!
//! **The two traps the spike surfaced are both load-bearing**, and their
//! regression tests came with the code:
//!
//! * [`char_to_byte`] — `Marker::index()` is a **char** index despite its
//!   accessor's doc comment saying bytes. Splicing on the raw value corrupts any
//!   file with non-ASCII before the target.
//! * [`Walker::node`] tolerates what it cannot address (#72). An alias or complex
//!   key *anywhere* must not abort the edit; only one at the addressed path is an
//!   error. Helm values files use anchors freely, and the spike's first draft
//!   refused to edit unrelated keys because of a `&baseline` twenty lines away.

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YamlError {
    Parse(String),
    NotFound,
    Unaddressable(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    pub value: String,
    pub style: ScalarStyle,
    /// Byte offsets, already converted from the parser's char indices.
    pub start: usize,
    pub end: usize,
}

type Ev = (Event<'static>, Span);

/// Collect events, owning the borrowed data so the walker stays simple. N is a
/// config file, not a data stream, so materialising the events costs nothing.
fn events(yaml: &str) -> Result<Vec<Ev>, YamlError> {
    let mut out = Vec::new();
    for r in Parser::new_from_str(yaml) {
        let (ev, span) = r.map_err(|e| YamlError::Parse(e.to_string()))?;
        let owned = match ev {
            Event::Scalar(v, style, aid, _tag) => {
                Event::Scalar(std::borrow::Cow::Owned(v.into_owned()), style, aid, None)
            }
            Event::Nothing => Event::Nothing,
            Event::StreamStart => Event::StreamStart,
            Event::StreamEnd => Event::StreamEnd,
            Event::DocumentStart(b) => Event::DocumentStart(b),
            Event::DocumentEnd => Event::DocumentEnd,
            Event::Alias(id) => Event::Alias(id),
            Event::SequenceStart(id, _) => Event::SequenceStart(id, None),
            Event::SequenceEnd => Event::SequenceEnd,
            Event::MappingStart(id, _) => Event::MappingStart(id, None),
            Event::MappingEnd => Event::MappingEnd,
        };
        out.push((owned, span));
    }
    Ok(out)
}

/// **The trap.** `saphyr_parser::Marker::index()` is documented on the accessor as
/// "in bytes"; the struct field it returns says "in chars", and the field is the
/// truthful one. Splicing on the raw value shifts by one byte per multi-byte
/// character before the target — a comment containing `naïve — … ✅` moved the
/// splice 8 bytes and produced `vers2.0.01.0.0`, silently.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or_else(|| s.len(), |(b, _)| b)
}

struct Walker {
    evs: Vec<Ev>,
    i: usize,
    target: Vec<String>,
    found: Option<Located>,
    doc: usize,
}

impl Walker {
    fn peek(&self) -> Option<&Ev> {
        self.evs.get(self.i)
    }

    fn next(&mut self) -> Option<Ev> {
        let e = self.evs.get(self.i).cloned();
        if e.is_some() {
            self.i += 1;
        }
        e
    }

    /// Consume one full node without inspecting it.
    fn skip_node(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.next() {
                Some((Event::MappingStart(..) | Event::SequenceStart(..), _)) => depth += 1,
                Some((Event::MappingEnd | Event::SequenceEnd, _)) => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                Some(_) => {
                    if depth == 0 {
                        return;
                    }
                }
                None => return,
            }
        }
    }

    /// Walk one node, recording it if `path` matches the target.
    fn node(&mut self, path: &[String]) -> Result<(), YamlError> {
        let Some((ev, span)) = self.next() else { return Ok(()) };
        match ev {
            Event::Scalar(v, style, _, _) => {
                if path == self.target.as_slice() && self.found.is_none() {
                    self.found = Some(Located {
                        value: v.into_owned(),
                        style,
                        start: span.start.index(),
                        end: span.end.index(),
                    });
                }
            }
            Event::MappingStart(..) => loop {
                match self.peek() {
                    Some((Event::MappingEnd, _)) => {
                        self.next();
                        break;
                    }
                    None => break,
                    _ => {}
                }
                let key = match self.next() {
                    Some((Event::Scalar(k, _, _, _), _)) => k.into_owned(),
                    // A complex key can never be addressed by a dotted path. Skip
                    // its value and carry on rather than failing the document (#72).
                    Some(_) => {
                        self.skip_node();
                        self.skip_node();
                        continue;
                    }
                    None => break,
                };
                let mut child = path.to_vec();
                child.push(key);
                self.node(&child)?;
            },
            Event::SequenceStart(..) => {
                let mut idx = 0usize;
                loop {
                    match self.peek() {
                        Some((Event::SequenceEnd, _)) => {
                            self.next();
                            break;
                        }
                        None => break,
                        _ => {}
                    }
                    let mut child = path.to_vec();
                    child.push(idx.to_string());
                    self.node(&child)?;
                    idx += 1;
                }
            }
            // Only a problem if it IS the target (#72). An alias anywhere else is
            // ordinary — Helm values files use anchors freely, and the spike's
            // first draft refused to edit unrelated keys because of one.
            Event::Alias(_) if path == self.target.as_slice() && self.found.is_none() => {
                return Err(YamlError::Unaddressable("is an alias, which has no span to replace"));
            }
            _ => {}
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), YamlError> {
        while let Some((ev, _)) = self.peek().cloned() {
            match ev {
                Event::StreamStart | Event::DocumentEnd => {
                    self.next();
                }
                Event::DocumentStart(_) => {
                    self.next();
                    if self.doc == 0 {
                        self.node(&[])?;
                    } else {
                        // Only the first document is addressable. A `---` separated
                        // multi-document file is skipped past rather than refused,
                        // which is the #72 rule applied at document scope.
                        while !matches!(
                            self.peek(),
                            Some((Event::DocumentEnd, _)) | Some((Event::StreamEnd, _)) | None
                        ) {
                            self.next();
                        }
                    }
                    self.doc += 1;
                }
                Event::StreamEnd | Event::Nothing => break,
                _ => {
                    self.next();
                }
            }
        }
        Ok(())
    }
}

/// Find the scalar at a dotted path. Sequence elements are addressed by index:
/// `containers.0.image`.
pub fn locate(yaml: &str, path: &str) -> Result<Located, YamlError> {
    let target: Vec<String> = path.split('.').map(String::from).collect();
    let mut w = Walker { evs: events(yaml)?, i: 0, target, found: None, doc: 0 };
    w.run()?;
    let mut loc = w.found.ok_or(YamlError::NotFound)?;
    loc.start = char_to_byte(yaml, loc.start);
    loc.end = char_to_byte(yaml, loc.end);
    Ok(loc)
}

/// Replace the scalar at `path`, preserving the quoting style found in place.
pub fn splice(yaml: &str, path: &str, new_value: &str) -> Result<String, YamlError> {
    let loc = locate(yaml, path)?;
    splice_at(yaml, &loc, new_value, loc.style)
}

/// Replace a located scalar with an explicitly chosen quoting style.
///
/// Separate from [`splice`] because Helm's `appVersion` must always be written
/// **quoted** regardless of what it found (#51) — `appVersion: 1.10` is the float
/// `1.1` to YAML, and byte-splicing faithfully preserves whatever quoting is
/// there, including none.
pub fn splice_at(
    yaml: &str,
    loc: &Located,
    new_value: &str,
    style: ScalarStyle,
) -> Result<String, YamlError> {
    let replacement = match style {
        ScalarStyle::Plain => new_value.to_string(),
        ScalarStyle::SingleQuoted => format!("'{}'", new_value.replace('\'', "''")),
        ScalarStyle::DoubleQuoted => {
            format!("\"{}\"", new_value.replace('\\', "\\\\").replace('"', "\\\""))
        }
        ScalarStyle::Literal | ScalarStyle::Folded => {
            return Err(YamlError::Unaddressable(
                "is a block scalar, whose span covers an indented body rather than a value",
            ));
        }
    };
    let mut out = String::with_capacity(yaml.len() + replacement.len());
    out.push_str(&yaml[..loc.start]);
    out.push_str(&replacement);
    out.push_str(&yaml[loc.end..]);
    Ok(out)
}

/// Re-parse after editing (#52). A splice that produces an unparseable file is a
/// bug in the transform, and the run must abort before anything is written.
pub fn verify(yaml: &str) -> Result<(), YamlError> {
    events(yaml).map(|_| ())
}

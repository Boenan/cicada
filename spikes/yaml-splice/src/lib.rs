//! Spike for cicada decision #49: locate a scalar by dotted path and replace it by
//! byte-splicing its span, so everything outside the span stays byte-identical.

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

#[derive(Debug, Clone, PartialEq)]
pub struct Located {
    pub value: String,
    pub style: ScalarStyle,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Parse(String),
    NotFound(String),
    NotAScalar(String),
    Unsupported(&'static str),
}

type Ev = (Event<'static>, Span);

/// Collect events, owning the borrowed data so the walker is simple.
fn events(yaml: &str) -> Result<Vec<Ev>, Error> {
    let mut out = Vec::new();
    for r in Parser::new_from_str(yaml) {
        let (ev, span) = r.map_err(|e| Error::Parse(e.to_string()))?;
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

/// saphyr-parser's `Marker::index()` is documented on the accessor as "in bytes" but the
/// struct field says "in chars" — and the field is the truthful one. Splicing on the raw
/// value corrupts any document with non-ASCII before the target.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map_or_else(|| s.len(), |(b, _)| b)
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
    fn node(&mut self, path: &[String]) -> Result<(), Error> {
        let Some((ev, span)) = self.next() else {
            return Ok(());
        };
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
                // key must be a scalar for a dotted path to address it
                let key = match self.next() {
                    Some((Event::Scalar(k, _, _, _), _)) => k.into_owned(),
                    // A complex key (rare) can never be addressed by a dotted path.
                    // Skip its value and carry on rather than failing the whole document.
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
            // Only a problem if it IS the target. Helm values files use anchors freely,
            // and an alias elsewhere must not stop us editing an unrelated key.
            Event::Alias(_) => {
                if path == self.target.as_slice() && self.found.is_none() {
                    return Err(Error::Unsupported("alias at addressed path"));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
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
                        // skip non-first documents entirely
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

pub fn locate(yaml: &str, path: &str) -> Result<Located, Error> {
    let target: Vec<String> = path.split('.').map(String::from).collect();
    let mut w = Walker {
        evs: events(yaml)?,
        i: 0,
        target,
        found: None,
        doc: 0,
    };
    w.run()?;
    let mut loc = w.found.ok_or_else(|| Error::NotFound(path.to_string()))?;
    loc.start = char_to_byte(yaml, loc.start);
    loc.end = char_to_byte(yaml, loc.end);
    Ok(loc)
}

/// Replace the scalar at `path` with `new_value`, preserving quoting style.
pub fn splice(yaml: &str, path: &str, new_value: &str) -> Result<String, Error> {
    let loc = locate(yaml, path)?;
    let replacement = match loc.style {
        ScalarStyle::Plain => new_value.to_string(),
        ScalarStyle::SingleQuoted => format!("'{}'", new_value.replace('\'', "''")),
        ScalarStyle::DoubleQuoted => format!("\"{}\"", new_value.replace('\\', "\\\\").replace('"', "\\\"")),
        ScalarStyle::Literal | ScalarStyle::Folded => {
            return Err(Error::Unsupported("block scalar at addressed path"))
        }
    };
    let mut out = String::with_capacity(yaml.len() + 8);
    out.push_str(&yaml[..loc.start]);
    out.push_str(&replacement);
    out.push_str(&yaml[loc.end..]);
    Ok(out)
}

/// Re-parse after writing (cicada decision #52).
pub fn verify(yaml: &str) -> Result<(), Error> {
    events(yaml).map(|_| ())
}

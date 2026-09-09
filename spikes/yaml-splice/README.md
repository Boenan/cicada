# spike: YAML span splicing

Validates cicada design decision **#49** — that a structured-format edit can be done by locating a
scalar's byte span and replacing only those bytes, leaving the rest of the file byte-identical.

This is the M0 de-risking spike. It is **not** production code; it is the evidence that the approach
works, plus a test suite that should carry over into the real updater.

```
cargo test          # 15 cases
cargo run --example realworld
```

## Findings

1. **`saphyr-parser` works.** Its `Parser` is an `Iterator<Item = Result<(Event, Span), ScanError>>`,
   and `Span` carries start/end `Marker`s for every scalar. Walking the event stream while tracking a
   path is straightforward.

2. **`Marker::index()` is a CHAR index, not a byte index** — despite the accessor's own doc comment
   saying "in bytes". The struct field comment ("in chars") is the truthful one. Splicing on the raw
   value corrupts any file containing non-ASCII *anywhere before* the target: in testing, a comment
   containing `naïve — … ✅` shifted the splice 8 bytes and produced `vers2.0.01.0.0`. Conversion via
   `char_indices().nth()` is mandatory, and the two regression tests for it must survive into the real
   implementation.

3. **The walker must tolerate what it cannot address.** An alias or a complex mapping key *anywhere*
   in the document must not abort the operation — only one at the addressed path is an error. Helm
   values files use anchors freely; the first draft refused to edit unrelated keys because of a
   `&baseline` twenty lines away.

4. **Quoting style is recoverable** (`ScalarStyle`), so `appVersion: "1.4.2"` stays quoted and
   `version: 0.9.0` stays bare — and an unquoted `appVersion` is detectable for the #51 warning.

5. Works as-is on: nested paths, sequence indices, flow mappings, CRLF, multi-document (first doc
   only), trailing comments, and anchored values. Block scalars and aliases at the target are refused
   explicitly rather than being silently wrong.

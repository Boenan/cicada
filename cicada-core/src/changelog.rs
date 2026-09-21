//! Changelog assembly (#5) — where a rendered entry goes in the file.
//!
//! Three placements, in order of preference:
//!
//! 1. **The marker.** `<!-- cicada:insert -->` present → the entry goes directly
//!    after it. Everything above and below is byte-identical (L2).
//! 2. **Prepend after the header.** No marker, but prior entries → the entry goes
//!    immediately before the first `## ` heading, so newest stays first (L3).
//! 3. **Create.** No file → the built-in header plus the entry (L1). The header
//!    carries the marker, so every later release takes path 1 and the fallback is
//!    only ever used by files cicada did not create.
//!
//! Changelogs are **append-at-top, never rewritten** (L4): an existing entry is
//! never re-read, re-rendered or re-indented, which is what lets a human correct
//! one by hand and keep the correction.

pub const MARKER: &str = "<!-- cicada:insert -->";

/// Insert `entry` into `existing`, or create the file when there is none.
///
/// `entry` is the rendered entry *without* trailing blank lines; spacing around it
/// is this function's business, so that two releases in a row produce the same
/// separation as one release twice.
pub fn insert(existing: Option<&str>, entry: &str, header: &str) -> String {
    let entry = entry.trim_end();

    let Some(existing) = existing else {
        // L1 — created with the built-in header, which carries the marker.
        return format!("{}\n\n{entry}\n", header.trim_end());
    };

    if let Some(pos) = existing.find(MARKER) {
        // L2 — directly after the marker, leaving the marker itself in place so
        // the next release finds it too.
        let after = pos + MARKER.len();
        let head = &existing[..after];
        let rest = existing[after..].trim_start_matches('\n');
        return if rest.trim().is_empty() {
            format!("{head}\n\n{entry}\n")
        } else {
            format!("{head}\n\n{entry}\n\n{rest}")
        };
    }

    // L3 — before the first entry heading, so newest stays first.
    if let Some(pos) = find_first_entry_heading(existing) {
        let (head, tail) = existing.split_at(pos);
        return format!("{}\n\n{entry}\n\n{tail}", head.trim_end_matches('\n'));
    }

    // A file with a header and no entries yet: the entry goes at the end.
    format!("{}\n\n{entry}\n", existing.trim_end())
}

/// The byte offset of the first `## ` heading at the start of a line.
///
/// Fenced code blocks are skipped, because a changelog entry quoting a diff can
/// legitimately contain a line starting `## ` inside a fence, and inserting there
/// would split the fence.
fn find_first_entry_heading(s: &str) -> Option<usize> {
    let mut offset = 0usize;
    let mut in_fence = false;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with("## ") {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "# Changelog\n\nBlurb.\n\n<!-- cicada:insert -->";

    #[test]
    fn l1_an_absent_file_is_created_with_the_header_and_the_entry() {
        let out = insert(None, "## 0.1.0 (2026-09-21)\n\n* a thing", HEADER);
        assert_eq!(
            out,
            "# Changelog\n\nBlurb.\n\n<!-- cicada:insert -->\n\n## 0.1.0 (2026-09-21)\n\n* a thing\n"
        );
        // And the created file carries the marker, so the next release takes the
        // marker path rather than the fallback.
        assert!(out.contains(MARKER));
    }

    #[test]
    fn l2_the_marker_path_leaves_everything_around_it_byte_identical() {
        let existing = "# Changelog\n\n<!-- cicada:insert -->\n\n## 1.0.0 (2026-01-01)\n\n* old\n";
        let out = insert(Some(existing), "## 1.1.0 (2026-09-21)\n\n* new", HEADER);
        assert_eq!(
            out,
            "# Changelog\n\n<!-- cicada:insert -->\n\n## 1.1.0 (2026-09-21)\n\n* new\n\n## 1.0.0 (2026-01-01)\n\n* old\n"
        );
        // L4 — the previous entry is byte-identical, not re-rendered.
        assert!(out.contains("## 1.0.0 (2026-01-01)\n\n* old\n"));
    }

    #[test]
    fn l3_without_a_marker_the_entry_goes_before_the_first_existing_one() {
        let existing = "# Changelog\n\nSome blurb.\n\n## 1.0.0 (2026-01-01)\n\n* old\n";
        let out = insert(Some(existing), "## 1.1.0 (2026-09-21)\n\n* new", HEADER);
        assert_eq!(
            out,
            "# Changelog\n\nSome blurb.\n\n## 1.1.0 (2026-09-21)\n\n* new\n\n## 1.0.0 (2026-01-01)\n\n* old\n"
        );
    }

    #[test]
    fn l4_a_second_release_stacks_above_the_first_without_touching_it() {
        let first = insert(None, "## 1.0.0 (2026-01-01)\n\n* one", HEADER);
        let second = insert(Some(&first), "## 1.1.0 (2026-09-21)\n\n* two", HEADER);
        assert!(second.find("1.1.0").unwrap() < second.find("1.0.0").unwrap());
        // The first entry survives byte-for-byte.
        assert!(second.contains("## 1.0.0 (2026-01-01)\n\n* one\n"));
        // Inserting twice more keeps the shape stable rather than drifting.
        let third = insert(Some(&second), "## 1.2.0 (2026-10-01)\n\n* three", HEADER);
        assert!(third.find("1.2.0").unwrap() < third.find("1.1.0").unwrap());
        assert!(third.contains("## 1.0.0 (2026-01-01)\n\n* one\n"));
    }

    #[test]
    fn a_marked_file_with_no_entries_yet_does_not_grow_stray_blank_lines() {
        let existing = "# Changelog\n\n<!-- cicada:insert -->\n";
        let out = insert(Some(existing), "## 1.0.0 (2026-01-01)\n\n* one", HEADER);
        assert_eq!(out, "# Changelog\n\n<!-- cicada:insert -->\n\n## 1.0.0 (2026-01-01)\n\n* one\n");
        assert!(!out.contains("\n\n\n"), "{out:?}");
    }

    #[test]
    fn a_header_only_file_with_no_marker_gets_the_entry_appended() {
        let out = insert(Some("# Changelog\n\nNothing yet.\n"), "## 1.0.0 (2026-01-01)\n\n* one", HEADER);
        assert_eq!(out, "# Changelog\n\nNothing yet.\n\n## 1.0.0 (2026-01-01)\n\n* one\n");
    }

    #[test]
    fn a_heading_inside_a_fenced_block_is_not_mistaken_for_an_entry() {
        // A previous entry quoting a diff can legitimately contain `## ` inside a
        // fence; inserting there would split the fence and corrupt the file.
        let existing = "# Changelog\n\n```\n## not an entry\n```\n\n## 1.0.0 (2026-01-01)\n\n* old\n";
        let out = insert(Some(existing), "## 1.1.0 (2026-09-21)\n\n* new", HEADER);
        assert!(out.contains("```\n## not an entry\n```"), "{out}");
        assert!(out.find("1.1.0").unwrap() < out.find("1.0.0").unwrap(), "{out}");
    }

    #[test]
    fn insertion_is_idempotent_in_shape_if_not_in_content() {
        // Running the same release twice must not change the spacing rules, which
        // is what keeps a re-run's diff limited to the entry itself.
        let a = insert(None, "## 1.0.0 (2026-01-01)\n\n* one", HEADER);
        let b = insert(None, "## 1.0.0 (2026-01-01)\n\n* one", HEADER);
        assert_eq!(a, b);
    }
}

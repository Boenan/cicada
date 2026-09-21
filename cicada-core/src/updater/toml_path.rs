//! Path-addressed TOML editing for the `generic` updater (#50).
//!
//! `toml_edit` does the span preservation, so this is only the dotted-path walk
//! on top of it: `toml = "package.version"`, or `tool.poetry.version` for a
//! `pyproject.toml`. Array elements are addressed by index.
//!
//! Unlike the `cargo` updater this writes whatever node the path names, and does
//! not know or care what the file is for.

use crate::edit::EditError;
use toml_edit::{DocumentMut, Item, Table, Value, value};

/// Replace the scalar at a dotted path with a string.
pub fn splice(path: &str, src: &str, address: &str, new_value: &str) -> Result<String, EditError> {
    let mut doc = src
        .parse::<DocumentMut>()
        .map_err(|e| EditError::Parse { path: path.to_string(), message: e.to_string() })?;

    let segments: Vec<&str> = address.split('.').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(EditError::Unaddressable {
            path: path.to_string(),
            address: address.to_string(),
            message: "is empty — a toml edit needs a dotted path such as `package.version`",
        });
    }

    set_in_table(doc.as_table_mut(), &segments, new_value).map_err(|reason| match reason {
        Reason::NotFound => {
            EditError::NotFound { path: path.to_string(), address: address.to_string() }
        }
        Reason::Unaddressable(message) => EditError::Unaddressable {
            path: path.to_string(),
            address: address.to_string(),
            message,
        },
    })?;
    Ok(doc.to_string())
}

/// Replace a value, keeping the whitespace and comments around it.
///
/// `*item = value(new)` is the obvious spelling and it is wrong: a `Value` owns
/// its surrounding *decor*, so assigning a fresh one silently deletes the trailing
/// comment on the edited line. That is the one line a reader is most likely to
/// have annotated, and losing it would break the formatting-preservation claim
/// (#49) precisely where it is most visible.
pub(crate) fn replace_preserving_decor(item: &mut Item, new_value: &str) {
    if let Some(old) = item.as_value() {
        *item = Item::Value(with_decor(old, new_value));
    } else {
        *item = value(new_value);
    }
}

fn with_decor(old: &Value, new_value: &str) -> Value {
    let mut new = Value::from(new_value);
    *new.decor_mut() = old.decor().clone();
    new
}

enum Reason {
    NotFound,
    Unaddressable(&'static str),
}

/// Walk a table by key. A numeric segment addresses an array-of-tables element
/// (`bin.0.version`), which is the only indexed shape TOML realistically puts a
/// version in — a plain array of scalars is refused by name rather than guessed at.
fn set_in_table(table: &mut Table, segs: &[&str], new_value: &str) -> Result<(), Reason> {
    let (head, rest) = segs.split_first().expect("non-empty by construction");

    if let Ok(idx) = head.parse::<usize>() {
        // An index at the *start* of a walk through a table cannot resolve: the
        // caller reached here through a key, so the array it means is that key's.
        let _ = idx;
        return Err(Reason::Unaddressable("indexes a table, which has no elements"));
    }

    let item = table.get_mut(head).ok_or(Reason::NotFound)?;
    set_in_item(item, rest, new_value)
}

/// `segs` is what remains *after* the key that produced `item`.
fn set_in_item(item: &mut Item, segs: &[&str], new_value: &str) -> Result<(), Reason> {
    let Some((head, rest)) = segs.split_first() else {
        // The path ends here, so this node is the one to replace.
        if item.as_value().is_none() {
            return Err(Reason::Unaddressable("names a table, not a value"));
        }
        replace_preserving_decor(item, new_value);
        return Ok(());
    };

    if let Ok(idx) = head.parse::<usize>() {
        if let Some(aot) = item.as_array_of_tables_mut() {
            let t = aot.get_mut(idx).ok_or(Reason::NotFound)?;
            return set_in_table(t, rest, new_value);
        }
        if let Some(arr) = item.as_array_mut() {
            // A leaf inside a plain array: `workspaces.1`.
            if !rest.is_empty() {
                return Err(Reason::Unaddressable(
                    "reaches through a plain array, whose elements are values rather than tables",
                ));
            }
            let v = arr.get_mut(idx).ok_or(Reason::NotFound)?;
            *v = with_decor(v, new_value);
            return Ok(());
        }
        return Err(Reason::Unaddressable("indexes something that is not an array"));
    }

    let table = item.as_table_like_mut().ok_or(Reason::Unaddressable(
        "reaches through a value, which has no keys",
    ))?;
    let next = table.get_mut(head).ok_or(Reason::NotFound)?;
    set_in_item(next, rest, new_value)
}

/// #52 — re-parse after editing.
pub fn verify(path: &str, src: &str) -> Result<(), EditError> {
    src.parse::<DocumentMut>().map(|_| ()).map_err(|e| EditError::Reparse {
        path: path.to_string(),
        kind: "generic (toml)",
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"# keep me
[tool.poetry]
name    = "app"
version = "1.4.2"   # and keep this trailing comment

[tool.other]
version = "9.9.9"
"#;

    #[test]
    fn only_the_addressed_node_changes() {
        let out = splice("pyproject.toml", SRC, "tool.poetry.version", "1.5.0").unwrap();
        assert!(out.contains(r#"version = "1.5.0"   # and keep this trailing comment"#), "{out}");
        // The identically named key under a different table is untouched.
        assert!(out.contains(r#"version = "9.9.9""#), "{out}");
        assert!(out.starts_with("# keep me\n"), "{out}");
    }

    #[test]
    fn a_missing_path_names_itself() {
        let err = splice("f.toml", SRC, "tool.nope.version", "1.5.0").unwrap_err();
        assert!(matches!(err, EditError::NotFound { .. }), "{err}");
        assert!(err.to_string().contains("tool.nope.version"), "{err}");
    }

    #[test]
    fn a_path_naming_a_table_is_refused_rather_than_overwriting_it() {
        let err = splice("f.toml", SRC, "tool.poetry", "1.5.0").unwrap_err();
        assert!(err.to_string().contains("names a table"), "{err}");
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn an_array_of_tables_element_is_addressable_by_index() {
        let src = "[[bin]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[bin]]\nname = \"b\"\nversion = \"2.0.0\"\n";
        let out = splice("f.toml", src, "bin.1.version", "3.0.0").unwrap();
        assert!(out.contains("name = \"b\"\nversion = \"3.0.0\""), "{out}");
        assert!(out.contains("name = \"a\"\nversion = \"1.0.0\""), "{out}");
    }

    #[test]
    fn a_plain_array_element_is_addressable_as_a_leaf() {
        let out = splice("f.toml", "tags = [\"a\", \"b\"]\n", "tags.1", "c").unwrap();
        assert!(out.contains("[\"a\", \"c\"]"), "{out}");
    }

    #[test]
    fn reaching_through_a_plain_array_is_refused_by_name_rather_than_panicking() {
        // The first draft of this walker had an `unreachable!()` here, which is a
        // panic waiting for the first person to write `tags.0.x`.
        let err = splice("f.toml", "tags = [\"a\"]\n", "tags.0.x", "c").unwrap_err();
        assert!(err.to_string().contains("plain array"), "{err}");
    }

    #[test]
    fn an_index_past_the_end_is_not_found_rather_than_a_panic() {
        let err = splice("f.toml", "tags = [\"a\"]\n", "tags.9", "c").unwrap_err();
        assert!(matches!(err, EditError::NotFound { .. }), "{err}");
    }

    #[test]
    fn reaching_through_a_scalar_says_so() {
        let err = splice("f.toml", "name = \"a\"\n", "name.version", "1.0.0").unwrap_err();
        assert!(err.to_string().contains("has no keys"), "{err}");
    }
}

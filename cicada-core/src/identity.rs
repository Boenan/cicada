//! Cicada's commit identity (#55).
//!
//! ```toml
//! committer = "cicada[bot] <cicada@users.noreply.github.com>"
//! ```
//!
//! It must be **configurable and stable across cicada versions**, because #32
//! decides whether to clobber the release PR branch by asking whether that
//! branch's tip is cicada's own commit. If the identity drifts between releases,
//! every existing release PR branch suddenly looks human-authored and every repo
//! on earth starts refusing to regenerate its release PR.
//!
//! Parsing lives here, purely, so `check` can reject a malformed value before any
//! command reaches the point of making a commit with it.

/// A `Name <email>` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not a `Name <email>` pair")]
pub struct IdentityError {
    pub value: String,
}

impl Identity {
    /// Parse `Name <email>`.
    ///
    /// Deliberately strict about the shape and indifferent to what is inside the
    /// angle brackets: git does not validate addresses either, and a tool that
    /// rejected `cicada@users.noreply.github.com` on some RFC ground would be
    /// wrong about the one identity it ships with.
    pub fn parse(s: &str) -> Result<Self, IdentityError> {
        let err = || IdentityError { value: s.to_string() };
        let (name, rest) = s.trim().split_once('<').ok_or_else(err)?;
        let email = rest.strip_suffix('>').ok_or_else(err)?;
        let name = name.trim();
        let email = email.trim();
        if name.is_empty() || email.is_empty() {
            return Err(err());
        }
        Ok(Identity { name: name.to_string(), email: email.to_string() })
    }

    /// The `Name <email>` spelling git prints for `%cn <%ce>` — the form #32's
    /// guard compares against, so it must round-trip [`Identity::parse`].
    pub fn to_git(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_default_parses_and_round_trips() {
        let id = Identity::parse("cicada[bot] <cicada@users.noreply.github.com>").unwrap();
        assert_eq!(id.name, "cicada[bot]");
        assert_eq!(id.email, "cicada@users.noreply.github.com");
        // The guard compares strings, so this equality is load-bearing (#32).
        assert_eq!(Identity::parse(&id.to_git()).unwrap(), id);
    }

    #[test]
    fn surrounding_whitespace_is_not_part_of_the_identity() {
        assert_eq!(
            Identity::parse("  Name   <  a@b  >  ").unwrap(),
            Identity { name: "Name".into(), email: "a@b".into() }
        );
    }

    #[test]
    fn a_value_that_is_not_a_pair_is_refused_rather_than_guessed_at() {
        for bad in ["cicada[bot]", "<a@b>", "name <>", "name <a@b", "name a@b>"] {
            assert!(Identity::parse(bad).is_err(), "{bad} should not parse");
        }
    }
}

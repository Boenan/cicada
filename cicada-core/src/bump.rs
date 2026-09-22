//! Bump computation — semver arithmetic, and the two tables that surround it.
//!
//! **Not every commit justifies a release** (#36). Two questions that look like
//! one are kept apart: *does this type appear in the changelog?* is answered by a
//! section's `title`/`hidden`, and *does this type justify cutting a release at
//! all?* by its `bump`. A range holding only `chore(deps): bump serde` therefore
//! produces no release — otherwise every dependency bump would ship a version.

use semver::Version;

use crate::commit::ParsedCommit;
use crate::config::{Bump, Config, PreMajor};

/// What one commit contributes.
///
/// Three rules, in this order:
///
/// 1. **An unconfigured type contributes nothing** — not an error, ignored
///    (design §Commit parsing, A6). The configured sections are the whole universe
///    of types cicada knows, so `wibble:` is invisible to bump computation. This
///    holds for `wibble!:` too: cicada cannot judge the compatibility of a change
///    whose kind it has no opinion about, and inventing a major from a `!` on an
///    unknown type would be exactly the invention #39 refuses everywhere else.
/// 2. **Breaking overrides the section** (#37, B3/B4). `feat!:` and a
///    `BREAKING CHANGE:` footer are equivalent, and both take major.
/// 3. Otherwise the section's own `bump`.
pub fn commit_bump(config: &Config, commit: &ParsedCommit) -> Bump {
    let Some(section) = config.section_for(&commit.kind) else { return Bump::None };
    if commit.breaking { Bump::Major } else { section.bump }
}

/// The largest bump any of these commits justifies. `Bump::None` means the range
/// contains nothing releasable, which is a normal outcome and not a failure (#68).
pub fn highest(config: &Config, commits: &[ParsedCommit]) -> Bump {
    commits.iter().map(|c| commit_bump(config, c)).max().unwrap_or(Bump::None)
}

/// Apply a bump to an existing version.
///
/// `pre_major` governs 0.x and nothing else (#37): with the default `"minor"`, a
/// breaking change at `0.3.0` gives `0.4.0` rather than escaping to `1.0.0` by
/// accident (B5). Reaching 1.0.0 is a deliberate act — `pre_major = "major"` plus
/// a breaking change (B6), a statement about the package in config rather than an
/// instruction buried in one commit message (#39).
///
/// The result is always a bare `MAJOR.MINOR.PATCH`: prerelease and build metadata
/// are dropped rather than carried, because `from` is the last **stable** release
/// by construction (#60) and a computed stable version has neither.
pub fn next_version(from: &Version, bump: Bump, pre_major: PreMajor) -> Version {
    let effective = match (bump, from.major, pre_major) {
        (Bump::Major, 0, PreMajor::Minor) => Bump::Minor,
        _ => bump,
    };
    match effective {
        Bump::Major => Version::new(from.major + 1, 0, 0),
        Bump::Minor => Version::new(from.major, from.minor + 1, 0),
        Bump::Patch => Version::new(from.major, from.minor, from.patch + 1),
        Bump::None => Version::new(from.major, from.minor, from.patch),
    }
}

/// The first-release table (#37). An unreleased package is treated as `0.0.0` with
/// **strict semver** applied:
///
/// | contains | version |
/// |---|---|
/// | a breaking change | `1.0.0` |
/// | a feature | `0.1.0` |
/// | only fixes | `0.0.1` |
///
/// Note this is a conscious exception to [`next_version`] rather than a special
/// case of it: `feat!:` yields `1.0.0` here but `0.4.0` at `0.3.0`, because the
/// first release is where you *choose* your entry point and `!` is how you say so
/// without config. `initial_version` overrides the table outright (A4).
pub fn first_release(bump: Bump) -> Version {
    match bump {
        Bump::Major => Version::new(1, 0, 0),
        Bump::Minor => Version::new(0, 1, 0),
        Bump::Patch => Version::new(0, 0, 1),
        Bump::None => Version::new(0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().unwrap()
    }

    #[test]
    fn version_arithmetic_zeroes_the_segments_below_it() {
        assert_eq!(next_version(&v("1.4.2"), Bump::Patch, PreMajor::Minor), v("1.4.3"));
        assert_eq!(next_version(&v("1.4.2"), Bump::Minor, PreMajor::Minor), v("1.5.0"));
        assert_eq!(next_version(&v("1.4.2"), Bump::Major, PreMajor::Minor), v("2.0.0"));
    }

    #[test]
    fn pre_major_keeps_zero_x_inside_zero_x() {
        // B5 — the default never escapes to 1.0.0 by accident.
        assert_eq!(next_version(&v("0.3.0"), Bump::Major, PreMajor::Minor), v("0.4.0"));
        // B6 — the opt-in escape.
        assert_eq!(next_version(&v("0.3.0"), Bump::Major, PreMajor::Major), v("1.0.0"));
        // It governs 0.x *and nothing else*: minor and patch are untouched.
        assert_eq!(next_version(&v("0.3.0"), Bump::Minor, PreMajor::Minor), v("0.4.0"));
        assert_eq!(next_version(&v("0.3.0"), Bump::Patch, PreMajor::Minor), v("0.3.1"));
        // And it is about `from`, not about the answer: 1.x is unaffected.
        assert_eq!(next_version(&v("1.0.0"), Bump::Major, PreMajor::Minor), v("2.0.0"));
    }

    #[test]
    fn the_first_release_table_is_not_next_version_from_zero() {
        // The distinction that makes it a table rather than an arithmetic: a
        // breaking change gives 1.0.0 here, where `next_version` from 0.0.0 under
        // the default `pre_major` would give 0.1.0.
        assert_eq!(first_release(Bump::Major), v("1.0.0"));
        assert_eq!(next_version(&v("0.0.0"), Bump::Major, PreMajor::Minor), v("0.1.0"));
        assert_eq!(first_release(Bump::Minor), v("0.1.0"));
        assert_eq!(first_release(Bump::Patch), v("0.0.1"));
    }

    #[test]
    fn bump_ordering_is_the_one_max_relies_on() {
        assert!(Bump::None < Bump::Patch);
        assert!(Bump::Patch < Bump::Minor);
        assert!(Bump::Minor < Bump::Major);
    }
}

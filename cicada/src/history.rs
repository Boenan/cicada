//! Anchor resolution and range collection — the git half of planning.
//!
//! **The tag is the only anchor** (#2, #3). The versions file holds a version;
//! the tag is derived from it and resolved. There is no commit SHA stored
//! anywhere and therefore no fallback: a missing tag is a hard error naming the
//! tag and the repair, because a resolvable-but-*wrong* range produces a
//! changelog that looks fine and is incomplete — the failure mode the tool exists
//! to avoid.
//!
//! **Anchors are per-target, so ranges are too.** Packages release at different
//! times, so each gets its own `git log` (P1). N is small and each call is
//! milliseconds, so the union-range-plus-ancestry-filtering alternative buys
//! nothing but complexity.

use std::collections::BTreeMap;

use cicada_core::commit::{ParsedCommit, parse};
use cicada_core::config::{Config, PackageId, Target};
use cicada_core::plan::render_tag;
use cicada_core::versions::VersionsFile;

use crate::git::{GitBackend, GitError};

#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    /// J1. The repair is spelled out because the alternative — cicada guessing
    /// where the tag should have been — is the fallback #2 removed.
    #[error(
        "'{package}' records version {version}, but its anchor tag `{tag}` does not exist\n\
         the tag is the only anchor: cicada has no way to tell which commits {version} contained\n\
         repair: `git tag {tag} <the commit it was released from> && git push origin {tag}`"
    )]
    TagMissing { package: String, tag: String, version: String },

    /// J2 / G8. The same primitive as the collision check (#9), reported from the
    /// other end: a tag that exists elsewhere means the range would silently span
    /// the merge base.
    #[error(
        "'{package}' records version {version}, and its anchor tag `{tag}` exists but is not \
         reachable from {head}\n\
         the tag was cut on another line, so the range would silently span the merge base\n\
         check out the branch that tag belongs to, or correct `{package}` in .cicada-versions.toml"
    )]
    TagUnreachable { package: String, tag: String, version: String, head: String },

    #[error(transparent)]
    Git(#[from] GitError),
}

/// What each target's range holds, ready for the planner.
pub struct Collected {
    pub history: BTreeMap<Target, Vec<ParsedCommit>>,
    /// The anchor tag per target — `None` where the target has never released and
    /// the range is therefore all reachable history (A8, #4).
    pub anchors: BTreeMap<Target, Option<String>>,
}

/// Resolve every target's anchor and read its range.
///
/// Targets are visited in release order (#96) and the **first** failure stops the
/// run: in a four-package repo, "tag not found" without attribution is a scavenger
/// hunt, and planning the rest while one package is unresolvable would advertise a
/// release that cannot be computed (P4).
pub fn collect(
    git: &impl GitBackend,
    config: &Config,
    versions: &VersionsFile,
    head: &str,
) -> Result<Collected, AnchorError> {
    let mut history = BTreeMap::new();
    let mut anchors = BTreeMap::new();

    for i in 0..config.packages.len() {
        let (name, pkg) = config.packages.get_index(i).expect("index within config");
        let recorded = versions.package(name).and_then(|e| e.version.clone());
        let anchor = resolve(git, name, &pkg.tag_format, recorded.as_ref(), head)?;
        let commits = read_range(git, config, anchor.as_deref(), head)?;
        let t = Target::Package(PackageId(i));
        history.insert(t, commits);
        anchors.insert(t, anchor);
    }

    // The umbrella has its own anchor and its own range, independent of every
    // child's (Q10) — it is a fourth line that happens to live in one tree, not a
    // function of the others.
    if let Some(u) = &config.umbrella {
        let recorded = versions.umbrella.as_ref().and_then(|e| e.version.clone());
        let anchor = resolve(git, &u.name, &u.tag_format, recorded.as_ref(), head)?;
        let commits = read_range(git, config, anchor.as_deref(), head)?;
        history.insert(Target::Umbrella, commits);
        anchors.insert(Target::Umbrella, anchor);
    }

    Ok(Collected { history, anchors })
}

/// Anchor resolution for one target (design §Anchor resolution).
///
/// ```text
/// no entry in the versions file -> None  (first release: all reachable history)
/// otherwise: render the tag, resolve it, and require it to be reachable from HEAD
/// ```
fn resolve(
    git: &impl GitBackend,
    package: &str,
    tag_format: &str,
    recorded: Option<&semver::Version>,
    head: &str,
) -> Result<Option<String>, AnchorError> {
    // "Never released" is an absent entry, not a sentinel version (#4).
    let Some(version) = recorded else { return Ok(None) };
    let tag = render_tag(tag_format, version);

    if git.rev_parse(&tag)?.is_none() {
        return Err(AnchorError::TagMissing {
            package: package.to_string(),
            tag,
            version: version.to_string(),
        });
    }
    if !git.is_ancestor(&tag, head)? {
        return Err(AnchorError::TagUnreachable {
            package: package.to_string(),
            tag,
            version: version.to_string(),
            head: head.to_string(),
        });
    }
    Ok(Some(tag))
}

/// Read a range and parse what parses.
///
/// A subject that is not a conventional commit is **dropped**, not an error:
/// reading history must not stop on a malformed message, and a repo adopting
/// cicada has plenty of them behind its first anchor (A8). `check` is where a
/// malformed subject becomes a finding, at review time and at exit `3` (#12, #79).
fn read_range(
    git: &impl GitBackend,
    config: &Config,
    anchor: Option<&str>,
    head: &str,
) -> Result<Vec<ParsedCommit>, AnchorError> {
    let raw = git.log(anchor, head, config.settings.commit_source)?;
    Ok(raw.iter().filter_map(|c| parse(c).ok()).collect())
}

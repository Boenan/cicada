//! Branch identity.
//!
//! All three branch settings are **glob patterns matched against the full branch
//! name** (#82), never scalars living on the branch they describe. A scalar
//! travels through a merge and silently reconfigures the target branch; a pattern
//! reads identically everywhere, so cutting a line needs no config commit.
//!
//! Patterns are anchored at both ends: `*` matches within a path segment, `**`
//! crosses `/`. `release/*` therefore matches `release/1.4` but not
//! `release/1.4/hotfix`, which is why the defaults in #83 come in pairs.

use globset::{Glob, GlobSet, GlobSetBuilder};

#[derive(Debug, thiserror::Error)]
#[error("invalid glob `{pattern}`: {source}")]
pub struct GlobError {
    pub pattern: String,
    #[source]
    pub source: globset::Error,
}

/// Compile a pattern list. Kept separate from matching so a bad pattern is
/// reported against the config key that holds it rather than at match time.
pub fn compile(patterns: &[String]) -> Result<GlobSet, GlobError> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        let g = Glob::new(p).map_err(|e| GlobError { pattern: p.clone(), source: e })?;
        b.add(g);
    }
    // An empty builder yields a set that matches nothing, which is the correct
    // reading of an empty pattern list.
    b.build().map_err(|e| GlobError { pattern: String::new(), source: e })
}

pub fn matches_any(patterns: &[String], branch: &str) -> Result<bool, GlobError> {
    Ok(compile(patterns)?.is_match(branch))
}

/// Render `release_pr_branch` for a base branch. The branch's own slashes are
/// preserved rather than flattened, so two lines can never collide on one head
/// (#27): `cicada/{branch}` gives `cicada/release/1.4`.
pub fn render_pr_branch(template: &str, branch: &str) -> String {
    template.replace("{branch}", branch)
}

//! Scope routing — which package a commit is attributed to (#11).
//!
//! Attribution is by conventional-commit **scope**, and nothing else. No path
//! heuristics, no dependency graph: the author declares blast radius by what they
//! type, and `feat(api,worker):` releases both (O2). That is better information
//! than a graph can provide — the human knows whether an internal change is
//! breaking for its consumers, and a graph does not — which is why dependency
//! propagation is a permanent non-goal rather than a deferral.
//!
//! Two rules, both load-bearing:
//!
//! * an **empty** scope routes to `default_scope` (#38) — the umbrella in a
//!   monorepo, the sole package in a single-service repo, and the reason a
//!   single-service repo needs no special case (#16);
//! * an **unknown** scope is a hard error (#11), softened only by a did-you-mean.

use crate::check::did_you_mean;
use crate::commit::ParsedCommit;
use crate::config::{Config, Target};

/// A scope that names no package and no umbrella. Carried rather than thrown so a
/// caller can report *every* bad scope in a range at once, rather than one per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownScope {
    pub sha: String,
    pub scope: String,
    /// The nearest configured name, when there is one (#12).
    pub suggestion: Option<String>,
}

impl std::fmt::Display for UnknownScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sha: String = self.sha.chars().take(7).collect();
        write!(f, "{sha}: unknown scope '{}' — no such package", self.scope)
    }
}

/// Everything this commit is attributed to, in the order the scopes were written.
///
/// A multi-scope commit appears in **each** target's set, which is how one commit
/// lands in two changelogs (O2). Duplicates within one commit (`feat(api,api):`)
/// collapse, so a repeated scope cannot count twice.
pub fn targets_of(config: &Config, commit: &ParsedCommit) -> Result<Vec<Target>, UnknownScope> {
    if commit.scopes.is_empty() {
        // `default_scope` is validated to resolve by `check`, and every command
        // runs those validations before planning — so this cannot silently route
        // to nothing.
        let t = config.resolve_scope(&config.settings.default_scope).ok_or_else(|| {
            UnknownScope {
                sha: commit.sha.clone(),
                scope: config.settings.default_scope.clone(),
                suggestion: None,
            }
        })?;
        return Ok(vec![t]);
    }

    let mut out: Vec<Target> = Vec::new();
    for scope in &commit.scopes {
        let t = config.resolve_scope(scope).ok_or_else(|| UnknownScope {
            sha: commit.sha.clone(),
            scope: scope.clone(),
            suggestion: did_you_mean(scope, &config.scope_targets()).map(str::to_string),
        })?;
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

/// Is this commit attributed to `target`?
pub fn routes_to(config: &Config, commit: &ParsedCommit, target: Target) -> bool {
    targets_of(config, commit).is_ok_and(|ts| ts.contains(&target))
}

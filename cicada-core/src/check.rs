//! `cicada check` — the pure half.
//!
//! Two independent jobs, deliberately kept apart (#12, #79):
//!
//! * [`lint_commits`] — are these commits conventional, and do their scopes name
//!   real packages?
//! * [`validate_config`] — is this config internally coherent?
//!
//! Both return *findings* rather than errors. A finding is exit `3` (cicada read
//! your config and your commits and found problems); a config that will not parse
//! at all is exit `1`, and never reaches here.
//!
//! Filesystem-dependent validations — a package `path` that exists, two crates
//! resolving to one Cargo version field (#24) — cannot live here (#29). They are
//! in the binary and append to the same `Vec<Finding>`.

use crate::commit::{ParseError, ParsedCommit, RawCommit, parse};
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Contributes to exit `3`.
    Problem,
    /// Reported, does not change the exit code.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    /// What the finding is about: a commit sha, a config key, a scope.
    pub subject: String,
    pub message: String,
    /// The one line that would fix it, when there is one.
    pub help: Option<String>,
}

impl Finding {
    pub fn problem(subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Problem,
            subject: subject.into(),
            message: message.into(),
            help: None,
        }
    }
    pub fn warning(subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            subject: subject.into(),
            message: message.into(),
            help: None,
        }
    }
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// The outcome of linting one commit, kept so the caller can render a ✓ per
/// commit rather than only listing failures.
#[derive(Debug, Clone)]
pub struct CommitReport {
    pub sha: String,
    pub subject: String,
    pub parsed: Option<ParsedCommit>,
    pub findings: Vec<Finding>,
}

impl CommitReport {
    pub fn ok(&self) -> bool {
        self.findings.iter().all(|f| f.severity != Severity::Problem)
    }
}

/// Lint a range of commits against the config.
pub fn lint_commits(config: &Config, commits: &[RawCommit]) -> Vec<CommitReport> {
    commits.iter().map(|c| lint_commit(config, c)).collect()
}

fn lint_commit(config: &Config, raw: &RawCommit) -> CommitReport {
    let mut findings = Vec::new();
    let parsed = match parse(raw) {
        Ok(p) => Some(p),
        Err(e) => {
            let mut f = Finding::problem(short_sha(&raw.sha), e.to_string());
            if e == ParseError::NotConventional {
                f = f.with_help("expected `type(scope): description`, e.g. `fix(api): handle nil`");
            }
            findings.push(f);
            None
        }
    };

    if let Some(p) = &parsed {
        // Unknown scope is fatal (#11) — and the did-you-mean is what turns that
        // hard error from an obstacle into help.
        for scope in &p.scopes {
            if config.resolve_scope(scope).is_none() {
                let mut f = Finding::problem(
                    short_sha(&raw.sha),
                    format!("unknown scope '{scope}' — no such package"),
                );
                f = match did_you_mean(scope, &config.scope_targets()) {
                    Some(s) => f.with_help(format!("did you mean '{s}'?")),
                    None => f.with_help(format!("known scopes: {}", config.scope_targets().join(", "))),
                };
                findings.push(f);
            }
        }
    }

    CommitReport {
        sha: raw.sha.clone(),
        subject: raw.subject.clone(),
        parsed,
        findings,
    }
}

/// Edit distance over the configured names (#12).
///
/// **Damerau-Levenshtein, not plain Levenshtein**, because a transposition is the
/// commonest scope typo and plain Levenshtein charges 2 for it — which is over the
/// threshold for a short name and silently suppresses the suggestion. `aip` → `api`
/// is the design's own worked example (§`cicada check`), and it is a transposition.
///
/// Deliberately conservative past that: a scope with no near match gets the full
/// list rather than a misleading guess (monorepo Y2).
pub fn did_you_mean<'a>(input: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let max = match input.chars().count() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    candidates
        .iter()
        .map(|c| (*c, strsim::damerau_levenshtein(input, c)))
        .filter(|(_, d)| *d <= max)
        .min_by_key(|(c, d)| (*d, c.len()))
        .map(|(c, _)| c)
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Validations that are a pure function of the config (design §Loading and
/// validation). The filesystem half lives in the binary.
pub fn validate_config(config: &Config) -> Vec<Finding> {
    let mut out = Vec::new();

    if config.settings.release_branches.is_empty() {
        out.push(
            Finding::problem("settings.release_branches", "must not be empty")
                .with_help("cicada would exit 0 on every branch and release nothing, forever"),
        );
    }

    if config.packages.is_empty() {
        out.push(Finding::problem(
            "packages",
            "no packages declared — the package list is explicit, always (#8)",
        ));
    }

    // default_scope must name a real package or the umbrella (#38, #73).
    if config.resolve_scope(&config.settings.default_scope).is_none() {
        let targets = config.scope_targets();
        let mut f = Finding::problem(
            "settings.default_scope",
            format!("'{}' names no package or umbrella", config.settings.default_scope),
        );
        f = match did_you_mean(&config.settings.default_scope, &targets) {
            Some(s) => f.with_help(format!("did you mean '{s}'?")),
            None => f.with_help(format!("valid targets: {}", targets.join(", "))),
        };
        out.push(f);
    }

    // Every tag_format must template the version, and no two may collide (#9).
    let mut tag_formats: Vec<(&str, &str)> = Vec::new();
    for (name, pkg) in &config.packages {
        if !pkg.tag_format.contains("{version}") {
            out.push(Finding::problem(
                format!("packages.{name}.tag_format"),
                "must contain `{version}`",
            ));
        }
        tag_formats.push((name.as_str(), pkg.tag_format.as_str()));
    }
    if let Some(u) = &config.umbrella {
        if !u.tag_format.contains("{version}") {
            out.push(Finding::problem("umbrella.tag_format", "must contain `{version}`"));
        }
        tag_formats.push((u.name.as_str(), u.tag_format.as_str()));
    }
    for i in 0..tag_formats.len() {
        for j in (i + 1)..tag_formats.len() {
            if tag_formats[i].1 == tag_formats[j].1 {
                out.push(
                    Finding::problem(
                        format!("packages.{}.tag_format", tag_formats[j].0),
                        format!(
                            "duplicate tag format `{}` — also used by '{}'",
                            tag_formats[j].1, tag_formats[i].0
                        ),
                    )
                    .with_help("two packages producing one tag is a version collision waiting to happen"),
                );
            }
        }
    }

    // No two packages may share a path.
    let mut seen_paths: Vec<(&str, &str)> = Vec::new();
    for (name, pkg) in &config.packages {
        if let Some((other, _)) = seen_paths.iter().find(|(_, p)| *p == pkg.path) {
            out.push(Finding::problem(
                format!("packages.{name}.path"),
                format!("duplicate path `{}` — also used by '{other}'", pkg.path),
            ));
        }
        seen_paths.push((name.as_str(), pkg.path.as_str()));
    }

    // release_pr_branch must template {branch}; whether its *rendered* form lands
    // inside the release allowlist needs a concrete branch, so that half is
    // `validate_release_pr_branch` below (#27).
    if !config.settings.release_pr_branch.contains("{branch}") {
        out.push(
            Finding::problem("settings.release_pr_branch", "must contain `{branch}`")
                .with_help("two release lines would otherwise share one head branch"),
        );
    }

    // A pattern that will not compile is a config problem, reported against the
    // key that holds it rather than at match time.
    for (key, pats) in [
        ("settings.release_branches", &config.settings.release_branches),
        ("settings.maintenance_branches", &config.settings.maintenance_branches),
        ("settings.prerelease_branches", &config.settings.prerelease_branches),
    ] {
        if let Err(e) = crate::branch::compile(pats) {
            out.push(Finding::problem(key, e.to_string()));
        }
    }

    // Updater coherence, and the two cross-references a package can get wrong.
    let mut outputs: Vec<(&str, &str)> = Vec::new();
    for (name, pkg) in &config.packages {
        let mut has_helm = false;
        for (i, u) in pkg.updaters.iter().enumerate() {
            use crate::config::UpdaterKind::*;
            let key = format!("packages.{name}.updaters[{i}]");
            match u.kind {
                Cargo | Helm | Generic => {
                    if u.path.is_none() {
                        out.push(Finding::problem(&key, "`path` is required for this updater kind"));
                    }
                }
                Template => {
                    if u.template.is_none() || u.output.is_none() {
                        out.push(Finding::problem(
                            &key,
                            "`template` and `output` are both required for a template updater",
                        ));
                    }
                }
            }
            if u.kind == Helm {
                has_helm = true;
                if let Some(av) = &u.app_version
                    && config.package_id(&av.track).is_none()
                {
                    let targets: Vec<&str> = config.package_names().collect();
                    let mut f = Finding::problem(
                        format!("{key}.app_version.track"),
                        format!("'{}' names no package", av.track),
                    );
                    f = match did_you_mean(&av.track, &targets) {
                        Some(s) => f.with_help(format!("did you mean '{s}'?")),
                        None => f.with_help(format!("valid packages: {}", targets.join(", "))),
                    };
                    out.push(f);
                }
            }
            // Two updaters writing one output path is a silent, config-order-
            // dependent bug, so it is a hard error (#44).
            if let Some(o) = &u.output {
                if let Some((other, _)) = outputs.iter().find(|(_, p)| p == o) {
                    out.push(Finding::problem(
                        format!("{key}.output"),
                        format!("duplicate output path `{o}` — also written by '{other}'"),
                    ));
                }
                outputs.push((name.as_str(), o.as_str()));
            }
        }

        // Helm mandates `version` in Chart.yaml, so the number already exists and
        // its author knows it — cicada never invents one (#88).
        if has_helm && pkg.initial_version.is_none() {
            out.push(
                Finding::problem(
                    format!("packages.{name}.initial_version"),
                    "required on a package with a `helm` updater",
                )
                .with_help("Helm requires `version` in Chart.yaml; state the chart's current version here"),
            );
        }
    }

    out
}

/// The half of the `release_pr_branch` check that needs a concrete branch (#27).
///
/// The prefix is a **namespace, not a label**. With
/// `release_branches = ["main", "release/*"]`, a head template of
/// `release/{branch}` renders `release/main` — which matches `release/*`, so
/// cicada's own force-push re-triggers cicada on the branch it just wrote.
pub fn validate_release_pr_branch(config: &Config, branch: &str) -> Option<Finding> {
    let head = crate::branch::render_pr_branch(&config.settings.release_pr_branch, branch);
    let matches = crate::branch::matches_any(&config.settings.release_branches, &head).ok()?;
    if !matches {
        return None;
    }
    Some(
        Finding::problem(
            "settings.release_pr_branch",
            format!("renders to `{head}`, which matches `release_branches`"),
        )
        .with_help("cicada's own push to that branch would re-trigger it; use a reserved prefix such as `cicada/{branch}`"),
    )
}

//! `cicada.toml` — the human-owned config (design §Configuration).
//!
//! The *types* and every validation that is a pure function of them live here.
//! Validations that need the filesystem — "this path exists", "these two crates
//! resolve to one Cargo version field" — live in the binary, because this crate
//! cannot open a file (#29).
//!
//! Parsing **denies unknown fields** (#67): a mistyped `tag_fromat` is an error
//! naming the key, never a setting that silently does nothing.

use indexmap::IndexMap;
use serde::Deserialize;

/// A package's position in config order. Config order is the repo's one
/// observable ordering (#31), so identifying a package by its ordinal makes
/// "ordered by config order" mechanical rather than a convention to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageId(pub usize);

/// The umbrella, when one is declared, sorts after every package — which is the
/// release order decision #96 requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Package(PackageId),
    Umbrella,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub settings: Settings,
    pub forge: Forge,
    #[serde(default)]
    pub umbrella: Option<Umbrella>,
    #[serde(default = "Changelog::default")]
    pub changelog: Changelog,
    #[serde(default)]
    pub packages: IndexMap<String, Package>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Required (#83). The gate: cicada acts only on a branch matching one of
    /// these. No default, because every wrong guess here is silent.
    pub release_branches: Vec<String>,
    /// Required (#38). Where unscoped commits land.
    pub default_scope: String,

    #[serde(default = "default_maintenance_branches")]
    pub maintenance_branches: Vec<String>,
    #[serde(default = "default_prerelease_branches")]
    pub prerelease_branches: Vec<String>,
    #[serde(default = "default_prerelease_identifier")]
    pub prerelease_identifier: String,

    #[serde(default = "default_committer")]
    pub committer: String,
    #[serde(default)]
    pub commit_source: CommitSource,
    #[serde(default = "default_release_pr_branch")]
    pub release_pr_branch: String,
    #[serde(default = "default_release_pr_title")]
    pub release_pr_title: String,
}

/// Which commits a range is read from (#40). Identical on squash/rebase repos;
/// only a merge-commit repo can tell the difference (J5, J7).
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommitSource {
    /// `--no-merges`
    #[default]
    Commits,
    /// `--first-parent`
    Merges,
    Both,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Forge {
    /// Required (#66). Never inferred from the remote at release time.
    pub kind: ForgeKind,
    #[serde(default)]
    pub api_url: Option<String>,
    /// Derived from `origin` when absent — the one forge value that *is* derived,
    /// because it cannot be ambiguous.
    #[serde(default)]
    pub repo: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    Github,
    Gitlab,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Umbrella {
    pub name: String,
    pub tag_format: String,
    pub changelog: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Changelog {
    pub sections: Vec<Section>,
}

impl Default for Changelog {
    /// The five-row table from §Configuration, printable and overridable like any
    /// other default (#48).
    fn default() -> Self {
        let row = |t: &str, title: &str, bump: Bump, hidden: bool| Section {
            kind: t.into(),
            title: title.into(),
            bump,
            hidden,
        };
        Changelog {
            sections: vec![
                row("feat", "Features", Bump::Minor, false),
                row("fix", "Bug Fixes", Bump::Patch, false),
                row("perf", "Performance", Bump::Patch, false),
                row("docs", "Documentation", Bump::None, false),
                row("chore", "Chores", Bump::None, true),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// The commit type this section collects. Named `type` in TOML.
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    /// Whether this type justifies cutting a release at all (#36) — a separate
    /// question from whether it renders, which `title`/`hidden` answer.
    pub bump: Bump,
    #[serde(default)]
    pub hidden: bool,
}

/// Declaration order **is** the ordering (#36): `max` over a range's commits is
/// how the largest justified bump is found, so `None < Patch < Minor < Major` is
/// load-bearing rather than cosmetic.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    None,
    Patch,
    Minor,
    Major,
}

impl Bump {
    pub fn as_str(self) -> &'static str {
        match self {
            Bump::None => "none",
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub path: String,
    /// Required (#83). Two packages silently sharing a tag shape is a version
    /// collision waiting to happen (#9).
    pub tag_format: String,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub initial_version: Option<String>,
    #[serde(default)]
    pub pre_major: Option<PreMajor>,
    #[serde(default)]
    pub updaters: Vec<Updater>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreMajor {
    Minor,
    Major,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Updater {
    pub kind: UpdaterKind,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub edit: Option<toml::Value>,
    #[serde(default)]
    pub app_version: Option<AppVersion>,
    #[serde(default)]
    pub on_tracked_bump: Option<TrackedBump>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdaterKind {
    Cargo,
    Helm,
    Generic,
    Template,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppVersion {
    pub track: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TrackedBump {
    Patch,
    Minor,
}

// --- defaults (#83) ------------------------------------------------------

fn default_maintenance_branches() -> Vec<String> {
    vec!["maintenance/**".into(), "maintenance-*".into()]
}
fn default_prerelease_branches() -> Vec<String> {
    vec!["prerelease/**".into(), "prerelease-*".into()]
}
fn default_prerelease_identifier() -> String {
    "rc".into()
}
fn default_committer() -> String {
    "cicada[bot] <cicada@users.noreply.github.com>".into()
}
fn default_release_pr_branch() -> String {
    "cicada/{branch}".into()
}
fn default_release_pr_title() -> String {
    "chore(release): prepare release".into()
}

// --- loading -------------------------------------------------------------

/// A config that would not parse. Distinct from a *finding* — `check` exits `1`
/// here and `3` for semantic problems (#79).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(String);

impl Config {
    /// Parse from a string. Reading the file is the binary's job (#29).
    pub fn from_toml(src: &str) -> Result<Self, ParseError> {
        toml::from_str(src).map_err(|e| ParseError(e.to_string()))
    }

    pub fn package_names(&self) -> impl Iterator<Item = &str> {
        self.packages.keys().map(|s| s.as_str())
    }

    pub fn package_id(&self, name: &str) -> Option<PackageId> {
        self.packages.get_index_of(name).map(PackageId)
    }

    pub fn package_name(&self, id: PackageId) -> Option<&str> {
        self.packages.get_index(id.0).map(|(k, _)| k.as_str())
    }

    pub fn umbrella_name(&self) -> Option<&str> {
        self.umbrella.as_ref().map(|u| u.name.as_str())
    }

    /// Resolve a commit scope to the thing it releases. `None` means the scope
    /// names nothing, which is a hard error at the call site (#11).
    pub fn resolve_scope(&self, scope: &str) -> Option<Target> {
        if self.umbrella_name() == Some(scope) {
            return Some(Target::Umbrella);
        }
        self.package_id(scope).map(Target::Package)
    }

    /// Every name a scope may legally take, in config order, umbrella last.
    pub fn scope_targets(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.package_names().collect();
        if let Some(u) = self.umbrella_name() {
            v.push(u);
        }
        v
    }

    /// The section governing a commit type, if one is configured. An unmatched
    /// type is ignored rather than an error (design §Commit parsing).
    pub fn section_for(&self, kind: &str) -> Option<&Section> {
        self.changelog.sections.iter().find(|s| s.kind == kind)
    }
}

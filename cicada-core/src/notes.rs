//! The three template contexts (#46), and the link values they carry (A2).
//!
//! `commits` means something different in each, so they are defined separately
//! rather than as one flat list:
//!
//! | Context | Renders | `commits` holds |
//! |---|---|---|
//! | [`FileContext`] | once per package release | that package's commits |
//! | [`NotesContext`] | once per package release | the same, **pre-grouped into sections** |
//! | [`UmbrellaContext`] | once per run | every release in the run, grouped by package |
//!
//! **Section grouping stays config-driven.** The template receives sections
//! already grouped and decides only how to render them — otherwise config and
//! template could contradict each other, and the config would silently lose.
//!
//! **Links are data, not a dependency** (A2). Compare, commit and issue URL styles
//! differ per forge, but core must not depend on the forge layer — so the adapter
//! supplies a [`LinkStyle`] *value* and core substitutes into it. A trait object
//! here would leak the forge back into a crate whose whole purpose is not having it.

use serde::Serialize;

use crate::commit::ParsedCommit;
use crate::config::Config;
use crate::plan::Release;

/// URL templates, supplied by the forge adapter as a value.
///
/// `{sha}`, `{from}` and `{to}` are substituted. `None` everywhere is legitimate
/// and is what `plan`/`--no-push` uses when no forge is configured — the templates
/// guard on the field, so notes render without links rather than with broken ones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkStyle {
    pub commit: Option<String>,
    pub compare: Option<String>,
}

impl LinkStyle {
    pub fn commit_url(&self, sha: &str) -> Option<String> {
        self.commit.as_ref().map(|t| t.replace("{sha}", sha))
    }

    pub fn compare_url(&self, from: &str, to: &str) -> Option<String> {
        self.compare.as_ref().map(|t| t.replace("{from}", from).replace("{to}", to))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitContext {
    pub sha: String,
    pub short_sha: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub scopes: Vec<String>,
    /// The scopes joined for display; empty when the commit was unscoped.
    pub scope: String,
    pub breaking: bool,
    /// The `BREAKING CHANGE:` footer's text, when there was one. A `!` with no
    /// footer leaves this empty — the marker says *that* it breaks, the footer
    /// says *how*.
    pub breaking_note: String,
    pub description: String,
    pub body: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SectionContext {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub commits: Vec<CommitContext>,
}

/// The context a **file template** receives (`template` updater kind).
#[derive(Debug, Clone, Serialize)]
pub struct FileContext {
    pub package: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub tag: String,
    pub date: String,
    pub bump: String,
    pub commits: Vec<CommitContext>,
}

/// The context the **notes template** receives — one entry body (#43).
#[derive(Debug, Clone, Serialize)]
pub struct NotesContext {
    pub package: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub tag: String,
    pub date: String,
    pub bump: String,
    pub reason: String,
    /// Pre-grouped, in config order, hidden sections omitted (#46).
    pub sections: Vec<SectionContext>,
    /// Every commit routed to this release, including ones in hidden sections and
    /// ones whose type is configured nowhere. The sections decide what *renders*;
    /// this is what the release *contains*.
    pub commits: Vec<CommitContext>,
    pub breaking: Vec<CommitContext>,
    /// How a release with no commits explains itself (#97).
    pub derived_from: Option<DerivedContext>,
    pub compare_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DerivedContext {
    pub package: String,
    pub version: String,
}

/// One release as the umbrella's changelog sees it: its rendered body plus enough
/// identity to head a section with.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseSummary {
    pub package: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub tag: String,
    pub bump: String,
    pub notes: String,
}

/// The context the **umbrella changelog** receives — once per run (#15).
#[derive(Debug, Clone, Serialize)]
pub struct UmbrellaContext {
    pub package: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub tag: String,
    pub date: String,
    pub bump: String,
    pub releases: Vec<ReleaseSummary>,
}

fn commit_context(c: &ParsedCommit, links: &LinkStyle) -> CommitContext {
    let breaking_note = c
        .footers
        .iter()
        .find(|f| {
            f.key.eq_ignore_ascii_case("BREAKING CHANGE")
                || f.key.eq_ignore_ascii_case("BREAKING-CHANGE")
        })
        .map(|f| f.value.clone())
        .unwrap_or_default();

    CommitContext {
        short_sha: c.sha.chars().take(7).collect(),
        sha: c.sha.clone(),
        kind: c.kind.clone(),
        scope: c.scopes.join(", "),
        scopes: c.scopes.clone(),
        breaking: c.breaking,
        breaking_note,
        description: c.description.clone(),
        body: c.body.clone(),
        url: links.commit_url(&c.sha),
    }
}

/// Group a release's commits by configured section, in config order.
///
/// Two commits are deliberately *not* dropped here: a commit whose type is in a
/// `hidden` section, and one whose type is configured nowhere. Neither appears in
/// `sections`, and both stay in `commits` — because hiding is a rendering decision
/// and the release still contains them.
fn sections(config: &Config, commits: &[CommitContext]) -> Vec<SectionContext> {
    config
        .changelog
        .sections
        .iter()
        .filter(|s| !s.hidden)
        .filter_map(|s| {
            let mine: Vec<CommitContext> =
                commits.iter().filter(|c| c.kind == s.kind).cloned().collect();
            if mine.is_empty() {
                return None;
            }
            Some(SectionContext { kind: s.kind.clone(), title: s.title.clone(), commits: mine })
        })
        .collect()
}

pub fn notes_context(
    config: &Config,
    r: &Release,
    date: &str,
    links: &LinkStyle,
) -> NotesContext {
    let commits: Vec<CommitContext> =
        r.commits.iter().map(|c| commit_context(c, links)).collect();
    let breaking: Vec<CommitContext> = commits.iter().filter(|c| c.breaking).cloned().collect();
    let previous_version = r.from.as_ref().map(ToString::to_string);

    // A first release has no previous tag, so there is nothing to compare against
    // and a compare URL built from `0.0.0` would 404 (#89).
    let compare_url = match (&r.from, &links.compare) {
        (Some(from), Some(_)) => {
            let from_tag = crate::plan::render_tag(tag_format(config, r), from);
            links.compare_url(&from_tag, &r.tag)
        }
        _ => None,
    };

    NotesContext {
        package: r.name.clone(),
        version: r.to.to_string(),
        previous_version,
        tag: r.tag.clone(),
        date: date.to_string(),
        bump: r.bump.as_str().to_string(),
        reason: r.reason.as_str().to_string(),
        sections: sections(config, &commits),
        commits,
        breaking,
        derived_from: r.derived_from.as_ref().map(|d| DerivedContext {
            package: d.package.clone(),
            version: d.version.to_string(),
        }),
        compare_url,
    }
}

pub fn file_context(r: &Release, date: &str, links: &LinkStyle) -> FileContext {
    FileContext {
        package: r.name.clone(),
        version: r.to.to_string(),
        previous_version: r.from.as_ref().map(ToString::to_string),
        tag: r.tag.clone(),
        date: date.to_string(),
        bump: r.bump.as_str().to_string(),
        commits: r.commits.iter().map(|c| commit_context(c, links)).collect(),
    }
}

/// The `tag_format` a release's target declares — needed to spell the *previous*
/// tag for a compare URL.
fn tag_format<'a>(config: &'a Config, r: &Release) -> &'a str {
    match r.target {
        crate::config::Target::Umbrella => {
            config.umbrella.as_ref().map(|u| u.tag_format.as_str()).unwrap_or("{version}")
        }
        crate::config::Target::Package(id) => config
            .packages
            .get_index(id.0)
            .map(|(_, p)| p.tag_format.as_str())
            .unwrap_or("{version}"),
    }
}

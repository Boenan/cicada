//! Phase B — `cicada release`. **Stateless reconciliation.**
//!
//! Phase B asks one question per package, twice: *does the tag for the recorded
//! version exist, and does a release object exist for it?* It reads no forge
//! state to decide anything, consults no labels, and stores nothing (#9).
//!
//! ```text
//! for pkg in config order, umbrella last (#96):
//!     if pkg has no entry in the versions file:   # never released (#4, A7)
//!         skip
//!     tag = tag_for(pkg, versions[pkg])
//!     match resolve(tag):                        # artifact 1: the tag
//!         exists, NOT reachable -> ERROR: version collision (#9)
//!         exists, reachable     -> already tagged
//!         missing               -> staleness_guard(pkg)   # #78
//!                                  create and push the tag
//!     if no release object for tag:              # artifact 2: the release
//!         create it (notes + embedded manifest)
//! ```
//!
//! **Tag and release object reconcile independently** (#56). Skipping the whole
//! package when the tag exists would break self-healing: if tagging succeeded and
//! the release API call then failed, the release object would never be created on
//! any later run. Two artifacts, two checks, both idempotent (C4).
//!
//! Consequences, all of them good: immune to squash-merge, merge commits,
//! rebase-merge and rewritten commit messages; idempotent, so re-running a
//! pipeline double-tags nothing; self-healing; a hand-edited versions file is a
//! valid manual release (J4); and zero forge state.

use std::collections::BTreeMap;

use semver::Version;

use cicada_core::config::{Config, PackageId, Target};
use cicada_core::manifest::{Entry, Manifest};
use cicada_core::notes::LinkStyle;
use cicada_core::plan::{Plan, PlanInput, Release, parse_tag, render_tag};
use cicada_core::versions::VersionsFile;
use cicada_core::{Identity, template};

use crate::forge::{Forge, ReleaseRequest};
use crate::git::{GitBackend, GitError};

#[derive(Debug, thiserror::Error)]
pub enum PhaseBError {
    /// #9 — the tag exists somewhere this branch cannot see, so two release lines
    /// are converging on one number. Skipping it would silently swallow a
    /// release: commits made, changelog written, nothing tagged or published.
    #[error(
        "'{package}' records version {version}, and tag `{tag}` already exists but is not \
         reachable from HEAD\n\
         two release lines are converging on one number — reconcile them before releasing either"
    )]
    Collision { package: String, version: String, tag: String },

    /// #76 — a recorded version that goes **backwards** past a tag this branch
    /// has already published.
    ///
    /// J4b claims the staleness guard catches a downward hand-edit "with no extra
    /// machinery", and it does not: #90 selects `prev` from tags *below* the
    /// recorded version, so editing `1.4.2` down to `1.0.0` leaves no eligible
    /// `prev` at all, the recomputation becomes a first release, and `1.0.0`
    /// comfortably clears it. The two rules cannot both hold, so monotonicity is
    /// checked directly — which is what #76 says in the first place.
    #[error(
        "'{package}' is recorded at {recorded}, but `{tag}` ({published}) is already released on \
         this branch\n\
         a version may only move forwards: {recorded} would re-release a number the line has \
         passed\n\
         \n\
         correct `{package}` in .cicada-versions.toml to something above {published}"
    )]
    WentBackwards { package: String, recorded: String, tag: String, published: String },

    /// #33 layer 3 — the release PR was merged while stale.
    ///
    /// Hard-errors rather than warning, with eyes open: the branch is then
    /// merged-but-untagged and needs manual repair. That is the correct trade —
    /// **a visible unreleased state beats an invisible mis-versioned release.**
    #[error(
        "'{package}' is recorded at {recorded}, but the commits in range justify {expected}\n\
         the release pull request was merged while stale, so the version it carries is lower than\n\
         the changes it contains — shipping it would break the semver contract silently\n\
         \n\
         no tag was created. Correct `{package}` in .cicada-versions.toml to {expected} (or higher)\n\
         and push; cicada will tag it on the next run."
    )]
    StaleMerge { package: String, recorded: String, expected: String },

    #[error(transparent)]
    Git(#[from] GitError),

    #[error(transparent)]
    Forge(#[from] crate::forge::ForgeError),

    #[error(transparent)]
    Plan(#[from] cicada_core::plan::PlanError),

    #[error("{0}")]
    Render(String),
}

pub struct PhaseBInput<'a> {
    pub config: &'a Config,
    pub versions: &'a VersionsFile,
    pub date: &'a str,
    pub links: &'a LinkStyle,
    pub identity: &'a Identity,
    pub overrides: &'a BTreeMap<String, String>,
    /// The HTTPS push URL and its credentials, or `None` for `--no-push`.
    pub push: Option<(&'a str, &'a [(String, String)])>,
    /// `None` when no forge is reachable — a rehearsal, or `--no-push`.
    pub forge: Option<&'a dyn Forge>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    /// Tags created by this run, in config order.
    pub tagged: Vec<(String, String)>,
    /// Tags that already existed and were left alone.
    pub already_tagged: Vec<String>,
    /// Release objects created by this run.
    pub released: Vec<(String, String)>,
    pub manifest: Manifest,
}

impl Outcome {
    /// True when the run changed nothing at all — the assertion C3 makes about a
    /// re-run.
    pub fn is_no_op(&self) -> bool {
        self.tagged.is_empty() && self.released.is_empty()
    }
}

pub fn run(git: &impl GitBackend, input: &PhaseBInput<'_>) -> Result<Outcome, PhaseBError> {
    let cfg = input.config;
    let head = git.rev_parse("HEAD")?.unwrap_or_default();
    let all_tags = git.tags()?;

    // Targets in release order (#96): config order, umbrella last. The umbrella's
    // embedded manifest therefore describes work already done rather than work in
    // flight.
    let mut targets: Vec<Target> =
        (0..cfg.packages.len()).map(|i| Target::Package(PackageId(i))).collect();
    if cfg.umbrella.is_some() {
        targets.push(Target::Umbrella);
    }

    // --- the replan, computed once ------------------------------------------
    //
    // It serves two purposes that must agree: the staleness guard's `expected`,
    // and the commits the release notes render from. Computing it twice would let
    // them drift.
    let mut history = BTreeMap::new();
    // The versions the replan computes *from*.
    //
    // **Not the recorded versions.** `expected = plan(prev..HEAD)` means both the
    // range and the base come from `prev`: anchoring the range at `prev` while
    // basing the arithmetic on the recorded version applies the bump twice, so a
    // perfectly ordinary release recomputes one bump too high and the guard fires
    // on every run. The recorded version is what is being *released*; `prev` is
    // what it was released *from*.
    let mut base = VersionsFile::default();
    let mut highest: BTreeMap<Target, Option<(Version, String)>> = BTreeMap::new();
    for t in &targets {
        highest.insert(*t, highest_reachable_stable(git, cfg, input.versions, *t, &all_tags)?);
        let prev = previous_stable_tag(git, cfg, input.versions, *t, &all_tags)?;
        let commits = match &prev {
            Some(tag) => git.log(Some(tag), "HEAD", cfg.settings.commit_source)?,
            None => git.log(None, "HEAD", cfg.settings.commit_source)?,
        };
        history.insert(
            *t,
            commits.iter().filter_map(|c| cicada_core::commit::parse(c).ok()).collect(),
        );

        // No `prev` means this is a first release, and an **absent** entry is how
        // that is said (#4) — the first-release table then applies, exactly as it
        // did when Phase A computed the number.
        if let Some(tag) = &prev
            && let Some((name, tag_format, _)) = target_bits(cfg, input.versions, *t)
            && let Some(v) = parse_tag(&tag_format, tag)
        {
            let entry = cicada_core::versions::Entry { version: v, prerelease: None };
            match t {
                Target::Umbrella => base.umbrella = Some(entry),
                Target::Package(_) => {
                    base.packages.insert(name, entry);
                }
            }
        }
    }

    let replan = cicada_core::plan::plan(&PlanInput {
        config: cfg,
        versions: &base,
        history: &history,
        // Deliberately off. The maintenance guard (#28) gates *opening* a release;
        // re-firing it here would refuse to tag work that is already merged, and
        // a stale merge onto a patch-only line is caught by the guard below
        // anyway — by the check that is actually about this situation.
        maintenance: false,
        branch: String::new(),
    })?;

    let mut out = Outcome {
        manifest: Manifest { date: input.date.to_string(), releases: Vec::new() },
        ..Default::default()
    };
    let mut pending: Vec<(Target, String, String, String)> = Vec::new();

    // --- artifact 1: the tag -------------------------------------------------
    for t in &targets {
        let Some((name, tag_format, entry)) = target_bits(cfg, input.versions, *t) else {
            // No entry in the versions file: never released, nothing to
            // reconcile. Not an error — this is A7, a virgin repo's Phase B.
            continue;
        };
        let version = entry.prerelease.clone().unwrap_or_else(|| entry.version.clone());
        let tag = render_tag(&tag_format, &version);

        let sha = match git.rev_parse(&tag)? {
            Some(sha) => {
                // **Reachability is load-bearing, not a nicety.** cicada's own
                // tags are always ancestors of HEAD on a later run, so idempotency
                // is preserved. A tag that exists *elsewhere* means two branches
                // are about to claim the same version.
                if !git.is_ancestor(&tag, "HEAD")? {
                    return Err(PhaseBError::Collision {
                        package: name.clone(),
                        version: version.to_string(),
                        tag,
                    });
                }
                out.already_tagged.push(tag.clone());
                sha
            }
            None => {
                // #78 — both guards run **only** here, at the moment a tag is
                // about to be created. Run unconditionally they would fire on
                // every ordinary push after a release: once `v1.5.0` is tagged,
                // the next `fix:` legitimately makes the recomputation say
                // `1.5.1` while the versions file still says `1.5.0`. Only now is
                // the versions file claiming to describe HEAD.
                monotonic_guard(&name, &entry.version, &highest[t])?;
                staleness_guard(&name, &version, &replan, *t)?;

                let message = format!("{name} {version}");
                git.create_tag(&tag, &head, &message, input.identity)?;
                if let Some((url, env)) = input.push {
                    git.push_tag(url, &tag, env)?;
                }
                out.tagged.push((name.clone(), tag.clone()));
                head.clone()
            }
        };

        out.manifest.releases.push(Entry {
            package: name.clone(),
            version: version.to_string(),
            tag: tag.clone(),
            sha: sha.clone(),
        });
        pending.push((*t, name, tag, sha));
    }

    // --- artifact 2: the release object -------------------------------------
    //
    // Checked independently of the tag (#56). If tagging succeeded and the
    // release API call then failed, this is what creates just the missing release
    // on the next run (C4) — and skipping the package on "tag exists" would
    // strand it forever.
    let Some(forge) = input.forge else { return Ok(out) };

    for (t, name, tag, sha) in pending {
        if forge.find_release(&tag)?.is_some() {
            continue;
        }
        let version = out
            .manifest
            .releases
            .iter()
            .find(|e| e.tag == tag)
            .map(|e| e.version.clone())
            .unwrap_or_default();

        let body = release_body(input, &replan, t, &name, &tag, &version, &out.manifest)?;
        forge.create_release(&ReleaseRequest {
            tag: tag.clone(),
            name: format!("{name} {version}"),
            body,
            prerelease: version.contains('-'),
            target: sha,
        })?;
        out.released.push((name, tag));
    }

    Ok(out)
}

/// #33 layer 3 — refuse a recorded version **lower** than the range justifies.
///
/// **The comparison is deliberately one-sided.** Only *lower than expected* is an
/// error. A recorded version *higher* than the recomputation is a deliberate
/// over-bump — a human hand-editing the versions file to cut a release by hand —
/// which is a supported workflow (J4, D4b). The same asymmetry enforces #76 for
/// free: editing the versions file *downwards* puts the recorded version below
/// the recomputed one and trips this guard without needing a separate rule (J4b).
/// #76 — refuse a recorded version that a reachable tag has already passed.
///
/// Reachability is what makes this safe on parallel lines: `release/1.4` records
/// `1.4.8` while `main` has long since tagged `v2.0.0`, and that tag is not
/// reachable from the maintenance branch, so it is not this line's business (G6,
/// G10).
fn monotonic_guard(
    package: &str,
    recorded: &Version,
    highest: &Option<(Version, String)>,
) -> Result<(), PhaseBError> {
    let Some((published, tag)) = highest else { return Ok(()) };
    if published > recorded {
        return Err(PhaseBError::WentBackwards {
            package: package.to_string(),
            recorded: recorded.to_string(),
            tag: tag.clone(),
            published: published.to_string(),
        });
    }
    Ok(())
}

fn staleness_guard(
    package: &str,
    recorded: &Version,
    replan: &Plan,
    target: Target,
) -> Result<(), PhaseBError> {
    let Some(expected) = replan.get(target).map(|r| &r.to) else { return Ok(()) };
    if recorded < expected {
        return Err(PhaseBError::StaleMerge {
            package: package.to_string(),
            recorded: recorded.to_string(),
            expected: expected.to_string(),
        });
    }
    Ok(())
}

/// The highest **stable** tag for this target that is reachable from HEAD and
/// below the recorded version (#90).
///
/// Stable-only is the point. A prerelease tag can match `tag_format` *and* sort
/// below the recorded version — `v1.5.0-rc.2` does both when the record says
/// `1.5.0` — so an unqualified "highest tag" would anchor the recomputation on an
/// rc and replan over a range that may be empty. The guard is one-sided, so it
/// would pass either way and the bug would never surface; stating the eligibility
/// rule is what stops it being decided by accident.
/// The highest stable tag for this target that is reachable from HEAD, whatever
/// its relation to the recorded version. [`previous_stable_tag`] deliberately
/// ignores anything at or above the record; this deliberately does not, because
/// "a tag above the record exists" is precisely the condition #76 forbids.
fn highest_reachable_stable(
    git: &impl GitBackend,
    cfg: &Config,
    versions: &VersionsFile,
    target: Target,
    all_tags: &[String],
) -> Result<Option<(Version, String)>, GitError> {
    let Some((_, tag_format, _)) = target_bits(cfg, versions, target) else { return Ok(None) };
    let mut best: Option<(Version, String)> = None;
    for tag in all_tags {
        let Some(v) = parse_tag(&tag_format, tag) else { continue };
        if !v.pre.is_empty() {
            continue;
        }
        if best.as_ref().is_some_and(|(b, _)| *b >= v) {
            continue;
        }
        if !git.is_ancestor(tag, "HEAD")? {
            continue;
        }
        best = Some((v, tag.clone()));
    }
    Ok(best)
}

fn previous_stable_tag(
    git: &impl GitBackend,
    cfg: &Config,
    versions: &VersionsFile,
    target: Target,
    all_tags: &[String],
) -> Result<Option<String>, GitError> {
    let Some((_, tag_format, entry)) = target_bits(cfg, versions, target) else { return Ok(None) };
    let recorded = entry.version.clone();

    let mut best: Option<(Version, String)> = None;
    for tag in all_tags {
        let Some(v) = parse_tag(&tag_format, tag) else { continue };
        if !v.pre.is_empty() || v >= recorded {
            continue;
        }
        if best.as_ref().is_some_and(|(b, _)| *b >= v) {
            continue;
        }
        if !git.is_ancestor(tag, "HEAD")? {
            continue;
        }
        best = Some((v, tag.clone()));
    }
    Ok(best.map(|(_, t)| t))
}

/// Name, tag format and versions-file entry for a target, or `None` when it has
/// never been released (#4).
fn target_bits(
    cfg: &Config,
    versions: &VersionsFile,
    target: Target,
) -> Option<(String, String, cicada_core::versions::Entry)> {
    match target {
        Target::Umbrella => {
            let u = cfg.umbrella.as_ref()?;
            let e = versions.umbrella.as_ref()?;
            Some((u.name.clone(), u.tag_format.clone(), e.clone()))
        }
        Target::Package(id) => {
            let (name, pkg) = cfg.packages.get_index(id.0)?;
            let e = versions.package(name)?;
            Some((name.clone(), pkg.tag_format.clone(), e.clone()))
        }
    }
}

/// The release body: the notes, then the manifest (#42, #43, #57).
///
/// The notes are rendered from the **same range and the same template** the
/// changelog entry used, so the two cannot disagree. The version comes from the
/// versions file rather than from the replan, because the recorded version is
/// what is being released — an over-bump (J4) is legitimate and the notes must
/// name the number on the tag.
fn release_body(
    input: &PhaseBInput<'_>,
    replan: &Plan,
    target: Target,
    name: &str,
    tag: &str,
    version: &str,
    manifest: &Manifest,
) -> Result<String, PhaseBError> {
    let tmpl = input
        .overrides
        .get("notes")
        .cloned()
        .unwrap_or_else(|| template::NOTES.to_string());

    let release = match replan.get(target) {
        Some(r) => Release { to: version.parse().unwrap_or_else(|_| r.to.clone()), tag: tag.to_string(), ..r.clone() },
        // Nothing in range — a re-tag of an already-released version, or a purely
        // hand-made release (J4). The notes are then empty rather than absent,
        // which is honest: there is nothing to say about a range with nothing in it.
        None => Release {
            target,
            name: name.to_string(),
            from: None,
            to: version.parse().unwrap_or_else(|_| Version::new(0, 0, 0)),
            bump: cicada_core::Bump::None,
            reason: cicada_core::plan::Reason::Commits,
            tag: tag.to_string(),
            commits: Vec::new(),
            derived_from: None,
        },
    };

    let ctx = cicada_core::notes::notes_context(input.config, &release, input.date, input.links);
    let notes = template::render("notes", &tmpl, &ctx)
        .map_err(|e| PhaseBError::Render(e.message))?;

    Ok(format!("{}\n\n{}\n", notes.trim(), manifest.to_details_block()))
}

/// What Phase B *would* reconcile, without creating anything.
///
/// `--dry-run` for Phase B cannot show a diff, because Phase B writes no files —
/// it creates tags and release objects. So it reports the two questions it asks
/// per package and the answer it currently has, which is the honest equivalent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub package: String,
    pub tag: String,
    pub state: TagState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagState {
    /// The tag exists and is reachable: nothing to do for artifact 1.
    Tagged,
    /// The tag is missing and would be created at HEAD.
    WouldTag,
    /// The tag exists elsewhere — a collision that would refuse (#9).
    Collides,
}

pub fn pending(
    git: &impl GitBackend,
    config: &Config,
    versions: &VersionsFile,
) -> Result<Vec<Pending>, GitError> {
    let mut targets: Vec<Target> =
        (0..config.packages.len()).map(|i| Target::Package(PackageId(i))).collect();
    if config.umbrella.is_some() {
        targets.push(Target::Umbrella);
    }

    let mut out = Vec::new();
    for t in targets {
        // No entry means never released: nothing to reconcile, and not an error
        // (#4, A7).
        let Some((name, tag_format, entry)) = target_bits(config, versions, t) else { continue };
        let version = entry.prerelease.clone().unwrap_or_else(|| entry.version.clone());
        let tag = render_tag(&tag_format, &version);
        let state = match git.rev_parse(&tag)? {
            None => TagState::WouldTag,
            Some(_) if git.is_ancestor(&tag, "HEAD")? => TagState::Tagged,
            Some(_) => TagState::Collides,
        };
        out.push(Pending { package: name, tag, state });
    }
    Ok(out)
}

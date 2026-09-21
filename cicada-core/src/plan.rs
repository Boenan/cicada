//! The `Plan` — what would be released, and why.
//!
//! Everything interesting in cicada is a function from inputs to a `Plan`, and
//! this module is that function. It is pure: the caller has already resolved each
//! target's anchor and read its range (that needs git, so it lives in the binary),
//! and hands the commits in as data.
//!
//! **Per-target ranges, per-target attribution.** Packages release at different
//! times, so each has its own anchor and therefore its own range (#2). The range
//! decides *what is read*; the scope decides *what is attributed* (P2). A worker
//! range that spans an api release commit is correct and must not be filtered —
//! the api commits in it simply route elsewhere.
//!
//! **Length 1 is not a mode** (#16, #38). A single-service repo is one entry in
//! the package list whose unscoped commits route to it through `default_scope`.
//! There is no single-package branch anywhere below this line.

use std::collections::BTreeMap;

use semver::Version;

use crate::bump::{commit_bump, first_release, highest, next_version};
use crate::commit::ParsedCommit;
use crate::config::{Bump, Config, PackageId, PreMajor, Target};
use crate::route::{UnknownScope, routes_to, targets_of};
use crate::versions::VersionsFile;

/// Render a `tag_format` for a version. The one place `{version}` is substituted,
/// shared with anchor resolution in the binary so the tag cicada *looks for* and
/// the tag it *creates* can never be spelled by two different pieces of code.
pub fn render_tag(format: &str, version: &Version) -> String {
    format.replace("{version}", &version.to_string())
}

/// Split a prerelease version into `(base, identifier, counter)`.
///
/// `1.5.0-rc.2` → `(1.5.0, "rc", 2)`. Identifiers are free strings with dotted
/// numbering (#64), so the split is at the **last** dot whose tail is numeric —
/// `1.5.0-alpha.beta.3` is `("alpha.beta", 3)`, which is the reading that keeps a
/// dotted identifier from silently becoming a counter.
///
/// A prerelease with no numeric tail (`1.5.0-rc`) reports counter `0`, so the
/// next one computed is `rc.1` and sorts above it.
pub fn split_prerelease(v: &Version) -> Option<(Version, String, u64)> {
    if v.pre.is_empty() {
        return None;
    }
    let base = Version::new(v.major, v.minor, v.patch);
    let pre = v.pre.as_str();
    match pre.rsplit_once('.') {
        Some((id, n)) if !id.is_empty() => match n.parse::<u64>() {
            Ok(n) => Some((base, id.to_string(), n)),
            Err(_) => Some((base, pre.to_string(), 0)),
        },
        _ => Some((base, pre.to_string(), 0)),
    }
}

/// Attach `-{identifier}.{counter}` to a base version.
fn with_prerelease(base: &Version, identifier: &str, counter: u64) -> Option<Version> {
    let mut v = base.clone();
    v.pre = semver::Prerelease::new(&format!("{identifier}.{counter}")).ok()?;
    Some(v)
}

/// Read a version back out of a tag, given the format that produced it.
///
/// The inverse of [`render_tag`], and the only way Phase B can ask "which
/// versions of this package have ever been tagged?" — the versions file records
/// one number, and the history of a line lives in its tags.
///
/// Deliberately strict: the tag must match the format exactly around
/// `{version}`, so `api-v1.5.0` is `api`'s and `api-chart-v1.5.0` is not. Two
/// packages whose formats could confuse each other are already refused by `check`
/// (#9), but a *prefix* relationship like `api-` and `api-chart-` is legal and
/// common, and it is this function that keeps them apart.
pub fn parse_tag(format: &str, tag: &str) -> Option<Version> {
    let (prefix, suffix) = format.split_once("{version}")?;
    let rest = tag.strip_prefix(prefix)?;
    let version = rest.strip_suffix(suffix)?;
    // A tag for `api-chart` reaches here when parsed against `api-v{version}`
    // only if the remainder still parses as semver, which `chart-v0.9.0` does
    // not. Relying on the parse rather than on a character class keeps the rule
    // in one place.
    version.parse().ok()
}

/// Why this release is happening. Reported in `--json` and in the human plan, so
/// a reader can tell an earned release from a consequence of someone else's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Earned by commits in its own range.
    Commits,
    /// No prior version — the first-release table or `initial_version` (#37).
    FirstRelease,
    /// The umbrella, taking the highest bump its children earned (#10).
    Aggregate,
    /// A chart dragged by the app it tracks, with no commits of its own (#22).
    Derived,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Commits => "commits",
            Reason::FirstRelease => "first-release",
            Reason::Aggregate => "aggregate",
            Reason::Derived => "derived",
        }
    }
}

/// The package and version that caused a derived release (#97).
///
/// Carried as **data**, never as prose: a derived release has no commits of its
/// own, so without this its changelog entry would render empty — and an empty
/// entry is honest but reads as a rendering bug. The default notes template names
/// the cause; the wording lives there and can be overridden (#46, #47).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedFrom {
    pub package: String,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub target: Target,
    pub name: String,
    /// `None` on a first release, and that is deliberate (#89): `0.0.0` names a
    /// release that never existed and produces compare URLs that 404.
    pub from: Option<Version>,
    pub to: Version,
    pub bump: Bump,
    pub reason: Reason,
    pub tag: String,
    /// This target's own commits, oldest first — the ones routed to it, not the
    /// whole range.
    pub commits: Vec<ParsedCommit>,
    pub derived_from: Option<DerivedFrom>,
}

/// An empty plan is a normal, successful outcome (#68) — "nothing to release" is
/// exit `0`, not a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Config order, umbrella last (#96) — which falls out of `Target`'s derived
    /// `Ord` rather than being sorted into place.
    pub releases: Vec<Release>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.releases.is_empty()
    }

    pub fn get(&self, target: Target) -> Option<&Release> {
        self.releases.iter().find(|r| r.target == target)
    }
}

/// One package whose computed bump is above patch on a patch-only line (#28).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offender {
    pub package: String,
    pub bump: Bump,
    /// The commits that computed above patch — named individually, because on a
    /// cherry-pick batch of twenty "a `feat:` is present" is not actionable (B11).
    pub commits: Vec<(String, String)>,
    /// Set when the offending bump was derived rather than earned (#22).
    pub derived_from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// A scope naming no package (#11). Fatal for the **whole run**, never a
    /// partial release of the packages that did parse (O5): shipping the half
    /// that parsed would cut a version whose changelog is missing the change that
    /// motivated it.
    UnknownScopes(Vec<UnknownScope>),
    /// The maintenance guard (#28). A **guard, not a clamp**: a computed bump
    /// above patch is refused, never silently reduced.
    MaintenanceGuard { branch: String, offenders: Vec<Offender> },

    /// #85 — a computed prerelease that does not sort **strictly above** the one
    /// already open.
    ///
    /// This is the **only** guarantee a maturity ladder gets (#93, #94). cicada
    /// draws no distinction between identifiers whatsoever: `alpha`, `beta` and
    /// `rc` anchor the same way, compute the same base, run the same counter and
    /// produce the same release object. The one thing that differs is the string
    /// in the version and the ordering it implies — so the rule is monotonicity
    /// and nothing else. It permits `alpha` → `beta` → `rc` and refuses
    /// `rc` → `beta`, without cicada knowing what any of those words mean.
    PrereleaseWentBackwards { package: String, open: Version, computed: Version },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::UnknownScopes(scopes) => {
                writeln!(f, "{} commit(s) name a scope that is not a package:", scopes.len())?;
                for s in scopes {
                    write!(f, "\n  {s}")?;
                    if let Some(sug) = &s.suggestion {
                        write!(f, "\n      did you mean '{sug}'?")?;
                    }
                }
                Ok(())
            }
            PlanError::PrereleaseWentBackwards { package, open, computed } => {
                write!(
                    f,
                    "'{package}' has {open} open, and this run computes {computed}, which sorts \
                     below it\n\
                     \n\
                     A prerelease line may only move forwards. Semver's ordering is the contract \
                     consumers\nresolve against, so cicada refuses the edits that violate it \
                     rather than redefining them —\nwhich is why there is no declared ladder of \
                     stages to appeal to (#94).\n\
                     \n\
                     Pick an identifier that sorts above `{}`, or close the open line by releasing \
                     it stable.",
                    open.pre.as_str()
                )
            }
            PlanError::MaintenanceGuard { branch, offenders } => {
                writeln!(
                    f,
                    "'{branch}' is a maintenance branch, and these commits compute above patch:"
                )?;
                for o in offenders {
                    write!(f, "\n  {} → {:?}", o.package, o.bump)?;
                    if let Some(d) = &o.derived_from {
                        write!(f, " (derived from '{d}')")?;
                    }
                    for (sha, subject) in &o.commits {
                        let sha: String = sha.chars().take(7).collect();
                        write!(f, "\n      {sha} {subject}")?;
                    }
                }
                // G2b: a blocked pipeline with no stated way forward is how an
                // escape hatch gets reintroduced. There is no flag, footer or
                // config key that releases this (#39) — so the message names the
                // two answers that are real.
                write!(
                    f,
                    "\n\nA maintenance line promises its consumers only patches, so cicada refuses \
                     rather than\nreleasing a feature as one. There is no override. The two answers \
                     are:\n  · put the change on a newer line, or\n  · re-do the cherry-pick as the \
                     fix it actually is."
                )
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// Everything the planner needs, all of it already gathered.
pub struct PlanInput<'a> {
    pub config: &'a Config,
    pub versions: &'a VersionsFile,
    /// Each target's own range, oldest first, parsed where parsing succeeded.
    /// A commit that is not a conventional commit is absent rather than an error —
    /// `check` is where a malformed subject becomes a finding (#12, #79).
    pub history: &'a BTreeMap<Target, Vec<ParsedCommit>>,
    /// Does the current branch match `maintenance_branches` (#28, #82)?
    pub maintenance: bool,
    /// The branch being planned, for the guard's error text.
    pub branch: String,
    /// The identifier this run's prereleases carry, when it cuts them (#59).
    ///
    /// **Branch-level, never per-package.** Per-package prereleases make the
    /// umbrella incoherent — semver aggregation has no meaning when some children
    /// are on rc and others are not — so every release in a run is decorated or
    /// none is.
    pub prerelease: Option<String>,
}

pub fn plan(input: &PlanInput<'_>) -> Result<Plan, PlanError> {
    let cfg = input.config;
    let targets = all_targets(cfg);

    // --- every releasable scope must name something (#11) -------------------
    //
    // Checked over the union of ranges rather than per package, because ranges
    // differ: a bad scope may sit in one package's range and not another's, and
    // O5 refuses the whole run either way.
    //
    // **Only commits that can release are checked.** The rule exists so a `feat`
    // is never silently misfiled; a `chore(deps):` cannot move a version number
    // whatever it is attributed to, and erroring on it would block every repo
    // whose dependency bot writes that scope — the design's own example of a
    // non-releasable commit (#36, B9). `check` still reports it, as a warning.
    let mut bad: Vec<UnknownScope> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for t in &targets {
        for c in commits_in_range(input, *t) {
            if seen.contains(&c.sha.as_str()) {
                continue;
            }
            seen.push(&c.sha);
            if commit_bump(cfg, c) == Bump::None {
                continue;
            }
            if let Err(e) = targets_of(cfg, c) {
                bad.push(e);
            }
        }
    }
    if !bad.is_empty() {
        return Err(PlanError::UnknownScopes(bad));
    }

    // --- what each target earned on its own merits --------------------------
    let n = cfg.packages.len();
    let mut own: Vec<Vec<ParsedCommit>> = Vec::with_capacity(n);
    let mut earned: Vec<Bump> = Vec::with_capacity(n);
    for i in 0..n {
        let t = Target::Package(PackageId(i));
        let mine: Vec<ParsedCommit> =
            commits_in_range(input, t).iter().filter(|c| routes_to(cfg, c, t)).cloned().collect();
        earned.push(highest(cfg, &mine));
        own.push(mine);
    }

    // --- charts dragged by the app they track (#22) -------------------------
    //
    // The only dependency propagation in cicada, and it is *declared* rather than
    // inferred. Run to a fixpoint so the answer does not depend on whether the
    // tracked package happens to sit earlier in config order; bumps only ever
    // increase and are bounded by major, so this terminates.
    let mut derived: Vec<Option<(PackageId, Bump)>> = vec![None; n];
    loop {
        let mut changed = false;
        for i in 0..n {
            if derived[i].is_some() {
                continue;
            }
            let Some((tracked, size)) = tracking(cfg, PackageId(i)) else { continue };
            let releases = earned[tracked.0] > Bump::None || derived[tracked.0].is_some();
            if releases {
                derived[i] = Some((tracked, size));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let final_bump = |i: usize| -> Bump {
        earned[i].max(derived[i].map(|(_, b)| b).unwrap_or(Bump::None))
    };

    // --- the umbrella aggregates the bump *kind*, never the number (#10) ----
    let umbrella_own: Vec<ParsedCommit> = if cfg.umbrella.is_some() {
        commits_in_range(input, Target::Umbrella)
            .iter()
            .filter(|c| routes_to(cfg, c, Target::Umbrella))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let umbrella_own_bump = highest(cfg, &umbrella_own);
    // Derived bumps are excluded (Q7): a chart's derived bump is a consequence of
    // a child bump already counted, and counting it twice would announce a minor
    // release of the whole platform on the strength of one `fix:`.
    let aggregate = (0..n).map(|i| earned[i]).max().unwrap_or(Bump::None);
    let umbrella_bump = umbrella_own_bump.max(aggregate);

    // --- versions ------------------------------------------------------------
    let mut releases: Vec<Release> = Vec::new();
    let mut package_to: Vec<Option<Version>> = vec![None; n];

    for i in 0..n {
        let bump = final_bump(i);
        if bump == Bump::None {
            continue;
        }
        let (name, pkg) = cfg.packages.get_index(i).expect("index within config");
        let entry = input.versions.package(name);
        let from = entry.and_then(|e| e.version.clone());
        let to = match &from {
            None => initial(pkg.initial_version.as_deref(), bump),
            Some(f) => next_version(f, bump, pkg.pre_major.unwrap_or(PreMajor::Minor)),
        };
        package_to[i] = Some(to.clone());
        let reason = if from.is_none() {
            Reason::FirstRelease
        } else if own[i].is_empty() {
            Reason::Derived
        } else {
            Reason::Commits
        };
        releases.push(Release {
            target: Target::Package(PackageId(i)),
            name: name.clone(),
            from,
            to: to.clone(),
            bump,
            reason,
            tag: render_tag(&pkg.tag_format, &to),
            commits: own[i].clone(),
            // Filled in below, once every package's `to` is known.
            derived_from: None,
        });
    }

    // `derived_from` needs the tracked package's computed version, so it is a
    // second pass rather than an ordering constraint on the first.
    for r in &mut releases {
        let Target::Package(PackageId(i)) = r.target else { continue };
        let Some((tracked, _)) = derived[i] else { continue };
        let Some(v) = package_to[tracked.0].clone() else { continue };
        r.derived_from = Some(DerivedFrom {
            package: cfg.package_name(tracked).unwrap_or_default().to_string(),
            version: v,
        });
    }

    if let Some(u) = &cfg.umbrella
        && umbrella_bump > Bump::None
    {
        let from = input.versions.umbrella.as_ref().and_then(|e| e.version.clone());
        let to = match &from {
            // The umbrella carries no `initial_version` and no `pre_major`: it is
            // a label on the set, so it takes the defaults rather than growing two
            // config keys that would only ever restate them.
            None => first_release(umbrella_bump),
            Some(f) => next_version(f, umbrella_bump, PreMajor::Minor),
        };
        let reason = if from.is_none() {
            Reason::FirstRelease
        } else if umbrella_own.is_empty() {
            Reason::Aggregate
        } else {
            Reason::Commits
        };
        releases.push(Release {
            target: Target::Umbrella,
            name: u.name.clone(),
            from,
            to: to.clone(),
            bump: umbrella_bump,
            reason,
            tag: render_tag(&u.tag_format, &to),
            commits: umbrella_own,
            derived_from: None,
        });
    }

    // Config order, umbrella last (#96). `Target`'s derived `Ord` is what says so;
    // sorting here makes that mechanical rather than a property of the loop order
    // above, which is the same argument as `PackageId` being an ordinal (#31).
    releases.sort_by_key(|r| r.target);

    // Decoration is a **separate pass over the finished stable plan**, and that
    // is the whole trick (#61): the anchor, the range and the computed base are
    // untouched, so every release — rc.1, rc.2 and the final stable — computes
    // from the same last-stable anchor and the notes are cumulative by
    // construction. Prereleases cost one extra field, not a second model.
    if let Some(identifier) = &input.prerelease {
        for r in &mut releases {
            decorate(cfg, input.versions, r, identifier)?;
        }
    }

    if input.maintenance {
        guard_maintenance(cfg, &releases, &input.branch)?;
    }

    Ok(Plan { releases })
}

/// Turn a computed stable release into the next prerelease on its line.
///
/// **The counter resets on a change to the base *or* the identifier** (#63), and
/// both are recomputed from this run's inputs with nothing stored. The identifier
/// axis is the one that is easy to miss: with only the base considered, an
/// `alpha.3` line moving to `beta` computes an unchanged base, increments, and
/// publishes `1.5.0-beta.4` — a counter continuing across a stage it never
/// belonged to. It even passes #85, because `beta.4` sorts above `alpha.3`, so
/// nothing catches it (F10d).
fn decorate(
    cfg: &Config,
    versions: &VersionsFile,
    r: &mut Release,
    identifier: &str,
) -> Result<(), PlanError> {
    let open = match r.target {
        Target::Umbrella => versions.umbrella.as_ref().and_then(|e| e.prerelease.clone()),
        Target::Package(id) => cfg
            .package_name(id)
            .and_then(|n| versions.package(n))
            .and_then(|e| e.prerelease.clone()),
    };

    let counter = match open.as_ref().and_then(split_prerelease) {
        Some((base, id, n)) if base == r.to && id == identifier => n + 1,
        _ => 1,
    };

    let computed = with_prerelease(&r.to, identifier, counter).ok_or_else(|| {
        PlanError::PrereleaseWentBackwards {
            package: r.name.clone(),
            open: r.to.clone(),
            computed: r.to.clone(),
        }
    })?;

    // #85 — strictly greater than the open one, or refuse.
    if let Some(open) = &open
        && computed <= *open
    {
        return Err(PlanError::PrereleaseWentBackwards {
            package: r.name.clone(),
            open: open.clone(),
            computed,
        });
    }

    r.tag = render_tag(tag_format_for(cfg, r.target), &computed);
    r.to = computed;
    Ok(())
}

/// The `tag_format` a target declares.
pub fn tag_format_for(cfg: &Config, target: Target) -> &str {
    match target {
        Target::Umbrella => {
            cfg.umbrella.as_ref().map(|u| u.tag_format.as_str()).unwrap_or("{version}")
        }
        Target::Package(id) => cfg
            .packages
            .get_index(id.0)
            .map(|(_, p)| p.tag_format.as_str())
            .unwrap_or("{version}"),
    }
}

/// Every declared target, in release order (#96).
fn all_targets(cfg: &Config) -> Vec<Target> {
    let mut v: Vec<Target> = (0..cfg.packages.len()).map(|i| Target::Package(PackageId(i))).collect();
    if cfg.umbrella.is_some() {
        v.push(Target::Umbrella);
    }
    v
}

fn commits_in_range<'a>(input: &'a PlanInput<'_>, t: Target) -> &'a [ParsedCommit] {
    input.history.get(&t).map(Vec::as_slice).unwrap_or(&[])
}

/// The chart-tracking relationship a package declares, if any (#22).
fn tracking(cfg: &Config, id: PackageId) -> Option<(PackageId, Bump)> {
    use crate::config::{TrackedBump, UpdaterKind};
    let (_, pkg) = cfg.packages.get_index(id.0)?;
    for u in &pkg.updaters {
        if u.kind != UpdaterKind::Helm {
            continue;
        }
        let Some(av) = &u.app_version else { continue };
        let tracked = cfg.package_id(&av.track)?;
        // A chart tracking itself is not a relationship; ignoring it keeps the
        // fixpoint above from depending on its own output.
        if tracked == id {
            continue;
        }
        let size = match u.on_tracked_bump.unwrap_or(TrackedBump::Patch) {
            TrackedBump::Patch => Bump::Patch,
            TrackedBump::Minor => Bump::Minor,
        };
        return Some((tracked, size));
    }
    None
}

/// `initial_version` overrides the first-release table outright (A4). A value that
/// is not semver cannot reach here — `check` rejects it before any command plans.
fn initial(declared: Option<&str>, bump: Bump) -> Version {
    declared.and_then(|s| s.parse().ok()).unwrap_or_else(|| first_release(bump))
}

/// The maintenance guard (#28) — exit `4`, never a reduced version.
///
/// It reports the package that *caused* the bump, not every package carrying one.
/// The umbrella's minor is a consequence of a child's (Q9), and naming it as the
/// cause would send the reader to the wrong place.
fn guard_maintenance(cfg: &Config, releases: &[Release], branch: &str) -> Result<(), PlanError> {
    let mut offenders: Vec<Offender> = Vec::new();
    for r in releases {
        if r.bump <= Bump::Patch {
            continue;
        }
        let culprits: Vec<(String, String)> = r
            .commits
            .iter()
            .filter(|c| commit_bump(cfg, c) > Bump::Patch)
            .map(|c| (c.sha.clone(), format!("{}: {}", c.kind, c.description)))
            .collect();
        if !culprits.is_empty() {
            offenders.push(Offender {
                package: r.name.clone(),
                bump: r.bump,
                commits: culprits,
                derived_from: None,
            });
        } else if let Some(d) = &r.derived_from {
            // A chart whose `on_tracked_bump = "minor"` puts it above patch. No
            // commit caused it, so the config and the tracked package are named
            // instead of an empty list.
            offenders.push(Offender {
                package: r.name.clone(),
                bump: r.bump,
                commits: Vec::new(),
                derived_from: Some(d.package.clone()),
            });
        }
        // Otherwise: a pure aggregate. Its cause is already in the list.
    }
    if offenders.is_empty() {
        return Ok(());
    }
    Err(PlanError::MaintenanceGuard { branch: branch.to_string(), offenders })
}

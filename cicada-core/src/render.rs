//! `Plan` → `Vec<FileEdit>` — the whole write side of a release, as data (#30).
//!
//! Nothing here touches the world. The binary reads every file this needs
//! beforehand ([`required_sources`]) and writes back what comes out, which is what
//! makes `--dry-run` *the identical computation with the write skipped* rather
//! than a parallel branch describing what would happen.
//!
//! ## Two updaters may share a file, and that is not a collision
//!
//! `FileEdit` carries **final content**, so two edits for one path would be
//! last-writer-wins — the silent, config-order-dependent bug #44 refuses. But the
//! legitimate case is common: one `values.yaml` holding `image.tag` and
//! `labels.version`, or two services stamping their own tag into one deploy file.
//!
//! So edits **fold**. Each updater transforms the content the run has for that
//! path so far, not what is on disk, and one `FileEdit` per path comes out at the
//! end. Order is config order (#31), so the fold is deterministic.
//!
//! What genuinely cannot fold is a *whole-file* writer: a `template` updater
//! renders the entire output and would obliterate any surgical edit under it.
//! That is the collision #44 means, and `check` refuses it (§Loading and
//! validation) before any file is opened.

use std::collections::BTreeMap;

use indexmap::IndexMap;

use crate::config::{Config, PackageId, Target, UpdaterKind};
use crate::edit::{EditError, EditSource, FileEdit};
use crate::notes::{LinkStyle, ReleaseSummary, UmbrellaContext, file_context, notes_context};
use crate::plan::{Plan, Release};
use crate::template;
use crate::updater;
use crate::versions::{Entry, VersionsFile};

pub struct RenderInput<'a> {
    pub config: &'a Config,
    pub plan: &'a Plan,
    /// The versions file as it stands. The new one is built from it, so packages
    /// that are not releasing keep their entries untouched.
    pub versions: &'a VersionsFile,
    /// The date, as a value (#95). Core never reads a clock.
    pub date: &'a str,
    /// Supplied by the forge adapter as a value, never as a dependency (A2).
    pub links: &'a LinkStyle,
    /// Repo-relative path → content. An absent key means the file does not exist,
    /// which is an error for an updater and the *create* path for a changelog (#5).
    pub sources: &'a BTreeMap<String, String>,
    /// For a cargo updater whose manifest inherits, the workspace root manifest
    /// that actually holds the number (#24). Finding it means walking the tree, so
    /// the binary resolves it and passes the answer in.
    pub workspace_roots: &'a BTreeMap<String, String>,
    /// Template overrides picked up from `.cicada/templates/<name>.j2` (#47),
    /// keyed by built-in name.
    pub overrides: &'a BTreeMap<String, String>,
}

pub struct Rendered {
    /// Every file that changes, in the order its path was first touched —
    /// deterministic, because that order is config order.
    pub edits: Vec<FileEdit>,
    /// Non-fatal, reported to the human. Today: an unquoted `appVersion` (#51).
    pub warnings: Vec<String>,
    /// The rendered notes body per release. **One template, two destinations**
    /// (#43) — the forge release reuses this exact string rather than rendering
    /// its own, so the two can never drift.
    pub notes: BTreeMap<Target, String>,
}

/// The changelog path a target writes to: declared, or `<path>/CHANGELOG.md`.
pub fn changelog_path(config: &Config, target: Target) -> Option<String> {
    match target {
        Target::Umbrella => config.umbrella.as_ref().map(|u| u.changelog.clone()),
        Target::Package(id) => config.packages.get_index(id.0).map(|(_, p)| {
            p.changelog.clone().unwrap_or_else(|| join(&p.path, "CHANGELOG.md"))
        }),
    }
}

fn join(dir: &str, name: &str) -> String {
    match dir.trim_end_matches('/') {
        "" | "." => name.to_string(),
        d => format!("{d}/{name}"),
    }
}

/// Every repo-relative path this render will read, so the binary can slurp exactly
/// those and no more.
///
/// A path appearing here does not mean the file must exist — a changelog is
/// created when absent (#5). It means "read it if it is there".
pub fn required_sources(config: &Config, plan: &Plan) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for r in &plan.releases {
        if let Some(c) = changelog_path(config, r.target) {
            push(c);
        }
        let Target::Package(id) = r.target else { continue };
        let Some((_, pkg)) = config.packages.get_index(id.0) else { continue };
        for u in &pkg.updaters {
            match u.kind {
                UpdaterKind::Template => {
                    if let Some(t) = &u.template {
                        push(t.clone());
                    }
                    if let Some(o) = &u.output {
                        push(o.clone());
                    }
                }
                _ => {
                    if let Some(p) = &u.path {
                        push(p.clone());
                    }
                }
            }
        }
    }
    out
}

/// The versions-file path, fixed beside `cicada.toml` (#1).
pub const VERSIONS_FILE: &str = ".cicada-versions.toml";

pub fn render(input: &RenderInput<'_>) -> Result<Rendered, EditError> {
    let cfg = input.config;
    let mut pending: IndexMap<String, (String, EditSource)> = IndexMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut notes: BTreeMap<Target, String> = BTreeMap::new();

    let tmpl = |name: &str, built_in: &'static str| -> String {
        input.overrides.get(name).cloned().unwrap_or_else(|| built_in.to_string())
    };
    let notes_tmpl = tmpl("notes", template::NOTES);

    // --- version files, via each package's updaters -------------------------
    for r in &plan_packages(input.plan) {
        let Target::Package(id) = r.target else { continue };
        let Some((_, pkg)) = cfg.packages.get_index(id.0) else { continue };

        for u in &pkg.updaters {
            if u.kind == UpdaterKind::Template {
                let (template_path, output) = match (&u.template, &u.output) {
                    (Some(t), Some(o)) => (t, o),
                    // `check` requires both; reaching here means it was skipped.
                    _ => {
                        return Err(EditError::Invalid {
                            path: r.name.clone(),
                            message: "a template updater needs both `template` and `output`".into(),
                        });
                    }
                };
                let src = input.sources.get(template_path).ok_or_else(|| EditError::NotFound {
                    path: template_path.clone(),
                    address: "the template file".into(),
                })?;
                let ctx = file_context(r, input.date, input.links);
                let content = template::render(template_path, src, &ctx).map_err(|e| {
                    EditError::Invalid { path: template_path.clone(), message: e.message }
                })?;
                // A whole-file writer, so it replaces rather than folds — which is
                // exactly why `check` refuses it sharing a path with anything else.
                pending.insert(
                    output.clone(),
                    (content, EditSource::Updater { package: r.name.clone(), kind: "template" }),
                );
                continue;
            }

            let declared = u.path.as_ref().ok_or_else(|| EditError::Invalid {
                path: r.name.clone(),
                message: format!("a {:?} updater needs a `path`", u.kind).to_lowercase(),
            })?;

            // Where the edit actually lands. For a cargo crate that inherits, the
            // number lives in the workspace root's `[workspace.package]` (#24).
            let target_path = resolve_path(input, declared);
            let redirected = target_path != *declared;

            // Fold: read what the run has so far, not what is on disk.
            let current = match pending.get(&target_path) {
                Some((c, _)) => c.clone(),
                None => input
                    .sources
                    .get(&target_path)
                    .cloned()
                    .ok_or_else(|| EditError::NotFound {
                        path: target_path.clone(),
                        address: "the file itself".into(),
                    })?,
            };

            let tracked = tracked_version(input.plan, cfg, u);
            let out =
                updater::apply(u, &target_path, &current, &r.to, tracked.as_ref(), redirected)?;
            warnings.extend(out.warnings);
            pending.insert(
                target_path,
                (out.content, EditSource::Updater { package: r.name.clone(), kind: kind_str(u.kind) }),
            );
        }
    }

    // --- notes, once per release, reused by both destinations (#43) ---------
    for r in &input.plan.releases {
        let ctx = notes_context(cfg, r, input.date, input.links);
        let body = template::render("notes", &notes_tmpl, &ctx)
            .map_err(|e| EditError::Invalid { path: "notes".into(), message: e.message })?;
        // Trimmed at both ends: the *frame* owns the spacing around an entry, so
        // a template whose whitespace control leaves a leading newline cannot
        // silently widen every changelog by a blank line.
        notes.insert(r.target, body.trim().to_string());
    }

    // --- changelogs ----------------------------------------------------------
    let header = tmpl("changelog-header", template::CHANGELOG_HEADER);
    for r in &input.plan.releases {
        // **Prereleases never touch `CHANGELOG.md`** (#62). If every release wrote
        // a cumulative entry the file would accumulate rc.1, then rc.2 repeating
        // it, then the stable repeating it again. Instead the changelog holds only
        // stable releases — clean and linear — while each rc's *release notes* are
        // cumulative since the last stable, which is exactly what a tester wants.
        // Version files are still written, because the rc artifact has to build.
        if !r.to.pre.is_empty() {
            continue;
        }
        let Some(path) = changelog_path(cfg, r.target) else { continue };

        let entry = if r.target == Target::Umbrella {
            // The umbrella's entry groups every release in the run by service
            // (#15) — one entry per run, not one per package.
            let ctx = umbrella_context(input, r, &notes);
            template::render("umbrella", &tmpl("umbrella", template::UMBRELLA), &ctx)
                .map_err(|e| EditError::Invalid { path: path.clone(), message: e.message })?
        } else {
            let ctx = ChangelogFrame {
                package: r.name.clone(),
                version: r.to.to_string(),
                previous_version: r.from.as_ref().map(ToString::to_string),
                tag: r.tag.clone(),
                date: input.date.to_string(),
                notes: notes.get(&r.target).cloned().unwrap_or_default(),
            };
            template::render("changelog", &tmpl("changelog", template::CHANGELOG), &ctx)
                .map_err(|e| EditError::Invalid { path: path.clone(), message: e.message })?
        };

        let existing = pending
            .get(&path)
            .map(|(c, _)| c.clone())
            .or_else(|| input.sources.get(&path).cloned());
        let content = crate::changelog::insert(existing.as_deref(), &entry, &header);
        pending.insert(path, (content, EditSource::Changelog { package: r.name.clone() }));
    }

    // --- the versions file ---------------------------------------------------
    pending.insert(
        VERSIONS_FILE.to_string(),
        (next_versions(input).to_toml(), EditSource::VersionsFile),
    );

    // A file whose content did not change is not an edit. Keeping it would make
    // `--dry-run` list files with an empty diff, and "nothing was written" a
    // weaker assertion than it should be.
    let edits = pending
        .into_iter()
        .filter(|(path, (content, _))| input.sources.get(path) != Some(content))
        .map(|(path, (content, source))| FileEdit { path, content, source })
        .collect();

    Ok(Rendered { edits, warnings, notes })
}

fn kind_str(k: UpdaterKind) -> &'static str {
    match k {
        UpdaterKind::Cargo => "cargo",
        UpdaterKind::Helm => "helm",
        UpdaterKind::Generic => "generic",
        UpdaterKind::Template => "template",
    }
}

fn plan_packages(plan: &Plan) -> Vec<Release> {
    plan.releases.iter().filter(|r| r.target != Target::Umbrella).cloned().collect()
}

fn resolve_path(input: &RenderInput<'_>, declared: &str) -> String {
    // Only consulted when the manifest actually inherits; the binary puts an entry
    // here only in that case, so a plain lookup is the whole rule.
    input.workspace_roots.get(declared).cloned().unwrap_or_else(|| declared.to_string())
}

/// The version a helm updater's `appVersion` should mirror — the tracked package's
/// *computed* version, when that package is releasing in this run (#22).
fn tracked_version(
    plan: &Plan,
    cfg: &Config,
    u: &crate::config::Updater,
) -> Option<semver::Version> {
    let av = u.app_version.as_ref()?;
    let id = cfg.package_id(&av.track)?;
    plan.get(Target::Package(id)).map(|r| r.to.clone())
}

#[derive(serde::Serialize)]
struct ChangelogFrame {
    package: String,
    version: String,
    previous_version: Option<String>,
    tag: String,
    date: String,
    notes: String,
}

fn umbrella_context(
    input: &RenderInput<'_>,
    r: &Release,
    notes: &BTreeMap<Target, String>,
) -> UmbrellaContext {
    UmbrellaContext {
        package: r.name.clone(),
        version: r.to.to_string(),
        previous_version: r.from.as_ref().map(ToString::to_string),
        tag: r.tag.clone(),
        date: input.date.to_string(),
        bump: r.bump.as_str().to_string(),
        releases: input
            .plan
            .releases
            .iter()
            .filter(|c| c.target != Target::Umbrella)
            .map(|c| ReleaseSummary {
                package: c.name.clone(),
                version: c.to.to_string(),
                previous_version: c.from.as_ref().map(ToString::to_string),
                tag: c.tag.clone(),
                bump: c.bump.as_str().to_string(),
                notes: notes.get(&c.target).cloned().unwrap_or_default(),
            })
            .collect(),
    }
}

/// The versions file after this run.
///
/// Built fresh in **config order** (#31) rather than by mutating the parsed map,
/// so the file's key order is the repo's declared order however it was written by
/// hand. Entries for packages that are not releasing are carried over unchanged —
/// this file is the sole source of truth (#75) and must not lose a line.
fn next_versions(input: &RenderInput<'_>) -> VersionsFile {
    let cfg = input.config;
    let mut out = VersionsFile::default();

    for i in 0..cfg.packages.len() {
        let Some((name, _)) = cfg.packages.get_index(i) else { continue };
        let released = input.plan.get(Target::Package(PackageId(i)));
        let existing = input.versions.package(name);
        match (released, existing) {
            (Some(r), existing) => {
                out.packages.insert(name.clone(), next_entry(r, existing));
            }
            (None, Some(e)) => {
                out.packages.insert(name.clone(), e.clone());
            }
            // Never released and not releasing now: an absent entry, not a
            // sentinel (#4).
            (None, None) => {}
        }
    }

    out.umbrella = match (input.plan.get(Target::Umbrella), &input.versions.umbrella) {
        (Some(r), existing) => Some(next_entry(r, existing.as_ref())),
        (None, Some(e)) => Some(e.clone()),
        (None, None) => None,
    };

    out
}

/// The versions-file entry a release produces.
///
/// A **stable** release moves `version` and **deletes** `prerelease`, which is
/// what closes an open line (#60, F5). A **prerelease** does the opposite: it
/// records itself under `prerelease` and leaves `version` exactly where it was,
/// because `version` means *last stable release* and no stable release happened
/// (F1). That split is the entire mechanism — it is why anchor semantics are
/// untouched and why the stable changelog comes out complete.
fn next_entry(r: &Release, existing: Option<&Entry>) -> Entry {
    if r.to.pre.is_empty() {
        Entry::stable(r.to.clone())
    } else {
        Entry {
            version: existing.and_then(|e| e.version.clone()),
            prerelease: Some(r.to.clone()),
        }
    }
}

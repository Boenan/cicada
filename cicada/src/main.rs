//! cicada — release automation driven by conventional commits.
//!
//! Git is the substrate and the forge API is a thin edge; everything interesting
//! is a pure function in `cicada-core`, and this crate is the part that touches
//! the world (design §Architecture).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cicada_core::check::{Finding, Severity, lint_commits, validate_config, validate_release_pr_branch};
use cicada_core::plan::{PlanError, PlanInput};
use cicada::git::{GitBackend, GitCli};
use cicada::report::Style;
use cicada::{config, history, prepare, report, versions};

/// Exit codes are a contract, not an implementation detail (#68). CI branches on
/// them, so `3` (your commits are malformed) and `4` (cicada declined to act) are
/// separated from `1` (cicada broke).
mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    // 2 is clap's, for usage errors.
    pub const FINDINGS: u8 = 3;
    pub const REFUSED: u8 = 4;
}

#[derive(Parser)]
#[command(
    name = "cicada",
    version,
    about = "Release automation: conventional commits in, versions and releases out.",
    disable_help_subcommand = true
)]
struct Cli {
    /// Path to cicada.toml. Defaults to the repository root.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// The date changelog and release-notes entries carry. Defaults to today in
    /// UTC. An input rather than a clock read (#95), so a rehearsal and the run
    /// that follows it produce identical bytes rather than identical-looking ones.
    #[arg(long, global = true, value_name = "YYYY-MM-DD")]
    date: Option<jiff::civil::Date>,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Verbose logging.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Suppress non-essential output.
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum TemplateCmd {
    /// Write a built-in template to stdout.
    Print {
        /// One of `notes`, `changelog`, `umbrella`, `changelog-header`.
        name: String,
    },
}

#[derive(Subcommand)]
enum Command {
    /// Validate the config and lint a range of commits.
    ///
    /// Takes a git range, not a forge PR object — no API call, no token,
    /// identical on GitHub, GitLab and a laptop.
    Check {
        /// Where the range starts, exclusive. Omit to validate config only.
        #[arg(long, value_name = "REF")]
        base: Option<String>,
        /// Where the range ends. Defaults to HEAD.
        #[arg(long, value_name = "REF", default_value = "HEAD")]
        head: String,
    },

    /// Phase A — compute the release, write the files, open the PR.
    ///
    /// In this build only `--dry-run` is implemented: it prints exactly the diff
    /// it would write (#30). The git write path lands in M3 and the forge in M4.
    ReleasePr {
        /// Compute and print the diff; write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Write and commit locally, but push nothing. Lands in M3.
        #[arg(long)]
        no_push: bool,
    },

    /// Emit a built-in template, so customising starts from the real thing (#48).
    Template {
        #[command(subcommand)]
        what: TemplateCmd,
    },

    /// What would be released, and why. Touches nothing.
    ///
    /// Fully offline: no token, no network, no forge (#25, §Goals 6). Unlike
    /// `run`, it is **not** gated on `release_branches` — it answers a question
    /// rather than performing a release, and refusing to answer it on a feature
    /// branch would make the rehearsal useless exactly where it is wanted. When
    /// the branch is outside the gate it says so.
    Plan,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let code = match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            // `{:#}` renders anyhow's context chain on one line, so an error keeps
            // the "what cicada was doing" frame rather than only the leaf cause.
            eprintln!("error: {e:#}");
            exit::ERROR
        }
    };
    ExitCode::from(code)
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
}

fn run(cli: &Cli) -> anyhow::Result<u8> {
    match &cli.command {
        Command::Check { base, head } => cmd_check(cli, base.as_deref(), head),
        Command::Plan => cmd_plan(cli),
        Command::ReleasePr { dry_run, no_push } => cmd_release_pr(cli, *dry_run, *no_push),
        Command::Template { what } => match what {
            TemplateCmd::Print { name } => cmd_template_print(name),
        },
    }
}

/// `cicada template print <name>` (#48).
///
/// Deliberately the opposite of release-please, whose changelog format is hard to
/// customise *because* you cannot see the template you are trying to deviate from.
fn cmd_template_print(name: &str) -> anyhow::Result<u8> {
    match cicada_core::template::built_in(name) {
        Some(src) => {
            print!("{src}");
            Ok(exit::OK)
        }
        None => anyhow::bail!(
            "no built-in template named `{name}`
available: {}",
            cicada_core::template::built_in_names().join(", ")
        ),
    }
}

/// Today in **UTC** (#95).
///
/// UTC rather than a configurable zone: a zone is one more input that changes the
/// answer, and a changelog entry dated by the release runner's locale is a worse
/// artifact than one dated consistently. This is the only clock cicada reads, and
/// it is read here — in the binary — so `cicada-core` stays pure.
fn today_utc() -> jiff::civil::Date {
    jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date()
}

fn cmd_check(cli: &Cli, base: Option<&str>, head: &str) -> anyhow::Result<u8> {
    let cwd = std::env::current_dir()?;
    let repo = GitCli::discover(&cwd)?;

    // A truncated history is detected up front, before any range is computed, so
    // the failure is never a silently incomplete changelog (H8).
    repo.assert_not_shallow()?;

    let loaded = config::load(repo.root(), cli.config.as_deref())?;
    tracing::debug!(config = %loaded.path.display(), "loaded config");
    let cfg = &loaded.config;

    // Pure validations, then the ones that need the tree. One list either way.
    let mut findings = validate_config(cfg);
    findings.extend(config::validate_on_disk(cfg, repo.root()));
    if let Some(branch) = repo.current_branch()?
        && let Some(f) = validate_release_pr_branch(cfg, &branch)
    {
        findings.push(f);
    }

    let commits = match base {
        Some(b) => repo.log(Some(b), head, cfg.settings.commit_source)?,
        None => Vec::new(),
    };
    let reports = lint_commits(cfg, &commits);

    let style = if cli.json { Style::plain() } else { Style::detect() };
    let config_problems = findings.iter().filter(|f| f.severity == Severity::Problem).count();
    let commit_problems: usize =
        reports.iter().map(|r| r.findings.iter().filter(|f| f.severity == Severity::Problem).count()).sum();

    if cli.json {
        print!("{}", json_report(&findings, &reports));
    } else if !cli.quiet {
        if !findings.is_empty() {
            print!("{}", report::render_config_findings(&style, &findings));
        }
        if !reports.is_empty() {
            print!("{}", report::render_commits(&style, &reports));
        }
        print!("{}", report::summarise(&style, config_problems + commit_problems, reports.len()));
    }

    // Semantic findings are `3`; a config that would not parse never reached here
    // and was already `1` (#79).
    Ok(if config_problems + commit_problems > 0 { exit::FINDINGS } else { exit::OK })
}

/// Hand-rolled rather than via serde: the schema is a contract (#69) and writing
/// it out makes that visible. `schema` is the version; fields may be added within
/// it, removals or meaning changes bump it.
fn json_report(
    findings: &[cicada_core::check::Finding],
    reports: &[cicada_core::check::CommitReport],
) -> String {
    let esc = report::json_escape;
    let finding_json = |f: &cicada_core::check::Finding| {
        let sev = match f.severity {
            Severity::Problem => "problem",
            Severity::Warning => "warning",
        };
        let help = match &f.help {
            Some(h) => format!("\"{}\"", esc(h)),
            None => "null".into(),
        };
        format!(
            r#"{{"severity":"{sev}","subject":"{}","message":"{}","help":{help}}}"#,
            esc(&f.subject),
            esc(&f.message)
        )
    };

    let config: Vec<String> = findings.iter().map(finding_json).collect();
    let commits: Vec<String> = reports
        .iter()
        .map(|r| {
            let fs: Vec<String> = r.findings.iter().map(finding_json).collect();
            format!(
                r#"{{"sha":"{}","subject":"{}","ok":{},"findings":[{}]}}"#,
                esc(&r.sha),
                esc(&r.subject),
                r.ok(),
                fs.join(",")
            )
        })
        .collect();

    format!(
        "{{\"schema\":1,\"config\":[{}],\"commits\":[{}]}}\n",
        config.join(","),
        commits.join(",")
    )
}

/// Every config finding — the pure half and the filesystem half — in one list.
///
/// `check` renders these and exits `3`; every other command treats a `Problem`
/// as fatal and exits `1`, because it cannot proceed on a config it has already
/// found incoherent (I4–I6c).
fn gather_config_findings(
    repo: &GitCli,
    cfg: &cicada_core::Config,
    branch: Option<&str>,
) -> Vec<Finding> {
    let mut findings = validate_config(cfg);
    findings.extend(config::validate_on_disk(cfg, repo.root()));
    if let Some(b) = branch
        && let Some(f) = validate_release_pr_branch(cfg, b)
    {
        findings.push(f);
    }
    findings
}

/// Everything `plan` and `release-pr` both do before they diverge: discover the
/// repo, load and validate the config, classify the branch, resolve anchors, and
/// compute the plan.
///
/// Kept in one place because the *order* matters and is easy to get subtly wrong:
/// shallow detection precedes any range (H8), config validation precedes any
/// git work, and the prerelease refusal precedes planning rather than following it.
struct Prepared {
    repo: GitCli,
    loaded: config::Loaded,
    branch: String,
    on_release_branch: bool,
    versions: versions::Loaded,
    plan: cicada_core::plan::Plan,
}

enum Prep {
    Ready(Box<Prepared>),
    /// An exit code that is the whole answer — a refusal, or the branch gate.
    Exit(u8),
}

fn prepare_run(cli: &Cli) -> anyhow::Result<Prep> {
    let cwd = std::env::current_dir()?;
    let repo = GitCli::discover(&cwd)?;

    // Up front, before any range is computed, so a truncated history is never
    // mistaken for a short one (H8).
    repo.assert_not_shallow()?;

    let loaded = config::load(repo.root(), cli.config.as_deref())?;
    let cfg = &loaded.config;
    let branch = repo.current_branch()?;

    let findings = gather_config_findings(&repo, cfg, branch.as_deref());
    if findings.iter().any(|f| f.severity == Severity::Problem) {
        eprint!("{}", report::render_config_findings(&Style::detect(), &findings));
        anyhow::bail!("cicada.toml is not usable — `cicada check` lists every problem");
    }

    let branch_name = branch.clone().unwrap_or_else(|| "HEAD".to_string());
    let on_release_branch =
        cicada_core::branch::matches_any(&cfg.settings.release_branches, &branch_name)?;
    let maintenance =
        cicada_core::branch::matches_any(&cfg.settings.maintenance_branches, &branch_name)?;

    // The one deliberate gap left from M1. A prerelease branch would compute a
    // *stable* version here, which is not a missing feature but a wrong answer —
    // and a plan that lies offline defeats the point of it running offline
    // (§Goals 6). Refusing is loud; computing would be quiet. Deleting this block
    // is what lands prerelease support.
    if cicada_core::branch::matches_any(&cfg.settings.prerelease_branches, &branch_name)? {
        anyhow::bail!(
            "'{branch_name}' matches `prerelease_branches`, and prerelease planning is not \
             implemented yet\ncicada would otherwise report a stable version here, which would be \
             wrong rather than incomplete"
        );
    }

    let versions = versions::load(&loaded.path)?;
    tracing::debug!(
        versions = %versions.path.display(),
        exists = versions.exists,
        "loaded versions file"
    );

    let collected = history::collect(&repo, cfg, &versions.versions, "HEAD")?;
    for (target, anchor) in &collected.anchors {
        tracing::debug!(?target, anchor = ?anchor, "resolved anchor");
    }

    let input = PlanInput {
        config: cfg,
        versions: &versions.versions,
        history: &collected.history,
        maintenance,
        branch: branch_name.clone(),
    };
    let plan = match cicada_core::plan::plan(&input) {
        Ok(p) => p,
        // "cicada declined to act" is exit `4`, distinct from "cicada broke" (#68)
        // — a pipeline may well want to handle them differently.
        Err(e @ PlanError::MaintenanceGuard { .. }) => {
            eprintln!("error: {e}");
            return Ok(Prep::Exit(exit::REFUSED));
        }
        // An unknown scope is fatal for the whole run (#11, O5): releasing the
        // half that parsed would cut a version whose changelog is missing the
        // change that motivated it.
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(Prep::Exit(exit::ERROR));
        }
    };

    Ok(Prep::Ready(Box::new(Prepared {
        repo,
        loaded,
        branch: branch_name,
        on_release_branch,
        versions,
        plan,
    })))
}

fn cmd_plan(cli: &Cli) -> anyhow::Result<u8> {
    let p = match prepare_run(cli)? {
        Prep::Ready(p) => p,
        Prep::Exit(code) => return Ok(code),
    };
    let date = cli.date.unwrap_or_else(today_utc);

    if cli.json {
        print!("{}", plan_json(&date, &p.branch, p.on_release_branch, &p.plan));
    } else if !cli.quiet {
        let style = Style::detect();
        print!("{}", report::render_plan(&style, &p.plan));
        if !p.on_release_branch {
            // Not an error: `plan` is a question, and the answer is the same
            // wherever it is asked. But `run` here would exit 0 without acting
            // (#26, H1), and discovering that from a silent pipeline is worse
            // than reading it now.
            eprintln!(
                "\nnote: '{}' matches no `release_branches` pattern — `cicada run` would exit 0 \
                 here without releasing",
                p.branch
            );
        }
    }

    // "Nothing to release" is a normal outcome, not a failure (#68).
    Ok(exit::OK)
}

/// Phase A, as far as this build goes: compute the plan, render it to
/// `Vec<FileEdit>`, and print the diff.
fn cmd_release_pr(cli: &Cli, dry_run: bool, no_push: bool) -> anyhow::Result<u8> {
    let p = match prepare_run(cli)? {
        Prep::Ready(p) => p,
        Prep::Exit(code) => return Ok(code),
    };
    let cfg = &p.loaded.config;
    let style = Style::detect();

    // The gate (#26). Unlike `plan`, this command writes — so a branch outside
    // `release_branches` exits 0 quietly rather than being asked to explain
    // itself (H1).
    if !p.on_release_branch {
        tracing::info!(branch = %p.branch, "not a release branch; nothing to do");
        return Ok(exit::OK);
    }

    if p.plan.is_empty() {
        if !cli.quiet {
            print!("{}", report::summarise_edits(&style, 0));
        }
        return Ok(exit::OK);
    }

    let root = p.repo.root();
    let date = cli.date.unwrap_or_else(today_utc);

    // `forge.repo` when stated, otherwise derived from `origin`. Deriving is fine
    // here and nowhere near `forge.kind` (#66): a wrong link costs a 404 in a
    // changelog, a wrong forge costs a release.
    let repo_slug = cfg.forge.repo.clone().or_else(|| {
        p.repo.remote_url("origin").ok().flatten().as_deref().and_then(prepare::repo_from_remote)
    });
    let links = prepare::link_style(cfg, repo_slug.as_deref());

    // Resolved first: `sources` must read the redirect targets as well as the
    // declared paths.
    let workspace_roots = prepare::workspace_roots(root, cfg, &p.plan);
    let sources = prepare::sources(root, cfg, &p.plan, &workspace_roots)?;
    let overrides = prepare::overrides(root);

    let rendered = cicada_core::render::render(&cicada_core::render::RenderInput {
        config: cfg,
        plan: &p.plan,
        versions: &p.versions.versions,
        date: &date.to_string(),
        links: &links,
        sources: &sources,
        workspace_roots: &workspace_roots,
        overrides: &overrides,
    })?;

    for w in &rendered.warnings {
        eprintln!("warning: {w}");
    }

    if dry_run {
        if !cli.quiet {
            print!("{}", report::render_diff(&style, &rendered.edits, &sources));
            print!("{}", report::summarise_edits(&style, rendered.edits.len()));
        }
        return Ok(exit::OK);
    }

    // M3 is the git write path: reset the release PR branch, commit as the
    // configured committer, and (M4) upsert the PR. Writing the files without any
    // of that would leave a dirty tree with no branch to carry it, which is worse
    // than not writing them.
    let _ = no_push;
    anyhow::bail!(
        "only `--dry-run` is implemented in this build: the release commit and branch land in M3, \
         the pull request in M4\nthe plan and the diff are correct now — `cicada release-pr \
         --dry-run` prints exactly what would be written"
    );
}

/// The `plan --json` payload (#69).
///
/// Hand-rolled for the same reason `check`'s is: the schema is a machine
/// interface with a compatibility promise, and writing it out keeps that visible
/// rather than emergent from a derive. Fields may be added within `schema`;
/// removals or meaning changes bump it.
fn plan_json(
    date: &jiff::civil::Date,
    branch: &str,
    on_release_branch: bool,
    plan: &cicada_core::plan::Plan,
) -> String {
    let esc = report::json_escape;
    let releases: Vec<String> = plan
        .releases
        .iter()
        .map(|r| {
            let commits: Vec<String> = r
                .commits
                .iter()
                .map(|c| {
                    let scopes: Vec<String> =
                        c.scopes.iter().map(|s| format!("\"{}\"", esc(s))).collect();
                    format!(
                        r#"{{"sha":"{}","type":"{}","scopes":[{}],"breaking":{},"description":"{}"}}"#,
                        esc(&c.sha),
                        esc(&c.kind),
                        scopes.join(","),
                        c.breaking,
                        esc(&c.description)
                    )
                })
                .collect();
            // `from` is null on a first release — present and explicit (#89).
            // `"0.0.0"` names a version that never existed and produces compare
            // URLs that 404; omitting the key is ambiguous with an older schema.
            let from = match &r.from {
                Some(v) => format!("\"{}\"", esc(&v.to_string())),
                None => "null".to_string(),
            };
            let derived = match &r.derived_from {
                Some(d) => format!(
                    r#"{{"package":"{}","version":"{}"}}"#,
                    esc(&d.package),
                    esc(&d.version.to_string())
                ),
                None => "null".to_string(),
            };
            format!(
                r#"{{"package":"{}","from":{from},"to":"{}","bump":"{}","reason":"{}","tag":"{}","derived_from":{derived},"commits":[{}]}}"#,
                esc(&r.name),
                esc(&r.to.to_string()),
                r.bump.as_str(),
                r.reason.as_str(),
                esc(&r.tag),
                commits.join(",")
            )
        })
        .collect();

    format!(
        "{{\"schema\":1,\"date\":\"{date}\",\"branch\":\"{}\",\"release_branch\":{},\"releases\":[{}]}}\n",
        esc(branch),
        on_release_branch,
        releases.join(",")
    )
}

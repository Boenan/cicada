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
use cicada::forge::{self, Forge, PrRequest, flow};
use cicada::{auth, config, git, history, phase_b, prepare, report, versions};

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

    /// Forge API token. Also read from `CICADA_TOKEN`.
    ///
    /// Never persisted, never written to `.git/config`, and never placed in
    /// `argv` for the git subprocess — it reaches git through `GIT_ASKPASS` (#18).
    #[arg(long, global = true, value_name = "TOKEN", env = "CICADA_TOKEN", hide_env_values = true)]
    token: Option<String>,

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

    /// Phase B — reconcile tags and release objects.
    ///
    /// Stateless: it asks whether the tag for each recorded version exists, and
    /// whether a release object exists for it, and creates what is missing (#56).
    Release {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        no_push: bool,
    },

    /// Phase B then Phase A, in that order. What CI should call.
    ///
    /// **The order is the point** (#35). Both phases are triggered by the same
    /// event, so getting it wrong is CI's to get wrong and is silent: if A ran
    /// first it would compute its range from the *old* tag, re-include everything
    /// the just-merged release contained, and propose releasing it again with
    /// duplicate changelog entries, every cycle. This subcommand exists so a CI
    /// file cannot express the broken order.
    Run {
        #[arg(long)]
        dry_run: bool,
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
    // Before argument parsing: when git spawns cicada to answer a credential
    // prompt it passes the prompt text as `argv[1]`, which clap would reject.
    if let Some(code) = auth::askpass_short_circuit() {
        return code;
    }

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
        Command::Release { dry_run, no_push } => cmd_release(cli, mode_of(*dry_run, *no_push)),
        Command::Run { dry_run, no_push } => cmd_run(cli, mode_of(*dry_run, *no_push)),
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

/// Everything every command does before the phases diverge: discover the repo,
/// load and validate the config, classify the branch, read the versions file.
///
/// Kept in one place because the *order* matters and is easy to get subtly wrong:
/// shallow detection precedes any range (H8), config validation precedes any git
/// work, and the prerelease refusal precedes planning rather than following it.
struct Context {
    repo: GitCli,
    loaded: config::Loaded,
    branch: String,
    on_release_branch: bool,
    maintenance: bool,
    versions: versions::Loaded,
}

/// A context plus the plan. **Phase B deliberately does not have one**: anchor
/// resolution hard-errors when the recorded version's tag does not exist yet
/// (J1), which is precisely the state between a merge and the tag Phase B is
/// about to create (C8). Computing a plan there would refuse to run the thing
/// that fixes it.
struct Prepared {
    ctx: Context,
    plan: cicada_core::plan::Plan,
}

enum Prep<T> {
    Ready(Box<T>),
    /// An exit code that is the whole answer — a refusal, or the branch gate.
    Exit(u8),
}

fn prepare_context(cli: &Cli) -> anyhow::Result<Context> {
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
    // *stable* version, which is not a missing feature but a wrong answer — and a
    // plan that lies offline defeats the point of it running offline (§Goals 6).
    // Refusing is loud; computing would be quiet. Deleting this block is what
    // lands prerelease support.
    if cicada_core::branch::matches_any(&cfg.settings.prerelease_branches, &branch_name)? {
        anyhow::bail!(
            "'{branch_name}' matches `prerelease_branches`, and prerelease support is not \
             implemented yet\ncicada would otherwise compute a stable version here, which would be \
             wrong rather than incomplete"
        );
    }

    let versions = versions::load(&loaded.path)?;
    tracing::debug!(
        versions = %versions.path.display(),
        exists = versions.exists,
        "loaded versions file"
    );

    Ok(Context { repo, loaded, branch: branch_name, on_release_branch, maintenance, versions })
}

fn prepare_run(cli: &Cli) -> anyhow::Result<Prep<Prepared>> {
    let ctx = prepare_context(cli)?;
    let cfg = &ctx.loaded.config;

    let collected = history::collect(&ctx.repo, cfg, &ctx.versions.versions, "HEAD")?;
    for (target, anchor) in &collected.anchors {
        tracing::debug!(?target, anchor = ?anchor, "resolved anchor");
    }

    let input = PlanInput {
        config: cfg,
        versions: &ctx.versions.versions,
        history: &collected.history,
        maintenance: ctx.maintenance,
        branch: ctx.branch.clone(),
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

    Ok(Prep::Ready(Box::new(Prepared { ctx, plan })))
}

fn cmd_plan(cli: &Cli) -> anyhow::Result<u8> {
    let p = match prepare_run(cli)? {
        Prep::Ready(p) => p,
        Prep::Exit(code) => return Ok(code),
    };
    let date = cli.date.unwrap_or_else(today_utc);

    if cli.json {
        print!("{}", plan_json(&date, &p.ctx.branch, p.ctx.on_release_branch, &p.plan));
    } else if !cli.quiet {
        let style = Style::detect();
        print!("{}", report::render_plan(&style, &p.plan));
        if !p.ctx.on_release_branch {
            // Not an error: `plan` is a question, and the answer is the same
            // wherever it is asked. But `run` here would exit 0 without acting
            // (#26, H1), and discovering that from a silent pipeline is worse
            // than reading it now.
            eprintln!(
                "\nnote: '{}' matches no `release_branches` pattern — `cicada run` would exit 0 \
                 here without releasing",
                p.ctx.branch
            );
        }
    }

    // "Nothing to release" is a normal outcome, not a failure (#68).
    Ok(exit::OK)
}

/// How far a `release-pr` run goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Compute and print; touch nothing (#25).
    DryRun,
    /// Write a real release commit on a real local branch; push nothing.
    NoPush,
    /// Push the branch and upsert the pull request.
    Full,
}

impl Mode {
    fn touches_the_forge(self) -> bool {
        self == Mode::Full
    }
}

fn mode_of(dry_run: bool, no_push: bool) -> Mode {
    match (dry_run, no_push) {
        (true, _) => Mode::DryRun,
        (false, true) => Mode::NoPush,
        (false, false) => Mode::Full,
    }
}

/// Phase A — compute the release, write the files, open the pull request.
///
/// The steps are the design's, in the design's order (§Phase A). Two of them are
/// refusals, and both run in every mode: a rehearsal that reports success where
/// the real run would refuse is a rehearsal that lies.
fn cmd_release_pr(cli: &Cli, dry_run: bool, no_push: bool) -> anyhow::Result<u8> {
    let mode = mode_of(dry_run, no_push);

    let p = match prepare_run(cli)? {
        Prep::Ready(p) => p,
        Prep::Exit(code) => return Ok(code),
    };
    let cfg = &p.ctx.loaded.config;
    let style = Style::detect();

    // 0. The gate (#26). This command writes, so a branch outside
    //    `release_branches` does nothing and exits `0` — a push to a feature
    //    branch is not a failure, and a CI job must not go red for it.
    //
    //    But *silently* doing nothing is indistinguishable from a broken tool,
    //    and that is what a human running this by hand hits first. The note goes
    //    to **stderr**: H1's assertion is that stdout stays empty (so `--json`
    //    consumers and pipelines are unaffected), not that cicada refuses to
    //    explain itself.
    if !p.ctx.on_release_branch {
        gate_note(cli, &p.ctx);
        return Ok(exit::OK);
    }

    let head_branch =
        cicada_core::branch::render_pr_branch(&cfg.settings.release_pr_branch, &p.ctx.branch);

    // The forge, and the credentials for it, only when a run will actually reach
    // one. `plan`, `--dry-run` and `--no-push` need no token (#25), and building
    // an adapter would demand one.
    let token = auth::resolve(cli.token.as_deref());
    let repo_slug = cfg.forge.repo.clone().or_else(|| {
        p.ctx.repo.remote_url("origin").ok().flatten().as_deref().and_then(prepare::repo_from_remote)
    });
    let forge: Option<Box<dyn Forge>> = if mode.touches_the_forge() {
        let token = token.clone().ok_or(forge::ForgeError::NoToken)?;
        let slug = repo_slug.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "cicada cannot tell which repository to open a pull request against\n\
                 set `repo` under `[forge]` in cicada.toml, or give the repo an `origin` remote"
            )
        })?;
        Some(forge::build(cfg, &slug, token.expose().to_string())?)
    } else {
        None
    };

    // 4. An empty plan closes any open release PR (#34) — leaving it open
    //    advertises a release that is no longer justified. On a quiet repo this
    //    is the most-executed path in the tool (D5a), so it makes exactly one
    //    read call and no mutating call unless there is something to close.
    if p.plan.is_empty() {
        if let Some(f) = &forge
            && let Some(number) = flow::close_open_pr(f.as_ref(), &head_branch)?
            && !cli.quiet
        {
            println!("  {} closed #{number} — nothing left to release", style.dim("·"));
        }
        if !cli.quiet {
            print!("{}", report::summarise_edits(&style, 0, false));
        }
        return Ok(exit::OK);
    }

    let root = p.ctx.repo.root();
    let date = cli.date.unwrap_or_else(today_utc);

    // 5. Render the plan to edits.
    //
    // `forge.repo` when stated, otherwise derived from `origin`. Deriving is fine
    // here and nowhere near `forge.kind` (#66): a wrong link costs a 404 in a
    // changelog, a wrong forge costs a release.
    let links = prepare::link_style(cfg, repo_slug.as_deref());

    // Resolved first: `sources` must read the redirect targets as well as the
    // declared paths.
    let workspace_roots = prepare::workspace_roots(root, cfg, &p.plan);
    let sources = prepare::sources(root, cfg, &p.plan, &workspace_roots)?;
    let overrides = prepare::overrides(root);

    let rendered = cicada_core::render::render(&cicada_core::render::RenderInput {
        config: cfg,
        plan: &p.plan,
        versions: &p.ctx.versions.versions,
        date: &date.to_string(),
        links: &links,
        sources: &sources,
        workspace_roots: &workspace_roots,
        overrides: &overrides,
    })?;

    for w in &rendered.warnings {
        eprintln!("warning: {w}");
    }

    // 6. A computed version whose tag already exists elsewhere means two release
    //    lines are converging on one number (#80). Refuse *before* a PR exists,
    //    rather than after a human has reviewed and merged it.
    if let Some(code) = collision_check(&p)? {
        return Ok(code);
    }

    // 7. Do not clobber a branch cicada did not write (#32).
    //
    //    The branch at risk is the one about to be overwritten. When cicada is
    //    going to push, that is the **remote** branch — and in CI the local one
    //    does not exist at all, so consulting it would make this guard inert
    //    exactly where it matters. Fetch first, then judge.
    let askpass = token.as_ref().map(auth::askpass_env).unwrap_or_default();
    let push_url = if mode.touches_the_forge() {
        p.ctx.repo
            .remote_url("origin")?
            .as_deref()
            .and_then(|u| auth::push_url(u, cfg.forge.kind))
    } else {
        None
    };

    let remote_tip = match (&push_url, mode.touches_the_forge()) {
        (Some(url), true) => p.ctx.repo.fetch_branch(url, &head_branch, &askpass)?,
        _ => None,
    };
    let guard_tip = match mode.touches_the_forge() {
        true => remote_tip.clone(),
        // Without a push there is no remote to consult and no token to consult it
        // with. The local branch is what a rehearsal can see, and it is what a
        // rehearsal would overwrite.
        false => p.ctx.repo.branch_tip(&head_branch)?,
    };

    let identity = cicada_core::Identity::parse(&cfg.settings.committer)
        .map_err(|e| anyhow::anyhow!("settings.committer: {e}"))?;

    if let Some(tip) = &guard_tip
        && p.ctx.repo.committer_of(tip)?.as_deref() != Some(identity.to_git().as_str())
    {
        return refuse_stale(&p, forge.as_deref(), &head_branch, tip, &identity, cli.quiet);
    }

    if mode == Mode::DryRun {
        if !cli.quiet {
            print!("{}", report::render_diff(&style, &rendered.edits, &sources));
            print!("{}", report::summarise_edits(&style, rendered.edits.len(), false));
        }
        return Ok(exit::OK);
    }

    // 8. Reset the head branch to HEAD, apply the edits, commit.
    let message = commit_message(cfg, &p.plan);
    let outcome = p.ctx.repo.commit_edits(&git::CommitRequest {
        branch: &head_branch,
        base: "HEAD",
        edits: &rendered.edits,
        message: &message,
        identity: &identity,
    })?;

    if !cli.quiet {
        let verb = if outcome.created { "committed" } else { "unchanged" };
        println!(
            "  {} {}  {}",
            style.green("✓"),
            head_branch,
            style.dim(&format!("{verb} {}", short(&outcome.sha)))
        );
        print!("{}", report::summarise_edits(&style, rendered.edits.len(), true));
    }

    let Some(f) = forge else {
        if !cli.quiet {
            println!("  {}", style.dim(&format!("inspect it with `git show {head_branch}`")));
        }
        return Ok(exit::OK);
    };

    // 9. Push, then upsert the pull request.
    let url = push_url.ok_or_else(|| {
        anyhow::anyhow!(
            "cicada cannot work out where to push: `origin` is missing, or its URL has no HTTPS \
             equivalent\ntoken auth is HTTP Basic, so the push must be over HTTPS"
        )
    })?;
    p.ctx.repo.push_branch(&url, &outcome.sha, &head_branch, remote_tip.as_deref(), &askpass)?;

    let req = PrRequest {
        head: head_branch.clone(),
        // The base is the branch cicada ran on, never a fixed default (#27).
        base: p.ctx.branch.clone(),
        title: cfg.settings.release_pr_title.clone(),
        body: pr_body(&p.plan, &rendered.notes),
    };

    let (pr, created) = flow::upsert(f.as_ref(), &req)?;

    if !cli.quiet {
        let verb = if created { "opened" } else { "updated" };
        println!("  {} {verb} {}", style.green("✓"), pr.url);
    }
    Ok(exit::OK)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(9)]
}

/// #32 / #33 layer 2 — refuse, and make the resulting pull request unmergeable.
///
/// Refusing alone is not enough. A stale release PR that gets merged ships the
/// intervening commits with **no changelog entry and no influence on the version
/// number**: the code ships, the semver contract breaks silently, and a `feat`
/// goes out under a patch bump where nobody sees it. Draft state is what the
/// forge itself enforces, which is why it is the layer that does not depend on
/// repo configuration.
fn refuse_stale(
    p: &Prepared,
    forge: Option<&dyn Forge>,
    head_branch: &str,
    tip: &str,
    identity: &cicada_core::Identity,
    quiet: bool,
) -> anyhow::Result<u8> {
    let committer = p.ctx.repo.committer_of(tip)?.unwrap_or_default();
    eprintln!(
        "error: `{head_branch}` has a commit cicada did not write, so cicada will not replace it\n\
         \n  tip       {}\n  committer {committer}\n  expected  {}\n\
         \nThat commit exists nowhere else — it is not derivable from `{}`, so discarding it would\n\
         lose it. Move the change onto `{}` if it belongs there, then delete or reset\n\
         `{head_branch}` and run again.",
        short(tip),
        identity.to_git(),
        p.ctx.branch,
        p.ctx.branch,
    );

    if let Some(f) = forge {
        let body = format!(
            "cicada could not refresh this pull request.\n\n             `{head_branch}` carries a commit cicada did not write (`{}`, by {committer}), and              discarding it would lose work that exists nowhere else.\n\n             **This pull request is now a draft and must not be merged.** Merging it would ship              the commits that have landed since, with no changelog entry and no effect on the              version number.\n\n             To recover: move that change onto `{}` if it belongs there, then delete or reset              `{head_branch}`. The next run will regenerate this pull request.",
            short(tip),
            p.ctx.branch,
        );
        if let Some(number) = flow::mark_stale(f, head_branch, &body)?
            && !quiet
        {
            eprintln!(
                "\n#{number} has been drafted and labelled `{}`, so it cannot be merged.",
                cicada::forge::STALE_LABEL
            );
        }
    }
    Ok(exit::REFUSED)
}

/// The pull request body (D7).
///
/// **The same notes the changelog entries carry** (#43) — reused verbatim from
/// the render rather than produced by a second template, because two templates
/// drift apart within a month. With more than one release the entries are grouped
/// by package, the way the umbrella changelog groups them (#15).
fn pr_body(
    plan: &cicada_core::plan::Plan,
    notes: &std::collections::BTreeMap<cicada_core::Target, String>,
) -> String {
    if plan.releases.len() == 1 {
        let r = &plan.releases[0];
        let body = notes.get(&r.target).cloned().unwrap_or_default();
        return format!("## {} {}\n\n{body}\n", r.name, r.to);
    }
    let mut out = String::new();
    for r in &plan.releases {
        out.push_str(&format!("## {} {}\n\n", r.name, r.to));
        out.push_str(notes.get(&r.target).map(String::as_str).unwrap_or_default());
        out.push_str("\n\n");
    }
    out
}

/// Say why a command that writes did nothing (#26).
///
/// Acting on a branch outside `release_branches` is not an error and must not
/// fail a CI job — but *silently* doing nothing is indistinguishable from a
/// broken tool, and that is what a human running the command by hand hits first.
///
/// It goes to **stderr**: H1's assertion is that stdout stays empty, so `--json`
/// consumers and pipelines are unaffected, not that cicada refuses to explain
/// itself.
fn gate_note(cli: &Cli, ctx: &Context) {
    tracing::info!(branch = %ctx.branch, "not a release branch; nothing to do");
    if cli.quiet {
        return;
    }
    eprintln!(
        "cicada: nothing to do — '{}' matches no `release_branches` pattern ({})\n\
         \n\
         cicada only writes on branches you have listed, so this is a no-op rather than\n\
         an error. To rehearse a release from here, either run `cicada plan`, which is\n\
         not gated, or add this branch to `release_branches` in {}.",
        ctx.branch,
        ctx.loaded
            .config
            .settings
            .release_branches
            .iter()
            .map(|b| format!("`{b}`"))
            .collect::<Vec<_>>()
            .join(", "),
        ctx.loaded.path.display(),
    );
}

/// The credentials and forge a run needs, when it needs them.
///
/// Built once and shared by both phases, because `cicada run` reaches the forge
/// twice and resolving a token twice would let the two halves disagree.
struct Wiring {
    forge: Option<Box<dyn Forge>>,
    push: Option<(String, Vec<(String, String)>)>,
    links: cicada_core::notes::LinkStyle,
}

fn wire(cli: &Cli, ctx: &Context, mode: Mode) -> anyhow::Result<Wiring> {
    let cfg = &ctx.loaded.config;
    let token = auth::resolve(cli.token.as_deref());
    let repo_slug = cfg.forge.repo.clone().or_else(|| {
        ctx.repo.remote_url("origin").ok().flatten().as_deref().and_then(prepare::repo_from_remote)
    });
    let links = prepare::link_style(cfg, repo_slug.as_deref());

    if !mode.touches_the_forge() {
        return Ok(Wiring { forge: None, push: None, links });
    }

    let token = token.ok_or(forge::ForgeError::NoToken)?;
    let slug = repo_slug.ok_or_else(|| {
        anyhow::anyhow!(
            "cicada cannot tell which repository to act on\n\
             set `repo` under `[forge]` in cicada.toml, or give the repo an `origin` remote"
        )
    })?;
    let url = ctx
        .repo
        .remote_url("origin")?
        .as_deref()
        .and_then(|u| auth::push_url(u, cfg.forge.kind))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cicada cannot work out where to push: `origin` is missing, or its URL has no \
                 HTTPS equivalent\ntoken auth is HTTP Basic, so the push must be over HTTPS"
            )
        })?;

    Ok(Wiring {
        forge: Some(forge::build(cfg, &slug, token.expose().to_string())?),
        push: Some((url, auth::askpass_env(&token))),
        links,
    })
}

/// Phase B — `cicada release`.
fn cmd_release(cli: &Cli, mode: Mode) -> anyhow::Result<u8> {
    let ctx = prepare_context(cli)?;
    if !ctx.on_release_branch {
        gate_note(cli, &ctx);
        return Ok(exit::OK);
    }
    run_phase_b(cli, &ctx, mode).map(|(code, _)| code)
}

/// `cicada run` — Phase B **then** Phase A (#35).
fn cmd_run(cli: &Cli, mode: Mode) -> anyhow::Result<u8> {
    let ctx = prepare_context(cli)?;
    let (code, _) = run_phase_b(cli, &ctx, mode)?;
    if code != exit::OK {
        // A refused or failed Phase B must not be followed by a Phase A that
        // plans against the state it refused to reconcile.
        return Ok(code);
    }
    // Phase A re-reads everything from scratch: the tag Phase B just created is
    // what makes its anchor resolve at all, which is the whole reason for the
    // ordering.
    cmd_release_pr(cli, mode == Mode::DryRun, mode == Mode::NoPush)
}

fn run_phase_b(cli: &Cli, ctx: &Context, mode: Mode) -> anyhow::Result<(u8, phase_b::Outcome)> {
    let cfg = &ctx.loaded.config;
    let style = Style::detect();

    // The gate (#26), same as Phase A: cicada writes here too. Silent, because
    // the caller explains it — `cicada run` would otherwise say it twice, once
    // per phase.
    if !ctx.on_release_branch {
        return Ok((exit::OK, phase_b::Outcome::default()));
    }

    let wiring = wire(cli, ctx, mode)?;
    let identity = cicada_core::Identity::parse(&cfg.settings.committer)
        .map_err(|e| anyhow::anyhow!("settings.committer: {e}"))?;
    let date = cli.date.unwrap_or_else(today_utc);
    let overrides = prepare::overrides(ctx.repo.root());

    if mode == Mode::DryRun {
        // Phase B's whole job is creating things; with nothing to create there is
        // no diff to show, so it reports what it *would* reconcile and stops.
        let pending = phase_b::pending(&ctx.repo, cfg, &ctx.versions.versions)?;
        if !cli.quiet {
            print!("{}", report::render_phase_b_dry_run(&style, &pending));
        }
        return Ok((exit::OK, phase_b::Outcome::default()));
    }

    let push = wiring.push.as_ref().map(|(u, e)| (u.as_str(), e.as_slice()));
    let outcome = match phase_b::run(
        &ctx.repo,
        &phase_b::PhaseBInput {
            config: cfg,
            versions: &ctx.versions.versions,
            date: &date.to_string(),
            links: &wiring.links,
            identity: &identity,
            overrides: &overrides,
            push,
            forge: wiring.forge.as_deref(),
        },
    ) {
        Ok(o) => o,
        // Both are refusals, not breakage (#68): cicada declined to act because
        // acting would ship something wrong.
        Err(e @ (phase_b::PhaseBError::Collision { .. } | phase_b::PhaseBError::StaleMerge { .. })) => {
            eprintln!("error: {e}");
            return Ok((exit::REFUSED, phase_b::Outcome::default()));
        }
        Err(e) => return Err(e.into()),
    };

    // The manifest goes to disk as well as into every release body (#42): under
    // `--no-push` there is no release object at all, so the file is the only way
    // to inspect a rehearsal.
    if !outcome.manifest.releases.is_empty() {
        prepare::write_manifest(ctx.repo.root(), &outcome.manifest)?;
    }

    if !cli.quiet {
        print!("{}", report::render_phase_b(&style, &outcome));
    }
    Ok((exit::OK, outcome))
}

/// #80 — the version-collision check, run in Phase A as well as Phase B.
///
/// A tag that exists but is **not reachable from HEAD** means it was cut on
/// another line that is about to claim the same number. Discovering that in Phase
/// B would mean the release PR had already been reviewed and merged, leaving the
/// branch merged-but-untagged; discovering it here costs nothing and lands the
/// error before a human has read anything.
fn collision_check(p: &Prepared) -> anyhow::Result<Option<u8>> {
    for r in &p.plan.releases {
        if p.ctx.repo.rev_parse(&r.tag)?.is_none() {
            continue;
        }
        if p.ctx.repo.is_ancestor(&r.tag, "HEAD")? {
            continue;
        }
        eprintln!(
            "error: '{}' computes version {}, but tag `{}` already exists somewhere this branch \
             cannot see\ntwo release lines are converging on one number — cicada will not open a \
             pull request for it\nthe tag was cut on another line; reconcile the two before \
             releasing either",
            r.name, r.to, r.tag
        );
        return Ok(Some(exit::REFUSED));
    }
    Ok(None)
}

/// The release commit's message.
///
/// The subject is `release_pr_title`, which is conventional and **non-releasable**
/// (#41, #36) — so cicada's own release commit cannot trigger a release on the
/// next run. The body lists what moved, in config order (#31), because `git show`
/// on this commit is the whole point of `--no-push` (H6).
fn commit_message(cfg: &cicada_core::Config, plan: &cicada_core::plan::Plan) -> String {
    let mut m = cfg.settings.release_pr_title.clone();
    m.push_str("\n\n");
    for r in &plan.releases {
        let from = r.from.as_ref().map(ToString::to_string).unwrap_or_else(|| "—".into());
        m.push_str(&format!("{} {} → {}\n", r.name, from, r.to));
    }
    m
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

//! cicada — release automation driven by conventional commits.
//!
//! Git is the substrate and the forge API is a thin edge; everything interesting
//! is a pure function in `cicada-core`, and this crate is the part that touches
//! the world (design §Architecture).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cicada_core::check::{Severity, lint_commits, validate_config, validate_release_pr_branch};
use cicada::git::{GitBackend, GitCli};
use cicada::report::Style;
use cicada::{config, report};

/// Exit codes are a contract, not an implementation detail (#68). CI branches on
/// them, so `3` (your commits are malformed) and `4` (cicada declined to act) are
/// separated from `1` (cicada broke).
mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    // 2 is clap's, for usage errors.
    pub const FINDINGS: u8 = 3;
    #[allow(dead_code)]
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
    }
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
    let esc = |s: &str| {
        s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t")
    };
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

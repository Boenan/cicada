//! Rendering findings for humans.
//!
//! Colour follows `NO_COLOR` and TTY detection (§CLI surface). No colour crate —
//! the palette is four codes and the rule is one function.

use std::io::IsTerminal;

use cicada_core::check::{CommitReport, Finding, Severity};

#[derive(Clone, Copy)]
pub struct Style {
    colour: bool,
}

impl Style {
    pub fn detect() -> Self {
        // NO_COLOR is honoured when set to anything at all, per the convention.
        let forced_off = std::env::var_os("NO_COLOR").is_some();
        Self { colour: !forced_off && std::io::stdout().is_terminal() }
    }
    pub fn plain() -> Self {
        Self { colour: false }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.colour { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
    }
    pub fn green(&self, s: &str) -> String {
        self.paint("32", s)
    }
    pub fn red(&self, s: &str) -> String {
        self.paint("31", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.paint("33", s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
}

/// The shape §`cicada check` specifies: a ✓ or ✗ per commit, findings indented
/// beneath the commit they belong to, and a count at the end.
pub fn render_commits(style: &Style, reports: &[CommitReport]) -> String {
    let mut s = String::new();
    for r in reports {
        if r.ok() {
            s.push_str(&format!("  {} {}\n", style.green("✓"), r.subject));
        } else {
            s.push_str(&format!("  {} {}\n", style.red("✗"), r.subject));
            for f in &r.findings {
                s.push_str(&render_finding(style, f, 6));
            }
        }
    }
    s
}

pub fn render_config_findings(style: &Style, findings: &[Finding]) -> String {
    let mut s = String::new();
    for f in findings {
        let mark = match f.severity {
            Severity::Problem => style.red("✗"),
            Severity::Warning => style.yellow("!"),
        };
        s.push_str(&format!("  {mark} {}\n", style.dim(&f.subject)));
        s.push_str(&render_finding(style, f, 6));
    }
    s
}

fn render_finding(style: &Style, f: &Finding, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut s = String::new();
    for line in f.message.lines() {
        s.push_str(&format!("{pad}{line}\n"));
    }
    if let Some(h) = &f.help {
        for line in h.lines() {
            s.push_str(&format!("{pad}{}\n", style.dim(line)));
        }
    }
    s
}

pub fn summarise(style: &Style, problems: usize, commits: usize) -> String {
    if problems == 0 {
        // A config-only run has no range, and "0 commits checked" reads like
        // something went wrong rather than like the thing that was asked for.
        if commits == 0 {
            return format!("{} config ok\n", style.green("✓"));
        }
        let n = format!("{commits} commit{}", plural(commits));
        format!("{} {n} checked\n", style.green("✓"))
    } else {
        format!(
            "{}\n",
            style.red(&format!(
                "{problems} problem{} in {commits} commit{}",
                plural(problems),
                plural(commits)
            ))
        )
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

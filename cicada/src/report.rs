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

// --- the plan ------------------------------------------------------------

/// What `cicada plan` prints. One line per release, aligned, with the commits
/// that earned it beneath — so "why" is answered on the same screen as "what".
///
/// An empty plan is a **successful** outcome (#68), so it says so plainly rather
/// than printing an empty table that reads like a failure.
pub fn render_plan(style: &Style, plan: &cicada_core::plan::Plan) -> String {
    use cicada_core::plan::Reason;

    if plan.is_empty() {
        return format!("  {}\n", style.dim("nothing to release"));
    }

    let width = plan.releases.iter().map(|r| r.name.chars().count()).max().unwrap_or(0);
    let mut s = String::new();
    for r in &plan.releases {
        let from = match &r.from {
            Some(v) => v.to_string(),
            // A first release has no previous version, and saying `0.0.0` would
            // name one that never existed (#89).
            None => "—".to_string(),
        };
        s.push_str(&format!(
            "  {:width$}  {} {} {}  {}",
            r.name,
            style.dim(&from),
            style.dim("→"),
            r.to,
            style.dim(r.bump.as_str()),
            width = width,
        ));
        let note = match r.reason {
            Reason::FirstRelease => Some("first release".to_string()),
            Reason::Aggregate => Some("aggregate".to_string()),
            Reason::Derived => r
                .derived_from
                .as_ref()
                .map(|d| format!("tracks {} {}", d.package, d.version)),
            Reason::Commits => None,
        };
        if let Some(n) = note {
            s.push_str(&format!("  {}", style.dim(&format!("({n})"))));
        }
        s.push('\n');
        for c in &r.commits {
            let scopes =
                if c.scopes.is_empty() { String::new() } else { format!("({})", c.scopes.join(",")) };
            let bang = if c.breaking { "!" } else { "" };
            s.push_str(&format!(
                "  {:width$}  {}\n",
                "",
                style.dim(&format!("{}{scopes}{bang}: {}", c.kind, c.description)),
                width = width,
            ));
        }
    }
    let n = plan.releases.len();
    s.push_str(&format!("\n  {} release{}\n", n, plural(n)));
    s
}

/// Escape a string for a JSON document.
///
/// The `--json` payloads are hand-rolled rather than serialised, because the
/// schema is a contract (#69) and writing it out is what makes that visible — but
/// escaping is not a place for judgement, so it lives in one function that both
/// payloads use. Control characters below 0x20 are escaped as `\uXXXX`; a commit
/// subject can contain them.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// --- the dry-run diff ----------------------------------------------------

/// What `release-pr --dry-run` prints: a real unified diff of the edits cicada
/// would write (#30).
///
/// A diff rather than a summary, because the claim being made is *these exact
/// bytes*. A list of filenames would be a description of the computation; this is
/// the computation's output.
pub fn render_diff(
    style: &Style,
    edits: &[cicada_core::edit::FileEdit],
    sources: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::new();
    for e in edits {
        let before = sources.get(&e.path).map(String::as_str).unwrap_or("");
        let new_file = !sources.contains_key(&e.path);

        out.push_str(&format!(
            "{} {}\n",
            style.dim(if new_file { "+++ new" } else { "---" }),
            e.path
        ));
        out.push_str(&format!("{}\n", style.dim(&format!("    {}", e.source))));

        let diff = similar::TextDiff::from_lines(before, &e.content);
        for group in diff.grouped_ops(3) {
            for op in group {
                for change in diff.iter_changes(&op) {
                    let (sign, paint): (&str, fn(&Style, &str) -> String) = match change.tag() {
                        similar::ChangeTag::Delete => ("-", Style::red),
                        similar::ChangeTag::Insert => ("+", Style::green),
                        similar::ChangeTag::Equal => (" ", Style::dim),
                    };
                    let text = change.value();
                    out.push_str(&paint(style, &format!("  {sign} {}", text.trim_end_matches('\n'))));
                    out.push('\n');
                }
            }
        }
        out.push('\n');
    }
    out
}

/// `would` for a rehearsal, `were` once it has happened. The distinction is the
/// only thing separating the two modes, so the output says which one ran.
pub fn summarise_edits(style: &Style, n: usize, written: bool) -> String {
    if n == 0 {
        return format!("  {}\n", style.dim("nothing to release"));
    }
    let verb = if written { "written" } else { "would be written" };
    format!("  {} file{} {verb}\n", n, plural(n))
}

// --- Phase B -------------------------------------------------------------

pub fn render_phase_b(style: &Style, outcome: &crate::phase_b::Outcome) -> String {
    let mut s = String::new();
    for (pkg, tag) in &outcome.tagged {
        s.push_str(&format!("  {} {tag}  {}\n", style.green("✓"), style.dim(&format!("tagged {pkg}"))));
    }
    for (pkg, tag) in &outcome.released {
        s.push_str(&format!("  {} {tag}  {}\n", style.green("✓"), style.dim(&format!("released {pkg}"))));
    }
    if outcome.is_no_op() {
        // A re-run that changes nothing is the expected outcome on most pushes,
        // not a problem (C3).
        let n = outcome.already_tagged.len();
        s.push_str(&format!(
            "  {}\n",
            style.dim(&if n == 0 {
                "nothing to reconcile".to_string()
            } else {
                format!("nothing to reconcile — {n} tag{} already in place", plural(n))
            })
        ));
    }
    s
}

pub fn render_phase_b_dry_run(style: &Style, pending: &[crate::phase_b::Pending]) -> String {
    use crate::phase_b::TagState;
    if pending.is_empty() {
        return format!("  {}\n", style.dim("nothing to reconcile"));
    }
    let width = pending.iter().map(|p| p.tag.chars().count()).max().unwrap_or(0);
    let mut s = String::new();
    for p in pending {
        let (mark, note) = match p.state {
            TagState::Tagged => (style.dim("·"), style.dim("already tagged")),
            TagState::WouldTag => (style.green("+"), style.dim("would be created")),
            TagState::Collides => (style.red("✗"), style.red("exists elsewhere — would refuse")),
        };
        s.push_str(&format!("  {mark} {:width$}  {note}\n", p.tag, width = width));
    }
    s
}

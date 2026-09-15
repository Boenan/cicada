//! Conventional-commit parsing.
//!
//! Hand-written rather than an off-the-shelf crate, because of the multi-scope
//! extension: `feat(api,worker)!:` names two packages in one commit, and that is
//! how blast radius is declared (design §Commit parsing, decision #11).
//!
//! Pure: `&str -> ParsedCommit`. No I/O, no config, no scope resolution — routing
//! a scope to a package needs the config and happens in [`crate::route`].

/// A commit as git handed it to us, before any interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCommit {
    pub sha: String,
    pub subject: String,
    pub body: String,
}

impl RawCommit {
    pub fn new(
        sha: impl Into<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self { sha: sha.into(), subject: subject.into(), body: body.into() }
    }
}

/// A commit that parsed as a conventional commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommit {
    pub sha: String,
    /// `feat`, `fix`, … — matched against configured changelog sections. An
    /// unrecognised type is ignored, never an error (design §Commit parsing).
    pub kind: String,
    /// Comma-separated scopes, in the order written. Empty means unscoped, which
    /// routes to `default_scope` (#38).
    pub scopes: Vec<String>,
    /// `!` after the scope, or a `BREAKING CHANGE:` / `BREAKING-CHANGE:` footer.
    pub breaking: bool,
    /// The text after `: `, with no type, scope or `!`.
    pub description: String,
    pub body: String,
    pub footers: Vec<Footer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub key: String,
    pub value: String,
}

/// Why a commit is not a conventional commit. Carried rather than thrown: reading
/// history must not stop on a malformed subject, it must skip it. `check` is where
/// this becomes a finding (#12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// No `type:` prefix at all — `update deps`.
    NotConventional,
    /// `feat: ` with nothing after it.
    EmptyDescription,
    /// `feat(): …` — parentheses present but empty.
    EmptyScope,
    /// `feat(api,,worker): …` or a scope that is only whitespace.
    EmptyScopeItem,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ParseError::NotConventional => "not a conventional commit",
            ParseError::EmptyDescription => "empty description",
            ParseError::EmptyScope => "empty scope — write `feat:` rather than `feat():`",
            ParseError::EmptyScopeItem => "empty scope in a multi-scope list",
        };
        f.write_str(s)
    }
}

/// `BREAKING CHANGE` and `BREAKING-CHANGE` are both spelled in the wild; the spec
/// blesses the hyphen as the footer token and the space as a synonym.
fn is_breaking_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("BREAKING CHANGE") || key.eq_ignore_ascii_case("BREAKING-CHANGE")
}

/// Type and scope are restricted so that a subject like
/// `fix a bug: it was broken` does not parse as type `fix a bug`.
fn is_type_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

pub fn parse(raw: &RawCommit) -> Result<ParsedCommit, ParseError> {
    let subject = raw.subject.trim_end();

    // --- type -------------------------------------------------------------
    let type_end = subject
        .find(|c: char| !is_type_char(c))
        .ok_or(ParseError::NotConventional)?;
    if type_end == 0 {
        return Err(ParseError::NotConventional);
    }
    let kind = &subject[..type_end];
    let mut rest = &subject[type_end..];

    // --- optional (scope,scope) ------------------------------------------
    let mut scopes = Vec::new();
    if let Some(stripped) = rest.strip_prefix('(') {
        let close = stripped.find(')').ok_or(ParseError::NotConventional)?;
        let inner = &stripped[..close];
        if inner.trim().is_empty() {
            return Err(ParseError::EmptyScope);
        }
        for part in inner.split(',') {
            let s = part.trim();
            if s.is_empty() {
                return Err(ParseError::EmptyScopeItem);
            }
            scopes.push(s.to_string());
        }
        rest = &stripped[close + 1..];
    }

    // --- optional `!`, then the mandatory `:` -----------------------------
    let mut breaking = false;
    if let Some(stripped) = rest.strip_prefix('!') {
        breaking = true;
        rest = stripped;
    }
    let rest = rest.strip_prefix(':').ok_or(ParseError::NotConventional)?;

    // The spec requires `: ` — a space after the colon. Enforcing it keeps
    // `https://example.com` and `note:see below` out of the parser.
    if !rest.is_empty() && !rest.starts_with(' ') {
        return Err(ParseError::NotConventional);
    }
    let description = rest.trim();
    if description.is_empty() {
        return Err(ParseError::EmptyDescription);
    }

    let (body, footers) = split_body_and_footers(&raw.body);
    if footers.iter().any(|f| is_breaking_key(&f.key)) {
        breaking = true;
    }

    Ok(ParsedCommit {
        sha: raw.sha.clone(),
        kind: kind.to_string(),
        scopes,
        breaking,
        description: description.to_string(),
        body,
        footers,
    })
}

/// Footers are the trailing run of `Key: value` (or `Key #value`) lines. A line
/// that is not a footer ends the run, so prose containing a colon stays prose.
///
/// Note what is deliberately absent: `Release-As:` is read like any other footer
/// and **acted on by nothing** (#39). Forced versions are out of v1 scope, and a
/// repo migrating from release-please will carry those footers in its history —
/// honouring them would be wrong and erroring on them would block adoption.
fn split_body_and_footers(body: &str) -> (String, Vec<Footer>) {
    let body = body.trim_end();
    if body.is_empty() {
        return (String::new(), Vec::new());
    }
    let lines: Vec<&str> = body.lines().collect();

    // The footer block, if there is one, is the trailing run after the last blank
    // line — and it is a footer block only if its *first* line parses as a footer.
    // Anchoring on the first line rather than scanning upward is what keeps a
    // multi-line footer value from ending the run at its own continuation.
    let block_start = lines
        .iter()
        .rposition(|l| l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);

    if block_start >= lines.len() || parse_footer_line(lines[block_start]).is_none() {
        return (body.to_string(), Vec::new());
    }

    let mut footers: Vec<Footer> = Vec::new();
    for line in &lines[block_start..] {
        match parse_footer_line(line) {
            Some(f) => footers.push(f),
            // Not a footer: a continuation of the previous footer's value.
            None => {
                if let Some(last) = footers.last_mut() {
                    last.value.push('\n');
                    last.value.push_str(line.trim_end());
                }
            }
        }
    }

    let body_text = lines[..block_start].join("\n").trim_end().to_string();
    (body_text, footers)
}

fn parse_footer_line(line: &str) -> Option<Footer> {
    // `BREAKING CHANGE: …` is the one key allowed to contain a space.
    if let Some(rest) = line.strip_prefix("BREAKING CHANGE:") {
        return Some(Footer { key: "BREAKING CHANGE".into(), value: rest.trim().into() });
    }
    let colon = line.find(": ")?;
    let key = &line[..colon];
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(Footer { key: key.to_string(), value: line[colon + 2..].trim().to_string() })
}

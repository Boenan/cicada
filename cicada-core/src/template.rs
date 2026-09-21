//! Templating (#43–#48) — `minijinja`, Jinja2 syntax.
//!
//! **Two** things are rendered: user-declared file templates (`template` → `output`,
//! an updater kind) and the notes body. The shipped CI assets are *not* templates —
//! they are static YAML users reference, and minijinja never touches them.
//!
//! **Autoescape is off** (#45), because the output is Markdown and YAML rather than
//! HTML and escaping it as HTML would corrupt it. That places the burden on
//! filters, since commit subjects are semi-untrusted input flowing into generated
//! files: a subject containing `: ` or a stray quote can produce an invalid YAML
//! manifest. `| yaml`, `| json` and `| md` are provided for exactly that, and
//! template authors are expected to use them on anything derived from commit text.
//!
//! **The built-in defaults are printable** (#48), so customising starts from the
//! real thing rather than from reverse-engineering it. This is deliberately the
//! opposite of release-please, whose changelog format is notoriously hard to
//! customise *because* you cannot see the template you are deviating from.

use minijinja::{Environment, Error as JinjaError, ErrorKind, Value};

/// The entry body (#43). One template, two destinations: the changelog entry and
/// the forge release body are the same content in different frames, so rendering
/// them from two templates would let them drift apart within a month.
pub const NOTES: &str = include_str!("templates/notes.md.j2");

/// The changelog *frame* — the version header around a rendered notes body. The
/// release body does not repeat the version, which is the only difference between
/// the two destinations.
pub const CHANGELOG: &str = include_str!("templates/changelog.md.j2");

/// The umbrella's changelog entry, grouped by service (#15).
pub const UMBRELLA: &str = include_str!("templates/umbrella.md.j2");

/// The header a `CHANGELOG.md` is created with when none exists (#5). Carries the
/// insert marker, so every later release takes the marker path rather than the
/// prepend-after-header fallback.
pub const CHANGELOG_HEADER: &str = include_str!("templates/changelog-header.md");

/// Every built-in, by the name `cicada template print <name>` accepts.
pub const BUILT_INS: &[(&str, &str)] = &[
    ("notes", NOTES),
    ("changelog", CHANGELOG),
    ("umbrella", UMBRELLA),
    ("changelog-header", CHANGELOG_HEADER),
];

pub fn built_in(name: &str) -> Option<&'static str> {
    BUILT_INS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

pub fn built_in_names() -> Vec<&'static str> {
    BUILT_INS.iter().map(|(n, _)| *n).collect()
}

#[derive(Debug, thiserror::Error)]
#[error("{name}: {message}")]
pub struct RenderError {
    pub name: String,
    pub message: String,
}

fn render_error(name: &str, e: JinjaError) -> RenderError {
    // minijinja's `Display` gives the kind and line; the chained cause carries the
    // detail, and losing it turns "invalid operation" into a guessing game.
    let mut message = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(e) = src {
        message.push_str(&format!(": {e}"));
        src = e.source();
    }
    RenderError { name: name.to_string(), message }
}

/// An environment with the filters registered and autoescape off.
pub fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.add_filter("yaml", filter_yaml);
    env.add_filter("json", filter_json);
    env.add_filter("md", filter_md);
    env
}

/// Render one template by source. Templates are small and rendered a handful of
/// times per run, so a fresh environment per render costs nothing and removes any
/// question about name collisions between a built-in and an override.
pub fn render(name: &str, source: &str, ctx: &impl serde::Serialize) -> Result<String, RenderError> {
    let mut env = environment();
    env.add_template(name, source).map_err(|e| render_error(name, e))?;
    let tmpl = env.get_template(name).map_err(|e| render_error(name, e))?;
    tmpl.render(ctx).map_err(|e| render_error(name, e))
}

/// `| yaml` — emit a value as a YAML scalar that always parses back.
///
/// Double-quoted unconditionally rather than cleverly: a subject beginning `- `,
/// containing `: `, or reading as a number, a date or `yes` all change meaning
/// unquoted, and the cost of always quoting is one pair of quotes.
fn filter_yaml(v: Value) -> Result<String, JinjaError> {
    let s = to_text(&v)?;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// `| json` — a JSON-encoded value, for embedding in a JSON or YAML document.
fn filter_json(v: Value) -> Result<String, JinjaError> {
    serde_json::to_string(&v).map_err(|e| {
        JinjaError::new(ErrorKind::InvalidOperation, format!("cannot encode as JSON: {e}"))
    })
}

/// `| md` — escape the Markdown metacharacters a commit subject plausibly carries,
/// so a subject cannot silently become emphasis, a link, or a heading.
fn filter_md(v: Value) -> Result<String, JinjaError> {
    let s = to_text(&v)?;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|') {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(out)
}

fn to_text(v: &Value) -> Result<String, JinjaError> {
    if v.is_none() || v.is_undefined() {
        return Ok(String::new());
    }
    Ok(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Ctx {
        s: String,
    }

    fn one(tmpl: &str, s: &str) -> String {
        render("t", tmpl, &Ctx { s: s.into() }).unwrap()
    }

    #[test]
    fn l7_a_hostile_subject_still_produces_parseable_yaml() {
        // The subject carries every character that breaks an unquoted scalar.
        let subject = r#"- fix: handle "quoted" input: properly"#;
        let out = one("annotations:\n  changelog: {{ s | yaml }}\n", subject);
        let parsed: serde_json::Value =
            serde_json::from_str(out.split_once(": ").unwrap().1.trim()).unwrap();
        assert_eq!(parsed.as_str().unwrap(), subject);
        // And the YAML parser agrees it is one scalar.
        assert!(crate::updater::yaml::verify(&out).is_ok(), "{out}");
    }

    #[test]
    fn yaml_quoting_survives_newlines_and_control_characters() {
        let out = one("v: {{ s | yaml }}\n", "a\nb\tc");
        assert!(crate::updater::yaml::verify(&out).is_ok(), "{out}");
        assert!(out.contains("\\n"), "{out}");
    }

    #[test]
    fn md_escapes_what_would_otherwise_become_formatting() {
        assert_eq!(one("{{ s | md }}", "a_b_c *d*"), r"a\_b\_c \*d\*");
        assert_eq!(one("{{ s | md }}", "[link](x)"), r"\[link\](x)");
    }

    #[test]
    fn json_round_trips() {
        let out = one("{{ s | json }}", "a\"b");
        assert_eq!(serde_json::from_str::<String>(&out).unwrap(), "a\"b");
    }

    #[test]
    fn autoescape_is_off_so_markdown_passes_through_unharmed() {
        // If autoescape were on, `<` and `&` would become entities and every
        // changelog would be subtly corrupt.
        assert_eq!(one("{{ s }}", "a < b & c"), "a < b & c");
    }

    #[test]
    fn every_built_in_parses_and_is_printable() {
        for (name, src) in BUILT_INS {
            if name.ends_with("header") {
                continue;
            }
            let mut env = environment();
            env.add_template(name, src).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn a_broken_template_reports_its_cause_not_just_its_kind() {
        let err = render("t", "{{ nope.deeper() }}", &Ctx { s: String::new() }).unwrap_err();
        assert!(err.message.len() > 20, "the chained cause is kept: {err}");
    }
}

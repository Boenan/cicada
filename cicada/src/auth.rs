//! Token handling and the push credential path (#18).
//!
//! **Token-only, always**, for both the API and the git push. Sourced from
//! `--token` or `CICADA_TOKEN`, and **never** written to `.git/config`, never to
//! disk, never into `argv`.
//!
//! ## How the token reaches git
//!
//! Through `GIT_ASKPASS`, which git invokes to answer a credential prompt. The
//! helper is **cicada itself**: `GIT_ASKPASS` points at the running binary, the
//! token travels in the child's environment, and [`askpass_short_circuit`] at the
//! top of `main` answers and exits before any other work happens.
//!
//! The obvious alternative — writing a two-line shell script to a temp file — was
//! rejected for two reasons: it needs an executable temp directory, which
//! hardened CI images routinely do not have (`noexec` on `/tmp`), and it leaves a
//! file to clean up on every error path. Neither is true of re-invoking a binary
//! that is already on disk and already executable.
//!
//! The **username** rides in the URL rather than through the helper, because git
//! asks for username and password with two separate prompts and answering both
//! from one helper means parsing prompt text. `x-access-token` and `oauth2` are
//! not secrets, so a URL carrying one is safe in `argv`; the token never is.

use cicada_core::config::ForgeKind;

/// Set on the git child to mark it as an askpass invocation.
const ASKPASS_MARKER: &str = "CICADA_INTERNAL_ASKPASS";
/// Carries the secret to that child, in its environment and nowhere else.
const ASKPASS_TOKEN: &str = "CICADA_INTERNAL_ASKPASS_TOKEN";

/// A token, deliberately without `Debug` or `Display`.
///
/// The type is the guard: a token that cannot be formatted cannot be logged by
/// accident, and `tracing` at `-vvv` is exactly where that would otherwise happen.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// `--token` beats `CICADA_TOKEN`; absent is not an error here, because `plan`,
/// `check`, `--dry-run` and `--no-push` all need no token at all (#25).
pub fn resolve(flag: Option<&str>) -> Option<Token> {
    flag.map(str::to_string)
        .or_else(|| std::env::var("CICADA_TOKEN").ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(Token)
}

/// If this process was spawned by git to answer a credential prompt, answer it
/// and exit.
///
/// Called as the **first** thing in `main`, before argument parsing: git passes
/// the prompt text as `argv[1]`, which clap would reject as an unknown argument.
pub fn askpass_short_circuit() -> Option<std::process::ExitCode> {
    std::env::var_os(ASKPASS_MARKER)?;
    // Both variables, so a stray marker in someone's shell cannot turn an
    // ordinary `cicada plan` into a credential helper that prints nothing.
    let token = std::env::var(ASKPASS_TOKEN).ok()?;
    println!("{token}");
    Some(std::process::ExitCode::SUCCESS)
}

/// The environment a git subprocess needs to authenticate.
///
/// `GIT_TERMINAL_PROMPT=0` is already pinned globally, so a missing or rejected
/// credential fails immediately instead of hanging a CI job on a prompt nobody
/// will ever answer.
pub fn askpass_env(token: &Token) -> Vec<(String, String)> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "cicada".to_string());
    vec![
        ("GIT_ASKPASS".to_string(), exe),
        (ASKPASS_MARKER.to_string(), "1".to_string()),
        (ASKPASS_TOKEN.to_string(), token.expose().to_string()),
    ]
}

/// The username the forge expects for token auth.
pub fn username(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::Github => "x-access-token",
        ForgeKind::Gitlab => "oauth2",
    }
}

/// The HTTPS URL to push to, derived from `origin` **in memory**.
///
/// Token auth is HTTP Basic, so the push must be HTTPS. If `origin` is an SSH
/// URL, cicada derives the equivalent and leaves the configured remote untouched
/// — rewriting someone's remote as a side effect of a release would be a
/// surprising and durable change to their repository.
pub fn push_url(origin: &str, kind: ForgeKind) -> Option<String> {
    let user = username(kind);
    let s = origin.trim();

    // scp-like: git@host:org/repo(.git)
    if !s.contains("://")
        && let Some((prefix, path)) = s.split_once(':')
    {
        let host = prefix.rsplit('@').next()?;
        if host.is_empty() || path.is_empty() {
            return None;
        }
        return Some(format!("https://{user}@{host}/{}", path.trim_start_matches('/')));
    }

    let (scheme, rest) = s.split_once("://")?;
    // Strip any userinfo already present rather than stacking another on.
    let rest = rest.rsplit('@').next()?;
    let (host, path) = rest.split_once('/')?;
    match scheme {
        "ssh" | "git" | "https" | "http" => {
            Some(format!("https://{user}@{host}/{}", path.trim_start_matches('/')))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ssh_origin_becomes_https_with_the_forges_username() {
        assert_eq!(
            push_url("git@github.com:acme/app.git", ForgeKind::Github).as_deref(),
            Some("https://x-access-token@github.com/acme/app.git")
        );
        assert_eq!(
            push_url("ssh://git@gitlab.com/group/sub/proj.git", ForgeKind::Gitlab).as_deref(),
            Some("https://oauth2@gitlab.com/group/sub/proj.git")
        );
    }

    #[test]
    fn an_https_origin_keeps_its_host_and_gains_the_username() {
        assert_eq!(
            push_url("https://github.com/acme/app.git", ForgeKind::Github).as_deref(),
            Some("https://x-access-token@github.com/acme/app.git")
        );
        // A userinfo already in place is replaced, not stacked.
        assert_eq!(
            push_url("https://someone@ghe.example/acme/app", ForgeKind::Github).as_deref(),
            Some("https://x-access-token@ghe.example/acme/app")
        );
    }

    #[test]
    fn a_local_path_remote_has_no_https_equivalent() {
        assert_eq!(push_url("/srv/git/app.git", ForgeKind::Github), None);
        assert_eq!(push_url("file:///srv/git/app.git", ForgeKind::Github), None);
    }

    #[test]
    fn the_token_carries_no_formatting_impl_that_could_leak_it() {
        // Compile-time, really: if `Token` grows `Debug` or `Display`, a
        // `tracing::debug!(?token)` somewhere becomes a credential in a log.
        let t = Token("secret".into());
        assert_eq!(t.expose(), "secret");
        let env = askpass_env(&t);
        assert!(env.iter().any(|(k, v)| k == ASKPASS_TOKEN && v == "secret"));
        assert!(env.iter().any(|(k, _)| k == "GIT_ASKPASS"));
    }
}

//! The credential path, end to end (#18).
//!
//! Token-only, never persisted, never in `argv`. These drive the real binary,
//! because the askpass mechanism *is* the binary re-invoking itself and cannot be
//! observed from inside a single process.

use std::process::Command;

use assert_cmd::prelude::*;

fn cicada() -> Command {
    let mut c = Command::cargo_bin("cicada").unwrap();
    c.env("NO_COLOR", "1").env_remove("CICADA_TOKEN");
    c
}

#[test]
fn git_asking_for_a_password_gets_the_token_and_nothing_else() {
    // git invokes `$GIT_ASKPASS <prompt>`; cicada answers and exits before clap
    // ever sees that prompt as an argument.
    let out = cicada()
        .env("CICADA_INTERNAL_ASKPASS", "1")
        .env("CICADA_INTERNAL_ASKPASS_TOKEN", "s3cret")
        .arg("Password for 'https://x-access-token@github.com': ")
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "s3cret");
    assert!(String::from_utf8_lossy(&out.stderr).is_empty());
}

#[test]
fn the_marker_alone_does_not_turn_an_ordinary_run_into_a_credential_helper() {
    // Both variables are required, so a stray marker in someone's environment
    // cannot silently replace every cicada invocation with a token print.
    let out = cicada().env("CICADA_INTERNAL_ASKPASS", "1").arg("--help").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Release automation"), "{s}");
}

#[test]
fn the_token_never_appears_in_help_or_in_verbose_logging() {
    // `Token` deliberately has no `Debug`/`Display`, and the flag hides its env
    // value — a credential in a CI log is not recoverable once it is there.
    let out = cicada().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("CICADA_TOKEN"), "the variable is named: {help}");

    let out = cicada()
        .env("CICADA_TOKEN", "s3cret-value")
        .args(["plan", "-vvv"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!all.contains("s3cret-value"), "the token leaked:\n{all}");
}

#[test]
fn commands_that_need_no_token_do_not_ask_for_one() {
    // `plan`, `check` and the rehearsal modes are offline by design (§Goals 6),
    // and demanding a token would make `check` unusable on forked pull requests
    // where none exists.
    for args in [vec!["plan"], vec!["check"]] {
        let out = cicada().args(&args).current_dir(env!("CARGO_MANIFEST_DIR")).output().unwrap();
        let e = String::from_utf8_lossy(&out.stderr);
        assert!(!e.contains("no token"), "{args:?} asked for a token: {e}");
    }
}

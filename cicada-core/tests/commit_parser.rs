use cicada_core::commit::{ParseError, parse};
use cicada_core::RawCommit;

fn p(subject: &str) -> Result<cicada_core::ParsedCommit, ParseError> {
    parse(&RawCommit::new("a1b2c3d", subject, ""))
}
fn pb(subject: &str, body: &str) -> Result<cicada_core::ParsedCommit, ParseError> {
    parse(&RawCommit::new("a1b2c3d", subject, body))
}

#[test]
fn plain_type_and_description() {
    let c = p("feat: add login").unwrap();
    assert_eq!(c.kind, "feat");
    assert!(c.scopes.is_empty());
    assert!(!c.breaking);
    assert_eq!(c.description, "add login");
}

#[test]
fn single_scope() {
    let c = p("fix(api): handle nil").unwrap();
    assert_eq!(c.kind, "fix");
    assert_eq!(c.scopes, ["api"]);
    assert_eq!(c.description, "handle nil");
}

#[test]
fn multi_scope_is_ordered_and_trimmed() {
    // The extension the hand-written parser exists for (#11).
    let c = p("feat(api, worker ,gateway): add retry backoff").unwrap();
    assert_eq!(c.scopes, ["api", "worker", "gateway"]);
}

#[test]
fn bang_marks_breaking() {
    let c = p("feat(api)!: drop v1 endpoints").unwrap();
    assert!(c.breaking);
    assert_eq!(c.scopes, ["api"]);
    assert_eq!(c.description, "drop v1 endpoints");
}

#[test]
fn bang_without_scope() {
    assert!(p("feat!: drop v1").unwrap().breaking);
}

#[test]
fn breaking_change_footer_is_equivalent_to_bang() {
    let c = pb("fix: tighten defaults", "BREAKING CHANGE: retries are now on by default").unwrap();
    assert!(c.breaking);
    assert_eq!(c.footers.len(), 1);
    assert_eq!(c.footers[0].key, "BREAKING CHANGE");
    assert_eq!(c.footers[0].value, "retries are now on by default");
}

#[test]
fn hyphenated_breaking_change_footer_also_counts() {
    assert!(pb("fix: x", "BREAKING-CHANGE: y").unwrap().breaking);
}

#[test]
fn release_as_footer_is_parsed_and_ignored() {
    // #39: forced versions are out of v1. The footer must be neither honoured
    // nor rejected — a repo migrating from release-please has these in history.
    let c = pb("feat(api): rewrite storage", "Release-As: 2.0.0").unwrap();
    assert_eq!(c.footers, vec![cicada_core::Footer {
        key: "Release-As".into(),
        value: "2.0.0".into()
    }]);
    // Nothing on ParsedCommit expresses a forced version.
    assert!(!c.breaking);
}

#[test]
fn body_and_footers_are_split() {
    let c = pb(
        "feat(api): add caching",
        "Caches responses for 60s.\nTunable via CACHE_TTL.\n\nReviewed-by: someone\nRefs: #42",
    )
    .unwrap();
    assert_eq!(c.body, "Caches responses for 60s.\nTunable via CACHE_TTL.");
    assert_eq!(c.footers.len(), 2);
    assert_eq!(c.footers[0].key, "Reviewed-by");
    assert_eq!(c.footers[1].value, "#42");
}

#[test]
fn prose_containing_a_colon_stays_prose() {
    // A non-footer line ends the trailing run, so this body has no footers at all.
    let c = pb("fix: x", "We tried this: it did not work.").unwrap();
    assert!(c.footers.is_empty());
    assert_eq!(c.body, "We tried this: it did not work.");
}

#[test]
fn multiline_footer_value_is_joined() {
    let c = pb("fix: x", "BREAKING CHANGE: the flag is gone\n  and will not return").unwrap();
    assert!(c.breaking);
    assert_eq!(c.footers[0].value, "the flag is gone\n  and will not return");
}

#[test]
fn unknown_type_parses_fine() {
    // Unknown *types* are ignored later by bump computation, not rejected here.
    let c = p("wibble(api): something").unwrap();
    assert_eq!(c.kind, "wibble");
}

#[test]
fn non_conventional_subjects_are_rejected() {
    assert_eq!(p("update deps").unwrap_err(), ParseError::NotConventional);
    assert_eq!(p("").unwrap_err(), ParseError::NotConventional);
    assert_eq!(p(": no type").unwrap_err(), ParseError::NotConventional);
    // No space after the colon: keeps URLs and `note:see` out.
    assert_eq!(p("fix:no space").unwrap_err(), ParseError::NotConventional);
    // Unclosed scope.
    assert_eq!(p("feat(api: x").unwrap_err(), ParseError::NotConventional);
}

#[test]
fn empty_description_is_rejected() {
    assert_eq!(p("feat: ").unwrap_err(), ParseError::EmptyDescription);
    assert_eq!(p("feat:").unwrap_err(), ParseError::EmptyDescription);
}

#[test]
fn empty_scopes_are_rejected() {
    assert_eq!(p("feat(): x").unwrap_err(), ParseError::EmptyScope);
    assert_eq!(p("feat(api,): x").unwrap_err(), ParseError::EmptyScopeItem);
    assert_eq!(p("feat(,api): x").unwrap_err(), ParseError::EmptyScopeItem);
    assert_eq!(p("feat(api,,worker): x").unwrap_err(), ParseError::EmptyScopeItem);
}

#[test]
fn a_merge_commit_subject_is_not_conventional() {
    // GitHub's default merge message — J7 asserts a repo using commit_source =
    // "merges" on such a repo sees nothing releasable.
    assert_eq!(
        p("Merge pull request #41 from acme/feature/x").unwrap_err(),
        ParseError::NotConventional
    );
}

#[test]
fn release_commit_subject_parses_as_chore() {
    // cicada's own commits ride along in ranges and must be inert (#41).
    let c = p("chore(release): prepare release").unwrap();
    assert_eq!(c.kind, "chore");
    assert_eq!(c.scopes, ["release"]);
}

#[test]
fn trailing_whitespace_in_the_subject_is_ignored() {
    assert_eq!(p("feat: add login   ").unwrap().description, "add login");
}

// --- did-you-mean --------------------------------------------------------

use cicada_core::check::did_you_mean;

#[test]
fn transposed_scopes_are_suggested() {
    // The design's worked example (§`cicada check`). A transposition costs 2
    // under plain Levenshtein, which is over the threshold for a 3-character
    // name — so this only works with Damerau.
    assert_eq!(did_you_mean("aip", &["api", "platform"]), Some("api"));
    assert_eq!(did_you_mean("wroker", &["api", "worker", "api-chart"]), Some("worker"));
}

#[test]
fn single_character_slips_are_suggested() {
    assert_eq!(did_you_mean("ap", &["api"]), Some("api"));
    assert_eq!(did_you_mean("apii", &["api"]), Some("api"));
    assert_eq!(did_you_mean("apo", &["api"]), Some("api"));
}

#[test]
fn a_scope_with_no_near_match_gets_no_suggestion() {
    // A misleading suggestion is worse than none — the caller prints the full
    // list of valid scopes instead (monorepo Y2).
    assert_eq!(did_you_mean("database", &["api", "worker"]), None);
    assert_eq!(did_you_mean("", &["api"]), None);
}

#[test]
fn the_closest_candidate_wins() {
    assert_eq!(did_you_mean("apo", &["api", "app"]), Some("api"));
}

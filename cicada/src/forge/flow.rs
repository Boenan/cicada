//! The three forge protocols, separated from the CLI that sequences them.
//!
//! Each is a short, exact sequence of calls with a rule behind it, and each is
//! asserted by a scenario on the *ordered list of calls* rather than on its
//! return value. They live here rather than in `main` so a test can drive them
//! against [`super::fake::FakeForge`] and check what was and was not called —
//! "zero mutating calls" (D5a) is not expressible against a binary.

use super::{Forge, ForgeError, PrRequest, PullRequest, STALE_LABEL};

/// #34 — an empty plan closes any open release PR.
///
/// Leaving it open advertises a release that is no longer justified: the config
/// changed, a section's `bump` went to `none`, a package was removed. Returns the
/// number it closed, or `None` when there was nothing open.
///
/// **One read, and a write only when there is something to close.** On a quiet
/// repo this is the most-executed path in the whole tool (D5a).
pub fn close_open_pr(forge: &dyn Forge, head: &str) -> Result<Option<u64>, ForgeError> {
    let Some(pr) = forge.find_pull_request(head)? else { return Ok(None) };
    forge.close_pull_request(pr.number)?;
    Ok(Some(pr.number))
}

/// #33 layer 2 — make a release PR cicada could not refresh **unmergeable**.
///
/// Refusing to clobber the branch is only half the job. A stale release PR that
/// gets merged ships the intervening commits with no changelog entry and no
/// influence on the version number: the code ships, the semver contract breaks,
/// and a `feat` goes out under a patch bump where nobody sees it.
///
/// Draft state is the layer that needs no repo configuration, because the forge
/// itself refuses to merge a draft. The label and the comment are for the human
/// who has to work out what happened.
///
/// The order matters: **draft first**. If the run dies after commenting but
/// before drafting, the PR is still mergeable and the comment is the only thing
/// standing between a reader and a broken release.
pub fn mark_stale(
    forge: &dyn Forge,
    head: &str,
    comment: &str,
) -> Result<Option<u64>, ForgeError> {
    let Some(pr) = forge.find_pull_request(head)? else { return Ok(None) };
    forge.set_draft(&pr, true)?;
    forge.set_labels(pr.number, &[STALE_LABEL.to_string()])?;
    forge.comment(pr.number, comment)?;
    Ok(Some(pr.number))
}

/// Open the release PR, or refresh the one that is already open (#77).
///
/// **Keyed on the head branch, never the title.** That is what makes the upsert
/// idempotent by construction, so two concurrent Phase A runs cannot open two
/// pull requests (D6), and what lets `release_pr_title` be the same plain string
/// on every release (#86).
///
/// It searches open PRs only, so a closed release PR is never reopened: a new one
/// is created instead, with a different number (D5c). Idempotency rests on the
/// branch name, not on PR identity.
///
/// A PR that was drafted by [`mark_stale`] and is now refreshable comes back out
/// of draft and loses the label (D3) — recovery has to be automatic, or every
/// refusal would need a human to undo it by hand.
pub fn upsert(forge: &dyn Forge, req: &PrRequest) -> Result<(PullRequest, bool), ForgeError> {
    match forge.find_pull_request(&req.head)? {
        Some(existing) => {
            let updated = forge.update_pull_request(existing.number, req)?;
            if existing.draft {
                forge.set_draft(&existing, false)?;
                forge.set_labels(existing.number, &[])?;
            }
            Ok((updated, false))
        }
        None => Ok((forge.create_pull_request(req)?, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::fake::{Call, FakeForge};

    fn req() -> PrRequest {
        PrRequest {
            head: "cicada/main".into(),
            base: "main".into(),
            title: "chore(release): prepare release".into(),
            body: "notes".into(),
        }
    }

    #[test]
    fn d5a_an_empty_plan_with_no_open_pr_makes_one_read_and_no_writes() {
        // The most-executed path in the tool on a quiet repo.
        let f = FakeForge::new();
        assert_eq!(close_open_pr(&f, "cicada/main").unwrap(), None);
        assert_eq!(f.call_names(), vec!["find_pull_request"]);
        assert!(f.mutating_calls().is_empty());
    }

    #[test]
    fn d5b_an_empty_plan_closes_the_pr_that_is_open() {
        let f = FakeForge::new().with_open_pr("cicada/main");
        assert_eq!(close_open_pr(&f, "cicada/main").unwrap(), Some(1));
        assert_eq!(f.call_names(), vec!["find_pull_request", "close_pull_request"]);
        assert!(f.open_pr("cicada/main").is_none());
    }

    #[test]
    fn d5c_a_closed_pr_is_never_reopened_a_new_one_is_created() {
        // `find_pull_request` searches open PRs only (#77), so the closed one is
        // invisible and idempotency rests on the branch name rather than on PR
        // identity.
        let f = FakeForge::new().with_open_pr("cicada/main");
        close_open_pr(&f, "cicada/main").unwrap();

        let (pr, created) = upsert(&f, &req()).unwrap();
        assert!(created);
        assert_ne!(pr.number, 1, "a different number, not a reopen");
        assert!(f.call_names().contains(&"create_pull_request"));
        assert!(!f.call_names().contains(&"update_pull_request"));
    }

    #[test]
    fn the_upsert_creates_once_and_updates_thereafter() {
        let f = FakeForge::new();
        let (first, created) = upsert(&f, &req()).unwrap();
        assert!(created);

        let (second, created) = upsert(&f, &req()).unwrap();
        assert!(!created);
        assert_eq!(second.number, first.number, "same PR, found by head branch");
        assert_eq!(
            f.call_names(),
            vec![
                "find_pull_request",
                "create_pull_request",
                "find_pull_request",
                "update_pull_request",
            ]
        );
    }

    #[test]
    fn d6_two_overlapping_runs_leave_exactly_one_pull_request() {
        // Keying on the head branch is what makes this true by construction: the
        // second run finds the first run's PR and updates it.
        let f = FakeForge::new();
        upsert(&f, &req()).unwrap();
        upsert(&f, &req()).unwrap();
        upsert(&f, &req()).unwrap();
        assert_eq!(f.call_names().iter().filter(|c| **c == "create_pull_request").count(), 1);
    }

    #[test]
    fn d2_marking_stale_drafts_before_it_comments() {
        let f = FakeForge::new().with_open_pr("cicada/main");
        assert_eq!(mark_stale(&f, "cicada/main", "why").unwrap(), Some(1));

        // Draft first: a run that dies after commenting but before drafting leaves
        // a mergeable PR and a comment nobody has to obey.
        assert_eq!(
            f.call_names(),
            vec!["find_pull_request", "set_draft", "set_labels", "comment"]
        );
        assert!(f.open_pr("cicada/main").unwrap().draft);
        assert!(f.calls().contains(&Call::SetLabels {
            number: 1,
            labels: vec![STALE_LABEL.to_string()]
        }));
        assert!(f.calls().contains(&Call::SetDraft { number: 1, draft: true }));
    }

    #[test]
    fn a_refusal_with_no_pr_open_yet_touches_nothing() {
        // The branch can be stale before any PR exists — a human pushed to
        // `cicada/main` directly. There is nothing to draft, and inventing a PR
        // to draft it would be worse than silence.
        let f = FakeForge::new();
        assert_eq!(mark_stale(&f, "cicada/main", "why").unwrap(), None);
        assert!(f.mutating_calls().is_empty());
    }

    #[test]
    fn d3_recovering_takes_the_pr_back_out_of_draft_and_clears_the_label() {
        let f = FakeForge::new().with_open_pr("cicada/main");
        mark_stale(&f, "cicada/main", "why").unwrap();
        assert!(f.open_pr("cicada/main").unwrap().draft);

        let (_, created) = upsert(&f, &req()).unwrap();
        assert!(!created);
        assert!(!f.open_pr("cicada/main").unwrap().draft, "recovery must be automatic");
        assert!(f.calls().contains(&Call::SetDraft { number: 1, draft: false }));
        assert!(f.calls().contains(&Call::SetLabels { number: 1, labels: vec![] }));
    }

    #[test]
    fn a_pr_that_was_never_drafted_is_not_un_drafted_on_every_run() {
        // Otherwise every ordinary refresh would make two pointless API calls.
        let f = FakeForge::new().with_open_pr("cicada/main");
        upsert(&f, &req()).unwrap();
        assert_eq!(f.call_names(), vec!["find_pull_request", "update_pull_request"]);
    }

    #[test]
    fn a_forge_failure_is_returned_rather_than_swallowed() {
        let f = FakeForge::new().failing("503 from the forge");
        assert!(close_open_pr(&f, "cicada/main").is_err());
        assert!(upsert(&f, &req()).is_err());
        assert!(mark_stale(&f, "cicada/main", "why").is_err());
    }
}

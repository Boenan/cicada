//! A recording `Forge`, for tests.
//!
//! The scenario files assert on **the ordered list of `Forge` calls made against
//! a recording fake** — that is one of the five surfaces every scenario pins, and
//! several of them (D5a's "zero mutating calls", C3's "re-running changes
//! nothing") are assertions about calls that did *not* happen. A mock that only
//! returned canned values could not express those.
//!
//! It is a normal `pub` module rather than `#[cfg(test)]` because integration
//! tests are separate crates and cannot see a test-only module.

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::{Forge, ForgeError, PrRequest, PullRequest, Release, ReleaseRequest};

/// One recorded call. Mutating calls and read calls are both recorded, because
/// "exactly one read and no writes" is a real assertion (D5a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    FindPullRequest { head: String },
    CreatePullRequest(PrRequest),
    UpdatePullRequest { number: u64, req: PrRequest },
    ClosePullRequest { number: u64 },
    SetDraft { number: u64, draft: bool },
    Comment { number: u64, body: String },
    SetLabels { number: u64, labels: Vec<String> },
    FindRelease { tag: String },
    CreateRelease(ReleaseRequest),
}

impl Call {
    /// Does this call change anything on the forge?
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Call::FindPullRequest { .. } | Call::FindRelease { .. })
    }

    /// A short name, for asserting on a call *sequence* without spelling out
    /// every field.
    pub fn name(&self) -> &'static str {
        match self {
            Call::FindPullRequest { .. } => "find_pull_request",
            Call::CreatePullRequest(_) => "create_pull_request",
            Call::UpdatePullRequest { .. } => "update_pull_request",
            Call::ClosePullRequest { .. } => "close_pull_request",
            Call::SetDraft { .. } => "set_draft",
            Call::Comment { .. } => "comment",
            Call::SetLabels { .. } => "set_labels",
            Call::FindRelease { .. } => "find_release",
            Call::CreateRelease(_) => "create_release",
        }
    }
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    /// Open pull requests, keyed by head branch — the same key the real upsert
    /// uses (#77), so the fake cannot accidentally be more forgiving.
    open: BTreeMap<String, PullRequest>,
    releases: BTreeMap<String, Release>,
    next_number: u64,
    /// When set, every call fails with this. For asserting that a forge failure
    /// leaves local state alone.
    fail_with: Option<String>,
}

#[derive(Default)]
pub struct FakeForge {
    state: RefCell<State>,
}

impl FakeForge {
    pub fn new() -> Self {
        Self { state: RefCell::new(State { next_number: 1, ..State::default() }) }
    }

    /// Seed an already-open pull request for `head`.
    pub fn with_open_pr(self, head: &str) -> Self {
        {
            let mut s = self.state.borrow_mut();
            let number = s.next_number;
            s.next_number += 1;
            s.open.insert(
                head.to_string(),
                PullRequest {
                    number,
                    url: format!("https://forge.invalid/pull/{number}"),
                    draft: false,
                    node_id: Some(format!("PR_{number}")),
                },
            );
        }
        self
    }

    pub fn failing(self, message: &str) -> Self {
        self.state.borrow_mut().fail_with = Some(message.to_string());
        self
    }

    /// Seed a release object without recording a call.
    ///
    /// For the re-run assertions (C3, C4): the state a *previous* run left behind
    /// is not a call this run made, and counting it as one would make "zero
    /// mutating calls" impossible to assert.
    pub fn create_release_for(&self, tag: &str) {
        let mut s = self.state.borrow_mut();
        let id = s.next_number;
        s.next_number += 1;
        s.releases.insert(
            tag.to_string(),
            Release { id, url: format!("https://forge.invalid/releases/{id}"), tag: tag.to_string() },
        );
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state.borrow().calls.clone()
    }

    /// The call sequence by name — what a scenario's "forge" surface asserts.
    pub fn call_names(&self) -> Vec<&'static str> {
        self.state.borrow().calls.iter().map(Call::name).collect()
    }

    pub fn mutating_calls(&self) -> Vec<Call> {
        self.state.borrow().calls.iter().filter(|c| c.is_mutating()).cloned().collect()
    }

    pub fn open_pr(&self, head: &str) -> Option<PullRequest> {
        self.state.borrow().open.get(head).cloned()
    }

    fn record(&self, call: Call) -> Result<(), ForgeError> {
        let mut s = self.state.borrow_mut();
        s.calls.push(call);
        match &s.fail_with {
            Some(m) => Err(ForgeError::Http { status: 500, message: m.clone() }),
            None => Ok(()),
        }
    }
}

impl Forge for FakeForge {
    fn find_pull_request(&self, head: &str) -> Result<Option<PullRequest>, ForgeError> {
        self.record(Call::FindPullRequest { head: head.to_string() })?;
        Ok(self.state.borrow().open.get(head).cloned())
    }

    fn create_pull_request(&self, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        self.record(Call::CreatePullRequest(req.clone()))?;
        let mut s = self.state.borrow_mut();
        let number = s.next_number;
        s.next_number += 1;
        let pr = PullRequest {
            number,
            url: format!("https://forge.invalid/pull/{number}"),
            draft: false,
            node_id: Some(format!("PR_{number}")),
        };
        s.open.insert(req.head.clone(), pr.clone());
        Ok(pr)
    }

    fn update_pull_request(&self, number: u64, req: &PrRequest) -> Result<PullRequest, ForgeError> {
        self.record(Call::UpdatePullRequest { number, req: req.clone() })?;
        Ok(self
            .state
            .borrow()
            .open
            .get(&req.head)
            .cloned()
            .unwrap_or(PullRequest { number, url: String::new(), draft: false, node_id: None }))
    }

    fn close_pull_request(&self, number: u64) -> Result<(), ForgeError> {
        self.record(Call::ClosePullRequest { number })?;
        self.state.borrow_mut().open.retain(|_, pr| pr.number != number);
        Ok(())
    }

    fn set_draft(&self, pr: &PullRequest, draft: bool) -> Result<(), ForgeError> {
        self.record(Call::SetDraft { number: pr.number, draft })?;
        for open in self.state.borrow_mut().open.values_mut() {
            if open.number == pr.number {
                open.draft = draft;
            }
        }
        Ok(())
    }

    fn comment(&self, number: u64, body: &str) -> Result<(), ForgeError> {
        self.record(Call::Comment { number, body: body.to_string() })
    }

    fn set_labels(&self, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        self.record(Call::SetLabels { number, labels: labels.to_vec() })
    }

    fn find_release(&self, tag: &str) -> Result<Option<Release>, ForgeError> {
        self.record(Call::FindRelease { tag: tag.to_string() })?;
        Ok(self.state.borrow().releases.get(tag).cloned())
    }

    fn create_release(&self, req: &ReleaseRequest) -> Result<Release, ForgeError> {
        self.record(Call::CreateRelease(req.clone()))?;
        let mut s = self.state.borrow_mut();
        let id = s.next_number;
        s.next_number += 1;
        let rel = Release {
            id,
            url: format!("https://forge.invalid/releases/{id}"),
            tag: req.tag.clone(),
        };
        s.releases.insert(req.tag.clone(), rel.clone());
        Ok(rel)
    }
}

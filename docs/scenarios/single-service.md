# cicada — release scenarios: single-service repos

Every outcome cicada can produce in a repo that ships one service, written so each entry is directly
executable as a test. Companion to `docs/design.md`; decision numbers (`#n`) reference it.

Two purposes, in this order:

1. **Coverage check on the design.** A scenario whose expected outcome cannot be derived from
   `docs/design.md` is a hole in the design, not a hole in the test suite. Those are marked **OPEN** and
   collected in §Deltas.
2. **The acceptance suite.** These become fixture tests as M0–M5 land.

The monorepo scenarios live in `docs/scenarios/monorepo.md` and build on these.

---

## How to read a scenario

Every scenario pins **five surfaces**. A surface a scenario does not touch is asserted as untouched —
"no branch created" is an assertion, not an omission.

| Surface | Asserted by |
|---|---|
| **plan** | `cicada plan --json` — package, `from`, `to`, bump kind, reason |
| **files** | byte-level diff of the working tree against the fixture |
| **git** | branches, commits, committer identity, tags |
| **forge** | the ordered list of `Forge` calls made against a recording fake |
| **exit** | process exit code (#68) |

### Fixture conventions

- Each scenario builds a throwaway git repo from a commit script. Commits are `c1`, `c2`, …; merge commits
  `m1`, `m2`.
- The forge is a recording fake implementing the nine-method trait (#53). **No scenario in this file makes
  a network call.** Where a scenario names real forge behaviour (draft semantics, prerelease flags) the fake
  asserts the call, not the remote effect.
- Unless stated, HEAD is on `main`, `release_branches = ["main"]`, and the command is `cicada run`.
- **Terminology** (#27): a **release branch** is one of `release_branches` — a branch cicada releases *from*.
  The **release PR branch** is cicada's own ephemeral head branch, named by `release_pr_branch`.
- "byte-identical" means exactly that: the assertion is on bytes, not on a re-parse. Comment and formatting
  preservation is the whole point of #49.

---

## The two shapes

Per Q1: **cicada does not assume an umbrella.** `[umbrella]` is opt-in, declared by the developer when they
actually want an aggregate version. Neither shape below declares one. An app and its Helm chart in one repo
are **two packages with independent version lines** (#14), not an umbrella and a child.

### Shape 1 — one package

```toml
[settings]
release_branches  = ["main"]
default_scope     = "app"
committer         = "cicada[bot] <cicada@users.noreply.github.com>"
commit_source     = "commits"
release_pr_branch = "cicada/{branch}"
release_pr_title  = "chore(release): prepare release"

[forge]
kind = "github"
repo = "acme/app"

[changelog]
sections = [
  { type = "feat",  title = "Features",      bump = "minor" },
  { type = "fix",   title = "Bug Fixes",     bump = "patch" },
  { type = "perf",  title = "Performance",   bump = "patch" },
  { type = "docs",  title = "Documentation", bump = "none"  },
  { type = "chore", title = "Chores",        bump = "none", hidden = true },
]

[packages.app]
path       = "."
tag_format = "v{version}"
changelog  = "CHANGELOG.md"

[[packages.app.updaters]]
kind = "cargo"
path = "Cargo.toml"
```

No `[umbrella]`. `default_scope = "app"` means unscoped commits route to the only package (#38).

**Scopes (Q2).** Both `feat:` and `feat(app):` are valid and produce identical outcomes — there is only one
possible destination. Unscoped is the recommended style. A scope naming anything else (`feat(parser):`) is a
hard error with a did-you-mean (#11); free-form subsystem scopes are **not** supported. This is a deliberate
consequence of keeping one code path (#16) — see B12/B13.

### Shape 2 — app plus its chart

Shape 1 plus a second package. Still no umbrella.

```toml
[packages.app-chart]
path       = "deploy/chart"
tag_format = "chart-v{version}"
changelog  = "deploy/chart/CHANGELOG.md"

[[packages.app-chart.updaters]]
kind            = "helm"
path            = "deploy/chart/Chart.yaml"
app_version     = { track = "app" }
on_tracked_bump = "patch"
```

Two version lines, two tag formats, two changelogs, two release objects, **one** release manifest per run
(#42). `default_scope` stays `app`, so an unscoped commit is an app change. Scopes become load-bearing here:
`feat(app-chart):` is the only way to express a chart-only change.

---

## A · Bootstrap — the repo has never released

"Never released" is an **absent entry** in `.cicada-versions.toml` (#4). There is no sentinel version. On a
first release the package is treated as `0.0.0` and strict semver applies (#37); the range is all reachable
history.

| id | given | commits in range | expected `to` | notes |
|---|---|---|---|---|
| A1 | no versions file | `feat: add login` | `0.1.0` | minor from `0.0.0` |
| A2 | no versions file | `fix: handle empty body` | `0.0.1` | patch from `0.0.0` |
| A3 | no versions file | `feat!: initial public API` | `1.0.0` | breaking → major, even on a first release |
| A4 | `initial_version = "1.0.0"` | `fix: typo` | `1.0.0` | override wins over the table entirely |
| A5 | no versions file | `chore: tidy` + `docs: readme` | — | **no release** |
| A6 | no versions file | `wibble: something` | — | unknown type ignored (not an error), no release |
| A9 | no versions file, `Cargo.toml` at `0.1.0` | `fix: …` | `0.0.1` | the manifest is an **output**, never an input (#75) — cicada writes it *downwards* |

**A1 in full** — the reference scenario for the format:

- **given** no `.cicada-versions.toml`; no tags; `Cargo.toml` at `0.0.0`; no `CHANGELOG.md`
- **when** `c1 feat: add login`, then `cicada release-pr --no-push`
- **then**
  - plan · `app: from=null to=0.1.0 bump=minor reason=first-release`
  - files · `Cargo.toml` → `0.1.0` (all other bytes identical) · `CHANGELOG.md` **created** with header and
    a `## 0.1.0` entry under *Features* · `.cicada-versions.toml` **created** with
    `[packages.app] version = "0.1.0"` — a version and nothing else (#1)
  - git · `cicada/main` created at `c1` carrying exactly one commit, committer `cicada[bot]`
  - forge · none (`--no-push`)
  - exit · `0`

**A5 in full** — the "nothing to release" path, which is a *success*:

- **then** plan empty · **no files written** · **no branch created** · **no forge calls** · exit `0`

**A7 · Phase B on a virgin repo.** `cicada run` executes Phase B first (#35). With no versions-file entry
there is no version to derive a tag from, so Phase B **skips the package entirely** — no tag, no release, no
error — and Phase A then produces the A1 outcome. Assert: zero `git tag` invocations and zero forge calls
attributable to Phase B. *(Resolves Q5's first state; see J2 for the second.)*

**A8 · first release covers all history.** A repo with 40 pre-cicada commits, then `c41 feat: …`. The range
is unanchored, so the `0.1.0` changelog entry contains every conventional commit among all 41 — not just
`c41`. Asserted explicitly because it surprises adopters, and because it is the behaviour that makes J2's
open question matter.

---

## B · Version arithmetic

Established state for every row: `.cicada-versions.toml` has `app = { version = "1.4.2" }`, and tag `v1.4.2`
exists and is reachable from HEAD.

| id | given version | commits in range | expected | rule |
|---|---|---|---|---|
| B1 | `1.4.2` | `fix:` | `1.4.3` | section `bump = "patch"` |
| B2 | `1.4.2` | `feat:` + `fix:` | `1.5.0` | highest wins |
| B3 | `1.4.2` | `feat!:` | `2.0.0` | `!` overrides the section bump |
| B4 | `1.4.2` | `fix:` with `BREAKING CHANGE:` footer | `2.0.0` | footer is equivalent to `!` |
| B5 | `0.3.0` | `feat!:` | `0.4.0` | `pre_major = "minor"` (default) — never escapes 0.x |
| B6 | `0.3.0` | `feat!:` , `pre_major = "major"` | `1.0.0` | opt-in escape |
| B7 | `1.4.2` | `perf:` | `1.4.3` | `perf` is `bump = "patch"` |
| B8 | `1.4.2` | `feat:` + `chore(deps):` | `1.5.0` | chore contributes nothing and is `hidden` in the entry |
| B9 | `1.4.2` | `chore(deps):` only | — | **no release** (#36) — the dependency-bump case |
| B10 | `1.4.2` | `docs:` only | — | no release |
| B11 | `1.4.2` | `feat:` , branch matches `maintenance_branches` | **error**, exit `4` | the guard refuses rather than clamping (#28) |
| B14 | `1.4.2` | `feat!:` , branch matches `maintenance_branches` | **error**, exit `4` | the case a clamp handled worst — a breaking change on a patch upgrade |
| B15 | `1.4.2` | `fix:` , branch matches `maintenance_branches` | `1.4.3` | the guard is silent in the normal case |
| B12 | `1.4.2` | `feat(app):` | `1.5.0` | explicit scope ≡ unscoped in shape 1 (Q2) |
| B13 | `1.4.2` | `feat(parser):` | **error** | unknown scope is fatal (#11); `check` exits `3` with *did you mean 'app'?* |

**B11/B14 detail.** The error must name the offending commits, not just the computed bump — on a cherry-pick
batch of twenty, "a `feat:` is present" is not actionable. Assert the message lists each commit that
contributed above patch. B15 is the assertion that matters most in practice: a maintenance branch doing its
ordinary job must never see this error.

**B17 · there is no way through the guard** (#39). Assert the negative, because the design used to have an
escape and no longer does: with a `feat:` in the range on a maintenance line, no flag, footer, or config key
releases it. Hand-editing `.cicada-versions.toml` to `1.4.8` does not work either — Phase B's staleness guard
recomputes `1.5.0` and refuses the lower recorded version (J4b). The line stays blocked until the commit
leaves the range. This is the assertion that stops the escape being quietly reintroduced.

**B13 detail.** The failure surfaces in two places with different codes: `cicada check` reports it as a
finding (exit `3`, with the suggestion), while `cicada plan`/`release-pr` hard-error on it (exit `1`). Both
name the scope and the configured package list.

---

## C · The round trip, and idempotency

**C1 · first push produces a PR.**
- given B2's state; `cicada run` on `main`
- then Phase B no-op (tag `v1.4.2` exists and is reachable) → Phase A writes the edits, force-pushes
  `cicada/main`, and calls `create_pull_request` with `base = "main"`, `head = "cicada/main"`,
  title from `release_pr_title`, body = the rendered notes
- forge · `find_pull_request("cicada/main")` → `None`, then `create_pull_request`
- exit · `0`

**C2 · merge, then tag and release.**
- given C1's PR merged as `m1`; `cicada run` on `main`
- then Phase B reads `version = "1.5.0"`, derives `v1.5.0`, finds it missing → runs the staleness guard
  (#78: replan from `v1.4.2..HEAD`, expected `1.5.0`, recorded `1.5.0`, not lower → pass) → creates and
  pushes the tag at `m1`; `find_release("v1.5.0")` → `None` → `create_release` with notes and the embedded
  manifest (#57). Phase A then resolves the anchor to `v1.5.0`, finds an empty range, and makes **no** forge
  calls.
- files · `target/cicada/release-manifest.json` written (gitignored, never committed)
- exit · `0`

**C3 · re-run is a total no-op.** Running C2 again: tag exists and is reachable → "already tagged"; release
exists → nothing. Assert **zero** writes, zero tag pushes, zero mutating forge calls. Two read calls
(`find_release`, `find_pull_request`) are permitted. exit `0`.

**C4 · self-healing release object (#56).** Tag `v1.5.0` exists and is reachable, but no release object —
the shape produced by a `create_release` that 500s after the tag push. Phase B creates **only** the release.
Assert no tag push. This is why tag and release reconcile independently; skipping the package on
"tag exists" would strand the release forever.

**C5 · tag deleted after main advanced.** Tag `v1.5.0` deleted by a human; `c6 feat: …` has since landed.
Phase B is about to recreate the tag, but placing it on HEAD would sweep in unreleased work. The staleness
guard fires first: replanning `v1.4.2..HEAD` now yields `2.0.0`, which is higher than the recorded `1.5.0`
→ exit `4`, no tag, no release. Assert the error names both versions.

Note the guard is not a general tag-placement check — it catches this case because the extra commits are
*releasable*. `c6 docs: …` instead would replan to `1.5.0`, pass the guard, and place the tag on HEAD. That
is accepted: a tag one commit late over a docs change has no version consequence, and detecting it would
require exactly the stored SHA that was removed.

**C6 · squash-merge.** The release PR squashes to a single `chore(release): …` commit on `main`. Outcome
identical to C2 — the tag lands on the squashed commit. Asserted because #9's independence from merge
strategy is a headline claim.

**C7 · rebase-merge.** Same assertion as C6.

**C9 · auto-merged release PR (#92).** The PR from C1 is merged by the forge's auto-merge rather than by a
human. Assert Phase B's behaviour is **byte-identical to C2** — it reconciles a tag against HEAD and never
asks who merged, or whether a PR existed at all. This is the assertion that makes "cicada always opens a PR"
compatible with continuous deployment: the gate is the repo's, the artifact is cicada's. Assert also that no
forge call inspects merge authorship, which would reintroduce the forge state #9 removed.

**C8 · anchor missing between merge and Phase B.** After the merge at C2 but *before* Phase B has run, a
developer runs `cicada plan` locally. The versions file says `1.5.0`; tag `v1.5.0` does not exist yet →
**hard error**, exit `1`, naming the tag and asking whether `cicada release` has run on this branch. There
is no SHA fallback (#2). `cicada run` never hits this, because Phase B precedes Phase A (#35).

---

## D · Release-PR lifecycle

**D1 · the branch is replaced, not updated.** PR open proposing `1.4.3` from a `fix:`. `c5 feat: …` lands on
`main`. Next run: `cicada/main` is reset to `main`'s tip and force-pushed with one fresh commit;
the **same** PR (found by head branch) is updated; the proposed version is now `1.5.0`. Assert the release
branch contains exactly one commit and `R1` is unreachable from it — nothing is merged or rebased.

**D2 · human commit on the release PR branch → refuse (#32).** Branch tip's committer is not
`cicada[bot]`. cicada does **not** force-push. It calls `set_draft(true)`, `set_labels(["cicada:stale"])`,
and `comment(<why>)`, then exits `4`. Assert the branch is byte-identical to before the run and the versions
file upstream is unchanged.

**D3 · recovery.** The human commit is removed (branch reset to cicada's commit, or deleted). Next run
refreshes normally, calls `set_draft(false)`, clears the label, exits `0`.

**D4 · stale PR un-drafted and merged (#33 layer 3).** PR proposes `1.4.3` from a `fix:`. Someone pushes
`c9 feat!: …` to `main`, un-drafts D2's PR, and merges it. Phase B is about to create `v1.4.3`, so the
staleness guard runs: replanning `v1.4.2..HEAD` yields `2.0.0`, the recorded version `1.4.3` is lower →
hard error naming both, exit `4`. No tag, no release. Main is left merged-but-untagged, which is the
deliberate trade: a visible unreleased state beats an invisible mis-versioned release.

**D4b · the guard is one-sided.** Same setup, but the recorded version is *higher* than the recomputation —
a hand-edited versions file naming `3.0.0`. The guard passes: over-bumping is a supported workflow,
under-bumping is the harm (#33). Assert no error.

**D4c · what the guard does not catch.** PR proposes `1.5.0` from a `feat:`; a stale `fix:` lands before the
merge. Replanning still yields `1.5.0`, so the guard passes and the fix ships with no changelog entry. The
version is correct, the notes are one line short. Accepted: catching it would require recording the commit
the changelog covers, which is the state #2 deliberately removed. Layers 1 and 2 are what prevent it.

**D5d · a scheduled run with nothing to release.** `cicada run` fired by cron rather than by a push, on a range
holding only `chore(deps):` commits. Outcome is identical to D5a: plan empty, exit `0`, nothing released.
Assert cicada makes no distinction between a cron-triggered and a push-triggered run — the gate is the branch
(#26), never the event — and assert that no configuration exists that would make this range release anyway.
Cadence releases are a permanent non-goal (§Non-goals), and this is the assertion that records it as behaviour
rather than as an omission someone later "fixes".

**D5a · empty plan, no PR open — the common case.** A `docs:` commit lands on a repo with no open release
PR. Plan empty → exit `0`. Assert **zero** forge calls, including `find_pull_request`… *or* exactly one, if
the close-check is unconditional. Pin it: cicada calls `find_pull_request` once and makes no mutating call.
On a quiet repo this is the most-executed path in the whole tool.

**D5b · empty plan, PR open → close it (#34).** Constructed by editing `cicada.toml`: the range holds only
`perf:` commits and a PR proposing `1.4.3` is open; the config is changed to `{ type = "perf", bump = "none" }`.
Next run: plan empty, `close_pull_request` called, exit `0`. *(This replaces the design's revert-based
example, which does not hold — see future-work §11.)*

**D5c · the config edit is reverted.** The `perf` section goes back to `bump = "patch"`. Next run must
produce a PR again. `find_pull_request` searches **open PRs only**, so the closed one is never reopened and
a new PR is created. Assert the old PR stays closed and the new one has a different number. Idempotency
comes from the head branch name, not from PR identity.

**D6 · concurrency.** Two Phase A runs overlap. Because the upsert keys on the head branch name and never on
the title, exactly one PR exists at the end. The force-push race is self-correcting: whichever run lands
second leaves the branch as a pure function of `main`'s tip at that moment. Assert one PR; the shipped CI
templates still set `concurrency:` / `resource_group` to avoid the wasted work.

**D7 · PR presentation.** Title renders `release_pr_title`; body carries the same notes the changelog entry
will contain (#43), rendered from the single notes template.

---

## E · Human-stated versions

Forced versions are **out of v1 scope** (#39): neither the `Release-As:` footer nor `--release-as` exists.
The group is kept, and kept short, because the assertions that remain are negative ones — a removed feature
that nobody asserts the absence of grows back.

| id | given | input | expected |
|---|---|---|---|
| E1 | `1.4.2`, computed `1.5.0` | `feat: …` with a `Release-As: 2.0.0` footer | `1.5.0` — the footer is **ignored**, not an error; it is an unknown footer like any other |
| E2 | `1.4.2` | `--release-as app=2.0.0` | exit `2` — unknown flag (clap) |
| E3 | `1.4.2` | hand-edited versions file → `2.0.0`, pushed | `2.0.0` tagged by Phase B; the one way a human states a version (J4) |
| E4 | `1.4.2` | hand-edited versions file → `1.0.0` | **hard error**, exit `4` — the staleness guard, not a separate rule (J4b) |
| E5 | `1.4.2` | hand-edited versions file → `1.4.2` | no-op; the tag already exists and is reachable, so Phase B reports "already tagged" |

**E1 is the assertion with teeth.** A repo migrating from release-please will have `Release-As:` footers in
its history, and a range containing one must plan exactly as if it were absent. Silently honouring it would
be the worst outcome; erroring on it would make adoption fail on historical commits. Ignoring it is the only
behaviour that is safe in both directions — and it falls out of the parser reading footers generically and
acting on `BREAKING CHANGE:` alone (§Commit parsing).

**Monotonicity now has one subject** (#76). The only human-stated version left is a hand-edited
`.cicada-versions.toml`, and it must be strictly greater than the number it replaces. E4 asserts that this is
enforced by the staleness guard's one-sided comparison rather than by a rule of its own — a lower recorded
version is below the recomputation, which is already an error. Nothing it forbids could be correct: a lower
value puts `## 1.0.0` above `## 1.4.2` in the changelog and resolves consumers' semver ranges backwards.

---

## F · Prereleases

Prerelease is **branch-level** (#59). The versions file gains a `prerelease` field; `version` keeps its exact
meaning — *last stable release* — so anchor semantics are untouched (#60).

**F1 · first rc.**
- given branch `prerelease-v1.5-rc`, matched by `prerelease_branches = ["prerelease-v*-rc"]` with
  `prerelease_identifier = "rc"`; `version = "1.4.2"`, tag
  `v1.4.2` reachable; `c1 feat: …`
- then
  - plan · `app: from=1.4.2 to=1.5.0-rc.1`
  - files · `Cargo.toml` → `1.5.0-rc.1` · versions file → `version = "1.4.2"` (**unchanged**),
    `prerelease = "1.5.0-rc.1"` · **`CHANGELOG.md` byte-identical** (#62)
  - exit · `0`

**F2 · counter increments.** `c2 fix: …` lands. Computed stable base is still `1.5.0` → `1.5.0-rc.2` (#63).

**F3 · base change resets the counter.** `c3 feat!: …` lands. Base moves `1.5.0` → `2.0.0` → `2.0.0-rc.1`,
not `2.0.0-rc.3`.

**F4 · Phase B on `prerelease-v1.5-rc`.** Reconciles the **prerelease** tag (`v1.5.0-rc.1`) because the
field is present. Release object created with GitHub's `prerelease: true`; the GitLab fake asserts the
name-marker fallback instead. Reachability and collision checks apply unchanged.

**F5 · promotion to stable.** `prerelease-v1.5-rc` merges into `main`, whose config has no `prerelease`. The range is
`1.4.2..HEAD` — every release computes from the last **stable** anchor (#61) — so:
  - plan · `1.4.2 → 1.5.0`
  - files · `CHANGELOG.md` gains **one** entry containing `c1`, `c2`, `c3`… everything since `1.4.2`, with no
    rc entries and no duplication · versions file → `version = "1.5.0"`, **`prerelease` key deleted**
  - This is the scenario the whole `version`/`prerelease` split exists to make work. Assert the changelog has
    no `-rc.` string anywhere.

**F6 · ad-hoc.** `cicada release-pr --prerelease rc` on `main` with no config stanza → same as F1.

**F7 · notes are still visible.** The rc's PR body and forge release notes are cumulative since `1.4.2`,
even though no changelog file was written. Reviewers and testers see the content; the file stays linear.

**F8 · `plan` says so.** `cicada plan` on `prerelease-v1.5-rc` reports the rc version and states that
`CHANGELOG.md` will not be written. Asserted because a silently skipped file write reads as a bug.

### The line after promotion, and more than one line at a time

F1–F8 cover a single prerelease line from first rc to promotion. The two cases below start where F5 ends,
and both came out of tracing the sequence past the promotion rather than stopping at it.

**F9 · the promoted line is stale until `main` comes back.** Given F5: `main` is at `1.5.0`, tag `v1.5.0`
created, and nobody has touched `prerelease-v1.5-rc`. Its versions file still reads `version = "1.4.2"`,
`prerelease = "1.5.0-rc.2"`, because the key was deleted by a commit on `main`. Now `c4 feat: …` lands on
`prerelease-v1.5-rc`:

- its anchor is still `v1.4.2` — its own `version` — so the range is `v1.4.2..HEAD`
- the base recomputes to `1.5.0`, unchanged from the open prerelease's base, so the counter increments
- result: `v1.5.0-rc.3` — a release candidate for a version that has **already shipped as stable**, and one
  that sorts *below* it (`1.5.0-rc.3 < 1.5.0`)

No existing guard fires. The collision check (#9, #80) inspects the tag cicada is about to create, and
`v1.5.0-rc.3` is genuinely new. The stale-merge guard (#33, #78) only errors when the recorded version is
*lower* than the recomputation, and here they agree.

**Decided: this is accepted behaviour** (#91). cicada acts on what the branch and the config say, and this
branch says an rc line is open at `1.5.0-rc.2`. Refusing would mean extending the collision check from the tag
being created to the *computed stable base*, which is machinery added to detect a branch its operator has
stopped maintaining — and F9b shows the fix costs a fast-forward. Assert the `rc.3` outcome explicitly, as
behaviour rather than a known bug, so it is not "corrected" later by someone reading it as one.

**F9b · merging `main` back resolves it, and the merge is trivial.** F5 merged `prerelease-v1.5-rc` into `main`, so
the line's tip is an ancestor of `main` and merging `main` back is a **fast-forward** — there is no conflict on
`.cicada-versions.toml` to resolve, which is the objection this case invites. Afterwards the line reads
`version = "1.5.0"` with no `prerelease` key, the anchor resolves to `v1.5.0`, and `c4 feat:` cuts
`1.6.0-rc.1`. Assert the fast-forward and the reset counter.

**F10 · one identifier at a time (#84).** `prerelease_identifier` is a scalar, so a `beta` line and an `rc`
line cannot run **simultaneously** off one repo. Assert the config shape rejects a per-branch mapping, and
assert what makes the restriction costless: cicada distinguishes identifiers **in no way at all** — `alpha`,
`beta` and `rc` share the anchor, the base, the counter, the skipped changelog and the `prerelease: true`
release object, differing only in the string in the version.

**F10d · a line walks the ladder (#93).** The k8s/Node shape, and the reason "one at a time" is not "one
forever". Given `prerelease_branches = ["release-*"]`, branch `release-1.5`, `version = "1.4.2"`:

| step | config | commits | expected |
|---|---|---|---|
| 1 | `prerelease_identifier = "alpha"` | `c1 feat:` | `1.5.0-alpha.1` |
| 2 | unchanged | `c2 fix:` | `1.5.0-alpha.2` |
| 3 | edited to `"beta"`, committed | `c3 fix:` | `1.5.0-beta.1` — **counter resets** (#63) |
| 4 | edited to `"rc"`, committed | `c4 fix:` | `1.5.0-rc.1` — resets again |
| 5 | merged to `main` | — | `1.5.0`, one changelog entry containing `c1`–`c4` |

Assert the base is `1.5.0` at every step — the notes stay cumulative from `v1.4.2` throughout (#61), which is
what makes step 5's single entry complete. Assert the counter resets in steps 3 and 4 and **does not** produce
`beta.3` / `rc.4`: with only the base considered, an unchanged base would increment, and #85 would not catch it
because `beta.3` sorts above `alpha.2`. This is the assertion the whole two-axis rule exists for.

**F10e · the ladder must sort upward (#85, #94).** Three edits against an open `1.5.0-beta.2`:

| edited to | sorts | expected |
|---|---|---|
| `rc` | up | `1.5.0-rc.1` |
| `alpha` | down | **hard error**, naming `1.5.0-alpha.1` and `1.5.0-beta.2` |
| `preview` | down (`p` > `b`) | **hard error** — the case that looks like a legitimate ladder and is not |

Assert also that no config key accepts an ordered list of stages: a declared order would make row 3 legal in
cicada while the published versions still sorted the other way, and consumers resolve with semver (#94).

**F10f · the identifier travels through a merge, and the next line inherits it.** After F10d step 5, `main`'s
`cicada.toml` carries `prerelease_identifier = "rc"` — inert there, since `main` matches no prerelease pattern
(#82). Assert that a new line cut from `main` therefore starts at `rc`, **not** `alpha`, and that cutting it at
`alpha` requires editing the value back. Asserted because it looks like a bug and is the direct consequence of
the identifier being ordinary config rather than branch state.

**F10b · the identifier cannot be walked downwards (#85).** `1.5.0-rc.2` is open and a developer edits
`prerelease_identifier` from `rc` to `beta`. The computed `1.5.0-beta.1` sorts *below* the `1.5.0-rc.2` that
already shipped, so the run **hard-errors**, naming both versions. This is #76's monotonicity rule applied to
a computed version rather than a forced one, and it is the **only** rule governing a maturity ladder (#93) —
cicada never learns that `beta` follows `alpha`, only that versions must go up.

**F10c · two prerelease branches, one identifier.** `prerelease-v1.5-rc` and `prerelease-v2.0-rc` both match
the list, as two maintenance branches both match `release/*`. Each carries its own `prerelease` key and its
own base, and if both ever compute the same version the second to run hits the collision check (#9, #80),
exit `4` — identical to G4. Assert no coordination mechanism exists or is needed.

**F11 · the entries are globs, and the branch name is not an input.** `prerelease_branches =
["prerelease-v*-rc"]` is matched against the full branch name. Assert three things, because each is a separate
trap:

- `prerelease-v1.5-rc` and `prerelease-v6-rc` both match the one pattern, and cutting the second needs **no
  config commit** — the property `maintenance_branches` already has for `release/*` (G7b).
- the `v1.5` in the name is **never parsed**. A `feat!:` on `prerelease-v1.5-rc` computes `2.0.0-rc.1`, the
  branch name still says `v1.5`, and cicada neither corrects it nor warns. Asserted because putting a version
  in the branch name invites the assumption that cicada honours it.
- a literal entry (`["prerelease-v1.5-rc"]`) is a valid glob matching exactly one branch — supported, and the
  thing #82 exists to stop anyone *having* to write.

**F11b · the default classifiers (#83).** With neither classifier configured, the built-in
`prerelease_branches = ["prerelease/**", "prerelease-*"]` and `prerelease_identifier = "rc"` apply. Assert:

- `prerelease-v1.5-rc`, `prerelease-v6-rc` and `prerelease/foo/bar/v1.5.x` all match it — the `/**` and `-*`
  spellings are both needed, because `*` stops at `/`
- the branch must **still** be in `release_branches`, which has no default. A branch matching
  `prerelease-*` but absent from the gate exits `0` quietly (H1). This interlock is what bounds the default's
  blast radius, and it is asserted rather than assumed
- the two settings are now the **same shape** — a glob list each — so neither has a precedence rule and
  neither needs an ordered map (#84)
- `maintenance_branches` behaves identically with `["maintenance/**", "maintenance-*"]`, and a
  `maintenance/1.4` branch gets the patch-only guard with no config
- scenario 2's `maintenance_branches = ["release/*"]` **overrides** the default rather than adding to it —
  `maintenance/*` branches in that repo are ordinary

**F11c · precedence is semver's, not cicada's.** Assert `1.4.2 < 1.5.0-rc.1 < 1.5.0-rc.2 < 1.5.0`: a version
carrying any prerelease field ranks below the same version without one, and the counter compares numerically
so `rc.10` follows `rc.9`. Neither property is configured or enforced by cicada — both fall out of the `semver`
crate, which is why the stable tag can never collide with the rcs that preceded it. The ordering *between*
different identifiers is the same ASCII accident (`beta` < `rc`) — which is exactly why a staged line (F10d)
gets its ordering from #85 rather than from a declared ladder (#94), and why F10e's `preview` → `beta` is
refused despite reading like a sensible progression.

---

## G · Maintenance branches

All of cicada's state lives inside the branch, so these need no setup beyond cutting the branch.

**G1 · a patch on an old line.** `release/1.4` cut from tag `v1.4.7`; `release_branches = ["main", "release/*"]`;
a `fix:` cherry-picked onto it. The anchor `v1.4.7` resolves and is reachable **from this branch**; range is
`v1.4.7..HEAD`; result `1.4.8`, tagged on the branch. Assert `main` is untouched and its versions file
unchanged.

**G2 · the maintenance guard.** `release/1.4` matches `maintenance_branches = ["release/*"]`. A cherry-picked
`feat:` **hard-errors, exit `4`**, naming the commit (#28). Assert no release PR branch is created and no
forge call is made. The config is byte-identical to `main`'s — `main` simply does not match the pattern
(#82), so cutting the branch required no config commit.

**G2b · there is no resolving it inside cicada** (#39). The operator decides the feature does belong on this
line — and cicada offers nothing. Assert that the error text says so and names the two real answers: put the
change on a newer line, or re-do the cherry-pick as the fix it is. Asserted on the *message* rather than the
behaviour, because a blocked pipeline with no stated way forward is how an escape hatch gets reintroduced.

**G2c · caught before merge.** `cicada check --base release/1.4 --head <pr-head>` on a PR targeting the
maintenance branch reports the same problem at exit `3`, so it surfaces at review time rather than at release
time.

**G3 · branch naming.** `release_pr_branch = "cicada/{branch}"` renders `cicada/release/1.4`. Legal git
ref; asserted so nobody "fixes" it by flattening the slash, which would map both `release/1.4` and a
hypothetical `release-1.4` onto one head branch. PR base is `release/1.4`. Assert also that the rendered
head matches **neither** `release_branches` nor `maintenance_branches` — `cicada/*` is outside both
namespaces, which is what stops cicada's own force-push from re-triggering it (#27).

**G4 · version collision, caught in Phase A (#9, #80).** Same as G2 but **without** `maintenance`. The
cherry-picked `feat:` computes `1.5.0`, which `main` already tagged. `v1.5.0` exists but is **not reachable**
from this branch's HEAD → collision, exit `4`. Assert this fires in **Phase A**: no release PR branch is pushed
and no PR is opened. Discovering it in Phase B would mean the PR was already reviewed and merged, leaving the
branch merged-but-untagged.

**G4b · Phase B keeps the check as a backstop.** A hand-edited versions file naming a version whose tag
exists unreachably never passes through Phase A. Phase B catches it at tag time, exit `4`.

**G6 · trunk-release arrangement (#81).** `release_branches = ["main", "release/*"]`, main on `1.5.x`,
`release/1.4` cut after main reached `1.5.0`. A fix on each line releases `1.5.1` and `1.4.8` independently,
no collision. Assert the two branches' versions files and changelogs never reference each other.

**G7 · a trunk excluded from the gate still exits quietly.** `release_branches = ["release/*"]` with main
absent: a push to main exits `0` before any config-dependent work (H1), and main's `.cicada-versions.toml` is
never written. This is the *gate* behaving correctly, not the release-branch arrangement — that arrangement is
deferred (#81, future-work §14), because a new line's opening minor has no way to be stated once #39 is gone.
Assert the gate; do not assert a release from such a line.

**G7b · cutting a line needs no config commit (#82).** `release/1.4` is created with
`git checkout -b release/1.4 v1.4.7 && git push`. Assert cicada acts on it on the very next push — it matches
both `release_branches` and `maintenance_branches`, both declared once on `main` and inherited by the cut.
Assert the branch's `cicada.toml` is byte-identical to `main`'s.

**G7c · merging a branch back carries no branch state** (#82). `release/1.4` is merged into `main`. Assert
`main`'s `cicada.toml` is unchanged by the merge, and that `main` does not become patch-only. The same
assertion for `prerelease-v1.5-rc` → `main`: `main` must not start cutting rcs. This is the case that killed
the per-branch scalar, and it is asserted on both classifiers because both would have travelled.

**G8 · a tag that is not reachable is a hard error, whatever put it there.** A branch is cut from `main`
while `main`'s versions file names `1.4.0`, whose tag lives on a diverged branch. Anchor resolution finds
`v1.4.0` unreachable from HEAD → hard error, exit `1` (J2). Asserted so the error is recognisable rather than
mysterious: it is the same primitive as the collision check (#9), reported from the other end.

**G5 · divergence is permanent and accepted.** After G1, `main`'s `CHANGELOG.md` never learns about `1.4.8`.
Asserted as expected behaviour, not a bug (future-work §10).

**G10 · a parallel line that still takes features.** The variant of G6 where the older line is *maintained*
rather than frozen — an LTS on `1.x` still shipping features while `main` is on `2.x`. It differs from G1–G6
in one line of config:

```toml
release_branches     = ["main", "release/*"]
maintenance_branches = []                     # or a pattern that release/1.x does not match
```

- given `main` at `2.1.0` (tag `v2.1.0`); `release/1.x` at `1.9.3` (tag `v1.9.3`, reachable from that branch)
- when `feat: add export` lands on `release/1.x`
- then `1.9.3 → 1.10.0` — a **minor on a release branch**, which G2 would have refused
- assert `main` is untouched, and assert **no collision**: `v1.10.0` does not exist anywhere, because the two
  lines are on different majors and cannot converge (#9, #80)

Asserted because G2 and G4 between them make "a `feat:` on a `release/*` branch" look like it is always an
error, and it is not — it is an error only when the branch matches `maintenance_branches` (G2) or when the
computed version is already taken (G4). Nothing about a release branch is inherently patch-only; the pattern
is what makes it so.

**G10b · the two lines are on different majors, and that is load-bearing.** Construct the failure: move
`main` back to `1.9.x` so both lines compute `1.10.0`. The second to run hits the collision check, exit `4`
(G4). Assert the pair together — G10 works *because* the majors differ, not because parallel lines are
special-cased.

---

## H · Branch gating, offline, rehearsal

**H1 · non-release branch.** HEAD on `feature/foo`, `release_branches = ["main"]` → exit `0` immediately.
Assert: no config-dependent work beyond the branch check, no files read beyond `cicada.toml`, zero forge
calls, and nothing on stdout except at `-v` (#26).

**H2 · glob match.** `release_branches = ["main", "release/*"]`, HEAD on `release/1.4` → proceeds.

| id | command | writes files | local commit/tag | pushes | forge calls | token |
|---|---|---|---|---|---|---|
| H3 | `cicada plan` | — | — | — | **none** | not required |
| H4 | `cicada check --base … --head …` | — | — | — | **none** | not required |
| H5 | `cicada release-pr --dry-run` | — | — | — | none | not required |
| H6 | `cicada release-pr --no-push` | ✓ | ✓ | — | none | not required |
| H7 | `cicada release-pr` | ✓ | ✓ | ✓ | ✓ | required |

**H3/H4 are a stated goal (§Goals 6), not an implementation detail.** The assertion is structural: the test
harness fails the run if any HTTP client is constructed.

**H5 detail.** The dry-run diff must be produced by the *same* computation as H6 with the write step skipped
(#30) — the test asserts the `Vec<FileEdit>` from H5 and H6 are equal, which is the only way a dry-run stays
honest.

**H6 detail.** Leaves a real commit on a real local branch. `git show cicada/main` displays the
version bumps and changelog entry. The release manifest is on disk, which is the only way to inspect a
rehearsal. This is the M3 dogfooding gate.

**H8 · shallow clone.** `git rev-parse --is-shallow-repository` returns true → error naming `fetch-depth: 0`
(Actions) and `GIT_DEPTH: 0` (GitLab), exit `1`. Detected up front, before any range is computed, so the
failure is never a silently truncated changelog.

**H9 · on-demand and scheduled runs are ordinary runs.** cicada is gated on the **branch**, never on what
started the process (#26). Assert identical behaviour and an identical forge-call log across three triggers on
the same repo state: a push-triggered CI run, a cron-triggered CI run (D5d), and a developer running
`cicada run` from a laptop with `CICADA_TOKEN` set. Assert also that nothing in the config names a trigger —
there is no schedule, no event filter, and no `on:` equivalent anywhere in `cicada.toml`.

This is what makes "release when we decide to" a supported arrangement needing no feature: the decision is
*when to run cicada*, and that has always been the caller's. What a run cannot do is release a range that
justifies no release (§Non-goals) — which is D5d, and the reason that scenario sits next to this one.

---

## I · `cicada check`

Takes a **git range, not a forge PR object** — no API call, no token, identical on GitHub, GitLab, and a
laptop.

| id | input | exit | output |
|---|---|---|---|
| I1 | all commits conventional, scopes valid | `0` | one ✓ per commit |
| I2 | `update deps` | `3` | *not a conventional commit* |
| I3 | `feat(aip): …` | `3` | *unknown scope 'aip'* + *did you mean 'app'?* (`strsim`) |
| I4 | config: package `path` missing from disk | `3` | names the path |
| I5 | config: two updaters share an `output` (#44) | `3` | names both |
| I6 | config: `default_scope` names nothing | `3` | names the valid targets |
| I6b | config: `release_pr_branch = "release/{branch}"` with `release_branches = ["release/*"]` (#27) | `3` | the head branch would match the release allowlist; names the rendered ref |
| I6c | config: `release_branches` absent (#83) | `3` | required; the error names the current branch as the likely value |
| I6d | config: both classifiers absent, branch `prerelease-v1.5-rc` (#83) | `0` | the default maps `prerelease-*` → `rc`; no stanza needed for the commonest case |
| I6e | config: both classifiers absent, branch `prerelease-v1.5-beta` (#83) | `3` | the trailing segment says `beta`, `prerelease_identifier` defaults to `rc` — names both and the one line to change |
| I7 | config: unknown key `tag_fromat` (#67) | `1` | config does not parse; this is not a "finding" |
| I8 | config: malformed TOML | `1` | parse error with line |

**I4–I6c and I6e exit `3`, I7–I8 exit `1`** (Q6). The split is deliberate: `3` means *cicada read your config and
your commits and found problems*, which CI may treat as a lint failure; `1` means *cicada could not load
its own configuration*. Every other command hard-errors (`1`) on I4–I6c, since they cannot proceed. I6d is
not a finding at all — it is the config the defaults exist to serve, and asserting it is what stops the
default being a dead letter.

**I9 · fork safety.** `check` runs with no token against an untrusted fork's range. Assert zero HTTP.

**I10 · there is no title mode** (#87). `check` takes a range and nothing else; `--title` does not exist and
`commit_source` has three values (`commits` | `merges` | `both`), none of them `title`. Assert `--title`
exits `2` as an unknown flag, and that `commit_source = "title"` is a config parse error under
`deny_unknown_fields`-equivalent enum validation (#67) rather than a silently accepted value. Linting a
squash-merge PR title is future-work §15.

---

## J · Anchor resolution and history

The versions file holds **a version and nothing else** (#1), so the tag is the only anchor (#2) and this
group collapses to three outcomes.

**J1 · tag missing.** Versions file says `version = "1.4.2"`; tag `v1.4.2` does not resolve. **Hard error**,
exit `1`, naming the tag and the repair (`git tag v1.4.2 <commit> && git push origin v1.4.2`). No fallback,
no warning-and-continue — a resolvable-but-wrong range produces a changelog that looks fine and is
incomplete, which is the failure mode the tool exists to avoid.

**J2 · tag exists but is not reachable from HEAD (#3).** `v1.4.2` was cut on another branch. Hard error,
exit `1` — the range would otherwise silently span the merge base. Same primitive Phase B uses for collision
detection (#9), and it needs nothing stored.

**J3 · tag reachable.** The normal path: `anchor = v1.4.2`'s commit, range `anchor..HEAD`. Assert cicada
performs exactly one `git log` for the package and reads no version out of `Cargo.toml`.

**J4 · manual release.** A human edits `.cicada-versions.toml` from `1.4.2` to `1.6.0` and pushes to `main`.
Phase B derives `v1.6.0`, finds it missing, runs the staleness guard — recorded `1.6.0` is *higher* than the
recomputed `1.5.0`, which is permitted (D4b) — and creates the tag. A supported workflow, not an accident:
the versions file is one number.

**J4b · manual edit downwards.** The same edit to `1.0.0`. The staleness guard catches it with no extra
machinery: recorded `1.0.0` is lower than the recomputed `1.5.0` → hard error, exit `4`. This is #76
enforced by #33, not by a separate rule. The hand edit must land on `main` — on the release PR branch a human
commit trips the staleness guard first (D2).

**J5 · `commit_source` on a merge-commit repo.** Same history, three settings (#40):

| setting | traversal | sees |
|---|---|---|
| `"commits"` | `--no-merges` | the individual commits inside each merged PR |
| `"merges"` | `--first-parent` | one entry per PR, from the merge commit's message |
| `"both"` | — | everything; can double-count a change |

**J6 · `commit_source` on a squash/rebase repo.** `main` is linear, so all three produce **identical** plans.
Asserted because it is the claim that lets cicada work out of the box regardless of merge strategy.

**J7 · a trunk fed only by merges.** J5 asserts the three traversals; this asserts the *arrangement* they
exist for — a git-flow-shaped `main` that never takes a direct commit, where everything arrives as a merge
from a feature or release branch. Build the fixture so the two readings genuinely disagree:

```
main    A ──────── m1 ──────── m2
                    ╲           ╲
feature/x     c1 feat: add export · c2 chore: tidy
feature/y                         c3 fix: handle nil
```

`m1`'s merge message is `Merge pull request #41 from feature/x` under GitHub's default, or
`Merge branch 'feature/x' into 'main'` carrying the MR title under GitLab's.

| setting | plan | why |
|---|---|---|
| `"commits"` (default) | `feat` + `chore` + `fix` → **minor** | reads `c1`, `c2`, `c3`; merge commits skipped |
| `"merges"` | depends entirely on the merge messages | reads `m1`, `m2` only — GitHub's default message is not a conventional commit, so **no release** |
| `"both"` | `feat` + `fix`, plus whatever the merge commits parse as | can count `c1` and `m1` as two changes |

**J7 detail — the default is not always right, and the failure is quiet.** On GitLab, where the default merge
message carries the MR title, `"merges"` is the setting that reads what humans actually wrote and
`"commits"` reads the individual commits inside — often WIP messages that were never meant to be release
notes. On GitHub the reverse holds, because its default merge message is not conventional at all and
`"merges"` would see nothing releasable. Assert the GitHub case explicitly: `commit_source = "merges"` on a
repo whose merge messages are GitHub's default produces an **empty plan**, exit `0`, and no release — a repo
that silently stops releasing, with a green pipeline, which is the failure mode this row exists to make
recognisable.

Assert also what J6 already implies: switch the same repo to squash-merge and all three settings agree again,
so the setting only ever matters on a repo that keeps merge commits.

---

## K · Updaters and file edits

**K1 · cargo.** `Cargo.toml` `version` rewritten via `toml_edit`. Assert every other byte — comments, key
order, blank lines, trailing whitespace — is identical.

**K2 · generic, yaml path.** `edit = { yaml = "image.tag" }` on a `values.yaml`. Assert **exactly one line**
differs, byte-for-byte, including the trailing comment on that line (#49).

**K3 · aliases outside the target (#72).** The same file contains `&baseline` / `*baseline` twenty lines away
from `image.tag`. The edit **succeeds** — the walker tolerates constructs it cannot address as long as they
are not on the addressed path. This was the first draft's bug.

**K4 · non-ASCII before the target (#71).** A comment containing `naïve — … ✅` precedes the target. Assert
the splice lands correctly. `Marker::index()` is a **char** index despite its doc comment saying bytes;
without conversion this produces `vers2.0.01.0.0`. This regression test must survive from the spike into the
real updater.

**K5 · marker comment (#50).** `image: acme/app:1.4.2 # x-cicada-version` in a Dockerfile or Makefile. The
first semver-shaped token on the annotated line is replaced; the rest of the line, including the marker, is
untouched. This is the escape hatch for ecosystems with no dedicated `kind` yet.

**K6 · template.** `templates/deployment.yaml.j2` → `deploy/deployment.yaml`, rendered by minijinja. cicada
owns the output file wholesale. Paths resolve from the **repo root**, not the package path (#44).

**K7 · duplicate output.** Two updaters declare the same `output`. Hard error at both `check` (exit `3`) and
`plan` (exit `1`). Last-writer-wins would be a silent, config-order-dependent bug.

**K8 · post-write re-parse failure (#52).** A splice produces an unparseable file. The run aborts **before
anything is committed**. Assert the working tree is clean and no release PR branch commit exists — the abort
must precede the commit, not follow it.

**K9 · tag-only package.** `updaters = []`. Changelog and versions file are written; no other file is
touched; tag and release are still created.

**K10 · missing updater path.** `check` exits `3` naming the path.

---

## L · Changelog rendering

**L1 · created.** No `CHANGELOG.md` → created with a header and the first entry (#5).

**L2 · marker.** `<!-- cicada:insert -->` present → the entry is inserted at the marker. Assert everything
above and below the insertion point is byte-identical.

**L3 · no marker.** File exists with an H1 and prior entries, no marker → the entry is prepended after the
header.

**L4 · accumulation.** A second release adds its entry above the first. Assert the first entry is
byte-identical to before — changelogs are append-at-top, never rewritten.

**L5 · template override (#47).** `.cicada/templates/notes.md.j2` exists → picked up by convention with no
config change.

**L6 · round-trip the default (#48).** `cicada template print notes > .cicada/templates/notes.md.j2`, then
re-run: output is byte-identical to the un-overridden run. This is the property that makes customisation
start from the real thing rather than from reverse engineering.

**L7 · escaping (#45).** A commit subject containing `: `, a quote, and a leading `-` is rendered into a YAML
template through `| yaml`. Assert the output parses as YAML. Autoescape is off because the output is
Markdown and YAML, so the filters carry the whole burden.

**L9 · the entry date is an input, not a clock read (#95).** `cicada release-pr --no-push --date 2026-09-11`
renders `## 1.5.0 (2026-09-11)`. Assert three things: re-running with the same `--date` on a different day is
byte-identical; `--dry-run --date X` and `--no-push --date X` produce the identical `Vec<FileEdit>` (#30); and
the test harness fails the run if core reads a clock at all. The failure this prevents is small but daily — a
dry-run at 23:59 and the real run at 00:01 writing different changelog entries, which makes "the same
computation with the write skipped" false for one minute in every 1440.

**L8 · one template, two destinations (#43).** Assert the notes body embedded in the changelog entry and the
notes body sent to `create_release` derive from the same render — the changelog adds only the version header
and the marker handling.

---

## M · Shape 2 — app plus its chart

Two packages, no umbrella. Established state: `app = 1.4.2` (tag `v1.4.2`), `app-chart = 0.9.0`
(tag `chart-v0.9.0`), `on_tracked_bump = "patch"`.

**M1 · app release drags the chart (#22).**
- when `c1 feat(app): add caching`
- then
  - plan · `app: 1.4.2 → 1.5.0 (minor)` · `app-chart: 0.9.0 → 0.9.1 (patch, derived)` — the derived bump is
    labelled distinctly from an earned one
  - files · `Cargo.toml` → `1.5.0` · `Chart.yaml` `version` → `0.9.1` and `appVersion` → `"1.5.0"`,
    **quoted** (#51) · two changelogs, each containing only its own package's commits — the chart's entry is
    rendered from `derived_from` rather than a range (M1b, #97) · versions file updates both entries
  - git · two tags: `v1.5.0` and `chart-v0.9.1`
  - forge · two `create_release` calls, in config order (#96); **one** release manifest listing both (#42),
    embedded in **both** release bodies — this repo declares no umbrella, so there is no summary release to
    single out, and the manifest is byte-identical in each

  This is the only dependency propagation in cicada, and it is *declared* in config, never inferred.

**M1b · a derived release's changelog entry explains itself** (#97). The chart released without receiving a
single commit, and each changelog holds only its own package's commits (#15) — so `commits` is empty and the
notes context carries `derived_from = { package = "app", version = "1.5.0" }` instead. The default template
renders:

```markdown
## 0.9.1 (2026-09-15)

_Released to track app 1.5.0._
```

Assert the entry **is** written — skipping it would leave a hole in the chart's changelog where a real tagged
release was — that the app's commit does **not** appear in it, and that overriding `.cicada/templates/notes.md.j2`
changes the wording (#47), because the sentence is a template's, not cicada's.

**M1c · earned and derived at once.** M3's range gives the chart its own `feat` *and* a derived bump. Assert
`commits` is non-empty **and** `derived_from` is set, and that the default template renders the sections
normally without the tracking line — the entry already explains itself.

**M2 · chart-only change.** `c1 feat(app-chart): add a PodDisruptionBudget` → `app-chart: 0.9.0 → 0.10.0`
(`pre_major = "minor"` keeps 0.x inside 0.x). `app` is untouched: no bump, no tag, no changelog entry, and
**`appVersion` is not rewritten**.

**M3 · both apply, larger wins.** `feat(app):` + `feat(app-chart):` in one range. The chart's own bump
(minor) and its derived bump (patch) are reconciled to the larger: `app-chart: 0.9.0 → 0.10.0`, and
`appVersion` still updates to `1.5.0`.

**M4 · unscoped routing.** `c1 fix: handle nil` with `default_scope = "app"` → app patch `1.4.3`, chart
derived patch `0.9.1`. Asserted because in shape 2 an unscoped commit is a *silent* choice about which
package it hits.

**M5 · unquoted appVersion found in place.** `Chart.yaml` contains `appVersion: 1.10` (which YAML parses as
the float `1.1`). cicada emits a warning and writes the new value **quoted**. Assert both the warning and the
quoting.

**M6 · a chart cannot reach a derived first release** (#88). `app` at `1.4.2` releases `1.5.0`; `app-chart`
has **no** versions-file entry. A derived `patch` on an unreleased package would take its version from the
first-release table (#37) and land on `0.0.1` — a number chosen by `on_tracked_bump`, a config default that
says nothing about the chart. The case is now unreachable: a package with a `helm` updater must declare
`initial_version`, so `check` exits `3` naming the package before this can arise, and the chart's first
release is whatever its author stated. Assert the `check` finding, and assert that with
`initial_version = "0.9.0"` declared the derived first release is exactly `0.9.0`.

The reason it is required only on `helm` packages: Helm makes `version` **mandatory** in `Chart.yaml`, so the
number already exists and its author knows it — while cicada cannot read it back, because `Chart.yaml` is an
output (#75). Config is the only place the two can meet.

**M7 · `on_tracked_bump = "minor"`.** Same as M1 but `app-chart: 0.9.0 → 0.10.0`.

**M8 · divergent anchors.** After M1, `app` is anchored at `v1.5.0` and `app-chart` at `chart-v0.9.1`, which
sit on the same commit. After M2, they diverge: `app`'s range starts earlier than the chart's. Assert one
`git log` per package with different ranges — the per-package anchor is the whole reason this works.

**M9 · a tracked bump is not a copied version** (#22). `feat(app):` takes `app` from `1.4.2` to `1.5.0`, and
the chart takes its configured derived bump to `0.9.1` — it does **not** become `1.5.0`. Only `appVersion`
carries the app's number across; the chart's own version line is independent (#14). Asserted because "tracks"
reads like "mirrors", and for `appVersion` it does.

**M10 · the guard covers derived bumps too.** On a branch matching `maintenance_branches`, an
`on_tracked_bump = "minor"` chart dragged by an app release computes above patch → hard error, exit `4`. Assert the message names the chart and
its tracking relationship, not just the app commit, since the cause is one step removed.

**M11 · partial reconciliation.** Phase B tagged `v1.5.0` but `chart-v0.9.1` push failed. Next run creates
only the missing chart tag and its release. Per-package, per-artifact idempotency (#56 generalised).

---

## Where a version lives, and when it moves

Six artifacts carry a version in shape 1. They do **not** move together, and the gap between Phase A and
Phase B is where most confusion lives.

| Artifact | Written by | Reaches `main` | Value between releases |
|---|---|---|---|
| `.cicada-versions.toml` `version` | Phase A | at merge | last released version |
| `Cargo.toml` `version` | Phase A (updater) | at merge | last released version |
| `CHANGELOG.md` heading | Phase A | at merge | — |
| git tag | Phase B | after merge | — |
| forge release | Phase B | after merge | — |
| `target/cicada/release-manifest.json` | Phase B | never (gitignored) | — |

Three properties follow, each asserted by a scenario above:

- **Main's own files are untouched until merge.** The bump exists only on the release PR branch (A1, C1).
- **`.cicada-versions.toml` is the sole source of truth** (#75). Every other file is an output, which is why
  a first release can write a manifest *downwards* (A9).
- **Between releases, `main` carries an already-shipped version.** There is no `-dev` or `-SNAPSHOT`
  convention: build from `main` at an arbitrary commit and the binary is stamped with the last release. This
  is coherent because cicada does not build artifacts (#70) — a tag-triggered workflow does, and at the tag
  the stamp is accurate. The alternative is the Java SNAPSHOT flow deferred in future-work §3.

---

## Deltas this exercise found for `docs/design.md`

Each needs a decision entry or a wording change. Numbered for the discussion, not for the design log.

**Applied to `docs/design.md` as decisions #73–#79, #83–#91.** **None remain open** — the last five were
closed in the scope pass that removed forced versions (#39), and four of them dissolved rather than needing
a rule.

| # | Delta | Status |
|---|---|---|
| 1 | `[umbrella]` is optional; cicada never assumes an aggregate version | **#73** |
| 2 | An app and its chart in one repo are two packages, not an umbrella | **#74** |
| 3 | `check` rule: with no umbrella declared, `default_scope` must name a package | **#73** |
| 4 | Revert deferred | future-work §11 |
| 5 | #34's justification replaced — config edits, not reverts. The empty-plan path is the steady state, not an exception | future-work §11 |
| 6 | Phase B skips packages with no versions-file entry. The old second state (entry present, neither tag nor sha resolves) **no longer exists** — there is no sha | **#2**, dissolved |
| 7 | The staleness guard runs only when a tag is about to be created | **#78** |
| 8 | `tagging = false` implies no release objects — **dissolved**: tagging is now mandatory, since the tag is the anchor | **#6** |
| 9 | `commit_source = "title"` contradicts its own enum | **#87** — `check` lints a range only; title linting deferred to future-work §15 |
| 10 | `check` exits `3` for semantic findings, `1` for a config that will not parse | **#79** |
| 11 | Forced-version monotonicity, enforced by the staleness guard's one-sided comparison (J4b) rather than a separate rule. Its two sub-questions **dissolved**: forced versions left v1 entirely | **#76**, narrowed to the hand-edited versions file by **#39** |
| 12 | `{summary}` in `release_pr_title` has no defined rendering | **#86** — the title is a plain string; rendering deferred to future-work §16 |
| 13 | `find_pull_request` searches open PRs only | **#77** |
| 14 | Derived bump on a never-released tracked package lands on `0.0.1` | **#88** — `initial_version` required on `helm` packages, so the case is unreachable |
| 15 | `from` on a first release in `--json`: `null`, absent, or `"0.0.0"` | **#89** — `null`, present and explicit |
| 16 | A promoted prerelease line keeps a stale `prerelease` key until `main` is merged back, and cuts an rc *below* an already-shipped stable. No guard fires (F9) | **#91** — accepted; merging back is the operator's job |
| 17 | The counter rule (#63) is silent when the identifier changes under an unchanged base | **#93** — reopened when staged prerelease lines came into scope, and answered: the counter resets on a change to the base **or** the identifier. #84's "the base is the only axis" no longer holds, and #85 does not cover it (`beta.3` sorts above `alpha.2`) |
| 18 | The staleness guard's `prev` could select a prerelease tag, replanning over a possibly-empty range | **#90** — `prev` is stable-only |
| 19 | Forced versions removed from v1: the maintenance guard loses its escape hatch, and release-branch releases lose the only way to state a line's opening version | **#39**, **#81** — the arrangement is deferred as a unit (future-work §13, §14) |
| 22 | **What goes in a derived release's changelog entry?** A chart dragged by its tracked app receives no commits, so the body renders empty and the design was silent (M1b). *(20 and 21 live in `monorepo.md`; the numbering is shared across both files.)* | **#97** — the entry is written, and the notes context carries `derived_from` so the default template names the cause. Data, not prose cicada emits |

Two findings came out of tracing the single-service lifecycle commit by commit rather than from reading a
section, which is the method `docs/next-steps.md` recommends:

- **The old stale-merge check fired on every normal release.** It compared committer identity over
  `sha..HEAD`, which after any merge contains the merge commit — whose committer is the forge's web-flow
  identity, not `cicada[bot]`. Squash-merge had the same problem. Removing the stored SHA dissolved the bug
  rather than requiring a fix; the release **PR** branch guard (#32) is unaffected, because it inspects a branch
  cicada wrote itself with no forge merge in between.
- **A first release can lower a version file**, and that is correct (#75, A9).

---

## Coverage against the design

Sections of `docs/design.md` exercised by at least one scenario here: anchor resolution · commit parsing ·
`commit_source`, both as traversals (J5, J6) and as the merge-fed-trunk arrangement they exist for (J7) ·
bump computation · first-release table · `pre_major` · the `maintenance` guard · prereleases (all of
#59–#63, #84–#85, #93–#94, including promotion and staged `alpha`/`beta`/`rc` lines) · release manifest and
its per-release embedding (#42, #96) · all four updater kinds · both addressing modes · YAML splice traps ·
changelog insertion, including a derived release's entry (M1b, #97) · templates and filters · both phases ·
staleness (all three layers) · tag reconciliation · independent release reconciliation · version collision ·
maintenance branches · **parallel feature-taking lines (G10)** · branch gating and **trigger independence
(H9, D5d)** · offline modes · exit codes · `--json` schema · chart tracking.

**Release strategies covered by at least one scenario:** trunk releases (C1–C8) · trunk plus patch-only
maintenance lines (G1–G5) · trunk plus a parallel feature-taking line (G10) · a prerelease line feeding trunk
(F1–F8) · staged `alpha`/`beta`/`rc` lines (F10d–F10f) · continuous deployment via auto-merge (C9) ·
on-demand and scheduled runs (H9, D5d) · git-flow's merge-fed trunk (J7) · 0ver (B5, B6) · tag-only releases
(K9) · app plus chart (group M).

**Not exercised here, by construction** — these need more than one service and move to
`docs/scenarios/monorepo.md`: umbrella aggregation (#10, #15), multi-scope commits (`feat(api,worker):`),
scope routing across many packages, per-package release timing divergence at scale, and the Cargo
workspace-inheritance trap (#24).

**Not exercised anywhere yet:** the GitLab adapter's behavioural differences beyond the two asserted
fallbacks, and forge retry/backoff (#58).

**Out of scope by decision, and asserted as absent rather than untested:** forced versions (#39, group E),
CalVer and other non-semver schemes (future-work §17),
release-branch and milestone-line arrangements (#81, G7), `check`'s title mode (#87, I10), and a rendered
release-PR title (#86). `cicada init` is gone too — config authoring is a human job (future-work §12). Each
has a negative assertion somewhere above, because a removed feature that nobody asserts the absence of grows
back.

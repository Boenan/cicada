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
release_pr_title  = "chore(release): {summary}"

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
| B16 | `1.4.2` | `feat:` + `--release-as app=1.4.8` , branch matches `maintenance_branches` | `1.4.8` | `Release-As` bypasses bump computation, so the guard never sees a computed bump |
| B12 | `1.4.2` | `feat(app):` | `1.5.0` | explicit scope ≡ unscoped in shape 1 (Q2) |
| B13 | `1.4.2` | `feat(parser):` | **error** | unknown scope is fatal (#11); `check` exits `3` with *did you mean 'app'?* |

**B11/B14 detail.** The error must name the offending commits, not just the computed bump — on a cherry-pick
batch of twenty, "a `feat:` is present" is not actionable. Assert the message lists each commit that
contributed above patch. B15 is the assertion that matters most in practice: a maintenance branch doing its
ordinary job must never see this error.

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
a `Release-As: 3.0.0`, or a hand-edited versions file. The guard passes: over-bumping is a supported
workflow, under-bumping is the harm (#33). Assert no error.

**D4c · what the guard does not catch.** PR proposes `1.5.0` from a `feat:`; a stale `fix:` lands before the
merge. Replanning still yields `1.5.0`, so the guard passes and the fix ships with no changelog entry. The
version is correct, the notes are one line short. Accepted: catching it would require recording the commit
the changelog covers, which is the state #2 deliberately removed. Layers 1 and 2 are what prevent it.

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

## E · Forced versions

| id | given | input | expected |
|---|---|---|---|
| E1 | `1.4.2`, computed `1.5.0` | `feat: …` with `Release-As: 2.0.0` footer | `2.0.0` exactly — unscoped applies to `default_scope` (#39) |
| E2 | `1.4.2` | `--release-as app=2.0.0` | `2.0.0` |
| E3 | `1.4.2` | two commits, `Release-As: 2.0.0` and `Release-As: 3.0.0` | **hard error**, exit `1` — silent last-wins is a trap |
| E4 | `1.4.2` | `Release-As: 1.0.0` | **hard error**, exit `1` — a forced version must be strictly greater than the recorded one |
| E5 | `1.4.2` | `Release-As: 1.4.2` | **hard error** — equal is not greater |
| E6 | `1.4.2` | `Release-As: 2.0.0-beta.1` | **OPEN — Delta 11**: is a prerelease value legal outside the §F flow? |

**E4/E5 — the monotonicity rule.** One rule covers every way a human can force a version: the `Release-As:`
footer, `--release-as`, and a hand-edited `.cicada-versions.toml`. The forced value must be **strictly
greater** than the version cicada currently has recorded, or the run hard-errors naming both. Nothing it
forbids could be correct: a lower or equal value puts `## 1.0.0` above `## 1.4.2` in the changelog and
resolves consumers' semver ranges backwards. See J4 for the hand-edited case.

**E1 detail.** `Release-As` bypasses bump computation but not the release decision: a `chore:` commit
carrying `Release-As: 2.0.0` is still a `bump = "none"` type. Pin whether it forces a release anyway.
Recommended: yes — an explicit `Release-As` is an unambiguous instruction to release. Folded into Delta 11.

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
*lower* than the recomputation, and here they agree. **OPEN — Delta 16.**

Candidates: (a) accept it, and document merging `main` back as the operator's job; (b) extend the #9 primitive
from the tag being created to the **computed stable base** — refuse when a tag for that base exists in the repo
and is not reachable from HEAD, which is exactly this situation and reuses machinery that already exists;
(c) report it in `check` only. A sub-question rides along with (b): with both keys present, which one does the
staleness guard read as "the recorded version"?

**F9b · merging `main` back resolves it, and the merge is trivial.** F5 merged `prerelease-v1.5-rc` into `main`, so
the line's tip is an ancestor of `main` and merging `main` back is a **fast-forward** — there is no conflict on
`.cicada-versions.toml` to resolve, which is the objection this case invites. Afterwards the line reads
`version = "1.5.0"` with no `prerelease` key, the anchor resolves to `v1.5.0`, and `c4 feat:` cuts
`1.6.0-rc.1`. Assert the fast-forward and the reset counter.

**F10 · one line, one identifier (#84).** A repo gets one prerelease line and one identifier;
`prerelease_branches` is a glob list and `prerelease_identifier` a scalar, so a `beta` line feeding an `rc`
line is not expressible. Assert the config shape rejects a per-branch mapping, and assert what makes the
restriction costless: cicada distinguishes identifiers **in no way at all** — `beta` and `rc` share the
anchor, the base, the counter, the skipped changelog and the `prerelease: true` release object, differing
only in the string in the version.

**F10b · the identifier cannot be walked downwards (#85).** `1.5.0-rc.2` is open and a developer edits
`prerelease_identifier` from `rc` to `beta`. The computed `1.5.0-beta.3` sorts *below* the `1.5.0-rc.2` that
already shipped, so the run **hard-errors**, naming both versions. This is #76's monotonicity rule applied to
a computed version rather than a forced one. The reverse edit — `beta` to `rc` — is strictly greater and
passes: tightening a line's name toward release is legal, loosening it is not.

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
different identifiers is the same ASCII accident (`beta` < `rc`), which is why #85 checks monotonicity rather
than trusting the identifier to be chosen well.

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

**G2b · resolving it.** The operator decides the feature does belong on this line and re-runs with
`--release-as app=1.4.8`. The guard does not fire, because `Release-As` bypasses bump computation.

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

**G7 · release-branch arrangement (#81).** `release_branches = ["release/*"]` — main excluded. A push to main
exits `0` quietly (H1). `release/1.4` releases `1.4.1`. Assert main's `.cicada-versions.toml` is never
written.

**G7b · cutting a line needs no config commit (#82).** `release/1.4` is created with
`git checkout -b release/1.4 v1.4.7 && git push`. Assert cicada acts on it on the very next push — it matches
both `release_branches` and `maintenance_branches`, both declared once on `main` and inherited by the cut.
Assert the branch's `cicada.toml` is byte-identical to `main`'s.

**G7c · merging a release branch back carries no branch state.** `release/1.4` is merged into `main` (the
G9 prerequisite). Assert `main`'s `cicada.toml` is unchanged by the merge, and that `main` does not become
patch-only. The same assertion for `prerelease-v1.5-rc` → `main`: `main` must not start cutting rcs.

**G8 · release-branch arrangement, unmerged branch.** `release/1.5` is cut from main **without** merging
`release/1.4` back first. Main's versions file still says `1.4.0`, whose tag lives on the diverged
`release/1.4`. Anchor resolution finds `v1.4.0` unreachable from HEAD → hard error, exit `1` (J2). This is
the documented constraint of that arrangement, asserted so the error is recognisable rather than mysterious.

**G9 · release-branch arrangement, merged back.** Same as G8 but `release/1.4` was merged into main first.
`v1.4.1` is now an ancestor of main and of `release/1.5`; the anchor resolves, the range covers main's
accumulated commits, and the branch computes `1.5.0`.

**G5 · divergence is permanent and accepted.** After G1, `main`'s `CHANGELOG.md` never learns about `1.4.8`.
Asserted as expected behaviour, not a bug (future-work §10).

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

**I10 · title mode.** Under squash-merge the PR title is the only thing that lands on the default branch, so
`check` must be able to lint `--title` instead of the commits. The design routes this through
`commit_source = "title"`, which contradicts that setting's own enum (`commits` | `merges` | `both`) and
would be meaningless for history reading. **OPEN — Delta 9.**

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
    **quoted** (#51) · two changelogs, each containing only its own package's commits · versions file updates
    both entries
  - git · two tags: `v1.5.0` and `chart-v0.9.1`
  - forge · two `create_release` calls; **one** release manifest listing both (#42)

  This is the only dependency propagation in cicada, and it is *declared* in config, never inferred.

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

**M6 · derived bump on a never-released chart.** `app` at `1.4.2` releases `1.5.0`; `app-chart` has **no**
versions-file entry. A derived `patch` on an unreleased package would give `0.0.1` by the first-release
table (#37) — a plausible but odd first chart version. **OPEN — Delta 14.** Candidates: `0.0.1` (consistent
with #37), `0.1.0` (charts conventionally start there), or require `initial_version` and error without it.

**M7 · `on_tracked_bump = "minor"`.** Same as M1 but `app-chart: 0.9.0 → 0.10.0`.

**M8 · divergent anchors.** After M1, `app` is anchored at `v1.5.0` and `app-chart` at `chart-v0.9.1`, which
sit on the same commit. After M2, they diverge: `app`'s range starts earlier than the chart's. Assert one
`git log` per package with different ranges — the per-package anchor is the whole reason this works.

**M9 · `Release-As` does not cross packages (#39).** `feat(app): … Release-As: 2.0.0` sets `app` to `2.0.0`
exactly; the chart still takes its configured derived bump (`0.9.1`), not `2.0.0`.

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

**Applied to `docs/design.md` as decisions #73–#79, #83–#85.** Five remain open.

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
| 9 | `commit_source = "title"` contradicts its own enum; `check`'s title mode belongs on `--title` | **OPEN** |
| 10 | `check` exits `3` for semantic findings, `1` for a config that will not parse | **#79** |
| 11 | Forced-version monotonicity. Enforced by the staleness guard's one-sided comparison (J4b), not a separate rule. Two sub-questions still **OPEN**: is a prerelease value legal in `Release-As`, and does it force a release on a `bump = "none"` type? | **#76** |
| 12 | `{summary}` in `release_pr_title` has no defined rendering | **OPEN** |
| 13 | `find_pull_request` searches open PRs only | **#77** |
| 14 | Derived bump on a never-released tracked package lands on `0.0.1` | **OPEN** |
| 15 | `from` on a first release in `--json`: `null`, absent, or `"0.0.0"` | **OPEN** |
| 16 | A promoted prerelease line keeps a stale `prerelease` key until `main` is merged back, and cuts an rc *below* an already-shipped stable. No guard fires (F9) | **OPEN** |
| 17 | The counter rule (#63) is silent when the identifier changes under an unchanged base | **dissolved by #84** — one line, one identifier, so the base is the only axis that can change. The remaining case, an identifier *edited* mid-line, is caught by #85 (F10b) |

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
`commit_source` · `Release-As` · bump computation · first-release table · `pre_major` · the `maintenance` guard ·
prereleases (all of #59–#63, #84–#85, including promotion) · release manifest · all four updater kinds ·
both addressing modes · YAML
splice traps · changelog insertion · templates and filters · both phases · staleness (all three layers) ·
tag reconciliation · independent release reconciliation · version collision · maintenance branches ·
branch gating · offline modes · exit codes · `--json` schema · chart tracking.

**Not exercised here, by construction** — these need more than one service and move to
`docs/scenarios/monorepo.md`: umbrella aggregation (#10, #15), multi-scope commits (`feat(api,worker):`),
scope routing across many packages, `Release-As` hitting multiple packages at once, per-package release
timing divergence at scale, and the Cargo workspace-inheritance trap (#24).

**Not exercised anywhere yet:** the GitLab adapter's behavioural differences beyond the two asserted
fallbacks, and forge retry/backoff (#58). `cicada init` is gone — config authoring is a human job
(future-work §12).

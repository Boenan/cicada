# cicada — next steps (handoff)

Written at the end of the design session. Read `docs/design.md` first — this file only says *what to do
next* and *what is easy to get wrong*.

---

## Where things stand

| | |
|---|---|
| **Design** | Complete. `docs/design.md`, 97 numbered decisions, every section walked and argued through. **No open questions** |
| **Behavioural coverage** | `docs/scenarios/single-service.md` (~140 scenarios, groups A–N) and `docs/scenarios/monorepo.md` (groups O–Y, only what needs N > 1), in five-surface form, doubling as the acceptance suite. Four HTML walkthroughs under `docs/scenarios/single-service/` |
| **Deferred scope** | `docs/future-work.md`, 16 items, each with why it was deferred and what would trigger it |
| **M0 spike** | **Done and confirmed.** `spikes/yaml-splice`, 15 tests green |
| **M0** | **Done.** Two-crate workspace, config parsing, `GitBackend` + subprocess impl, conventional-commit parser, `cicada check`. 52 tests green |
| **Production code** | `cicada-core/` (pure) and `cicada/` (lib + bin). Next milestone is M1 |

**The scenario pass rewrote parts of the design.** Decisions #73–#85 all came out of writing expected
outcomes down, and four earlier decisions were reversed outright — see *Do not re-litigate* below, and note
that #2, #3, #6, #23 and #28 no longer say what an older reading of this file would suggest.

**A scope pass then took features out.** Forced versions (#39), release-branch and milestone-line
arrangements (#81), `check`'s title mode (#87) and a rendered release-PR title (#86) are all deferred to
`docs/future-work.md` §13–§16. That pass closed every remaining open question — four of the five dissolved
with the scope rather than needing a rule — and added #86–#91.

**A capability pass then went the other way.** Staged prerelease lines (`alpha` → `beta` → `rc`, #93–#94)
were added so cicada follows the release strategies repos actually run rather than the one it prefers, and
#92 recorded that "cicada always opens a PR" is compatible with continuous deployment through auto-merge. Of
the eighteen single-service release strategies catalogued in that pass, eleven are supported, four are
deferred with a recorded reason — human-stated versions (future-work §13), release-branch arrangements (§14),
non-semver schemes (§17), `-SNAPSHOT` conventions (§3) — and two are permanent non-goals: cadence releases
and PR-label-driven versioning. **None fails by omission.**

---

## M0 is done

**Exit criterion met:** `cicada check --base <ref> --head <ref>` lints a commit range with did-you-mean
suggestions and validates the config, exiting `0` clean / `3` on findings / `1` on a config that will not
parse. cicada checks its own repo — `cicada.toml` at the root is both its configuration and the worked
example an adopter copies from (#21).

What landed, and where:

| Piece | Location |
|---|---|
| Two-crate workspace, purity enforced by the manifest (#29) | `cicada-core/`, `cicada/` |
| Conventional-commit parser, multi-scope, `!`, `BREAKING CHANGE:` | `cicada-core/src/commit.rs` |
| Config types, `deny_unknown_fields`, defaults (#67, #83) | `cicada-core/src/config.rs` |
| Branch globs (#82) | `cicada-core/src/branch.rs` |
| Pure validations + did-you-mean (#12) | `cicada-core/src/check.rs` |
| `GitBackend` trait + `git` subprocess impl (#17) | `cicada/src/git/` |
| Filesystem validations, incl. the workspace trap (#24) | `cicada/src/config.rs` |
| CLI, exit codes (#68), `--json` schema (#69) | `cicada/src/main.rs` |

**Three things the build settled that the design had left implicit:**

1. **`PackageId` is an ordinal, not a name.** §Data flow shows `BTreeMap<PackageId, …>` annotated "ordered
   by config order" — which is only true if the key sorts in config order, and a `String` key sorts
   alphabetically. `PackageId(usize)` indexing into the config's `IndexMap` makes #31 mechanical rather than
   a convention to remember, and makes "umbrella last" (#96) fall out of `Target`'s derived `Ord`.
2. **Did-you-mean needs Damerau-Levenshtein, not plain Levenshtein.** A transposition costs 2 under plain
   Levenshtein, which is over the threshold for a short name — so `aip` → `api`, the design's own worked
   example in §`cicada check`, produced no suggestion at all. Found by running the example by hand; every
   unit test had passed, because they happened to use substitutions rather than transpositions.
3. **`cicada` is a lib **and** a bin.** Integration tests need to drive the real `GitBackend` against real
   repositories rather than a mock, and a bin-only crate cannot be imported.

**Test shape, for the milestones that follow:** 22 table tests on the parser and did-you-mean, 12 against
real temp repos for `GitBackend`, 15 end-to-end on `check` asserting **exit codes** rather than only output —
they are a contract (#68), and CI branches on them.

### ~~1. Workspace restructure~~ — done

Root `Cargo.toml` is currently a plain package. It needs to become a two-crate workspace (#29):

```
cicada-core/     PURE — no I/O dependencies in its Cargo.toml at all. This is enforced by
                 the manifest, and that enforcement is the entire reason for the split.
cicada/          the binary — git/, forge/, updaters/, cli/ as modules
spikes/          standalone, already carries its own [workspace] so it stays out
```

Do **not** split git/forge/updaters into separate crates. That was considered and rejected: they are all
I/O adapters, none will be consumed independently, and separate crates buy only `pub` noise.

### ~~2. Config parsing and validation~~ — done

Schema is fully specified in §Configuration. Two things that are easy to skip and shouldn't be:

- **`deny_unknown_fields`** (#67). A mistyped `tag_fromat` must be an error naming the key. Silently
  ignoring it produces a release that looks fine and is wrong.
- **The validation list** in §Configuration → *Loading and validation* is part of `check`, not a later
  nicety — duplicate tag formats, `default_scope` naming a real package, duplicate updater outputs, and
  the Cargo workspace-inheritance trap (#24).

### ~~3. `GitBackend` (trait + `git` subprocess impl)~~ — done

Needs: log walking with the three `commit_source` traversals (#40), tag list/resolve, ancestry checks
(`--is-ancestor`, for #3 and #9), shallow detection, and `git --version` at startup.

Subprocess discipline from the first line (§Tech choices): machine-readable formats only
(`--format=%H%x00%s%x00%b`, `-z`), pinned environment (`LC_ALL=C`, `GIT_CONFIG_NOSYSTEM=1`,
`GIT_TERMINAL_PROMPT=0`). Never parse human-facing output.

### ~~4. Conventional-commit parser~~ — done

Hand-written, not an off-the-shelf crate — the multi-scope extension (`feat(api,worker)!:`) is the reason.
Handles scopes, `!`, and `BREAKING CHANGE:`. Footers are parsed generically but **only `BREAKING CHANGE:` is
acted on** — `Release-As:` is out of scope (#39) and must be ignored rather than honoured or rejected, since
a repo migrating from release-please will have them in its history (E1). Lives in `cicada-core`, so it is a
pure `&str -> ParsedCommit` function and trivially table-tested.

### ~~5. `cicada check`~~ — done

Takes a **git range, not a forge PR object** — no API call, no token, identical on GitHub, GitLab and a
laptop. A range and nothing else: there is no `--title` flag and no title mode (#87). Levenshtein (`strsim`)
for did-you-mean over configured package names; that suggestion is what turns decision #11's hard error from
an obstacle into help.

---

## Constraints that are easy to violate

These are cheap to honour from the start and expensive to retrofit:

| # | Constraint |
|---|---|
| #29 | `cicada-core` has **no I/O dependencies in its manifest**. If you find yourself reaching for `std::fs` there, the code belongs in `cicada` |
| #31 | Ordered maps keyed by config order, **never `HashMap`** — iteration order is randomised per process and would make output non-deterministic (Goal 7) |
| #30 | Core emits `Vec<FileEdit>` as data; the I/O layer only writes them. `--dry-run` must be *the same computation with the write skipped*, never a parallel branch that describes what would happen |
| #16 / #38 | The package list is a `Vec` from line one. **Single-service is length 1, not a mode.** Building a single-package special case first means unwinding it later |
| #71 | `saphyr-parser`'s `Marker::index()` is a **char** index despite its doc comment. Convert to a byte offset, and keep the two non-ASCII regression tests |
| #68 | Exit codes are a contract: `0` ok (including "nothing to release") · `1` error · `2` usage · `3` check findings · `4` refused |
| #67 | Unknown config fields are errors |
| #83 | Defaults are tabulated in §Configuration, not discovered from the source. `release_branches`, `default_scope`, `forge.kind` and each package's `path` and `tag_format` are **required**. The two branch classifiers default to a convention (`maintenance/**`, `prerelease-*`, …) — safe only because they classify branches `release_branches` already admitted, so do not default the gate |
| #84 / #93 | One identifier **at a time**, not one forever. `prerelease_identifier` stays a scalar (never a per-branch map), but its value moves as a line matures: `alpha` → `beta` → `rc`. The counter therefore resets on a change to the base **or** the identifier (#63) — with only the base considered, `alpha.3` → `beta` silently publishes `beta.4`, and #85 does not catch it |
| #94 | Never accept a declared ladder of stages. Ordering comes from semver alone, so an edit that sorts downward (`preview` → `beta`) is an error — a declared order would bless a sequence consumers' resolvers disagree with |
| #75 | `.cicada-versions.toml` is the **only** source of versions. Never read a version back out of `Cargo.toml` or `Chart.yaml` to seed a computation — it breaks #30 and makes `--dry-run` a lie |
| #2 | The tag is the **only** anchor. No commit SHA is stored anywhere. A missing tag is a hard error, never a fallback |
| #78 | The staleness recomputation runs **only** when a tag is about to be created. Unconditionally it fires on every ordinary push after a release |
| #82 | Branch-*identity* settings are glob patterns matched against the current branch name, never scalars living on the branch. A scalar travels through a merge-back and silently reconfigures the target branch |
| #39 | No forced versions. A `Release-As:` footer is parsed like any other footer and **ignored** — never honoured, never an error. The maintenance guard has no escape hatch, and adding one is a scope change, not a fix |
| #88 | `initial_version` is required on a package with a `helm` updater, and `Chart.yaml` is still never read back (#75). The requirement exists so cicada never invents a chart's first version |
| #97 | A derived release still writes a changelog entry. It has no commits, so the body comes from `derived_from` in the notes context — never an empty entry, never a skipped one, and never prose emitted by core |
| #96 | Releases are created in config order, umbrella last. Deterministic ordering is what makes the forge-call log assertable, and the umbrella's body embeds a manifest describing the releases created before it |
| #95 | The date is an **input**, never a clock read inside `cicada-core`. It feeds changelog entry dates and nothing else, but an ambient read would make core impure and let `--dry-run` disagree with the run that follows it |

---

## Do not re-litigate these

Each looks arguable and was argued, at length. The reasoning is in `docs/design.md`; re-opening without new
information just costs a session:

- **The tag is the only anchor** (#2, #3). The versions file holds a version and nothing else. The SHA
  fallback and the descendant check are *gone* — the fallback silently continued with a wrong range, and the
  descendant check has been replaced by "reachable from HEAD", which needs no stored state and reuses #9's
  primitive. A missing tag is a hard error naming the repair.
- **Tagging is mandatory** (#6). `tagging = false` was removed: with no tag there is no anchor, and both
  forges hang release objects off a tag anyway.
- **Release detection by tag reconciliation, never commit messages** (#9). Immune to squash-merge and
  rewritten messages. Tag and release object reconcile *independently* (#56), or self-healing breaks.
- **Phase B before Phase A** (#35). The reverse re-releases just-merged work every cycle.
- **Scope-based attribution, unknown scope is a hard error** (#11). Blast radius is declared by the author;
  this is why dependency-graph propagation is a permanent non-goal rather than a deferral.
- **Three-layer staleness model** (#32–34). Merging a stale release PR ships commits with no changelog entry
  and no version effect — a `feat` released under a patch bump, invisibly. Layer 3 is now **recomputation**,
  not a stored SHA: replan from the previous tag and refuse if the recorded version is *lower* than the
  recomputed one. The comparison is deliberately one-sided, because over-bumping (a hand-edited versions
  file) is a supported workflow. `prev` is stable-only (#90).
- **The maintenance guard is a guard, not a clamp** (#28). A computed bump above patch on a
  `maintenance_branches` line is a hard error, not a silently reduced version. Clamping is precisely the harm
  #33 exists to prevent, and worst for `feat!:` — a breaking change delivered on a patch upgrade.
- **No `cicada init`** (future-work §12). Config authoring is a human job; scanning is opinion that has to be
  reviewed line by line anyway. `check` does the validation `init` used to warn about.
- **Prerelease is branch-level, `version` stays the last stable** (#59–61). Anything else breaks the anchor.
- **cicada always opens a release PR** (#92). A repo that wants no review gate auto-merges it; a direct-commit
  mode was considered and rejected, because it forks the write path and the staleness model to remove a step
  the forge already removes.
- **Cadence releases are a permanent non-goal** (§Non-goals). Running cicada on a schedule already works —
  the trigger has never been part of the gate (#26) — but a range with nothing releasable in it produces no
  release, forever. There is no number to compute, so cicada would have to invent one, and an empty release
  is noise in a version history. A repo wanting a version stamped by the calendar regardless of content wants
  CalVer (future-work §17), not a scheduled semver release.

---

## Nothing is open

The shape-2 walkthrough added one question and it is decided: a **derived** release's changelog entry is
written, and the notes context carries `derived_from` so the default template names the cause (#97). Core
supplies the fact, the template supplies the words — the same split as pre-grouped sections (#46).

The monorepo pass added two questions and both are decided: the release manifest is embedded in **every**
release a run creates rather than the umbrella's alone (#42 — the old rule assumed an umbrella that #73 makes
optional), and releases are created in **config order with the umbrella last** (#96), so the forge-call log is
deterministic and the umbrella's embedded manifest describes work already done.

The five items that previously stood here are decided too, and **four of them dissolved when scope came out**
rather than needing a new rule. Recorded in full in `docs/scenarios/single-service.md` §Deltas and
`docs/design.md` §Open questions:

| Was open | Now |
|---|---|
| `check`'s title mode | `check` lints a range only (#87); title linting → future-work §15 |
| `{summary}` rendering | The title is a plain string (#86); rendering → future-work §16 |
| A derived chart bump on a never-released chart | `initial_version` required on `helm` packages (#88) — unreachable |
| A promoted prerelease line cutting rcs below its stable | Accepted (#91); merging back is the operator's job |
| `from` on a first release in `--json` | `"from": null`, present (#89) |
| Two sub-questions on `Release-As:` | Gone with forced versions (#39) |

## What came out of scope, and why it matters to M1

Four features were removed rather than deferred quietly, and two of them change what M1 must **not** do:

- **Forced versions** (#39 → future-work §13). No `Release-As:` footer, no `--release-as`. The bump computed
  from commits and config is the only version there is. A `Release-As:` footer in history is **ignored**, not
  an error (E1) — a repo migrating from release-please will have them.
- **The maintenance guard has no escape hatch** (#28). This is the visible consequence of the above, and it
  is deliberate: a `feat:` on a patch-only line blocks that line until the commit leaves the range. Do not
  add a flag for it while implementing, however obvious it looks in the moment (B17, G2b).
- **Release-branch and milestone-line arrangements** (#81 → future-work §14). v1 supports trunk releases
  only. `release_branches` excluding `main` still works as a *gate* (G7); what does not exist is a way to
  state a new line's opening minor.
- **`check --title`** (#87 → §15) and **`{summary}`** (#86 → §16).

---

## Next: M1

Anchor resolution (per-package ranges, tag reachability), scope routing incl. `default_scope`, bump
computation (section `bump`, first-release table, `pre_major`, the `maintenance` guard), umbrella
aggregation, `Plan`. Exit criterion: `cicada plan --json` correct on a fixture **monorepo and a
single-package repo through the same code path**, fully offline.

The read half of `GitBackend` that M1 needs is already built and tested — `rev_parse`, `is_ancestor`, `tags`
— so M1 is core work with no new I/O surface.

`docs/design.md` §Milestones has M1–M8. Two waypoints worth knowing:

- **M3 is the first genuinely useful build** — `release-pr --no-push` produces a real local release commit
  you can inspect with `git show`, with no token and no upstream involvement. This is the dogfooding gate;
  everything before it is theory.
- **M5 is the first end-to-end release tool**, on GitHub.

---

## A note on method

Three real bugs were found during the design walkthrough (#3's equality check, the phase ordering, and #9's
broken self-healing claim). All three were the same shape: **a stated property contradicted by the algorithm
meant to deliver it**, and none was visible from reading the section it lived in — each surfaced only from
tracing a full sequence commit by commit. The spike then found a fourth (char-vs-byte indexing) that all the
ASCII tests passed straight through.

The scenario pass found three more of exactly the same shape, all by tracing rather than reading:

- **The old stale-merge check fired on every normal release.** It compared committer identity over
  `sha..HEAD`, which after any merge contains the merge commit — whose committer is the forge's web-flow
  identity, not cicada's. Squash-merge had the same problem. Removing the stored SHA dissolved it.
- **#34's motivating example did not hold.** "Close the PR after the only `feat:` was reverted" — but a
  revert does not remove the `feat` from `git log`, so the plan never empties. Revert handling is deferred
  (future-work §11) and #34's real triggers are config edits.
- **The prerelease merge-back claim was false.** An earlier draft held that merging a prerelease line into
  `main` "just works, because main's config has no `prerelease`" — except the merge brings the line's config
  file along, so main starts cutting rcs. Fixed by #82: branch *identity* is a glob matched against the
  current branch name, never a scalar living on the branch it describes.

Worth keeping that habit: when a section claims a property, trace the sequence that is supposed to produce
it. Writing expected outcomes down is the cheapest way to force the trace.

## Verifying the spike still works

```
cd spikes/yaml-splice && cargo test     # expect 15 passed
cargo run --example realworld           # expect exactly one changed line vs fixtures/values.yaml
```

# cicada — next steps (handoff)

Written at the end of the design session. Read `docs/design.md` first — this file only says *what to do
next* and *what is easy to get wrong*.

---

## Where things stand

| | |
|---|---|
| **Design** | Complete. `docs/design.md`, 85 numbered decisions, every section walked and argued through |
| **Behavioural coverage** | `docs/scenarios/single-service.md` — ~130 scenarios in five-surface form, doubling as the acceptance suite. Monorepo file not yet written |
| **Deferred scope** | `docs/future-work.md`, 12 items, each with why it was deferred and what would trigger it |
| **M0 spike** | **Done and confirmed.** `spikes/yaml-splice`, 15 tests green |
| **Production code** | **None yet.** `src/main.rs` is still `cargo new` output |

Committed: `docs/`. Untracked: `.gitignore`, `Cargo.toml`, `src/`, `spikes/`.

**The scenario pass rewrote parts of the design.** Decisions #73–#85 all came out of writing expected
outcomes down, and four earlier decisions were reversed outright — see *Do not re-litigate* below, and note
that #2, #3, #6, #23 and #28 no longer say what an older reading of this file would suggest.

---

## The next step: finish M0

The spike discharged M0's only unproven assumption (decision #49 — span-preserving YAML splice — holds;
`saphyr-parser` is the crate). Everything left in M0 is assembly of known parts.

**M0 exit criterion:** `cicada check --base <ref> --head <ref>` lints a commit range with did-you-mean
suggestions and validates the config, exiting `0` clean / `3` on findings.

### 1. Workspace restructure

Root `Cargo.toml` is currently a plain package. It needs to become a two-crate workspace (#29):

```
cicada-core/     PURE — no I/O dependencies in its Cargo.toml at all. This is enforced by
                 the manifest, and that enforcement is the entire reason for the split.
cicada/          the binary — git/, forge/, updaters/, cli/ as modules
spikes/          standalone, already carries its own [workspace] so it stays out
```

Do **not** split git/forge/updaters into separate crates. That was considered and rejected: they are all
I/O adapters, none will be consumed independently, and separate crates buy only `pub` noise.

### 2. Config parsing and validation

Schema is fully specified in §Configuration. Two things that are easy to skip and shouldn't be:

- **`deny_unknown_fields`** (#67). A mistyped `tag_fromat` must be an error naming the key. Silently
  ignoring it produces a release that looks fine and is wrong.
- **The validation list** in §Configuration → *Loading and validation* is part of `check`, not a later
  nicety — duplicate tag formats, `default_scope` naming a real package, duplicate updater outputs, and
  the Cargo workspace-inheritance trap (#24).

### 3. `GitBackend` (trait + `git` subprocess impl)

Needs: log walking with the three `commit_source` traversals (#40), tag list/resolve, ancestry checks
(`--is-ancestor`, for #3 and #9), shallow detection, and `git --version` at startup.

Subprocess discipline from the first line (§Tech choices): machine-readable formats only
(`--format=%H%x00%s%x00%b`, `-z`), pinned environment (`LC_ALL=C`, `GIT_CONFIG_NOSYSTEM=1`,
`GIT_TERMINAL_PROMPT=0`). Never parse human-facing output.

### 4. Conventional-commit parser

Hand-written, not an off-the-shelf crate — the multi-scope extension (`feat(api,worker)!:`) is the reason.
Handles scopes, `!`, `BREAKING CHANGE:`, and `Release-As:` footers. Lives in `cicada-core`, so it is a pure
`&str -> ParsedCommit` function and trivially table-tested.

### 5. `cicada check`

Takes a **git range, not a forge PR object** — no API call, no token, identical on GitHub, GitLab and a
laptop. Levenshtein (`strsim`) for did-you-mean over configured package names; that suggestion is what turns
decision #11's hard error from an obstacle into help.

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
| #84 | One prerelease line, one identifier. `prerelease_branches` is a glob list and `prerelease_identifier` a scalar — never a per-branch map, which would express a maturity ladder cicada cannot enforce |
| #75 | `.cicada-versions.toml` is the **only** source of versions. Never read a version back out of `Cargo.toml` or `Chart.yaml` to seed a computation — it breaks #30 and makes `--dry-run` a lie |
| #2 | The tag is the **only** anchor. No commit SHA is stored anywhere. A missing tag is a hard error, never a fallback |
| #78 | The staleness recomputation runs **only** when a tag is about to be created. Unconditionally it fires on every ordinary push after a release |
| #82 | Branch-*identity* settings are glob patterns matched against the current branch name, never scalars living on the branch. A scalar travels through a merge-back and silently reconfigures the target branch |

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
  recomputed one. The comparison is deliberately one-sided, because over-bumping (`Release-As`, a hand-edited
  versions file) is a supported workflow.
- **The maintenance guard is a guard, not a clamp** (#28). A computed bump above patch on a
  `maintenance_branches` line is a hard error, not a silently reduced version. Clamping is precisely the harm
  #33 exists to prevent, and worst for `feat!:` — a breaking change delivered on a patch upgrade.
- **No `cicada init`** (future-work §12). Config authoring is a human job; scanning is opinion that has to be
  reviewed line by line anyway. `check` does the validation `init` used to warn about.
- **Prerelease is branch-level, `version` stays the last stable** (#59–61). Anything else breaks the anchor.

---

## Still open

Five, all non-blocking and all recorded in `docs/scenarios/single-service.md` §Deltas:

1. **`check`'s title mode.** §`cicada check` routes it through `commit_source = "title"`, which contradicts
   that setting's own enum (`commits` | `merges` | `both`) and is meaningless for history reading. It belongs
   on the existing `--title` flag.
2. **`{summary}` in `release_pr_title`** has no defined rendering, for one package or for many.
3. **A derived chart bump on a never-released chart** (#22 meeting #37) lands on `0.0.1`, an odd first chart
   version. `0.1.0` or requiring `initial_version` are the alternatives.
4. **A promoted prerelease line is stale until `main` is merged back** (F9). The `prerelease` key is deleted
   by a commit on `main`, so the line still reads `version = "1.4.2"`, `prerelease = "1.5.0-rc.2"` and cuts
   `1.5.0-rc.3` — an rc for a version already shipped, sorting below it. No guard fires. Either accept it and
   document the merge-back, or extend #9's reachability primitive from the tag being created to the computed
   stable base.
5. **`from` on a first release in `--json`** (delta 15): `null`, absent, or `"0.0.0"`. A machine interface
   with a compatibility promise (#69) should not leave this to the implementation.

Two smaller ones on `Release-As:` — whether a prerelease value is legal there outside the §Prereleases flow,
and whether it forces a release when attached to a `bump = "none"` type.

---

## After M0

`docs/design.md` §Milestones, M1–M8. Two waypoints worth knowing:

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
- **The prerelease merge-back claim was false.** "Merging `next` into `main` just works because main's config
  has no `prerelease`" — except the merge brings `next`'s config file along, so main starts cutting rcs. Fixed
  by #82.

Worth keeping that habit: when a section claims a property, trace the sequence that is supposed to produce
it. Writing expected outcomes down is the cheapest way to force the trace.

## Verifying the spike still works

```
cd spikes/yaml-splice && cargo test     # expect 15 passed
cargo run --example realworld           # expect exactly one changed line vs fixtures/values.yaml
```

# cicada — next steps (handoff)

Written at the end of the design session. Read `docs/design.md` first — this file only says *what to do
next* and *what is easy to get wrong*.

---

## Where things stand

| | |
|---|---|
| **Design** | Complete. `docs/design.md`, 72 numbered decisions, every section walked and argued through |
| **Deferred scope** | `docs/future-work.md`, 10 items, each with why it was deferred and what would trigger it |
| **M0 spike** | **Done and confirmed.** `spikes/yaml-splice`, 15 tests green |
| **Production code** | **None yet.** `src/main.rs` is still `cargo new` output |

Committed: `docs/`. Untracked: `.gitignore`, `Cargo.toml`, `src/`, `spikes/`.

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

---

## Do not re-litigate these

Each looks arguable and was argued, at length. The reasoning is in `docs/design.md`; re-opening without new
information just costs a session:

- **Anchor = tag derived from version, SHA as fallback, descendant check not equality** (#2, #3). Equality
  was the original design and was a bug — the SHA is recorded pre-merge, the tag lands on the merge commit.
- **Release detection by tag reconciliation, never commit messages** (#9). Immune to squash-merge and
  rewritten messages. Tag and release object reconcile *independently* (#56), or self-healing breaks.
- **Phase B before Phase A** (#35). The reverse re-releases just-merged work every cycle.
- **Scope-based attribution, unknown scope is a hard error** (#11). Blast radius is declared by the author;
  this is why dependency-graph propagation is a permanent non-goal rather than a deferral.
- **Three-layer staleness model** (#32–34). Merging a stale release PR ships commits with no changelog entry
  and no version effect — a `feat` released under a patch bump, invisibly.
- **Prerelease is branch-level, `version` stays the last stable** (#59–61). Anything else breaks the anchor.

---

## Still open

One item, non-blocking: whether `cicada init` should ever propose a `next` branch stanza for prereleases.
Current answer is no — it emits a commented-out `prerelease` line, because a prerelease workflow is a
deliberate choice and not something to infer from a repo scan.

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

Worth keeping that habit: when a section claims a property, trace the sequence that is supposed to produce it.

## Verifying the spike still works

```
cd spikes/yaml-splice && cargo test     # expect 15 passed
cargo run --example realworld           # expect exactly one changed line vs fixtures/values.yaml
```

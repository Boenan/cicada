# cicada — next steps (handoff)

Written at the end of the design session. Read `docs/design.md` first — this file only says *what to do
next* and *what is easy to get wrong*.

---

## Where things stand

| | |
|---|---|
| **Design** | Complete. `docs/design.md`, 102 numbered decisions, every section walked and argued through. **No open questions** |
| **Behavioural coverage** | `docs/scenarios/single-service.md` (~140 scenarios, groups A–N) and `docs/scenarios/monorepo.md` (groups O–Y, only what needs N > 1), in five-surface form, doubling as the acceptance suite. Four HTML walkthroughs under `docs/scenarios/single-service/` |
| **Deferred scope** | `docs/future-work.md`, 16 items, each with why it was deferred and what would trigger it |
| **M0 spike** | **Done and confirmed.** `spikes/yaml-splice`, 15 tests green |
| **M0** | **Done.** Two-crate workspace, config parsing, `GitBackend` + subprocess impl, conventional-commit parser, `cicada check`. 52 tests green |
| **M1** | **Done.** Versions file, scope routing, bump computation, chart tracking, umbrella aggregation, anchor resolution, `Plan`, `cicada plan [--json]` |
| **M2** | **Done.** Updaters (cargo, helm, generic path + marker, template), templates with three contexts + filters + `template print`, changelog insertion, versions-file write, the `FileEdit` boundary, `release-pr --dry-run` |
| **M3** | **Done.** The git write path: release PR branch, committer identity, the #32 refusal, the #80 collision check, `--no-push` end to end |
| **M4** | **Built; one half unverified.** `Forge` trait + GitHub adapter, `GIT_ASKPASS` auth, fetch/push with a lease, the staleness model's forge half. 251 tests green. **Real HTTP against github.com is not yet exercised** — see below |
| **Production code** | `cicada-core/` (pure) and `cicada/` (lib + bin). Next milestone is M5 |

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

## M1 is done

**Exit criterion met:** `cicada plan --json` is correct on a fixture monorepo *and* a single-package repo
through the same code path, fully offline — no token, no network, no forge. Both fixtures are driven through
the real binary against real git repositories in `cicada/tests/plan_cmd.rs`, because anchor resolution,
reachability and per-package ranges are precisely the parts a pure test cannot reach.

cicada plans its own release: `cicada plan` on this repo reports `cicada — → 0.1.0 minor (first release)`.

What landed, and where:

| Piece | Location |
|---|---|
| `.cicada-versions.toml` shape — a version and nothing else (#1, #4, #60) | `cicada-core/src/versions.rs` |
| Scope routing, multi-scope, `default_scope` (#11, #38) | `cicada-core/src/route.rs` |
| Bump computation, first-release table, `pre_major` (#36, #37) | `cicada-core/src/bump.rs` |
| `Plan`, umbrella aggregation, chart tracking, maintenance guard (#10, #22, #28, #96, #97) | `cicada-core/src/plan.rs` |
| Anchor resolution, per-package ranges (#2, #3) | `cicada/src/history.rs` |
| `cicada plan`, `--date`, `--json` schema 1 (#69, #89, #95) | `cicada/src/main.rs` |

**Four things the build settled that the design had left implicit:**

1. **The scope rule had to be narrowed, and the design contradicted itself on it** — now #98. `chore(deps):`
   is this design's canonical non-releasable example, used in four places, and `deps` names no package in any
   repo. Under a blanket reading of #11 the design's own worked example was a hard error, and every repo with
   a dependency bot would have been unable to release. Found by writing B8/B9 as tests: `b1_to_b10` failed on
   the row the design uses to *explain* the feature. An unknown scope is now fatal only on a commit that can
   release; `check` reports the rest as a **warning**.
2. **`Release.commits` holds every routed commit, not only the releasable ones.** `bump` and *renders in the
   changelog* are two questions (#36), and `docs` is `bump = "none"` with a section title — so filtering to
   releasable commits here would silently empty the Documentation section at M2.
3. **Chart tracking runs to a fixpoint, not a single pass.** Whether a chart fires depends on whether the
   package it tracks releases, and a single pass makes that depend on config order — which #31 exists to stop
   mattering. Bumps only increase and are bounded by major, so a fixpoint terminates.
4. **`plan` is not gated on `release_branches`.** The gate decides where cicada may *write* (#26); `plan`
   writes nothing and answers a question, and refusing to answer it on a feature branch would make the
   offline rehearsal useless exactly where it is wanted. It prints a note when the branch is outside the
   gate, because a `run` that exits 0 silently is worse than a line of output.

### The one gap: prereleases

A branch matching `prerelease_branches` **refuses** with a clear error rather than planning. This is
deliberate and it is the only thing in M1 that is not finished.

Prerelease decoration is not in M1's stated content, but a prerelease branch would otherwise compute a
*stable* version — a **wrong answer**, not an incomplete one, and a `plan` that lies offline defeats the
point of it running offline (§Goals 6). Refusing is loud; computing quietly would not be. The refusal is one
`if` in `cmd_plan`, marked in the source, and deleting it is what lands the feature.

What it needs when picked up, all fully specified already: the counter rule (#63 — reset on a change to the
base **or** the identifier), monotonicity against the recorded open prerelease (#85), and the anchor staying
the last *stable* (#61). The versions file already parses the `prerelease` field, so nothing is blocked.

---

## M2 is done

**Exit criterion met:** `cicada release-pr --dry-run` prints exactly the diff it would write, and the
working tree is asserted unchanged afterwards — a dry run that writes is not a dry run. Duplicate-output and
post-write re-parse checks both fire.

What landed, and where:

| Piece | Location |
|---|---|
| `FileEdit` — the compute/perform boundary (#30) | `cicada-core/src/edit.rs` |
| Versions-file writer (#1, #75) | `cicada-core/src/versions.rs` |
| `cargo` updater, incl. the workspace redirect (#24) | `cicada-core/src/updater/cargo.rs` |
| `helm` updater, `appVersion` always quoted (#51) | `cicada-core/src/updater/helm.rs` |
| YAML byte-splice, promoted from the spike (#49, #71, #72) | `cicada-core/src/updater/yaml.rs` |
| TOML / JSON path editing, marker comments (#50) | `cicada-core/src/updater/{toml_path,json,marker}.rs` |
| minijinja env, `\| yaml`/`\| json`/`\| md`, built-ins (#43–#48) | `cicada-core/src/template.rs`, `src/templates/` |
| The three contexts and `LinkStyle` (#46, A2) | `cicada-core/src/notes.rs` |
| Changelog marker / prepend / create (#5) | `cicada-core/src/changelog.rs` |
| `Plan` → `Vec<FileEdit>` (#30, #99) | `cicada-core/src/render.rs` |
| Reading sources, workspace roots, overrides, links | `cicada/src/prepare.rs` |
| `release-pr --dry-run`, `template print`, the diff | `cicada/src/main.rs`, `src/report.rs` |

**Four things the build settled that the design had left implicit:**

1. **Updater edits fold** — now #99. `FileEdit` carries *final content*, so two updaters on one file would
   mean last-writer-wins, silently. But two surgical edits on one `values.yaml` is an ordinary thing to want,
   so each transforms what the run has so far, in config order. #44's hard error narrows to the case that
   genuinely cannot fold: a `template` sharing a path with anything.
2. **`edit` is a typed enum and is required on `generic`.** It was an untyped `toml::Value`, which meant a
   misspelled addressing mode was a runtime surprise rather than a parse error (#67). A `generic` updater
   with no `edit` names a file and no value in it — a silent no-op, now a `check` finding.
3. **The cargo workspace redirect needs a flag, not a re-derivation.** Whether to write `[package] version`
   or `[workspace.package] version` cannot be recovered from the file's contents: a workspace root may be a
   real package too, carrying both keys. Whether the redirect happened is knowledge the caller has and the
   content does not.
4. **`prepare::sources` must read the redirect *target*.** Found by dogfooding — cicada's own repo is exactly
   this shape, and `release-pr --dry-run` failed with `no value at the file itself` because only the declared
   path had been read.

**Two formatting bugs the tests caught, both in the claim #49 rests on:**

- **`*item = value(new)` deletes the trailing comment on the edited line.** A `toml_edit` `Value` owns its
  surrounding decor, so assigning a fresh one drops it — on the one line a reader is most likely to have
  annotated. Both the `cargo` and `generic (toml)` paths had it; `replace_preserving_decor` is the fix and
  the fixtures now carry a comment *on the version line* rather than only near it.
- **A template's leading newline widened every changelog by a blank line.** The frame owns the spacing around
  an entry, so the rendered body is trimmed at both ends and a template's whitespace control cannot leak.

---

## M3 is done — and it is the first genuinely useful build

**Exit criterion met:** `cicada release-pr --no-push` produces a real local release commit on
`cicada/{branch}`, inspectable with `git show`. No token, no network, nothing pushed. cicada does this on its
own repo: `git show cicada/main` shows `CHANGELOG.md` and `.cicada-versions.toml` at `0.1.0`, authored by
`cicada[bot]`.

What landed, and where:

| Piece | Location |
|---|---|
| Commit identity, parsed and validated (#55) | `cicada-core/src/identity.rs` |
| `commit_edits`, `branch_tip`, `committer_of` — the write half | `cicada/src/git/{mod,subprocess}.rs` |
| The #32 refusal, the #80 collision check, the commit message | `cicada/src/main.rs` |

**The write path does not check anything out** — now #100, and the most consequential thing this milestone
settled. The obvious implementation (check out `cicada/main`, write, commit, check back) fails three ways:
`--no-push` is the *local rehearsal* mode, so a developer must get their tree back; a failure between the two
checkouts strands the run on the wrong branch; and two overlapping Phase A jobs in one workspace (D6) would
corrupt each other. Instead a **private index** is seeded from HEAD's tree, the edits are hashed into it, and
`commit-tree` + `update-ref` do the rest. A test asserts the property that buys: a developer's staged and
unstaged work is never consulted, so uncommitted edits cannot be swept into a release.

**Three things the build settled that the design had left implicit:**

1. **File modes come from the base tree.** Hard-coding `100644` in `update-index` silently un-executes a
   shell script — and a script carrying a marker comment (#50) is exactly the file most likely to be
   executable. Caught by a test that sets the bit on disk; an earlier version of that test set it in the
   index, where a later `git add -A` undid it and the test passed for the wrong reason.
2. **A run that changes nothing must create nothing.** The timestamp alone guarantees a new sha for
   identical content, so without a tree-and-parent check every re-run force-pushes a fresh commit and "did
   anything change?" stops being answerable from outside.
3. **Both refusals fire under `--dry-run` too.** A rehearsal that reports success where the real run would
   refuse is a rehearsal that lies, which is the same standard #30 sets for the diff.

**One real bug, found by a test that had been passing:** the idempotency check asked `rev_parse` for a
*tree*, and `rev_parse` peels `^{commit}` — so it resolved nothing and the comparison never matched. The CLI
test passed anyway, because two runs inside one second produce the same sha regardless. The direct
`GitBackend` test is what caught it, and the CLI test now asserts the output wording rather than only the sha.

---

## M4 is built, and half of its exit criterion is unverified

The criterion is *"Real release PR on GitHub; hand-editing the release PR branch makes cicada refuse, draft
the PR, and fail loudly."* The second half is done and tested. **The first half is not**: creating a pull
request on a real repository needs a token and an account, so it has not been exercised. Everything below it
has:

| Surface | How it is verified |
|---|---|
| The three forge protocols (#34, #33 layer 2, #77) | Asserted against a recording fake on the *ordered list of calls* — including calls that must **not** happen (D5a) |
| Error mapping (#58) | Unit tests on the mapper: a 403 names `pull-requests: write`, a 404 says a private repo answers 404 rather than 403 |
| Retry policy (#58) | Unit tests: 5xx and 429 only, `Retry-After` honoured and capped |
| `fetch_branch` / `push_branch` / the lease | Against a **real bare repo**, including a genuine lost-lease race |
| `GIT_ASKPASS` | End to end through the real binary, plus a check that the token never reaches `--help` or `-vvv` |
| The GitHub HTTP calls themselves | **Not verified.** Written against the documented API; unproven until run |

What landed, and where:

| Piece | Location |
|---|---|
| `Forge` trait, types, retry policy (#53, #58) | `cicada/src/forge/mod.rs` |
| GitHub adapter, REST + one GraphQL mutation | `cicada/src/forge/github.rs` |
| The three protocols, unit-tested | `cicada/src/forge/flow.rs` |
| Recording fake | `cicada/src/forge/fake.rs` |
| Token, `GIT_ASKPASS`, SSH→HTTPS (#18) | `cicada/src/auth.rs` |
| `fetch_branch`, `push_branch`, `--force-with-lease` | `cicada/src/git/` |

### The bug M3 shipped, found while wiring the push

**#32's guard was inert in CI** — the only place it matters. `branch_tip` reads `refs/heads/cicada/main`, and
a fresh CI clone has no such local branch. So the guard saw `None`, found no human commit, and would have
force-pushed straight over one. The branch at risk is the **remote** one; that is now what gets fetched and
judged (#101), and the push carries `--force-with-lease` against exactly the sha that was read, so the window
cicada cannot see is closed by git rather than by luck.

This is the same shape as every other bug these docs record: a stated property contradicted by the algorithm
meant to deliver it, invisible from reading the section it lived in, and surfaced only by tracing what
actually happens in the environment the feature exists for.

### The design was optimistic about GitHub drafting

#54 calls it "a real API field (`draft: true`)". That is true at **creation** and false afterwards — `PATCH
/pulls/{n}` has no `draft` parameter, and converting an existing PR is the GraphQL mutation
`convertPullRequestToDraft`. Staleness layer 2 drafts a PR that already exists, which is exactly the case
REST cannot express. Recorded as #102; the visible consequence is that `set_draft` takes the whole
`PullRequest` rather than a number, because a node id is needed to name the thing being changed.

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
| #101 | The staleness guard's subject is the branch **about to be overwritten** — remote when pushing, local otherwise. Reading only `refs/heads/` makes it inert in CI, which is the one environment it exists for |
| #18 | The token never reaches `argv`, `.git/config`, or a log. `Token` has no `Debug`/`Display` on purpose; adding one puts a credential in a CI log the first time someone writes `tracing::debug!(?token)` |
| #100 | The release commit is built through a **private index**, never a checkout. The working tree, the real index and the current branch stay exactly as they were. Reintroducing a checkout breaks local rehearsal, error unwinding and concurrent runs at once |
| #32 | The staleness refusal and the #80 collision check run under `--dry-run` as well. A rehearsal that reports success where the real run refuses is worse than no rehearsal |
| #99 | Updater edits **fold**, one `FileEdit` per path, in config order. Emitting one edit per *updater* reintroduces last-writer-wins — the exact bug #44 names — and it is silent. The hard error is only for a whole-file writer sharing a path |
| #52 | Every edited file is re-parsed **before anything is written**. The check is inside the transform, so a bad splice cannot reach the working tree even on the path that writes without committing |
| #98 | An unknown scope is fatal only on a commit that **can release**. Widening it back to every commit breaks every repo whose dependency bot writes `chore(deps):` — including this design's own worked example (B8/B9). Narrowing it further, so a releasable commit's bad scope is tolerated, is the silent misfiling #11 exists to prevent |
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

## Next: verify M4, then M5

**First, the gate M4 has not passed.** Run it against a real repository with a token:

```
CICADA_TOKEN=… cicada release-pr        # on a release branch, with something to release
```

and check three things: the pull request opens with the notes as its body, a second run *updates* that same
PR rather than opening another, and hand-editing `cicada/<branch>` then re-running drafts the PR, labels it
`cicada:stale`, comments, and exits `4`. Those are the assertions the fake already makes; what is unproven is
the HTTP underneath them. Expect the first failures to be in the places adapters always fail — the `head`
qualifier on the PR search, the GraphQL mutation shape, and error bodies that are not JSON.

**Then M5**, which is the first end-to-end release tool: Phase B — tag reconciliation with the reachability
and collision checks, **independent** release-object reconciliation (#56), the stale-merge recomputation
(#33 layer 3, #78, #90), the release manifest (#42, #57), and `cicada run` doing Phase B **before** Phase A
(#35).

Three things M4 leaves ready for it:

- **`find_release` and `create_release` already exist** on the trait and in both the adapter and the fake,
  and `find_release` returns `Ok(None)` on a 404 — absent is an answer, which is what makes tag and release
  reconcile independently (#56).
- **The notes are already rendered** and reused verbatim (#43), so the release body must take
  `Rendered.notes`, not render again.
- **`cicada run` needs Phase A and Phase B composed in one place.** That is the moment to lift the
  orchestration out of `main.rs` the way the forge protocols were lifted into `forge/flow.rs` — CI must not
  be able to express the broken order (#35), and a function that runs them in sequence is what guarantees it.

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

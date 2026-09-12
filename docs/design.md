# cicada — design

A release automation tool in the spirit of [release-please](https://github.com/googleapis/release-please),
built in Rust, git-forge agnostic, for monorepos of microservices and single-service repos alike.

## Goals

1. Written in Rust, distributed as a Linux CLI binary and a multi-arch Linux Docker image.
2. One code path for single-service repos and monorepos with many services.
3. Forge-agnostic core; GitHub and GitLab adapters at the edge; others addable.
4. Conventional-commit driven versioning, changelogs, and release PRs/MRs.
5. Shipped CI templates for GitHub Actions and GitLab CI.
6. **`plan` and `check` run fully offline** — no token, no network, no forge. Releases can be rehearsed
   locally before anything is pushed, and `check` works on untrusted forks where no token exists.
7. **Deterministic and idempotent** — the same repo state produces the same output, and a second run
   changes nothing. A constraint on every feature, not an emergent property.

## Non-goals (v1)

- Publishing packages to registries (crates.io, npm, Maven Central). cicada tags and releases; CI publishes.
- Go, Java, and Python version-file updaters. **v1 is Rust and Helm only.** The `generic` and `template`
  updaters cover other ecosystems in the meantime; new `kind` values are additive (#13).
- **Config generation.** There is no `cicada init`. The developer writes `cicada.toml` and, when adopting
  into a repo with existing history, seeds `.cicada-versions.toml` by hand. Scanning a repo to guess its
  package layout is opinionated, imperfect, and reviewable only by the person who already knows the answer
  (future-work §12).
- Dependency-graph propagation inferred from Cargo/npm/Maven manifests. Blast radius is *declared*
  by the author via multi-scope commits (`feat(auth,api,gateway):`), and the one declared exception is
  Helm `appVersion` tracking (#22).
- Signed/verified commits (would require a forge-API commit backend; see Future).

---

## How cicada differs from release-please

release-please never touches git. It reads history over the GitHub GraphQL API and writes commits over
the REST API (`createTree` → `createCommit` → `updateRef`). That is *why* it is GitHub-only: its entire
I/O layer is Octokit.

cicada inverts this. **Git is the substrate; the forge API is a thin edge.** History reading, version
anchoring, tagging, and release detection are all pure git. The forge API is used for exactly two things:
opening/updating a PR/MR, and creating a release object. That is what makes GitLab, Gitea, and Bitbucket
adapters cheap instead of a rewrite.

| | release-please | cicada |
|---|---|---|
| History source | GitHub GraphQL API | local git |
| Commit creation | GitHub REST API | `git commit` + `git push` |
| Release detection | PR labels (`autorelease: pending`/`tagged`) | tag reconciliation |
| State | `.release-please-manifest.json` | `.cicada-versions.toml` |
| Forges | GitHub only | GitHub, GitLab (trait-based) |
| Working tree | not needed | required |

---

## Decision log

| # | Decision |
|---|---|
| 1 | Two files: `cicada.toml` (human-owned config), `.cicada-versions.toml` (tool-owned versions — a version and nothing else) |
| 2 | Commit-range anchor: derive the tag from the current version and resolve it. **The tag is the only anchor** — no commit SHA is recorded anywhere. Missing tag → hard error |
| 3 | The anchor tag must be **reachable from HEAD**; existing-but-unreachable is a hard error. Same primitive as #9, no stored state |
| 4 | "Never released" = **absent entry** in the versions file. No sentinel version |
| 5 | Changelog inserted at `<!-- cicada:insert -->`; fallback prepend-after-header; file created if absent |
| 6 | Tags are write-only and `tag_format` is configurable. **Tagging is mandatory** — the tag *is* the anchor, so a repo that does not tag has no way to compute a range |
| 7 | Release **PR/MR flow** — humans review the diff before anything is tagged |
| 8 | Monorepo packages are an **explicit list** in `cicada.toml`, written by a human. Nothing is discovered by scanning |
| 9 | Release detection by **tag reconciliation**, never by commit message. A tag counts as released only if it is **reachable from HEAD**; existing-but-unreachable is a version collision (hard error) |
| 10 | Independent per-package versions **+ umbrella version (semver-aggregate)** + a release-manifest artifact |
| 11 | Attribution by **conventional-commit scope**; multi-scope `feat(api,worker):` supported; unscoped → umbrella; unknown scope → hard error |
| 12 | `cicada check` validates PR commits/title before merge |
| 13 | A package declares a **list of updaters**, not a single `type`. v1 kinds: `cargo`, `helm`, `generic`, `template` |
| 14 | A Helm chart is its **own package** with its own version; its `appVersion` *tracks* a named app package |
| 15 | Umbrella gets its own tag and root `CHANGELOG.md` grouped by service; per-service changelogs live in package dirs |
| 16 | Single-service repo is the degenerate monorepo: one package, unscoped commits normal, same code path |
| 17 | Git via **shelling out to `git`** behind a `GitBackend` trait; `git` is a required runtime dependency |
| 18 | Auth is **token-only**, always: `--token` or `CICADA_TOKEN`, never persisted, injected via `GIT_ASKPASS` |
| 19 | Forge selected by explicit flag/config (`--forge github\|gitlab`), since the token is provider-agnostic |
| 20 | v1 flags: `--dry-run`, `--json`, `--release-as`, `Release-As:` footer, prerelease support |
| 21 | cicada releases itself with cicada |
| 22 | An app release forces a derived bump of any chart tracking it (default `patch`, configurable) |
| 23 | *(withdrawn — `cicada init` is deferred; see future-work §12. Number retained so earlier references stay valid.)* |
| 24 | Cargo workspace-inherited versions (`version.workspace = true`): `check` and `plan` hard-error if two packages resolve to the same version field |
| 25 | `plan`/`check` need no token and no network; `release-pr`/`release` take `--no-push` for local rehearsal |
| 26 | cicada acts on the **currently checked-out branch**, gated by a `release_branches` allowlist; a non-matching branch exits 0 quietly |
| 27 | The PR head is `release_pr_branch`, templated per target branch (`cicada/{branch}`); the base is the branch cicada ran on. The default prefix is a **namespace, not a label**: the head must not match any `release_branches` pattern, or cicada's own push re-triggers it. **Terminology: "release branch" always means one of `release_branches`; cicada's own is the "release PR branch"** |
| 28 | `maintenance_branches` marks matching branches as patch-only. It is a **guard, not a clamp**: a computed bump above patch is a hard error (exit `4`), never a silently reduced version |
| 29 | Two crates: `cicada-core` (pure, no I/O deps — purity enforced by the compiler) and `cicada` (binary; git, forge, updaters as modules) |
| 30 | Core emits `Vec<FileEdit>` as **data**; the I/O layer only writes them. `--dry-run` is therefore not a separate code path |
| 31 | Ordered maps keyed by config order, never `HashMap` — output determinism is observable |
| 32 | Cicada **replaces** the release PR branch each run. If its tip is not cicada's own commit, cicada refuses to clobber: drafts the PR, labels `cicada:stale`, comments, exits non-zero |
| 33 | Stale-merge prevention in three layers: branch protection (documented) → cicada auto-drafts (enforced) → Phase B **recomputes the plan** and refuses if the recorded version is lower than the recomputed one (stateless backstop) |
| 34 | Empty plan while a release PR is open → close the PR |
| 35 | **Phase B runs before Phase A.** `cicada run` does both in order so CI cannot wire it backwards |
| 36 | Releasable types are config-driven: each changelog section carries `bump = "minor" \| "patch" \| "none"`. `chore`/`docs` are `none`, so cicada itself declines to release |
| 37 | An unreleased package is treated as `0.0.0` under strict semver: breaking → `1.0.0`, feat → `0.1.0`, fix → `0.0.1`. Thereafter `pre_major = "minor"` keeps 0.x inside 0.x |
| 38 | `default_scope` names the package that receives unscoped commits — the umbrella in a monorepo, the sole package in a single-service repo |
| 39 | `Release-As:` applies to the commit's own scopes (unscoped → `default_scope`); two conflicting values for one package is a hard error; CLI form is repeatable `--release-as pkg=version` |
| 40 | History reading: `commit_source = "commits"` (`--no-merges`, default) \| `"merges"` (`--first-parent`) \| `"both"`. Identical on squash/rebase repos; only affects merge-commit repos |
| 41 | Release PR title defaults to `chore(release): …` — conventional (survives commit linting) and non-releasable (#36) |
| 42 | One release manifest per run, written to `target/cicada/release-manifest.json` (gitignored) **and** attached to the umbrella's release |
| 43 | **One** notes template produces the entry body, reused by both the changelog and the forge release; per-target override optional |
| 44 | Template/output paths resolve from the **repo root**; two updaters writing the same output path is a hard error |
| 45 | Autoescape **off** (Markdown/YAML output, not HTML); `\| yaml`, `\| json`, `\| md` filters provided for embedding commit text safely |
| 46 | Section grouping stays config-driven; templates receive **pre-grouped** sections and decide only how to render them |
| 47 | Template overrides live at `.cicada/templates/<name>.j2` by convention, overridable in config |
| 48 | `cicada template print <name>` emits the built-in default, so it can be copied and edited rather than reverse-engineered |
| 49 | Structured-format edits use a **span-preserving parser + byte splice** — never parse-and-reserialize, never line-walking |
| 50 | Two addressing modes: **path** (structured formats) and **marker comment** (any text file) |
| 51 | Helm `appVersion` is always written **quoted**; an unquoted value found in place warns |
| 52 | Every written file is **re-parsed after the edit**; a parse failure aborts before anything is committed |
| 53 | `Forge` is nine methods, not three (PR find/create/update/close/draft/comment/labels, release find/create) |
| 54 | `set_draft` is forge-specific (GitHub field, GitLab `Draft:` title prefix); fallback where unsupported is title prefix + label + a failing `cicada/freshness` status |
| 55 | Cicada's committer identity is configurable and **stable across versions** — decision #32's guard depends on it |
| 56 | Phase B reconciles **tag and release object independently**; a tag that exists without a release still creates the release |
| 57 | Release manifest is embedded in the release body as fenced JSON **and** written to disk; no asset upload in v1 |
| 58 | Forge HTTP: retry with backoff on 5xx/429 (honouring `Retry-After`), never on 4xx; errors mapped to actionable text |
| 59 | Prerelease is **branch-level**, not per-package: `prerelease_branches` is a glob list, exactly like `maintenance_branches`, and `prerelease_identifier` names what its prereleases are called. Plus `--prerelease` for ad-hoc use |
| 60 | Versions file gains a `prerelease` field; `version` keeps its exact meaning — **last stable release** — so anchor semantics are untouched |
| 61 | Every release, rc and stable alike, computes from the last **stable** anchor: notes are cumulative and the stable changelog is complete |
| 62 | Prereleases write version files and cut tags + forge releases, but **never write `CHANGELOG.md`** |
| 63 | The rc counter increments while the computed base is unchanged; a changed base (`1.5.0` → `2.0.0`) resets it to `.1` |
| 64 | Prerelease identifiers are free strings with dotted numbering (`1.5.0-rc.1`) — sorts numerically per semver |
| 65 | Determinism is a **stated goal** (§Goals 7), not an emergent property |
| 66 | `forge.kind` is **required**, written by a human. cicada never infers it from the remote URL; a missing value errors naming the value it would have detected, ready to paste |
| 67 | Config parsing **denies unknown fields** — a mistyped key is an error, never a silent no-op |
| 68 | Exit codes are a contract: `0` ok · `1` error · `2` usage · `3` check found problems · `4` refused/blocked |
| 69 | `--json` output carries a `schema` version field; it is a machine interface with a compatibility promise |
| 70 | Cicada owns versioning, tagging and releases; **artifact building is a separate tag-triggered workflow** |
| 71 | YAML splicing uses `saphyr-parser` — **spike-confirmed** (`spikes/yaml-splice`). `Marker::index()` is a **char** index despite its doc comment; it must be converted to a byte offset or non-ASCII files corrupt silently |
| 72 | The path walker **tolerates unsupported constructs outside the addressed path** (aliases, complex keys); only an unaddressable *target* is an error |
| 73 | `[umbrella]` is **optional**. cicada never assumes an aggregate version; the developer declares one when they want it. A single-service repo has none |
| 74 | An app and its Helm chart in one repo are **two packages with independent version lines** (#14) — never an umbrella and a child |
| 75 | `.cicada-versions.toml` is the **sole source of truth** for versions. Version files (`Cargo.toml`, `Chart.yaml`, …) are pure outputs and are never read back to seed a computation |
| 76 | Any human-forced version (`Release-As:`, `--release-as`, a hand-edited versions file) must be **strictly greater** than the recorded one |
| 77 | `find_pull_request` searches **open PRs only**; a closed release PR is never reopened. Idempotency rests on the head branch name, not on PR identity |
| 78 | The stale-merge recomputation runs **only when a tag is about to be created** — never on an ordinary push, where the range legitimately holds new commits |
| 79 | `check` exits `3` for semantic findings (config or commits) and `1` for a config that will not parse |
| 80 | The version-collision check (#9) runs in **Phase A as well as Phase B** — a computed version whose tag already exists unreachably refuses to open a PR, rather than failing after review and merge |
| 81 | Two release arrangements are supported and named — **trunk releases** and **release-branch releases**. cicada has no `release_strategy` setting: the difference is entirely expressed by `release_branches` and `maintenance_branches`, and a preset over existing keys would unlock no behaviour |
| 82 | Branch-*identity* settings are declared as **glob patterns matched against the current branch name** (`release_branches`, `maintenance_branches`, `prerelease_branches`), never as scalar values living on the branch they describe. Config is still read from the current branch (#26) — the patterns make it identical on every branch, so cutting a line needs no config commit |
| 83 | `release_branches` is **required** — cicada refuses to guess where it may write. The two branch *classifiers* default to a convention instead: `maintenance_branches = ["maintenance/**", "maintenance-*"]` and `prerelease_branches = ["prerelease/**", "prerelease-*"]` with `prerelease_identifier = "rc"`. Safe because they only ever classify branches `release_branches` already admitted. Every default is tabulated in §Configuration, so "what happens if I omit this" is never answered by reading the source |
| 84 | **One prerelease line, one identifier.** A repo has at most one kind of prerelease in flight; `beta` feeding `rc` is not an arrangement cicada expresses. A prerelease line has one function, so the identifier is a single scalar and not a per-branch mapping — which also removes any precedence rule between patterns. Changing the identifier mid-line is a config edit, guarded by #85 |
| 85 | A computed prerelease must be **strictly greater** than the open one recorded in the versions file — #76's monotonicity rule applied to computed versions, not only forced ones. It is what catches an identifier edited from `rc` to `beta` mid-line, which would otherwise compute a version sorting *below* what already shipped |

---

## Architecture

```
cicada-core   PURE. no I/O, no network, no filesystem — enforced by its Cargo.toml
              commit model · conventional-commit parser · scope routing
              bump computation · umbrella aggregation · changelog rendering
              updater transforms (pure str → str)
              Plan · FileEdit types

cicada        the binary. everything that touches the world.
              git/       GitBackend trait + `git` subprocess impl
              forge/     Forge trait + github, gitlab adapters
              updaters/  thin file read/write around core's pure transforms
              cli/       clap commands, output rendering, exit codes
```

**Two crates, not five** (#29). The split earns its keep for exactly one boundary: if `cicada-core`'s
manifest lists no I/O dependencies, purity is *mechanically enforced* — nobody can casually reach for
`std::fs`. That argument does not extend to splitting git, forge, and updaters from each other: they are
all I/O adapters, none will be consumed independently, and separate crates would buy only `pub` noise and
cross-manifest churn. Split further when a second consumer actually appears.

Everything interesting is a function from inputs to a `Plan` — which commits count, what the next version
is, what the changelog says — testable with no repo, no network, and no filesystem.

Three consequences of the purity boundary that are easy to get wrong:

- **Changelog links are data, not a dependency** (A2). Compare/commit/issue URL styles differ per forge,
  but core must not depend on the forge layer. The forge adapter supplies a `LinkStyle` **value** (URL
  templates) as an input. Never a trait object, or purity leaks back in.
- **Core produces edits; it does not apply them** (#30). Core returns `Vec<FileEdit>` — path plus final
  content — and the I/O layer's whole job is writing them. This is what makes `--dry-run` trustworthy: it
  is not a parallel branch of logic that describes what *would* happen, it is the identical computation
  with the write step skipped. A dry-run implemented any other way drifts from reality.
- **Updater transforms are pure** (A5). `fn update(old: &str, v: &Version) -> Result<String>` lives in
  core and is snapshot-tested; reading and writing the file is three lines outside it. The
  comment-preserving YAML editing is the fiddliest code in the project, so it must also be the easiest
  to test.

### Data flow

```
git log <anchor>..HEAD
        ↓  parse
Vec<ParsedCommit>          (type, scopes, breaking, subject, body, footers, sha)
        ↓  route by scope
BTreeMap<PackageId, Vec<ParsedCommit>>     ordered by config order, never HashMap (#31)
        ↓  bump per package + aggregate umbrella
Plan { releases: Vec<PackageRelease { pkg, from, to, bump, commits }> }
        ↓  render                                    ← still pure
Vec<FileEdit { path, content }>            version files · changelogs · versions file · templates
        ↓  write                                     ← the only I/O; skipped by --dry-run
branch → PR/MR                            (Phase A: release-pr)
merge → tag reconciliation → tags+release (Phase B: release)
```

---

## The two phases

**Phase B runs before Phase A** (#35). Both are triggered by the same event — a push to a branch matching `release_branches` —
so the order is CI's to get right, and getting it wrong is silent. If A runs first, it computes its range
from the *old* tag, re-includes everything the just-merged release contained, and proposes releasing it
again with duplicate changelog entries, every cycle. `cicada run` does both in the correct order so a CI
file cannot express the broken one.

### Phase A — `cicada release-pr`

Runs on every push to a branch matching `release_branches`.

0. Resolve HEAD's branch; if it doesn't match `release_branches`, exit 0 quietly.
1. Load config + versions file — **both read from this branch**, so per-branch config diverges for free.
2. For each package: resolve the anchor (tag, or error) and collect `anchor..HEAD`.
3. Parse commits, route by scope, compute bumps → `Plan`.
4. If the plan is empty: **close any open release PR** for this branch (#34) and exit 0. Leaving it open
   would advertise a release that is no longer justified — e.g. after the only `feat:` was reverted.
5. Render the plan to `Vec<FileEdit>` — version files, changelogs, versions file, templates.
6. For each computed version, check its tag does not already exist unreachably (#80). If it does, two release
   lines are converging on one number — **stop**, exit `4`, before any PR is opened.
7. Inspect the release PR branch tip (#32). If it exists and its committer is not cicada, **stop**: draft the
   PR, apply `cicada:stale`, comment why, exit non-zero. Do not clobber.
8. Reset `cicada/{branch}` to HEAD, write the edits, commit, force-push, upsert the PR/MR **with
   base = the branch cicada ran on**.

### The release PR branch is replaced, never updated

`release_pr_branch` is the PR's **head**. The base is the branch cicada ran on — one of `release_branches`.
Nothing is ever merged *into* the head; it only ever merges *out*, when a human merges the PR.

```
t0   main        A──B
     release-pr       ╲──R1        R1 computed from B, changelog covers …B

t1   two commits land on main
     main        A──B──C──D
     release-pr       ╲──R1        R1 is now stale — knows nothing about C, D

t2   Phase A runs
     main        A──B──C──D
     release-pr             ╲──R2  R2 computed from D, changelog covers …D
     force-push; R1 is discarded
```

R1 is not rebased or merged into R2 — it is thrown away, and R2 is computed from scratch against main's
current tip. There is never a conflict to resolve, because nothing is being reconciled: the branch holds
exactly one commit that is a pure function of the current repo state. The PR is the same PR (same head
branch), showing an entirely new diff.

**The head branch must live outside `release_branches`.** The prefix is a namespace, not decoration. With
`release_branches = ["main", "release/*"]`, a head template of `release/{branch}` renders `release/main` —
which matches `release/*`, so cicada's own force-push re-triggers cicada on the branch it just wrote, and
under a `maintenance_branches = ["release/*"]` config that branch is patch-only as well. The default
`cicada/{branch}` renders `cicada/main` and `cicada/release/1.4`: one reserved segment, outside any
plausible `release_branches` glob, and the branch's own slashes are preserved rather than flattened so two
lines can never collide on one head. Overriding it is fine; overriding it into the release namespace is not,
which is why `check` rejects that (§Loading and validation).

This is also why the upsert must key on the **head branch name**, never the title: it is idempotent by
construction, so two concurrent Phase A jobs cannot open two PRs. Concurrent runs still race on the
force-push, which is self-correcting — but the shipped CI templates set concurrency guards anyway
(`concurrency: cicada-release` on Actions, `resource_group` on GitLab) to avoid the wasted work.

### Staleness (#32, #33)

A human commit on the release PR branch is the one thing on it that is *not* derivable from the base branch,
so "throw it away and recompute" would destroy information that exists nowhere else. Cicada refuses — and
refusing safely requires making the resulting stale PR unmergeable, because a stale release PR that gets
merged ships the intervening commits with **no changelog entry and no influence on the version number**.
The code ships; the semver contract silently breaks. A `feat` released under a patch bump is worse than a
blocked pipeline, because nobody sees it.

Three layers, each covering the others' gap:

| Layer | Mechanism | Covers |
|---|---|---|
| 1. Branch protection | GitHub *"require branches up to date"* / GitLab fast-forward merge. **Repo setting — documented, not enforced by cicada** | The race: main moves and someone merges the PR in the seconds before Phase A regenerates |
| 2. Auto-draft | Cicada converts the PR to **draft** + `cicada:stale` + comment whenever it cannot refresh it. Draft PRs are unmergeable on both forges — enforced by the forge, needing no repo config | The sustained case: cicada is blocked from regenerating (human commits, failed computation) |
| 3. Phase B recomputation | Before creating a tag, Phase B replans from the previous tag; a recorded version **lower** than the recomputed one means the PR was merged stale. Stateless — no SHA is stored | Someone deliberately un-drafted and merged anyway |

Layer 3 hard-errors rather than warning, with eyes open: main is then merged-but-untagged and needs manual
repair. That is the correct trade — a visible unreleased state beats an invisible mis-versioned release.

Labels and draft state are **outputs for humans and the forge**, never inputs cicada reads back to decide
anything. Decision #9 is intact.

### Phase B — `cicada release`

Also runs on every push to a matching branch. **Stateless reconciliation:**

```
for pkg in config.packages (+ umbrella, if declared):
    if pkg has no entry in the versions file:     # never released — nothing to reconcile
        skip

    tag = tag_for(pkg, versions_file[pkg])

    match resolve(tag):                           # artifact 1: the tag
        exists, NOT reachable from HEAD -> ERROR: version collision
        exists, reachable               -> already tagged
        missing                         -> staleness_guard(pkg)      # #78
                                           create and push tag

    if no release object exists for tag:          # artifact 2: the release
        create release (notes + embedded manifest)
```

**Tag and release object are reconciled independently** (#56). Skipping the whole package when the tag exists
would break the self-healing property below: if tagging succeeded and the release API call then failed, the
release object would never be created on any later run. Two artifacts, two checks, both idempotent.

Before creating a tag — and **only** then (#78) — Phase B re-derives the answer and checks the versions file
agrees (layer 3 above):

```
staleness_guard(pkg):
    prev     = highest tag matching pkg.tag_format that is reachable from
               HEAD and below the recorded version   (none -> first release)
    expected = plan(prev..HEAD)[pkg].version
    if recorded_version < expected:
        ERROR: release PR was merged while stale — the recorded version is
               lower than the commits in the range justify
```

Phase B already has the planner, so this costs a second `git log` and no stored state at all. It catches
precisely the harm the staleness model exists to prevent: a `feat` shipped under a patch bump, invisibly.

**The comparison is deliberately one-sided.** Only *lower than expected* is an error. A recorded version
*higher* than the recomputation is a deliberate over-bump — a `Release-As:` footer, or a human hand-editing
the versions file to cut a release by hand — and both are supported workflows. The same asymmetry enforces
#76 for free: editing the versions file downwards puts the recorded version below the recomputed one and
trips this guard without needing a separate check.

**The gating matters as much as the check.** Run unconditionally, it would fire on every ordinary push after
a release: once `v1.5.0` is tagged, the next `fix:` legitimately makes the recomputation say `1.5.1` while
the versions file still says `1.5.0`. Only at the moment a tag is about to be created is the versions file
claiming to describe HEAD.

**Reachability is load-bearing, not a nicety.** Cicada's own tags are always ancestors of HEAD on a later
run, so idempotency is preserved. But a tag that exists *elsewhere* in the repo — on another branch, at a
version this branch is only now reaching — means two branches are about to claim the same version. Skipping
it would silently swallow a release: commits made, changelog written, nothing tagged or published. It is an
error, and a loud one.

**Phase A runs the same check** (#80). Discovering a collision here, in Phase B, means the release PR was
already reviewed and merged: `Cargo.toml` carries the contested version, the changelog entry is written, and
the branch is merged-but-untagged awaiting manual repair. Phase A computing the same version and finding the
tag already present refuses to open the PR at all. Same primitive, no extra state, and the error lands before
a human has read anything. Phase B keeps the check as the backstop for a version reached by a hand-edited
versions file.

Consequences, all of them good:

- Immune to squash-merge, merge commits, rebase-merge, and rewritten commit messages.
- Idempotent — re-running a pipeline double-tags nothing.
- Self-healing — if tagging succeeded but the release API call failed, the next push creates just the missing release object (#56).
- A human hand-editing the versions file and pushing is a valid manual release.
- Zero forge state. No labels, no PR queries, no API call needed to decide anything.

---

## Maintenance branches

Supporting `1.4.x` from a `release/1.4` branch while `main` is on `2.x` costs almost nothing here, because
**every piece of cicada's state lives inside the branch**: `.cicada-versions.toml`, `cicada.toml`,
`CHANGELOG.md`. Branches are isolated by construction.

```
main          .cicada-versions.toml → api = 2.0.3
release/1.4   .cicada-versions.toml → api = 1.4.7
```

Cut `release/1.4` from tag `api-v1.4.7`, cherry-pick a fix, run cicada on that branch: the anchor resolves
to `api-v1.4.7` (reachable from the branch), the range is that tag to HEAD, one `fix:` → `1.4.8`, tagged on
the branch. `main` is untouched and never knew. No setup, no separate config, no per-branch state store.

This is where the design diverges usefully from release-please, which needs `--target-branch` and per-branch
configuration precisely *because* its state is centralized in the forge — one manifest, one set of labels.

What it required (#26–28, and the reachability rule in #9):

| Concern | Resolution |
|---|---|
| Two branches pushing the same release PR branch | Templated `release_pr_branch`: `cicada/{branch}` |
| PR opened against the wrong base | Base = the branch cicada ran on |
| cicada acting on branches it shouldn't | `release_branches` allowlist; non-match exits 0 |
| Cherry-picked `feat:` producing `1.5.0` on a maintenance line | the branch matches `maintenance_branches` — cicada refuses rather than quietly releasing a feature as a patch |
| Two branches reaching the same version number | Reachability check → version-collision error (#9) |

Per-branch config divergence is free and useful: `release/1.4`'s own `cicada.toml` carries
a narrower `release_branches`, and `main`'s does not, because each is read from the branch being released.

Branch *identity* is the one exception (#82). `maintenance_branches` and `prerelease_branches` are patterns
matched against the current branch name, so they read identically on every branch — they describe the repo's
conventions, not one branch's state. Config still comes from the branch cicada is running on; when `main`'s
config changes, cherry-pick it to the live maintenance branches like any other change.

All three branch-identity settings are `globset` globs over the **full branch name**, anchored at both ends:
`*` matches within a path segment, `**` crosses `/`, and `[0-9]` / `{a,b}` are available. `release/*` therefore
matches `release/1.4` but not `release/1.4/hotfix`, while `release/**` matches both. This is why the defaults
in #83 come in pairs: `maintenance/**` reaches `maintenance/foo/bar/1.4` while `maintenance-*` reaches
`maintenance-1.4` and `maintenance-foo-bar`, and no single `*` pattern covers both once `*` stops at `/`.

Slashes in branch names are ordinary — `refs/heads` is a path — and cicada relies on that in
`release_pr_branch` (#27). The one thing git forbids is a branch named `prerelease` alongside a branch named
`prerelease/rc`, since a ref cannot be both a file and a directory.

### The maintenance guard is a guard, not a clamp (#28)

```toml
[settings]
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]      # declared once, reads the same on every branch
```

A maintenance line promises its consumers one thing: **only patches**. `maintenance_branches` makes that
mechanical for every branch matching the pattern. When a cherry-pick brings a `feat:` — usually because the
wrong commit was picked, or because the change belongs on a newer line — cicada **hard-errors, exit `4`**,
naming the offending commits.

It deliberately does *not* clamp the version down. #33 justifies the entire staleness model with the
observation that *a `feat` released under a patch bump is worse than a blocked pipeline, because nobody sees
it* — and a clamp is a config key whose sole function is to do exactly that. Worse for `feat!:`: a consumer
pinned to `~1.4.0` would take a breaking change on a patch upgrade with no signal in the version number at
all. Every other ambiguity in cicada is a hard error — unknown scope (#11), conflicting `Release-As` (#39),
stale merge (#33) — and this was the one place the tool silently answered wrong.

The guard is silent in the normal case: cherry-pick only fixes and it never fires. The escape hatch needs no
special-casing, because `Release-As:` bypasses bump computation entirely:

```
--release-as api=1.4.8        # yes, I know, do it anyway
```

And because the offending commit is visible in a pull request, `cicada check` reports it **before merge**
rather than at release time. There is no `max_bump = "patch"` scalar, because patch is the only value that
was ever meaningful — a string field accepting one string invites `"minor"` and a parse error for no reason.

**Why a pattern rather than a flag on the branch** (#82). A `maintenance = true` written onto each branch has
to be remembered at every cut, and forgetting it means the guard silently does not apply. It also travels
through merges: the release-branch arrangement below *requires* merging release branches back into `main`,
which would carry that flag onto main and quietly make the trunk patch-only. A pattern is identical
everywhere, so a merge-back is a no-op on the config file and cutting a line needs no config commit at all:

```
git checkout -b release/1.4 v1.4.7 && git push -u origin release/1.4
```

Cicada picks the branch up on the next push, recognises it as a maintenance line, and enforces the guard.

**Known divergence, accepted:** `main`'s `CHANGELOG.md` never learns about `1.4.8`, since that entry was
written on the maintenance branch. Merging changelogs across lines is out of scope.

---

## Release arrangements

Both of the following work today and need no new machinery. They differ only in what is listed in
`release_branches` — which is why there is no `release_strategy` setting (#81): a preset over existing keys
would unlock no behaviour, and a wrongly declared one would be worse than none. What they *do* need is
naming, because their failure modes are completely different.

### Trunk releases

`main` releases the current line. A maintenance branch serves a line main has already passed.

```
main          1.5.0 ── 1.5.1 ── 2.0.0 ──►
               ╲
release/1.4     1.4.8 ── 1.4.9            cut after main reached 1.5.0
```

```toml
release_branches = ["main", "release/*"]
```

Lines can never collide, because `release/1.4` is only cut once main is past 1.4. Cherry-picks flow from main
to the older branch. `main` does not match `maintenance_branches`, so the guard never applies to it.

**The failure mode** is cutting the maintenance branch too early — while main is still releasing the same
line. Both branches then compute the same next patch, and the second one to run hits the collision check
(#9, #80). That is a misconfiguration, and the error is the correct answer: if main is still releasing 1.4.x,
the `release/1.4` branch has nothing to do.

### Release-branch releases

`main` is a trunk that never releases. Every release comes from a release branch that owns its line. Fixes
land on main and are cherry-picked into whichever branches need them.

```
main         X ─────────── Y ─────────►    never released
              ╲             ╲
release/1.4    1.4.0 ── 1.4.1 ╲
release/1.5                    1.5.0 ──►
```

```toml
release_branches     = ["release/*"]    # main excluded — pushes to it exit 0 quietly (#26)
maintenance_branches = ["release/*"]
```

A line's **first** release is necessarily a minor (`1.4.1` → `1.5.0`), which the guard refuses. Cut it with
`--release-as api=1.5.0`, which bypasses bump computation (#39) — cutting a release line is inherently a
decision about a version number, so stating one is the honest form. Every release after that is a patch
computed normally.

**The constraint that will bite silently: release branches must be merged back into `main`.** Cicada never
writes to main in this arrangement, so main's `.cicada-versions.toml` holds whatever was there when the last
branch was cut — and the tags live on branches that diverged. Cut `release/1.5` from main without merging
`release/1.4` back, and its anchor tag `v1.4.1` is not reachable from the new branch, so anchor resolution
hard-errors (#3). If the versions file is absent entirely, cicada sees a first release and computes `0.1.0`.

Merging each release branch back — what git-flow already prescribes — makes `v1.4.1` an ancestor of main and
of every branch cut from it, keeps main's versions file current, and lets `release/1.5` anchor correctly and
compute `1.5.0` from the accumulated features. One consequence to expect: merging `release/1.4` back *after*
`release/1.5` exists will conflict on `CHANGELOG.md`, which is the divergence recorded in future-work §10.

---

## Configuration

### `cicada.toml` — human-owned

```toml
[settings]
# which branches cicada acts on, and what kind of line each is (#26, #82)
release_branches     = ["main", "release/*"]     # the gate: cicada acts only if HEAD's branch matches
maintenance_branches = ["release/*"]             # overrides the default; patch-only lines (#28, #83)

default_scope     = "platform"                   # unscoped commits land here (#38)
committer         = "cicada[bot] <cicada@users.noreply.github.com>"   # stable — #32's guard depends on it
commit_source     = "commits"                    # "commits" (--no-merges) | "merges" (--first-parent) | "both" (#40)

# the release PR cicada opens — its head branch and its title (#27, #41)
release_pr_branch = "cicada/{branch}"    # the PR's HEAD; base is whichever branch cicada ran on
release_pr_title  = "chore(release): {summary}"  # conventional and non-releasable

# which branches cut prereleases, and what those prereleases are called (#59, #84)
prerelease_branches   = ["prerelease-v*-rc"]     # a glob list, like maintenance_branches
prerelease_identifier = "rc"                     # one line, one identifier — "beta" is the other common value

[forge]
kind    = "github"                 # "github" | "gitlab"
api_url = "https://api.github.com" # override for GHE / self-hosted GitLab
repo    = "org/repo"               # optional; derived from origin when absent

[umbrella]
name       = "platform"
tag_format = "platform-v{version}"
changelog  = "CHANGELOG.md"

[changelog]
# `bump` decides whether a type justifies cutting a release at all (#36);
# `title`/`hidden` only decide how it renders.
sections = [
  { type = "feat",  title = "Features",      bump = "minor" },
  { type = "fix",   title = "Bug Fixes",     bump = "patch" },
  { type = "perf",  title = "Performance",   bump = "patch" },
  { type = "docs",  title = "Documentation", bump = "none"  },
  { type = "chore", title = "Chores",        bump = "none", hidden = true },
]

[packages.api]
path       = "services/api"
tag_format = "api-v{version}"
changelog  = "services/api/CHANGELOG.md"
# initial_version = "1.0.0"   # override the first-release rule (#37)
# pre_major       = "minor"   # "minor" (default) | "major" — breaking-change behaviour while on 0.x

[[packages.api.updaters]]
kind = "cargo"
path = "services/api/Cargo.toml"

[[packages.api.updaters]]
kind = "generic"
path = "deploy/api/values.yaml"
edit = { yaml = "image.tag" }

[[packages.api.updaters]]
kind     = "template"
template = "templates/deployment.yaml.j2"
output   = "deploy/api/deployment.yaml"

# The chart is its own package with its own version line (decision #14).
[packages.api-chart]
path       = "deploy/charts/api"
tag_format = "api-chart-v{version}"
changelog  = "deploy/charts/api/CHANGELOG.md"

[[packages.api-chart.updaters]]
kind            = "helm"
path            = "deploy/charts/api/Chart.yaml"
app_version     = { track = "api" }   # appVersion mirrors the `api` package
on_tracked_bump = "patch"             # chart's own bump when `api` releases
```

A package with an empty `updaters` list is tag-only: it gets a version, a tag, and a changelog,
but no file is written.

### `.cicada-versions.toml` — tool-owned

```toml
[umbrella]                  # present only if the repo declares one (#73)
version = "1.7.0"

[packages.api]
version    = "1.4.2"        # last STABLE release — the anchor (#60)
prerelease = "1.5.0-rc.2"   # present only while a prerelease line is open; deleted when 1.5.0 cuts

# `chart` absent → never released → first release takes all reachable history
```

**A version and nothing else** (#1, #2). Written by cicada, committed by the release PR. Its path is fixed —
beside `cicada.toml` at the repo root — because tooling that reads it should not have to hunt.

It is also the **sole source of truth** for versions (#75). `Cargo.toml` and `Chart.yaml` are outputs; cicada
never reads a version back out of them to seed a computation. The visible consequence is that a first release
computed as `0.0.1` rewrites a `cargo new` manifest from `0.1.0` *downwards* — correct behaviour, because the
manifest was never an input. Reading it back would also break the `FileEdit` boundary (#30), and with it the
guarantee that `--dry-run` is the same computation with the write skipped.

### Defaults (#83)

Every key, what it does when omitted, and whether omitting it is allowed. A release tool whose behaviour on a
missing key has to be discovered by experiment is a release tool nobody trusts.

| Key | Omitted → | Why |
|---|---|---|
| `release_branches` | **error** | The gate decides where cicada may write. Empty would exit 0 on every branch and release nothing, forever, with a green pipeline; `["main"]` would be wrong for `master`, `trunk`, and every release-branch repo. Both failures are silent, so it is declared — the same call as `forge.kind` (#66) and the package list (#8) |
| `maintenance_branches` | `["maintenance/**", "maintenance-*"]` | A branch called `maintenance/1.4` wants the patch-only guard. Two spellings because both conventions are in the wild, and `*` does not cross `/` |
| `prerelease_branches` | `["prerelease/**", "prerelease-*"]` | Same convention as `maintenance_branches`, same two spellings |
| `prerelease_identifier` | `"rc"` | The commonest choice. A `beta` line sets it — and `check` says so when the branch name and this value disagree, rather than leaving it to be noticed in a version string (#84) |
| `default_scope` | **error** | Unscoped commits must land somewhere, and in a monorepo the answer is not derivable (#38) |
| `committer` | `cicada[bot] <cicada@users.noreply.github.com>` | Has a default *and* must not drift between versions — #32's guard reads it back |
| `commit_source` | `"commits"` | `--no-merges`; identical to the alternatives on squash/rebase repos (#40) |
| `release_pr_branch` | `"cicada/{branch}"` | Outside any plausible `release_branches` glob (#27) |
| `release_pr_title` | `"chore(release): {summary}"` | Conventional and non-releasable (#41) |
| `forge.kind` | **error** | Never inferred from the remote at release time; the error names the value it would have detected (#66) |
| `forge.api_url` | the public host for `kind` | |
| `forge.repo` | derived from `origin` | The one forge value that *is* derived, because it cannot be ambiguous |
| `[umbrella]` | no umbrella | cicada never assumes an aggregate version (#73) |
| `[changelog] sections` | the five-row table above | `feat`/`fix`/`perf`/`docs`/`chore`. Printable and overridable like any other default (#48) |
| `packages.<id>.path` | **error** | |
| `packages.<id>.tag_format` | **error** | Two packages silently sharing a tag shape is a version collision waiting to happen (#9) |
| `packages.<id>.changelog` | `<path>/CHANGELOG.md` | |
| `packages.<id>.updaters` | `[]` | A tag-only package: version, tag and changelog, no file written |
| `pre_major` | `"minor"` | 0.x stays inside 0.x; reaching 1.0.0 is deliberate (#37) |
| `initial_version` | the first-release table | breaking → `1.0.0`, feat → `0.1.0`, fix → `0.0.1` (#37) |
| `on_tracked_bump` | `"patch"` | The derived bump a chart takes when its tracked app releases (#22) |

**Required means required, and there are five.** `release_branches`, `default_scope`, `forge.kind`, and each
package's `path` and `tag_format`. Every one of them is a place where a wrong guess is invisible rather than
noisy — which is the same test #66 and #8 already apply.

**Why the gate cannot be defaulted but the classifiers can.** They are asked different questions.
`release_branches` asks *may cicada write here at all*, and it is the only setting standing between a push and
a release. The other two only ever answer *what kind of line is this* about a branch that is *already* in
`release_branches` — so a defaulted pattern can never make cicada act somewhere it was not explicitly
admitted. The blast radius of a wrong default is bounded by a required key, which is what makes defaulting it
honest rather than convenient.

The two failure modes are also not equivalent:

- A wrong **maintenance** classification is **loud** — exit `4`, naming the commits that computed above patch
  (#28). Nothing ships.
- A wrong **prerelease** classification is quieter: a branch named `prerelease-beta` matching the default would
  cut `1.5.0-rc.1`, and the only signal is the identifier in a version string. It is visible in the release PR
  before any tag exists, but nobody reads a version string as a warning.

So `check` closes that gap directly: when a branch matches a **defaulted** prerelease pattern and its trailing
`-` or `/` segment is one of `alpha`, `beta`, `rc`, `preview`, `next` and differs from `prerelease_identifier`,
that is a finding (exit `3`) naming both and the one line to change. The word list is a lint heuristic, not
semantics — cicada still never parses meaning out of a branch name (§Prereleases). It only notices that the
developer wrote one thing and is about to get another.

### Loading and validation

`cicada.toml` is read from the repo root (`--config` overrides). Parsing **denies unknown fields** (#67): a
mistyped `tag_fromat` is an error naming the key, not a setting that silently does nothing — the failure mode
config typos otherwise have is a release that looks fine and is wrong.

`cicada check` validates the config itself, independent of any commit range:

- every package `path` exists, and no two packages share one
- every `tag_format` contains `{version}`, and no two packages produce the same tag for the same version
- `default_scope` names a real package or the umbrella
- every updater `path` exists; no two updaters declare the same `output` (#44)
- `app_version.track` names a real package
- no two packages resolve to the same Cargo version field (#24 — the workspace-inheritance trap)
- `release_pr_branch` contains `{branch}`, and renders to a ref that matches no `release_branches` pattern (#27)
- the five required keys are present (#83); a missing `release_branches` names the current branch as the likely value
- a branch matching a **defaulted** prerelease pattern does not contradict `prerelease_identifier` (#83, #84)

That last one is the Cargo workspace-inheritance trap (#24): two crates with `version.workspace = true`
resolve to a single version field, so they cannot be bumped independently. `check` is where an unreleasable
arrangement becomes a legible error rather than a silent one.

---

## Core algorithms

### Anchor resolution

```
if no entry in the versions file:
    anchor = None                              (first release: all reachable history)
else:
    tag = render(tag_format, current_version)
    if tag does not resolve                 -> ERROR: tag not found
    if tag is not reachable from HEAD       -> ERROR: tag exists elsewhere in the repo
    anchor = tag's commit
```

**The tag is the only anchor** (#2, #3). An earlier design also recorded the commit SHA that Phase A computed
against, and fell back to it when the tag was missing. That is gone: the versions file holds a version and
nothing else. The fallback was a mechanism for silently continuing in a degraded state — a resolvable but
*wrong* range, producing a changelog that looks fine and is incomplete — which is the failure mode this tool
exists to avoid. A missing tag is now an error naming the tag and the repair.

The SHA was doing two other jobs, both replaced by something stateless:

- **Validating the tag.** The old descendant check caught a tag pointing somewhere unrelated. *Reachable from
  HEAD* catches the same class with nothing stored, and it is the primitive Phase B already uses for collision
  detection (#9). A tag that exists but is not an ancestor of HEAD means the range would silently span the
  merge base.
- **Recording what the changelog covers**, for stale-merge detection. Replaced by recomputation in Phase B
  (#33, #78) — see §The two phases.

**Anchors are per-package, so ranges are too.** Packages release at different times:

```
c1   feat(api): add caching          ← tag api-v1.5.0
c2   feat(api-chart): add PDB        ← tag api-chart-v0.9.0
c3   fix(api): handle nil
c4   fix(api-chart): fix probe path

api's range        c1..HEAD
api-chart's range  c2..HEAD
```

Implementation: one `git log` per package. N is small and each call is milliseconds, so the union-range-plus-
ancestry-filtering alternative buys nothing but complexity.

Shallow clones are detected up front (`git rev-parse --is-shallow-repository`) and produce an actionable
error naming `fetch-depth: 0` / `GIT_DEPTH: 0`, rather than a silently truncated changelog.

### Commit parsing

Hand-written parser rather than an off-the-shelf crate, because of the multi-scope extension:

```
feat(api,worker)!: add retry backoff

BREAKING CHANGE: retries are now on by default
Release-As: 2.0.0
```

- `type` — matched against configured changelog sections; unknown types are ignored (not errors).
- `scopes` — comma-separated list; each must resolve to a configured package or it is a **hard error**.
- Breaking — `!` after the scope, or a `BREAKING CHANGE:` footer.
- Empty scope → attributed to `default_scope` (#38).

**Which commits are read** (#40). In a squash-merge repo the commits on main *are* the PR titles — the forge
puts the title into the squashed commit — so cicada never needs to know your merge strategy. It only matters
for merge-commit repos, where two conventions exist: the real message is on the individual commits, or it is
on the merge commit (GitLab's default merge message carries the MR title). Hence:

| `commit_source` | git traversal | Reads |
|---|---|---|
| `"commits"` (default) | `--no-merges` | every real commit; merge commits skipped |
| `"merges"` | `--first-parent` | one entry per PR; the commits it merged in are never visited |
| `"both"` | — | everything; can double-count |

On squash and rebase repos main is linear, so `--no-merges` and `--first-parent` return the identical set —
the setting is a no-op there, and all three merge strategies work out of the box.

**`Release-As:` in a monorepo** (#39). The footer forces an exact version instead of the computed one:

```
feat(api): rewrite the storage layer

Release-As: 2.0.0        →  api becomes exactly 2.0.0, not the computed 1.6.0
```

- It applies to **the commit's own scopes**. `feat(api,worker): … Release-As: 2.0.0` sets *both*, which is
  rarely intended when their versions differ — scope it narrowly.
- Unscoped commits apply it to `default_scope`.
- Two commits in one range forcing **different** versions for the same package is a **hard error**; silent
  last-wins is a trap.
- CLI equivalent is repeatable and package-qualified: `--release-as api=2.0.0`.

### Bump computation

**Not every commit justifies a release** (#36). Two independent questions, previously conflated:

- *Does this type appear in the changelog?* → the section's `title` / `hidden`
- *Does this type justify cutting a release at all?* → the section's `bump`

`feat` → minor, `fix`/`perf` → patch, `chore`/`docs`/everything else → `none`. A range containing only
`chore(deps): bump serde` produces **no release** — otherwise every dependency bump would ship a version.
Breaking changes override the section's `bump` and always take major. This check lives in cicada itself, not
only in a CI trigger condition, so `cicada plan` gives the same answer wherever it runs.

**First release** (#37). An unreleased package is treated as `0.0.0` with strict semver applied:

| First release contains | Version |
|---|---|
| a breaking change (`feat(api)!:`) | `1.0.0` |
| a feature (`feat(api):`) | `0.1.0` |
| only fixes | `0.0.1` |

`initial_version` overrides this outright.

**Afterwards, `pre_major` governs 0.x.** With the default `"minor"`, a breaking change at `0.3.0` gives
`0.4.0` — never escaping to 1.0.0 by accident. Reaching 1.0.0 is a deliberate act (`--release-as api=1.0.0`).
Note the first release is a conscious exception: `feat!:` yields `1.0.0` there but `0.4.0` at `0.3.0`, because
the first release is where you *choose* your entry point and `!` is how you say so without config.

**Umbrella (semver-aggregate).** It aggregates the **bump kind**, not the version number: `api` going
`1.4.2 → 2.0.0` makes the umbrella take *a major bump from its own line* (`1.7.0 → 2.0.0`), not literally
copy 2.0.0. Any child major → umbrella major; any child minor → at least minor.

- Unscoped commits are judged on their own merits via `default_scope`, so the umbrella can release alone —
  a root-level `fix:` gives it a patch bump with no child involved.
- A chart's **derived** bump (#22) does *not* feed the aggregate; it is a consequence of a child bump already
  counted, and counting it twice would inflate the umbrella.
- `maintenance` (#28) guards the umbrella exactly as it guards children — if any package computes above patch, the run refuses.

**Single-service repos need no special case** (#38). `default_scope` names whichever package receives unscoped
commits: the umbrella in a monorepo, the one package in a single-service repo. Same code path, no branching.

The umbrella's changelog groups entries by service; each service's own changelog contains only its own.

### Prereleases

**The collision.** The anchor model says "current version → derive tag → that is where the last release was".
Prereleases break it:

```
api-v1.4.2 released
   feat, fix land
cut 1.5.0-rc.1
   fix lands
cut 1.5.0-rc.2
   cut stable 1.5.0        ← what does its changelog cover?
```

Anchoring on the last release *of any kind* means 1.5.0 covers only `rc.2..HEAD` — a single fix. The feature
that motivated the release would be **absent from the 1.5.0 notes**. So a stable release must anchor on the
last *stable* release.

**The fix is one extra field, and `version` keeps its exact current meaning** (#60):

```toml
[packages.api]
version    = "1.4.2"        # last STABLE release — anchor semantics unchanged
prerelease = "1.5.0-rc.2"   # only while a prerelease line is open
```

Anchor resolution and range computation are untouched. Every release — rc.1, rc.2, and
the final 1.5.0 — computes from `1.4.2..HEAD`, so notes are **cumulative by construction** (#61) and the
stable changelog is automatically complete. `prerelease` tracks only which rc number is current, and is
deleted when the stable release cuts.

**Counter rule** (#63). Compute the target stable version from commits as usual (`1.4.2` + feat = `1.5.0`),
then attach `-rc.N`. If the computed base still matches the open prerelease's base, increment N. If the base
changes — a breaking change arrives and `1.5.0` becomes `2.0.0` — reset to `2.0.0-rc.1`. The awkward case
handles itself.

**Prereleases never touch `CHANGELOG.md`** (#62). If every release wrote a cumulative entry, the file would
accumulate rc.1, then rc.2 repeating it, then 1.5.0 repeating it again. Instead the changelog contains only
stable releases — clean and linear — while each rc's *forge release notes* are cumulative since the last
stable, which is exactly what a tester wants. Version files (`Cargo.toml`, `Chart.yaml`) **are** written,
since the rc artifact has to build. The release PR body carries the notes either way, so reviewers see the
content.

**Branch-level, not per-package** (#59). Per-package prereleases make the umbrella incoherent — semver
aggregation has no meaning when some children are on rc and others are not. Branch-level costs nothing new:

```toml
# the same on every branch (#82)
prerelease_branches   = ["prerelease-v*-rc"]
prerelease_identifier = "rc"
```

`prerelease-v1.5-rc` matches, so it produces `1.5.0-rc.N`; `main` does not match, so it produces stable. Same
mechanism as `maintenance_branches` — the same shape, even: a glob list and nothing else. A `--prerelease rc`
flag covers ad-hoc rcs without a branch, and overrides the identifier when both apply.

### One line, one identifier (#84)

A prerelease line has exactly one function: ship the next stable early, under a name that sorts below it. That
is one job, so a repo gets one line and one identifier. `beta` feeding `rc` is **not** an arrangement cicada
expresses.

This is worth stating because the alternative is easy to build and buys nothing. cicada draws no distinction
between identifiers whatsoever: `beta` and `rc` anchor the same way, compute the same base, run the same
counter, skip the same changelog, and produce the same `prerelease: true` release object. The *only* thing
that differs is the string in the version and the ordering that string implies. A per-branch mapping would
therefore let a repo express a maturity ladder that cicada does not understand and cannot enforce — the
ordering between stages would be an alphabetical accident, not a guarantee.

Collapsing it to a scalar removes real machinery, not just a line of config:

- **No precedence rule.** With a map, two patterns matching one branch needed a tie-break, and "first in config
  order wins" is exactly the silent last-wins this design refuses elsewhere (#39, #44). A list has nothing to
  tie-break.
- **No ordered-map requirement** for this setting, and no question about whether a TOML table preserves
  document order.
- **The counter rule stays complete.** #63 decides increment-versus-reset from the computed base alone. With
  one identifier the base is the only thing that can change, so there is no second axis for it to be silent
  about.

**Changing the identifier mid-line is a config edit, and #85 guards it.** Editing `prerelease_identifier` from
`rc` to `beta` while `1.5.0-rc.2` is open would compute `1.5.0-beta.3`, which sorts *below* what already
shipped. So a computed prerelease must be strictly greater than the one recorded in the versions file — #76's
monotonicity rule, applied to computed versions rather than only forced ones. The reverse edit, `beta` to `rc`,
is strictly greater and passes: tightening a line's name toward release is legal, loosening it is not.

Several prerelease *branches* may match the list at once — `prerelease-v1.5-rc` now, `prerelease-v2.0-rc` next
quarter — exactly as several maintenance branches may. They share the identifier, and if two of them ever
compute the same version the existing collision check refuses the second (#9, #80), the same way two
maintenance lines converging does.

**The pattern is a glob, not a branch name** (#82). Writing the literal `"prerelease-v1.5-rc"` works and is the
degenerate case, but it has to be edited every time a line is cut, which is the failure mode #82 exists to
remove. `"prerelease-v*-rc"` is written once and picks up `prerelease-v2.0-rc` two quarters later with no
config commit — the same property that lets `release/*` absorb a new maintenance line.

**A version in the branch name is decoration, and cicada never reads it.** `prerelease-v1.5-rc` is matched as
a string and nothing is parsed out of it: the version comes from the anchor plus the commits, exactly as on
any other branch. A `feat!:` on that branch computes `2.0.0-rc.1` while the branch still says `v1.5`, and
cicada neither corrects the name nor warns. The name is a human label on the line; the tag is the truth.

**It has to be a pattern, not a value on the branch** (#82). An earlier design put `prerelease = "rc"` in
the prerelease branch's own `cicada.toml` and claimed that merging it into `main` "just works, because
main's config has no `prerelease`". That claim was wrong: the merge brings that config file with it, so
main's config now *does* carry `prerelease = "rc"` and main starts cutting rcs. The only escape was
hand-resolving the config file during every merge — exactly the step that gets missed. With a pattern, both
branches carry the same config, the merge is a no-op on it, and `main` cuts `1.5.0` because it does not
match `"prerelease-v*-rc"`.

Phase B reconciles the prerelease tag when the field is present and the stable tag when it is not;
reachability and collision checks apply unchanged.

**Forge asymmetry.** GitHub releases carry a `prerelease: true` flag; **GitLab releases have no equivalent**.
The GitLab adapter marks it in the release name instead. Same class of gap as draft PRs (#54).

### Release manifest

**One manifest per run, not per package** (#42). A single run can release three packages independently; what
deployment needs is the set that shipped together — package, version, tag, the tagged commit, timestamp.

It is written to `target/cicada/release-manifest.json` (gitignored, **never committed**) *and* attached to the
umbrella's release object. Both, for two reasons: under `--no-push` there is no release object to attach to,
so the file is the only way to inspect a rehearsal (Goal 6); and the deploy job that runs after the release
job can read a file in the workspace without a token and an API round trip.

This manifest is the artifact that is genuinely useful for deployment — feeding an umbrella Helm chart or a
compose bundle. The umbrella *version* is a human-friendly label on it.

---

## Updaters

A package owns a **list** of updaters. This collapses what were three separate concepts (`type`,
`extra_files`, `templates`) into one uniform mechanism, and extends to Go/Java/Python later by adding
`kind` values with **no schema change**.

| kind | Writes | Notes |
|---|---|---|
| `cargo` | `Cargo.toml` `version` | `toml_edit` preserves formatting and comments; `version.workspace = true` redirects to the workspace root (see #24) |
| `helm` | `Chart.yaml` `version`, `appVersion` | `appVersion` may `track` another package; `on_tracked_bump` sets the derived bump size |
| `generic` | one value in any file | In-place surgical edit; the file stays the user's |
| `template` | a whole file | minijinja render; cicada owns the output file |

### Two addressing modes (#50)

They look similar in config but are different mechanisms with different reach:

**Path addressing** — `toml = "package.version"`, `yaml = "image.tag"`, `json = "$.version"`. Requires a
structured format, addresses a node precisely.

**Marker comments** — `image: myapp:1.4.0 # x-cicada-version`. Works on **any text file**: Dockerfile,
Makefile, `version.go`, `.py`, shell scripts. Line-oriented: the first semver-shaped token on the annotated
line is replaced. This is the escape hatch for the ecosystems deferred out of v1 — a Go or Python service is
served by a marker comment today without waiting for a dedicated `kind`.

### How the editing actually works (#49, #52)

The naive options both fail: `serde_yaml` round-trips destroy comments and formatting, and hand-rolled
line-walking means reimplementing quoting, trailing comments, flow mappings, anchors, multi-document files,
and indentation tracking — a YAML parser, written badly.

**Parse with a span-preserving parser, then splice bytes.** Locate the scalar node at the addressed path,
take its byte range, replace exactly those bytes. **Validated by the M0 spike** — see
`spikes/yaml-splice`, 15 passing cases:

```
before:  tag: 1.4.2   # managed by cicada
                ^^^^^  only this span is replaced
after:   tag: 1.5.0   # managed by cicada
```

Everything outside the span is byte-identical **by construction**, because the document is never re-emitted.
Correct parsing and perfect preservation at once — this is what `toml_edit` already does for TOML.

`saphyr-parser` is the chosen crate (#71): its `Parser` is an `Iterator<Item = Result<(Event, Span), _>>`
and every scalar carries start/end markers plus its `ScalarStyle`, so quoting is recoverable — `appVersion:
"1.4.2"` stays quoted, `version: 0.9.0` stays bare, and an unquoted `appVersion` is detectable for the #51
warning. Verified against a realistic `values.yaml` with anchors, flow mappings, sequences, emoji and
non-ASCII comments: exactly one line changed, the rest byte-identical.

**Two traps the spike surfaced, both load-bearing:**

- **`Marker::index()` returns a *char* index**, though its own doc comment says "in bytes" (the struct field
  comment, "in chars", is the truthful one). Splicing on the raw value corrupts any file with non-ASCII
  before the target — a comment containing `naïve — … ✅` shifted the splice 8 bytes and produced
  `vers2.0.01.0.0`. Conversion is mandatory, and the regression tests for it must survive into the real
  updater.
- **The walker must tolerate what it cannot address** (#72). An alias or complex key *anywhere* in the
  document must not abort the edit — only one at the addressed path is an error. Helm values files use
  anchors freely, and the first draft refused to edit unrelated keys because of a `&baseline` twenty lines
  away.

Every edited file is **re-parsed after writing** (#52). A splice that produces an unparseable file aborts the
run before anything is committed — cheap insurance against exactly the class of bug this technique can have.

### Helm `appVersion` quoting (#51)

```yaml
appVersion: 1.10      # unquoted → YAML parses this as the float 1.1
```

Helm treats `appVersion` as a string, but YAML does not, and byte-splicing faithfully preserves whatever
quoting it finds — including none. The helm updater therefore **always writes `appVersion` quoted**, and
warns when it finds an unquoted value already in place.

**Chart tracking is the only dependency propagation in cicada** (see Non-goals). It is declared, not
inferred: when the tracked app releases, the chart's `appVersion` is rewritten and the chart takes a
derived bump, because a changed chart artifact requires a new chart version. A chart also bumps on its
own merits from `feat(api-chart): …` commits; when both apply, the larger bump wins.

### Known limitation: intra-workspace dependency requirements

```toml
# services/gateway/Cargo.toml
auth = { version = "0.3", path = "../../libs/auth" }
```

When `auth` bumps to `0.4.0`, cicada does **not** rewrite that `version = "0.3"`. For a private workspace
this is harmless — `path` wins locally. It only matters when publishing to a registry, which is out of scope
(see `docs/future-work.md`).

---

## CLI surface

```
cicada check [--base <ref>] [--head <ref>] [--title <s>]
                                validate config; lint commits/PR title
cicada plan [--json]            what would be released, and why. touches nothing
cicada release-pr [--dry-run] [--no-push] [--prerelease <id>]
                                Phase A: compute, apply, push branch, upsert PR/MR
cicada release [--dry-run] [--no-push]
                                Phase B: reconcile tags, create releases
cicada run [--dry-run] [--no-push]
                                Phase B then Phase A, in that order. What CI should call
cicada template print <name>    emit a built-in template (notes, changelog) to stdout
```

Global: `--token`, `--forge`, `--api-url`, `--repo`, `--config`, `--json`, `-v`, `--quiet`.
Colour follows `NO_COLOR` and TTY detection.

### Exit codes (#68)

CI branches on these, so they are a contract rather than an implementation detail:

| Code | Meaning |
|---|---|
| `0` | Success — **including "nothing to release"**, which is a normal outcome, not a failure |
| `1` | Error — invalid config, git failure, forge API failure |
| `2` | Usage error (clap) |
| `3` | `check` found problems — lint failures, unknown scopes |
| `4` | Refused — stale release PR branch (#32), version collision (#9, #80), stale merge (#33), maintenance guard (#28) |

`3` and `4` are separated from `1` deliberately: "your commits are malformed" and "cicada declined to act"
are conditions a pipeline may want to handle differently from "cicada broke".

### `--json` (#69)

Machine interface, so it carries its own version:

```json
{ "schema": 1, "releases": [ { "package": "api", "from": "1.4.2", "to": "1.5.0", "bump": "minor", ... } ] }
```

Fields may be added within a schema version; removals or meaning changes bump it.

### Offline and local rehearsal (#25)

Three levels of "don't touch the upstream", which matter both for dogfooding cicada in its own repo and
for `check` running on forked PRs where no token is available:

| | Writes files | Local commit/tag | Pushes | Forge API | Token |
|---|---|---|---|---|---|
| `plan`, `check` | — | — | — | — | not required |
| `--dry-run` | — | — | — | — | not required |
| `--no-push` | ✓ | ✓ | — | — | not required |
| default | ✓ | ✓ | ✓ | ✓ | required |

`--no-push` leaves the result in the working tree and on a local branch/tag, so the diff cicada *would*
have pushed can be inspected with plain `git diff` and `git show`. `plan` and `check` must never acquire
a network dependency — that is a goal (§Goals 6), not an implementation detail.

### `cicada check`

Takes a **git range, not a forge PR object** — no API call, no token, identical on GitHub, GitLab, and a
laptop. CI templates wire the range in from their own environment variables.

```
$ cicada check --base origin/main --head HEAD
  ✓ feat(api,worker): add retry backoff
  ✓ fix(gateway): handle 429
  ✗ feat(aip): tune pool size
      unknown scope 'aip' — no such package
      did you mean 'api'?
  ✗ update deps
      not a conventional commit
  2 problems in 4 commits
```

The "did you mean" suggestion (Levenshtein over configured package names) is what turns decision #11's
hard error from an obstacle into a helpful one. Exit non-zero, `--json` for annotations.

`commit_source = "title"` lints the PR title instead of the commits, because under squash-merge the title
is the only thing that lands on the default branch.

---

## Forge abstraction

```rust
trait Forge {
    fn find_pull_request(&self, head: &str)          -> Result<Option<PullRequest>>;
    fn create_pull_request(&self, req: PrReq)        -> Result<PullRequest>;
    fn update_pull_request(&self, id: u64, r: PrReq) -> Result<PullRequest>;
    fn close_pull_request(&self, id: u64)            -> Result<()>;   // #34, empty plan
    fn set_draft(&self, id: u64, draft: bool)        -> Result<()>;   // #33, staleness layer 2
    fn comment(&self, id: u64, body: &str)           -> Result<()>;   // #33, staleness layer 2
    fn set_labels(&self, id: u64, labels: &[String]) -> Result<()>;
    fn find_release(&self, tag: &str)                -> Result<Option<Release>>;
    fn create_release(&self, req: ReleaseReq)        -> Result<Release>;
}
```

**Nine methods** (#53) — the staleness model (#32–34) needs drafting, commenting, and closing, which the
original three-method sketch predated. Still a thin edge: everything else is git. Adapters: `github`
(REST v3, GHE via `api_url`), `gitlab` (REST v4, self-hosted via `api_url`). Bitbucket/Gitea are later
additions needing no core change.

`find_pull_request` keys on the **head branch name**, never the title — that is what makes the upsert
idempotent and stops concurrent runs opening two PRs.

### Forge selection is explicit, detection happens once (#66)

`forge.kind` is **required at release time**; cicada never guesses from the remote URL mid-pipeline. But
requiring it need not be unhelpful: a missing `forge.kind` produces an error that parses `origin`, names the
value it *would* have detected, and prints the stanza to paste. Detection is a diagnostic, never a decision —
the runtime reads what a human approved.

The same applies to `repo`: derivable from `origin`, but explicit in config. GitLab subgroups
(`gitlab.internal/group/sub/proj`) are URL-encoded for the API path.

### Draft PRs are not symmetric (#54)

Staleness layer 2 depends on drafting, and the two forges implement it completely differently:

- **GitHub** — a real API field (`draft: true`). Historically unavailable on private repos below certain plans.
- **GitLab** — there is no flag at all. Draft state *is* a `Draft:` prefix on the MR title, so the adapter
  prefixes and strips a string.

Where drafting is unavailable, the fallback is a title prefix plus the `cicada:stale` label plus a failing
`cicada/freshness` commit status — both forges support statuses, and a required status blocks the merge.

### Cicada's commit identity (#55)

Decision #32 decides whether to clobber the release PR branch by asking whether its tip commit is cicada's own,
so cicada needs a stable, configurable identity:

```toml
committer = "cicada[bot] <cicada@users.noreply.github.com>"
```

It must not drift between cicada versions. If it does, every existing release PR branch suddenly looks
human-authored and every repo starts refusing to regenerate.

### Errors, retries, rate limits (#58)

Nine methods that can each return 429 or 5xx. Retry with exponential backoff on 5xx and 429 (honouring
`Retry-After`), never on 4xx. Errors are mapped to actionable text: a 403 caused by a token lacking
`pull-requests: write` must say that, not print a status code.

### The release manifest is embedded, not uploaded (#57)

GitHub releases host uploaded files; **GitLab releases do not** — they hold *links* to artifacts stored
elsewhere. "Attach the manifest to the release" is therefore not one operation, so cicada does not attach it:
the manifest is embedded in the release body inside a collapsed `<details>` block of fenced JSON, and written
to `target/cicada/release-manifest.json` on disk. Identical behaviour on every forge, no asset API, and still
greppable by a human reading the release. Per-forge asset upload is future work.

### Auth

Token-only, always, for both the API and the git push.

- Sourced from `--token` or `CICADA_TOKEN`. **Never** written to `.git/config`, never to disk, never in `argv`.
- Passed to git through **`GIT_ASKPASS`** — a tiny helper that echoes the value from the process environment.
- `GIT_TERMINAL_PROMPT=0` so a missing credential fails immediately instead of hanging a CI job.
- Token auth is HTTP Basic, so the push uses HTTPS. If `origin` is an SSH URL, cicada derives the HTTPS
  equivalent **in memory** for its push and leaves the configured remote untouched.
- Username per forge: `x-access-token` (GitHub), `oauth2` (GitLab).

### Permissions and the CI-trigger footgun

Inherited from the forges, and prominent in the docs rather than a footnote — because a release PR whose
CI never runs defeats the entire point of decision #7:

- **GitHub:** `contents: write`, `pull-requests: write`, `issues: write`, plus *Allow GitHub Actions to
  create and approve pull requests*. The default `GITHUB_TOKEN` **does not trigger workflows on PRs it
  creates** — a PAT or GitHub App token is required for CI to run on the release PR.
- **GitLab:** Project Access Token, Maintainer role, scopes `api` + `write_repository`. `CI_JOB_TOKEN`
  has the same non-triggering problem for MRs.

---

## Templates

**Two** things are rendered, both with `minijinja` (Jinja2 syntax — familiar to anyone who has touched Helm
or Ansible). The shipped GitHub Action and GitLab CI `include:` are *not* templates: they are static YAML
assets users reference, and minijinja never touches them. See §Distribution.

1. **File templates** — user-declared `template` → `output` pairs (an updater `kind`, #13).
2. **The notes template** — the release entry body.

### One notes template, two destinations (#43)

The changelog entry and the forge release body are the same content in different frames: the changelog adds
a version header and handles the `<!-- cicada:insert -->` marker, the release body does not repeat the
version. So **one template produces the body** and both consume it. Two independent templates would drift
apart within a month. A per-target override exists for the rare case where they genuinely must differ.

Link styles — compare URLs, commit URLs, issue references — are supplied to the template as a `LinkStyle`
**value** by the forge adapter, never as a dependency (see §Architecture).

### Three contexts (#46)

`commits` means something different in each, so they are defined separately rather than as one flat list:

| Context | Renders | `commits` holds | Also available |
|---|---|---|---|
| File template | once per package release | that package's commits | `version`, `previous_version`, `package`, `tag`, `sha`, `date` |
| Notes template | once per package release | that package's commits, **pre-grouped into sections** | the above, plus `links`, `breaking` |
| Umbrella changelog | once per run | every release in the run, grouped by package | the above, plus `releases` |

Section grouping and ordering stay **config-driven** (`[changelog] sections`). The template receives
sections already grouped and decides only how to render them — otherwise config and template could
contradict each other, and the config would silently lose.

### Overrides and defaults (#47, #48)

Overrides are picked up by convention, no configuration needed:

```
.cicada/templates/notes.md.j2        overrides the built-in notes template
.cicada/templates/changelog.md.j2    overrides the changelog frame
```

And the built-in defaults are printable, so customising starts from the real thing:

```
cicada template print notes > .cicada/templates/notes.md.j2
```

This is deliberately the opposite of release-please, whose changelog format is notoriously hard to
customise precisely *because* you cannot see the template you are trying to deviate from.

### Safety (#45)

Autoescape is **off** — output is Markdown and YAML, not HTML, and escaping it as HTML would corrupt it.
That places the burden on filters, because commit subjects are semi-untrusted input flowing into generated
files: a subject containing `: ` or a stray quote can produce an invalid YAML manifest.

```jinja
image:
  tag: {{ version | yaml }}
annotations:
  changelog: {{ commits[0].subject | yaml }}
```

`| yaml`, `| json`, and `| md` are provided, and template authors are expected to use them on anything
derived from commit text.

### Collisions (#44)

`template` and `output` paths resolve from the **repo root**, never from the package's `path` — packages
already declare their own paths, and repo-root resolution removes the ambiguity. Two updaters declaring the
same `output` path is a **hard error**: last-writer-wins would be a silent, config-order-dependent bug.

---

## Tech choices

| Concern | Choice | Rationale |
|---|---|---|
| CLI | `clap` (derive) | standard |
| Config | `serde` + `toml` | reading |
| TOML editing | `toml_edit` | preserves formatting and comments on write |
| Versions | `semver` | correctness, prerelease ordering |
| Templates | `minijinja` | small, fast, Jinja2 syntax |
| HTTP | `ureq` + `rustls` | blocking — avoids pulling in tokio for ~4 API calls; pure-Rust TLS keeps musl static builds trivial |
| Errors | `thiserror` (libs) + `anyhow` (cli) | typed where it matters |
| Logging | `tracing` | structured `-v` output |
| YAML editing | `saphyr-parser` | byte-splice scalars, never re-emit (#49). **Spike-confirmed** — `spikes/yaml-splice` |
| JSON | `serde_json` | manifest emit; span-preserving read for `generic` edits |
| Globs | `globset` | `release_branches` patterns, package discovery |
| Dates | `jiff` | changelog dates; no `chrono` legacy surface |
| Suggestions | `strsim` | the did-you-mean in `cicada check` (#12) |
| Diffing | `similar` | rendering `--dry-run` output as a real diff |
| Testing | `insta` snapshots, `assert_cmd`, temp-repo fixtures | changelog and plan output are snapshot-shaped |

**Edition 2024**, so the MSRV floor is Rust 1.85. Stated explicitly because a release tool tends to run on
whatever Rust a CI image happens to ship.

**No async runtime.** With git as the substrate and only a handful of sequential API calls, `ureq` keeps
the dependency tree and binary small and the code straight-line.

**Git subprocess discipline**, from the first line:
- machine-readable formats only — `--format=%H%x00%s%x00%b`, `-z`, `--porcelain`; never human output
- pinned environment — `LC_ALL=C`, `GIT_CONFIG_NOSYSTEM=1`, `GIT_TERMINAL_PROMPT=0`
- `git --version` checked at startup with a clear error if missing or too old
- all of it behind `GitBackend`, so a `gix` read path can be swapped in later for speed

---

## Distribution

- **Binaries:** `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, statically linked, attached
  to releases. Linux only in v1; macOS/Windows later.
- **Docker:** multi-arch `linux/amd64` + `linux/arm64`. Alpine or Debian-slim base — **must include `git`**
  (decision #17), so `scratch` is out.
- **`cargo install cicada`**.
- **CI templates:** a GitHub composite action and a GitLab CI `include:` — **static YAML assets**, not
  rendered templates. Both call `cicada run`, bake in `fetch-depth: 0` / `GIT_DEPTH: 0`, and set concurrency
  guards (`concurrency:` / `resource_group`).
- **Registry:** `ghcr.io/<org>/cicada` as primary, tagged `v1.2.3`, `v1.2`, `v1`, `latest`.
- **Checksums:** a `SHA256SUMS` file alongside the binaries.
- **Self-release:** cicada releases itself with cicada as soon as Phase A/B work end-to-end. Fastest way
  to find the rough edges, and it makes the repo its own reference implementation. `cicada --version` reports
  `CARGO_PKG_VERSION` — a number cicada bumped itself.

**Cicada does not build artifacts** (#70). It owns versioning, changelogs, tagging and release objects; the
binaries and images are built by a **separate workflow triggered by the tag cicada pushes**. Keeping the two
apart means artifact building can fail and be retried without touching version state, and cicada never grows
a cross-compilation matrix. This is also why `cargo-dist` and cicada compose rather than compete: cicada
decides the version and cuts the tag, the artifact workflow reacts to it.

---

## Milestones

Two sequencing principles, both learned from the decisions above:

- **The multi-package model exists from M1, not later.** `default_scope` (#38) and #16 make a single-service
  repo *the degenerate case* — one entry in the package list. Building a single-package special case first
  would mean building the wrong thing and unwinding it. Length 1 is not a mode.
- **Local rehearsal is proven before any forge code exists** (M3). Everything up to a real release commit on
  a real branch is verifiable with `git show`, with no token and no upstream involvement (Goal 6).

| M | Content | Exit criterion |
|---|---|---|
| **M0** | Workspace (`cicada-core` pure + `cicada`), config parsing, `GitBackend` + subprocess impl, conventional-commit parser (multi-scope, breaking, footers). ~~Spike: span-preserving YAML splice~~ **done** | `cicada check` lints a range with did-you-mean suggestions. ✅ Spike complete: `spikes/yaml-splice`, 15 cases green |
| **M1** | Anchor resolution (per-package ranges, tag reachability), scope routing incl. `default_scope`, bump computation (section `bump`, first-release table, `pre_major`, `maintenance` guard, `Release-As`), umbrella aggregation, `Plan` | `cicada plan --json` correct on a fixture **monorepo and a single-package repo through the same code path**, fully offline |
| **M2** | Updaters (cargo, helm, generic path + marker, template), minijinja notes template with three contexts + filters + `template print`, changelog marker insert/create, versions-file write, `FileEdit` boundary | `release-pr --dry-run` prints exactly the diff it would write; snapshot tests green; duplicate-output and re-parse checks fire |
| **M3** | Git write path: release PR branch reset/commit, committer identity, `release_branches` allowlist, templated `release_pr_branch`, `--no-push` end to end | **`release-pr --no-push` produces a real local release commit**, inspectable with `git show`. First dogfooding gate — no token, no network |
| **M4** | `Forge` trait (9 methods) + GitHub adapter, upsert by head branch, auth via `GIT_ASKPASS`, retries/error mapping, **staleness model** (committer guard, draft, comment, close on empty plan) | Real release PR on GitHub; hand-editing the release PR branch makes cicada refuse, draft the PR, and fail loudly |
| **M5** | Phase B: tag reconciliation (reachability + collision), **independent release-object reconciliation**, stale-merge detection, release manifest (disk + embedded), `cicada run` ordering | merge → tag → release; re-run is a no-op; a simulated failed release call self-heals on the next run; a maintenance branch releases independently of `main` |
| **M6** | GitLab adapter (REST v4, draft-as-title-prefix, self-hosted `api_url`) | Same flow green on GitLab |
| **M7** | Shipped CI templates (Actions composite action, GitLab `include:`), docs incl. the CI-trigger footgun and a worked `cicada.toml` per repo shape | Third party can adopt from the README alone |
| **M8** | musl static binaries (x86_64, aarch64), multi-arch Docker with `git`, self-release | cicada v0.1.0 released by cicada |

**M0's unproven assumption has been discharged.** Decision #49 — span-preserving parse and byte splice — is
what every updater rests on. `spikes/yaml-splice` confirms it works, names the crate, and captures the two
traps (char-vs-byte indexing, and tolerating aliases outside the target) as regression tests. Everything
remaining in this plan is assembly of known parts.

**First genuinely useful build is M3** (offline planning and local release commits, usable on this repo).
**First end-to-end release tool is M5**, on GitHub.

---

## Open questions

None blocking. The design is complete enough to build M0 against.

Deferred scope lives in `docs/future-work.md` — twelve items, each with why it was deferred and what would
trigger picking it up. Behavioural coverage lives in `docs/scenarios/` — one file per repo shape, written as
executable scenarios, which is where decisions #73–#79 came from.

The M0 spike is **complete and #49 holds** — see `spikes/yaml-splice`. Five items are left to first contact
with the code:

1. **`check`'s title mode.** §`cicada check` routes it through `commit_source = "title"`, which contradicts
   that setting's own enum (`commits` | `merges` | `both`) and would be meaningless for history reading. The
   mode belongs on the existing `--title` flag or a setting of its own.
2. **`{summary}` in `release_pr_title`** has no defined rendering, for one package or for many.
3. **A derived chart bump on a never-released chart** (#22 meeting #37) lands on `0.0.1`, which is a strange
   first chart version. `0.1.0`, or requiring `initial_version`, are the alternatives.
4. **A promoted prerelease line is stale until `main` is merged back** (F9). The `prerelease` key is deleted
   by a commit on `main`, so the line still reads `version = "1.4.2"`, `prerelease = "1.5.0-rc.2"` and cuts
   `1.5.0-rc.3` — an rc for a version already shipped, sorting below it. No guard fires. Either accept it and
   document the merge-back, or extend #9's reachability primitive from the tag being created to the computed
   stable base.
5. **`from` on a first release in `--json`** (delta 15): `null`, absent, or `"0.0.0"`. A machine interface
   with a compatibility promise (#69) should not leave this to the implementation.

Two smaller ones, both on `Release-As:`: whether a prerelease value is legal there outside the §Prereleases
flow, and whether it forces a release when attached to a `bump = "none"` type.

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
- Adopting cicada into a repo with existing release history. `init` emits an example config; matching it
  to an existing repo's tags and layout is the developer's job, done once, by hand.
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
| 1 | Two files: `cicada.toml` (human-owned config), `.cicada-versions.toml` (tool-owned versions + anchor SHA) |
| 2 | Commit-range anchor: derive tag from current version → resolve; fall back to recorded SHA; neither resolves → hard error |
| 3 | The tag's commit must be a **descendant of (or equal to)** the recorded SHA; otherwise hard error. Equality is *not* expected — the tag lands on the merge commit, the SHA was recorded pre-merge |
| 4 | "Never released" = **absent entry** in the versions file. No sentinel version |
| 5 | Changelog inserted at `<!-- cicada:insert -->`; fallback prepend-after-header; file created if absent |
| 6 | Tags are write-only, `tag_format` configurable, tagging disable-able |
| 7 | Release **PR/MR flow** — humans review the diff before anything is tagged |
| 8 | Monorepo packages are an **explicit list** in `cicada.toml`; `cicada init` generates it by scanning |
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
| 23 | `cicada init` writes `cicada.example.toml` — never clobbers a live config; the human reviews and renames |
| 24 | Cargo workspace-inherited versions: `init` warns loudly and writes anyway; `plan` hard-errors if two packages share one version field |
| 25 | `plan`/`check` need no token and no network; `release-pr`/`release` take `--no-push` for local rehearsal |
| 26 | cicada acts on the **currently checked-out branch**, gated by a `release_branches` allowlist; a non-matching branch exits 0 quietly |
| 27 | Release branch name is templated per target branch (`cicada/release/{branch}`); the PR/MR base is the branch cicada ran on |
| 28 | `max_bump` clamps a branch's bumps (maintenance lines take `patch` only) |
| 29 | Two crates: `cicada-core` (pure, no I/O deps — purity enforced by the compiler) and `cicada` (binary; git, forge, updaters as modules) |
| 30 | Core emits `Vec<FileEdit>` as **data**; the I/O layer only writes them. `--dry-run` is therefore not a separate code path |
| 31 | Ordered maps keyed by config order, never `HashMap` — output determinism is observable |
| 32 | Cicada **replaces** the release branch each run. If its tip is not cicada's own commit, cicada refuses to clobber: drafts the PR, labels `cicada:stale`, comments, exits non-zero |
| 33 | Stale-merge prevention in three layers: branch protection (documented) → cicada auto-drafts (enforced) → Phase B hard error (backstop) |
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
| 59 | Prerelease is **branch-level**, not per-package: `[settings] prerelease = "rc"` on that branch, plus `--prerelease` for ad-hoc use |
| 60 | Versions file gains a `prerelease` field; `version` keeps its exact meaning — **last stable release** — so anchor semantics are untouched |
| 61 | Every release, rc and stable alike, computes from the last **stable** anchor: notes are cumulative and the stable changelog is complete |
| 62 | Prereleases write version files and cut tags + forge releases, but **never write `CHANGELOG.md`** |
| 63 | The rc counter increments while the computed base is unchanged; a changed base (`1.5.0` → `2.0.0`) resets it to `.1` |
| 64 | Prerelease identifiers are free strings with dotted numbering (`1.5.0-rc.1`) — sorts numerically per semver |
| 65 | Determinism is a **stated goal** (§Goals 7), not an emergent property |
| 66 | `forge.kind` is **required at runtime**; `cicada init` detects it from `origin` and writes it down. Detection never happens silently at release time |
| 67 | Config parsing **denies unknown fields** — a mistyped key is an error, never a silent no-op |
| 68 | Exit codes are a contract: `0` ok · `1` error · `2` usage · `3` check found problems · `4` refused/blocked |
| 69 | `--json` output carries a `schema` version field; it is a machine interface with a compatibility promise |
| 70 | Cicada owns versioning, tagging and releases; **artifact building is a separate tag-triggered workflow** |
| 71 | YAML splicing uses `saphyr-parser` — **spike-confirmed** (`spikes/yaml-splice`). `Marker::index()` is a **char** index despite its doc comment; it must be converted to a byte offset or non-ASCII files corrupt silently |
| 72 | The path walker **tolerates unsupported constructs outside the addressed path** (aliases, complex keys); only an unaddressable *target* is an error |

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

**Phase B runs before Phase A** (#35). Both are triggered by the same event — a push to a release branch —
so the order is CI's to get right, and getting it wrong is silent. If A runs first, it computes its range
from the *old* tag, re-includes everything the just-merged release contained, and proposes releasing it
again with duplicate changelog entries, every cycle. `cicada run` does both in the correct order so a CI
file cannot express the broken one.

### Phase A — `cicada release-pr`

Runs on every push to a branch matching `release_branches`.

0. Resolve HEAD's branch; if it doesn't match `release_branches`, exit 0 quietly.
1. Load config + versions file — **both read from this branch**, so per-branch config diverges for free.
2. For each package: resolve the anchor (tag → SHA → error) and collect `anchor..HEAD`.
3. Parse commits, route by scope, compute bumps → `Plan`.
4. If the plan is empty: **close any open release PR** for this branch (#34) and exit 0. Leaving it open
   would advertise a release that is no longer justified — e.g. after the only `feat:` was reverted.
5. Render the plan to `Vec<FileEdit>` — version files, changelogs, versions file (new versions, `sha` = HEAD),
   templates.
6. Inspect the release branch tip (#32). If it exists and its committer is not cicada, **stop**: draft the
   PR, apply `cicada:stale`, comment why, exit non-zero. Do not clobber.
7. Reset `cicada/release/{branch}` to HEAD, write the edits, commit, force-push, upsert the PR/MR **with
   base = the branch cicada ran on**.

### The release branch is replaced, never updated

It is the PR's *head*; the release branch cicada ran on is the *base*. Nothing is ever merged into it.

```
t0   main     A──B
     release       ╲──R1        R1 computed from B, changelog covers …B

t1   two commits land on main
     main     A──B──C──D
     release       ╲──R1        R1 is now stale — knows nothing about C, D

t2   Phase A runs
     main     A──B──C──D
     release             ╲──R2  R2 computed from D, changelog covers …D
     force-push; R1 is discarded
```

R1 is not rebased or merged into R2 — it is thrown away, and R2 is computed from scratch against main's
current tip. There is never a conflict to resolve, because nothing is being reconciled: the branch holds
exactly one commit that is a pure function of the current repo state. The PR is the same PR (same head
branch), showing an entirely new diff.

This is also why the upsert must key on the **head branch name**, never the title: it is idempotent by
construction, so two concurrent Phase A jobs cannot open two PRs. Concurrent runs still race on the
force-push, which is self-correcting — but the shipped CI templates set concurrency guards anyway
(`concurrency: cicada-release` on Actions, `resource_group` on GitLab) to avoid the wasted work.

### Staleness (#32, #33)

A human commit on the release branch is the one thing on it that is *not* derivable from the base branch,
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
| 3. Phase B hard error | Recorded `sha` vs HEAD: if `sha..HEAD` holds anything but cicada's own release commit, the PR was merged stale | Someone deliberately un-drafted and merged anyway |

Layer 3 hard-errors rather than warning, with eyes open: main is then merged-but-untagged and needs manual
repair. That is the correct trade — a visible unreleased state beats an invisible mis-versioned release.

Labels and draft state are **outputs for humans and the forge**, never inputs cicada reads back to decide
anything. Decision #9 is intact.

### Phase B — `cicada release`

Also runs on every push to a matching branch. **Stateless reconciliation:**

```
for pkg in config.packages + umbrella:
    tag = tag_for(pkg, versions_file[pkg])

    match resolve(tag):                          # artifact 1: the tag
        exists, NOT reachable from HEAD -> ERROR: version collision
        exists, reachable               -> already tagged
        missing                         -> create and push tag

    if no release object exists for tag:         # artifact 2: the release
        create release (notes + embedded manifest)
```

**Tag and release object are reconciled independently** (#56). Skipping the whole package when the tag exists
would break the self-healing property below: if tagging succeeded and the release API call then failed, the
release object would never be created on any later run. Two artifacts, two checks, both idempotent.

Before tagging, Phase B also checks that the merged PR was not stale (layer 3 above):

```
covered = recorded sha              # the commit the changelog actually describes
if commits(covered..HEAD) contains anything but cicada's own release commit:
    ERROR: release PR was merged while stale — N commits are being released
           with no changelog entry and no effect on the version
```

This is the first real use of the `sha` field beyond being a fallback anchor: it records *what the
changelog covers*, which is a different fact from *what is released*.

**Reachability is load-bearing, not a nicety.** Cicada's own tags are always ancestors of HEAD on a later
run, so idempotency is preserved. But a tag that exists *elsewhere* in the repo — on another branch, at a
version this branch is only now reaching — means two branches are about to claim the same version. Skipping
it would silently swallow a release: commits made, changelog written, nothing tagged or published. It is an
error, and a loud one.

Consequences, all of them good:

- Immune to squash-merge, merge commits, rebase-merge, and rewritten commit messages.
- Idempotent — re-running a pipeline double-tags nothing.
- Self-healing — if tagging succeeded but the release API call failed, the next push creates just the missing release object (#56).
- A human hand-editing the versions file and pushing is a valid manual release.
- Zero forge state. No labels, no PR queries, no API call needed to decide anything.

The one limitation: with `tagging = false`, release detection has no signal and falls back to diffing the
versions file against the parent commit. Documented, not worked around.

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
| Two branches pushing the same release branch | Templated name: `cicada/release/{branch}` |
| PR opened against the wrong base | Base = the branch cicada ran on |
| cicada acting on branches it shouldn't | `release_branches` allowlist; non-match exits 0 |
| Cherry-picked `feat:` producing `1.5.0` on a maintenance line | `max_bump = "patch"` in that branch's config |
| Two branches reaching the same version number | Reachability check → version-collision error (#9) |

Per-branch config divergence is free and useful: `release/1.4`'s own `cicada.toml` carries
`max_bump = "patch"`, and `main`'s does not, because each is read from the branch being released.

**Known divergence, accepted:** `main`'s `CHANGELOG.md` never learns about `1.4.8`, since that entry was
written on the maintenance branch. Merging changelogs across lines is out of scope.

---

## Configuration

### `cicada.toml` — human-owned

```toml
[settings]
release_branches = ["main", "release/*"]        # cicada acts only if HEAD's branch matches (#26)
release_branch   = "cicada/release/{branch}"    # templated, so branches never collide (#27)
default_scope    = "platform"                   # unscoped commits land here (#38)
committer        = "cicada[bot] <cicada@users.noreply.github.com>"   # stable — #32's guard depends on it
# prerelease     = "rc"                         # set on a `next`/`beta` branch to cut prereleases (#59)
commit_source    = "commits"                    # "commits" (--no-merges) | "merges" (--first-parent) | "both" (#40)
release_pr_title = "chore(release): {summary}"  # conventional and non-releasable (#41)
tagging          = true
# max_bump       = "patch"                      # on a maintenance branch: never minor/major (#28)

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
[umbrella]
version = "1.7.0"
sha     = "9f2c1a…"

[packages.api]
version    = "1.4.2"        # last STABLE release — the anchor (#60)
sha        = "9f2c1a…"
prerelease = "1.5.0-rc.2"   # present only while a prerelease line is open; deleted when 1.5.0 cuts

# `chart` absent → never released → first release takes all reachable history
```

Written by cicada, committed by the release PR. `sha` is the anchor fallback when the tag is gone. Its path
is fixed — beside `cicada.toml` at the repo root — because tooling that reads it should not have to hunt.

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

That last one is why `init` can warn and write anyway: the config it produces is legible, and `check`/`plan`
is where an unreleasable arrangement becomes an error.

---

## Core algorithms

### Anchor resolution

```
tag = render(tag_format, current_version)
if tag resolves:
    if recorded sha exists and tag's commit is NOT a descendant of it  -> ERROR (mismatch)
    anchor = tag's commit
elif recorded sha resolves:
    anchor = recorded sha                              (warn: tag missing)
elif no entry in versions file:
    anchor = None                                      (first release: all reachable history)
else:
    ERROR: neither tag nor sha resolves
```

**Descendant, not equal** (#3). The recorded SHA is captured when Phase A *computes*; the tag lands on the
merge commit created when the PR is *merged*. They are never equal in normal operation — requiring equality
would hard-error after every single release. The tag's commit being a descendant of the recorded SHA is the
real invariant, and it still catches a tag pointing somewhere unrelated or a hand-edited versions file.

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
- `max_bump` (#28) clamps the umbrella exactly as it clamps children.

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
sha        = "abc123…"
prerelease = "1.5.0-rc.2"   # only while a prerelease line is open
```

Anchor resolution, the descendant check, and range computation are untouched. Every release — rc.1, rc.2, and
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
# cicada.toml on branch `next`
[settings]
prerelease = "rc"
```

Because config is read from the branch (#26), `next` produces `1.5.0-rc.N` and `main` produces stable, using
the same per-branch isolation already built for maintenance branches — **zero new machinery**. Merging `next`
into `main` then just works: main's config has no `prerelease`, so the next run cuts `1.5.0` and drops the
field. A `--prerelease rc` flag covers ad-hoc rcs without a branch.

Phase B reconciles the prerelease tag when the field is present and the stable tag when it is not;
reachability and collision checks apply unchanged.

**Forge asymmetry.** GitHub releases carry a `prerelease: true` flag; **GitLab releases have no equivalent**.
The GitLab adapter marks it in the release name instead. Same class of gap as draft PRs (#54).

### Release manifest

**One manifest per run, not per package** (#42). A single run can release three packages independently; what
deployment needs is the set that shipped together — package, version, tag, sha, timestamp.

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
cicada init [--output <path>]   scan the repo, write cicada.example.toml for review
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
| `4` | Refused — stale release branch (#32), version collision (#9), stale merge (#33) |

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

### `cicada init`

One-shot, non-interactive, and **it never writes `cicada.toml`** — it writes `cicada.example.toml`
(overridable with `--output`) for the human to read, edit, and rename. Running it twice can never
destroy a live config.

Discovery walks the repo honoring `.gitignore` and collects two marker types:

- **`Cargo.toml`** — classified into virtual manifest (`[workspace]`, no `[package]`; not a package,
  but supplies the member globs), real package, or version-inheriting package (`version.workspace = true`).
- **`Chart.yaml`** — reading `name`, `version`, `appVersion`. **Anything under a `charts/` subdirectory
  is skipped**: that is Helm's vendored-subchart location, and treating those as packages would invent a
  dozen phantom entries in any repo where `helm dependency update` has ever run. A chart carrying a
  `dependencies:` list is proposed as the umbrella.

Skipped everywhere: `target/`, `vendor/`, `node_modules/`, `.git/`, and gitignored paths.

App and chart are emitted as **separate packages** (#14), paired by name match (crate name ≡ chart name),
falling back to directory name, with the chart's `app_version.track` pre-wired to its app. Pairings it
cannot make confidently are emitted separately with a comment.

```
Scanning…

  found  3 cargo packages   api, worker, auth
  found  2 helm charts      api, platform
  skipped 1 vendored subchart (deploy/charts/api/charts/redis)

  paired  services/api  ⇄  deploy/charts/api    → 'api' + 'api-chart' (appVersion tracks 'api')
  package services/worker                       → 'worker' (no chart found)
  package libs/auth                             → 'auth'
  umbrella candidate: deploy/charts/platform    (chart declares dependencies)

  ⚠  'worker' uses version.workspace = true — its version is shared across the whole
     workspace and cannot be bumped independently. Remove the inheritance, or group
     those crates into one cicada package.

Wrote cicada.example.toml — review, edit, rename to cicada.toml.
```

`init` is allowed to be imperfect precisely because decision #8 makes the list explicit and
human-reviewed: discovery heuristics run **once, under review**, never on every release.

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
requiring it need not be unhelpful: `cicada init` parses `origin`, detects the forge, and writes it into the
generated config, and a missing `forge.kind` produces an error that names the value it *would* have detected,
ready to paste. Detection is a one-time authoring convenience; the runtime reads what a human approved.

The same applies to `repo`: derived from `origin` by `init`, explicit thereafter. GitLab subgroups
(`gitlab.internal/group/sub/proj`) are URL-encoded for the API path.

### Draft PRs are not symmetric (#54)

Staleness layer 2 depends on drafting, and the two forges implement it completely differently:

- **GitHub** — a real API field (`draft: true`). Historically unavailable on private repos below certain plans.
- **GitLab** — there is no flag at all. Draft state *is* a `Draft:` prefix on the MR title, so the adapter
  prefixes and strips a string.

Where drafting is unavailable, the fallback is a title prefix plus the `cicada:stale` label plus a failing
`cicada/freshness` commit status — both forges support statuses, and a required status blocks the merge.

### Cicada's commit identity (#55)

Decision #32 decides whether to clobber the release branch by asking whether its tip commit is cicada's own,
so cicada needs a stable, configurable identity:

```toml
committer = "cicada[bot] <cicada@users.noreply.github.com>"
```

It must not drift between cicada versions. If it does, every existing release branch suddenly looks
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
| **M1** | Anchor resolution (per-package ranges, descendant check), scope routing incl. `default_scope`, bump computation (section `bump`, first-release table, `pre_major`, `max_bump`, `Release-As`), umbrella aggregation, `Plan` | `cicada plan --json` correct on a fixture **monorepo and a single-package repo through the same code path**, fully offline |
| **M2** | Updaters (cargo, helm, generic path + marker, template), minijinja notes template with three contexts + filters + `template print`, changelog marker insert/create, versions-file write, `FileEdit` boundary | `release-pr --dry-run` prints exactly the diff it would write; snapshot tests green; duplicate-output and re-parse checks fire |
| **M3** | Git write path: branch reset/commit, committer identity, `release_branches` allowlist, templated branch name, `--no-push` end to end | **`release-pr --no-push` produces a real local release commit**, inspectable with `git show`. First dogfooding gate — no token, no network |
| **M4** | `Forge` trait (9 methods) + GitHub adapter, upsert by head branch, auth via `GIT_ASKPASS`, retries/error mapping, **staleness model** (committer guard, draft, comment, close on empty plan) | Real release PR on GitHub; hand-editing the release branch makes cicada refuse, draft the PR, and fail loudly |
| **M5** | Phase B: tag reconciliation (reachability + collision), **independent release-object reconciliation**, stale-merge detection, release manifest (disk + embedded), `cicada run` ordering | merge → tag → release; re-run is a no-op; a simulated failed release call self-heals on the next run; a maintenance branch releases independently of `main` |
| **M6** | GitLab adapter (REST v4, draft-as-title-prefix, self-hosted `api_url`) | Same flow green on GitLab |
| **M7** | `cicada init` (discovery, app⇄chart pairing, workspace-inheritance warning), shipped CI templates, docs incl. the CI-trigger footgun | Third party can adopt from the README alone |
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

Deferred scope lives in `docs/future-work.md` — ten items, each with why it was deferred and what would
trigger picking it up.

The M0 spike is **complete and #49 holds** — see `spikes/yaml-splice`. One item is left to first contact
with the code:

1. **`cicada init` and prerelease branches.** `init` emits a commented-out `prerelease` stanza rather than
   proposing a `next` branch — a prerelease workflow is a deliberate choice, not something to infer from a
   repo scan.

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
7. **Deterministic and idempotent** — the same repo state **and the same date** produce the same output, and
   a second run changes nothing. A constraint on every feature, not an emergent property. The date is in that
   sentence because changelog entries carry one; it is an explicit input rather than an ambient clock read
   (#95), so the property holds exactly rather than approximately.

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
- **Version schemes other than semver.** CalVer and build-number versioning are deferred (future-work §17),
  which also records the narrow, semver-shaped form that would cost least. Ordering is what every refusal in
  this design rests on (#9, #76, #85, #90), and a second scheme is a second arithmetic at minimum.
- **Releasing a range that contains nothing releasable.** Permanent, not deferred — see below.

### Cadence releases: the trigger is yours, the content is cicada's

Worth separating, because most of what "release on a cadence" means already works and the part that does not
will never be added.

**Running cicada on a schedule needs nothing.** The trigger has never been part of the gate: #26 gates on the
*branch*, not on the event that started the run. A cron job calling `cicada run` every second Thursday
releases whatever accumulated since the last release, exactly as a push-triggered run would. So does a human
running it by hand. Nothing in the design cares which.

**What cicada will never do is release when the range justifies no release** (#36). If the fortnight held only
`chore(deps):` commits, a scheduled run produces nothing, and that is the correct answer rather than a missing
feature:

- **There is no number to compute.** A version is a statement about what changed; a range with no releasable
  change has nothing to say, so cicada would have to *invent* a bump. Inventing is the one thing this design
  refuses everywhere else — it is the whole argument behind #39, #11 and #66.
- **The artifact would carry no information.** A release whose changelog is empty is indistinguishable from
  its predecessor except for the tag, and every consumer that upgrades to it does so for nothing. Publishing
  noise into a version history is worse than publishing less often.
- **The honest version of the request is already satisfied.** "We ship on Thursdays" means *we do not ship
  between Thursdays* — a statement about the trigger, which is yours. It does not mean *we ship even when
  there is nothing to ship*, which is what the unsupported half would be.

A repo that genuinely wants a version stamped on a schedule regardless of content is asking for calendar
versioning, not for a scheduled semver release — and that is future-work §17, where it belongs.

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
| 20 | v1 flags: `--dry-run`, `--json`, prerelease support. **Forced versions are not in v1** (#39) |
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
| 39 | *(withdrawn — forced versions are out of v1 scope. The `Release-As:` footer and `--release-as` are both removed; see future-work §13. Number retained so earlier references stay valid.)* |
| 40 | History reading: `commit_source = "commits"` (`--no-merges`, default) \| `"merges"` (`--first-parent`) \| `"both"`. Identical on squash/rebase repos; only affects merge-commit repos |
| 41 | Release PR title defaults to `chore(release): …` — conventional (survives commit linting) and non-releasable (#36) |
| 42 | One release manifest per run, written to `target/cicada/release-manifest.json` (gitignored) **and** embedded in **every** release object that run creates. Not the umbrella's alone — `[umbrella]` is optional (#73), so that would leave the manifest with no home in a repo that declares none |
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
| 57 | Release manifest is embedded in each release body as fenced JSON inside a collapsed `<details>` block **and** written to disk; no asset upload in v1 |
| 58 | Forge HTTP: retry with backoff on 5xx/429 (honouring `Retry-After`), never on 4xx; errors mapped to actionable text |
| 59 | Prerelease is **branch-level**, not per-package: `prerelease_branches` is a glob list, exactly like `maintenance_branches`, and `prerelease_identifier` names what its prereleases are called. Plus `--prerelease` for ad-hoc use |
| 60 | Versions file gains a `prerelease` field; `version` keeps its exact meaning — **last stable release** — so anchor semantics are untouched |
| 61 | Every release, rc and stable alike, computes from the last **stable** anchor: notes are cumulative and the stable changelog is complete |
| 62 | Prereleases write version files and cut tags + forge releases, but **never write `CHANGELOG.md`** |
| 63 | The prerelease counter increments while the computed base **and** the identifier are unchanged; a change to **either** resets it to `.1` — a changed base (`1.5.0` → `2.0.0`), or a stage change (`alpha.3` → `beta.1`) (#93) |
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
| 76 | A hand-edited version in `.cicada-versions.toml` must be **strictly greater** than the one it replaces — the only way a human states a version in v1 (#39), and enforced by #33's one-sided comparison rather than a separate rule |
| 77 | `find_pull_request` searches **open PRs only**; a closed release PR is never reopened. Idempotency rests on the head branch name, not on PR identity |
| 78 | The stale-merge recomputation runs **only when a tag is about to be created** — never on an ordinary push, where the range legitimately holds new commits |
| 79 | `check` exits `3` for semantic findings (config or commits) and `1` for a config that will not parse |
| 80 | The version-collision check (#9) runs in **Phase A as well as Phase B** — a computed version whose tag already exists unreachably refuses to open a PR, rather than failing after review and merge |
| 81 | v1 supports **one release arrangement: trunk releases** — a releasing trunk plus maintenance lines cut behind it. Release-branch releases (a trunk that never releases, every line owning its own version line) need a way to state a line's opening version, which #39 removed; deferred to future-work §14. cicada still has no `release_strategy` setting: the arrangement is expressed entirely by `release_branches` and `maintenance_branches` |
| 82 | Branch-*identity* settings are declared as **glob patterns matched against the current branch name** (`release_branches`, `maintenance_branches`, `prerelease_branches`), never as scalar values living on the branch they describe. Config is still read from the current branch (#26) — the patterns make it identical on every branch, so cutting a line needs no config commit |
| 83 | `release_branches` is **required** — cicada refuses to guess where it may write. The two branch *classifiers* default to a convention instead: `maintenance_branches = ["maintenance/**", "maintenance-*"]` and `prerelease_branches = ["prerelease/**", "prerelease-*"]` with `prerelease_identifier = "rc"`. Safe because they only ever classify branches `release_branches` already admitted. Every default is tabulated in §Configuration, so "what happens if I omit this" is never answered by reading the source |
| 84 | **One identifier at a time.** `prerelease_identifier` is a single scalar, never a per-branch mapping — so two patterns matching one branch need no tie-break and the setting needs no ordered map. It is *at a time*, not forever: the value may move as the line matures (#93). What cicada never expresses is two stages in flight **simultaneously** — a `beta` line and an `rc` line running in parallel off one repo |
| 85 | A computed prerelease must be **strictly greater** than the open one recorded in the versions file — #76's monotonicity rule applied to computed versions, not only forced ones. It is the **only** guarantee a maturity ladder gets (#93): it permits `alpha` → `beta` → `rc` and refuses `rc` → `beta`, without cicada knowing what any of those words mean |
| 86 | `release_pr_title` renders two placeholders, `{branch}` and `{summary}` — a plain `replace`, the same mechanism `release_pr_branch` uses for `{branch}`, not a template engine. The default carries **no** placeholders (`chore(release): prepare release`), so interpolation is opt-in and an existing config keeps its title byte-identical. `{summary}` names the releases in config order, truncated after three with `and N more` — **no umbrella special case**, because naming only the umbrella reads as though nothing else moved. Upsert still keys on the head branch, never the title (#77), so a title that changes per release changes nothing structural |
| 87 | `check` lints a **commit range and nothing else**. There is no `--title` flag and no title mode; linting a squash-merge PR title is future-work §15 |
| 88 | A package with a `helm` updater must declare `initial_version`. Helm requires `version` in `Chart.yaml`, so a chart always has a number its author already knows — cicada neither invents one nor reads `Chart.yaml` back to find it (#75) |
| 89 | `--json` emits `"from": null` on a first release — present and explicit. `"0.0.0"` is a version that never existed and produces broken compare URLs; omitting the key is ambiguous with an older schema (#69) |
| 90 | The staleness guard's `prev` is the highest **stable** tag reachable from HEAD below the recorded version. Prerelease tags are never eligible, so a graduating run cannot replan over an empty range |
| 91 | A prerelease line that outlives its own promotion is the **operator's** to reconcile. cicada acts on what the branch and the config say, so a line still recording an open `prerelease` keeps incrementing its counter; merging the promoted branch back (a fast-forward) is the documented fix |
| 92 | cicada **always opens a release PR** (#7); there is no direct-commit mode. A repo that wants no review gate **auto-merges** it — a forge setting, not a cicada mode. cicada's job is to produce a reviewable artifact; whether anyone gates on it is the repo's decision |
| 93 | A prerelease line **moves through stages** (`alpha` → `beta` → `rc`) by editing `prerelease_identifier` on the branch. The scalar is unchanged (#84); what is stated is that its value may move over time, and that this is the only way a maturity ladder is expressed. The counter resets on a stage change (#63) and the ordering is enforced by #85 alone |
| 94 | cicada takes **no declared ladder** — `prerelease_identifier` is never a list of ordered stages. A declared order could bless a sequence semver disagrees with (`preview` → `beta` sorts *down*), and consumers resolve with semver, not with cicada's config. The published versions are the contract; the config must not contradict them |
| 95 | The date is an **explicit input** — `--date YYYY-MM-DD`, defaulting to today in **UTC** — threaded into `cicada-core` as data, exactly as the forge adapter supplies `LinkStyle` (A2). It is the only clock cicada reads, and it feeds one thing: the date on a changelog or release-notes entry. Core therefore never reads a clock, and `--dry-run` at 23:59 cannot differ from the run that follows it at 00:01 (#30) |
| 96 | Within one run, releases are created in **config order (#31), umbrella last**. Deterministic, so the forge-call log is assertable; umbrella last, so its embedded manifest describes work already done rather than work in flight |
| 97 | A **derived** release (#22) still writes its changelog entry — never skipped. The notes context carries `derived_from` (the package and version that caused it), and the default template renders one line naming the cause when the release has no commits of its own. Core supplies the fact as data; the template decides the words (#46) |
| 98 | An unknown scope is fatal **only on a commit that can release** (#11). A type whose section is `bump = "none"` has no attribution to get wrong, and `chore(deps): bump serde` — the design's own canonical non-releasable example (#36) — names no package on any repo. `check` reports it as a **warning**, so it stays visible without blocking a release |
| 99 | Updater edits **fold** rather than collide. Two surgical updaters may write one file — a `values.yaml` holding both `image.tag` and `labels.version` is ordinary — so each transforms the content the run has so far, in config order (#31), and one `FileEdit` per path comes out. #44's hard error is narrowed to what genuinely cannot fold: a **whole-file** writer (`template`) sharing a path with anything |
| 100 | The release commit is built with **git plumbing into a private index** — never by checking the release PR branch out. The working tree, the real index and the current branch are untouched. `--no-push` is the *local rehearsal* mode (§Goals 6), a failure between two checkouts would strand the run on the wrong branch, and two overlapping Phase A jobs in one workspace (D6) would corrupt each other's checkout |
| 101 | #32's guard inspects the branch **that is about to be overwritten** — the *remote* head branch when cicada will push, not the local one. A fresh CI clone has no local `cicada/main`, so a guard reading `refs/heads/` sees nothing, finds no human commit, and force-pushes over one. cicada fetches the head branch into a private ref before judging, and pushes with `--force-with-lease` against exactly the sha it read |
| 102 | GitHub drafting is a **GraphQL** operation, not a REST field. `draft: true` works when a pull request is created and nowhere else; `PATCH /pulls/{n}` has no such parameter. Staleness layer 2 drafts a PR that already exists, which is precisely the case REST cannot express — so `set_draft` takes the whole `PullRequest` (for its node id) rather than a number, and #54's title-prefix fallback stays reserved for forges where drafting is genuinely unavailable |
| 103 | Monotonicity (#76) is checked **directly** in Phase B, not inferred from the staleness guard. J4b claims a downward hand-edit trips that guard "with no extra machinery"; it cannot, because #90 selects `prev` from tags *below* the recorded version — editing `1.4.2` down to `1.0.0` leaves no eligible `prev`, so the recomputation becomes a *first release* and `1.0.0` clears it. Phase B therefore refuses when a **reachable stable tag sits above** the recorded version. Reachability is what keeps parallel lines out of it (G6, G10) |
| 104 | `version` in `.cicada-versions.toml` is **optional**. A line whose *first* release is a prerelease has no last stable at all, and writing `0.0.0` there would be precisely the sentinel #4 refuses — a version that never existed, which anchor resolution would then go looking for a tag for. Absent means absent, and the anchor is unset. An entry with **neither** `version` nor `prerelease` is a parse error: that is what an absent entry says |
| 105 | `PullRequest` carries its **title**. On GitLab the draft state *is* the title — there is no flag, and `Draft: ` is a prefix the adapter adds and strips (#54) — so setting it needs to know what it is prefixing. The adapter **strips before it prefixes**, or a second refusal produces `Draft: Draft: …` and every one after that adds another |

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

### How the release commit is built (#100)

"Reset the branch and commit the edits" describes the result, not the method, and the obvious method is
wrong for this tool. Checking the release PR branch out, writing the files, committing and checking back
fails in three separate ways:

- **`--no-push` is the local rehearsal mode** (§Goals 6). A developer running it on their own machine must
  get their tree back exactly as it was. A command that leaves you on another branch — or that refuses to
  run because you have uncommitted work — is hostile precisely where it is meant to be safe.
- **A failure between the two checkouts strands the run**, and the caller, on the wrong branch with edits
  applied. Every error path would need to unwind a checkout.
- **Two overlapping Phase A jobs in one workspace would corrupt each other** (D6). The design already
  expects concurrent runs and calls the force-push race self-correcting; a checkout race is not.

So the commit is assembled with plumbing: a **private index** seeded from HEAD's tree, the edits hashed into
it, a tree written, a commit created with `commit-tree`, and the ref moved with `update-ref`. Nothing
observable changes except the branch. The developer's staged and unstaged work is not merely preserved — it
is never consulted, so uncommitted edits cannot be swept into a release.

Three details that follow:

- **File modes are read from the base tree**, not hard-coded. A Makefile or shell script carrying a marker
  comment (#50) is exactly the file most likely to be executable, and `100644` would silently un-execute it.
- **Content is hashed with `--path`**, so a repo with gitattributes filters (`* text=auto`) gets the bytes
  git would have stored had a human run `git add`.
- **A run that changes nothing creates nothing.** If the branch already points at a commit with this exact
  tree and parent, cicada returns it rather than minting a fresh sha for identical content — the timestamp
  alone would otherwise guarantee a new commit and a pointless force-push on every run.

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
| 1. Branch protection | GitHub *"require branches up to date"* / GitLab fast-forward merge. **Repo setting — documented, not enforced by cicada.** Not optional on an auto-merging repo (§No review gate) | The race: main moves and someone merges the PR in the seconds before Phase A regenerates |
| 2. Auto-draft | Cicada converts the PR to **draft** + `cicada:stale` + comment whenever it cannot refresh it. Draft PRs are unmergeable on both forges — enforced by the forge, needing no repo config | The sustained case: cicada is blocked from regenerating (human commits, failed computation) |
| 3. Phase B recomputation | Before creating a tag, Phase B replans from the previous tag; a recorded version **lower** than the recomputed one means the PR was merged stale. Stateless — no SHA is stored | Someone deliberately un-drafted and merged anyway |

Layer 3 hard-errors rather than warning, with eyes open: main is then merged-but-untagged and needs manual
repair. That is the correct trade — a visible unreleased state beats an invisible mis-versioned release.

Labels and draft state are **outputs for humans and the forge**, never inputs cicada reads back to decide
anything. Decision #9 is intact.

### No review gate: auto-merge the PR (#92)

cicada always opens a release PR, and there is no mode that commits a release straight onto a branch. A
continuously-deployed service does not need one: **both forges can merge the PR without a human** — GitHub's
auto-merge, GitLab's merge-when-pipeline-succeeds. Enable it on cicada's PR and the loop becomes

```
push to main  →  cicada opens the release PR  →  CI passes  →  the forge merges it
              →  cicada tags and releases     →  the tag-triggered workflow deploys
```

with no human in it, one extra CI cycle of latency, and a full record of exactly what shipped. The review
gate is the repo's decision, not cicada's; what cicada guarantees is that a reviewable artifact exists to
gate on. A direct-commit mode was considered and rejected on that basis — it would fork the write path and
the staleness model to remove a step the forge already removes.

Two interactions bite exactly the repo that turns this on, and neither is obvious:

- **Auto-merge plus the CI-trigger footgun is silent.** The default `GITHUB_TOKEN` does not trigger workflows
  on PRs it creates (§Permissions), so with auto-merge gated on required status checks the checks never
  start, auto-merge never fires, and the release PR sits open looking perfectly healthy. A PAT or App token
  is not optional here — it is what makes the loop turn at all.
- **Staleness layer 1 stops being advisory.** With a human merging, the window between Phase A regenerating
  and someone clicking merge is minutes and the layer rarely matters. Auto-merge closes the PR the instant
  checks go green, which may be after main has already moved — precisely the race layer 1 covers. *Require
  branches up to date* (GitHub) / fast-forward merge (GitLab) must be on.

### Phase B — `cicada release`

Also runs on every push to a matching branch. **Stateless reconciliation:**

```
for pkg in config.packages, then umbrella if declared:      # config order, umbrella last (#96)
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
    prev     = highest STABLE tag matching pkg.tag_format that is reachable
               from HEAD and below the recorded version  (none -> first release)
               prerelease tags are never eligible (#90)
    expected = plan(prev..HEAD)[pkg].version
    if recorded_version < expected:
        ERROR: release PR was merged while stale — the recorded version is
               lower than the commits in the range justify
```

Phase B already has the planner, so this costs a second `git log` and no stored state at all. It catches
precisely the harm the staleness model exists to prevent: a `feat` shipped under a patch bump, invisibly.

**The comparison is deliberately one-sided.** Only *lower than expected* is an error. A recorded version
*higher* than the recomputation is a deliberate over-bump — a human hand-editing the versions file to cut a
release by hand — which is a supported workflow (J4).

**It does not enforce #76 for free, though an earlier reading of this section claimed it did** (#103). The
claim was that editing the versions file *downwards* puts the recorded version below the recomputed one and
trips this guard. It does not, because `prev` is selected from tags *below the recorded version* (#90): edit
`1.4.2` down to `1.0.0` and there is no eligible `prev` at all, so the recomputation is a **first release**,
which `1.0.0` comfortably clears. The two rules cannot both hold.

So monotonicity is its own check, sitting beside this one under the same #78 gate: **a reachable stable tag
above the recorded version is a refusal.** Reachability is what keeps it off parallel lines — `release/1.4`
records `1.4.8` while `main` has tagged `v2.0.0`, and that tag is not reachable from the maintenance branch,
so it is not that line's business (G6, G10).

**And the recomputation's base is `prev`, not the record.** `expected = plan(prev..HEAD)` means both the
range *and* the arithmetic start from `prev`; anchoring the range at `prev` while basing the bump on the
recorded version applies the bump twice, so an ordinary release recomputes one bump too high and the guard
fires on every single run.

**`prev` is stable-only** (#90). A prerelease tag can match `tag_format` and sort below the recorded version
— `v1.5.0-rc.2` does both when the record says `1.5.0` — so an unqualified "highest tag" would anchor the
recomputation on an rc and replan over a range that may be empty. The guard is one-sided, so it would pass
either way and the bug would never surface; stating the eligibility rule is what stops it being decided by
accident.

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
all. Every other ambiguity in cicada is a hard error — unknown scope (#11), duplicate updater output (#44),
stale merge (#33) — and this was the one place the tool silently answered wrong.

The guard is silent in the normal case: cherry-pick only fixes and it never fires.

**There is no escape hatch, and that is the point** (#39). An earlier design let `Release-As:` bypass bump
computation, so "yes, I know, ship it as `1.4.8` anyway" was one footer away. Forced versions are out of v1
scope, and hand-editing the versions file downwards does not substitute for them — the staleness guard
catches that too, by design (J4b). So the only answers to a `feat:` on a patch-only line are the two that
were always the right ones: put the change on a newer line, or re-do the cherry-pick as the fix it actually
is. A line that promises only patches keeps that promise mechanically rather than by convention.

And because the offending commit is visible in a pull request, `cicada check` reports it **before merge**
rather than at release time. There is no `max_bump = "patch"` scalar, because patch is the only value that
was ever meaningful — a string field accepting one string invites `"minor"` and a parse error for no reason.

**Why a pattern rather than a flag on the branch** (#82). A `maintenance = true` written onto each branch has
to be remembered at every cut, and forgetting it means the guard silently does not apply. It also travels
through merges: promoting a prerelease line merges it into `main`, which would carry that flag onto main and
quietly make the trunk patch-only. A pattern is identical
everywhere, so a merge-back is a no-op on the config file and cutting a line needs no config commit at all:

```
git checkout -b release/1.4 v1.4.7 && git push -u origin release/1.4
```

Cicada picks the branch up on the next push, recognises it as a maintenance line, and enforces the guard.

**Known divergence, accepted:** `main`'s `CHANGELOG.md` never learns about `1.4.8`, since that entry was
written on the maintenance branch. Merging changelogs across lines is out of scope.

---

## Release arrangements

**v1 supports one arrangement: trunk releases** (#81). `main` releases the current line; maintenance branches
serve lines main has already passed. It needs no machinery beyond `release_branches` and
`maintenance_branches`, which is why there is still no `release_strategy` setting — a preset over two existing
keys would unlock no behaviour, and a wrongly declared one would be worse than none.

### Trunk releases

`main` releases the current line. A maintenance branch serves a line main has already passed.

```
main          1.5.0 ── 1.5.1 ── 2.0.0 ──►
               ╲
release/1.4     1.4.8 ── 1.4.9            cut after main reached 1.5.0
```

```toml
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]
```

Lines can never collide, because `release/1.4` is only cut once main is past 1.4. Cherry-picks flow from main
to the older branch. `main` does not match `maintenance_branches`, so the guard never applies to it — every
line the guard *does* apply to was cut behind a version main already shipped, so its first release is a patch
and there is never an opening version to state.

**The failure mode** is cutting the maintenance branch too early — while main is still releasing the same
line. Both branches then compute the same next patch, and the second one to run hits the collision check
(#9, #80). That is a misconfiguration, and the error is the correct answer: if main is still releasing 1.4.x,
the `release/1.4` branch has nothing to do.

### Why release-branch releases are deferred

The other arrangement in the wild inverts the trunk's role: `main` never releases, and every release comes
from a branch that owns its own version line (`release/1.5` shipping `1.5.0` onwards). It is what Istio and
the Chrome milestone branches do, and cicada's branch-identity model expresses the *topology* of it without
any new machinery.

What it cannot express, after #39, is the **first release on a new line**. That release is necessarily a
minor — `release/1.5` opening at `1.5.0` while the repo's last release was `1.4.1` — and `release/*` matching
`maintenance_branches` means the guard refuses exactly that bump. Forced versions were the mechanism for
saying it, and they are out of v1 scope. Hand-editing the versions file does not substitute: the staleness
guard refuses a recorded version *below* the recomputation, which is precisely this case.

So the arrangement is deferred as a unit rather than half-supported (future-work §14). Picking it up means
deciding how a line states its opening version — a narrowed `initial_version`, a guard exception, or forced
versions returning — and that decision belongs with §13 rather than ahead of it.

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
# `{branch}` and `{summary}` are optional; the default below uses neither (#86)
release_pr_title  = "chore({branch}): release {summary}"   # chore(main): release app 1.5.0

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
# initial_version = "1.0.0"   # override the first-release rule (#37); REQUIRED on helm packages (#88)
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

[packages.gateway]
prerelease = "0.1.0-rc.1"   # no stable yet: the line's first release is an rc (#104)

# `chart` absent → never released → first release takes all reachable history
```

`version` is **optional** (#104), and `[packages.gateway]` above is why: a line whose first release is a
prerelease has no last stable to record. `0.0.0` would be the sentinel #4 exists to refuse, and anchor
resolution would go looking for a `v0.0.0` tag that never existed. An entry carrying **neither** key is a
parse error — saying nothing is what an *absent* entry is for.

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
| `release_pr_title` | `"chore(release): prepare release"` | Conventional and non-releasable (#41). Renders `{branch}` and `{summary}`; the default uses neither (#86) |
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
| `initial_version` | the first-release table | breaking → `1.0.0`, feat → `0.1.0`, fix → `0.0.1` (#37). **Required on a package with a `helm` updater** (#88) |
| `on_tracked_bump` | `"patch"` | The derived bump a chart takes when its tracked app releases (#22) |

**Required means required, and there are five — six on a chart.** `release_branches`, `default_scope`,
`forge.kind`, and each package's `path` and `tag_format`, plus `initial_version` on any package carrying a
`helm` updater (#88). Every one of them is a place where a wrong guess is invisible rather than noisy — which
is the same test #66 and #8 already apply.

`initial_version` earns its place on charts for a reason specific to Helm: `version` is **mandatory** in
`Chart.yaml`, so a chart that exists at all already has a number, and its author already knows it. cicada
must not invent a different one — and it cannot discover the existing one either, because `Chart.yaml` is an
output and is never read back (#75). The one place the two could meet is config, so that is where it is
stated. It also dissolves the awkward case a tracking chart otherwise lands in: a *derived* bump (#22) on a
never-released chart would take its first version from the first-release table, and the size of a derived
bump comes from `on_tracked_bump` — a config default that says nothing about the chart at all.

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
- every package with a `helm` updater declares `initial_version` (#88)
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
```

- `type` — matched against configured changelog sections; unknown types are ignored (not errors).
- `scopes` — comma-separated list; each must resolve to a configured package or it is a **hard error** —
  but only on a commit that can release (#98). See *The scope rule is about attribution* below.
- Breaking — `!` after the scope, or a `BREAKING CHANGE:` footer.
- Empty scope → attributed to `default_scope` (#38).

**The scope rule is about attribution, so it reaches exactly as far as attribution does** (#98). An unknown
scope is fatal because a `feat` filed under a scope nobody recognises would be silently dropped from the
release it belongs to — the version would be wrong and nothing would say so. A commit whose type computes
`bump = "none"` cannot do that: it moves no number, and wherever it is attributed it renders the same text.

The case is not hypothetical, and the design already relies on it. `chore(deps): bump serde` is this
document's own worked example of a non-releasable commit (§Bump computation, scenarios B8/B9/D5b), and `deps`
names no package in any repo — it is what Dependabot and Renovate write, unprompted, on every repository they
touch. Under a blanket rule the design would contradict itself, and every repo with a dependency bot would be
unable to release at all until it reconfigured that bot. That is the same adoption wall that made
`Release-As:` an **ignored** footer rather than an error (#39, E1).

So the rule is narrowed to what it is for, and the residue is handled where it belongs: `check` reports an
unresolvable scope on a non-releasable commit as a **warning** — printed, named, did-you-mean included, and
not counted toward exit `3`. A typo stays visible; a bot does not block a release.

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

**Footers cicada does not act on** (#39). The parser reads footers generically, but v1 acts on exactly one:
`BREAKING CHANGE:`. A `Release-As:` footer is **not** honoured — forced versions are out of v1 scope
(future-work §13) — so the version always comes from the commits and the config, never from an instruction
embedded in one commit. The consequence worth stating plainly: there is no per-commit override anywhere in
cicada, and the two places a design reflex reaches for one — the maintenance guard (§The maintenance guard)
and a new release line (§Why release-branch releases are deferred) — are answered by changing what is on the
branch, not by annotating a commit.

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

`initial_version` overrides this outright, and is **required** on a package with a `helm` updater (#88).

**Afterwards, `pre_major` governs 0.x.** With the default `"minor"`, a breaking change at `0.3.0` gives
`0.4.0` — never escaping to 1.0.0 by accident. Reaching 1.0.0 is a deliberate act, and with #39 gone the way
to say it is `pre_major = "major"` plus a breaking change — a statement about the package in config, rather
than an instruction buried in one commit message.
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
then attach `-<id>.N`. **Two things can reset N, and both are recomputed from scratch every run:**

| What changed | Example | N |
|---|---|---|
| nothing | another `fix:` lands | increments — `1.5.0-rc.2` |
| the base | a `feat!:` arrives, `1.5.0` → `2.0.0` | resets — `2.0.0-rc.1` |
| the identifier | the line moves `alpha` → `beta` (#93) | resets — `1.5.0-beta.1` |

The identifier axis is the one that is easy to get wrong. With only the base considered, an `alpha.3` line
moving to `beta` computes an unchanged base, increments, and publishes **`1.5.0-beta.4`** — a counter
continuing across a stage it never belonged to. It even passes #85, because `beta.4` sorts above `alpha.3`,
so nothing catches it. Both axes are read from the current run's inputs; nothing is stored.

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

### One identifier at a time, and how a line changes stage (#84, #93)

`prerelease_identifier` is a **scalar, not a map** — and "one at a time" is not "one forever". A line that
ships `alpha`, then `beta`, then `rc` before it ships `1.5.0` is exactly what Kubernetes and Node do, and
cicada follows it. What it never expresses is two stages **in flight simultaneously** — a `beta` line and an
`rc` line running in parallel off one repo, each with its own counter.

**How a line changes stage** (#93). Edit `prerelease_identifier` on the branch and commit it. That is the
whole mechanism:

```
prerelease_identifier = "alpha"    c1, c2, c3   →  1.5.0-alpha.1 · alpha.2 · alpha.3
                       ↓ config edit, committed on the line
prerelease_identifier = "beta"     c4           →  1.5.0-beta.1        counter resets (#63)
                       ↓
prerelease_identifier = "rc"       c5           →  1.5.0-rc.1
                       ↓ merge to main, which matches no prerelease pattern
                                   —            →  1.5.0               one changelog entry, all of c1..c5
```

The base never moved: every one of those computed `1.5.0` from the same stable anchor, so the notes stayed
cumulative and the stable changelog is complete (#61). Only the label on the pre-release changed, and the
counter restarted each time it did.

**The config edit is the truthful form; `--prerelease <id>` is the ad-hoc one.** Both work and the flag wins
where both apply, but they differ in a way that matters: the config edit lives on the branch, so `cicada plan`
run on a laptop with no arguments reports the stage the line is actually in. A stage that lives only in a CI
argument makes offline planning lie, and offline truthfulness is a stated goal (§Goals 6).

**The ladder's only guarantee is #85, and that is deliberate** (#94). cicada draws no distinction between
identifiers whatsoever: `alpha`, `beta` and `rc` anchor the same way, compute the same base, run the same
counter, skip the same changelog, and produce the same `prerelease: true` release object. The *only* thing
that differs is the string in the version and the ordering that string implies. So the rule is monotonicity
and nothing else — each computed prerelease must sort strictly above the one recorded in the versions file:

| Stage change | Sorts | Result |
|---|---|---|
| `alpha` → `beta` → `rc` | up (`a` < `b` < `r`) | permitted |
| `rc` → `beta` | down | **hard error**, naming both versions |
| `preview` → `beta` | down (`p` > `b`) | **hard error** — a legitimate-looking ladder semver disagrees with |

The third row is why `prerelease_identifier` is not a **declared list** of ordered stages (#94). A declared
order would make `preview` → `beta` legal in cicada while the published versions still sorted the other way
— and a consumer resolving `>=1.5.0-preview` does so with semver, not with cicada's config. Blessing a
sequence semver disagrees with would put the tool's opinion above the artifact's contract. Semver's ordering
is the truth; cicada's job is to refuse the edits that violate it, not to redefine them.

The practical consequence for anyone choosing stage names: **pick names that sort upward.** The conventional
`alpha` / `beta` / `rc` ladder works by a happy ASCII accident, and cicada reports the violation clearly when
a chosen one does not.

**One papercut worth knowing.** `prerelease_identifier` is ordinary config read from the branch, so it travels
through a merge like any other line. Promoting a line that reached `rc` merges `prerelease_identifier = "rc"`
onto `main` — inert there, since `main` matches no prerelease pattern (#82) — but the *next* line cut from
`main` inherits `rc` and starts there rather than at `alpha`. Reset it when cutting the line if the ladder is
meant to start from the bottom.

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

It is written to `target/cicada/release-manifest.json` (gitignored, **never committed**) *and* embedded in
**every release object the run creates**. Both, for two reasons: under `--no-push` there is no release object
at all, so the file is the only way to inspect a rehearsal (Goal 6); and the deploy job that runs after the
release job can read a file in the workspace without a token and an API round trip.

**Every release, not the umbrella's alone.** An earlier draft attached it to the umbrella, which quietly
assumed every repo has one — `[umbrella]` is optional (#73), and a two-package repo with no umbrella (an app
and its chart, §single-service shape 2) would have left the manifest with nowhere to go. Embedding it in each
release also answers the question a reader of any one release actually has — *what else shipped with this?* —
without them needing to know which release is the summary. The cost is a collapsed `<details>` block repeated
N times, which is cheap next to a rule with an exception in it.

This manifest is the artifact that is genuinely useful for deployment — feeding an umbrella Helm chart or a
compose bundle. An umbrella *version*, where one is declared, is a human-friendly label on it.

**Ordering within a run** (#96). Packages are released in config order and the umbrella last. Config order
because #31 already makes it the repo's one observable ordering, so the forge-call log is deterministic and
testable; umbrella last because its body embeds the manifest, and a summary created before the work it
summarises is a race waiting to be discovered.

---

## Updaters

A package owns a **list** of updaters. This collapses what were three separate concepts (`type`,
`extra_files`, `templates`) into one uniform mechanism, and extends to Go/Java/Python later by adding
`kind` values with **no schema change**.

| kind | Writes | Notes |
|---|---|---|
| `cargo` | `Cargo.toml` `version` | `toml_edit` preserves formatting and comments; `version.workspace = true` redirects to the workspace root (see #24) |
| `helm` | `Chart.yaml` `version`, `appVersion` | `appVersion` may `track` another package; `on_tracked_bump` sets the derived bump size |
| `generic` | one value in any file | In-place surgical edit; the file stays the user's. `edit` is a table with exactly one key — `{ toml = … }`, `{ yaml = … }`, `{ json = … }` or `{ marker = … }` — and is **required**, since naming a file and no value in it would be a silent no-op |
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

A derived release has **no commits of its own**, so its changelog entry is written from `derived_from` rather
than from a range (#97, §Three contexts).

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
cicada check [--base <ref>] [--head <ref>]
                                validate config; lint the commits in a range
cicada plan [--json]            what would be released, and why. touches nothing
cicada release-pr [--dry-run] [--no-push] [--prerelease <id>]
                                Phase A: compute, apply, push branch, upsert PR/MR
cicada release [--dry-run] [--no-push]
                                Phase B: reconcile tags, create releases
cicada run [--dry-run] [--no-push]
                                Phase B then Phase A, in that order. What CI should call
cicada template print <name>    emit a built-in template (notes, changelog) to stdout
```

Global: `--token`, `--forge`, `--api-url`, `--repo`, `--config`, `--date`, `--json`, `-v`, `--quiet`.
Colour follows `NO_COLOR` and TTY detection.

`--date YYYY-MM-DD` (#95) defaults to today in UTC and feeds exactly one thing: the date rendered into a
changelog entry and the release notes. It is a flag rather than a clock read inside the renderer for two
reasons. It keeps `cicada-core` pure — the date arrives as a value, like `LinkStyle` — and it makes a run
reproducible: pass the date a previous run used and the `Vec<FileEdit>` is byte-identical. Without it, a
`--dry-run` at 23:59 and the real run at 00:01 write different changelog entries, which would make #30's
"the same computation with the write skipped" quietly false once a day.

UTC rather than a configurable zone: a zone is one more input that changes the answer, and a changelog entry
dated by the release runner's locale is a worse artifact than one dated consistently.

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

The object carries the `date` the run used (#95), so a plan is reproducible from its own output.

**`from` is `null` on a first release** (#89), present rather than omitted. A package that has never been
released has no previous version, and each of the alternatives is worse in a specific way: `"0.0.0"` names a
release that never existed, so a consumer building `…/compare/v{from}...v{to}` emits a URL that 404s; and
omitting the key is ambiguous with an older cicada that did not emit it yet, which is exactly the ambiguity a
schema version (#69) exists to prevent. `null` says *there is no previous version* and nothing else.

### Offline and local rehearsal (#25)

Three levels of "don't touch the upstream", which matter both for dogfooding cicada in its own repo and
for `check` running on forked PRs where no token is available:

| | Writes files | Local commit/tag | Pushes | Forge API | Token |
|---|---|---|---|---|---|
| `plan`, `check` | — | — | — | — | not required |
| `--dry-run` | — | — | — | — | not required |
| `--no-push` | ✓ | ✓ | — | — | not required |
| default | ✓ | ✓ | ✓ | ✓ | required |

None of these levels reads a clock either — the date is an input at every one of them (#95), so a rehearsal
and the release it rehearses produce identical bytes rather than identical-looking ones.

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

**`check` lints a range and nothing else** (#87). Under squash-merge the PR *title* is what lands on the
default branch, so linting the title is the thing a squash-merge repo actually wants — and v1 does not do it.
It is deferred rather than half-specified (future-work §15): an earlier draft routed it through
`commit_source = "title"`, which is not a member of that setting's enum (`commits` | `merges` | `both`) and
would be meaningless for history reading, and a flag with no decided semantics is worse than an absent one.

---

## Forge abstraction

```rust
trait Forge {
    fn find_pull_request(&self, head: &str)          -> Result<Option<PullRequest>>;
    fn create_pull_request(&self, req: PrReq)        -> Result<PullRequest>;
    fn update_pull_request(&self, id: u64, r: PrReq) -> Result<PullRequest>;
    fn close_pull_request(&self, id: u64)            -> Result<()>;   // #34, empty plan
    fn set_draft(&self, pr: &PullRequest, on: bool)  -> Result<()>;   // #33, layer 2 — see #102
    fn comment(&self, id: u64, body: &str)           -> Result<()>;   // #33, staleness layer 2
    fn set_labels(&self, id: u64, labels: &[String]) -> Result<()>;
    fn find_release(&self, tag: &str)                -> Result<Option<Release>>;
    fn create_release(&self, req: ReleaseReq)        -> Result<Release>;
}
```

`set_draft` takes the whole pull request rather than its number, which is the one place the GitHub API's
shape reaches into the trait: drafting is a GraphQL mutation addressed by node id (#102), and a number is not
enough to name the thing being changed.

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

- **GitHub** — a real API field (`draft: true`), but **only at creation** (#102). `PATCH
  /repos/{o}/{r}/pulls/{n}` accepts `title`, `body`, `state`, `base` and `maintainer_can_modify`, and nothing
  else. Converting an *existing* PR to draft is the GraphQL mutation `convertPullRequestToDraft`, and coming
  back is `markPullRequestReadyForReview`. Layer 2 drafts a PR that already exists, so it is always the
  GraphQL path. Historically unavailable on private repos below certain plans.
- **GitLab** — there is no flag at all. Draft state *is* a `Draft:` prefix on the MR title, so the adapter
  prefixes and strips a string. Three consequences worth stating (#105): `PullRequest` has to carry its
  title, because setting the prefix means knowing what is being prefixed; the adapter **strips before it
  prefixes**, or repeated refusals stack `Draft: Draft: …` until the title is rejected for length; and the
  legacy `WIP: ` spelling is stripped too, because merge requests carrying it still exist. This is not a
  degradation — GitLab genuinely refuses to merge a draft MR, so layer 2 is as enforced there as on GitHub.

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
the manifest is embedded in **every** release body the run creates (#42), inside a collapsed `<details>`
block of fenced JSON, and written to `target/cicada/release-manifest.json` on disk. Identical behaviour on every forge, no asset API, and still
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
  creates** — a PAT or GitHub App token is required for CI to run on the release PR. On an auto-merging repo
  this is what makes the loop turn at all, not just a convenience (§No review gate).
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
| Notes template | once per package release | that package's commits, **pre-grouped into sections** | the above, plus `links`, `breaking`, `derived_from` |
| Umbrella changelog | once per run | every release in the run, grouped by package | the above, plus `releases` (each carrying its own `derived_from`) |

Section grouping and ordering stay **config-driven** (`[changelog] sections`). The template receives
sections already grouped and decides only how to render them — otherwise config and template could
contradict each other, and the config would silently lose.

**`derived_from` is how a release with no commits explains itself** (#97). A chart dragged by its tracked app
(#22) receives no commits at all, and a changelog holds only its own package's commits (#15) — so without this
the entry body renders empty. `derived_from` carries the package and version that caused the release:

```jinja
{% if derived_from and not commits %}
_Released to track {{ derived_from.package }} {{ derived_from.version }}._
{% endif %}
```

Three things about that choice. The entry is **written, not skipped** — the alternative leaves a hole in the
chart's changelog where a real tagged release was, and a reader looking up `0.9.1` finds nothing. It is
**data, not prose cicada emits**: core already labels the bump `derived` in the `Plan`, so this only surfaces
what exists, and the wording lives in a template that can be overridden (#47) like any other. And it is
**not an empty entry**, which would be honest and would read as a rendering bug — the same objection F8 makes
about a silently skipped file write.

A release can be both earned and derived at once (a chart with its own `feat` that its app also dragged). Then
`commits` is non-empty *and* `derived_from` is set: the default renders the sections normally and the
guard above suppresses the tracking line, since the entry already explains itself.

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

**But two updaters sharing a file is not automatically that bug** (#99). `FileEdit` carries *final content*,
so a naive implementation emits two edits for one path and the second silently wins — which is exactly the
harm above. The legitimate case is common, though: one `values.yaml` carrying `image.tag` and
`labels.version`, or two services stamping their own tag into one deploy file. So edits **fold** — each
updater transforms the content the run has for that path so far, not what is on disk, in config order — and
one `FileEdit` per path comes out at the end.

What genuinely cannot fold is a **whole-file** writer. A `template` updater renders the entire output and
would obliterate any surgical edit under it, config-order-dependently and without a word. That is the
collision the hard error is for, and `check` refuses it before any file is opened: a `template` `output`
sharing a path with *any* other updater, including another template.

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
| Dates | `jiff` | changelog dates, from the `--date` input rather than a clock read (#95); no `chrono` legacy surface |
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
- **Registry:** `ghcr.io/<org>/cicada` as primary, tagged `v1.2.3`, `v1.2`, `v1`, `latest`. The moving tags
  move for **stable releases only** — a prerelease publishes its exact tag and nothing else, or a pipeline
  pinned to `:v1` to avoid surprises gets handed an alpha and then silently gets taken off it again.
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
| **M1** | Anchor resolution (per-package ranges, tag reachability), scope routing incl. `default_scope`, bump computation (section `bump`, first-release table, `pre_major`, `maintenance` guard), umbrella aggregation, `Plan`. **done** | ✅ `cicada plan --json` correct on a fixture **monorepo and a single-package repo through the same code path**, fully offline |
| **M2** | Updaters (cargo, helm, generic path + marker, template), minijinja notes template with three contexts + filters + `template print`, changelog marker insert/create, versions-file write, `FileEdit` boundary. **done** | ✅ `release-pr --dry-run` prints exactly the diff it would write; duplicate-output and re-parse checks fire. 198 tests green |
| **M3** | Git write path: release PR branch reset/commit, committer identity, `release_branches` allowlist, templated `release_pr_branch`, `--no-push` end to end. **done** | ✅ **`release-pr --no-push` produces a real local release commit**, inspectable with `git show`, with the working tree untouched (#100). The #32 staleness refusal and the #80 collision check both fire. 216 tests green |
| **M4** | `Forge` trait (9 methods) + GitHub adapter, upsert by head branch, auth via `GIT_ASKPASS`, retries/error mapping, **staleness model** (committer guard, draft, comment, close on empty plan). **built, one half unverified** | Protocols asserted against a recording fake; fetch/push/lease against a real bare repo; askpass end to end. 251 tests green. **Not yet verified: real HTTP against github.com** — that needs a token and a repo, and is the remaining gate |
| **M5** | Phase B: tag reconciliation (reachability + collision), **independent release-object reconciliation**, stale-merge detection, release manifest (disk + embedded), `cicada run` ordering. **done**, and **prereleases with it** (#59–#64, #85, #93) | ✅ merge → tag → release; re-run is a no-op; a tag with no release object self-heals; a maintenance branch releases independently of `main`. 275 tests green. Verified end to end locally with `--no-push`; the forge half still rests on the fake (see M4) |
| **M6** | GitLab adapter (REST v4, draft-as-title-prefix, self-hosted `api_url`). **built, unverified against a live instance** | Nine methods against REST v4; project addressed by URL-encoded full path so subgroups need nothing special; `Draft:` prefix with strip-before-prefix (#105); prerelease marked in the release name (#54). Retry policy shared with GitHub rather than reimplemented. 302 tests green. **Live HTTP unverified**, exactly as for GitHub |
| **M7** | Shipped CI templates (Actions composite action, GitLab `include:`), docs incl. the CI-trigger footgun and a worked `cicada.toml` per repo shape. **done** | ✅ `README.md`, `action.yml`, `ci/gitlab-ci.yml`, and this repo's own workflows. Every config the README shows is parsed by the real parser in CI, and every shipped YAML asset by cicada's own reader — the adoption claim is mechanical, not asserted once. 314 tests green. The action's download step needs M8's published binaries |
| **M8** | musl static binaries (x86_64, aarch64), multi-arch Docker with `git`, self-release. **Artifact half built** — `.github/workflows/artifacts.yml`, `ci/Dockerfile` | cicada v0.1.0 released by cicada. Binaries, `SHA256SUMS` and the image are built and locally verified; the asset names, checksum format and image tags are held to `action.yml` and `ci/gitlab-ci.yml` by tests that execute the shell rather than restate it. **Remaining: the self-release itself**, which runs the GitHub adapter live and is therefore the same gate M4 left open |

**M0's unproven assumption has been discharged.** Decision #49 — span-preserving parse and byte splice — is
what every updater rests on. `spikes/yaml-splice` confirms it works, names the crate, and captures the two
traps (char-vs-byte indexing, and tolerating aliases outside the target) as regression tests. Everything
remaining in this plan is assembly of known parts.

**First genuinely useful build is M3** (offline planning and local release commits, usable on this repo).
**First end-to-end release tool is M5**, on GitHub.

---

## Open questions

None blocking. The design is complete enough to build M0 against.

Deferred scope lives in `docs/future-work.md` — seventeen items, each with why it was deferred and what would
trigger picking it up. Behavioural coverage lives in `docs/scenarios/` — one file per repo shape, written as
executable scenarios, which is where decisions #73–#79 came from. `single-service.md` covers shapes 1 and 2;
`monorepo.md` covers only what needs more than one package, and inherits the rest rather than repeating it.

The M0 spike is **complete and #49 holds** — see `spikes/yaml-splice`. **Nothing is open.** The shape-2
walkthrough's question — what goes in a derived release's changelog entry — is answered by #97: the entry is
written, and the notes context carries `derived_from` so the default template can name the cause.

The monorepo pass's two questions are decided — the manifest is embedded in every release the run creates
rather than the umbrella's alone (#42), and releases are created in config order with the umbrella last (#96).
The five items that stood here through the single-service pass are decided too, four of them by taking scope
out rather than adding a rule:

| Was open | Resolution |
|---|---|
| `check`'s title mode contradicted `commit_source`'s enum | `check` lints a range only; title linting deferred (#87, future-work §15) |
| `{summary}` in `release_pr_title` had no defined rendering | Defined and shipped: config order, truncated after three, no umbrella case (#86) |
| A derived chart bump on a never-released chart landed on `0.0.1` | `initial_version` is required on a `helm` package, so the case cannot arise (#88) |
| `from` on a first release in `--json` | `"from": null`, present and explicit (#89) |
| A promoted prerelease line keeps cutting rcs below the stable it shipped | Accepted: cicada acts on what the branch and the config say, and merging back is the operator's job (#91) |
| Two sub-questions on `Release-As:` | Dissolved — forced versions are out of v1 (#39, future-work §13) |

The scenario-4 pass added three more, and all three went the same way: **two dissolved with the scope cut**
(`Release-As` on a prerelease branch, and forced-version monotonicity against an open prerelease both
presuppose forced versions), and the third was a wording gap rather than a behaviour question — the staleness
guard's `prev` is now stated as stable-only (#90).

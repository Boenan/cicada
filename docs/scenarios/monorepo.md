# cicada — release scenarios: monorepos

Every outcome cicada can produce in a repo that ships several services from one tree. Companion to
`docs/design.md`; decision numbers (`#n`) reference it.

**Read `docs/scenarios/single-service.md` first.** This file does not repeat it. Anchor resolution, the
two-phase loop, staleness, the offline modes, updater mechanics and changelog insertion all behave identically
here — a monorepo is the same code path with a longer package list (#16), and asserting that twice would be
noise. What follows is only what *needs more than one package to exist*: scope routing, umbrella aggregation,
divergent version lines, and the failures that only appear at N > 1.

Group letters continue the single-service sequence (A–N) at **O**, so an id is unambiguous across both files.

---

## The shape

One tree, three Rust packages, one Helm chart, one umbrella.

```
services/api            api           Rust service
services/worker         worker        Rust service
libs/auth               auth          internal library, never published (future-work §1)
deploy/charts/api       api-chart     Helm chart, appVersion tracks `api`
                        platform      the umbrella — no directory of its own
```

```toml
[settings]
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]
default_scope        = "platform"                # the umbrella receives unscoped commits (#38)
committer            = "cicada[bot] <cicada@users.noreply.github.com>"
commit_source        = "commits"
release_pr_branch    = "cicada/{branch}"
release_pr_title     = "chore(release): prepare release"

[forge]
kind = "github"
repo = "acme/platform"

[umbrella]
name       = "platform"
tag_format = "platform-v{version}"
changelog  = "CHANGELOG.md"

[changelog]
sections = [
  { type = "feat",  title = "Features",      bump = "minor" },
  { type = "fix",   title = "Bug Fixes",     bump = "patch" },
  { type = "perf",  title = "Performance",   bump = "patch" },
  { type = "docs",  title = "Documentation", bump = "none"  },
  { type = "chore", title = "Chores",        bump = "none", hidden = true },
]

[packages.api]
path = "services/api"        ; tag_format = "api-v{version}"        ; changelog = "services/api/CHANGELOG.md"
[[packages.api.updaters]]
kind = "cargo"               ; path = "services/api/Cargo.toml"

[packages.worker]
path = "services/worker"     ; tag_format = "worker-v{version}"     ; changelog = "services/worker/CHANGELOG.md"
[[packages.worker.updaters]]
kind = "cargo"               ; path = "services/worker/Cargo.toml"

[packages.auth]
path = "libs/auth"           ; tag_format = "auth-v{version}"       ; changelog = "libs/auth/CHANGELOG.md"
[[packages.auth.updaters]]
kind = "cargo"               ; path = "libs/auth/Cargo.toml"

[packages.api-chart]
path            = "deploy/charts/api"
tag_format      = "api-chart-v{version}"
changelog       = "deploy/charts/api/CHANGELOG.md"
initial_version = "0.1.0"                        # required on a helm package (#88)
[[packages.api-chart.updaters]]
kind            = "helm"
path            = "deploy/charts/api/Chart.yaml"
app_version     = { track = "api" }
on_tracked_bump = "patch"
```

*(`;` separates keys onto one line for readability here only; real TOML puts them on their own lines.)*

**Established state** for every scenario unless stated otherwise — `.cicada-versions.toml`:

```toml
[umbrella]
version = "1.7.0"

[packages.api]
version = "1.4.2"
[packages.worker]
version = "0.9.1"
[packages.api-chart]
version = "0.9.0"
# `auth` absent — never released (#4)
```

Tags `platform-v1.7.0`, `api-v1.4.2`, `worker-v0.9.1`, `api-chart-v0.9.0` all exist and are reachable from
HEAD. HEAD is on `main`; the command is `cicada run`.

**Note what the version numbers do *not* mean.** The umbrella is at `1.7.0` while `api` is at `1.4.2` and
`worker` at `0.9.1`. These are four independent lines that happen to live in one tree; no arithmetic relates
them, and the umbrella's number is a label on the set, not a function of the children's numbers (#10).

---

## O · Scope routing and attribution

Attribution is by conventional-commit scope, and nothing else (#11). No path heuristics, no dependency graph
— the author declares blast radius by what they type.

| id | commits in range | expected plan |
|---|---|---|
| O1 | `feat(api): add caching` | `api 1.4.2→1.5.0` · `api-chart 0.9.0→0.9.1` (derived) · `platform 1.7.0→1.8.0` |
| O2 | `feat(api,worker): shared retry backoff` | `api →1.5.0` · `worker →0.10.0` · `api-chart →0.9.1` · `platform →1.8.0` |
| O3 | `fix: correct the deploy script` (unscoped) | `platform 1.7.0→1.7.1` **only** — no child releases |
| O7 | `fix(platform): correct the deploy script` | identical to O3 — naming the umbrella explicitly is the same commit |
| O8 | `chore(api): bump serde` | **no release** — `bump = "none"` applies per package, not just globally (#36) |
| O9 | `feat(auth): add token rotation` | `auth →0.1.0` (first release, #37) · `platform →1.8.0` · **api and worker untouched** |

**O1 in full** — the reference scenario:

- **given** the established state above
- **when** `c1 feat(api): add caching`, then `cicada release-pr --no-push`
- **then**
  - plan · four entries in **config order** (#31): `api 1.4.2→1.5.0 minor` · `worker` **absent** ·
    `auth` **absent** · `api-chart 0.9.0→0.9.1 patch reason=derived` · `platform 1.7.0→1.8.0 minor
    reason=aggregate`
  - files · `services/api/Cargo.toml` → `1.5.0` · `deploy/charts/api/Chart.yaml` `version` → `0.9.1` and
    `appVersion` → `"1.5.0"` quoted (#51) · `services/api/CHANGELOG.md` gains a `## 1.5.0` entry ·
    `deploy/charts/api/CHANGELOG.md` gains `## 0.9.1` · root `CHANGELOG.md` gains `## 1.8.0` grouped by
    service · versions file updates `api`, `api-chart`, `umbrella`
  - **byte-identical** · `services/worker/Cargo.toml`, `services/worker/CHANGELOG.md`, `libs/auth/**` — a
    package that does not release is not touched, and that is asserted rather than assumed
  - git · `cicada/main` with exactly one commit
  - exit · `0`

**O2 · multi-scope is the whole blast-radius mechanism.** `feat(api,worker):` releases **both** — this is why
dependency-graph propagation is a permanent non-goal (§Non-goals): the author knows whether a shared change
breaks its consumers and a graph does not. Assert the commit appears in *both* per-service changelogs, and see
R3 for what it does to the umbrella's.

**O4 · unknown scope is fatal (#11).** `feat(aip): tune pool size` → `plan` and `release-pr` hard-error, exit
`1`; `check` reports it at exit `3` with *did you mean 'api'?*. Both name the full configured package list —
which in a monorepo is the actually useful part of the message.

**O5 · one bad scope fails the whole run.** `feat(api,wrker): …` — `api` is valid, `wrker` is not. Assert the
run refuses **entirely**: no plan, no partial release of `api`, no release PR branch. Releasing the half that
parsed would ship a version whose changelog is missing the change that motivated it.

**O6 · config order is the output order (#31).** Packages are declared `api`, `worker`, `auth`, `api-chart`.
Assert the plan, the `--json` `releases` array, the release-manifest entries and the umbrella changelog's
service groups all follow that order — **not** alphabetical (`api`, `api-chart`, `auth`, `worker`), and not a
hash order that varies per process. Run the same fixture twice in one process and across two processes;
assert byte-identical output. This is the assertion that makes Goal 7 observable, and a monorepo is the only
place it can fail.

---

## P · Independent version lines

Every package has its own anchor, so every package has its own range (#2, §Anchor resolution). This is what
makes packages release at different times without any coordination.

**P1 · one `git log` per package, with different anchors.**

```
c1  feat(api): add caching            ← tag api-v1.5.0 cut here
c2  feat(worker): parallel drain
c3  fix(api): handle nil

api's range        api-v1.5.0..HEAD   → c3
worker's range     worker-v0.9.1..HEAD → c2
auth's range       (unanchored — never released) → all reachable history
```

Assert exactly one `git log` invocation per package with the per-package anchor, and that `auth`'s range is
unanchored rather than starting at some other package's tag.

**P2 · timing divergence accumulates.** After O1 releases `api` at `1.5.0`, `worker` is still anchored at
`worker-v0.9.1`. A later `feat(worker):` produces a worker range that **spans the api release commit** — which
is correct and must not be filtered. Assert worker's changelog entry contains only `worker`-scoped commits
from that range, not everything in it: the range decides *what is read*, the scope decides *what is attributed*.

**P3 · a never-released package among released ones (#4).** `auth` has no versions-file entry. `feat(auth):`
gives `0.1.0` from the first-release table, computed over *all reachable history* (A8's behaviour, per package).
Assert the other three packages' anchors are unaffected and that Phase B skipped `auth` entirely on the prior
run rather than erroring on a missing `auth-v…` tag (A7 at N > 1).

**P4 · a missing anchor names its package.** `worker-v0.9.1` is deleted. The hard error (J1) must name
**`worker`** and the tag, not fail anonymously — in a four-package repo "tag not found" without attribution is
a scavenger hunt. Assert no other package is planned or released in that run.

**P5 · one package can be tag-only.** Give `auth` `updaters = []`. It still gets a version, a tag, a changelog
and a release object; no file under `libs/auth` is written except its `CHANGELOG.md` (K9 at N > 1).

---

## Q · Umbrella aggregation

The umbrella aggregates the **bump kind**, never the version number (#10). `api` going `1.4.2 → 2.0.0` makes
the umbrella take *a major bump on its own line*, `1.7.0 → 2.0.0` — which coincides here only because both
lines happened to be below 2.

| id | commits | children | umbrella |
|---|---|---|---|
| Q1 | `feat(api)!: drop v1 endpoints` | `api 1.4.2→2.0.0` | `1.7.0→2.0.0` (major) |
| Q2 | `feat(api):` | `api →1.5.0` | `1.7.0→1.8.0` (minor) |
| Q3 | `feat(api):` + `fix(worker):` | `api →1.5.0` · `worker →0.9.2` | `1.7.0→1.8.0` — highest child bump wins |
| Q4 | `fix:` unscoped | none | `1.7.0→1.7.1` — the umbrella releases alone (O3) |
| Q5 | `fix(worker):` + `fix:` unscoped | `worker →0.9.2` | `1.7.0→1.7.1` — patch from two independent patch sources |
| Q6 | `chore(api):` only | none | **no release at all** — an empty plan, not an empty umbrella release |

**Q1 detail — aggregation is not propagation, and the fixture hides it.** Here the umbrella lands on `2.0.0`
and so does `api`, so the two readings — *take a major bump* and *copy the child's number* — agree. Assert the
case that separates them: set the umbrella to `0.4.0`, leave `api` at `1.4.2`, and land the same `feat(api)!:`.

- `api 1.4.2 → 2.0.0` — a major bump on its own line
- `platform 0.4.0 → 0.5.0` — a **major** bump applied to the umbrella's line, which under the default
  `pre_major = "minor"` does not escape `0.x` (#37)

Assert `0.5.0`. "The umbrella copies the child" would give `2.0.0` and looks right in every other scenario in
this file, so this is the row that has to exist.

**Q7 · a derived chart bump never feeds the aggregate.** The sharpest form of #22 meeting #10 — set
`on_tracked_bump = "minor"` and land a single `fix(api): handle nil`:

- `api 1.4.2 → 1.4.3` — patch, earned
- `api-chart 0.9.0 → 0.10.0` — **minor**, derived from the tracked app releasing
- `platform 1.7.0 → 1.7.1` — **patch**

Assert the umbrella is `1.7.1` and **not** `1.8.0`. If the chart's derived minor fed the aggregate, one
`fix:` in one service would announce a minor release of the whole platform. The derived bump is a consequence
of `api`'s patch, which is already counted; counting it again inflates the umbrella.

**Q8 · the chart's own bump *does* feed it.** Same config, but `feat(api-chart): add a PodDisruptionBudget`
alongside the `fix(api):`. The chart's earned minor is a real child bump, so `platform → 1.8.0`. Assert the
pair Q7/Q8 together — they differ only in whether the chart's bump was earned or derived, and nothing else in
the repo changes.

**Q9 · the maintenance guard covers the umbrella.** On a branch matching `maintenance_branches`, a
`feat(api):` computes `api` minor **and** `platform` minor. Assert the run refuses at exit `4` (#28) naming the
offending commit, and assert it names `api` as the package whose bump is above patch — the umbrella's minor is
a consequence, and reporting it as the cause would send the reader to the wrong place.

**Q10 · the umbrella has its own anchor and range.** `platform-v1.7.0`. Assert it resolves and is used for the
umbrella's own unscoped-commit range, independently of every child's anchor (P1 applies to it too).

---

## R · Changelogs at N > 1

The umbrella's changelog groups by service; each service's contains only its own (#15).

**R1 · the root entry is grouped by service.** After O2 (`feat(api,worker):` plus a `fix(worker):`), root
`CHANGELOG.md` gains one `## 1.8.0` entry containing per-service groups in config order (#31):

```markdown
## 1.8.0 (2026-09-15)

### api

#### Features
* shared retry backoff (a1b2c3d)

### worker

#### Features
* shared retry backoff (a1b2c3d)

#### Bug Fixes
* drain on shutdown (e4f5a6b)
```

**R2 · per-service changelogs are narrow.** `services/api/CHANGELOG.md` contains the `feat(api,worker):` line
and nothing about `worker`'s `fix:`. Assert `services/worker/CHANGELOG.md` byte-identical except for its own
two entries, and `libs/auth/CHANGELOG.md` byte-identical entirely.

**R3 · a multi-scope commit appears under every scope it names.** In R1 above, `shared retry backoff` appears
**twice** in the umbrella entry — once under `api`, once under `worker` — and once in each service's own
changelog. Asserted explicitly because it reads like duplication and is not: the commit genuinely changed both
services, and a reader scanning the `worker` section must not have to know that the line is filed under `api`
somewhere above. Grouping is by service, so a commit with two services appears in two groups.

**R4 · an unscoped commit appears only in the umbrella's.** O3's `fix:` produces a root `## 1.7.1` entry with
no service grouping to speak of, and touches no service changelog.

**R5 · a package that did not release has no entry and no file write.** Assert byte-identical, not "no new
entry" — a trailing-newline change is a diff in a release PR someone has to review.

**R6 · the umbrella template context differs from a package's** (#46, §Three contexts). The umbrella changelog
renders **once per run** with `releases` holding every release in the run; a file or notes template renders
once per package release with only that package's commits. Assert a template using `releases` fails to render
in a per-package context rather than silently producing an empty list.

**R7 · one notes template, many destinations** (#43). Assert the body embedded in `services/api/CHANGELOG.md`
and the body sent to `create_release` for `api-v1.5.0` derive from the same render, and that the same holds
independently for `worker` — one template, N packages, no drift between a package's changelog and its release.

---

## S · Tags, releases, and the manifest

**S1 · one run, several tags.** After O1, Phase B creates `api-v1.5.0`, `api-chart-v0.9.1` and
`platform-v1.8.0`; no `worker-v…` or `auth-v…` tag is created. Assert three tag pushes and three
`create_release` calls, and that each release's notes are that package's own (R7).

**S2 · one manifest per run, embedded in every release it creates** (#42). Written to
`target/cicada/release-manifest.json` (gitignored, never committed) with an entry per released package —
package, version, tag, tagged commit, timestamp — and embedded in the body of **each** of the three releases,
inside a collapsed `<details>` block (#57). Assert `worker` and `auth` are **absent** from it: the manifest
describes the set that shipped together, which is what a deploy consumer needs, not the repo's inventory.
Assert also that all three embedded copies are byte-identical — they are one artifact, not three renders.

**S2b · release creation order is config order, umbrella last** (#96). Assert the recording fake sees
`create_release` for `api-v1.5.0`, then `api-chart-v0.9.1`, then `platform-v1.8.0` — never alphabetical, never
varying between runs. The umbrella is last because its body embeds the manifest describing the other two;
creating it first would embed a summary of work not yet done.

**S3 · partial reconciliation is per package and per artifact** (#56 generalised). `api-v1.5.0` was tagged and
released; `platform-v1.8.0` was tagged but its `create_release` 500'd; `api-chart-v0.9.1` was never tagged.
The next run creates **only** the missing chart tag, the missing chart release, and the missing umbrella
release. Assert zero re-tagging and zero duplicate releases. This is the scenario that justifies reconciling
tag and release object independently — at N packages there are 2N chances to fail halfway.

**S4 · re-running is a total no-op.** Assert zero writes, zero tag pushes and zero mutating forge calls across
all four packages; read calls are permitted (C3 at N > 1).

**S5 · a collision in one package stops the whole run** (#9, #80). `worker` computes `0.10.0` and
`worker-v0.10.0` exists unreachably. Assert exit `4`, and assert that `api` — whose own version is
uncontested — is **not** released. A run is atomic in intent even though it produces N artifacts: shipping
three of four packages and erroring on the fourth leaves a release manifest that describes a set that never
existed.

---

## T · A monorepo with no umbrella

`[umbrella]` is optional (#73). A repo can ship four services and deliberately have no aggregate version —
there is nothing for one to mean if the services are not released or deployed together.

```toml
default_scope = "api"      # must name a real package when no umbrella is declared (#73)
```

**T1 · unscoped commits land on the named package.** `fix: handle nil` → `api` patch. Assert no aggregate
version is computed, no root `CHANGELOG.md` is written, and no `platform-v…` tag exists.

**T2 · `check` requires `default_scope` to resolve.** With no umbrella and `default_scope = "platform"`,
exit `3` naming the valid targets (I6 at N > 1).

**T3 · aggregation is absent, not empty.** Assert the plan has no umbrella entry at all rather than an entry
with a null version, and that `--json`'s `releases` array contains only real package releases.

**T4 · the manifest still has a home** (#42). With no umbrella there is no "umbrella's release" to attach to,
and the manifest is embedded in every release the run creates — so a two-package no-umbrella repo gets it in
both. Assert this explicitly: it is the case that made the old "attach it to the umbrella" rule unsatisfiable,
and it is reachable from single-service shape 2 as well.

**T5 · an app and its chart are two packages, never an umbrella and a child** (#74). Even in a monorepo,
`api` and `api-chart` relate through `app_version.track` and `on_tracked_bump` only. Assert the chart is not
treated as a child of `api` for aggregation purposes and that removing `[umbrella]` changes nothing about
their relationship.

---

## U · The Cargo workspace-inheritance trap

The one static-analysis failure that only exists in a workspace (#24), and the reason `check` validates config
against the tree rather than in isolation.

```toml
# Cargo.toml (workspace root)
[workspace.package]
version = "1.4.2"

# services/api/Cargo.toml
[package]
version.workspace = true

# services/worker/Cargo.toml
[package]
version.workspace = true
```

**U1 · two packages resolving to one version field is a hard error.** `api` and `worker` both inherit, so both
`cargo` updaters address `[workspace.package] version` in the root manifest. They cannot be bumped
independently: releasing `api` at `1.5.0` would silently set `worker` to `1.5.0` as well. `check` reports it at
exit `3`, `plan` and `release-pr` hard-error at exit `1`. Assert the message names **both packages and the
shared field**, since naming only one sends the reader to a manifest that looks fine.

**U2 · one inheriting package is legal.** Only `api` inherits; `worker` and `auth` carry explicit versions. The
field resolves uniquely, so the arrangement is allowed. Assert the `cargo` updater writes
`[workspace.package] version` in the **root** `Cargo.toml` and leaves `services/api/Cargo.toml` byte-identical
— the version it manages is not in the file the updater's `path` points at, which is the surprising half.

**U3 · this is a `check` finding, not a runtime discovery.** Assert `check` reports it with no commit range at
all (`cicada check` on a clean tree), because an unreleasable *arrangement* is a property of the config plus
the tree, independent of what has landed (I4–I6 at N > 1).

**U4 · intra-workspace dependency requirements are not rewritten.** `services/api/Cargo.toml` declares
`auth = { version = "0.3", path = "../../libs/auth" }`. When `auth` bumps to `0.4.0`, assert that `version =
"0.3"` is **unchanged**. Harmless for a private workspace where `path` wins; fatal for `cargo publish`, which is
out of scope (future-work §1). Asserted so the omission is visible rather than discovered at publish time.

---

## V · Updaters across packages

**V1 · two updaters declaring the same output is a hard error** (#44). `packages.api` and `packages.worker`
both declare a `template` writing `deploy/shared.yaml`. `check` exits `3` naming both; `plan` exits `1`.
Last-writer-wins would be a silent, config-order-dependent bug — and in a monorepo config order is exactly
what nobody reads.

**V2 · template and output paths resolve from the repo root** (#44), never from the package's `path`. Assert
`packages.worker`'s `output = "deploy/worker/deployment.yaml"` writes there and **not** to
`services/worker/deploy/worker/deployment.yaml`. Packages already declare their own paths; repo-root
resolution is what removes the ambiguity.

**V3 · a re-parse failure in one package aborts the entire run** (#52). `worker`'s YAML splice produces an
unparseable file. Assert the working tree is clean, **no** package's edits were committed — including `api`'s,
which were computed successfully — and no release PR branch commit exists. The abort must precede the commit,
and the commit is one commit for the whole run.

**V4 · updaters are per package and independent.** Assert `api`'s `cargo` updater, `api-chart`'s `helm`
updater and `auth`'s `cargo` updater each run only for their own package's release, and that a package not
releasing runs none.

---

## W · Prereleases in a monorepo

Prerelease is **branch-level, never per-package** (#59), and a monorepo is where that decision earns itself.

**W1 · the whole run is a prerelease.** On a branch matching `prerelease_branches` with
`prerelease_identifier = "rc"`, O1's commit produces `api 1.5.0-rc.1`, `api-chart 0.9.1-rc.1` and
`platform 1.8.0-rc.1`. Assert every package releasing in that run carries the identifier, that packages not
releasing get nothing, and that **no `CHANGELOG.md` is written for any of them** (#62) — including the root
one.

**W2 · this is why it is not per-package** (#59). Construct the rejected alternative and assert cicada has no
way to express it: `api` on `1.5.0-rc.1` while `worker` releases a stable `0.10.0` in the same run. The
umbrella would then have to aggregate a prerelease child and a stable child into one number, and there is no
coherent answer — is `platform` `1.8.0` or `1.8.0-rc.1`? Branch-level makes the question unaskable.

**W3 · each package keeps its own counter.** The versions file grows a `prerelease` key per package, and the
counters are independent: `api` may be at `1.5.0-rc.3` while `api-chart` is at `0.9.1-rc.1`, because the chart
only started releasing at rc.3's run. Assert both, and assert each counter follows #63 against **its own** base.

**W4 · a staged line stages every package at once** (#93). Editing `prerelease_identifier` from `alpha` to
`beta` resets the counter for **every** package releasing on that branch, each to `.1` against its own base.
Assert no package carries its `alpha` counter forward.

**W5 · promotion writes one entry per package plus the umbrella.** Merging the line into `main` produces
stable `api 1.5.0`, `api-chart 0.9.1`, `platform 1.8.0`, each with one cumulative changelog entry since its own
last **stable** anchor (#61) — and the anchors differ per package, so the ranges differ. Assert no `-rc.` string
appears in any changelog, and assert the umbrella's entry covers every service that moved during the line.

---

## X · Maintenance branches in a monorepo

All of cicada's state lives inside the branch, so a maintenance line in a monorepo needs no setup beyond
cutting it (#82) — but it releases a *subset*, which is the new behaviour.

**X1 · a maintenance line releases only what changed on it.** `release/1.4` cut from `platform-v1.4.9`; a
`fix(api):` cherry-picked. Assert `api` and `platform` release on that branch, `worker` and `auth` do not, and
`main` is untouched — its versions file, changelogs and tags unchanged.

**X2 · the versions file diverges per branch, per package.** `main` carries `api = 2.0.3`, `release/1.4` carries
`api = 1.4.8`, and each branch's anchor resolves within itself. Assert four independent divergences in one
repo with no coordination mechanism (G1 at N > 1).

**X3 · the guard refuses the run, not the package.** A cherry-pick brings `feat(worker):` onto `release/1.4`
alongside a legitimate `fix(api):`. Assert exit `4` with **nothing released** — not `api` released and `worker`
refused. Partial release under a guard failure would ship a manifest describing a set the operator never
approved (S5's rule applied to the guard).

**X4 · `check` catches it before merge.** `cicada check --base release/1.4 --head <pr-head>` reports the same
finding at exit `3`, naming the commit and the package (G2c at N > 1).

---

## Y · `check` in a monorepo

`check` earns most of its value here — at one package almost nothing it validates can be wrong.

| id | config or commits | exit | output |
|---|---|---|---|
| Y1 | `feat(aip):` | `3` | *unknown scope 'aip'* + *did you mean 'api'?* — `strsim` over four names |
| Y2 | `feat(wroker):` | `3` | *did you mean 'worker'?* — assert the suggestion is not `api-chart` |
| Y3 | `packages.api` and `packages.worker` share `path = "services/api"` | `3` | names both |
| Y4 | `api` and `worker` share `tag_format = "svc-v{version}"` | `3` | two packages producing one tag is a collision waiting to happen (#9) |
| Y5 | `app_version = { track = "apii" }` | `3` | names the valid package list |
| Y6 | `packages.api-chart` omits `initial_version` | `3` | required on a `helm` package (#88) |
| Y7 | `default_scope = "platfrom"` with an umbrella declared | `3` | names the umbrella and every package |
| Y8 | two updaters sharing an `output` (V1) | `3` | names both |
| Y9 | `version.workspace = true` on two packages (U1) | `3` | names both and the shared field |
| Y10 | a package `path` that does not exist on disk | `3` | names the path and the package |

**Y2 detail.** With four package names the suggestion must be *useful*, not merely present. Assert the
Levenshtein result picks `worker` for `wroker` and that a scope with no near match (`feat(database):`) reports
the unknown scope and the full package list **without** a misleading suggestion.

**Y11 · `check` is still offline and tokenless** (#25, Goal 6). Assert zero HTTP across all of the above — the
harness fails the run if an HTTP client is constructed. A monorepo changes the volume of validation, not its
dependencies.

---

## Deltas this exercise found for `docs/design.md`

Two, both surfaced by tracing a multi-package run rather than by reading a section — and both invisible at one
package, which is why the single-service pass did not find them.

| # | Delta | Status |
|---|---|---|
| 20 | **Where does the release manifest attach when no umbrella is declared?** #42 wrote it to disk *and* attached it to the umbrella's release, but #73 makes `[umbrella]` optional — and single-service shape 2 (M1) already has two packages and no umbrella, so that half of the rule had no subject | **#42**, amended — embedded in **every** release the run creates. Each release then answers *what else shipped with this?* without the reader needing to know which one is the summary, and the rule has no exception in it (S2, T4) |
| 21 | **In what order are a run's releases created?** Unspecified, and observable twice: these scenarios assert an ordered list of `Forge` calls against a recording fake, and a partial failure (S3) recovers differently depending on what was reached | **#96** — packages in config order (#31), umbrella last, so its embedded manifest describes work already done (S2b) |

Both were invisible at one package, which is why the single-service pass did not find them, and both are
observable in the forge-call log M5's tests assert against.

---

## Coverage against the design

Exercised here and **only** here: umbrella aggregation (#10, #15) including the derived-bump exclusion ·
multi-scope commits (#11) · scope routing across four packages · `default_scope` naming an umbrella (#38) ·
per-package anchors and ranges at scale (#2) · release timing divergence · config-order determinism (#31) ·
one manifest per run (#42) · per-package, per-artifact reconciliation (#56) · the Cargo workspace-inheritance
trap (#24) · duplicate updater outputs across packages (#44) · a monorepo with no umbrella (#73, #74) ·
branch-level prerelease coherence (#59) · the maintenance guard over an umbrella (#28) · `check`'s
did-you-mean at N names (#12).

Inherited from `docs/scenarios/single-service.md` and deliberately not repeated: anchor resolution mechanics ·
the two-phase loop and its ordering (#35) · staleness, all three layers (#32–34) · the release-PR lifecycle ·
tag reconciliation and collision detection (#9, #80) · updater mechanics and the YAML splice traps (#49, #71,
#72) · changelog insertion (#5) · template overrides and filters (#45, #47, #48) · offline modes (#25) · exit
codes (#68) · the absence of forced versions (#39) and of a title mode (#87).

**Not exercised anywhere yet:** the GitLab adapter's behavioural differences beyond its two asserted
fallbacks, and forge retry/backoff (#58).

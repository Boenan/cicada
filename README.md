# cicada

Release automation driven by conventional commits: versions, changelogs, tags and releases.
In the spirit of [release-please](https://github.com/googleapis/release-please), but **git is the substrate
and the forge API is a thin edge** — which is what makes it work on GitHub and GitLab from one code path,
and what lets you rehearse a release on your laptop with no token and no network.

```
push to main  →  cicada opens a release PR  →  you merge it  →  cicada tags and releases
```

| | release-please | cicada |
|---|---|---|
| History source | GitHub GraphQL API | local git |
| Commit creation | GitHub REST API | `git commit` + `git push` |
| Release detection | PR labels | tag reconciliation |
| Forges | GitHub only | GitHub, GitLab |
| Offline rehearsal | — | `plan`, `check`, `--dry-run`, `--no-push` |

---

## Installing

**On GitHub Actions, you do not install it at all** — `uses: boenan/cicada@v1` downloads and verifies the
binary for you. Same on GitLab: the shipped template runs the image. The rest of this section is for
everywhere else.

**A released binary.** Static musl builds, attached to every release with a `SHA256SUMS` beside them:

```bash
base=https://github.com/boenan/cicada/releases/latest/download
asset=cicada-x86_64-unknown-linux-musl.tar.gz    # or cicada-aarch64-unknown-linux-musl.tar.gz

curl -sSfLO "$base/$asset"
curl -sSfLO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar xzf "$asset"
install -m755 cicada /usr/local/bin/cicada
```

The archive holds a single `cicada`, already executable. That is why it is an archive: a GitHub
release asset carries no permissions, so a bare binary downloads without its executable bit and has
to be `chmod`-ed before it will run.

`--ignore-missing` because `SHA256SUMS` lists every architecture and you downloaded one. It still fails if
the file you did download is not in there, so it is a check rather than a formality.

**The image.** It carries `git`, which cicada requires (#17) and will not start without:

```bash
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/repo" -w /repo ghcr.io/boenan/cicada:v1 cicada plan
```

The `-u` is not optional in that line. Without it the container is root, your checkout is not, and git
refuses to operate on a repository it considers dubiously owned. The fix that suggests itself — baking
`safe.directory` into the image — does **not** work: cicada runs git with `GIT_CONFIG_NOSYSTEM=1`, so the
image's system config is not read at all.

Tags are `v1.2.3` exactly, plus `v1.2`, `v1` and `latest` moving with each stable release. **A prerelease
publishes only its exact tag**, so `:v1` never quietly resolves to an alpha.

**Or `cargo install cicada`**, which works anywhere Rust does — including the platforms the binaries and
images do not cover.

---

## Quick start

**1. Write `cicada.toml` at the repo root.** There is no `cicada init` — the package list is yours to own,
and a scanner's guesses need reviewing line by line anyway. For a single-service Rust repo:

```toml
[settings]
release_branches = ["main"]     # the gate: cicada acts only on these branches
default_scope    = "app"        # unscoped commits belong to this package

[forge]
kind = "github"                 # "github" | "gitlab" — never guessed
repo = "acme/app"               # optional; derived from `origin` when absent

[packages.app]
path       = "."
tag_format = "v{version}"

[[packages.app.updaters]]
kind = "cargo"
path = "Cargo.toml"
```

**2. Check it.** No token, no network:

```console
$ cicada check
✓ config ok
```

**3. Rehearse.** Still nothing written:

```console
$ cicada plan
  app  1.4.2 → 1.5.0  minor
       feat: add export
       fix: handle nil

  1 release
```

**4. See the exact bytes**, and then a real local commit you can inspect with `git show`:

```console
$ cicada release-pr --dry-run     # prints the diff it would write
$ cicada release-pr --no-push     # writes a real commit on cicada/main; pushes nothing
$ git show cicada/main
```

**5. Wire up CI** (below), and cicada does the rest.

---

## ⚠ The CI-trigger footgun

**Read this before anything else fails mysteriously.**

On GitHub, the default `GITHUB_TOKEN` **does not trigger workflows on pull requests it creates**. On GitLab,
`CI_JOB_TOKEN` has the same problem for merge requests.

The failure is silent and looks like success: cicada opens the release PR, the PR's checks never start, and
it sits open looking perfectly healthy. If you have enabled auto-merge, the merge is gated on checks that
will never run, so **the loop never turns at all**.

Use a **PAT or GitHub App token** (GitHub) or a **Project Access Token** (GitLab). This is not a convenience.

**And on GitHub, set `persist-credentials: false` on `actions/checkout`.** Without it the checkout writes
`GITHUB_TOKEN` into `.git/config` as an `http.<url>.extraheader`, git matches that against cicada's push URL,
and the push authenticates as `GITHUB_TOKEN` however correct your PAT is. The push *succeeds*, so nothing
looks wrong — but the tag it pushed was created by `GITHUB_TOKEN`, so no workflow reacts to it, and any
release-artifact job you have never runs. cicada wants none of those credentials: it derives its own push URL
and answers git through `GIT_ASKPASS`.

---

## CI

### GitHub

```yaml
name: release
on:
  push:
    branches: [main]

concurrency: cicada-release          # two overlapping runs race on the force-push

permissions:
  contents: write
  pull-requests: write
  issues: write

jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0             # full history, or the changelog is incomplete
          persist-credentials: false # or GITHUB_TOKEN hijacks cicada's push — see below
      - uses: boenan/cicada@v1
        with:
          token: ${{ secrets.CICADA_TOKEN }}   # see the footgun above
```

You also need **Settings → Actions → Allow GitHub Actions to create and approve pull requests**.

### GitLab

```yaml
include:
  - remote: https://raw.githubusercontent.com/boenan/cicada/v1/ci/gitlab-ci.yml
```

That ships `cicada:release` (ready to use) and `cicada:check` (lints a merge request's commits), plus a
hidden `.cicada` job to extend. `GIT_DEPTH: 0` and `resource_group` are baked in. Set `CICADA_TOKEN` to a
Project Access Token with **Maintainer** role and the `api` + `write_repository` scopes.

### Building artifacts

cicada does **not** build your binaries or images. It owns versioning, changelogs, tags and release objects;
your artifacts are built by a **separate workflow triggered by the tag cicada pushes**. Keeping them apart
means an artifact build can fail and be retried without touching version state.

---

## How it works

Two phases, and `cicada run` does both **in the right order** — which is why you should call `run` rather
than wiring the two yourself.

**Phase A — `cicada release-pr`.** Reads `anchor..HEAD` per package, routes commits by conventional-commit
scope, computes versions, writes version files and changelogs onto a branch named `cicada/{branch}`, and
opens a pull request. The branch is **replaced** every run, never updated — it holds exactly one commit that
is a pure function of your current tip, so there is never a conflict to resolve.

**Phase B — `cicada release`.** Stateless reconciliation: for each recorded version, does its tag exist, and
does a release object exist for it? Create whatever is missing. Nothing is read back from the forge to
decide anything, so it is immune to squash-merge, rebase-merge and rewritten commit messages, and re-running
is a no-op.

**Phase B runs first.** Both fire on the same push, and A-before-B would compute its range from the *old*
tag, re-include everything the just-merged release contained, and propose releasing it again — every cycle.

### Two files

| File | Owned by | Holds |
|---|---|---|
| `cicada.toml` | you | configuration |
| `.cicada-versions.toml` | cicada | a version per package, and nothing else |

`.cicada-versions.toml` is the **sole source of truth** for versions. `Cargo.toml`, `Chart.yaml` and friends
are *outputs* — cicada never reads a version back out of them. The visible consequence: a first release
computed as `0.0.1` rewrites a `cargo new` manifest from `0.1.0` **downwards**, and that is correct.

### The release manifest

Every run that releases anything writes `target/cicada/release-manifest.json` — the set of things that
shipped **together**, which is what a deploy step actually needs:

```json
{
  "schema": 1,
  "date": "2026-09-21",
  "releases": [
    { "package": "api", "version": "1.5.0", "tag": "api-v1.5.0", "sha": "…" }
  ]
}
```

Add `target/cicada/` to `.gitignore` if `target/` is not already there — it describes a run, not a state of
the repository, and is never committed.

The same JSON is embedded in **every** release object the run creates, inside a collapsed `<details>` block,
so a reader of any one release can answer *what else shipped with this?* without knowing which release is
the summary. The file on disk is the only way to inspect a `--no-push` rehearsal, and it lets the deploy job
read the result without a token and an API round trip.

### The tag is the anchor

There is no stored commit SHA anywhere. The recorded version becomes a tag name, that tag is resolved, and it
must be **reachable from HEAD**. A missing tag is a hard error naming the repair; a tag that exists on
another branch is a version collision. Both are loud, because the alternative — a resolvable-but-wrong range
— produces a changelog that looks fine and is incomplete.

---

## Worked configurations

### One package

The shape in Quick start. Unscoped commits route to it via `default_scope`, so `feat: add export` and
`feat(app): add export` are the same commit.

### An app and its Helm chart

Two packages with **independent version lines** — never an umbrella and a child. They relate through
`app_version.track`, and nothing else.

```toml
[settings]
release_branches = ["main"]
default_scope    = "app"

[forge]
kind = "github"

[packages.app]
path       = "."
tag_format = "v{version}"

[[packages.app.updaters]]
kind = "cargo"
path = "Cargo.toml"

[packages.chart]
path            = "deploy/chart"
tag_format      = "chart-v{version}"
initial_version = "0.1.0"          # REQUIRED on a helm package: Chart.yaml
                                   # already has a number and cicada will not
                                   # invent a different one

[[packages.chart.updaters]]
kind            = "helm"
path            = "deploy/chart/Chart.yaml"
app_version     = { track = "app" }   # appVersion mirrors `app`
on_tracked_bump = "patch"             # the chart's own bump when `app` releases
```

Releasing `app` drags the chart: its `appVersion` is rewritten (always **quoted** — `appVersion: 1.10` is
the float `1.1` to YAML) and it takes a patch bump, because a changed chart artifact needs a new chart
version. Its changelog entry says so rather than rendering empty.

### A monorepo

```toml
[settings]
release_branches     = ["main", "release/*"]
maintenance_branches = ["release/*"]
default_scope        = "platform"     # unscoped commits go to the umbrella

[forge]
kind = "github"

[umbrella]                            # optional — declare one only if you want
name       = "platform"               # an aggregate version
tag_format = "platform-v{version}"
changelog  = "CHANGELOG.md"

[packages.api]
path       = "services/api"
tag_format = "api-v{version}"

[packages.worker]
path       = "services/worker"
tag_format = "worker-v{version}"
```

Attribution is **by scope and nothing else** — no path heuristics, no dependency graph. `feat(api,worker):`
releases both, because the author knows whether a shared change breaks its consumers and a graph does not.
Each package has its own anchor, so each has its own range and they release at different times without any
coordination.

The umbrella aggregates the **bump kind**, not the number: `api` going `1.4.2 → 2.0.0` makes the umbrella
take *a major bump on its own line*.

---

## Adopting into a repo that already has releases

Two steps, both by hand and both small:

1. Write `cicada.toml`.
2. Seed `.cicada-versions.toml` with the current version of each package:

```toml
[packages.api]
version = "1.4.2"
```

The tag that version names (`api-v1.4.2` here) must **exist and be reachable** from the branch you release
from. That is the only requirement — there is no import step and no state to migrate.

`Release-As:` footers left over from release-please are **ignored**, not honoured and not an error, so
history carrying them is fine.

---

## Commands

```
cicada check [--base <ref>] [--head <ref>]   validate config; lint a commit range
cicada plan [--json]                          what would be released, and why
cicada release-pr [--dry-run] [--no-push]     Phase A
cicada release    [--dry-run] [--no-push]     Phase B
cicada run        [--dry-run] [--no-push]     Phase B then Phase A — what CI calls
cicada template print <name>                  emit a built-in template
```

Global: `--token`, `--config`, `--date`, `--prerelease <id>`, `--json`, `-v`, `--quiet`.

### What needs a token

| | Writes files | Local commit/tag | Pushes | Forge API | Token |
|---|---|---|---|---|---|
| `plan`, `check` | — | — | — | — | not required |
| `--dry-run` | — | — | — | — | not required |
| `--no-push` | ✓ | ✓ | — | — | not required |
| default | ✓ | ✓ | ✓ | ✓ | required |

`plan` and `check` never acquire a network dependency. That is a guarantee, not an implementation detail —
it is what lets `check` run on an untrusted fork's pull request where no token exists.

### Exit codes

CI branches on these, so they are a contract:

| Code | Meaning |
|---|---|
| `0` | Success — **including "nothing to release"**, which is normal |
| `1` | Error — invalid config, git failure, forge API failure |
| `2` | Usage error |
| `3` | `check` found problems |
| `4` | **Refused** — cicada declined to act |

`3` and `4` are separate from `1` on purpose: *your commits are malformed* and *cicada declined to act* are
conditions a pipeline may want to handle differently from *cicada broke*.

---

## When cicada refuses

Exit `4` always means cicada could have done something and decided it would be wrong. Each refusal names the
repair.

| Refusal | Why | Fix |
|---|---|---|
| A `feat:` on a `maintenance_branches` line | A maintenance line promises its consumers only patches. Clamping it to a patch would ship a feature — or a breaking change — where nobody sees it | Put the change on a newer line, or re-do the cherry-pick as the fix it is. **There is no override** |
| The release PR branch has a commit cicada did not write | That commit exists nowhere else, so discarding it would lose it | Move it to the base branch, then delete or reset `cicada/<branch>` |
| The release PR was merged while stale | The recorded version is lower than the commits justify, so shipping it would break the semver contract silently | Correct the version in `.cicada-versions.toml` upwards and push |
| A computed version's tag exists elsewhere | Two release lines are converging on one number | Reconcile the lines before releasing either |
| A prerelease that sorts below the open one | Semver's ordering is what consumers resolve against | Pick an identifier that sorts up, or release the line stable |

A stale release PR is also **drafted, labelled `cicada:stale` and commented on**, because draft PRs are
unmergeable on both forges — refusing is only safe if the thing you refused to refresh cannot be merged.

---

## Changelogs and templates

Changelog entries and release notes come from **one** template, so they cannot drift. The built-ins are
printable, so customising starts from the real thing rather than from reverse-engineering it:

```console
$ cicada template print notes > .cicada/templates/notes.md.j2
```

Overrides are picked up by convention with no config change. Available: `notes`, `changelog`, `umbrella`,
`changelog-header`.

Entries are inserted at `<!-- cicada:insert -->` when present, prepended after the header otherwise, and the
file is created when absent. Existing entries are never rewritten, so a hand-corrected one stays corrected.

---

## Prereleases

Branch-level, never per-package:

```toml
prerelease_branches   = ["prerelease-v*-rc"]
prerelease_identifier = "rc"
```

`version` in the versions file keeps meaning **last stable release**, and a `prerelease` key tracks the open
line. Every release — `rc.1`, `rc.2`, and the eventual stable — computes from the same last-stable anchor,
so the notes are cumulative and the stable changelog entry comes out complete.

Prereleases write version files and cut tags and release objects, but **never** `CHANGELOG.md`. A line walks
the ladder by editing `prerelease_identifier` (`alpha` → `beta` → `rc`); the counter resets when either the
base or the identifier changes. Ordering comes from semver alone, so an identifier that sorts *down* is
refused — pick names that sort upward.

`cicada release-pr --prerelease rc` does the same thing ad hoc, without a branch.

---

## Requirements

- **`git` on PATH**, 2.20 or newer. cicada shells out to it; that is the design, not an accident.
- **A full clone.** `fetch-depth: 0` / `GIT_DEPTH: 0`. cicada detects a shallow clone and errors rather than
  computing a silently truncated changelog.
- **Linux**, x86_64 or aarch64, for the shipped binaries and images. `cargo install cicada` works anywhere
  Rust does.

## Documentation

- [`docs/design.md`](docs/design.md) — the full design, with every decision and its reasoning
- [`docs/scenarios/`](docs/scenarios/) — behavioural coverage, written as executable scenarios
- [`docs/future-work.md`](docs/future-work.md) — what was deliberately deferred, and what would trigger it

## Licence

MIT OR Apache-2.0.

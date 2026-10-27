# cicada

cicada is a release tool. It reads conventional commits and makes versions, changelogs, tags and releases.
It is similar to [release-please](https://github.com/googleapis/release-please), with one important
difference. **cicada does its work with git, and it uses the forge API only for a small number of
operations.** (A forge is a code host, such as GitHub or GitLab.) Because of this design, one code path
supports GitHub and GitLab. You can also rehearse a release on your computer with no token and no network.

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

**On GitHub Actions, you do not install cicada.** The step `uses: boenan/cicada@v0` downloads the binary
and verifies it. On GitLab, the shipped template runs the cicada image. The rest of this section applies to
all other environments.

**A released binary.** Each release contains static musl builds and a `SHA256SUMS` file:

```bash
base=https://github.com/boenan/cicada/releases/latest/download
asset=cicada-x86_64-unknown-linux-musl.tar.gz    # or cicada-aarch64-unknown-linux-musl.tar.gz

curl -sSfLO "$base/$asset"
curl -sSfLO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
tar xzf "$asset"
install -m755 cicada /usr/local/bin/cicada
```

The archive contains one file, `cicada`, and that file is already executable. This is the reason for the
archive. A GitHub release asset has no file permissions. Thus a bare binary downloads without its executable
bit, and you must run `chmod` on it before it can run.

The command uses `--ignore-missing` because `SHA256SUMS` lists all architectures, and you downloaded only
one. The check still fails if `SHA256SUMS` does not list the file that you downloaded. Thus it is a real check.

**The image.** The image contains `git`. cicada requires git (#17) and does not start without it:

```bash
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/repo" -w /repo ghcr.io/boenan/cicada:v0 cicada plan
```

The `-u` option in this line is necessary. Without it, the container runs as root, but your user owns the
checkout. git then refuses to operate on a repository with a "dubious" owner. One possible solution is to set
`safe.directory` in the image. This solution does **not** work. cicada runs git with `GIT_CONFIG_NOSYSTEM=1`, so
git does not read the system configuration of the image.

Each release publishes its exact tag, for example `v1.2.3`. Each stable release also moves the tags `v1.2`,
`v1` and `latest`. **A prerelease publishes only its exact tag.** Thus `:v0` never points to an alpha.

**Or use `cargo install cicada`.** This works on all platforms that Rust supports. These include the
platforms that the binaries and images do not support.

---

## Quick start

**1. Write `cicada.toml` at the root of the repository.** cicada has no `cicada init` command. You control
the package list. Also, you must examine each line that a scanner guesses. This example is for a repository
with one Rust service:

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

**2. Check the configuration.** This step needs no token and no network:

```console
$ cicada check
✓ config ok
```

**3. Rehearse the release.** This step also writes nothing:

```console
$ cicada plan
  app  1.4.2 → 1.5.0  minor
       feat: add export
       fix: handle nil

  1 release
```

**4. See the exact bytes.** Then make a real local commit, and examine it with `git show`:

```console
$ cicada release-pr --dry-run     # prints the diff it would write
$ cicada release-pr --no-push     # writes a real commit on cicada/main; pushes nothing
$ git show cicada/main
```

**5. Configure CI** (see [CI](#ci)). cicada then does the remaining work.

---

## ⚠ The CI-trigger footgun

A "footgun" is a setting that is easy to get wrong and that causes damage without a warning. **Read this
section first.** If you do not, your releases can stop and you get no error message.

On GitHub, the default `GITHUB_TOKEN` **does not start workflows on pull requests that it creates**. On
GitLab, `CI_JOB_TOKEN` has the same problem for merge requests.

You get no error, and the result looks like success. cicada opens the release PR, but the checks of the PR
never start. The PR stays open and looks healthy. If you enabled auto-merge, the merge waits for checks that
never run. Thus **the release cycle stops completely**.

On GitHub, use a **PAT or a GitHub App token**. On GitLab, use a **Project Access Token**. This is a
requirement, not a convenience.

**On GitHub, also set `persist-credentials: false` on `actions/checkout`.** Without this setting, this
occurs:

1. The checkout writes `GITHUB_TOKEN` into `.git/config` as an `http.<url>.extraheader`.
2. git matches that header against the push URL of cicada.
3. The push authenticates as `GITHUB_TOKEN`, even if your PAT is correct.
4. The push *succeeds*, so you see no problem.
5. But `GITHUB_TOKEN` created the tag, so no workflow starts for it. Your release-artifact jobs never run.

cicada does not use these credentials. It makes its own push URL and gives git the token through
`GIT_ASKPASS`.

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
          persist-credentials: false # or GITHUB_TOKEN hijacks cicada's push — see above
      - uses: boenan/cicada@v0
        with:
          token: ${{ secrets.CICADA_TOKEN }}   # see the footgun above
```

You must also enable **Settings → Actions → Allow GitHub Actions to create and approve pull requests**.

### GitLab

```yaml
include:
  - remote: https://raw.githubusercontent.com/boenan/cicada/v0/ci/gitlab-ci.yml
```

This template contains three jobs:

- `cicada:release`: ready to use.
- `cicada:check`: checks the commits of a merge request.
- `.cicada`: a hidden job that you can extend.

The template already sets `GIT_DEPTH: 0` and `resource_group`. Set `CICADA_TOKEN` to a Project Access Token
with the **Maintainer** role and the `api` and `write_repository` scopes.

### Building artifacts

cicada does **not** build your binaries or images. cicada controls versions, changelogs, tags and release
objects. **A separate workflow builds your artifacts, and the tag that cicada pushes starts that workflow.**
Because of this separation, an artifact build can fail and run again with no change to the version state.

---

## How it works

cicada has two phases. `cicada run` does the two phases **in the correct order**. For this reason, we
recommend that you call `run` and do not call each phase yourself.

**Phase A — `cicada release-pr`.** For each package, this phase does these steps:

1. It reads the commits in `anchor..HEAD`.
2. It assigns each commit to a package by its conventional-commit scope.
3. It calculates the versions.
4. It writes the version files and changelogs on a branch named `cicada/{branch}`.
5. It opens a pull request.

cicada **replaces** this branch on each run. It never updates it. The branch holds exactly one commit, and
that commit depends only on the current tip of your branch. Thus you never get a conflict.

**Phase B — `cicada release`.** This phase keeps no state. For each recorded version, it asks two questions.
Does the tag exist? Does a release object exist? It then creates each item that is missing. It reads nothing
back from the forge to make a decision. Thus squash-merge, rebase-merge and changed commit messages have no
effect on it. If you run it again, it does nothing.

**Phase B runs first.** The same push starts both phases. If Phase A ran first, it would calculate its range
from the *old* tag. It would then include again all commits of the release that you just merged. It would
propose that release again, on each cycle.

### Two files

| File | Owned by | Holds |
|---|---|---|
| `cicada.toml` | you | configuration |
| `.cicada-versions.toml` | cicada | a version per package, and nothing else |

`.cicada-versions.toml` is the **only source of truth** for versions. `Cargo.toml`, `Chart.yaml` and similar
files are *outputs*. cicada never reads a version from them. This has one visible result. If cicada
calculates `0.0.1` for a first release, it changes a `cargo new` manifest from `0.1.0` **down** to `0.0.1`.
This is correct.

### The release manifest

Each run that releases a package writes `target/cicada/release-manifest.json`. This file lists the releases
that shipped **together**. A deploy step needs this information:

```json
{
  "schema": 1,
  "date": "2026-09-21",
  "releases": [
    { "package": "api", "version": "1.5.0", "tag": "api-v1.5.0", "sha": "…" }
  ]
}
```

Add `target/cicada/` to `.gitignore` if `target/` is not already in it. The file describes one run, not a
state of the repository. Do not commit it.

cicada also puts the same JSON into **each** release object that the run creates, in a collapsed
`<details>` block. Thus a reader of any one release can see what else shipped with it. The reader does not
need to know which release is the summary.

The file on disk has two more uses:

- It is the only way to examine a `--no-push` rehearsal.
- It lets the deploy job read the result without a token and without an API call.

### The tag is the anchor

cicada stores no commit SHA. The recorded version gives a tag name. cicada finds that tag, and the tag must
be **reachable from HEAD**. These two errors can occur:

- **The tag is missing.** cicada stops with an error that tells you how to repair the problem.
- **The tag exists on a different branch.** cicada reports a version collision.

cicada makes both errors loud for a reason. The alternative is a range that resolves but is wrong. That
range gives a changelog that looks correct but is incomplete.

---

## Worked configurations

### One package

This is the configuration from Quick start. `default_scope` assigns unscoped commits to this package. Thus
cicada treats `feat: add export` and `feat(app): add export` as the same commit.

### An app and its Helm chart

The two packages have **independent version lines**. One package is never an umbrella of the other. Their
only relation is `app_version.track`.

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

When `app` releases, the chart also changes:

- cicada changes the `appVersion` of the chart. The value always has **quotes**, because YAML reads
  `appVersion: 1.10` as the float `1.1`.
- The chart gets a patch bump, because a changed chart artifact needs a new chart version.
- The changelog entry of the chart gives this reason, so the entry is not empty.

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

cicada assigns commits to packages **only by scope**. It does not use file paths or a dependency graph.
`feat(api,worker):` releases both packages. The reason: the author knows if a shared change breaks its
consumers, but a graph does not. Each package has its own anchor. Thus each package has its own range, and
the packages release at different times with no coordination.

The umbrella uses the **bump type** of its packages, not their numbers. For example, if `api` goes from
`1.4.2 → 2.0.0`, the umbrella gets *a major bump on its own version line*.

---

## Adopting into a repo that already has releases

You do two small steps by hand:

1. Write `cicada.toml`.
2. Write the current version of each package into `.cicada-versions.toml`:

```toml
[packages.api]
version = "1.4.2"
```

The tag of that version (`api-v1.4.2` in this example) must **exist and be reachable** from the branch that
you release from. This is the only requirement. There is no import step and no state to migrate.

cicada **ignores** the `Release-As:` footers from release-please. It does not use them, and it does not
report an error. Thus a history that contains them is not a problem.

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

Global options: `--token`, `--config`, `--date`, `--prerelease <id>`, `--json`, `-v`, `--quiet`.

### What needs a token

| | Writes files | Local commit/tag | Pushes | Forge API | Token |
|---|---|---|---|---|---|
| `plan`, `check` | — | — | — | — | not required |
| `--dry-run` | — | — | — | — | not required |
| `--no-push` | ✓ | ✓ | — | — | not required |
| default | ✓ | ✓ | ✓ | ✓ | required |

`plan` and `check` never need the network. This is a guarantee, not an implementation detail. Because of
this guarantee, `check` can run on a pull request from an untrusted fork, where no token is available.

### Exit codes

CI uses these codes to make decisions, so they are a contract:

| Code | Meaning |
|---|---|
| `0` | Success — **including "nothing to release"**, which is normal |
| `1` | Error — invalid config, git failure, forge API failure |
| `2` | Usage error |
| `3` | `check` found problems |
| `4` | **Refused** — cicada declined to act |

Codes `3` and `4` are different from `1` on purpose. A pipeline can handle each of these conditions in a
different way:

- *Your commits are malformed* (`3`).
- *cicada refused to act* (`4`).
- *cicada failed* (`1`).

---

## When cicada refuses

Exit code `4` always means this: cicada could act, but it decided that the action would be wrong. Each
refusal tells you how to repair the problem.

| Refusal | Why | Repair |
|---|---|---|
| A `feat:` on a `maintenance_branches` line | A maintenance line promises only patches to its consumers. If cicada changed the feature to a patch, it would ship a feature, or a breaking change, that nobody sees | Put the change on a newer line. If the change is really a fix, do the cherry-pick again as a `fix:` commit. **No override exists** |
| The release PR branch has a commit that cicada did not write | That commit exists only on this branch. If cicada discarded the branch, the commit would be lost | Move the commit to the base branch. Then delete or reset `cicada/<branch>` |
| Someone merged the release PR while it was stale | The recorded version is lower than the commits require. A release would break the semver contract with no warning | Increase the version in `.cicada-versions.toml` to the correct value, and push |
| The tag of a calculated version exists on a different branch | Two release lines are moving to the same version number | Make the two lines agree before you release either line |
| A prerelease that sorts below the open prerelease | Consumers resolve versions by semver order | Use an identifier that sorts higher, or make a stable release of the line |

cicada also changes a stale release PR to a **draft**, adds the `cicada:stale` label, and adds a comment.
Neither forge lets you merge a draft PR. A refusal is safe only if nobody can merge the PR that cicada
refused to update.

---

## Changelogs and templates

Changelog entries and release notes come from **one** template. Thus they cannot become different from each
other. You can print the built-in templates. Thus you can start a custom template from the real template,
not from a guess:

```console
$ cicada template print notes > .cicada/templates/notes.md.j2
```

cicada finds a custom template by its file location. You do not change the configuration. The available
templates are `notes`, `changelog`, `umbrella` and `changelog-header`.

cicada writes the changelog as follows:

- If the file contains `<!-- cicada:insert -->`, cicada adds new entries at that marker.
- If the marker is not there, cicada adds new entries after the header.
- If the file does not exist, cicada creates it.

cicada never changes existing entries. Thus an entry that you corrected by hand stays correct.

---

## Prereleases

You configure prereleases per branch, never per package:

```toml
prerelease_branches   = ["prerelease-v*-rc"]
prerelease_identifier = "rc"
```

In the versions file, `version` always means the **last stable release**. A `prerelease` key records the
open prerelease line. cicada calculates each release (`rc.1`, `rc.2` and the final stable release) from the
same last stable anchor. Thus the notes include all changes since that anchor, and the changelog entry of
the stable release is complete.

Prereleases write version files, tags and release objects. But they **never** write `CHANGELOG.md`.

To move a line to the next stage, change `prerelease_identifier` (`alpha` → `beta` → `rc`). The counter
resets when the base version or the identifier changes. The order comes only from semver. Thus cicada
refuses an identifier that sorts *lower* than the current one. Use names that sort higher.

`cicada release-pr --prerelease rc` does the same thing for one run, without a branch.

---

## Requirements

- **`git` on PATH**, version 2.20 or newer. cicada runs git for its operations. This is part of the design,
  not an accident.
- **A full clone**: `fetch-depth: 0` on GitHub, or `GIT_DEPTH: 0` on GitLab. cicada detects a shallow clone
  and stops with an error. It does not calculate a changelog that is incomplete without a warning.
- **Linux** on x86_64 or aarch64, for the shipped binaries and images. `cargo install cicada` works on all
  platforms that Rust supports.

## Documentation

- [`docs/design.md`](docs/design.md): the full design, with each decision and its reason.
- [`docs/scenarios/`](docs/scenarios/): tests of the behavior, written as executable scenarios.
- [`docs/future-work.md`](docs/future-work.md): the features that we did not do yet, on purpose, and the
  conditions that would make us start them.

## Licence

MIT OR Apache-2.0.

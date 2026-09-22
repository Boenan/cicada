# cicada — future work

Deliberately deferred from v1. Each entry records *why* it was deferred and *what would trigger* picking it
up, so a future reader can tell a considered omission from an oversight. Decision numbers reference
`docs/design.md`.

---

## 1. Registry publishing and intra-workspace dependency rewriting

**Deferred because** nothing in scope is published to crates.io — every crate is private/internal, where
`path` dependencies win locally and stale version requirements are harmless.

**The problem it would solve.** Cicada writes a package's own version but leaves dependents' requirements
alone:

```toml
# services/gateway/Cargo.toml
auth = { version = "0.3", path = "../../libs/auth" }   # stays "0.3" after auth → 0.4.0
```

For a private workspace this is invisible. For `cargo publish` it is fatal: the published crate declares a
dependency on a version that is no longer the one it was built against.

**What it would take.** Parse the workspace dependency graph, rewrite every internal requirement when its
target bumps, and topologically order releases so dependencies publish before dependents. This is what
release-please's `cargo-workspace` plugin does, and it is a substantial piece of work — it is also the point
at which cicada would need real dependency-graph propagation (see §6).

**Trigger:** the first crate published to a registry.

---

## 2. Per-forge release asset upload

**Deferred because** the forges are asymmetric and v1 has a portable answer.

GitHub releases host uploaded files. GitLab releases do **not** — they hold *links* to artifacts stored
elsewhere (package registry, job artifacts). So "attach `release-manifest.json` to the release" is not one
operation, it is two unrelated ones.

**v1 behaviour (#57):** the manifest is embedded in the release body as fenced JSON inside a collapsed
`<details>` block, and written to `target/cicada/release-manifest.json`. Identical on every forge, needs no
asset API, and stays greppable by a human reading the release.

**What it would take.** A per-forge capability flag: upload directly on GitHub; on GitLab, push the file to
the generic package registry (or publish it as a job artifact) and attach a link. Fidelity improves; the
adapter grows a branch that only one forge exercises.

**Trigger:** manifests large enough to be unpleasant inline, or a consumer that wants to fetch the manifest
by URL rather than parse it out of a release body.

---

## 3. Additional language updaters

v1 ships `cargo` and `helm` (#13). The `generic` updater with marker comments (#50) covers everything else
in the meantime — a Go or Python service can be version-stamped today with `# x-cicada-version`.

| Ecosystem | Version lives in | Why it was deferred |
|---|---|---|
| **Go** | nowhere — the tag *is* the version | Major ≥ 2 requires the module path to end in `/v2`, and submodules need path-prefixed tags (`services/api/v1.2.3`). Real complexity for a language where the file update is usually a no-op |
| **Java** | `pom.xml` `<version>`, `build.gradle(.kts)` | The **SNAPSHOT convention** is the expensive part: release `1.4.0`, then immediately commit `1.5.0-SNAPSHOT`. That is a *second* commit after every release, which the two-phase model has no place for today |
| **Python** | `pyproject.toml`, often duplicated in `__init__.py` | Cheapest of the three; mostly needs deciding which of the several conventions to support |

Adding a `kind` is additive — no schema change (#13). Java's SNAPSHOT flow is the only one that would touch
the release *model* rather than just add an updater.

**Trigger:** a service in one of these languages joining a cicada-managed repo.

---

## 4. Signed / verified commits

**Deferred because** it would mean a second commit backend.

Cicada commits with `git` and pushes over HTTPS (#17, #18), so its commits show as unverified. release-please
gets verification for free precisely because it never touches git — commits created through the GitHub API
are signed by GitHub.

**What it would take.** Either an optional forge-API commit backend for GitHub (`createTree` →
`createCommit` → `updateRef`), which is GitHub-only and duplicates the commit path; or GPG/SSH signing in the
git backend, which needs a key in CI and a decision about where it lives. The API path is the one that
actually yields the green "Verified" badge.

**Trigger:** a repo whose branch protection requires signed commits.

---

## 5. `gix` read path

**Deferred because** shelling out to `git` was chosen deliberately (#17): push authentication is the hard
part, and `git` already solves credential helpers, SSH agents, token URLs, and proxies correctly.

**What it would take.** Swap the read operations (log walking, tag listing, ancestry checks) for `gix` behind
the existing `GitBackend` trait, keeping the subprocess path for pushes. The trait exists for this.

**Trigger:** measurable slowness on a large monorepo. Unlikely — N packages × one `git log` each is
milliseconds-scale.

---

## 6. Dependency-graph propagation

**Considered and rejected, not merely deferred.** Blast radius is *declared* by the author through
multi-scope commits (`feat(auth,api,gateway):`), which is better information than a dependency graph can
provide: the human knows whether an internal library change is actually breaking for its consumers, and a
graph does not.

The one declared exception is Helm `appVersion` tracking (#22), which is explicit in config rather than
inferred.

**What would change the calculus:** §1. Publishing to a registry forces real graph traversal for ordering
and requirement rewriting, at which point propagation comes almost free.

---

## 7. Bitbucket and Gitea adapters

**Deferred because** no repo in scope uses them. The `Forge` trait (#53) is nine methods and everything else
is git, so an adapter is a self-contained piece of work needing no core change — which is the whole point of
the git-as-substrate design.

**Trigger:** demand.

---

## 8. macOS and Windows binaries

v1 ships `x86_64` and `aarch64` Linux musl binaries plus multi-arch Linux Docker images. macOS is a
straightforward addition to the release matrix. Windows needs the git-subprocess layer audited for path and
quoting assumptions, and `GIT_ASKPASS` behaves differently there.

**Trigger:** developers wanting to run `cicada plan` locally on a Mac — likely early, and cheap.

---

## 9. Adoption into repos with existing release history

**Explicit non-goal in v1.** There is no config generation at all (§12); matching `cicada.toml` to an
existing repo's tags and layout is a one-time human job.

**What a future version could do.** Seed `.cicada-versions.toml` from the newest tag per declared package,
and import a `release-please-config.json`. Both are convenience only — the manual path always works, because
the versions file is a version and nothing else (#1, #2).

**Trigger:** enough repos being migrated that the manual step becomes the bottleneck.

---

## 10. Changelog reconciliation across maintenance branches

**Known divergence, accepted.** A fix released as `1.4.8` on `release/1.4` is written into that branch's
`CHANGELOG.md`. `main` never learns about it, because the changelog is per-branch state like everything else.

Merging changelogs across release lines means reconciling independently-written files with no common
ancestor for the new entries — a merge conflict generator. Out of scope, and arguably correct as-is: each
branch's changelog describes that branch.

**Trigger:** someone actually being confused by it in practice.

---

## 11. Revert handling

**Deferred because** doing it properly means matching a revert back to what it reverted, and the only
reliable signal is a footer the author may not have written.

**The problem it would solve.** Reverting a commit does not remove it from the range:

```
anchor v1.4.2 ── c1 feat: add export ── c2 revert: "feat: add export"
```

`git log v1.4.2..HEAD` returns **both**. c1 is still a `feat`, so the plan is still `1.5.0`, and the
changelog advertises a feature whose code is no longer in the tree. `revert` is not in the default
`sections` list, so c2 is an unknown type and is ignored entirely (§Commit parsing).

**What it would take.** Parse `Reverts: <sha>` footers — the Conventional Commits convention — and drop
both the revert and its target from the range. `git revert` itself writes `This reverts commit <sha>.`
into the body rather than a footer, so a body fallback is needed to cover the common case, which means
matching prose. Interactions to settle: a revert whose target is *before* the anchor (drop the revert,
leave the version alone), a revert that is itself later reverted, and whether a reverted breaking change
un-does the major bump.

**Consequence while deferred.** Decision #34 (empty plan → close the open release PR) loses its motivating
example. The behaviour still stands, but its realistic triggers are configuration edits — a section's
`bump` flipped to `none`, a type renamed out of `sections`, a package removed while its PR is open — not
reverts. Force-pushing a protected branch is not a trigger worth designing for.

**Workaround today.** There is none inside cicada — forced versions are deferred too (§13). The author's
options are to reword the revert so the range no longer contains a releasable type, or to accept the release
and correct the changelog entry by hand. That the workaround got *worse* when §13 was deferred is the reason
these two entries should be picked up together.

**Trigger:** the first time a released-then-reverted feature ships in a changelog and someone is misled by it.

---

## 12. `cicada init`

**Deferred because** it is opinionated code in service of a step that happens once per repo, and the person
running it is the only one who can check its answer.

An earlier design had `init` scan the repo — classifying `Cargo.toml` files into virtual manifests, real
packages and version-inheriting packages, reading `Chart.yaml` names while skipping vendored subcharts under
`charts/`, pairing apps to charts by name, detecting the forge from `origin` — and emit
`cicada.example.toml` for review. Every one of those is a heuristic that a developer must read line by line
before renaming the file, because #8 makes the package list explicit precisely so a human owns it. The
review is the expensive part, and `init` does not remove it.

**v1 behaviour.** The developer writes `cicada.toml` by hand and, when adopting into a repo with existing
history, seeds `.cicada-versions.toml` with the current version per package. Both files are small: the
versions file is one number per package (#1), and `cicada check` validates the config against the repo —
missing paths, duplicate tag formats, duplicate updater outputs, `default_scope` naming nothing, and the
Cargo workspace-inheritance trap (#24). Shipped documentation carries a worked `cicada.toml` per repo shape,
which is what an adopter actually copies from.

**What it would take.** The scanner described above, plus the app⇄chart pairing heuristic and a loud warning
for `version.workspace = true`. None of it is hard; all of it is opinion, and all of it needs reviewing
anyway.

**Trigger:** enough repos being onboarded that hand-writing the config becomes the bottleneck — the same
trigger as §9, and the two should be built together if either is.

---

## 13. Forced versions (`Release-As:` and `--release-as`)

**Removed from v1 scope, not merely unimplemented.** An earlier design let a human state a version outright:
a `Release-As: 2.0.0` commit footer applying to that commit's scopes, and a repeatable `--release-as
pkg=2.0.0` CLI equivalent. Both are gone (#39), and with them the whole notion of a per-commit override.

**What it was doing.** Four jobs, and it is worth being precise about which of them still have an answer:

| Job | Without forced versions |
|---|---|
| Escape the maintenance guard for a deliberate `feat:` on a patch-only line | **No answer.** Put the change on a newer line, or re-do the cherry-pick as the fix it is (#28) |
| Open a new release line at its first, necessarily-minor version | **No answer** — which is why §14 is deferred with it |
| Reach `1.0.0` from `0.x` | `pre_major = "major"` plus a breaking change — config, not a footer |
| Cut a release by hand | Hand-edit `.cicada-versions.toml` *upwards*; the staleness guard permits it (J4) |

**Why it needs discussion rather than reinstatement.** The mechanism is easy; its interactions are not, and
the scenario passes surfaced three that were never settled. Does a forced version bypass *prerelease
decoration* as well as bump computation — i.e. what does `1.5.0` mean on a branch matching
`prerelease_branches`? Is a prerelease value (`2.0.0-beta.1`) legal outside the §Prereleases flow? Does a
forced version force a *release* when attached to a `bump = "none"` type like `chore:`? And monotonicity
(#76) has to compare against the right field when both `version` and `prerelease` are recorded, or a forced
stable can sort below an rc that already shipped. Each has a defensible answer; none has a decided one.

**Trigger:** any of §14, a repo needing a deliberate exception to the maintenance guard, or the revert case
in §11 becoming common enough that hand-correcting the changelog is the bottleneck.

---

## 14. Release-branch releases and terminal milestone lines

**Deferred as a unit** (#81). `main` never releases; every release comes from a branch that owns its own
version line — `release/1.5` shipping `1.5.0`, `1.5.1`, `1.5.2` — while the trunk only integrates. Istio runs
exactly this, and the Chrome milestone branches are the same shape, so it is a real arrangement rather than a
hypothetical one.

**What blocks it is one thing: a line's opening version.** A new line's first release is necessarily a minor
(`1.4.1` → `1.5.0`), and `release/*` matching `maintenance_branches` means the guard refuses precisely that
bump. Forced versions were the mechanism for stating it (§13). Hand-editing the versions file does not
substitute, because the staleness guard refuses a recorded version *below* the recomputation — which is this
case exactly. Everything else the arrangement needs already works: per-branch state, the collision check, and
cherry-picking outwards from the integration point.

**What it would take.** Deciding how a line states its opening version. Three candidates, in rough order of
how much they cost: a narrowed `initial_version` meaning *first release on this line* rather than *first
release ever*; a guard exception when the computed stable equals the base of an already-open `prerelease`
(which covers a line that stabilised through rcs, and nothing else); or §13 returning.

A second question rides along, and it is the more interesting one. cicada's classifiers are globs over the
branch name (#82), so a line that is a prerelease line in March and a supported release line in April has to
be **renamed** to move between them. That works and costs one `git branch -m`, but neither Istio nor Chrome
renames — `release-1.25` is `release-1.25` from cut to retirement, and what changes is the kind of release it
cuts. That is a question about whether the classifier is shaped right, not about whether operators should
adapt. The obvious alternative — graduating by dropping a `--prerelease` flag — costs offline truthfulness,
since `cicada plan` run locally on the line would report a stable version unless the developer remembers the
flag, and that is a stated goal (§Goals 6) rather than an implementation detail.

**Trigger:** a repo that ships milestone lines. Take it together with §13.

---

## 15. Linting a pull-request title

**Deferred because** v1 has no decided semantics for it (#87). Under squash-merge the PR title is the only
thing that lands on the default branch, so it is the thing a squash-merge repo most wants linted — and
`check` lints a commit range and nothing else.

An earlier draft routed it through `commit_source = "title"`, which is not a member of that setting's enum
(`commits` | `merges` | `both`) and would be meaningless for history reading. A `--title <s>` flag was the
obvious home, but it left real questions: does it lint *instead of* the range or *in addition to* it, and is
"this repo lints titles" a property of the repo (config) or of the CI wiring (a flag)? A flag with undecided
semantics is worse than an absent one, so it is absent.

**What it would take.** Deciding those two questions and adding the flag. The lint itself is the conventional
-commit parser already in `cicada-core`, run over one string — the cheapest part by far.

**Trigger:** a squash-merge repo adopting cicada and wanting the check at review time.

---

## 16. A rendered summary in the release-PR title — **done**

Shipped. Kept here with its number so the references elsewhere still resolve.

`release_pr_title` renders `{branch}` and `{summary}`. The trigger recorded here was "the first time
someone asks which release a PR is proposing", and that is exactly how it came up.

`{summary}` names the releases in config order and truncates after three — `api 1.5.0, worker 0.9.2,
auth 0.1.0 and 2 more`. The umbrella candidate was **rejected** rather than deferred: naming only the
umbrella reads as though nothing else moved, and it is a rule that fires only in repos declaring one, so
the shape most likely to produce a surprising title would be the one least often exercised. The umbrella
is one entry among the rest, and being last (#96) it is usually counted rather than named.

The default is unchanged and carries no placeholders, so this is opt-in and no adopter's release commit
subject moves under them.

---

## 17. Version schemes other than semver

**Deferred because** a second scheme is a second *arithmetic* at minimum, and potentially a second *ordering*
— and ordering is the substrate under every refusal in the design.

**What it would serve.** CalVer (`2026.9.1`, `2024.1.3`) and build-number versioning (`1234`). For comparison,
release-please is semver-only too: its six `versioning-strategy` values (`default`, `always-bump-patch`,
`always-bump-minor`, `always-bump-major`, `service-pack`, `prerelease`) all change which segment is
incremented, none derives a number from a calendar. CalVer there is reachable only by stating each version by
hand through `release-as`, or by pinning `always-bump-patch` and rolling the period from a separate workflow.

**Why ordering is the expensive part.** Six separate refusals compare two versions: the collision check
(#9, #80), hand-edited monotonicity (#76), computed prerelease monotonicity (#85), `prev` selection (#90), the
staleness recomputation (#33, #78), and prerelease counter ordering (#64). A scheme cicada cannot order is a
scheme in which each of those either fails or, worse, silently passes.

**What it would take, and the natural narrowing.** A scheme whose output is *semver-shaped* — three numeric
segments, no leading zeroes — keeps the existing ordering entirely, so only the arithmetic that produces the
next number changes. That covers `YYYY.MM.MICRO` and `YYYY.MINOR.MICRO` unpadded, and it is a contained piece
of work: a per-package `versioning` setting, a format with calver.org tokens, and the date threaded into
`cicada-core` as a value rather than read from a clock (the `LinkStyle` pattern, A2) so that core stays pure
and `--dry-run` cannot disagree with the run that follows it.

Four things need deciding with it, all surfaced by tracing the sequence rather than reading:

- **Maintenance lines have to pin the period** to the recorded version rather than the date. Without it the
  computed period differs from the line's on every run, `MICRO` resets, the maintenance guard refuses the
  period change, and the line can never release again. release-please's `always-bump-patch` workaround has
  exactly this problem, pushed outside the tool.
- **Going backwards must be an error.** A skewed CI clock or a release cut from a branch whose period has
  passed computes a version sorting below the recorded one — #76's rule applied to a third source.
- **Breaking changes lose their expression.** A calendar carries no compatibility promise, so `pre_major`,
  the first-release table (#37) and umbrella aggregation (#10, which aggregates bump *kind*) have nothing to
  say and must be rejected rather than accepted and ignored.
- **Padded and two-segment forms stay out** (`2026.09.1`, `24.04`), along with `MAJOR`/`MINOR` tokens, which
  are not derivable without the human-stated version removed in #39.

**Trigger:** a service in scope that versions by calendar. Note the narrow form needs no ordering abstraction
at all, so it can be taken without reopening §13.


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

**Explicit non-goal in v1.** `cicada init` writes `cicada.example.toml` (#23); matching it to an existing
repo's tags and layout is a one-time human job.

**What a future version could do.** `cicada init --from-existing` reading the newest tag per detected package
to prefill `.cicada-versions.toml`, and a `release-please-config.json` importer. Both are convenience only —
the manual path always works, because the versions file is just a version and a SHA.

**Trigger:** enough repos being migrated that the manual step becomes the bottleneck.

---

## 10. Changelog reconciliation across maintenance branches

**Known divergence, accepted.** A fix released as `1.4.8` on `release/1.4` is written into that branch's
`CHANGELOG.md`. `main` never learns about it, because the changelog is per-branch state like everything else.

Merging changelogs across release lines means reconciling independently-written files with no common
ancestor for the new entries — a merge conflict generator. Out of scope, and arguably correct as-is: each
branch's changelog describes that branch.

**Trigger:** someone actually being confused by it in practice.

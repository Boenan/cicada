//! The things a third party adopts from: the README and the CI templates.
//!
//! M7's exit criterion is *"a third party can adopt from the README alone"*,
//! which is a claim about documentation — and documentation rots silently. These
//! tests make it mechanical: every configuration the README shows is parsed by
//! the real parser, and every shipped YAML asset is parsed by the same YAML code
//! the updaters use.
//!
//! The failure this prevents is specific and embarrassing: a config key renamed
//! in `config.rs`, every test updated, and the README left quietly telling
//! adopters to write the old one.

use std::path::{Path, PathBuf};

use cicada_core::config::Config;
use cicada_core::versions::VersionsFile;

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/cicada`.
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Every fenced block of a given language in a Markdown file.
fn code_blocks(md: &str, lang: &str) -> Vec<String> {
    let open = format!("```{lang}\n");
    let mut out = Vec::new();
    let mut rest = md;
    while let Some(i) = rest.find(&open) {
        let body = &rest[i + open.len()..];
        let Some(j) = body.find("```") else { break };
        out.push(body[..j].to_string());
        rest = &body[j + 3..];
    }
    out
}

#[test]
fn every_configuration_the_readme_shows_is_one_the_parser_accepts() {
    let md = read("README.md");
    let blocks = code_blocks(&md, "toml");
    assert!(blocks.len() >= 4, "the README should still be showing worked configs");

    let mut configs = 0;
    for (i, b) in blocks.iter().enumerate() {
        // A block with `[settings]` is a whole `cicada.toml`; anything else is a
        // fragment, which still has to be valid TOML.
        if b.contains("[settings]") {
            Config::from_toml(b).unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
            configs += 1;
        } else if b.contains("version =") && !b.contains("[packages.app.updaters]") {
            // The versions-file seed in "Adopting into a repo that already has
            // releases" must parse as one, or the adoption instructions are wrong.
            if VersionsFile::from_toml(b).is_err() {
                b.parse::<toml::Table>()
                    .unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
            }
        } else {
            b.parse::<toml::Table>()
                .unwrap_or_else(|e| panic!("README toml block {i}:\n{b}\n\n{e}"));
        }
    }
    assert!(configs >= 3, "one worked config per repo shape, and there were {configs}");
}

#[test]
fn the_readme_documents_every_template_name_that_exists() {
    // `cicada template print <name>` is in the README; if a built-in is added and
    // not listed, adopters cannot discover it.
    let md = read("README.md");
    for name in cicada_core::template::built_in_names() {
        assert!(md.contains(name), "the README does not mention the `{name}` template");
    }
}

#[test]
fn the_readme_documents_every_exit_code_the_binary_can_return() {
    let md = read("README.md");
    for code in ["`0`", "`1`", "`2`", "`3`", "`4`"] {
        assert!(md.contains(code), "the README's exit-code table is missing {code}");
    }
    // The one that is counter-intuitive and therefore the one most worth stating.
    assert!(
        md.contains("including \"nothing to release\""),
        "the README must say that exit 0 covers \"nothing to release\""
    );
}

#[test]
fn the_ci_trigger_footgun_is_prominent_rather_than_a_footnote() {
    // "prominent in the docs rather than a footnote — because a release PR whose
    // CI never runs defeats the entire point of decision #7".
    let md = read("README.md");
    let at = md.find("CI-trigger footgun").expect("the README must carry the footgun section");
    assert!(at < md.len() / 3, "it is buried at {at} of {}", md.len());

    assert!(md.contains("GITHUB_TOKEN"), "it must name the GitHub token");
    assert!(md.contains("CI_JOB_TOKEN"), "and the GitLab one");
    // Both forges' permission requirements, which are the other half of it.
    assert!(md.contains("pull-requests: write"));
    assert!(md.contains("write_repository"));
    assert!(md.contains("Allow GitHub Actions to create and approve pull requests"));
}

#[test]
fn every_shipped_yaml_asset_parses() {
    // With cicada's own YAML reader, which is the one that has to cope with
    // whatever a repository actually contains.
    for asset in [
        "action.yml",
        "ci/gitlab-ci.yml",
        ".github/workflows/release.yml",
        ".github/workflows/ci.yml",
        ".github/workflows/artifacts.yml",
    ] {
        let src = read(asset);
        cicada_core::updater::yaml::verify(&src)
            .unwrap_or_else(|e| panic!("{asset} does not parse: {e:?}"));
    }
}

#[test]
fn the_ci_templates_bake_in_the_two_settings_everyone_forgets() {
    // §Distribution: both templates call `cicada run`, bake in the full-history
    // flag, and set a concurrency guard.
    let gitlab = read("ci/gitlab-ci.yml");
    assert!(gitlab.contains("GIT_DEPTH: 0"), "a shallow clone truncates history");
    assert!(gitlab.contains("resource_group"), "overlapping runs race on the force-push");
    assert!(gitlab.contains("cicada run"), "the phase order must not be CI's to get wrong (#35)");

    let workflow = read(".github/workflows/release.yml");
    assert!(workflow.contains("fetch-depth: 0"));
    assert!(workflow.contains("concurrency:"));

    // The action runs `cicada run` by default, for the same reason.
    let action = read("action.yml");
    assert!(action.contains("default: run"), "the action must default to `run`");
}

#[test]
fn the_action_declares_the_inputs_the_readme_tells_people_to_pass() {
    let action = read("action.yml");
    let md = read("README.md");
    for input in ["command", "token", "version", "args"] {
        assert!(action.contains(&format!("  {input}:")), "action.yml has no `{input}` input");
    }
    // The README's GitHub example passes `token:`; if that were renamed the
    // example would silently stop working.
    assert!(md.contains("token: ${{ secrets.CICADA_TOKEN }}"));
}

#[test]
fn the_readme_is_honest_about_what_needs_a_token() {
    // The offline guarantee (§Goals 6) is the claim `check` on a forked pull
    // request rests on, so the table stating it must not quietly disappear.
    let md = read("README.md");
    assert!(md.contains("What needs a token"));
    for cmd in ["`plan`, `check`", "`--dry-run`", "`--no-push`"] {
        assert!(md.contains(cmd), "the token table is missing {cmd}");
    }
}

// ---------------------------------------------------------------------------
// M8's assets: the binaries, the checksums, and the image.
//
// Three files have to agree, and nothing about them is checked by compiling:
// `.github/workflows/artifacts.yml` publishes, `action.yml` downloads, and
// `ci/gitlab-ci.yml` pulls. A rename in any one of them breaks adopters and
// nobody else, which is exactly the rot M7's tests exist to stop — so these
// hold the three to each other rather than each to a comment.

/// The dedented body of the `run: |` block belonging to the step with `id: <id>`.
///
/// Extracting it rather than restating it is the whole point: a test that
/// reimplements the shell it is meant to be checking passes while the workflow
/// is broken.
fn run_block(yaml: &str, id: &str) -> String {
    let at = yaml.find(&format!("id: {id}")).unwrap_or_else(|| panic!("no step has `id: {id}`"));
    let rest = &yaml[at..];
    let run_at = rest.find("run: |").expect("that step has no literal `run:` block");
    let line_start = rest[..run_at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent = run_at - line_start;

    let mut out = String::new();
    for line in rest[run_at..].lines().skip(1) {
        let depth = line.chars().take_while(|c| *c == ' ').count();
        if !line.trim().is_empty() && depth <= indent {
            break;
        }
        out.push_str(line.get(indent + 2..).unwrap_or(""));
        out.push('\n');
    }
    out
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cicada-assets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The Rust target triples the artifact workflow builds.
fn published_targets(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- target: "))
        .map(str::to_string)
        .collect()
}

#[test]
fn the_action_downloads_exactly_the_assets_the_workflow_publishes() {
    let workflow = read(".github/workflows/artifacts.yml");
    let action = read("action.yml");

    let targets = published_targets(&workflow);
    assert_eq!(targets.len(), 2, "§Distribution ships x86_64 and aarch64: {targets:?}");

    for target in &targets {
        // The action builds the name as `cicada-${arch}-unknown-linux-musl`, so
        // both halves have to line up: the architecture it maps `uname -m` onto,
        // and the rest of the triple.
        let (arch, suffix) = target.split_once('-').expect("a target triple");
        assert!(
            action.contains(&format!("arch={arch} ")),
            "action.yml maps no `uname -m` result onto `{arch}`, so it can never \
             download cicada-{target}"
        );
        assert!(
            action.contains(&format!("asset=\"cicada-${{arch}}-{suffix}.tar.gz\"")),
            "action.yml does not build the asset name `cicada-{target}.tar.gz`"
        );
        assert!(
            workflow.contains("cicada-${{ matrix.target }}.tar.gz"),
            "the workflow does not publish a .tar.gz for {target}"
        );
    }
}

#[cfg(unix)]
#[test]
fn the_checksum_file_the_workflow_writes_is_one_the_action_can_verify() {
    // This is the pair that was actually wrong: the action rewrote the filename
    // inside SHA256SUMS with a single-space `sed`, `sha256sum` writes two, and
    // the result was a check that looked for a file named " cicada" and reported
    // it missing. Both sides are executed here, neither is restated.
    let workflow = read(".github/workflows/artifacts.yml");
    let action = read("action.yml");

    let generate = workflow
        .lines()
        .find(|l| l.contains("xargs sha256sum"))
        .expect("the workflow must still generate SHA256SUMS")
        .trim();
    let verify = action
        .lines()
        .find(|l| l.contains("sha256sum --check"))
        .expect("the action must still verify SHA256SUMS")
        .trim();

    let dir = scratch("sha");
    for target in published_targets(&workflow) {
        std::fs::write(
            dir.join(format!("cicada-{target}.tar.gz")),
            format!("pretend archive {target}"),
        )
        .unwrap();
    }

    let sh = |script: &str, dir: &Path| {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(script)
            .current_dir(dir)
            .output()
            .expect("bash")
    };

    let out = sh(generate, &dir);
    assert!(out.status.success(), "generating: {}", String::from_utf8_lossy(&out.stderr));

    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 2, "one line per published binary:\n{sums}");

    // The action downloads one architecture, not all of them, into a directory of
    // its own. That asymmetry is the thing `--ignore-missing` exists for, so the
    // check has to be run the way the action runs it.
    let one = scratch("sha-one");
    let target = &published_targets(&workflow)[0];
    std::fs::copy(
        dir.join(format!("cicada-{target}.tar.gz")),
        one.join(format!("cicada-{target}.tar.gz")),
    )
    .unwrap();
    std::fs::copy(dir.join("SHA256SUMS"), one.join("SHA256SUMS")).unwrap();

    let script = format!("tmp={} ; {verify}", one.display());
    let out = sh(&script, &one);
    assert!(
        out.status.success(),
        "the action cannot verify the file the workflow writes:\nstdout: {}\nstderr: {}\nSHA256SUMS:\n{sums}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // And it must still be a real check: a tampered binary has to fail it.
    std::fs::write(one.join(format!("cicada-{target}.tar.gz")), "tampered").unwrap();
    let out = sh(&script, &one);
    assert!(
        !out.status.success(),
        "a tampered binary passed verification, which makes SHA256SUMS decoration"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&one);
}

#[cfg(unix)]
#[test]
fn moving_image_tags_follow_stable_releases_only() {
    // `ci/gitlab-ci.yml` pins `ghcr.io/boenan/cicada:v1`, so `v1` has to keep
    // meaning "the current v1 release". A prerelease must therefore not move it:
    // a pipeline pinned to `v1` precisely to avoid surprises would silently start
    // running an alpha, and the next stable release would silently fix it.
    let workflow = read(".github/workflows/artifacts.yml");
    let script = run_block(&workflow, "tags");
    let dir = scratch("tags");

    let tags_for = |tag: &str| -> Vec<String> {
        let out_file = dir.join(format!("out-{tag}"));
        std::fs::write(&out_file, "").unwrap();
        let out = std::process::Command::new("bash")
            .arg("-c")
            .arg(&script)
            .current_dir(&dir)
            .env("TAG", tag)
            .env("GITHUB_REPOSITORY", "boenan/cicada")
            .env("GITHUB_OUTPUT", &out_file)
            .output()
            .expect("bash");
        assert!(out.status.success(), "{tag}: {}", String::from_utf8_lossy(&out.stderr));
        std::fs::read_to_string(&out_file)
            .unwrap()
            .lines()
            .find_map(|l| l.strip_prefix("tags=").map(str::to_string))
            .unwrap_or_else(|| panic!("{tag}: the step set no `tags` output"))
            .split(',')
            .map(str::to_string)
            .collect()
    };

    let image = "ghcr.io/boenan/cicada";

    assert_eq!(
        tags_for("v1.2.3"),
        vec![
            format!("{image}:v1.2.3"),
            format!("{image}:v1.2"),
            format!("{image}:v1"),
            format!("{image}:latest"),
        ],
        "a stable release moves every tag above it"
    );

    // 0.x is not special here. cicada's own first release is v0.1.0, and it
    // should still be pullable as `:latest`.
    assert_eq!(
        tags_for("v0.1.0"),
        vec![
            format!("{image}:v0.1.0"),
            format!("{image}:v0.1"),
            format!("{image}:v0"),
            format!("{image}:latest"),
        ],
    );

    assert_eq!(
        tags_for("v1.5.0-alpha.1"),
        vec![format!("{image}:v1.5.0-alpha.1")],
        "a prerelease publishes an image, but moves nothing onto it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_image_the_gitlab_template_pulls_is_the_one_the_workflow_pushes() {
    let gitlab = read("ci/gitlab-ci.yml");
    let config = read("cicada.toml");

    // The template's image name is a literal, and the workflow derives its own
    // from `$GITHUB_REPOSITORY` — so the join between them is the repo cicada is
    // configured to release to. A repo rename breaks the template silently.
    let repo = config
        .lines()
        .find_map(|l| l.trim().strip_prefix("repo = "))
        .expect("cicada.toml must name the forge repo")
        .trim()
        .trim_matches('"')
        .to_lowercase();

    assert!(
        gitlab.contains(&format!("ghcr.io/{repo}:")),
        "ci/gitlab-ci.yml does not pull ghcr.io/{repo}"
    );
}

#[test]
fn the_image_carries_git() {
    // #17: cicada shells out to git for every read and every write, so an image
    // without it is an image where cicada refuses at startup. This is the one
    // property of the Dockerfile that is not a matter of taste.
    let dockerfile = read("ci/Dockerfile");
    assert!(
        dockerfile.contains("apk add --no-cache git"),
        "the image must install git (#17)"
    );
    assert!(
        !dockerfile.contains("FROM scratch"),
        "a static binary makes `scratch` tempting; #17 rules it out"
    );
    // Built once per architecture and copied in, never compiled under emulation.
    assert!(dockerfile.contains("ARG TARGETARCH"));
    assert!(dockerfile.contains("COPY dist/cicada-${TARGETARCH}"));
}

#[test]
fn the_readme_tells_people_to_download_assets_that_exist() {
    // The install instructions are the third consumer of the asset names, and the
    // only one a human follows by hand — so they rot the most visibly and are
    // checked the least.
    let md = read("README.md");
    let workflow = read(".github/workflows/artifacts.yml");
    let config = read("cicada.toml");

    assert!(md.contains("## Installing"), "the README must say how to get cicada");

    for target in published_targets(&workflow) {
        assert!(
            md.contains(&format!("cicada-{target}.tar.gz")),
            "the README's install section does not mention cicada-{target}.tar.gz"
        );
    }
    assert!(md.contains("SHA256SUMS"), "and it must tell people to verify it");

    let repo = config
        .lines()
        .find_map(|l| l.trim().strip_prefix("repo = "))
        .unwrap()
        .trim()
        .trim_matches('"')
        .to_lowercase();
    assert!(
        md.contains(&format!("ghcr.io/{repo}:")),
        "the README does not pull the image this repo publishes"
    );

    // The `-u` in the documented `docker run` is load-bearing: without it git
    // refuses the bind-mounted checkout as dubiously owned, and the obvious
    // in-image fix is dead on arrival because cicada sets GIT_CONFIG_NOSYSTEM=1.
    let at = md.find("docker run").expect("a worked docker invocation");
    let line_end = md[at..].find('\n').map(|i| at + i).unwrap_or(md.len());
    assert!(
        md[at..line_end].contains("-u "),
        "the documented `docker run` drops `-u`, so it fails on a bind-mounted repo:\n  {}",
        &md[at..line_end]
    );
}

#[test]
fn the_checkout_does_not_hand_cicada_someone_elses_credentials() {
    // `actions/checkout` defaults to persisting GITHUB_TOKEN into .git/config as
    // an `http.<url>.extraheader`, and git matches that config against cicada's
    // push URL — the username cicada puts in the URL does not prevent it, since a
    // config URL carrying no username matches any username. The push then
    // authenticates as GITHUB_TOKEN rather than as the configured PAT.
    //
    // It still succeeds. That is what makes it worth a test: the tag lands, and
    // then no workflow reacts to it, because events created by GITHUB_TOKEN do
    // not start workflow runs. The release ends up with no binaries on it and
    // nothing anywhere says why.
    for asset in [".github/workflows/release.yml", "README.md"] {
        let src = read(asset);
        assert!(
            src.contains("persist-credentials: false"),
            "{asset} checks out without `persist-credentials: false`, so GITHUB_TOKEN \
             would be used for cicada's push and the tag would trigger nothing"
        );
    }
}

#[test]
fn every_gh_step_knows_which_repository_it_is_talking_about() {
    // `gh` infers the repository from the git remote, and the job that uploads
    // release assets has no checkout — nothing to infer from. Without `GH_REPO`
    // every call fails with "failed to determine base repo", which the retry loop
    // around it then reports as a five-minute timeout waiting for a release that
    // was there the whole time.
    //
    // The invariant is a pairing: a step that needs `GH_TOKEN` is a step running
    // `gh`, and every such step needs `GH_REPO` beside it.
    let workflow = read(".github/workflows/artifacts.yml");
    let tokens = workflow.matches("GH_TOKEN:").count();
    let repos = workflow.matches("GH_REPO:").count();

    assert!(tokens > 0, "the workflow no longer authenticates gh at all");
    assert_eq!(
        tokens, repos,
        "{tokens} step(s) set GH_TOKEN but only {repos} set GH_REPO — a gh step \
         without GH_REPO fails to find the repository, and fails slowly"
    );
}

#[cfg(unix)]
#[test]
fn the_executable_bit_survives_the_trip_to_a_users_disk() {
    // The reason the assets are archives at all. A GitHub release asset carries
    // no permissions, and actions/upload-artifact does not preserve them either —
    // so a bare binary published that way arrives non-executable and has to be
    // chmod-ed by whoever downloads it. That worked for the action, which chmods,
    // and failed for every human following the README.
    //
    // The workflow's whole packaging step runs here, rather than two lines picked
    // out of it. The first version of this test picked lines and ran them in an
    // empty scratch directory — and passed while the real step was broken, because
    // the repository root holds a `cicada` **directory**: `install … cicada` wrote
    // inside it, tar archived the source tree, and the step died with
    // "./cicada: Is a directory". The hazard was absent from the only place it was
    // being tested.
    //
    // It is reproduced below.
    let workflow = read(".github/workflows/artifacts.yml");
    let action = read("action.yml");
    let target = &published_targets(&workflow)[0];

    let package = run_block(&workflow, "package").replace("${{ matrix.target }}", target);
    let unpack = action
        .lines()
        .find(|l| l.contains("tar xzf"))
        .expect("the action must still extract the archive")
        .trim()
        .to_string();

    let src = scratch("tar-src");
    let built = src.join(format!("target/{target}/release"));
    std::fs::create_dir_all(&built).unwrap();
    std::fs::write(built.join("cicada"), "#!/bin/sh\necho 'cicada 9.9.9'\n").unwrap();

    // A directory named exactly like the binary, as the real repo root has.
    std::fs::create_dir_all(src.join("cicada/src")).unwrap();
    std::fs::write(src.join("cicada/src/main.rs"), "fn main() {}\n").unwrap();

    let sh = |script: &str, dir: &Path| {
        std::process::Command::new("bash")
            .arg("-c")
            .arg(script)
            .current_dir(dir)
            .output()
            .expect("bash")
    };

    let out = sh(&package, &src);
    assert!(
        out.status.success(),
        "packaging failed in a tree shaped like this repo:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let asset = format!("cicada-{target}.tar.gz");
    let dst = scratch("tar-dst");
    std::fs::copy(src.join(&asset), dst.join(&asset))
        .unwrap_or_else(|e| panic!("the workflow produced no {asset}: {e}"));

    let script = format!("tmp={} ; asset={asset} ; {unpack}", dst.display());
    let out = sh(&script, &dst);
    assert!(out.status.success(), "unpacking: {}", String::from_utf8_lossy(&out.stderr));

    let meta = std::fs::metadata(dst.join("cicada")).expect("the archive must hold a `cicada`");
    assert!(
        meta.is_file(),
        "the archive's `cicada` is a directory — the source tree got packaged \
         instead of the binary, which is what shipping from the repo root does"
    );

    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "the extracted cicada is not executable (mode {mode:o}) — the archive did not \
         carry the bit, which is the only reason it is an archive"
    );

    // Flat, no wrapping directory: `action.yml` and the README both extract
    // straight into place and expect `cicada` at the root.
    let out = sh(&format!("tar tzf {asset}"), &dst);
    let listing = String::from_utf8_lossy(&out.stdout);
    assert_eq!(listing.trim(), "cicada", "exactly one flat entry:\n{listing}");

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

/// The major version this repo currently builds, from the workspace manifest.
fn current_major() -> u64 {
    let toml = read("Cargo.toml");
    let v = toml
        .lines()
        .find_map(|l| l.trim().strip_prefix("version = "))
        .expect("the workspace must declare a version")
        .trim()
        .trim_matches('"')
        .to_string();
    v.split('.').next().unwrap().parse().expect("a numeric major")
}

#[test]
fn the_docs_never_point_at_a_major_that_does_not_exist() {
    // The README said `uses: boenan/cicada@v1` and the GitLab template pinned
    // `ghcr.io/boenan/cicada:v1` from the day they were written — while the repo
    // was, and still is, on 0.x. Nothing publishes a `v1` of anything, so both
    // instructions 404 for the adopter who follows them first.
    //
    // The rule is one-sided on purpose: **documented major must not be ahead of
    // the released major**. Requiring equality would fail inside the release PR
    // that first bumps to 1.0.0 — a generated PR that cannot be hand-edited
    // without tripping the staleness guard (#32) — and would block the major
    // release it was meant to keep honest.
    let major = current_major();
    let readme = read("README.md");
    let gitlab = read("ci/gitlab-ci.yml");

    let refs = |src: &str, pat: &str| -> Vec<u64> {
        src.match_indices(pat)
            .filter_map(|(i, _)| {
                src[i + pat.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect()
    };

    let mut seen = 0;
    for (name, found) in [
        ("README `uses:`", refs(&readme, "boenan/cicada@v")),
        ("README image", refs(&readme, "ghcr.io/boenan/cicada:v")),
        ("GitLab image", refs(&gitlab, "ghcr.io/boenan/cicada:v")),
    ] {
        for r in &found {
            seen += 1;
            assert!(
                *r <= major,
                "{name} points at v{r}, but this repo is on {major}.x — nothing \
                 publishes a v{r}, so that instruction 404s"
            );
        }
    }
    assert!(seen >= 3, "only {seen} version-pinned references found; did they move?");
}

#[test]
fn the_moving_major_tag_the_readme_relies_on_is_published() {
    // `uses: boenan/cicada@v0` resolves against a git ref, and cicada creates
    // only exact-version tags (#2) — so if this workflow stops pushing them, the
    // README's install line quietly stops resolving.
    let workflow = read(".github/workflows/artifacts.yml");
    assert!(
        workflow.contains(r#"git tag -f "v${major}""#),
        "nothing publishes the moving major tag that `uses: …@v0` needs"
    );
    assert!(
        workflow.contains("git push -f origin"),
        "the moving tags are created but never pushed"
    );
    // And it must not be able to trigger itself: a `v*` filter would let the
    // moving tags back in, with TAG=v0 failing the version check every time.
    assert!(
        !workflow.contains("tags: ['v*']"),
        "the tag filter would let the moving tags re-enter this workflow"
    );
}

#[test]
fn ci_builds_locked_so_a_stale_lockfile_fails_before_the_tag() {
    // The release commit bumps Cargo.toml *and* Cargo.lock, and the only thing
    // that proves they agree is a build with `--locked`. Without the flag cargo
    // silently rewrites the lock in the runner and passes — which is exactly how
    // v0.2.0 reached a tag with a stale lock, and then failed in the artifact
    // build, where `--locked` is used and where failure is far more expensive.
    // The rule is about commands that *resolve* dependencies. `cargo audit` reads
    // the lockfile as its input and takes no such flag, which is the whole point
    // of it — so the check is per-subcommand rather than "every line saying
    // cargo", or it fails on the one job that is already doing the right thing.
    const RESOLVES: [&str; 5] = ["build", "test", "check", "clippy", "run"];

    let ci = read(".github/workflows/ci.yml");
    let mut seen = 0;
    for line in ci.lines().map(str::trim) {
        let Some(cmd) = line.strip_prefix("- run: cargo ") else { continue };
        let Some(sub) = cmd.split_whitespace().next() else { continue };
        if !RESOLVES.contains(&sub) {
            continue;
        }
        seen += 1;
        assert!(
            cmd.contains("--locked"),
            "`cargo {cmd}` does not pass --locked, so a release commit whose \
             lockfile was left behind passes CI and fails after the tag"
        );
    }
    assert!(seen >= 3, "only {seen} dependency-resolving cargo commands in ci.yml");
}

#[test]
fn the_msrv_is_built_rather_than_asserted() {
    // `rust-version` is a promise to anyone running `cargo install cicada`, and
    // it was wrong for several milestones: it said 1.85 while the code had been
    // using let-chains, stable in 1.88. The install failed with "`let`
    // expressions in this position are unstable" instead of cargo's clean
    // "requires rustc 1.88".
    //
    // So the manifest and the toolchain CI actually builds with have to agree.
    let manifest = read("Cargo.toml");
    let declared = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("rust-version = "))
        .expect("the workspace must declare rust-version")
        .trim()
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap()
        .to_string();

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains(&format!("dtolnay/rust-toolchain@{declared}")),
        "Cargo.toml promises rust {declared}, but no CI job builds with that \
         toolchain — the promise is unverified again"
    );
}

#[test]
fn every_job_that_builds_states_which_rust_it_builds_with() {
    // A job with no toolchain step runs whatever Rust the runner image happens
    // to ship. That lags stable by weeks, and it changes underneath the repo
    // with no commit to point at — so "which compiler produced this binary?" has
    // no answer, and a toolchain regression looks like a code regression.
    //
    // `rust-version` is the floor and is checked elsewhere; this is the ceiling,
    // and every build wants to be on current stable.
    for asset in [
        ".github/workflows/ci.yml",
        ".github/workflows/artifacts.yml",
        ".github/workflows/release.yml",
    ] {
        let src = read(asset);
        let builds = src
            .lines()
            .map(str::trim)
            .filter(|l| l.contains("cargo build") || l.contains("cargo test"))
            .count();
        if builds == 0 {
            continue;
        }
        assert!(
            src.contains("dtolnay/rust-toolchain@"),
            "{asset} builds with cargo but pins no toolchain, so it uses whatever \
             the runner image ships"
        );
    }

    // And the musl target comes from the same pin rather than a separate rustup
    // call that could name a different toolchain's target set.
    let artifacts = read(".github/workflows/artifacts.yml");
    assert!(
        !artifacts.contains("rustup target add"),
        "the cross target should come from the toolchain action's `targets:`"
    );
}

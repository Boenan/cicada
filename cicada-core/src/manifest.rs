//! The release manifest (#42, #57).
//!
//! **One per run, not per package.** A single run can release three packages
//! independently; what a deployment needs is the set that shipped *together* —
//! package, version, tag, the tagged commit, and the date the run used.
//!
//! It goes to two places, and both are load-bearing:
//!
//! * `target/cicada/release-manifest.json` on disk (gitignored, **never
//!   committed**), because under `--no-push` there is no release object at all
//!   and the file is the only way to inspect a rehearsal (§Goals 6) — and because
//!   the deploy job that runs after the release job can read a file in the
//!   workspace without a token and an API round trip.
//! * embedded in **every** release object the run creates.
//!
//! **Every release, not the umbrella's alone.** An earlier draft attached it to
//! the umbrella, which quietly assumed every repo has one — `[umbrella]` is
//! optional (#73), so a two-package repo with no umbrella would have left the
//! manifest with nowhere to go. Embedding it in each release also answers the
//! question a reader of any one release actually has — *what else shipped with
//! this?* — without them needing to know which release is the summary. The cost
//! is a collapsed `<details>` block repeated N times, which is cheap next to a
//! rule with an exception in it.

/// One released artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub package: String,
    pub version: String,
    pub tag: String,
    /// The commit the tag points at.
    pub sha: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The date the run used — an input, never a clock read (#95), so a manifest
    /// is reproducible from the run that produced it.
    pub date: String,
    /// Config order, umbrella last (#96) — the umbrella's embedded copy therefore
    /// describes work already done rather than work in flight.
    pub releases: Vec<Entry>,
}

impl Manifest {
    /// Pretty-printed, because a human reads this inside a release body as often
    /// as a machine reads it off disk.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n  \"schema\": 1,\n");
        s.push_str(&format!("  \"date\": {},\n", quote(&self.date)));
        s.push_str("  \"releases\": [\n");
        for (i, r) in self.releases.iter().enumerate() {
            s.push_str("    {\n");
            s.push_str(&format!("      \"package\": {},\n", quote(&r.package)));
            s.push_str(&format!("      \"version\": {},\n", quote(&r.version)));
            s.push_str(&format!("      \"tag\": {},\n", quote(&r.tag)));
            s.push_str(&format!("      \"sha\": {}\n", quote(&r.sha)));
            s.push_str(if i + 1 == self.releases.len() { "    }\n" } else { "    },\n" });
        }
        s.push_str("  ]\n}\n");
        s
    }

    /// The block appended to a release body.
    ///
    /// Collapsed, so it does not bury the notes; fenced JSON, so it stays
    /// greppable by a human reading the release. **No asset upload** (#57):
    /// GitHub releases host files and GitLab releases do not, so "attach the
    /// manifest" is two unrelated operations rather than one, and per-forge asset
    /// upload is future work (§2).
    pub fn to_details_block(&self) -> String {
        format!(
            "<details>\n<summary>Release manifest</summary>\n\n```json\n{}```\n\n</details>",
            self.to_json()
        )
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            date: "2026-09-21".into(),
            releases: vec![
                Entry {
                    package: "api".into(),
                    version: "1.5.0".into(),
                    tag: "api-v1.5.0".into(),
                    sha: "abc123".into(),
                },
                Entry {
                    package: "platform".into(),
                    version: "1.8.0".into(),
                    tag: "platform-v1.8.0".into(),
                    sha: "abc123".into(),
                },
            ],
        }
    }

    #[test]
    fn the_manifest_is_valid_json_with_the_documented_shape() {
        let json = sample().to_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}\n{json}"));
        assert_eq!(v["schema"], 1);
        assert_eq!(v["date"], "2026-09-21");
        assert_eq!(v["releases"][0]["package"], "api");
        assert_eq!(v["releases"][1]["tag"], "platform-v1.8.0");
        assert_eq!(v["releases"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_run_that_released_one_package_still_produces_a_list() {
        // A consumer parses `releases` as an array whatever the repo shape, so a
        // single-service repo must not produce a different document.
        let m = Manifest { date: "2026-09-21".into(), releases: vec![sample().releases[0].clone()] };
        let v: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
        assert!(v["releases"].is_array());
        assert_eq!(v["releases"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_manifest_is_still_parseable() {
        let m = Manifest { date: "2026-09-21".into(), releases: vec![] };
        let v: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
        assert_eq!(v["releases"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn the_embedded_block_is_collapsed_and_the_json_survives_extraction() {
        let block = sample().to_details_block();
        assert!(block.starts_with("<details>"), "{block}");
        assert!(block.contains("<summary>Release manifest</summary>"));
        // Greppable: a reader — or a script — can lift the JSON back out.
        let json = block.split("```json").nth(1).unwrap().split("```").next().unwrap();
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["releases"][0]["version"], "1.5.0");
    }

    #[test]
    fn a_package_name_with_a_quote_in_it_cannot_break_the_document() {
        let m = Manifest {
            date: "2026-09-21".into(),
            releases: vec![Entry {
                package: "we\"ird".into(),
                version: "1.0.0".into(),
                tag: "t".into(),
                sha: "s".into(),
            }],
        };
        let v: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
        assert_eq!(v["releases"][0]["package"], "we\"ird");
    }
}

use saphyr_parser::ScalarStyle;
use spike_yaml::{locate, splice, verify, Error};

const CHART: &str = r#"apiVersion: v2
name: api
description: The API service   # keep me

# the chart version, bumped by cicada
version: 0.9.0
appVersion: "1.4.2"
"#;

#[test]
fn top_level_scalar_preserves_everything_else() {
    let out = splice(CHART, "version", "0.9.1").unwrap();
    assert_eq!(
        out,
        r#"apiVersion: v2
name: api
description: The API service   # keep me

# the chart version, bumped by cicada
version: 0.9.1
appVersion: "1.4.2"
"#
    );
    verify(&out).unwrap();
}

#[test]
fn quoting_style_is_preserved() {
    let out = splice(CHART, "appVersion", "1.5.0").unwrap();
    assert!(out.contains("appVersion: \"1.5.0\""), "{out}");
    let out2 = splice(CHART, "version", "1.0.0").unwrap();
    assert!(out2.contains("version: 1.0.0"), "{out2}");
}

#[test]
fn style_is_reported_so_helm_can_warn_on_unquoted_appversion() {
    // cicada decision #51
    let unquoted = "name: api\nappVersion: 1.10\n";
    let loc = locate(unquoted, "appVersion").unwrap();
    assert_eq!(loc.style, ScalarStyle::Plain);
    assert_eq!(loc.value, "1.10");

    let quoted = locate(CHART, "appVersion").unwrap();
    assert_eq!(quoted.style, ScalarStyle::DoubleQuoted);
}

const VALUES: &str = r#"replicaCount: 2

image:
  repository: registry.example.com/api   # internal registry
  tag: 1.4.2   # x-cicada-version
  pullPolicy: IfNotPresent

resources: {}
"#;

#[test]
fn nested_path_with_trailing_comment() {
    let out = splice(VALUES, "image.tag", "1.5.0").unwrap();
    assert!(out.contains("tag: 1.5.0   # x-cicada-version"), "{out}");
    assert!(out.contains("repository: registry.example.com/api   # internal registry"));
    assert_eq!(out.lines().count(), VALUES.lines().count());
    verify(&out).unwrap();
}

#[test]
fn non_ascii_before_target_proves_byte_indexing() {
    // The struct field is documented as "index (in chars)" while the accessor says
    // "in bytes". If it is chars, this splice lands in the wrong place.
    let y = "# naïve — bumped by cicada ✅\nversion: 1.0.0\n";
    let out = splice(y, "version", "2.0.0").unwrap();
    assert_eq!(out, "# naïve — bumped by cicada ✅\nversion: 2.0.0\n", "{out:?}");
}

#[test]
fn non_ascii_in_an_earlier_value() {
    let y = "name: café ☕\nversion: 1.0.0\n";
    let out = splice(y, "version", "2.0.0").unwrap();
    assert_eq!(out, "name: café ☕\nversion: 2.0.0\n", "{out:?}");
}

#[test]
fn crlf_line_endings_survive() {
    let y = "name: api\r\nversion: 1.0.0\r\nappVersion: \"1.0.0\"\r\n";
    let out = splice(y, "version", "1.1.0").unwrap();
    assert_eq!(out, "name: api\r\nversion: 1.1.0\r\nappVersion: \"1.0.0\"\r\n", "{out:?}");
}

#[test]
fn sequence_index_paths() {
    let y = r#"spec:
  containers:
    - name: api
      image: api:1.4.2
    - name: sidecar
      image: proxy:0.1.0
"#;
    let out = splice(y, "spec.containers.0.image", "api:1.5.0").unwrap();
    assert!(out.contains("image: api:1.5.0"), "{out}");
    assert!(out.contains("image: proxy:0.1.0"), "{out}");
}

#[test]
fn flow_mapping_works_too() {
    let y = "image: {repository: api, tag: 1.4.2}\n";
    let out = splice(y, "image.tag", "1.5.0").unwrap();
    assert_eq!(out, "image: {repository: api, tag: 1.5.0}\n", "{out:?}");
}

#[test]
fn multi_document_only_touches_the_first() {
    let y = "version: 1.0.0\n---\nversion: 9.9.9\n";
    let out = splice(y, "version", "1.1.0").unwrap();
    assert_eq!(out, "version: 1.1.0\n---\nversion: 9.9.9\n", "{out:?}");
}

#[test]
fn alias_at_target_is_refused_not_silently_wrong() {
    let y = "defaults: &d 1.0.0\nversion: *d\n";
    let err = splice(y, "version", "2.0.0").unwrap_err();
    assert_eq!(err, Error::Unsupported("alias at addressed path"), "{err:?}");
}

#[test]
fn missing_path_is_an_error() {
    assert_eq!(
        splice(CHART, "nope.nothere", "1.0.0").unwrap_err(),
        Error::NotFound("nope.nothere".into())
    );
}

#[test]
fn idempotent_and_repeatable() {
    // cicada goal 7
    let once = splice(CHART, "version", "0.9.1").unwrap();
    let again = splice(&once, "version", "0.9.1").unwrap();
    assert_eq!(once, again);
    let twice = splice(&once, "version", "0.9.2").unwrap();
    let direct = splice(CHART, "version", "0.9.2").unwrap();
    assert_eq!(twice, direct);
}

#[test]
fn block_scalar_is_refused() {
    let y = "notes: |\n  line one\n  line two\n";
    assert_eq!(
        splice(y, "notes", "x").unwrap_err(),
        Error::Unsupported("block scalar at addressed path")
    );
}

#[test]
fn anchored_value_still_splices() {
    let y = "version: &v 1.0.0\nother: *v\n";
    let out = splice(y, "version", "2.0.0").unwrap();
    assert!(out.starts_with("version: &v 2.0.0"), "{out:?}");
}

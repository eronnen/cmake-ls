use std::fs;
use std::path::Path;

use tempfile::tempdir;

use crate::cancellation::Cancellation;

use super::{BuildTree, Version, resolve_reference, validate_version};

#[test]
fn accepts_codemodel_v2() {
    let result = validate_version(
        Path::new("codemodel.json"),
        "codemodel",
        &Version { major: 2, minor: 9 },
        "codemodel",
        2,
    );

    assert!(result.is_ok());
}

#[test]
fn rejects_other_object_kinds() {
    let result = validate_version(
        Path::new("cache.json"),
        "cache",
        &Version { major: 2, minor: 0 },
        "codemodel",
        2,
    );

    assert!(result.is_err());
}

#[test]
fn rejects_other_codemodel_versions() {
    let result = validate_version(
        Path::new("codemodel.json"),
        "codemodel",
        &Version { major: 3, minor: 0 },
        "codemodel",
        2,
    );

    assert!(result.is_err());
}

#[test]
fn confines_references_to_the_reply_directory() {
    let reply_dir = Path::new("/tmp/reply");

    assert!(resolve_reference(reply_dir, Path::new("codemodel.json")).is_ok());
    assert!(resolve_reference(reply_dir, Path::new("nested/codemodel.json")).is_ok());
    assert!(resolve_reference(reply_dir, Path::new("../codemodel.json")).is_err());
    assert!(resolve_reference(reply_dir, Path::new("/tmp/codemodel.json")).is_err());
    assert!(resolve_reference(reply_dir, Path::new("")).is_err());
}

#[test]
fn reads_sorts_and_deduplicates_targets_across_configurations() {
    let build_dir = tempdir().expect("create temporary build directory");
    write_reply(
        build_dir.path(),
        r#"{
            "kind": "codemodel",
            "version": { "major": 2, "minor": 9 },
            "configurations": [
                {
                    "name": "Debug",
                    "targets": [
                        { "name": "zeta" },
                        { "name": "alpha" }
                    ]
                },
                {
                    "name": "Release",
                    "targets": [
                        { "name": "alpha" },
                        { "name": "middle" }
                    ]
                }
            ],
            "futureField": true
        }"#,
    );

    let targets = BuildTree::new(build_dir.path())
        .read_targets(&Cancellation::default())
        .expect("read target names");
    let names: Vec<_> = targets.into_iter().collect();

    assert_eq!(names, ["alpha", "middle", "zeta"]);
}

#[test]
fn accepts_an_empty_target_list() {
    let build_dir = tempdir().expect("create temporary build directory");
    write_reply(
        build_dir.path(),
        r#"{
            "kind": "codemodel",
            "version": { "major": 2, "minor": 0 },
            "configurations": [{ "name": "", "targets": [] }]
        }"#,
    );

    let targets = BuildTree::new(build_dir.path())
        .read_targets(&Cancellation::default())
        .expect("read target names");

    assert!(targets.is_empty());
}

#[test]
fn rejects_a_missing_client_response() {
    let build_dir = tempdir().expect("create temporary build directory");
    let reply_dir = build_dir.path().join(".cmake/api/v1/reply");
    fs::create_dir_all(&reply_dir).expect("create reply directory");
    fs::write(reply_dir.join("index-test.json"), r#"{ "reply": {} }"#).expect("write index");

    let error = BuildTree::new(build_dir.path())
        .read_targets(&Cancellation::default())
        .expect_err("reject missing response");

    assert!(error.to_string().contains("client-cmake-ls"));
}

#[test]
fn reports_malformed_json() {
    let build_dir = tempdir().expect("create temporary build directory");
    let reply_dir = build_dir.path().join(".cmake/api/v1/reply");
    fs::create_dir_all(&reply_dir).expect("create reply directory");
    fs::write(reply_dir.join("index-test.json"), "{invalid").expect("write index");

    let error = BuildTree::new(build_dir.path())
        .read_targets(&Cancellation::default())
        .expect_err("reject malformed JSON");

    assert!(error.to_string().contains("failed to parse"));
}

fn write_reply(build_dir: &Path, codemodel: &str) {
    let reply_dir = build_dir.join(".cmake/api/v1/reply");
    fs::create_dir_all(&reply_dir).expect("create reply directory");
    fs::write(
        reply_dir.join("index-test.json"),
        r#"{
            "reply": {
                "client-cmake-ls": {
                    "codemodel-v2": {
                        "kind": "codemodel",
                        "version": { "major": 2, "minor": 9 },
                        "jsonFile": "codemodel-test.json"
                    }
                }
            }
        }"#,
    )
    .expect("write index");
    fs::write(reply_dir.join("codemodel-test.json"), codemodel).expect("write codemodel");
}

use std::process::Stdio;

use super::support::{TestProject, assert_project_targets, assert_success};

#[test]
fn lists_buildable_targets_from_the_default_build_directory() {
    let project = TestProject::configured();
    let debug_build = project.configure_additional_build("build/debug");
    let release_build = project.configure_additional_build("build/release");

    let output = project.run_from_default_build_directory();

    assert_project_targets(&output);
    assert!(output.stderr.is_empty());
    assert!(
        project
            .build_path(".cmake/api/v1/query/client-cmake-ls/codemodel-v2")
            .is_file()
    );
    assert!(
        project
            .build_path(".cmake/api/v1/query/client-cmake-ls/cmakeFiles-v1")
            .is_file()
    );
    assert!(
        !debug_build
            .join(".cmake/api/v1/query/client-cmake-ls")
            .exists()
    );
    assert!(
        !release_build
            .join(".cmake/api/v1/query/client-cmake-ls")
            .exists()
    );
}

#[test]
fn falls_back_to_the_debug_build_directory_before_release() {
    let project = TestProject::configured_at_default_path("build/debug");
    let release_build = project.configure_additional_build("build/release");

    let output = project.run_from_default_build_directory();

    assert_project_targets(&output);
    assert!(
        project
            .build_path(".cmake/api/v1/query/client-cmake-ls")
            .is_dir()
    );
    assert!(
        !release_build
            .join(".cmake/api/v1/query/client-cmake-ls")
            .exists()
    );
}

#[test]
fn falls_back_to_the_release_build_directory() {
    let project = TestProject::configured_at_default_path("build/release");

    let output = project.run_from_default_build_directory();

    assert_project_targets(&output);
    assert!(
        project
            .build_path(".cmake/api/v1/query/client-cmake-ls")
            .is_dir()
    );
}

#[test]
fn accepts_an_explicit_build_directory_with_spaces() {
    let project = TestProject::configured_with_spaces();

    let output = project.run();

    assert_project_targets(&output);
}

#[test]
fn treats_a_closed_stdout_pipe_as_normal_termination() {
    let project = TestProject::configured();
    let mut child = project
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    drop(child.stdout.take().expect("take stdout pipe"));

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn reuses_a_current_file_api_reply_without_running_cmake() {
    let project = TestProject::configured();
    let initial_output = project.run();
    assert_project_targets(&initial_output);

    let output = project
        .command()
        .env("PATH", "")
        .output()
        .expect("run cmake-ls without CMake on PATH");

    assert_success(&output);
    assert_eq!(output.stdout, initial_output.stdout);
    assert!(output.stderr.is_empty());
}

#[test]
fn refresh_regenerates_even_when_the_file_api_reply_is_current() {
    let project = TestProject::configured();
    assert_success(&project.run());

    let output = project
        .command()
        .arg("--refresh")
        .env("PATH", "")
        .output()
        .expect("run cmake-ls without CMake on PATH");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to run `cmake`"));
}

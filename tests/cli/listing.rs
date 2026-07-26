use std::process::Stdio;

use super::support::{TARGET_LIST, TestProject, assert_success};

#[test]
fn lists_buildable_targets_from_the_default_build_directory() {
    let project = TestProject::configured();

    let output = project.run_from_default_build_directory();

    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), TARGET_LIST);
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
}

#[test]
fn accepts_an_explicit_build_directory_with_spaces() {
    let project = TestProject::configured_with_spaces();

    let output = project.run();

    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), TARGET_LIST);
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
    assert_success(&project.run());

    let output = project
        .command()
        .env("PATH", "")
        .output()
        .expect("run cmake-ls without CMake on PATH");

    assert_success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout), TARGET_LIST);
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

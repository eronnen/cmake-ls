//! End-to-end command-line tests using real `CMake` build trees.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use tempfile::tempdir;

const PROJECT: &str = r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_fixture C)

add_executable(app main.c)
add_library(core STATIC core.c)
add_custom_target(generate)
add_library(headers INTERFACE)
";

#[test]
fn lists_buildable_targets_from_the_default_build_directory() {
    let temporary = tempdir().expect("create temporary directory");
    let source_dir = temporary.path().join("source");
    let build_dir = temporary.path().join("build");
    create_project(&source_dir);
    configure_project(&source_dir, &build_dir);

    let output = cmake_ls()
        .current_dir(temporary.path())
        .output()
        .expect("run cmake-ls");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "app\ncore\ngenerate\n"
    );
    assert!(output.stderr.is_empty());
    assert!(
        build_dir
            .join(".cmake/api/v1/query/client-cmake-ls/codemodel-v2")
            .is_file()
    );
}

#[test]
fn accepts_an_explicit_build_directory_with_spaces() {
    let temporary = tempdir().expect("create temporary directory");
    let source_dir = temporary.path().join("source tree");
    let build_dir = temporary.path().join("build tree");
    create_project(&source_dir);
    configure_project(&source_dir, &build_dir);

    let output = cmake_ls().arg(&build_dir).output().expect("run cmake-ls");

    assert_success(&output);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "app\ncore\ngenerate\n"
    );
}

#[test]
fn rejects_an_unconfigured_directory_without_creating_a_query() {
    let temporary = tempdir().expect("create temporary directory");
    let build_dir = temporary.path().join("unconfigured");
    fs::create_dir(&build_dir).expect("create unconfigured directory");

    let output = cmake_ls().arg(&build_dir).output().expect("run cmake-ls");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("CMakeCache.txt is missing; configure the build tree first")
    );
    assert!(!build_dir.join(".cmake").exists());
}

#[test]
fn reports_cmake_regeneration_failures() {
    let temporary = tempdir().expect("create temporary directory");
    let source_dir = temporary.path().join("source");
    let build_dir = temporary.path().join("build");
    create_project(&source_dir);
    configure_project(&source_dir, &build_dir);
    fs::write(source_dir.join("CMakeLists.txt"), "not_a_cmake_command()\n")
        .expect("replace project file");

    let output = cmake_ls().arg(&build_dir).output().expect("run cmake-ls");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr.contains("CMake configuration failed"));
    assert!(stderr.contains("not_a_cmake_command"));
}

#[test]
fn treats_a_closed_stdout_pipe_as_normal_termination() {
    let temporary = tempdir().expect("create temporary directory");
    let source_dir = temporary.path().join("source");
    let build_dir = temporary.path().join("build");
    create_project(&source_dir);
    configure_project(&source_dir, &build_dir);

    let mut child = cmake_ls()
        .arg(&build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    drop(child.stdout.take().expect("take stdout pipe"));

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn ctrl_c_interrupts_an_active_cmake_process_group() {
    let temporary = tempdir().expect("create temporary directory");
    let source_dir = temporary.path().join("source");
    let build_dir = temporary.path().join("build");
    create_project(&source_dir);
    configure_project(&source_dir, &build_dir);
    fs::write(
        source_dir.join("CMakeLists.txt"),
        r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_interrupt NONE)
execute_process(COMMAND ${CMAKE_COMMAND} -E touch
                ${CMAKE_BINARY_DIR}/cmake-ls-sleeping)
execute_process(COMMAND ${CMAKE_COMMAND} -E sleep 30)
add_custom_target(waited)
",
    )
    .expect("replace project file");

    let started = Instant::now();
    let child = cmake_ls()
        .arg(&build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    wait_for_path(&build_dir.join("cmake-ls-sleeping"), Duration::from_secs(5));

    let signal_status = Command::new("kill")
        .arg("-INT")
        .arg(child.id().to_string())
        .status()
        .expect("send SIGINT");
    assert!(signal_status.success());

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn cmake_ls() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cmake-ls"))
}

fn create_project(source_dir: &Path) {
    fs::create_dir_all(source_dir).expect("create source directory");
    fs::write(source_dir.join("CMakeLists.txt"), PROJECT).expect("write project file");
    fs::write(source_dir.join("main.c"), "int main(void) { return 0; }\n")
        .expect("write executable source");
    fs::write(source_dir.join("core.c"), "int core(void) { return 0; }\n")
        .expect("write library source");
}

fn configure_project(source_dir: &Path, build_dir: &Path) {
    let output = Command::new("cmake")
        .arg("-S")
        .arg(source_dir)
        .arg("-B")
        .arg(build_dir)
        .output()
        .expect("run CMake");

    assert!(
        output.status.success(),
        "CMake failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "cmake-ls failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn wait_for_path(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;

    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for `{}`",
            path.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::support::{TestProject, wait_for_path};

#[test]
fn downstream_pipe_closure_interrupts_active_cmake() {
    let project = TestProject::configured();
    project.replace_cmake_lists(
        r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_closed_pipe NONE)
execute_process(COMMAND ${CMAKE_COMMAND} -E touch
                ${CMAKE_BINARY_DIR}/cmake-ls-sleeping)
execute_process(COMMAND ${CMAKE_COMMAND} -E sleep 30)
add_custom_target(waited)
",
    );

    let started = Instant::now();
    let mut child = project
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    wait_for_path(
        &project.build_path("cmake-ls-sleeping"),
        Duration::from_secs(5),
    );
    drop(child.stdout.take().expect("take stdout pipe"));

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn ctrl_c_interrupts_an_active_cmake_process_group() {
    let project = TestProject::configured();
    project.replace_cmake_lists(
        r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_interrupt NONE)
execute_process(COMMAND ${CMAKE_COMMAND} -E touch
                ${CMAKE_BINARY_DIR}/cmake-ls-sleeping)
execute_process(COMMAND ${CMAKE_COMMAND} -E sleep 30)
add_custom_target(waited)
",
    );

    let started = Instant::now();
    let child = project
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    wait_for_path(
        &project.build_path("cmake-ls-sleeping"),
        Duration::from_secs(5),
    );
    send_interrupt(&child);

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn ctrl_c_cleans_up_descendants_that_keep_output_open() {
    let project = TestProject::configured();
    project.write_source_file(
        "ignore-interrupt.sh",
        r#"trap '' INT
sleep 30 &
trap - INT
touch "$1"
sleep 30
"#,
    );
    project.replace_cmake_lists(
        r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_descendant_interrupt NONE)
execute_process(
  COMMAND /bin/sh ${CMAKE_SOURCE_DIR}/ignore-interrupt.sh
          ${CMAKE_BINARY_DIR}/cmake-ls-descendant-sleeping
)
add_custom_target(waited)
",
    );

    let started = Instant::now();
    let child = project
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cmake-ls");
    wait_for_path(
        &project.build_path("cmake-ls-descendant-sleeping"),
        Duration::from_secs(5),
    );
    send_interrupt(&child);

    let output = child.wait_with_output().expect("wait for cmake-ls");

    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn send_interrupt(child: &std::process::Child) {
    let signal_status = Command::new("kill")
        .arg("-INT")
        .arg(child.id().to_string())
        .status()
        .expect("send SIGINT");

    assert!(signal_status.success());
}

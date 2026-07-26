use super::support::{TestProject, assert_success};

#[test]
fn rejects_an_unconfigured_directory_without_creating_a_query() {
    let project = TestProject::unconfigured();

    let output = project.run();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("CMakeCache.txt is missing; configure the build tree first")
    );
    assert!(!project.build_path(".cmake").exists());
}

#[test]
fn reports_cmake_regeneration_failures() {
    let project = TestProject::configured();
    assert_success(&project.run());
    project.replace_cmake_lists("not_a_cmake_command()\n");

    let output = project.run();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr.contains("CMake configuration failed"));
    assert!(stderr.contains("not_a_cmake_command"));
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use tempfile::{TempDir, tempdir};

pub const TARGET_LIST: &str = "app\ncore\ngenerate\n";

const PROJECT: &str = r"
cmake_minimum_required(VERSION 3.14)
project(cmake_ls_fixture C)

add_executable(app main.c)
add_library(core STATIC core.c)
add_custom_target(generate)
add_library(headers INTERFACE)
";

pub struct TestProject {
    temporary: TempDir,
    source_dir: PathBuf,
    build_dir: PathBuf,
}

impl TestProject {
    pub fn configured() -> Self {
        Self::configured_with_names("source", "build")
    }

    pub fn configured_with_spaces() -> Self {
        Self::configured_with_names("source tree", "build tree")
    }

    pub fn unconfigured() -> Self {
        let temporary = tempdir().expect("create temporary directory");
        let source_dir = temporary.path().join("source");
        let build_dir = temporary.path().join("unconfigured");
        fs::create_dir(&build_dir).expect("create unconfigured directory");

        Self {
            temporary,
            source_dir,
            build_dir,
        }
    }

    fn configured_with_names(source_name: &str, build_name: &str) -> Self {
        let temporary = tempdir().expect("create temporary directory");
        let source_dir = temporary.path().join(source_name);
        let build_dir = temporary.path().join(build_name);
        create_project(&source_dir);
        configure_project(&source_dir, &build_dir);

        Self {
            temporary,
            source_dir,
            build_dir,
        }
    }

    pub fn command(&self) -> Command {
        let mut command = cmake_ls();
        command.arg(&self.build_dir);
        command
    }

    pub fn run(&self) -> Output {
        self.command().output().expect("run cmake-ls")
    }

    pub fn run_from_default_build_directory(&self) -> Output {
        cmake_ls()
            .current_dir(self.temporary.path())
            .output()
            .expect("run cmake-ls")
    }

    pub fn build_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.build_dir.join(path)
    }

    pub fn replace_cmake_lists(&self, contents: &str) {
        fs::write(self.source_dir.join("CMakeLists.txt"), contents).expect("replace project file");
    }

    #[cfg(unix)]
    pub fn write_source_file(&self, name: &str, contents: &str) {
        fs::write(self.source_dir.join(name), contents).expect("write project fixture");
    }
}

pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "cmake-ls failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
pub fn wait_for_path(path: &Path, timeout: Duration) {
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

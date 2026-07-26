//! `CMake` query installation and process execution.

pub mod file_api;

use std::fs::{self, File};
use std::io::{self, Read as _, Seek as _};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(unix)]
use std::process::Child;

#[cfg(windows)]
use command_group::{CommandGroup as _, GroupChild};
#[cfg(unix)]
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tempfile::tempfile;

use crate::cancellation::Cancellation;
use crate::error::Error;

const CACHE_FILE: &str = "CMakeCache.txt";
const QUERY_PATH: [&str; 6] = [
    ".cmake",
    "api",
    "v1",
    "query",
    "client-cmake-ls",
    "codemodel-v2",
];
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const INTERRUPT_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[cfg(unix)]
type CmakeChild = Child;
#[cfg(windows)]
type CmakeChild = GroupChild;

/// Validate a build tree and install the client-owned codemodel query.
pub fn prepare_query(build_dir: &Path) -> Result<(), Error> {
    let metadata = fs::metadata(build_dir)
        .map_err(|source| Error::io("inspect build directory", build_dir.to_path_buf(), source))?;

    if !metadata.is_dir() {
        return Err(Error::InvalidBuildDirectory {
            path: build_dir.to_path_buf(),
            reason: "path is not a directory",
        });
    }

    let cache_path = build_dir.join(CACHE_FILE);
    let cache_metadata = fs::metadata(&cache_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::InvalidBuildDirectory {
                path: build_dir.to_path_buf(),
                reason: "CMakeCache.txt is missing; configure the build tree first",
            }
        } else {
            Error::io("inspect CMake cache", cache_path.clone(), source)
        }
    })?;

    if !cache_metadata.is_file() {
        return Err(Error::InvalidBuildDirectory {
            path: build_dir.to_path_buf(),
            reason: "CMakeCache.txt is not a regular file",
        });
    }

    let query_path = QUERY_PATH
        .iter()
        .fold(build_dir.to_path_buf(), |path, component| {
            path.join(component)
        });
    let query_dir = query_path
        .parent()
        .expect("the constant query path has a parent");

    fs::create_dir_all(query_dir)
        .map_err(|source| Error::io("create File API query directory", query_dir.into(), source))?;
    fs::File::create(&query_path)
        .map_err(|source| Error::io("write File API query", query_path, source))?;

    Ok(())
}

/// Reconfigure an existing build tree so `CMake` services the File API query.
pub fn configure(build_dir: &Path, cancellation: &Cancellation) -> Result<(), Error> {
    let (mut stdout_capture, stdout_target) = create_output_capture(build_dir, "stdout")?;
    let (mut stderr_capture, stderr_target) = create_output_capture(build_dir, "stderr")?;
    let mut command = Command::new("cmake");
    command
        .arg(build_dir)
        .stdout(stdout_target)
        .stderr(stderr_target);

    let mut child = spawn_process_group(&mut command).map_err(|source| Error::StartCmake {
        path: build_dir.to_path_buf(),
        source,
    })?;
    let status = wait_for_cmake(build_dir, &mut child, cancellation)?;

    if status.success() {
        return Ok(());
    }

    let stdout = read_output_capture(&mut stdout_capture, build_dir, "stdout", cancellation)?;
    let stderr = read_output_capture(&mut stderr_capture, build_dir, "stderr", cancellation)?;
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let mut details = String::new();

    if !stdout.trim().is_empty() {
        details.push_str("\nCMake stdout:\n");
        details.push_str(stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        details.push_str("\nCMake stderr:\n");
        details.push_str(stderr.trim_end());
    }

    Err(Error::CmakeFailed {
        path: build_dir.to_path_buf(),
        status,
        output: details,
    })
}

fn create_output_capture(build_dir: &Path, stream: &'static str) -> Result<(File, Stdio), Error> {
    let capture = tempfile().map_err(|source| {
        Error::io(
            if stream == "stdout" {
                "create CMake stdout capture"
            } else {
                "create CMake stderr capture"
            },
            build_dir.into(),
            source,
        )
    })?;
    let target = capture.try_clone().map_err(|source| {
        Error::io(
            if stream == "stdout" {
                "clone CMake stdout capture"
            } else {
                "clone CMake stderr capture"
            },
            build_dir.into(),
            source,
        )
    })?;

    Ok((capture, Stdio::from(target)))
}

fn read_output_capture(
    capture: &mut File,
    build_dir: &Path,
    stream: &'static str,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, Error> {
    capture.rewind().map_err(|source| {
        Error::io(
            if stream == "stdout" {
                "rewind CMake stdout capture"
            } else {
                "rewind CMake stderr capture"
            },
            build_dir.into(),
            source,
        )
    })?;

    let mut output = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        cancellation.checkpoint()?;
        let read = capture.read(&mut buffer).map_err(|source| {
            Error::io(
                if stream == "stdout" {
                    "read CMake stdout capture"
                } else {
                    "read CMake stderr capture"
                },
                build_dir.into(),
                source,
            )
        })?;
        if read == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..read]);
    }

    Ok(output)
}

fn wait_for_cmake(
    build_dir: &Path,
    child: &mut CmakeChild,
    cancellation: &Cancellation,
) -> Result<ExitStatus, Error> {
    let wait_result = wait_for_exit(build_dir, child, cancellation);
    if wait_result.is_err() {
        let _ = kill_process_group(child, build_dir);
        let _ = child.wait();
    }

    wait_result
}

fn wait_for_exit(
    build_dir: &Path,
    child: &mut CmakeChild,
    cancellation: &Cancellation,
) -> Result<ExitStatus, Error> {
    let mut interrupt_deadline = None;
    let mut force_killed = false;

    loop {
        cancellation.checkpoint_output()?;

        if let Some(status) = child
            .try_wait()
            .map_err(|source| Error::io("poll CMake process", build_dir.into(), source))?
        {
            return if interrupt_deadline.is_some() {
                Err(Error::Interrupted)
            } else {
                Ok(status)
            };
        }

        let interrupt_count = cancellation.interrupt_count();
        if interrupt_count > 0 && interrupt_deadline.is_none() {
            request_interrupt(child, build_dir)?;
            interrupt_deadline = Some(Instant::now() + INTERRUPT_GRACE_PERIOD);
        } else if !force_killed
            && interrupt_deadline
                .is_some_and(|deadline| interrupt_count > 1 || Instant::now() >= deadline)
        {
            kill_process_group(child, build_dir)?;
            force_killed = true;
        }

        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(unix)]
fn request_interrupt(child: &CmakeChild, build_dir: &Path) -> Result<(), Error> {
    signal_process_group(child, Signal::SIGINT)
        .map_err(|source| Error::io("interrupt CMake process group", build_dir.into(), source))
}

#[cfg(windows)]
fn request_interrupt(child: &mut CmakeChild, build_dir: &Path) -> Result<(), Error> {
    kill_process_group(child, build_dir)
}

#[cfg(unix)]
fn kill_process_group(child: &CmakeChild, build_dir: &Path) -> Result<(), Error> {
    signal_process_group(child, Signal::SIGKILL)
        .map_err(|source| Error::io("terminate CMake process group", build_dir.into(), source))
}

#[cfg(windows)]
fn kill_process_group(child: &mut CmakeChild, build_dir: &Path) -> Result<(), Error> {
    child
        .kill()
        .or_else(ignore_finished_process)
        .map_err(|source| Error::io("terminate CMake process group", build_dir.into(), source))
}

#[cfg(unix)]
fn signal_process_group(child: &CmakeChild, signal: Signal) -> io::Result<()> {
    let process_group =
        i32::try_from(child.id()).expect("Unix process identifiers are representable as i32");

    match killpg(Pid::from_raw(process_group), signal) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(windows)]
fn ignore_finished_process(error: io::Error) -> io::Result<()> {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
    ) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(unix)]
fn spawn_process_group(command: &mut Command) -> io::Result<CmakeChild> {
    command.process_group(0).spawn()
}

#[cfg(windows)]
fn spawn_process_group(command: &mut Command) -> io::Result<CmakeChild> {
    command.group_spawn()
}

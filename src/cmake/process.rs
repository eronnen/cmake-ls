//! `CMake` process execution and cancellation.

use std::fs::File;
use std::io::{self, Read as _, Seek as _};
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

use super::BuildTree;

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const INTERRUPT_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[cfg(unix)]
type CmakeChild = Child;
#[cfg(windows)]
type CmakeChild = GroupChild;

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    const fn create_action(self) -> &'static str {
        match self {
            Self::Stdout => "create CMake stdout capture",
            Self::Stderr => "create CMake stderr capture",
        }
    }

    const fn clone_action(self) -> &'static str {
        match self {
            Self::Stdout => "clone CMake stdout capture",
            Self::Stderr => "clone CMake stderr capture",
        }
    }

    const fn rewind_action(self) -> &'static str {
        match self {
            Self::Stdout => "rewind CMake stdout capture",
            Self::Stderr => "rewind CMake stderr capture",
        }
    }

    const fn read_action(self) -> &'static str {
        match self {
            Self::Stdout => "read CMake stdout capture",
            Self::Stderr => "read CMake stderr capture",
        }
    }
}

impl BuildTree<'_> {
    /// Reconfigure the build tree so `CMake` services the File API query.
    pub(super) fn configure(&self, cancellation: &Cancellation) -> Result<(), Error> {
        let (mut stdout_capture, stdout_target) =
            self.create_output_capture(OutputStream::Stdout)?;
        let (mut stderr_capture, stderr_target) =
            self.create_output_capture(OutputStream::Stderr)?;
        let mut command = Command::new("cmake");
        command
            .arg(self.build_dir)
            .stdout(stdout_target)
            .stderr(stderr_target);

        let mut child = spawn_process_group(&mut command).map_err(|source| Error::StartCmake {
            path: self.build_dir.to_path_buf(),
            source,
        })?;
        let status = self.wait_for_cmake(&mut child, cancellation)?;

        if status.success() {
            return Ok(());
        }

        let stdout =
            self.read_output_capture(&mut stdout_capture, OutputStream::Stdout, cancellation)?;
        let stderr =
            self.read_output_capture(&mut stderr_capture, OutputStream::Stderr, cancellation)?;
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
            path: self.build_dir.to_path_buf(),
            status,
            output: details,
        })
    }

    fn create_output_capture(&self, stream: OutputStream) -> Result<(File, Stdio), Error> {
        let capture = tempfile()
            .map_err(|source| Error::io(stream.create_action(), self.build_dir.into(), source))?;
        let target = capture
            .try_clone()
            .map_err(|source| Error::io(stream.clone_action(), self.build_dir.into(), source))?;

        Ok((capture, Stdio::from(target)))
    }

    fn read_output_capture(
        &self,
        capture: &mut File,
        stream: OutputStream,
        cancellation: &Cancellation,
    ) -> Result<Vec<u8>, Error> {
        capture
            .rewind()
            .map_err(|source| Error::io(stream.rewind_action(), self.build_dir.into(), source))?;

        let mut output = Vec::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            cancellation.checkpoint()?;
            let read = capture
                .read(&mut buffer)
                .map_err(|source| Error::io(stream.read_action(), self.build_dir.into(), source))?;
            if read == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..read]);
        }

        Ok(output)
    }

    fn wait_for_cmake(
        &self,
        child: &mut CmakeChild,
        cancellation: &Cancellation,
    ) -> Result<ExitStatus, Error> {
        let wait_result = self.wait_for_exit(child, cancellation);
        if wait_result.is_err() {
            let _ = self.kill_process_group(child);
            let _ = child.wait();
        }

        wait_result
    }

    fn wait_for_exit(
        &self,
        child: &mut CmakeChild,
        cancellation: &Cancellation,
    ) -> Result<ExitStatus, Error> {
        let mut interrupt_deadline = None;
        let mut force_killed = false;

        loop {
            cancellation.checkpoint_output()?;

            if let Some(status) = child
                .try_wait()
                .map_err(|source| Error::io("poll CMake process", self.build_dir.into(), source))?
            {
                return if interrupt_deadline.is_some() {
                    Err(Error::Interrupted)
                } else {
                    Ok(status)
                };
            }

            let interrupt_count = cancellation.interrupt_count();
            if interrupt_count > 0 && interrupt_deadline.is_none() {
                self.request_interrupt(child)?;
                interrupt_deadline = Some(Instant::now() + INTERRUPT_GRACE_PERIOD);
            } else if !force_killed
                && interrupt_deadline
                    .is_some_and(|deadline| interrupt_count > 1 || Instant::now() >= deadline)
            {
                self.kill_process_group(child)?;
                force_killed = true;
            }

            thread::sleep(POLL_INTERVAL);
        }
    }

    #[cfg(unix)]
    fn request_interrupt(&self, child: &CmakeChild) -> Result<(), Error> {
        signal_process_group(child, Signal::SIGINT).map_err(|source| {
            Error::io(
                "interrupt CMake process group",
                self.build_dir.into(),
                source,
            )
        })
    }

    #[cfg(windows)]
    fn request_interrupt(&self, child: &mut CmakeChild) -> Result<(), Error> {
        self.kill_process_group(child)
    }

    #[cfg(unix)]
    fn kill_process_group(&self, child: &CmakeChild) -> Result<(), Error> {
        signal_process_group(child, Signal::SIGKILL).map_err(|source| {
            Error::io(
                "terminate CMake process group",
                self.build_dir.into(),
                source,
            )
        })
    }

    #[cfg(windows)]
    fn kill_process_group(&self, child: &mut CmakeChild) -> Result<(), Error> {
        child
            .kill()
            .or_else(ignore_finished_process)
            .map_err(|source| {
                Error::io(
                    "terminate CMake process group",
                    self.build_dir.into(),
                    source,
                )
            })
    }
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

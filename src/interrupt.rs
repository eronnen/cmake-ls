//! Cross-platform Ctrl+C state.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::fd::AsFd as _;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::error::Error;

static INTERRUPT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Install the process-wide Ctrl+C handler.
pub fn install() -> Result<(), Error> {
    ctrlc::set_handler(|| {
        INTERRUPT_COUNT.fetch_add(1, Ordering::SeqCst);
    })
    .map_err(Error::InstallInterruptHandler)
}

/// Return the number of interrupts received by this process.
pub fn count() -> usize {
    INTERRUPT_COUNT.load(Ordering::SeqCst)
}

/// Stop the current operation if an interrupt has been received.
pub fn check() -> Result<(), Error> {
    if count() > 0 {
        return Err(Error::Interrupted);
    }

    check_output()
}

/// Stop the current operation if the standard output consumer has exited.
#[cfg(unix)]
pub fn check_output() -> Result<(), Error> {
    let stdout = io::stdout();
    let mut descriptors = [PollFd::new(stdout.as_fd(), PollFlags::POLLOUT)];

    match poll(&mut descriptors, PollTimeout::ZERO) {
        Ok(_) => {
            let closed = descriptors[0].revents().is_some_and(|events| {
                events.intersects(PollFlags::POLLERR | PollFlags::POLLHUP | PollFlags::POLLNVAL)
            });

            if closed {
                Err(Error::write_stdout(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "standard output consumer exited",
                )))
            } else {
                Ok(())
            }
        }
        Err(Errno::EINTR) => Ok(()),
        Err(error) => Err(Error::write_stdout(io::Error::from(error))),
    }
}

/// Stop the current operation if the standard output consumer has exited.
#[cfg(not(unix))]
pub const fn check_output() -> Result<(), Error> {
    Ok(())
}

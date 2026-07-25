//! Cooperative cancellation for signals and closed output pipelines.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::os::fd::AsFd as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(unix)]
use nix::errno::Errno;
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

use crate::error::Error;

/// Shared cancellation state for one command invocation.
#[derive(Clone, Default)]
pub struct Cancellation {
    interrupt_count: Arc<AtomicUsize>,
}

impl Cancellation {
    /// Create shared state and install the process-wide Ctrl+C handler.
    pub fn install() -> Result<Self, Error> {
        let cancellation = Self::default();
        let handler_state = cancellation.clone();

        ctrlc::set_handler(move || {
            handler_state.record_interrupt();
        })
        .map_err(Error::InstallInterruptHandler)?;

        Ok(cancellation)
    }

    /// Return the number of interrupts received by this process.
    pub fn interrupt_count(&self) -> usize {
        self.interrupt_count.load(Ordering::SeqCst)
    }

    fn record_interrupt(&self) {
        self.interrupt_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Stop the current operation if cancellation has been requested.
    pub fn checkpoint(&self) -> Result<(), Error> {
        self.checkpoint_impl(true)
    }

    /// Stop the current operation if the standard output consumer has exited.
    pub fn checkpoint_output(&self) -> Result<(), Error> {
        self.checkpoint_impl(false)
    }

    fn checkpoint_impl(&self, include_interrupts: bool) -> Result<(), Error> {
        if include_interrupts && self.interrupt_count() > 0 {
            return Err(Error::Interrupted);
        }

        check_output()
    }
}

#[cfg(unix)]
fn check_output() -> Result<(), Error> {
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

#[cfg(not(unix))]
const fn check_output() -> Result<(), Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cancellation;

    #[test]
    fn clones_share_interrupt_state() {
        let cancellation = Cancellation::default();
        let handler_state = cancellation.clone();
        let independent = Cancellation::default();

        handler_state.record_interrupt();

        assert_eq!(cancellation.interrupt_count(), 1);
        assert_eq!(independent.interrupt_count(), 0);
        assert!(cancellation.checkpoint().is_err());
    }
}

//! Cross-platform Ctrl+C state.

use std::sync::atomic::{AtomicUsize, Ordering};

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
    if count() == 0 {
        Ok(())
    } else {
        Err(Error::Interrupted)
    }
}

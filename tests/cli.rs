//! End-to-end command-line tests using real `CMake` build trees.

#[path = "cli/failures.rs"]
mod failures;
#[cfg(unix)]
#[path = "cli/interruption.rs"]
mod interruption;
#[path = "cli/listing.rs"]
mod listing;
#[path = "cli/support.rs"]
mod support;

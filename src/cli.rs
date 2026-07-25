//! Command-line argument parsing.

use std::path::PathBuf;

use clap::Parser;

/// List buildable targets from an existing `CMake` build tree.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "List buildable targets from an existing CMake build tree"
)]
pub struct Cli {
    /// Existing `CMake` build directory.
    #[arg(
        default_value = "build",
        value_name = "BUILD_DIR",
        help = "Existing CMake build directory"
    )]
    pub build_dir: PathBuf,
}

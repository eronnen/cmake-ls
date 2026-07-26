//! List buildable targets from an existing `CMake` build tree.

mod cancellation;
mod cmake;
mod error;

use std::io::{self, BufWriter, Write as _};
use std::process::ExitCode;

use clap::Parser as _;

use crate::cancellation::Cancellation;
use crate::error::Error;

#[derive(Debug, clap::Parser)]
#[command(
    version,
    about = "List buildable targets from an existing CMake build tree"
)]
struct Cli {
    /// Regenerate the File API reply even when the existing reply is current.
    #[arg(long, help = "Regenerate the File API reply before listing targets")]
    pub refresh: bool,

    /// Existing `CMake` build directory.
    #[arg(
        default_value = "build",
        value_name = "BUILD_DIR",
        help = "Existing CMake build directory"
    )]
    pub build_dir: std::path::PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let cancellation = match Cancellation::install() {
        Ok(cancellation) => cancellation,
        Err(error) => {
            eprintln!("cmake-ls: {error}");
            return ExitCode::FAILURE;
        }
    };

    match run(&cli, &cancellation) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.is_interrupted() => ExitCode::from(130),
        Err(error) if error.is_broken_pipe() => {
            if cancellation.interrupt_count() > 0 {
                ExitCode::from(130)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("cmake-ls: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli, cancellation: &Cancellation) -> Result<(), Error> {
    let build_tree = cmake::BuildTree::new(&cli.build_dir);
    let targets = build_tree.targets(cli.refresh, cancellation)?;
    cancellation.checkpoint()?;
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());

    for target in targets {
        cancellation.checkpoint()?;
        writeln!(output, "{target}").map_err(Error::write_stdout)?;
    }

    output.flush().map_err(Error::write_stdout)
}

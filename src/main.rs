//! List buildable targets from an existing `CMake` build tree.

mod cli;
mod cmake;
mod error;
mod file_api;
mod interrupt;

use std::io::{self, BufWriter, Write as _};
use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::Cli;
use crate::error::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.is_interrupted() => ExitCode::from(130),
        Err(error) if error.is_broken_pipe() => {
            if interrupt::count() > 0 {
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

fn run() -> Result<(), Error> {
    let cli = Cli::parse();
    interrupt::install()?;

    cmake::prepare_query(&cli.build_dir)?;
    interrupt::check()?;
    cmake::configure(&cli.build_dir)?;

    let targets = file_api::read_targets(&cli.build_dir)?;
    interrupt::check()?;
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());

    for target in targets {
        interrupt::check()?;
        writeln!(output, "{target}").map_err(Error::write_stdout)?;
    }

    output.flush().map_err(Error::write_stdout)
}

//! List buildable targets from an existing `CMake` build tree.

mod cli;
mod cmake;
mod error;
mod file_api;

use std::io::{self, BufWriter, Write as _};
use std::process::ExitCode;

use clap::Parser as _;

use crate::cli::Cli;
use crate::error::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cmake-ls: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let cli = Cli::parse();

    cmake::prepare_query(&cli.build_dir)?;
    cmake::configure(&cli.build_dir)?;

    let targets = file_api::read_targets(&cli.build_dir)?;
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());

    for target in targets {
        writeln!(output, "{target}").map_err(Error::write_stdout)?;
    }

    output.flush().map_err(Error::write_stdout)
}

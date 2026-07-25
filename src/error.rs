//! Application errors.

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

/// Errors reported while querying a `CMake` build tree.
#[derive(Debug, Error)]
pub enum Error {
    /// A filesystem operation failed.
    #[error("failed to {action} `{}`: {source}", path.display())]
    Io {
        /// Description of the attempted operation.
        action: &'static str,
        /// Path involved in the operation.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// A path does not identify a usable configured build tree.
    #[error("invalid CMake build directory `{}`: {reason}", path.display())]
    InvalidBuildDirectory {
        /// Invalid path.
        path: PathBuf,
        /// Explanation of the validation failure.
        reason: &'static str,
    },

    /// `CMake` could not be launched.
    #[error("failed to run `cmake` for `{}`: {source}", path.display())]
    StartCmake {
        /// Build directory passed to `CMake`.
        path: PathBuf,
        /// Underlying process error.
        #[source]
        source: io::Error,
    },

    /// `CMake` completed unsuccessfully.
    #[error(
        "CMake configuration failed for `{}` with {status}{output}",
        path.display()
    )]
    CmakeFailed {
        /// Build directory passed to `CMake`.
        path: PathBuf,
        /// `CMake` process status.
        status: ExitStatus,
        /// Captured `CMake` output, formatted for display.
        output: String,
    },

    /// The interrupt handler could not be installed.
    #[error("failed to install Ctrl+C handler: {0}")]
    InstallInterruptHandler(#[source] ctrlc::Error),

    /// The operation was interrupted by the user.
    #[error("interrupted")]
    Interrupted,

    /// A background output reader panicked.
    #[error("failed to capture CMake {stream} for `{}`", path.display())]
    CmakeOutputReaderPanicked {
        /// Build directory passed to `CMake`.
        path: PathBuf,
        /// Name of the captured output stream.
        stream: &'static str,
    },

    /// A JSON reply could not be decoded.
    #[error("failed to parse CMake File API reply `{}`: {source}", path.display())]
    Json {
        /// JSON file being decoded.
        path: PathBuf,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// The File API response is absent, unsupported, or inconsistent.
    #[error("invalid CMake File API response in `{}`: {reason}", path.display())]
    FileApi {
        /// Reply directory or file where the problem was found.
        path: PathBuf,
        /// Explanation of the invalid response.
        reason: String,
    },

    /// Writing the target list failed.
    #[error("failed to write target list to stdout: {0}")]
    WriteStdout(#[source] io::Error),
}

impl Error {
    /// Construct a path-aware I/O error.
    pub const fn io(action: &'static str, path: PathBuf, source: io::Error) -> Self {
        Self::Io {
            action,
            path,
            source,
        }
    }

    /// Construct a stdout error.
    pub const fn write_stdout(source: io::Error) -> Self {
        Self::WriteStdout(source)
    }

    /// Whether this error can result from `CMake` replacing reply files concurrently.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::Io {
                source,
                action: _,
                path: _
            } if source.kind() == io::ErrorKind::NotFound
        )
    }

    /// Whether the operation was interrupted by the user.
    pub const fn is_interrupted(&self) -> bool {
        matches!(self, Self::Interrupted)
    }

    /// Whether the output consumer closed its pipe.
    pub fn is_broken_pipe(&self) -> bool {
        matches!(
            self,
            Self::WriteStdout(source) if source.kind() == io::ErrorKind::BrokenPipe
        )
    }
}

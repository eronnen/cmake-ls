//! Operations on a configured `CMake` build tree.

mod file_api;
mod process;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::cancellation::Cancellation;
use crate::error::Error;

const CACHE_FILE: &str = "CMakeCache.txt";
const QUERY_PATH: [&str; 6] = [
    ".cmake",
    "api",
    "v1",
    "query",
    "client-cmake-ls",
    "codemodel-v2",
];

/// A handle to a `CMake` build tree.
pub struct BuildTree<'a> {
    build_dir: &'a Path,
}

impl<'a> BuildTree<'a> {
    /// Create a handle for the build tree at `build_dir`.
    pub const fn new(build_dir: &'a Path) -> Self {
        Self { build_dir }
    }

    /// Query the build tree for its sorted, unique buildable target names.
    pub fn targets(&self, cancellation: &Cancellation) -> Result<BTreeSet<String>, Error> {
        self.prepare_query()?;
        cancellation.checkpoint()?;
        self.configure(cancellation)?;
        self.read_targets(cancellation)
    }

    fn prepare_query(&self) -> Result<(), Error> {
        let metadata = fs::metadata(self.build_dir).map_err(|source| {
            Error::io(
                "inspect build directory",
                self.build_dir.to_path_buf(),
                source,
            )
        })?;

        if !metadata.is_dir() {
            return Err(Error::InvalidBuildDirectory {
                path: self.build_dir.to_path_buf(),
                reason: "path is not a directory",
            });
        }

        let cache_path = self.build_dir.join(CACHE_FILE);
        let cache_metadata = fs::metadata(&cache_path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::InvalidBuildDirectory {
                    path: self.build_dir.to_path_buf(),
                    reason: "CMakeCache.txt is missing; configure the build tree first",
                }
            } else {
                Error::io("inspect CMake cache", cache_path.clone(), source)
            }
        })?;

        if !cache_metadata.is_file() {
            return Err(Error::InvalidBuildDirectory {
                path: self.build_dir.to_path_buf(),
                reason: "CMakeCache.txt is not a regular file",
            });
        }

        let query_path = QUERY_PATH
            .iter()
            .fold(self.build_dir.to_path_buf(), |path, component| {
                path.join(component)
            });
        let query_dir = query_path
            .parent()
            .expect("the constant query path has a parent");

        fs::create_dir_all(query_dir).map_err(|source| {
            Error::io("create File API query directory", query_dir.into(), source)
        })?;
        fs::File::create(&query_path)
            .map_err(|source| Error::io("write File API query", query_path, source))?;

        Ok(())
    }
}

//! `CMake` query installation and process execution.

use std::fs;
use std::path::Path;
use std::process::Command;

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

/// Validate a build tree and install the client-owned codemodel query.
pub fn prepare_query(build_dir: &Path) -> Result<(), Error> {
    let metadata = fs::metadata(build_dir)
        .map_err(|source| Error::io("inspect build directory", build_dir.to_path_buf(), source))?;

    if !metadata.is_dir() {
        return Err(Error::InvalidBuildDirectory {
            path: build_dir.to_path_buf(),
            reason: "path is not a directory",
        });
    }

    let cache_path = build_dir.join(CACHE_FILE);
    let cache_metadata = fs::metadata(&cache_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::InvalidBuildDirectory {
                path: build_dir.to_path_buf(),
                reason: "CMakeCache.txt is missing; configure the build tree first",
            }
        } else {
            Error::io("inspect CMake cache", cache_path.clone(), source)
        }
    })?;

    if !cache_metadata.is_file() {
        return Err(Error::InvalidBuildDirectory {
            path: build_dir.to_path_buf(),
            reason: "CMakeCache.txt is not a regular file",
        });
    }

    let query_path = QUERY_PATH
        .iter()
        .fold(build_dir.to_path_buf(), |path, component| {
            path.join(component)
        });
    let query_dir = query_path
        .parent()
        .expect("the constant query path has a parent");

    fs::create_dir_all(query_dir)
        .map_err(|source| Error::io("create File API query directory", query_dir.into(), source))?;
    fs::File::create(&query_path)
        .map_err(|source| Error::io("write File API query", query_path, source))?;

    Ok(())
}

/// Reconfigure an existing build tree so `CMake` services the File API query.
pub fn configure(build_dir: &Path) -> Result<(), Error> {
    let output = Command::new("cmake")
        .arg(build_dir)
        .output()
        .map_err(|source| Error::StartCmake {
            path: build_dir.to_path_buf(),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut details = String::new();

    if !stdout.trim().is_empty() {
        details.push_str("\nCMake stdout:\n");
        details.push_str(stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        details.push_str("\nCMake stderr:\n");
        details.push_str(stderr.trim_end());
    }

    Err(Error::CmakeFailed {
        path: build_dir.to_path_buf(),
        status: output.status,
        output: details,
    })
}

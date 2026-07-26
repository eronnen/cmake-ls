//! Typed parsing for `CMake` File API codemodel replies.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::cancellation::Cancellation;
use crate::error::Error;

use super::BuildTree;

const CLIENT_REPLY: &str = "client-cmake-ls";
const CODEMODEL_REPLY: &str = "codemodel-v2";
const MAX_READ_ATTEMPTS: usize = 3;

#[derive(Debug, Deserialize)]
struct ReplyIndex {
    reply: ReplyQueries,
}

#[derive(Debug, Deserialize)]
struct ReplyQueries {
    #[serde(rename = "client-cmake-ls")]
    client: Option<ClientQueries>,
}

#[derive(Debug, Deserialize)]
struct ClientQueries {
    #[serde(rename = "cmakeFiles-v1")]
    cmake_files: Option<QueryResponse>,
    #[serde(rename = "codemodel-v2")]
    codemodel: Option<QueryResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum QueryResponse {
    Reference(ReplyReference),
    Error(QueryError),
}

#[derive(Debug, Deserialize)]
struct QueryError {
    error: String,
}

#[derive(Debug, Deserialize)]
struct ReplyReference {
    kind: String,
    version: Version,
    #[serde(rename = "jsonFile")]
    json_file: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Version {
    major: u64,
    minor: u64,
}

#[derive(Debug, Deserialize)]
struct Codemodel {
    kind: String,
    version: Version,
    configurations: Vec<Configuration>,
}

#[derive(Debug, Deserialize)]
struct Configuration {
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
struct Target {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CmakeFiles {
    kind: String,
    version: Version,
    paths: CmakeFilePaths,
    inputs: Vec<CmakeInput>,
}

#[derive(Debug, Deserialize)]
struct CmakeFilePaths {
    source: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CmakeInput {
    path: PathBuf,
}

impl BuildTree<'_> {
    /// Read target names when the latest File API reply is still current.
    pub(super) fn read_fresh_targets(
        &self,
        cancellation: &Cancellation,
    ) -> Result<Option<BTreeSet<String>>, Error> {
        for attempt in 0..MAX_READ_ATTEMPTS {
            match self.read_fresh_targets_once(cancellation) {
                Ok(targets) => return Ok(targets),
                Err(error) if error.is_not_found() && attempt + 1 < MAX_READ_ATTEMPTS => {
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("the bounded read loop always returns")
    }

    fn read_fresh_targets_once(
        &self,
        cancellation: &Cancellation,
    ) -> Result<Option<BTreeSet<String>>, Error> {
        let reply_dir = self.build_dir.join(".cmake/api/v1/reply");
        let index_path = latest_index(&reply_dir, cancellation)?;
        let index_modified = modified(&index_path, "inspect File API reply index")?;
        let index: ReplyIndex = read_json(&index_path, cancellation)?;
        let Some(client) = index.reply.client else {
            return Ok(None);
        };
        let targets =
            read_targets_from_response(&reply_dir, &index_path, client.codemodel, cancellation)?;
        let Some(cmake_files_response) = client.cmake_files else {
            return Ok(None);
        };
        let cmake_files_reference =
            reply_reference(cmake_files_response, &index_path, "cmakeFiles-v1")?;
        validate_version(
            &reply_dir,
            &cmake_files_reference.kind,
            &cmake_files_reference.version,
            "cmakeFiles",
            1,
        )?;
        let cmake_files_path = resolve_reference(&reply_dir, &cmake_files_reference.json_file)?;
        let cmake_files: CmakeFiles = read_json(&cmake_files_path, cancellation)?;
        validate_version(
            &cmake_files_path,
            &cmake_files.kind,
            &cmake_files.version,
            "cmakeFiles",
            1,
        )?;

        let verify_globs_path = self.build_dir.join("CMakeFiles/VerifyGlobs.cmake");
        if verify_globs_path
            .try_exists()
            .map_err(|source| Error::io("inspect CMake glob verifier", verify_globs_path, source))?
        {
            return Ok(None);
        }

        for input in cmake_files.inputs {
            cancellation.checkpoint()?;
            let input_path = if input.path.is_absolute() {
                input.path
            } else {
                cmake_files.paths.source.join(input.path)
            };

            if modified(&input_path, "inspect CMake input")? > index_modified {
                return Ok(None);
            }
        }

        Ok(Some(targets))
    }

    /// Read target names from the latest File API reply.
    pub(super) fn read_targets(
        &self,
        cancellation: &Cancellation,
    ) -> Result<BTreeSet<String>, Error> {
        for attempt in 0..MAX_READ_ATTEMPTS {
            match self.read_targets_once(cancellation) {
                Ok(targets) => return Ok(targets),
                Err(error) if error.is_not_found() && attempt + 1 < MAX_READ_ATTEMPTS => {
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("the bounded read loop always returns")
    }

    fn read_targets_once(&self, cancellation: &Cancellation) -> Result<BTreeSet<String>, Error> {
        let reply_dir = self.build_dir.join(".cmake/api/v1/reply");
        let index_path = latest_index(&reply_dir, cancellation)?;
        let index: ReplyIndex = read_json(&index_path, cancellation)?;

        let client = index.reply.client.ok_or_else(|| Error::FileApi {
            path: index_path.clone(),
            reason: format!("reply does not contain `{CLIENT_REPLY}`"),
        })?;
        read_targets_from_response(&reply_dir, &index_path, client.codemodel, cancellation)
    }
}

fn read_targets_from_response(
    reply_dir: &Path,
    index_path: &Path,
    response: Option<QueryResponse>,
    cancellation: &Cancellation,
) -> Result<BTreeSet<String>, Error> {
    let response = response.ok_or_else(|| Error::FileApi {
        path: index_path.to_path_buf(),
        reason: format!("reply does not contain `{CODEMODEL_REPLY}`"),
    })?;
    let reference = reply_reference(response, index_path, CODEMODEL_REPLY)?;

    validate_version(
        reply_dir,
        &reference.kind,
        &reference.version,
        "codemodel",
        2,
    )?;
    let codemodel_path = resolve_reference(reply_dir, &reference.json_file)?;
    let codemodel: Codemodel = read_json(&codemodel_path, cancellation)?;
    validate_version(
        &codemodel_path,
        &codemodel.kind,
        &codemodel.version,
        "codemodel",
        2,
    )?;

    let mut targets = BTreeSet::new();
    for configuration in codemodel.configurations {
        cancellation.checkpoint()?;
        for target in configuration.targets {
            cancellation.checkpoint()?;
            targets.insert(target.name);
        }
    }

    Ok(targets)
}

fn reply_reference(
    response: QueryResponse,
    index_path: &Path,
    query: &str,
) -> Result<ReplyReference, Error> {
    match response {
        QueryResponse::Reference(reference) => Ok(reference),
        QueryResponse::Error(error) => Err(Error::FileApi {
            path: index_path.to_path_buf(),
            reason: format!("CMake rejected the `{query}` query: {}", error.error),
        }),
    }
}

fn latest_index(reply_dir: &Path, cancellation: &Cancellation) -> Result<PathBuf, Error> {
    let entries = fs::read_dir(reply_dir)
        .map_err(|source| Error::io("read File API reply directory", reply_dir.into(), source))?;

    let mut latest = None;
    for entry in entries {
        cancellation.checkpoint()?;
        let entry = entry
            .map_err(|source| Error::io("read File API reply entry", reply_dir.into(), source))?;
        let name = entry.file_name();
        let Some(name_text) = name.to_str() else {
            continue;
        };
        let is_index = name_text.starts_with("index-")
            && Path::new(&name)
                .extension()
                .is_some_and(|extension| extension == "json");

        if is_index
            && latest
                .as_ref()
                .is_none_or(|current: &PathBuf| entry.path().file_name() > current.file_name())
        {
            latest = Some(entry.path());
        }
    }

    latest.ok_or_else(|| Error::FileApi {
        path: reply_dir.to_path_buf(),
        reason: "no reply index was produced; CMake 3.14 or newer is required".to_owned(),
    })
}

fn read_json<T>(path: &Path, cancellation: &Cancellation) -> Result<T, Error>
where
    T: for<'de> Deserialize<'de>,
{
    let file =
        File::open(path).map_err(|source| Error::io("read File API reply", path.into(), source))?;
    let reader = BufReader::new(InterruptibleReader::new(file, cancellation));

    serde_json::from_reader(reader).map_err(|source| {
        if source.io_error_kind() == Some(io::ErrorKind::Interrupted)
            && cancellation.interrupt_count() > 0
        {
            Error::Interrupted
        } else {
            Error::Json {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

fn modified(path: &Path, action: &'static str) -> Result<std::time::SystemTime, Error> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|source| Error::io(action, path.to_path_buf(), source))
}

struct InterruptibleReader<'a, R> {
    inner: R,
    cancellation: &'a Cancellation,
}

impl<'a, R> InterruptibleReader<'a, R> {
    const fn new(inner: R, cancellation: &'a Cancellation) -> Self {
        Self {
            inner,
            cancellation,
        }
    }
}

impl<R> Read for InterruptibleReader<'_, R>
where
    R: Read,
{
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancellation.interrupt_count() > 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "operation interrupted",
            ));
        }

        self.inner.read(buffer)
    }
}

fn validate_version(
    path: &Path,
    kind: &str,
    version: &Version,
    expected_kind: &str,
    expected_major: u64,
) -> Result<(), Error> {
    if kind != expected_kind {
        return Err(Error::FileApi {
            path: path.to_path_buf(),
            reason: format!("expected object kind `{expected_kind}`, found `{kind}`"),
        });
    }
    if version.major != expected_major {
        return Err(Error::FileApi {
            path: path.to_path_buf(),
            reason: format!(
                "expected {expected_kind} major version {expected_major}, found {}.{}",
                version.major, version.minor
            ),
        });
    }

    Ok(())
}

fn resolve_reference(reply_dir: &Path, reference: &Path) -> Result<PathBuf, Error> {
    let is_safe = !reference.as_os_str().is_empty()
        && !reference.is_absolute()
        && reference
            .components()
            .all(|component| matches!(component, Component::Normal(_)));

    if !is_safe {
        return Err(Error::FileApi {
            path: reply_dir.to_path_buf(),
            reason: format!(
                "reply contains an unsafe JSON file reference `{}`",
                reference.display()
            ),
        });
    }

    Ok(reply_dir.join(reference))
}

#[cfg(test)]
mod tests;

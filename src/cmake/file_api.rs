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

impl BuildTree<'_> {
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
        let response = client.codemodel.ok_or_else(|| Error::FileApi {
            path: index_path.clone(),
            reason: format!("reply does not contain `{CODEMODEL_REPLY}`"),
        })?;
        let reference = match response {
            QueryResponse::Reference(reference) => reference,
            QueryResponse::Error(error) => {
                return Err(Error::FileApi {
                    path: index_path,
                    reason: format!("CMake rejected the codemodel query: {}", error.error),
                });
            }
        };

        validate_version(&reply_dir, &reference.kind, &reference.version)?;
        let codemodel_path = resolve_reference(&reply_dir, &reference.json_file)?;
        let codemodel: Codemodel = read_json(&codemodel_path, cancellation)?;
        validate_version(&codemodel_path, &codemodel.kind, &codemodel.version)?;

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

fn validate_version(path: &Path, kind: &str, version: &Version) -> Result<(), Error> {
    if kind != "codemodel" {
        return Err(Error::FileApi {
            path: path.to_path_buf(),
            reason: format!("expected object kind `codemodel`, found `{kind}`"),
        });
    }
    if version.major != 2 {
        return Err(Error::FileApi {
            path: path.to_path_buf(),
            reason: format!(
                "expected codemodel major version 2, found {}.{}",
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

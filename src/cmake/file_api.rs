//! Typed parsing for `CMake` File API codemodel replies.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::cancellation::Cancellation;
use crate::error::Error;

use super::Cmake;

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

impl Cmake<'_> {
    /// Read sorted, unique buildable target names from the latest File API reply.
    pub fn read_targets(&self, cancellation: &Cancellation) -> Result<BTreeSet<String>, Error> {
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
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use crate::cancellation::Cancellation;

    use super::{Cmake, Version, resolve_reference, validate_version};

    #[test]
    fn accepts_codemodel_v2() {
        let result = validate_version(
            Path::new("codemodel.json"),
            "codemodel",
            &Version { major: 2, minor: 9 },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn rejects_other_object_kinds() {
        let result = validate_version(
            Path::new("cache.json"),
            "cache",
            &Version { major: 2, minor: 0 },
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_other_codemodel_versions() {
        let result = validate_version(
            Path::new("codemodel.json"),
            "codemodel",
            &Version { major: 3, minor: 0 },
        );

        assert!(result.is_err());
    }

    #[test]
    fn confines_references_to_the_reply_directory() {
        let reply_dir = Path::new("/tmp/reply");

        assert!(resolve_reference(reply_dir, Path::new("codemodel.json")).is_ok());
        assert!(resolve_reference(reply_dir, Path::new("nested/codemodel.json")).is_ok());
        assert!(resolve_reference(reply_dir, Path::new("../codemodel.json")).is_err());
        assert!(resolve_reference(reply_dir, Path::new("/tmp/codemodel.json")).is_err());
        assert!(resolve_reference(reply_dir, Path::new("")).is_err());
    }

    #[test]
    fn reads_sorts_and_deduplicates_targets_across_configurations() {
        let build_dir = tempdir().expect("create temporary build directory");
        write_reply(
            build_dir.path(),
            r#"{
                "kind": "codemodel",
                "version": { "major": 2, "minor": 9 },
                "configurations": [
                    {
                        "name": "Debug",
                        "targets": [
                            { "name": "zeta" },
                            { "name": "alpha" }
                        ]
                    },
                    {
                        "name": "Release",
                        "targets": [
                            { "name": "alpha" },
                            { "name": "middle" }
                        ]
                    }
                ],
                "futureField": true
            }"#,
        );

        let targets = Cmake::new(build_dir.path())
            .read_targets(&Cancellation::default())
            .expect("read target names");
        let names: Vec<_> = targets.into_iter().collect();

        assert_eq!(names, ["alpha", "middle", "zeta"]);
    }

    #[test]
    fn accepts_an_empty_target_list() {
        let build_dir = tempdir().expect("create temporary build directory");
        write_reply(
            build_dir.path(),
            r#"{
                "kind": "codemodel",
                "version": { "major": 2, "minor": 0 },
                "configurations": [{ "name": "", "targets": [] }]
            }"#,
        );

        let targets = Cmake::new(build_dir.path())
            .read_targets(&Cancellation::default())
            .expect("read target names");

        assert!(targets.is_empty());
    }

    #[test]
    fn rejects_a_missing_client_response() {
        let build_dir = tempdir().expect("create temporary build directory");
        let reply_dir = build_dir.path().join(".cmake/api/v1/reply");
        fs::create_dir_all(&reply_dir).expect("create reply directory");
        fs::write(reply_dir.join("index-test.json"), r#"{ "reply": {} }"#).expect("write index");

        let error = Cmake::new(build_dir.path())
            .read_targets(&Cancellation::default())
            .expect_err("reject missing response");

        assert!(error.to_string().contains("client-cmake-ls"));
    }

    #[test]
    fn reports_malformed_json() {
        let build_dir = tempdir().expect("create temporary build directory");
        let reply_dir = build_dir.path().join(".cmake/api/v1/reply");
        fs::create_dir_all(&reply_dir).expect("create reply directory");
        fs::write(reply_dir.join("index-test.json"), "{invalid").expect("write index");

        let error = Cmake::new(build_dir.path())
            .read_targets(&Cancellation::default())
            .expect_err("reject malformed JSON");

        assert!(error.to_string().contains("failed to parse"));
    }

    fn write_reply(build_dir: &Path, codemodel: &str) {
        let reply_dir = build_dir.join(".cmake/api/v1/reply");
        fs::create_dir_all(&reply_dir).expect("create reply directory");
        fs::write(
            reply_dir.join("index-test.json"),
            r#"{
                "reply": {
                    "client-cmake-ls": {
                        "codemodel-v2": {
                            "kind": "codemodel",
                            "version": { "major": 2, "minor": 9 },
                            "jsonFile": "codemodel-test.json"
                        }
                    }
                }
            }"#,
        )
        .expect("write index");
        fs::write(reply_dir.join("codemodel-test.json"), codemodel).expect("write codemodel");
    }
}

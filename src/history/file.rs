//! Single-path rename following, streamed with NUL-delimited raw path records.
use crate::git::{Cancellation, Commit, CommitId, GitStream, parse_commit};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRevision {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

pub(super) struct FileHistory {
    stream: GitStream,
    next: Option<Vec<u8>>,
    path: PathBuf,
}

impl FileHistory {
    pub(super) fn open(
        root: &Path,
        revision: &CommitId,
        path: &Path,
        cancel: Cancellation,
    ) -> Result<Self> {
        let args: Vec<OsString> = [
            "--literal-pathspecs",
            "log",
            "--follow",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--name-status",
            "-z",
            "--pretty=tformat:%H%n%P%n%an%n%ae%n%aI%n%cn%n%ce%n%cI%n%B%x00",
        ]
        .into_iter()
        .map(Into::into)
        .chain([
            revision.to_hex().into(),
            "--".into(),
            path.as_os_str().to_owned(),
        ])
        .collect();
        Ok(Self {
            stream: GitStream::spawn(root, &args, cancel, &[0])?,
            next: None,
            path: path.to_owned(),
        })
    }

    pub(super) fn next(&mut self) -> Result<Option<(Commit, FileRevision)>> {
        let header = match self.next.take() {
            Some(header) => header,
            None => match self.stream.record(0)? {
                Some(record) => {
                    ensure!(record.terminated, "Incomplete file history header");
                    record.bytes
                }
                None => return Ok(None),
            },
        };
        let commit = parse_commit(&header)?;
        let mut version = FileRevision {
            path: self.path.clone(),
            old_path: None,
        };
        while let Some(record) = self.stream.record(0)? {
            ensure!(record.terminated, "Incomplete file history record");
            let status = record.bytes.strip_prefix(b"\n").unwrap_or(&record.bytes);
            if status.is_empty() {
                continue;
            }
            if status.contains(&b'\n') {
                self.next = Some(record.bytes);
                break;
            }
            ensure!(
                matches!(status[0], b'A' | b'M' | b'D' | b'R' | b'C' | b'T' | b'U'),
                "Unexpected file history status"
            );
            let path = self.path_record()?;
            if matches!(status[0], b'R' | b'C') {
                let new = self.path_record()?;
                if new == self.path {
                    version.old_path = Some(path.clone());
                    self.path = path;
                }
            }
        }
        Ok(Some((commit, version)))
    }

    fn path_record(&mut self) -> Result<PathBuf> {
        let record = self
            .stream
            .record(0)?
            .context("Missing file history path")?;
        ensure!(record.terminated, "Incomplete file history path");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Ok(OsString::from_vec(record.bytes).into())
        }
        #[cfg(not(unix))]
        {
            Ok(String::from_utf8(record.bytes)?.into())
        }
    }
}

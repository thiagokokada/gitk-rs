//! Cancellable porcelain blame parsing; paths never pass through display text.
use crate::git::{Cancellation, CommitId, GitStream};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct Attribution {
    pub commit: CommitId,
    pub path: Arc<PathBuf>,
    pub original_line: usize,
    pub final_line: usize,
    pub author: Arc<str>,
    pub summary: Arc<str>,
    pub boundary: bool,
    pub previous: Option<(CommitId, Arc<PathBuf>)>,
}

pub struct BlameLine {
    pub attribution: Attribution,
    pub text: String,
}

pub struct BlameStream {
    stream: GitStream,
    strings: HashSet<Arc<str>>,
    paths: HashSet<Arc<PathBuf>>,
}

impl BlameStream {
    pub fn open(
        root: &Path,
        revision: &CommitId,
        path: &Path,
        cancel: Cancellation,
    ) -> Result<Self> {
        let args: Vec<OsString> = [
            "--literal-pathspecs",
            "blame",
            "--line-porcelain",
            "--encoding=UTF-8",
            "--root",
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
            strings: HashSet::new(),
            paths: HashSet::new(),
        })
    }

    fn intern(&mut self, bytes: &[u8]) -> Arc<str> {
        let value = String::from_utf8_lossy(bytes).into_owned();
        if let Some(existing) = self.strings.get(value.as_str()) {
            return existing.clone();
        }
        let value: Arc<str> = value.into();
        self.strings.insert(value.clone());
        value
    }

    fn intern_path(&mut self, path: PathBuf) -> Arc<PathBuf> {
        if let Some(existing) = self.paths.get(&path) {
            return existing.clone();
        }
        let path = Arc::new(path);
        self.paths.insert(path.clone());
        path
    }

    pub fn next_line(&mut self) -> Result<Option<BlameLine>> {
        let Some(header) = self.stream.record(b'\n')? else {
            return Ok(None);
        };
        let header = std::str::from_utf8(&header.bytes)?;
        let mut fields = header.split_whitespace();
        let commit = CommitId::parse(fields.next().context("Missing blame commit")?)?;
        let original_line = fields.next().context("Missing original line")?.parse()?;
        let final_line = fields.next().context("Missing final line")?.parse()?;
        let mut attribution = Attribution {
            commit,
            path: Arc::new(PathBuf::new()),
            original_line,
            final_line,
            author: Arc::from(""),
            summary: Arc::from(""),
            boundary: false,
            previous: None,
        };
        while let Some(record) = self.stream.record(b'\n')? {
            if let Some(text) = record.bytes.strip_prefix(b"\t") {
                ensure!(
                    !text.contains(&0),
                    "Binary file attribution is not supported"
                );
                ensure!(
                    !attribution.path.as_os_str().is_empty(),
                    "Missing blame filename"
                );
                return Ok(Some(BlameLine {
                    attribution,
                    text: String::from_utf8_lossy(text).into_owned(),
                }));
            }
            if let Some(value) = record.bytes.strip_prefix(b"author ") {
                attribution.author = self.intern(value);
            } else if let Some(value) = record.bytes.strip_prefix(b"summary ") {
                attribution.summary = self.intern(value);
            } else if let Some(value) = record.bytes.strip_prefix(b"filename ") {
                attribution.path = self.intern_path(parse_path(value)?);
            } else if record.bytes == b"boundary" {
                attribution.boundary = true;
            } else if let Some(value) = record.bytes.strip_prefix(b"previous ") {
                let split = value
                    .iter()
                    .position(|b| *b == b' ')
                    .context("Invalid previous attribution")?;
                attribution.previous = Some((
                    CommitId::parse(std::str::from_utf8(&value[..split])?)?,
                    self.intern_path(parse_path(&value[split + 1..])?),
                ));
            }
        }
        anyhow::bail!("Incomplete blame record")
    }
}

fn parse_path(bytes: &[u8]) -> Result<PathBuf> {
    let mut raw = Vec::new();
    if bytes.starts_with(b"\"") {
        ensure!(bytes.ends_with(b"\""), "Incomplete quoted blame path");
        let mut rest = &bytes[1..bytes.len() - 1];
        while !rest.is_empty() {
            let byte = rest[0];
            rest = &rest[1..];
            if byte != b'\\' {
                raw.push(byte);
                continue;
            }
            let escape = *rest.first().context("Incomplete blame path escape")?;
            rest = &rest[1..];
            raw.push(match escape {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                b'\\' => b'\\',
                b'"' => b'"',
                b'0'..=b'3' => {
                    ensure!(
                        rest.len() >= 2 && rest[..2].iter().all(|b| (b'0'..=b'7').contains(b)),
                        "Invalid octal path escape"
                    );
                    let value = (escape - b'0') * 64 + (rest[0] - b'0') * 8 + (rest[1] - b'0');
                    rest = &rest[2..];
                    value
                }
                _ => anyhow::bail!("Unknown blame path escape"),
            });
        }
    } else {
        raw.extend(bytes);
    }
    crate::diff::path_from_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_paths_preserve_tabs_newlines_and_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let path = parse_path(br#""a\t\n\377\"\\b""#).unwrap();
        assert_eq!(path.as_os_str().as_bytes(), b"a\t\n\xff\"\\b");
        assert!(parse_path(br#""bad\9""#).is_err());
    }
}

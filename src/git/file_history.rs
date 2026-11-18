//! A bounded native Git file walk. Keep original commit parents for diffs and
//! carry each historical filename separately as Git follows renames backward.
use super::*;

const MARKER: &[u8] = b"FILE_COMMIT\n";

pub(super) struct FileStream {
    stream: GitStream,
    path: Arc<Path>,
    last_path: Option<Arc<Path>>,
    follow: bool,
    done: bool,
}

impl FileStream {
    pub(super) fn open(
        root: &Path,
        revision: &CommitId,
        history: &FileHistory,
        cancel: Cancellation,
    ) -> Result<Self> {
        ensure!(
            !history.path.as_os_str().is_empty()
                && history.path.is_relative()
                && !history
                    .path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "file history requires a repository-relative path"
        );
        let format = format!(
            "--pretty=tformat:FILE_COMMIT%n{}",
            LOG_FORMAT.strip_prefix("--pretty=tformat:").unwrap()
        );
        let mut args: Vec<OsString> = [
            "--literal-pathspecs",
            "-c",
            "log.follow=false",
            "log",
            "--no-color",
            "--no-decorate",
            "--no-notes",
            "--no-show-signature",
            "--name-status",
            "-z",
        ]
        .map(OsString::from)
        .into();
        args.push(format.into());
        if history.follow_renames {
            args.extend(["--follow", "--find-renames"].map(OsString::from));
        }
        args.extend([
            revision.to_hex().into(),
            "--".into(),
            history.path.as_os_str().to_owned(),
        ]);
        Ok(Self {
            stream: GitStream::spawn(root, &args, cancel, &[0])?,
            path: Arc::from(history.path.as_path()),
            last_path: None,
            follow: history.follow_renames,
            done: false,
        })
    }

    fn record(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(record) = self.stream.record(0)? else {
            self.done = true;
            return Ok(None);
        };
        if !record.terminated && record.bytes.iter().all(|b| matches!(b, b'\n' | b'\r')) {
            self.done = true;
            return Ok(None);
        }
        ensure!(
            record.terminated,
            "Git file history ended in an incomplete record"
        );
        Ok(Some(record.bytes))
    }

    fn path_record(&mut self) -> Result<PathBuf> {
        let bytes = self
            .record()?
            .context("Git file history omitted a changed path")?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            Ok(PathBuf::from(OsString::from_vec(bytes)))
        }
        #[cfg(not(unix))]
        {
            Ok(PathBuf::from(String::from_utf8(bytes)?))
        }
    }

    pub(super) fn next_commit(&mut self) -> Result<Option<Commit>> {
        if self.done {
            return Ok(None);
        }
        while let Some(record) = self.record()? {
            if record.is_empty() {
                continue;
            }
            if let Some(metadata) = record.strip_prefix(MARKER) {
                self.last_path = Some(self.path.clone());
                return Ok(Some(parse_commit(metadata)?));
            }
            let status = record.strip_prefix(b"\n").unwrap_or(&record);
            ensure!(
                !status.is_empty() && status.iter().all(u8::is_ascii_alphanumeric),
                "invalid Git file change status"
            );
            let old = self.path_record()?;
            if matches!(status[0], b'R' | b'C') {
                let new = self.path_record()?;
                if self.follow && new.as_path() == self.path.as_ref() {
                    self.path = Arc::from(old);
                }
            }
        }
        Ok(None)
    }

    pub(super) fn file_path(&self) -> Option<Arc<Path>> {
        self.last_path.clone()
    }
}

//! Cancellable, bounded access to the system Git executable. No GUI dependency.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
pub type Cancellation = Arc<AtomicBool>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommitId(String);

impl CommitId {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(
            matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Git object ID"
        );
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub time: String,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: CommitId,
    pub parents: Vec<CommitId>,
    pub author: Signature,
    pub committer: Signature,
    pub message: String,
}

impl Commit {
    pub fn summary(&self) -> String {
        self.message
            .trim()
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(160)
            .collect()
    }

    pub fn searchable_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.id.as_str(),
            self.author.name,
            self.author.email,
            self.message
        )
        .to_lowercase()
    }

    pub fn memory_bytes(&self) -> usize {
        self.message.len()
            + self.author.name.len()
            + self.author.email.len()
            + self.committer.name.len()
            + self.committer.email.len()
            + self.parents.len() * 64
            + 256
    }
}

#[derive(Clone, Debug)]
pub struct Repository {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub head: Option<CommitId>,
    pub branch: String,
}

impl Repository {
    pub fn open(path: &Path, cancel: &Cancellation) -> Result<Self> {
        let root = command_path(path, &["rev-parse", "--show-toplevel"], cancel)?;
        let git_dir = command_path(&root, &["rev-parse", "--absolute-git-dir"], cancel)?;
        let common_dir = root
            .join(command_path(
                &root,
                &["rev-parse", "--git-common-dir"],
                cancel,
            )?)
            .canonicalize()
            .context("resolve common Git directory")?;
        let head = run_bytes(
            &root,
            &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
            cancel,
            &[0, 1],
        )?;
        let head = if head.is_empty() {
            None
        } else {
            Some(CommitId::parse(std::str::from_utf8(trim_newline(&head))?)?)
        };
        let branch = run_bytes(
            &root,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            cancel,
            &[0, 1],
        )?;
        let branch = if branch.is_empty() {
            "HEAD".into()
        } else {
            String::from_utf8_lossy(trim_newline(&branch)).into_owned()
        };
        Ok(Self {
            root,
            git_dir,
            common_dir,
            head,
            branch,
        })
    }

    pub fn log(&self, cancel: Cancellation) -> Result<GitStream> {
        let head = self.head.as_ref().context("repository has no commits")?;
        // Default Git traversal streams immediately. Date/topological ordering may
        // visit a huge graph before emitting its first record.
        GitStream::spawn(
            &self.root,
            &[
                "log",
                "--no-color",
                "--no-decorate",
                "--no-patch",
                "--pretty=tformat:%H%n%P%n%an%n%ae%n%aI%n%cn%n%ce%n%cI%n%B%x00",
                head.as_str(),
            ],
            cancel,
            &[0],
        )
    }

    pub fn refs(&self, cancel: Cancellation) -> Result<GitStream> {
        GitStream::spawn(&self.root, &["show-ref", "--dereference"], cancel, &[0, 1])
    }
}

fn trim_newline(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

fn command_path(root: &Path, args: &[&str], cancel: &Cancellation) -> Result<PathBuf> {
    let bytes = run_bytes(root, args, cancel, &[0])?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(
            trim_newline(&bytes).to_vec(),
        )))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(String::from_utf8(
            trim_newline(&bytes).to_vec(),
        )?))
    }
}

pub fn run_bytes<S: AsRef<OsStr>>(
    root: &Path,
    args: &[S],
    cancel: &Cancellation,
    allowed: &[i32],
) -> Result<Vec<u8>> {
    let mut stream = GitStream::spawn(root, args, cancel.clone(), allowed)?;
    let mut bytes = Vec::new();
    while let Some(record) = stream.record(b'\n')? {
        ensure!(
            bytes.len() + record.bytes.len() < MAX_RECORD_BYTES,
            "Git command output exceeds the 2 MiB limit"
        );
        bytes.extend_from_slice(&record.bytes);
        if record.terminated {
            bytes.push(b'\n');
        }
    }
    Ok(bytes)
}

pub struct Record {
    pub bytes: Vec<u8>,
    pub terminated: bool,
}

pub struct GitStream {
    child: Arc<Mutex<Child>>,
    stdout: BufReader<ChildStdout>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    killer: Option<JoinHandle<()>>,
    finished: Arc<AtomicBool>,
    cancel: Cancellation,
    allowed: Vec<i32>,
    status: Option<ExitStatus>,
}

impl GitStream {
    pub fn spawn<S: AsRef<OsStr>>(
        root: &Path,
        args: &[S],
        cancel: Cancellation,
        allowed: &[i32],
    ) -> Result<Self> {
        ensure!(!cancel.load(Ordering::Acquire), "Git operation cancelled");
        let mut command = Command::new("git");
        command
            .args([
                "--no-pager",
                "-c",
                "color.ui=false",
                "-c",
                "log.showSignature=false",
                "-C",
            ])
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_OPTIONAL_LOCKS", "0");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().context("start system Git")?;
        let stdout = BufReader::new(child.stdout.take().context("Git stdout missing")?);
        let mut error_pipe = child.stderr.take().context("Git stderr missing")?;
        let stderr = thread::spawn(move || {
            let mut captured = Vec::new();
            let mut chunk = [0; 4096];
            while let Ok(count) = error_pipe.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                let retain = count.min(MAX_STDERR_BYTES - captured.len());
                captured.extend_from_slice(&chunk[..retain]);
            }
            captured
        });
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let killer = {
            let child = child.clone();
            let finished = finished.clone();
            let cancel = cancel.clone();
            thread::spawn(move || {
                while !finished.load(Ordering::Acquire) {
                    if cancel.load(Ordering::Acquire) {
                        terminate(&mut child.lock().unwrap());
                        break;
                    }
                    thread::park_timeout(Duration::from_millis(5));
                }
            })
        };
        Ok(Self {
            child,
            stdout,
            stderr: Some(stderr),
            killer: Some(killer),
            finished,
            cancel,
            allowed: allowed.to_vec(),
            status: None,
        })
    }

    /// Bounds every record before allocating it, even if a producer never emits
    /// a delimiter. EOF status is checked so a failing Git process is not success.
    pub fn record(&mut self, delimiter: u8) -> Result<Option<Record>> {
        ensure!(
            !self.cancel.load(Ordering::Acquire),
            "Git operation cancelled"
        );
        let mut bytes = Vec::new();
        loop {
            let available = self.stdout.fill_buf()?;
            if available.is_empty() {
                self.finish()?;
                return Ok((!bytes.is_empty()).then_some(Record {
                    bytes,
                    terminated: false,
                }));
            }
            let end = available.iter().position(|b| *b == delimiter);
            let count = end.unwrap_or(available.len());
            ensure!(
                bytes.len() + count <= MAX_RECORD_BYTES,
                "Git record exceeds the 2 MiB limit"
            );
            bytes.extend_from_slice(&available[..count]);
            self.stdout.consume(count + usize::from(end.is_some()));
            if end.is_some() {
                return Ok(Some(Record {
                    bytes,
                    terminated: true,
                }));
            }
            ensure!(
                !self.cancel.load(Ordering::Acquire),
                "Git operation cancelled"
            );
        }
    }

    pub fn next_commit(&mut self) -> Result<Option<Commit>> {
        let Some(record) = self.record(0)? else {
            return Ok(None);
        };
        // tformat writes a final LF after the final NUL, just as it separates
        // earlier records with LF. It is not an unterminated commit record.
        if !record.terminated && record.bytes.iter().all(|b| matches!(b, b'\n' | b'\r')) {
            return Ok(None);
        }
        ensure!(record.terminated, "Git log ended in an incomplete record");
        Ok(Some(parse_commit(&record.bytes)?))
    }

    fn finish(&mut self) -> Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        let status = loop {
            if let Some(status) = self.child.lock().unwrap().try_wait()? {
                break status;
            }
            thread::sleep(Duration::from_millis(1));
        };
        self.status = Some(status);
        let stderr = self
            .stderr
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        self.finished.store(true, Ordering::Release);
        if let Some(killer) = self.killer.take() {
            killer.thread().unpark();
            let _ = killer.join();
        }
        ensure!(
            !self.cancel.load(Ordering::Acquire),
            "Git operation cancelled"
        );
        if !status
            .code()
            .is_some_and(|code| self.allowed.contains(&code))
        {
            bail!(
                "Git failed ({status}): {}",
                String::from_utf8_lossy(&stderr).trim()
            );
        }
        Ok(())
    }
}

impl Drop for GitStream {
    fn drop(&mut self) {
        if self.status.is_none() {
            terminate(&mut self.child.lock().unwrap());
            let _ = self.child.lock().unwrap().wait();
        }
        self.finished.store(true, Ordering::Release);
        if let Some(killer) = self.killer.take() {
            killer.thread().unpark();
            let _ = killer.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
    }
    let _ = child.kill();
}

pub fn parse_commit(record: &[u8]) -> Result<Commit> {
    let record = record.strip_prefix(b"\n").unwrap_or(record);
    let mut parts = record.splitn(9, |b| *b == b'\n');
    let mut next = || -> Result<String> {
        Ok(
            String::from_utf8_lossy(parts.next().context("incomplete Git commit metadata")?)
                .into_owned(),
        )
    };
    let id = CommitId::parse(&next()?)?;
    let parents = next()?
        .split_whitespace()
        .map(CommitId::parse)
        .collect::<Result<Vec<_>>>()?;
    let author = Signature {
        name: next()?,
        email: next()?,
        time: next()?,
    };
    let committer = Signature {
        name: next()?,
        email: next()?,
        time: next()?,
    };
    let message = next()?;
    Ok(Commit {
        id,
        parents,
        author,
        committer,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;

    #[test]
    fn multiline_messages_and_ids() {
        let hash = "a".repeat(40);
        let record = format!(
            "\n{hash}\n\nAuthor\na@example.test\n2026-01-01T12:30:00+01:00\nCommitter\nc@example.test\n2026-01-02T12:30:00+01:00\nSubject é\n\nBody\n"
        );
        let commit = parse_commit(record.as_bytes()).unwrap();
        assert_eq!(commit.summary(), "Subject é");
        assert_eq!(commit.message, "Subject é\n\nBody\n");
        assert!(commit.parents.is_empty());
        assert!(parse_commit(b"short\n").is_err());
        assert!(CommitId::parse(&"g".repeat(40)).is_err());
        assert!(CommitId::parse(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn streaming_unborn_and_detached_heads() {
        let fixture = Fixture::new();
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(
            Repository::open(fixture.root(), &cancel)
                .unwrap()
                .head
                .is_none()
        );
        let first = fixture.commit("first\n\nBody");
        let second = fixture.commit("second");
        let repo = Repository::open(fixture.root(), &cancel).unwrap();
        assert_eq!(repo.branch, "main");
        let mut log = repo.log(cancel.clone()).unwrap();
        assert_eq!(log.next_commit().unwrap().unwrap().id.as_str(), second);
        assert_eq!(log.next_commit().unwrap().unwrap().id.as_str(), first);
        assert!(log.next_commit().unwrap().is_none());
        fixture.git(&["checkout", "--quiet", "--detach"]);
        assert_eq!(
            Repository::open(fixture.root(), &cancel).unwrap().branch,
            "HEAD"
        );
    }

    #[test]
    fn oversized_records_are_rejected_before_unbounded_allocation() {
        let fixture = Fixture::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut stream = GitStream::spawn(
            fixture.root(),
            &["-c", "alias.flood=!head -c 3145728 /dev/zero", "flood"],
            cancel,
            &[0],
        )
        .unwrap();
        assert!(
            stream
                .record(b'\n')
                .err()
                .unwrap()
                .to_string()
                .contains("2 MiB")
        );
    }

    #[test]
    fn stderr_is_drained_and_exit_errors_are_reported() {
        let fixture = Fixture::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let error = run_bytes(
            fixture.root(),
            &[
                "-c",
                "alias.failure=!head -c 131072 /dev/zero >&2; exit 7",
                "failure",
            ],
            &cancel,
            &[0],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Git failed"));
        assert!(error.len() < MAX_STDERR_BYTES + 256);
    }

    #[test]
    fn cancellation_kills_helpers_and_unblocks_reading() {
        let fixture = Fixture::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut stream = GitStream::spawn(
            fixture.root(),
            &[
                "-c",
                "alias.blocked=!printf 'ready\\n'; sleep 30",
                "blocked",
            ],
            cancel.clone(),
            &[0],
        )
        .unwrap();
        assert_eq!(stream.record(b'\n').unwrap().unwrap().bytes, b"ready");
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let failed = stream.record(b'\n').is_err();
            drop(stream);
            sender.send(failed).unwrap();
        });
        cancel.store(true, Ordering::Release);
        assert!(receiver.recv_timeout(Duration::from_secs(3)).unwrap());
        worker.join().unwrap();
    }
}

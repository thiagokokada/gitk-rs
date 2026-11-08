//! Cancellable, bounded access to the system Git executable. No GUI dependency.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Command, ExitStatus, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Shared cancellation flag with a notification for blocked process monitors.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<CancellationState>);

#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    waiting: Mutex<()>,
    changed: Condvar,
}

impl Cancellation {
    pub fn load(&self, ordering: Ordering) -> bool {
        self.0.cancelled.load(ordering)
    }

    pub fn store(&self, value: bool, ordering: Ordering) {
        let _guard = self.0.waiting.lock().unwrap();
        self.0.cancelled.store(value, ordering);
        self.0.changed.notify_all();
    }

    fn wake(&self) {
        // Synchronize with the predicate-to-wait transition, including when a
        // child finishes just before its monitor begins waiting.
        let _guard = self.0.waiting.lock().unwrap();
        self.0.changed.notify_all();
    }

    fn wait(&self, finished: &AtomicBool) {
        let guard = self.0.waiting.lock().unwrap();
        drop(
            self.0
                .changed
                .wait_while(guard, |_| {
                    !self.load(Ordering::Acquire) && !finished.load(Ordering::Acquire)
                })
                .unwrap(),
        );
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CommitId {
    Sha1([u8; 20]),
    Sha256([u8; 32]),
}

impl CommitId {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(
            matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Git object ID"
        );
        let mut bytes = [0; 32];
        for (target, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => unreachable!("validated hexadecimal digit"),
            };
            *target = digit(pair[0]) << 4 | digit(pair[1]);
        }
        Ok(if value.len() == 40 {
            Self::Sha1(bytes[..20].try_into().unwrap())
        } else {
            Self::Sha256(bytes)
        })
    }

    pub fn to_hex(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for CommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes: &[u8] = match self {
            Self::Sha1(bytes) => bytes,
            Self::Sha256(bytes) => bytes,
        };
        let mut hex = [0; 64];
        for (byte, pair) in bytes.iter().zip(hex.as_chunks_mut::<2>().0.iter_mut()) {
            pair[0] = b"0123456789abcdef"[(byte >> 4) as usize];
            pair[1] = b"0123456789abcdef"[(byte & 15) as usize];
        }
        f.write_str(std::str::from_utf8(&hex[..bytes.len() * 2]).unwrap())
    }
}

#[derive(Clone, Debug)]
pub struct Signature {
    pub name: Arc<str>,
    pub email: Arc<str>,
    pub time: Box<str>,
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub id: CommitId,
    pub parents: Box<[CommitId]>,
    pub author: Signature,
    pub committer: Signature,
    pub message: Box<str>,
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
            self.id, self.author.name, self.author.email, self.message
        )
    }

    pub fn memory_bytes(&self) -> usize {
        self.message.len()
            + self.author.name.len()
            + self.author.email.len()
            + self.committer.name.len()
            + self.committer.email.len()
            + self.author.time.len()
            + self.committer.time.len()
            + size_of_val(self.parents.as_ref())
            + size_of::<Self>()
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
                &head.to_hex(),
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

pub(crate) fn command_path(root: &Path, args: &[&str], cancel: &Cancellation) -> Result<PathBuf> {
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
    input: Option<JoinHandle<Result<()>>>,
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
        Self::spawn_inner(root, args, cancel, allowed, None)
    }

    /// Feeds bounded batches of object IDs while stdout and stderr are drained.
    pub(crate) fn spawn_input<S: AsRef<OsStr>>(
        root: &Path,
        args: &[S],
        cancel: Cancellation,
        allowed: &[i32],
        input: Receiver<Vec<CommitId>>,
    ) -> Result<Self> {
        Self::spawn_inner(root, args, cancel, allowed, Some(input))
    }

    fn spawn_inner<S: AsRef<OsStr>>(
        root: &Path,
        args: &[S],
        cancel: Cancellation,
        allowed: &[i32],
        input: Option<Receiver<Vec<CommitId>>>,
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
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_OPTIONAL_LOCKS", "0");
        if input.is_some() {
            command.env("GIT_FLUSH", "1");
        }
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
        let finished = Arc::new(AtomicBool::new(false));
        let input = input.map(|receiver| {
            let mut stdin = std::io::BufWriter::new(child.stdin.take().expect("piped Git stdin"));
            let cancel = cancel.clone();
            let finished = finished.clone();
            thread::spawn(move || -> Result<()> {
                while !cancel.load(Ordering::Acquire) && !finished.load(Ordering::Acquire) {
                    match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(ids) => {
                            for id in ids {
                                writeln!(stdin, "{id}")?;
                            }
                            stdin.flush()?;
                        }
                        Err(RecvTimeoutError::Timeout) => (),
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                Ok(())
            })
        });
        let child = Arc::new(Mutex::new(child));
        let killer = {
            let child = child.clone();
            let finished = finished.clone();
            let cancel = cancel.clone();
            thread::spawn(move || {
                cancel.wait(&finished);
                if cancel.load(Ordering::Acquire) && !finished.load(Ordering::Acquire) {
                    terminate(&mut child.lock().unwrap());
                }
            })
        };
        Ok(Self {
            child,
            stdout,
            stderr: Some(stderr),
            input,
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
        self.cancel.wake();
        if let Some(killer) = self.killer.take() {
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
        if let Some(input) = self.input.take() {
            input
                .join()
                .map_err(|_| anyhow::anyhow!("Git input worker panicked"))??;
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
        self.cancel.wake();
        if let Some(killer) = self.killer.take() {
            let _ = killer.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
        if let Some(input) = self.input.take() {
            let _ = input.join();
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
        .collect::<Result<Vec<_>>>()?
        .into_boxed_slice();
    let author = Signature {
        name: next()?.into(),
        email: next()?.into(),
        time: next()?.into_boxed_str(),
    };
    let committer = Signature {
        name: next()?.into(),
        email: next()?.into(),
        time: next()?.into_boxed_str(),
    };
    let message = next()?.into_boxed_str();
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
        assert_eq!(commit.message.as_ref(), "Subject é\n\nBody\n");
        assert!(commit.parents.is_empty());
        assert!(parse_commit(b"short\n").is_err());
        assert!(CommitId::parse(&"g".repeat(40)).is_err());
        assert!(CommitId::parse(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn binary_ids_preserve_hash_format_case_normalization_and_identity() {
        for hex in [
            "0123456789aBcDeF".repeat(4),
            "0123456789aBcDeF".repeat(4)[..40].to_owned(),
        ] {
            let id = CommitId::parse(&hex).unwrap();
            assert_eq!(id.to_hex(), hex.to_ascii_lowercase());
            assert_eq!(id, CommitId::parse(&hex.to_ascii_lowercase()).unwrap());
        }
        let sha1 = CommitId::parse(&"a".repeat(40)).unwrap();
        let sha256 = CommitId::parse(&format!("{}{}", "a".repeat(40), "0".repeat(24))).unwrap();
        assert_ne!(sha1, sha256);
        for invalid in [
            "é".repeat(20),
            "a".repeat(39),
            "a".repeat(41),
            "a".repeat(65),
        ] {
            assert!(CommitId::parse(&invalid).is_err());
        }
    }

    #[test]
    fn sha256_repository_streams_binary_ids_and_parents() {
        let fixture = Fixture::new();
        fixture.git(&["init", "--quiet", "--object-format=sha256", "sha256"]);
        fixture.git(&[
            "-C",
            "sha256",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "root",
        ]);
        let root = fixture.git(&["-C", "sha256", "rev-parse", "HEAD"]);
        fixture.git(&[
            "-C",
            "sha256",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "tip",
        ]);
        let cancellation = Cancellation::default();
        let repository = Repository::open(&fixture.root().join("sha256"), &cancellation).unwrap();
        let mut stream = repository.log(cancellation).unwrap();
        let tip = stream.next_commit().unwrap().unwrap();
        assert_eq!(tip.id.to_hex().len(), 64);
        assert_eq!(tip.parents[0].to_hex(), root);
        assert_eq!(stream.next_commit().unwrap().unwrap().id.to_hex(), root);
        assert!(stream.next_commit().unwrap().is_none());
    }

    #[test]
    fn streaming_unborn_and_detached_heads() {
        let fixture = Fixture::new();
        let cancel = Cancellation::default();
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
        assert_eq!(log.next_commit().unwrap().unwrap().id.to_hex(), second);
        assert_eq!(log.next_commit().unwrap().unwrap().id.to_hex(), first);
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
        let cancel = Cancellation::default();
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
        let cancel = Cancellation::default();
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
        // Create the helper before readiness, so cancellation cannot race its fork.
        let fixture = Fixture::new();
        let cancel = Cancellation::default();
        let mut stream = GitStream::spawn(
            fixture.root(),
            &[
                "-c",
                "alias.blocked=!sleep 30 & helper=$!; printf 'ready\\n'; wait \"$helper\"",
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

    #[test]
    fn cancellation_reaps_a_process_with_backpressured_input_and_output() {
        let fixture = Fixture::new();
        let id = CommitId::parse(&fixture.commit("candidate")).unwrap();
        let cancel = Cancellation::default();
        let (input, incoming) = std::sync::mpsc::sync_channel(1);
        let mut stream = GitStream::spawn_input(
            fixture.root(),
            &[
                "diff-tree",
                "--stdin",
                "--always",
                "--root",
                "-r",
                "--raw",
                "-z",
            ],
            cancel.clone(),
            &[0],
            incoming,
        )
        .unwrap();
        input.send(vec![id; 10000]).unwrap();
        assert!(stream.record(0).unwrap().is_some());
        let (done, finished) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            drop(stream);
            done.send(()).unwrap();
        });
        cancel.store(true, Ordering::Release);
        finished.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn shared_cancellation_wakes_all_monitors_after_another_child_finishes() {
        let fixture = Fixture::new();
        let cancel = Cancellation::default();
        let mut streams = Vec::new();
        for _ in 0..2 {
            let mut stream = GitStream::spawn(
                fixture.root(),
                &[
                    "-c",
                    "alias.blocked=!sleep 30 & helper=$!; printf 'ready\\n'; wait \"$helper\"",
                    "blocked",
                ],
                cancel.clone(),
                &[0],
            )
            .unwrap();
            assert_eq!(stream.record(b'\n').unwrap().unwrap().bytes, b"ready");
            streams.push(stream);
        }
        run_bytes(fixture.root(), &["rev-parse", "--git-dir"], &cancel, &[0]).unwrap();
        assert!(!cancel.load(Ordering::Acquire));
        assert!(
            streams
                .iter()
                .all(|stream| stream.child.lock().unwrap().try_wait().unwrap().is_none())
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        let workers = streams
            .into_iter()
            .map(|mut stream| {
                let sender = sender.clone();
                thread::spawn(move || {
                    let failed = stream.record(b'\n').is_err();
                    drop(stream);
                    sender.send(failed).unwrap();
                })
            })
            .collect::<Vec<_>>();
        cancel.store(true, Ordering::Release);
        for _ in 0..2 {
            assert!(receiver.recv_timeout(Duration::from_secs(3)).unwrap());
        }
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn rapid_normal_completion_cannot_miss_the_monitor_wakeup() {
        let fixture = Fixture::new();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let cancel = Cancellation::default();
            for _ in 0..32 {
                run_bytes(fixture.root(), &["rev-parse", "--git-dir"], &cancel, &[0]).unwrap();
            }
            sender.send(()).unwrap();
        });
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn linked_worktree_resolves_its_common_directory_and_history() {
        let fixture = Fixture::new();
        fixture.commit("first");
        fixture.commit("second");
        let linked = tempfile::tempdir().unwrap();
        fixture.git(&[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            linked.path().to_str().unwrap(),
        ]);
        let cancel = Cancellation::default();
        let repository = Repository::open(linked.path(), &cancel).unwrap();
        assert_eq!(
            repository.common_dir,
            fixture.root().join(".git").canonicalize().unwrap()
        );
        assert_ne!(repository.git_dir, repository.common_dir);
        assert_eq!(repository.branch, "linked");
        assert_eq!(
            repository
                .log(cancel)
                .unwrap()
                .next_commit()
                .unwrap()
                .unwrap()
                .summary(),
            "second"
        );
    }

    #[test]
    fn shallow_clone_streams_a_complete_boundary_commit() {
        let fixture = Fixture::new();
        fixture.commit("first");
        fixture.commit("second");
        let clone = tempfile::tempdir().unwrap();
        fixture.git(&[
            "clone",
            "--quiet",
            "--no-local",
            "--depth",
            "1",
            fixture.root().to_str().unwrap(),
            clone.path().to_str().unwrap(),
        ]);
        let cancel = Cancellation::default();
        let repository = Repository::open(clone.path(), &cancel).unwrap();
        let mut stream = repository.log(cancel).unwrap();
        let commit = stream.next_commit().unwrap().unwrap();
        assert_eq!(commit.summary(), "second");
        assert!(commit.parents.is_empty());
        assert!(stream.next_commit().unwrap().is_none());
    }
}

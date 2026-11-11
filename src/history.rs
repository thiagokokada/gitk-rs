//! Loaded entries live once in memory. The GUI receives only nearby Arc
//! references; neither history nor filter results persist on disk.
use crate::{
    git::{Cancellation, Commit, CommitId, CommitStream, GitStream, Repository, run_bytes},
    graph::{GraphBuilder, GraphRow},
    search::{CompiledQuery, Query},
};
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const PAGE_ROWS: usize = 512;
const BATCH_BYTES: usize = 2 * 1024 * 1024;
const RELOAD_STEP_COMMITS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Generation(pub u64);

/// Persistent invalidation token; increment to restart traversal and discard
/// retained history. Carry it on subsequent viewport/filter requests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReloadGeneration(pub u64);

#[derive(Clone, Debug)]
pub struct Request {
    pub generation: Generation,
    pub reload_generation: ReloadGeneration,
    pub id: u64,
    pub refs_revision: u64,
    pub repository: PathBuf,
    pub range: Range<usize>,
    pub query: String,
    pub regex: bool,
    /// None streams to EOF; Some(N) pauses after satisfying browsing demand in
    /// batches of N. Worker steps and the snapshot mailbox stay bounded either way.
    pub batch: Option<usize>,
    pub graph_columns: usize,
    pub history_readers: usize,
    pub anchor: Option<usize>,
    pub restore: Option<Restore>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restore {
    pub selected: Option<CommitId>,
    pub top: Option<CommitId>,
}

#[derive(Clone, Debug)]
pub struct Restored {
    pub selected: Option<(usize, Arc<Entry>)>,
    pub top: Option<usize>,
    pub pending: bool,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub ordinal: usize,
    pub commit: Commit,
    pub graph: GraphRow,
}

#[derive(Clone, Debug)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Branch,
    Remote,
    Tag,
}
pub type Refs = HashMap<CommitId, Vec<RefLabel>>;

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub generation: Generation,
    pub reload_generation: ReloadGeneration,
    pub request_id: u64,
    pub start: usize,
    pub rows: Vec<Arc<Entry>>,
    pub visible: usize,
    pub loaded: usize,
    pub eof: bool,
    /// Active worker time for this generation, excluding on-demand idle pauses.
    pub load_duration: Duration,
    pub filtering: bool,
    pub branch: String,
    pub refs: Arc<Refs>,
    pub error: Option<String>,
    pub query_error: Option<String>,
    pub anchor_position: Option<usize>,
    pub restored: Option<Restored>,
}

struct SharedState {
    request: Option<Request>,
    snapshot: Option<Snapshot>,
    cancellation: Cancellation,
    generation: Generation,
    reload_generation: ReloadGeneration,
    repository: PathBuf,
    graph_columns: usize,
    history_readers: usize,
    stopped: bool,
}

struct Shared {
    state: Mutex<SharedState>,
    changed: Condvar,
}

#[derive(Clone)]
pub struct HistoryHandle {
    shared: Arc<Shared>,
}

pub struct HistoryEngine {
    handle: HistoryHandle,
    worker: Option<JoinHandle<()>>,
}

impl HistoryEngine {
    pub fn spawn(initial: Request) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(SharedState {
                generation: initial.generation,
                reload_generation: initial.reload_generation,
                repository: initial.repository.clone(),
                graph_columns: initial.graph_columns,
                history_readers: initial.history_readers,
                request: Some(initial),
                snapshot: None,
                cancellation: Cancellation::default(),
                stopped: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let worker = thread::spawn(move || run(worker_shared));
        Self {
            handle: HistoryHandle { shared },
            worker: Some(worker),
        }
    }

    pub fn handle(&self) -> HistoryHandle {
        self.handle.clone()
    }
}

impl Drop for HistoryEngine {
    fn drop(&mut self) {
        {
            let mut state = self.handle.shared.state.lock().unwrap();
            state.stopped = true;
            state.cancellation.store(true, Ordering::Release);
        }
        self.handle.shared.changed.notify_all();
        // The owner lives outside Application::run, so this never joins on the UI loop.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl HistoryHandle {
    /// Replaces obsolete demand rather than accumulating a queue during scrolling.
    pub fn request(&self, request: Request) {
        let old = {
            let mut state = self.shared.state.lock().unwrap();
            // A UI generation invalidates snapshots, not necessarily the Git
            // traversal. Let the worker inspect HEAD before replacing a stream.
            if request.repository != state.repository
                || request.graph_columns != state.graph_columns
                || request.history_readers != state.history_readers
                || request.reload_generation != state.reload_generation
            {
                state.cancellation.store(true, Ordering::Release);
                state.cancellation = Cancellation::default();
            }
            let snapshot = if request.generation != state.generation
                || request.reload_generation != state.reload_generation
            {
                state.snapshot.take()
            } else {
                None
            };
            state.generation = request.generation;
            state.reload_generation = request.reload_generation;
            state.repository = request.repository.clone();
            state.graph_columns = request.graph_columns;
            state.history_readers = request.history_readers;
            (state.request.replace(request), snapshot)
        };
        drop(old);
        self.shared.changed.notify_one();
    }

    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.shared.state.lock().unwrap().snapshot.take()
    }
}

fn publish(shared: &Shared, snapshot: Snapshot) {
    let old = {
        let mut state = shared.state.lock().unwrap();
        if snapshot.generation != state.generation
            || snapshot.reload_generation != state.reload_generation
            || state.stopped
        {
            return;
        }
        state.snapshot.replace(snapshot)
    };
    drop(old);
}

fn run(shared: Arc<Shared>) {
    let mut session: Option<Session> = None;
    let mut current: Option<Request> = None;
    let mut work = false;
    let mut failed = false;
    let mut load_duration = Duration::ZERO;
    let mut completed = false;
    loop {
        let (cancel, incoming) = {
            let mut state = shared.state.lock().unwrap();
            while state.request.is_none() && !work && !state.stopped {
                state = shared.changed.wait(state).unwrap();
            }
            if state.stopped {
                break;
            }
            (state.cancellation.clone(), state.request.take())
        };
        let step_started = Instant::now();
        let mut reload = false;
        if let Some(request) = incoming {
            if current.as_ref().is_none_or(|previous| {
                previous.generation != request.generation
                    || previous.history_readers != request.history_readers
                    || previous.reload_generation != request.reload_generation
            }) {
                reload = true;
                load_duration = Duration::ZERO;
                completed = false;
                if failed
                    || current.as_ref().is_none_or(|previous| {
                        previous.repository != request.repository
                            || previous.graph_columns != request.graph_columns
                            || previous.history_readers != request.history_readers
                            || previous.reload_generation != request.reload_generation
                    })
                {
                    // Git cleanup must happen outside the shared UI mailbox lock.
                    session = None;
                }
                failed = false;
            }
            current = Some(request);
        }
        let request = current.as_ref().unwrap();
        let result = (|| -> Result<(Option<Snapshot>, bool)> {
            if failed {
                return Ok((
                    Some(empty_snapshot(
                        request,
                        Some("Reload to retry the failed history operation.".into()),
                    )),
                    false,
                ));
            }
            if reload
                && let Some(retained) = session.as_mut()
                && !retained.refresh(request)?
            {
                session = None;
            }
            if session.is_none() {
                session = Some(Session::open(request, cancel.clone())?);
            }
            match session.as_mut().unwrap().advance_reload()? {
                ReloadProgress::Pending => return Ok((None, true)),
                ReloadProgress::Replace => {
                    session = Some(Session::open(request, cancel.clone())?);
                }
                ReloadProgress::Ready => (),
            }
            let retained = session.as_mut().unwrap();
            let work = retained.step(request)?;
            // Do not restore the viewport using the old HEAD or an incomplete
            // prefix filter. Keep the existing UI snapshot until reconciliation.
            let snapshot = (retained.prefix_search.is_none() || retained.query_error.is_some())
                .then(|| retained.snapshot(request));
            Ok((snapshot, work))
        })();
        if !completed {
            load_duration += step_started.elapsed();
        }
        match result {
            Ok((snapshot, pending)) => {
                if let Some(mut snapshot) = snapshot {
                    snapshot.load_duration = load_duration;
                    completed |= snapshot.eof && !snapshot.filtering && snapshot.error.is_none();
                    publish(&shared, snapshot);
                }
                work = pending;
            }
            Err(error) => {
                if !cancel.load(Ordering::Acquire) {
                    let mut snapshot = session
                        .as_ref()
                        .map(|s| s.snapshot(request))
                        .unwrap_or_else(|| empty_snapshot(request, None));
                    snapshot.error = Some(format!("{error:#}"));
                    snapshot.load_duration = load_duration;
                    publish(&shared, snapshot);
                    if let Some(session) = session.as_mut() {
                        session.stream = None;
                    }
                    failed = true;
                }
                work = false;
            }
        }
    }
}

fn empty_snapshot(request: &Request, error: Option<String>) -> Snapshot {
    Snapshot {
        generation: request.generation,
        reload_generation: request.reload_generation,
        request_id: request.id,
        start: 0,
        rows: Vec::new(),
        visible: 0,
        loaded: 0,
        eof: false,
        load_duration: Duration::ZERO,
        filtering: false,
        branch: String::new(),
        refs: Arc::new(Refs::new()),
        error,
        query_error: None,
        anchor_position: None,
        restored: None,
    }
}

// Only metadata candidates are sent through diff-tree. Both directions are
// bounded, and cancellation owns the process and all pipe threads.
struct DiffSearch {
    cancel: Cancellation,
    input: Option<SyncSender<Vec<CommitId>>>,
    receiver: Option<Receiver<Result<(CommitId, bool)>>>,
    worker: Option<JoinHandle<()>>,
    done: bool,
}

impl DiffSearch {
    fn start(repository: &Repository, query: &CompiledQuery) -> Option<Self> {
        if query.path.is_none() && query.contents.is_none() {
            return None;
        }
        let root = repository.root.clone();
        let path = query.path.clone();
        let contents = query.contents.clone();
        let cancel = Cancellation::default();
        let token = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(256);
        let (input, incoming) = mpsc::sync_channel(2);
        let worker = thread::spawn(move || {
            let result = (|| -> Result<()> {
                let mut args = vec![
                    "diff-tree",
                    "--stdin",
                    "--always",
                    "--root",
                    "-r",
                    "--raw",
                    "-z",
                    "--no-renames",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--diff-merges=first-parent",
                ];
                let prefilter = contents
                    .as_ref()
                    .and_then(|pattern| pattern.git_literal_prefilter());
                if let Some(pattern) = &prefilter {
                    // --always retains one header per candidate, even on a miss.
                    // Keep all changed paths so path/contents remain commit-level AND.
                    args.extend(["-G", pattern, "--pickaxe-all"]);
                }
                if contents.is_some() {
                    args.extend([
                        "--patch",
                        "--unified=0",
                        "--no-color",
                        "--format=%x00%H%x0a",
                    ]);
                }
                let mut stream = GitStream::spawn_input(&root, &args, token, &[0], incoming)?;
                let mut current = None;
                let mut path_matched = path.is_none();
                let mut contents_matched = contents.is_none();
                let mut raw = true;
                let mut in_hunk = false;
                while let Some(record) = stream.record(if raw { 0 } else { b'\n' })? {
                    ensure!(record.terminated, "Incomplete diff-search record");
                    if raw && record.bytes.is_empty() {
                        raw = false;
                        continue;
                    }
                    let bytes = if raw {
                        record.bytes.strip_prefix(b"\n").unwrap_or(&record.bytes)
                    } else {
                        &record.bytes
                    };
                    if raw && bytes.starts_with(b":") {
                        ensure!(current.is_some(), "Diff-search change without a commit");
                        let changed = stream
                            .record(0)?
                            .ok_or_else(|| anyhow::anyhow!("Missing changed path"))?;
                        ensure!(changed.terminated, "Incomplete changed path");
                        if !path_matched {
                            path_matched = path
                                .as_ref()
                                .unwrap()
                                .is_match(&String::from_utf8_lossy(&changed.bytes));
                        }
                    } else if raw
                        || bytes.starts_with(&[0])
                        || (matches!(bytes.len(), 40 | 64)
                            && bytes.iter().all(u8::is_ascii_hexdigit))
                    {
                        // Headers are NUL-prefixed and newline-terminated, followed
                        // by the format terminator NUL. Empty commits can consume
                        // the leading NUL while switching out of raw mode. Patch
                        // content always has a diff prefix, so it cannot collide.
                        let header = bytes.strip_prefix(&[0]).unwrap_or(bytes);
                        let header = header.strip_suffix(&[0]).unwrap_or(header);
                        let id = CommitId::parse(std::str::from_utf8(header)?)?;
                        if contents.is_some() {
                            let terminator = stream
                                .record(0)?
                                .ok_or_else(|| anyhow::anyhow!("Missing diff header terminator"))?;
                            ensure!(
                                terminator.terminated && terminator.bytes.is_empty(),
                                "Invalid diff header terminator"
                            );
                        }
                        if let Some(previous) = current.replace(id)
                            && sender
                                .send(Ok((previous, path_matched && contents_matched)))
                                .is_err()
                        {
                            return Ok(());
                        }
                        path_matched = path.is_none();
                        contents_matched = contents.is_none();
                        in_hunk = false;
                        raw = true;
                    } else if record.bytes.starts_with(b"diff --git ") {
                        in_hunk = false;
                    } else if record.bytes.starts_with(b"@@ ") {
                        in_hunk = true;
                    } else if in_hunk
                        && !contents_matched
                        && matches!(record.bytes.first(), Some(b'+' | b'-'))
                    {
                        contents_matched = contents
                            .as_ref()
                            .unwrap()
                            .is_match(&String::from_utf8_lossy(&record.bytes[1..]));
                    }
                }
                if let Some(id) = current {
                    let _ = sender.send(Ok((id, path_matched && contents_matched)));
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        Some(Self {
            cancel,
            input: Some(input),
            receiver: Some(receiver),
            worker: Some(worker),
            done: false,
        })
    }
}

impl Drop for DiffSearch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.input.take();
        self.receiver.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum ReloadProgress {
    Pending,
    Ready,
    Replace,
}

struct LinearReload {
    repository: Repository,
    stream: CommitStream,
    old_head: CommitId,
    expected: CommitId,
    entries: Vec<Arc<Entry>>,
    graph: GraphBuilder,
    strings: HashSet<Arc<str>>,
}

impl LinearReload {
    fn step(
        &mut self,
        retained_strings: &HashSet<Arc<str>>,
        cancellation: &Cancellation,
    ) -> Result<ReloadProgress> {
        let mut bytes = 0;
        for _ in 0..RELOAD_STEP_COMMITS {
            ensure!(
                !cancellation.load(Ordering::Acquire),
                "Git operation cancelled"
            );
            let Some(mut commit) = self.stream.next_commit()? else {
                return Ok(ReloadProgress::Replace);
            };
            if commit.id != self.expected || commit.parents.len() != 1 {
                return Ok(ReloadProgress::Replace);
            }
            bytes += commit.memory_bytes();
            self.expected = commit.parents[0].clone();
            for signature in [&mut commit.author, &mut commit.committer] {
                for value in [&mut signature.name, &mut signature.email] {
                    if let Some(shared) = retained_strings
                        .get(value.as_ref())
                        .or_else(|| self.strings.get(value.as_ref()))
                    {
                        *value = shared.clone();
                    } else {
                        self.strings.insert(value.clone());
                    }
                }
            }
            self.entries.push(Arc::new(Entry {
                ordinal: self.entries.len(),
                graph: self.graph.push(&commit),
                commit,
            }));
            if self.expected == self.old_head {
                return Ok(ReloadProgress::Ready);
            }
            if bytes >= BATCH_BYTES {
                break;
            }
        }
        Ok(ReloadProgress::Pending)
    }
}

struct PrefixSearch {
    range: Range<usize>,
    suffix_matches: Vec<usize>,
    suffix_candidates: Vec<(CommitId, usize)>,
}

struct Session {
    repository: Repository,
    stream: Option<CommitStream>,
    cancellation: Cancellation,
    entries: Vec<Arc<Entry>>,
    strings: HashSet<Arc<str>>,
    graph: GraphBuilder,
    refs: Arc<Refs>,
    refs_loaded: bool,
    refs_revision: u64,
    eof: bool,
    query: String,
    regex: bool,
    parsed: CompiledQuery,
    query_error: Option<String>,
    paths: Option<DiffSearch>,
    path_pending: Vec<CommitId>,
    candidates: HashMap<CommitId, usize>,
    matches: Vec<usize>,
    scanned: usize,
    restore: Option<Restore>,
    restore_ordinals: [Option<usize>; 2],
    restore_reachable: [bool; 2],
    reusable: bool,
    reload: Option<LinearReload>,
    prefix_search: Option<PrefixSearch>,
}

impl Session {
    fn open(request: &Request, cancellation: Cancellation) -> Result<Self> {
        let repository = Repository::open(&request.repository, &cancellation)?;
        let reusable = history_reusable(&repository, &cancellation)?;
        let stream = repository
            .head
            .as_ref()
            .map(|_| repository.log_with_readers(cancellation.clone(), request.history_readers))
            .transpose()?;
        let eof = stream.is_none();
        Ok(Self {
            repository,
            stream,
            cancellation,
            entries: Vec::new(),
            strings: HashSet::new(),
            graph: GraphBuilder::new(request.graph_columns),
            refs: Arc::new(Refs::new()),
            refs_loaded: false,
            refs_revision: 0,
            eof,
            query: String::new(),
            regex: false,
            parsed: CompiledQuery::default(),
            query_error: None,
            paths: None,
            path_pending: Vec::new(),
            candidates: HashMap::new(),
            matches: Vec::new(),
            scanned: 0,
            restore: None,
            restore_ordinals: [None; 2],
            restore_reachable: [false; 2],
            reusable,
            reload: None,
            prefix_search: None,
        })
    }

    /// Revalidate traversal inputs on the worker before carrying data into a
    /// new UI generation. Retain both loaded entries and the unread stream.
    fn refresh(&mut self, request: &Request) -> Result<bool> {
        self.reload = None;
        if self.prefix_search.take().is_some() {
            // An unfinished reconciliation was superseded. Keep the commits,
            // and let step rebuild the new request's search in bounded chunks.
            self.query.clear();
            self.paths = None;
            self.path_pending.clear();
            self.candidates.clear();
            self.matches.clear();
            self.scanned = 0;
        }
        if self.cancellation.load(Ordering::Acquire) || self.query_error.is_some() {
            return Ok(false);
        }
        let repository = Repository::open(&request.repository, &self.cancellation)?;
        if !self.reusable
            || repository.root != self.repository.root
            || repository.git_dir != self.repository.git_dir
            || repository.common_dir != self.repository.common_dir
            || !history_reusable(&repository, &self.cancellation)?
        {
            return Ok(false);
        }
        if repository.head != self.repository.head {
            let Some(old_head) = self.repository.head.clone() else {
                return Ok(false);
            };
            let Some(expected) = repository.head.clone() else {
                return Ok(false);
            };
            if self.entries.is_empty() {
                return Ok(false);
            }
            self.reload = Some(LinearReload {
                stream: repository
                    .log_with_readers(self.cancellation.clone(), request.history_readers)?,
                repository,
                old_head,
                expected,
                entries: Vec::new(),
                graph: GraphBuilder::new(request.graph_columns),
                strings: HashSet::new(),
            });
            return Ok(true);
        }
        self.repository = repository;
        self.refs_loaded = false;
        Ok(true)
    }

    fn advance_reload(&mut self) -> Result<ReloadProgress> {
        let Some(mut reload) = self.reload.take() else {
            return Ok(ReloadProgress::Ready);
        };
        match reload.step(&self.strings, &self.cancellation)? {
            ReloadProgress::Pending => {
                self.reload = Some(reload);
                Ok(ReloadProgress::Pending)
            }
            ReloadProgress::Replace => Ok(ReloadProgress::Replace),
            ReloadProgress::Ready => {
                self.repository = reload.repository;
                self.refs_loaded = false;
                self.strings.extend(reload.strings);
                self.prepend(reload.entries);
                Ok(ReloadProgress::Ready)
            }
        }
    }

    fn prepend(&mut self, prefix: Vec<Arc<Entry>>) {
        let offset = prefix.len();
        // This single-lane prefix leaves the graph builder at the old HEAD in
        // precisely its original state, except for the incoming edge at that row.
        Arc::make_mut(&mut self.entries[0]).graph.incoming = true;
        for entry in &mut self.entries {
            Arc::make_mut(entry).ordinal += offset;
        }
        for ordinal in &mut self.matches {
            *ordinal += offset;
        }
        for ordinal in self.candidates.values_mut() {
            *ordinal += offset;
        }
        self.scanned += offset;

        self.entries.splice(0..0, prefix);

        if !self.query.is_empty() {
            // Filter the prefix separately before joining the known suffix.
            // This avoids both a delta-sized candidate queue and repeated
            // insertion of prefix matches at the front of a large suffix.
            self.paths = None;
            self.paths = DiffSearch::start(&self.repository, &self.parsed);
            let mut suffix_candidates: Vec<_> = self.candidates.drain().collect();
            suffix_candidates.sort_unstable_by_key(|(_, ordinal)| *ordinal);
            self.path_pending.clear();
            self.prefix_search = Some(PrefixSearch {
                range: 0..offset,
                suffix_matches: std::mem::take(&mut self.matches),
                suffix_candidates,
            });
        }
        self.restore = None;
        self.restore_ordinals = [None; 2];
        self.restore_reachable = [false; 2];
    }

    fn intern(&mut self, commit: &mut Commit) {
        for signature in [&mut commit.author, &mut commit.committer] {
            for value in [&mut signature.name, &mut signature.email] {
                if let Some(shared) = self.strings.get(value.as_ref()) {
                    *value = shared.clone();
                } else {
                    self.strings.insert(value.clone());
                }
            }
        }
    }

    fn step(&mut self, request: &Request) -> Result<bool> {
        if self.restore != request.restore {
            self.restore = request.restore.clone();
            self.restore_ordinals = [None; 2];
            self.restore_reachable = [false; 2];
            if let Some(restore) = &self.restore {
                for (index, target) in [&restore.selected, &restore.top].into_iter().enumerate() {
                    if let Some(target) = target {
                        self.restore_ordinals[index] =
                            self.entries.iter().position(|e| &e.commit.id == target);
                        // Avoid streaming an unrelated branch to EOF just to find
                        // a bookmark which is no longer reachable from HEAD.
                        self.restore_reachable[index] = self.restore_ordinals[index].is_some()
                            || run_bytes(
                                &self.repository.root,
                                &["merge-base", "--all", &target.to_hex(), "HEAD"],
                                &self.cancellation,
                                &[0, 1, 128],
                            )
                            .is_ok_and(|bytes| {
                                bytes
                                    .split(|b| *b == b'\n')
                                    .any(|id| id == target.to_hex().as_bytes())
                            });
                    }
                }
            }
        }
        let query = request.query.trim().to_owned();
        if self.query != query || self.regex != request.regex {
            self.prefix_search = None;
            self.query = query;
            self.regex = request.regex;
            self.matches.clear();
            self.scanned = 0;
            self.paths = None;
            self.path_pending.clear();
            self.candidates = HashMap::new();
            self.query_error = None;
            match Query::parse_with_mode(&self.query, request.regex)
                .and_then(|query| query.compile(request.regex))
            {
                Ok(parsed) => {
                    self.paths = DiffSearch::start(&self.repository, &parsed);
                    self.parsed = parsed;
                }
                Err(error) => self.query_error = Some(error.to_string()),
            }
        }
        if self.query_error.is_some() {
            return Ok(false);
        }
        if let Some(paths) = &mut self.paths {
            for _ in 0..256 {
                match paths.receiver.as_ref().unwrap().try_recv() {
                    Ok(Ok((id, matched))) => {
                        let ordinal = self
                            .candidates
                            .remove(&id)
                            .ok_or_else(|| anyhow::anyhow!("Unexpected diff-search commit"))?;
                        if matched {
                            // A reload can queue new prefix candidates before
                            // the remaining suffix has finished filtering.
                            let position = self.matches.partition_point(|i| *i < ordinal);
                            self.matches.insert(position, ordinal);
                        }
                    }
                    Ok(Err(error)) => {
                        self.query_error = Some(format!("Diff search failed: {error:#}"));
                        paths.done = true;
                        return Ok(false);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        paths.done = true;
                        ensure!(
                            self.candidates.is_empty(),
                            "Diff search ended before all commits were checked"
                        );
                        break;
                    }
                }
            }
        }
        if let Some(paths) = &mut self.paths {
            if !self.path_pending.is_empty() {
                let batch = std::mem::take(&mut self.path_pending);
                match paths.input.as_ref().unwrap().try_send(batch) {
                    Ok(()) => (),
                    Err(TrySendError::Full(batch)) => {
                        self.path_pending = batch;
                        thread::sleep(Duration::from_millis(5));
                        return Ok(true);
                    }
                    Err(TrySendError::Disconnected(batch)) => {
                        // Drain the worker's error before reporting a closed pipe.
                        self.path_pending = batch;
                        return Ok(true);
                    }
                }
            }
            if self.candidates.len() >= 4096 {
                thread::sleep(Duration::from_millis(5));
                return Ok(true);
            }
        }
        if self
            .prefix_search
            .as_ref()
            .is_some_and(|prefix| prefix.range.is_empty())
        {
            if self.path_pending.is_empty()
                && let Some(paths) = &mut self.paths
            {
                // diff-tree can buffer the last decision until stdin closes.
                paths.input.take();
            }
            if !self.candidates.is_empty() || !self.path_pending.is_empty() {
                thread::sleep(Duration::from_millis(5));
                return Ok(true);
            }
            let mut prefix = self.prefix_search.take().unwrap();
            self.paths = None;
            self.paths = DiffSearch::start(&self.repository, &self.parsed);
            self.matches.append(&mut prefix.suffix_matches);
            for (id, ordinal) in prefix.suffix_candidates {
                self.path_pending.push(id.clone());
                self.candidates.insert(id, ordinal);
            }
            if !self.path_pending.is_empty() {
                return Ok(true);
            }
        }
        let scan = self
            .prefix_search
            .as_ref()
            .map(|prefix| prefix.range.clone())
            .unwrap_or(self.scanned..self.entries.len());
        if !self.query.is_empty() && !scan.is_empty() {
            let end = (scan.start + 1000).min(scan.end);
            for index in scan.start..end {
                let commit = &self.entries[index].commit;
                if self.parsed.matches(commit) {
                    if self.paths.is_some() {
                        self.candidates.insert(commit.id.clone(), index);
                        self.path_pending.push(commit.id.clone());
                    } else {
                        self.matches.push(index);
                    }
                }
            }
            if let Some(prefix) = &mut self.prefix_search {
                prefix.range.start = end;
            } else {
                self.scanned = end;
            }
            return Ok(true);
        }
        // Publish a small first viewport before ref discovery or further batches.
        if self.entries.is_empty() && !self.eof {
            self.load(request.batch.unwrap_or(1000).clamp(1, 64), 64)?;
            return Ok(true);
        }
        if !self.refs_loaded || self.refs_revision != request.refs_revision {
            self.refs = Arc::new(load_refs(&self.repository, &self.cancellation)?);
            self.refs_loaded = true;
            self.refs_revision = request.refs_revision;
            return Ok(true);
        }
        // The requested batch controls prefetch demand, not the size of one
        // worker step. Publish bounded chunks even when --limit is very large.
        let batch = request.batch.unwrap_or(1000).max(1);
        let step = batch.min(if self.paths.is_some() { 1000 } else { 10000 });
        let needed = if request.batch.is_none() {
            usize::MAX
        } else {
            request
                .range
                .end
                .saturating_add(64)
                .div_ceil(batch)
                .saturating_mul(batch)
        };
        let visible = self.visible_count();
        if self.restore_pending() {
            self.load(step, usize::MAX)?;
            return Ok(true);
        }
        if !self.eof && visible < needed {
            if let Some(paths) = &self.paths {
                if paths.done {
                    self.paths = DiffSearch::start(&self.repository, &self.parsed);
                } else if paths.input.is_none() {
                    thread::sleep(Duration::from_millis(5));
                    return Ok(true);
                }
            }
            self.load(step, needed)?;
            return Ok(true);
        }
        if self.paths.as_ref().is_some_and(|paths| !paths.done) {
            // Close at EOF or an opt-in prefetch pause to flush the last diff.
            // Resuming demand opens a new diff stream for newly loaded candidates.
            if self.eof || visible >= needed {
                self.paths.as_mut().unwrap().input.take();
            }
            thread::sleep(Duration::from_millis(5));
            return Ok(true);
        }
        Ok(false)
    }

    fn visible_count(&self) -> usize {
        if self.query.is_empty() {
            self.entries.len()
        } else {
            self.matches.len()
        }
    }

    fn load(&mut self, count: usize, wanted: usize) -> Result<()> {
        let mut bytes = 0;
        for _ in 0..count {
            if self.cancellation.load(Ordering::Acquire) {
                break;
            }
            let Some(mut commit) = self.stream.as_mut().unwrap().next_commit()? else {
                self.eof = true;
                self.stream = None;
                break;
            };
            bytes += commit.memory_bytes();
            // Share repeated identities within this history generation. Full
            // messages and exact timestamps remain available for search/details.
            self.intern(&mut commit);
            let graph = self.graph.push(&commit);
            let ordinal = self.entries.len();
            if let Some(restore) = &self.restore {
                for (index, target) in [&restore.selected, &restore.top].into_iter().enumerate() {
                    if target.as_ref() == Some(&commit.id) {
                        self.restore_ordinals[index] = Some(ordinal);
                    }
                }
            }
            if !self.query.is_empty() && self.parsed.matches(&commit) {
                if self.paths.is_some() {
                    self.candidates.insert(commit.id.clone(), ordinal);
                    self.path_pending.push(commit.id.clone());
                } else {
                    self.matches.push(ordinal);
                }
            }
            self.entries.push(Arc::new(Entry {
                ordinal,
                commit,
                graph,
            }));
            self.scanned = self.entries.len();
            if self.visible_count() >= wanted || bytes >= BATCH_BYTES {
                break;
            }
        }
        Ok(())
    }

    fn restore_pending(&self) -> bool {
        !self.eof && (0..2).any(|i| self.restore_reachable[i] && self.restore_ordinals[i].is_none())
    }

    fn visible_position(&self, ordinal: usize) -> Option<usize> {
        if self.query.is_empty() {
            Some(ordinal)
        } else {
            self.matches.binary_search(&ordinal).ok()
        }
    }

    fn snapshot(&self, request: &Request) -> Snapshot {
        let visible = self.visible_count();
        let restored = self.restore.as_ref().map(|_| Restored {
            selected: self.restore_ordinals[0].and_then(|ordinal| {
                self.visible_position(ordinal)
                    .map(|position| (position, self.entries[ordinal].clone()))
            }),
            top: self.restore_ordinals[1].and_then(|ordinal| self.visible_position(ordinal)),
            pending: self.restore_pending()
                || !self.query.is_empty() && self.scanned < self.entries.len()
                || self.restore.as_ref().is_some_and(|restore| {
                    [&restore.selected, &restore.top]
                        .into_iter()
                        .flatten()
                        .any(|id| self.candidates.contains_key(id))
                }),
        });
        let range = if let Some(restored) = restored.as_ref().filter(|r| !r.pending) {
            let top = restored
                .top
                .or_else(|| restored.selected.as_ref().map(|s| s.0))
                .unwrap_or(0);
            let start = (top / 128).saturating_sub(1) * 128;
            start..start + PAGE_ROWS
        } else {
            request.range.clone()
        };
        let start = range.start.min(visible);
        let end = range.end.min(start + PAGE_ROWS).min(visible);
        let rows = (start..end)
            .map(|i| {
                let ordinal = if self.query.is_empty() {
                    i
                } else {
                    self.matches[i]
                };
                self.entries[ordinal].clone()
            })
            .collect();
        Snapshot {
            generation: request.generation,
            reload_generation: request.reload_generation,
            request_id: request.id,
            start,
            rows,
            visible,
            loaded: self.entries.len(),
            eof: self.eof && self.paths.as_ref().is_none_or(|paths| paths.done),
            load_duration: Duration::ZERO,
            filtering: self.query_error.is_none()
                && !self.query.is_empty()
                && (self.scanned < self.entries.len()
                    || !self.eof && request.batch.is_none()
                    || self.paths.as_ref().is_some_and(|paths| !paths.done)),
            branch: self.repository.branch.clone(),
            refs: self.refs.clone(),
            error: self.query_error.clone(),
            query_error: self.query_error.clone(),
            anchor_position: request.anchor.and_then(|ordinal| {
                if self.query.is_empty() {
                    (ordinal < self.entries.len()).then_some(ordinal)
                } else {
                    self.matches.binary_search(&ordinal).ok()
                }
            }),
            restored,
        }
    }
}

fn history_reusable(repository: &Repository, cancel: &Cancellation) -> Result<bool> {
    // These inputs can change ancestry while the HEAD object ID stays the same.
    // Disable reuse rather than retaining a second traversal-metadata cache.
    if ["GIT_REPLACE_REF_BASE", "GIT_GRAFT_FILE", "GIT_SHALLOW_FILE"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
        || repository.git_dir.join("info/grafts").try_exists()?
        || repository.common_dir.join("info/grafts").try_exists()?
    {
        return Ok(false);
    }
    let shallow = run_bytes(
        &repository.root,
        &["rev-parse", "--is-shallow-repository"],
        cancel,
        &[0],
    )?;
    let replacements = run_bytes(
        &repository.root,
        &[
            "for-each-ref",
            "--count=1",
            "--format=%(objectname)",
            "refs/replace/",
        ],
        cancel,
        &[0],
    )?;
    Ok(shallow == b"false\n" && replacements.is_empty())
}

fn load_refs(repository: &Repository, cancel: &Cancellation) -> Result<Refs> {
    let mut refs = Refs::new();
    let mut stream = repository.refs(cancel.clone())?;
    let mut bytes = 0;
    while let Some(record) = stream.record(b'\n')? {
        bytes += record.bytes.len();
        ensure!(
            bytes <= 16 * 1024 * 1024,
            "repository refs exceed the 16 MiB metadata budget"
        );
        let line = String::from_utf8_lossy(&record.bytes);
        let Some((hash, name)) = line.split_once(' ') else {
            continue;
        };
        let id = CommitId::parse(hash)?;
        let peeled = name.ends_with("^{}");
        let name = name.strip_suffix("^{}").unwrap_or(name);
        let (kind, short) = if let Some(name) = name.strip_prefix("refs/heads/") {
            (RefKind::Branch, name)
        } else if let Some(name) = name.strip_prefix("refs/remotes/") {
            (RefKind::Remote, name)
        } else if let Some(name) = name.strip_prefix("refs/tags/") {
            (RefKind::Tag, name)
        } else {
            continue;
        };
        if peeled {
            for labels in refs.values_mut() {
                labels.retain(|label| !(label.kind == kind && label.name == short));
            }
        }
        refs.entry(id).or_default().push(RefLabel {
            name: short.to_owned(),
            kind,
        });
    }
    if let Some(head) = &repository.head {
        refs.entry(head.clone()).or_default().insert(
            0,
            RefLabel {
                name: format!("HEAD -> {}", repository.branch),
                kind: RefKind::Head,
            },
        );
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;
    use std::time::{Duration, Instant};

    fn request(fixture: &Fixture) -> Request {
        Request {
            generation: Generation(1),
            reload_generation: ReloadGeneration::default(),
            id: 1,
            refs_revision: 0,
            repository: fixture.root().to_owned(),
            range: 0..10,
            query: String::new(),
            regex: false,
            batch: Some(1000),
            graph_columns: crate::graph::DEFAULT_GRAPH_LANES,
            history_readers: crate::git::DEFAULT_HISTORY_READERS,
            anchor: None,
            restore: None,
        }
    }

    fn refresh(session: &mut Session, request: &Request) -> bool {
        if !session.refresh(request).unwrap() {
            return false;
        }
        loop {
            match session.advance_reload().unwrap() {
                ReloadProgress::Pending => (),
                ReloadProgress::Ready => return true,
                ReloadProgress::Replace => return false,
            }
        }
    }

    #[test]
    fn unchanged_head_keeps_entries_search_and_partial_stream() {
        let fixture = Fixture::new();
        for i in 0..90 {
            fixture.git(&["commit", "--allow-empty", "-m", &format!("commit {i}")]);
        }
        let mut request = request(&fixture);
        request.batch = Some(1);
        request.range = 0..1;
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        assert!(!session.eof);
        let before = session.entries.clone();
        request.query = "message:commit".into();
        while session.step(&request).unwrap() {}
        let scanned = session.scanned;
        let matches = session.matches.clone();
        fixture.git(&["checkout", "-qb", "same-head"]);
        fixture.git(&["tag", "new-label"]);
        request.generation = Generation(2);
        assert!(refresh(&mut session, &request));
        assert_eq!(session.scanned, scanned);
        assert_eq!(session.matches, matches);
        for (old, retained) in before.iter().zip(&session.entries) {
            assert!(Arc::ptr_eq(old, retained));
        }
        assert_eq!(session.repository.branch, "same-head");
        while session.step(&request).unwrap() {}
        assert!(
            session
                .refs
                .values()
                .flatten()
                .any(|label| label.name == "new-label")
        );
        request.batch = None;
        while session.step(&request).unwrap() {}
        assert!(session.eof);
        assert_eq!(session.entries.len(), 90);
        assert_eq!(
            session
                .entries
                .iter()
                .map(|e| &e.commit.id)
                .collect::<HashSet<_>>()
                .len(),
            90
        );
    }

    #[test]
    fn worker_reload_reuses_completed_history_and_rejects_old_snapshots() {
        let fixture = Fixture::new();
        fixture.commit("retained");
        let mut request = request(&fixture);
        let engine = HistoryEngine::spawn(request.clone());
        let handle = engine.handle();
        let wait = |generation| {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(snapshot) = handle.take_snapshot() {
                    assert_eq!(snapshot.generation, generation);
                    assert!(snapshot.error.is_none());
                    if snapshot.eof {
                        break snapshot;
                    }
                }
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(5));
            }
        };
        let before = wait(request.generation);
        request.generation = Generation(2);
        handle.request(request.clone());
        let after = wait(request.generation);
        assert!(Arc::ptr_eq(&before.rows[0], &after.rows[0]));
        request.generation = Generation(3);
        request.graph_columns = 1;
        handle.request(request.clone());
        let rebuilt = wait(request.generation);
        assert!(!Arc::ptr_eq(&after.rows[0], &rebuilt.rows[0]));
    }

    #[test]
    fn traversal_overrides_disable_same_head_reuse() {
        let fixture = Fixture::new();
        let root = fixture.commit("root");
        let head = fixture.commit("tip");
        let request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        fixture.git(&["replace", &head, &root]);
        assert!(!refresh(&mut session, &request));
        let mut replaced = Session::open(&request, Cancellation::default()).unwrap();
        fixture.git(&["replace", "-d", &head]);
        assert!(!refresh(&mut replaced, &request));
        std::fs::write(fixture.root().join(".git/info/grafts"), "").unwrap();
        assert!(!refresh(&mut session, &request));
        std::fs::remove_file(fixture.root().join(".git/info/grafts")).unwrap();
        std::fs::write(fixture.root().join(".git/shallow"), format!("{head}\n")).unwrap();
        assert!(!refresh(&mut session, &request));
    }

    fn assert_same_history(actual: &Session, expected: &Session) {
        assert_eq!(actual.entries.len(), expected.entries.len());
        for (actual, expected) in actual.entries.iter().zip(&expected.entries) {
            assert_eq!(actual.ordinal, expected.ordinal);
            assert_eq!(actual.commit.id, expected.commit.id);
            assert_eq!(actual.commit.message, expected.commit.message);
            assert_eq!(actual.graph, expected.graph);
        }
        assert_eq!(actual.matches, expected.matches);
        assert_eq!(actual.eof, expected.eof);
    }

    #[test]
    fn linear_reload_retains_messages_and_continues_partial_history() {
        let fixture = Fixture::new();
        for i in 0..90 {
            fixture.git(&["commit", "--allow-empty", "-m", &format!("commit {i}")]);
        }
        let mut request = request(&fixture);
        request.batch = Some(1);
        request.range = 0..1;
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        assert!(!session.eof);
        let loaded = session.entries.len();
        let message = session.entries[1].commit.message.as_ptr();
        let old_tip = session.entries[0].clone();
        fixture.commit("new tip one");
        fixture.commit("new tip two");
        request.generation = Generation(2);
        request.restore = Some(Restore {
            selected: Some(old_tip.commit.id.clone()),
            top: Some(old_tip.commit.id.clone()),
        });
        assert!(refresh(&mut session, &request));
        assert_eq!(session.entries.len(), loaded + 2);
        assert_eq!(session.entries[3].commit.message.as_ptr(), message);
        assert!(!old_tip.graph.incoming);
        assert!(session.entries[2].graph.incoming);
        while session.step(&request).unwrap() {}
        let restored = session.snapshot(&request).restored.unwrap();
        assert_eq!(restored.selected.unwrap().0, 2);
        assert_eq!(restored.top, Some(2));
        request.batch = None;
        while session.step(&request).unwrap() {}
        // Repeated updates also reuse a completed stream.
        fixture.commit("third tip");
        assert!(refresh(&mut session, &request));
        while session.step(&request).unwrap() {}
        let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
        while fresh.step(&request).unwrap() {}
        assert_same_history(&session, &fresh);
    }

    #[test]
    fn linear_reload_reconciles_metadata_and_completed_or_pending_path_search() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.commit("matching change");
        fixture.git(&["commit", "--allow-empty", "-m", "unrelated"]);
        for query in [
            "message:matching",
            "path:file.txt",
            "path:file.txt message:matching",
            "contents:matching",
            "contents:matching path:file.txt",
        ] {
            for complete in [false, true] {
                let mut request = request(&fixture);
                request.query = query.into();
                let mut session = Session::open(&request, Cancellation::default()).unwrap();
                // Leave candidates pending in the path worker in one case.
                for _ in 0..3 {
                    session.step(&request).unwrap();
                }
                if complete {
                    while session.step(&request).unwrap() {}
                }
                fixture.commit(&format!("matching new change {query} {complete}"));
                request.generation = Generation(2);
                assert!(refresh(&mut session, &request));
                while session.step(&request).unwrap() {}
                assert!(session.candidates.is_empty());
                let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
                while fresh.step(&request).unwrap() {}
                assert_same_history(&session, &fresh);
                assert!(session.matches.windows(2).all(|pair| pair[0] < pair[1]));
            }
        }
    }

    #[test]
    fn linear_prefix_preserves_graph_geometry_above_an_existing_merge() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.git(&["checkout", "-qb", "side"]);
        fixture.git(&["commit", "--allow-empty", "-m", "side"]);
        fixture.git(&["checkout", "main"]);
        fixture.commit("main");
        fixture.git(&["merge", "--no-ff", "side", "-m", "merge"]);
        for cap in [1, 2, 256] {
            let mut request = request(&fixture);
            request.graph_columns = cap;
            let mut session = Session::open(&request, Cancellation::default()).unwrap();
            while session.step(&request).unwrap() {}
            fixture.commit(&format!("linear tip {cap}"));
            assert!(refresh(&mut session, &request));
            while session.step(&request).unwrap() {}
            let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
            while fresh.step(&request).unwrap() {}
            assert_same_history(&session, &fresh);
        }
    }

    #[test]
    fn nonlinear_updates_leave_retained_history_untouched() {
        let fixture = Fixture::new();
        let root = fixture.commit("root");
        fixture.git(&["checkout", "-qb", "side"]);
        fixture.git(&["commit", "--allow-empty", "-m", "side"]);
        fixture.git(&["checkout", "main"]);
        fixture.commit("tip");
        let request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let before = session.entries.clone();
        fixture.git(&["merge", "--no-ff", "side", "-m", "merge"]);
        assert!(!refresh(&mut session, &request));
        fixture.git(&["checkout", "side"]);
        assert!(!refresh(&mut session, &request));
        fixture.git(&["checkout", "--detach", &root]);
        assert!(!refresh(&mut session, &request));
        for (old, retained) in before.iter().zip(&session.entries) {
            assert!(Arc::ptr_eq(old, retained));
        }
        assert_eq!(before.len(), session.entries.len());
    }

    #[test]
    fn linear_reload_yields_across_commit_and_byte_budgets() {
        for (count, message) in [(513, "new".to_owned()), (4, "x".repeat(750 * 1024))] {
            let fixture = Fixture::new();
            fixture.commit("root");
            let mut request = request(&fixture);
            let mut session = Session::open(&request, Cancellation::default()).unwrap();
            while session.step(&request).unwrap() {}
            let before = session.entries[0].clone();
            fixture.append_history(count, false, &message);
            request.generation = Generation(2);
            assert!(session.refresh(&request).unwrap());
            assert!(matches!(
                session.advance_reload().unwrap(),
                ReloadProgress::Pending
            ));
            let provisional = session.reload.as_ref().unwrap().entries.len();
            assert!(provisional <= RELOAD_STEP_COMMITS && provisional < count);
            assert_eq!(session.entries.len(), 1);
            assert!(Arc::ptr_eq(&before, &session.entries[0]));
            loop {
                match session.advance_reload().unwrap() {
                    ReloadProgress::Pending => (),
                    ReloadProgress::Ready => break,
                    ReloadProgress::Replace => panic!("linear delta must be retained"),
                }
            }
            assert_eq!(session.entries.len(), count + 1);
            assert!(Arc::ptr_eq(
                &before.commit.author.name,
                &session.entries[count].commit.author.name
            ));
            request.batch = None;
            while session.step(&request).unwrap() {}
            let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
            while fresh.step(&request).unwrap() {}
            assert_same_history(&session, &fresh);
        }
    }

    #[test]
    fn large_prefix_search_keeps_queues_bounded_and_preserves_the_suffix() {
        for (count, complete) in [(9000, true), (513, false)] {
            let fixture = Fixture::new();
            fixture.commit("root");
            fixture.append_history(3, true, "old");
            let mut request = request(&fixture);
            request.query = "path:ci.yml".into();
            request.batch = None;
            let mut session = Session::open(&request, Cancellation::default()).unwrap();
            for _ in 0..3 {
                session.step(&request).unwrap();
            }
            if complete {
                while session.step(&request).unwrap() {}
            }
            let message = session.entries[0].commit.message.as_ptr();
            fixture.append_history(count, true, "new");
            request.generation = Generation(2);
            assert!(refresh(&mut session, &request));
            let deadline = Instant::now() + Duration::from_secs(30);
            while session.step(&request).unwrap() {
                assert!(session.candidates.len() <= 5096);
                assert!(session.path_pending.len() <= 5096);
                assert!(Instant::now() < deadline, "prefix search did not finish");
            }
            assert!(session.prefix_search.is_none());
            assert_eq!(session.entries[count].commit.message.as_ptr(), message);
            assert_eq!(session.matches.len(), count + 3);
            let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
            while fresh.step(&request).unwrap() {}
            assert_same_history(&session, &fresh);
        }
    }

    #[test]
    fn superseded_and_cancelled_probes_do_not_modify_the_retained_suffix() {
        let fixture = Fixture::new();
        let root = fixture.commit("root");
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let before = session.entries[0].clone();
        let tip = fixture.append_history(600, false, "new");
        assert!(session.refresh(&request).unwrap());
        assert!(matches!(
            session.advance_reload().unwrap(),
            ReloadProgress::Pending
        ));
        fixture.git(&["update-ref", "refs/heads/main", &root]);
        request.generation = Generation(2);
        assert!(refresh(&mut session, &request));
        assert!(session.reload.is_none());
        assert!(Arc::ptr_eq(&before, &session.entries[0]));
        assert_eq!(session.entries.len(), 1);
        fixture.git(&["update-ref", "refs/heads/main", &tip]);
        assert!(session.refresh(&request).unwrap());
        assert!(matches!(
            session.advance_reload().unwrap(),
            ReloadProgress::Pending
        ));
        session.cancellation.store(true, Ordering::Release);
        assert!(session.advance_reload().is_err());
        assert!(session.reload.is_none());
        assert!(Arc::ptr_eq(&before, &session.entries[0]));
        assert_eq!(session.entries.len(), 1);
    }

    #[test]
    fn nonlinear_history_found_after_multiple_steps_leaves_the_suffix_untouched() {
        let fixture = Fixture::new();
        let root = fixture.commit("root");
        fixture.commit("old tip");
        let request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let before = session.entries.clone();
        fixture.git(&["update-ref", "refs/heads/main", &root]);
        fixture.append_history(600, false, "divergent");
        assert!(session.refresh(&request).unwrap());
        for _ in 0..2 {
            assert!(matches!(
                session.advance_reload().unwrap(),
                ReloadProgress::Pending
            ));
        }
        assert!(matches!(
            session.advance_reload().unwrap(),
            ReloadProgress::Replace
        ));
        assert!(session.reload.is_none());
        assert_eq!(before.len(), session.entries.len());
        for (old, retained) in before.iter().zip(&session.entries) {
            assert!(Arc::ptr_eq(old, retained));
        }
    }

    #[test]
    fn superseded_prefix_search_adopts_the_latest_query() {
        let fixture = Fixture::new();
        fixture.commit("root");
        let mut request = request(&fixture);
        request.query = "path:ci.yml".into();
        request.batch = None;
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        fixture.append_history(600, true, "new");
        assert!(refresh(&mut session, &request));
        session.step(&request).unwrap();
        assert!(session.prefix_search.is_some());
        request.generation = Generation(2);
        assert!(refresh(&mut session, &request));
        request.query = "message:root".into();
        while session.step(&request).unwrap() {}
        assert_eq!(session.matches, vec![600]);
        let mut fresh = Session::open(&request, Cancellation::default()).unwrap();
        while fresh.step(&request).unwrap() {}
        assert_same_history(&session, &fresh);
    }

    /// Opt-in, history-only memory probe. Does not modify the repository.
    #[test]
    #[ignore = "requires GITK_PROFILE_REPOSITORY; reads real history"]
    fn profile_retained_history() {
        use std::mem::size_of;

        let repository =
            std::env::var_os("GITK_PROFILE_REPOSITORY").expect("set GITK_PROFILE_REPOSITORY");
        let count: usize = std::env::var("GITK_PROFILE_COMMITS")
            .unwrap_or_else(|_| "100000".into())
            .parse()
            .unwrap();
        assert!(count > 64);
        let readers: usize = std::env::var("GITK_PROFILE_READERS")
            .unwrap_or_else(|_| crate::git::DEFAULT_HISTORY_READERS.to_string())
            .parse()
            .unwrap();
        assert!((1..=crate::git::MAX_HISTORY_READERS).contains(&readers));
        let request = Request {
            generation: Generation(1),
            reload_generation: ReloadGeneration::default(),
            id: 1,
            refs_revision: 0,
            repository: repository.into(),
            range: 0..1,
            query: String::new(),
            regex: false,
            batch: Some(count),
            graph_columns: crate::graph::DEFAULT_GRAPH_LANES,
            history_readers: readers,
            anchor: None,
            restore: None,
        };
        let started = Instant::now();
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        session.step(&request).unwrap();
        let first_batch_ms = started.elapsed().as_millis();
        while session.step(&request).unwrap() {}
        let mut records = 0;
        let mut metadata = 0;
        let mut messages = 0;
        let mut graph = 0;
        for entry in &session.entries {
            records += size_of::<Entry>() + 2 * size_of::<usize>();
            let commit = &entry.commit;
            metadata += size_of_val(commit.parents.as_ref());
            for signature in [&commit.author, &commit.committer] {
                metadata += signature.time.len();
            }
            messages += commit.message.len();
            graph += entry.graph.edges.memory_bytes();
        }
        metadata += session
            .strings
            .iter()
            .map(|value| value.len() + 2 * size_of::<usize>())
            .sum::<usize>();
        let index = session.entries.capacity() * size_of::<Arc<Entry>>()
            + session.matches.capacity() * size_of::<usize>()
            + session.strings.capacity() * (size_of::<Arc<str>>() + 1);
        eprintln!(
            "history_probe readers={} graph_lanes={} loaded={} eof={} elapsed_ms={} records_bytes={records} metadata_bytes={metadata} messages_bytes={messages} graph_bytes={graph} index_bytes={index} accounted_bytes={}",
            request.history_readers,
            request.graph_columns,
            session.entries.len(),
            session.eof,
            started.elapsed().as_millis(),
            records + metadata + messages + graph + index,
        );
        eprintln!("history_probe first_batch_ms={first_batch_ms}");
        // RSS includes allocator overhead and the test harness. Keep the probe
        // separate from desktop measurements; never combine Git RSS with ours.
        #[cfg(target_os = "linux")]
        {
            fn rss(pid: &str) -> String {
                std::fs::read_to_string(format!("/proc/{pid}/status"))
                    .unwrap_or_default()
                    .lines()
                    .find(|line| line.starts_with("VmRSS:"))
                    .unwrap_or("VmRSS: unavailable (process exited)")
                    .to_owned()
            }
            let pid = std::process::id().to_string();
            eprintln!("history_probe process_pid={pid} {}", rss(&pid));
            // Git is spawned on this test thread, which need not be the main thread.
            for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
                let children = std::fs::read_to_string(task.unwrap().path().join("children"))
                    .unwrap_or_default();
                for child in children.split_whitespace() {
                    eprintln!("history_probe child_pid={child} {}", rss(child));
                }
            }
        }
    }

    #[test]
    fn retained_history_shares_exact_identities_and_preserves_distinct_case() {
        let fixture = Fixture::new();
        fixture.commit("oldest\n\nBody é");
        fixture.commit("middle");
        fixture.git(&[
            "-c",
            "user.name=fixture",
            "commit",
            "--allow-empty",
            "-m",
            "newest",
        ]);
        let request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let commits: Vec<_> = session.entries.iter().map(|entry| &entry.commit).collect();
        assert!(Arc::ptr_eq(
            &commits[1].author.name,
            &commits[2].committer.name
        ));
        assert!(!Arc::ptr_eq(
            &commits[0].author.name,
            &commits[1].author.name
        ));
        assert!(Arc::ptr_eq(
            &commits[0].author.email,
            &commits[2].committer.email
        ));
        assert_eq!(commits[0].author.name.as_ref(), "fixture");
        assert_eq!(commits[1].author.name.as_ref(), "Fixture");
        assert_eq!(commits[2].message.as_ref(), "oldest\n\nBody é\n");
    }

    #[test]
    fn filtering_searches_full_messages_without_copying_loaded_commits() {
        let fixture = Fixture::new();
        fixture.commit("first\n\nHidden body needle");
        fixture.commit("second");
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let before = session.entries[1].clone();
        request.anchor = Some(1);
        request.query = "  NEEDLE  ".into();
        while session.step(&request).unwrap() {}
        let snapshot = session.snapshot(&request);
        assert_eq!((snapshot.visible, snapshot.loaded), (1, 2));
        assert!(Arc::ptr_eq(&snapshot.rows[0], &before));
        assert_eq!(snapshot.anchor_position, Some(0));
        assert!(snapshot.eof);
    }

    #[test]
    fn metadata_filters_combine_and_invalid_queries_recover() {
        let fixture = Fixture::new();
        fixture.git(&[
            "commit",
            "--allow-empty",
            "--date=2026-01-01T12:00:00+05:00",
            "-m",
            "First\n\nHidden needle",
        ]);
        fixture.git(&[
            "commit",
            "--allow-empty",
            "--author=Alice Smith <alice@example.test>",
            "--date=2026-01-02T00:30:00+05:00",
            "-m",
            "Second",
        ]);
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        for (query, expected) in [
            (
                "author:\"ALICE Smith\" committer:fixture after:2026-01-01 before:2026-01-03 merge:false",
                1,
            ),
            ("message:needle before:2026-01-02", 1),
            ("after:2026-01-02", 0),
            ("before:2026-01-01", 0),
            ("author:fixture message:second", 0),
            ("merge:true", 0),
        ] {
            request.query = query.into();
            while session.step(&request).unwrap() {}
            assert_eq!(session.snapshot(&request).visible, expected, "{query}");
        }
        request.query = format!("hash:{}", &session.entries[0].commit.id.to_hex()[..12]);
        while session.step(&request).unwrap() {}
        assert_eq!(session.visible_count(), 1);
        request.query = "before:invalid".into();
        assert!(!session.step(&request).unwrap());
        assert!(session.snapshot(&request).error.is_some());
        request.query = String::new();
        while session.step(&request).unwrap() {}
        assert!(session.snapshot(&request).error.is_none());
        assert_eq!(session.visible_count(), 2);
    }

    #[test]
    fn paths_stream_literal_substrings_and_preserve_retained_history() {
        let fixture = Fixture::new();
        std::fs::create_dir(fixture.root().join("config")).unwrap();
        std::fs::write(fixture.root().join("config/CI.yml"), "first").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-m", "add workflow"]);
        fixture.commit("unrelated");
        fixture.git(&["mv", "config/CI.yml", "literal[1]*.yml"]);
        fixture.git(&["commit", "-m", "rename workflow"]);
        fixture.git(&["rm", "literal[1]*.yml"]);
        fixture.git(&["commit", "-m", "delete workflow"]);
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        let before = session.entries[0].clone();
        for (query, expected) in [
            ("path:ci.yml", 2),
            ("path:config/", 2),
            ("path:literal[1]*.yml", 2),
            ("path:ci.yml message:add", 1),
            ("path:absent", 0),
        ] {
            request.query = query.into();
            while session.step(&request).unwrap() {}
            let snapshot = session.snapshot(&request);
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            assert_eq!(snapshot.visible, expected, "{query}");
            assert!(snapshot.eof);
        }
        request.query.clear();
        while session.step(&request).unwrap() {}
        assert!(Arc::ptr_eq(&before, &session.snapshot(&request).rows[0]));
        // Replacing a running search releases its producer and cancellation token.
        request.query = "path:yml".into();
        session.step(&request).unwrap();
        let path_cancel = session.paths.as_ref().unwrap().cancel.clone();
        request.query = "author:fixture".into();
        while session.step(&request).unwrap() {}
        assert!(path_cancel.load(Ordering::Acquire));
        assert!(session.paths.is_none());
        assert_eq!(session.visible_count(), 4);
    }

    #[test]
    fn path_search_handles_empty_commits_merges_and_unusual_names() {
        let fixture = Fixture::new();
        fixture.commit("root");
        fixture.git(&["checkout", "-qb", "side"]);
        let name = "abc\n:1234567890123456789012345678901234567890";
        std::fs::write(fixture.root().join(name), "side").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-m", "side change"]);
        fixture.git(&["checkout", "-q", "main"]);
        fixture.git(&["commit", "--allow-empty", "-m", "empty"]);
        fixture.git(&["merge", "--no-ff", "side", "-m", "merge side"]);
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        for (query, expected) in [
            ("path:abc", 2),
            ("path:abc merge:true", 1),
            ("path:abc message:empty", 0),
            ("path:abc merge:false", 1),
            ("contents:side", 2),
            ("contents:side merge:true", 1),
            ("contents:side message:empty", 0),
        ] {
            request.query = query.into();
            while session.step(&request).unwrap() {}
            assert!(session.query_error.is_none(), "{:?}", session.query_error);
            assert_eq!(session.visible_count(), expected, "{query}");
            assert!(session.candidates.is_empty());
        }
    }

    #[test]
    fn contents_search_matches_changed_lines_and_combines_filters() {
        let fixture = Fixture::new();
        let name = "strange\nname.txt";
        let write = |text| {
            std::fs::write(fixture.root().join(name), text).unwrap();
            fixture.git(&["add", "."]);
        };
        write("Unchanged context\nNeedle[1] λ\n--- header-like text\n");
        fixture.git(&["commit", "-m", "introduce"]);
        write("Unchanged context\nReplacement\n+++ header-like text\n");
        fixture.git(&["commit", "-m", "replace"]);
        fixture.git(&["commit", "--allow-empty", "-m", "empty one"]);
        fixture.git(&["commit", "--allow-empty", "-m", "empty two"]);
        std::fs::write(fixture.root().join("binary.dat"), b"\0BinaryNeedle").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-m", "binary"]);
        fixture.git(&["rm", "--", name]);
        fixture.git(&["commit", "-m", "delete"]);
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        for (query, regex, expected) in [
            ("contents:\"needle[1] λ\"", false, 2),
            ("contents:^needle\\[1\\]", true, 2),
            ("contents:^Needle", false, 0),
            ("contents:(?-i)^NEEDLE", true, 0),
            ("contents:Replacement", false, 2),
            ("contents:context message:replace", false, 0),
            ("contents:Needle message:introduce author:fixture", false, 1),
            ("contents:Needle path:name.txt", false, 2),
            ("contents:Needle path:absent", false, 0),
            ("contents:header-like", false, 3),
            ("contents:BinaryNeedle", false, 0),
            ("contents:diff", false, 0),
            ("contents:Needle message:empty", false, 0),
        ] {
            request.query = query.into();
            request.regex = regex;
            while session.step(&request).unwrap() {}
            assert!(
                session.query_error.is_none(),
                "{query}: {:?}",
                session.query_error
            );
            assert_eq!(session.visible_count(), expected, "{query}");
            assert!(session.candidates.is_empty());
        }
        request.query = "contents:Needle".into();
        session.step(&request).unwrap();
        let cancel = session.paths.as_ref().unwrap().cancel.clone();
        request.query = "message:introduce".into();
        while session.step(&request).unwrap() {}
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(session.visible_count(), 1);
    }

    #[test]
    fn contents_prefilter_preserves_case_unicode_path_and_rename_matches() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.root().join("source.txt"),
            "COPY_CLOSURE\nMONKEY\nneedle[1]\n",
        )
        .unwrap();
        std::fs::write(fixture.root().join("other.txt"), "unrelated\n").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-m", "root"]);
        fixture.git(&["mv", "source.txt", "renamed.txt"]);
        fixture.git(&["commit", "-m", "rename"]);
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        for (query, expected) in [
            ("contents:copy_closure", 2),
            ("contents:copy_closure path:other.txt", 1),
            ("contents:monkey", 2),
            ("contents:needle[1]", 2),
            ("contents:needle[2]", 0),
            ("contents:absent", 0),
        ] {
            request.query = query.into();
            while session.step(&request).unwrap() {}
            assert!(
                session.query_error.is_none(),
                "{query}: {:?}",
                session.query_error
            );
            assert_eq!(session.visible_count(), expected, "{query}");
        }
    }

    #[test]
    fn path_git_failures_are_reported_and_metadata_search_can_recover() {
        let fixture = Fixture::new();
        fixture.commit("retained");
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        while session.step(&request).unwrap() {}
        // Damage only this disposable fixture after retaining its commit metadata.
        let tree = fixture.git(&["rev-parse", "HEAD^{tree}"]);
        std::fs::remove_file(
            fixture
                .root()
                .join(".git/objects")
                .join(&tree[..2])
                .join(&tree[2..]),
        )
        .unwrap();
        request.query = "contents:retained".into();
        while session.step(&request).unwrap() {}
        let snapshot = session.snapshot(&request);
        assert!(
            snapshot
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Diff search failed"))
        );
        request.query = "author:fixture".into();
        while session.step(&request).unwrap() {}
        assert!(session.snapshot(&request).error.is_none());
        assert_eq!(session.visible_count(), 1);
    }

    #[cfg(target_os = "linux")]
    fn report_search_rss(stage: &str, loaded: usize) {
        let pid = std::process::id();
        let rss = |pid: &str| {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .unwrap_or_default()
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .unwrap_or("VmRSS: unavailable (exited)")
                .to_owned()
        };
        eprintln!(
            "search_probe stage={stage} loaded={loaded} test_process_pid={pid} {}",
            rss(&pid.to_string())
        );
        for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
            let children =
                std::fs::read_to_string(task.unwrap().path().join("children")).unwrap_or_default();
            for child in children.split_whitespace() {
                let command =
                    std::fs::read_to_string(format!("/proc/{child}/comm")).unwrap_or_default();
                eprintln!(
                    "search_probe stage={stage} loaded={loaded} child_pid={child} command={} {}",
                    command.trim(),
                    rss(child)
                );
            }
        }
    }

    /// Explicit real-repository probe: reports first results and completion for the
    /// actual worker, rather than presenting Git command timings as application timings.
    #[test]
    #[ignore = "requires GITK_SEARCH_REPOSITORY; reads real history"]
    fn profile_metadata_search() {
        let repository =
            std::env::var_os("GITK_SEARCH_REPOSITORY").expect("set GITK_SEARCH_REPOSITORY");
        let query = std::env::var("GITK_SEARCH_QUERY")
            .unwrap_or_else(|_| "path:ci.yml after:2026-01-01".into());
        let mut request = Request {
            generation: Generation(1),
            reload_generation: ReloadGeneration::default(),
            id: 1,
            refs_revision: 0,
            repository: repository.into(),
            range: 0..128,
            query,
            regex: std::env::var("GITK_SEARCH_REGEX").is_ok_and(|value| value == "1"),
            batch: None,
            graph_columns: crate::graph::DEFAULT_GRAPH_LANES,
            history_readers: crate::git::DEFAULT_HISTORY_READERS,
            anchor: None,
            restore: None,
        };
        let start = Instant::now();
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
        eprintln!(
            "search_probe head={:?} query={:?} regex={} graph_lanes={}",
            session.repository.head.as_ref().map(CommitId::to_hex),
            request.query,
            request.regex,
            request.graph_columns
        );
        let mut first = false;
        while session.step(&request).unwrap() {
            assert!(session.query_error.is_none(), "{:?}", session.query_error);
            if !first && session.visible_count() > 0 {
                eprintln!(
                    "search_probe first_ms={} loaded={} matches={}",
                    start.elapsed().as_millis(),
                    session.entries.len(),
                    session.visible_count()
                );
                #[cfg(target_os = "linux")]
                report_search_rss("first", session.entries.len());
                first = true;
            }
        }
        assert!(session.query_error.is_none(), "{:?}", session.query_error);
        eprintln!(
            "search_probe complete_ms={} loaded={} matches={} eof={}",
            start.elapsed().as_millis(),
            session.entries.len(),
            session.visible_count(),
            session.eof
        );
        #[cfg(target_os = "linux")]
        report_search_rss("complete", session.entries.len());
        if let Ok(query) = std::env::var("GITK_SEARCH_REFINE") {
            request.query = query;
            let start = Instant::now();
            while session.step(&request).unwrap() {}
            assert!(session.query_error.is_none(), "{:?}", session.query_error);
            eprintln!(
                "search_probe refine_query={:?} refine_ms={} loaded={} matches={}",
                request.query,
                start.elapsed().as_millis(),
                session.entries.len(),
                session.visible_count()
            );
        }
    }

    #[test]
    fn worker_surfaces_open_errors_and_switches_repositories() {
        let fixture = Fixture::new();
        fixture.commit("working");
        let mut bad = request(&fixture);
        bad.repository = fixture.root().join("does-not-exist");
        let engine = HistoryEngine::spawn(bad);
        let handle = engine.handle();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(snapshot) = handle.take_snapshot() {
                assert!(snapshot.error.is_some());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        let mut good = request(&fixture);
        good.generation = Generation(2);
        handle.request(good);
        loop {
            if let Some(snapshot) = handle.take_snapshot() {
                assert_eq!(snapshot.generation, Generation(2));
                assert!(snapshot.error.is_none());
                if !snapshot.rows.is_empty() {
                    break;
                }
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        drop(engine);
    }

    #[test]
    fn annotated_tags_are_attached_to_the_peeled_commit() {
        let fixture = Fixture::new();
        let id = fixture.commit("tagged");
        fixture.git(&["tag", "-a", "v1", "-m", "version one"]);
        let cancel = Cancellation::default();
        let repository = Repository::open(fixture.root(), &cancel).unwrap();
        let refs = load_refs(&repository, &cancel).unwrap();
        assert!(
            refs[&CommitId::parse(&id).unwrap()]
                .iter()
                .any(|r| r.kind == RefKind::Tag && r.name == "v1")
        );
    }
}

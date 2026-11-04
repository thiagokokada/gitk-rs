//! Go-style streamed history. Loaded entries live once in memory. The GUI receives
//! only nearby Arc references; neither history nor filter results persist on disk.
use crate::{
    git::{Cancellation, Commit, CommitId, GitStream, Repository},
    graph::{GraphBuilder, GraphRow},
};
use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

pub const PAGE_ROWS: usize = 512;
const BATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Generation(pub u64);

#[derive(Clone, Debug)]
pub struct Request {
    pub generation: Generation,
    pub id: u64,
    pub refs_revision: u64,
    pub repository: PathBuf,
    pub range: Range<usize>,
    pub query: String,
    pub batch: usize,
    pub graph_columns: usize,
    pub anchor: Option<usize>,
}

#[derive(Debug)]
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
    pub request_id: u64,
    pub start: usize,
    pub rows: Vec<Arc<Entry>>,
    pub visible: usize,
    pub loaded: usize,
    pub eof: bool,
    pub filtering: bool,
    pub branch: String,
    pub refs: Arc<Refs>,
    pub error: Option<String>,
    pub anchor_position: Option<usize>,
}

struct SharedState {
    request: Option<Request>,
    snapshot: Option<Snapshot>,
    cancellation: Cancellation,
    generation: Generation,
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
                request: Some(initial),
                snapshot: None,
                cancellation: Arc::new(AtomicBool::new(false)),
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
            if request.generation != state.generation {
                state.cancellation.store(true, Ordering::Release);
                state.cancellation = Arc::new(AtomicBool::new(false));
                state.generation = request.generation;
            }
            state.request.replace(request)
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
        if snapshot.generation != state.generation || state.stopped {
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
        if let Some(request) = incoming {
            if current
                .as_ref()
                .is_none_or(|previous| previous.generation != request.generation)
            {
                // Git cleanup must happen outside the shared UI mailbox lock.
                session = None;
                failed = false;
            }
            current = Some(request);
        }
        let request = current.as_ref().unwrap();
        let result = (|| -> Result<(Snapshot, bool)> {
            ensure!(
                request.query.len() <= 8192,
                "filter is too long (maximum 8192 bytes)"
            );
            if failed {
                return Ok((
                    empty_snapshot(
                        request,
                        Some("Reload to retry the failed history operation.".into()),
                    ),
                    false,
                ));
            }
            if session.is_none() {
                session = Some(Session::open(request, cancel.clone())?);
            }
            let session = session.as_mut().unwrap();
            let work = session.step(request)?;
            Ok((session.snapshot(request), work))
        })();
        match result {
            Ok((snapshot, pending)) => {
                publish(&shared, snapshot);
                work = pending;
            }
            Err(error) => {
                if !cancel.load(Ordering::Acquire) {
                    let mut snapshot = session
                        .as_ref()
                        .map(|s| s.snapshot(request))
                        .unwrap_or_else(|| empty_snapshot(request, None));
                    snapshot.error = Some(format!("{error:#}"));
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
        request_id: request.id,
        start: 0,
        rows: Vec::new(),
        visible: 0,
        loaded: 0,
        eof: false,
        filtering: false,
        branch: String::new(),
        refs: Arc::new(Refs::new()),
        error,
        anchor_position: None,
    }
}

struct Session {
    repository: Repository,
    stream: Option<GitStream>,
    cancellation: Cancellation,
    entries: Vec<Arc<Entry>>,
    graph: GraphBuilder,
    refs: Arc<Refs>,
    refs_loaded: bool,
    refs_revision: u64,
    eof: bool,
    query: String,
    matches: Vec<usize>,
    scanned: usize,
}

impl Session {
    fn open(request: &Request, cancellation: Cancellation) -> Result<Self> {
        let repository = Repository::open(&request.repository, &cancellation)?;
        let stream = repository
            .head
            .as_ref()
            .map(|_| repository.log(cancellation.clone()))
            .transpose()?;
        let eof = stream.is_none();
        Ok(Self {
            repository,
            stream,
            cancellation,
            entries: Vec::new(),
            graph: GraphBuilder::new(request.graph_columns),
            refs: Arc::new(Refs::new()),
            refs_loaded: false,
            refs_revision: 0,
            eof,
            query: String::new(),
            matches: Vec::new(),
            scanned: 0,
        })
    }

    fn step(&mut self, request: &Request) -> Result<bool> {
        let query = request.query.trim().to_lowercase();
        if self.query != query {
            self.query = query;
            self.matches.clear();
            self.scanned = 0;
        }
        if !self.query.is_empty() && self.scanned < self.entries.len() {
            let end = (self.scanned + 1000).min(self.entries.len());
            for index in self.scanned..end {
                if self.entries[index]
                    .commit
                    .searchable_text()
                    .contains(&self.query)
                {
                    self.matches.push(index);
                }
            }
            self.scanned = end;
            return Ok(true);
        }
        // Publish a small first viewport before ref discovery or further batches.
        if self.entries.is_empty() && !self.eof {
            self.load(request.batch.clamp(1, 64), 64)?;
            return Ok(true);
        }
        if !self.refs_loaded || self.refs_revision != request.refs_revision {
            self.refs = Arc::new(load_refs(&self.repository, &self.cancellation)?);
            self.refs_loaded = true;
            self.refs_revision = request.refs_revision;
            return Ok(true);
        }
        let needed = request.range.end.saturating_add(64);
        let visible = self.visible_count();
        if !self.eof && visible < needed {
            self.load(request.batch.clamp(1, 10000), needed)?;
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
            let Some(commit) = self.stream.as_mut().unwrap().next_commit()? else {
                self.eof = true;
                self.stream = None;
                break;
            };
            bytes += commit.memory_bytes();
            let graph = self.graph.push(&commit);
            let ordinal = self.entries.len();
            if !self.query.is_empty() && commit.searchable_text().contains(&self.query) {
                self.matches.push(ordinal);
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

    fn snapshot(&self, request: &Request) -> Snapshot {
        let visible = self.visible_count();
        let start = request.range.start.min(visible);
        let end = request.range.end.min(start + PAGE_ROWS).min(visible);
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
            request_id: request.id,
            start,
            rows,
            visible,
            loaded: self.entries.len(),
            eof: self.eof,
            filtering: !self.query.is_empty() && self.scanned < self.entries.len(),
            branch: self.repository.branch.clone(),
            refs: self.refs.clone(),
            error: None,
            anchor_position: request.anchor.and_then(|ordinal| {
                if self.query.is_empty() {
                    (ordinal < self.entries.len()).then_some(ordinal)
                } else {
                    self.matches.binary_search(&ordinal).ok()
                }
            }),
        }
    }
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
            id: 1,
            refs_revision: 0,
            repository: fixture.root().to_owned(),
            range: 0..10,
            query: String::new(),
            batch: 1000,
            graph_columns: 200,
            anchor: None,
        }
    }

    #[test]
    fn filtering_searches_full_messages_without_copying_loaded_commits() {
        let fixture = Fixture::new();
        fixture.commit("first\n\nHidden body needle");
        fixture.commit("second");
        let mut request = request(&fixture);
        let mut session = Session::open(&request, Arc::new(AtomicBool::new(false))).unwrap();
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
        let cancel = Arc::new(AtomicBool::new(false));
        let repository = Repository::open(fixture.root(), &cancel).unwrap();
        let refs = load_refs(&repository, &cancel).unwrap();
        assert!(
            refs[&CommitId::parse(&id).unwrap()]
                .iter()
                .any(|r| r.kind == RefKind::Tag && r.name == "v1")
        );
    }
}

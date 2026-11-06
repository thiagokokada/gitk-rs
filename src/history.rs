//! Go-style streamed history. Loaded entries live once in memory. The GUI receives
//! only nearby Arc references; neither history nor filter results persist on disk.
use crate::{
    git::{Cancellation, Commit, CommitId, GitStream, Repository, run_bytes},
    graph::{GraphBuilder, GraphRow},
};
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
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
    /// None streams to EOF; Some(N) pauses after satisfying browsing demand in
    /// batches of N. Worker steps and the snapshot mailbox stay bounded either way.
    pub batch: Option<usize>,
    pub graph_columns: usize,
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
    pub restored: Option<Restored>,
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
            if request.generation != state.generation {
                state.cancellation.store(true, Ordering::Release);
                state.cancellation = Cancellation::default();
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
        restored: None,
    }
}

struct Session {
    repository: Repository,
    stream: Option<GitStream>,
    cancellation: Cancellation,
    entries: Vec<Arc<Entry>>,
    strings: HashSet<Arc<str>>,
    graph: GraphBuilder,
    refs: Arc<Refs>,
    refs_loaded: bool,
    refs_revision: u64,
    eof: bool,
    query: String,
    matches: Vec<usize>,
    scanned: usize,
    restore: Option<Restore>,
    restore_ordinals: [Option<usize>; 2],
    restore_reachable: [bool; 2],
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
            strings: HashSet::new(),
            graph: GraphBuilder::new(request.graph_columns),
            refs: Arc::new(Refs::new()),
            refs_loaded: false,
            refs_revision: 0,
            eof,
            query: String::new(),
            matches: Vec::new(),
            scanned: 0,
            restore: None,
            restore_ordinals: [None; 2],
            restore_reachable: [false; 2],
        })
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
        let step = batch.min(10000);
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
            self.load(step, needed)?;
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
            for signature in [&mut commit.author, &mut commit.committer] {
                for value in [&mut signature.name, &mut signature.email] {
                    if let Some(shared) = self.strings.get(value.as_ref()) {
                        *value = shared.clone();
                    } else {
                        self.strings.insert(value.clone());
                    }
                }
            }
            let graph = self.graph.push(&commit);
            let ordinal = self.entries.len();
            if let Some(restore) = &self.restore {
                for (index, target) in [&restore.selected, &restore.top].into_iter().enumerate() {
                    if target.as_ref() == Some(&commit.id) {
                        self.restore_ordinals[index] = Some(ordinal);
                    }
                }
            }
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
                || !self.query.is_empty() && self.scanned < self.entries.len(),
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
            restored,
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
            batch: Some(1000),
            graph_columns: crate::graph::DEFAULT_GRAPH_LANES,
            anchor: None,
            restore: None,
        }
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
        let request = Request {
            generation: Generation(1),
            id: 1,
            refs_revision: 0,
            repository: repository.into(),
            range: 0..1,
            query: String::new(),
            batch: Some(count),
            graph_columns: crate::graph::DEFAULT_GRAPH_LANES,
            anchor: None,
            restore: None,
        };
        let started = Instant::now();
        let mut session = Session::open(&request, Cancellation::default()).unwrap();
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
            "history_probe graph_lanes={} loaded={} eof={} elapsed_ms={} records_bytes={records} metadata_bytes={metadata} messages_bytes={messages} graph_bytes={graph} index_bytes={index} accounted_bytes={}",
            request.graph_columns,
            session.entries.len(),
            session.eof,
            started.elapsed().as_millis(),
            records + metadata + messages + graph + index,
        );
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

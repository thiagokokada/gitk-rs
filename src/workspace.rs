//! Repository state and ordinary protected branch operations, independent of GPUI.
use crate::git::{Cancellation, GitStream, Repository, run_bytes};
use anyhow::{Context, Result, ensure};
use notify::{RecursiveMode, Watcher};
use std::{
    hash::{Hash, Hasher},
    sync::{Condvar, Mutex},
    thread::{self, JoinHandle},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalStatus {
    pub staged: bool,
    pub unstaged: bool,
    pub unmerged: bool,
}

pub fn local_status(root: &Path, cancel: &Cancellation) -> Result<LocalStatus> {
    Ok(probe_status(root, cancel)?.0)
}

fn probe_status(root: &Path, cancel: &Cancellation) -> Result<(LocalStatus, u64, Vec<PathBuf>)> {
    let mut stream = GitStream::spawn(
        root,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        cancel.clone(),
        &[0],
    )?;
    let mut status = LocalStatus::default();
    let mut fingerprint = std::collections::hash_map::DefaultHasher::new();
    let mut parents = Vec::new();
    let mut bytes = 0;
    while let Some(record) = stream.record(0)? {
        bytes += record.bytes.len();
        ensure!(bytes <= 16 * 1024 * 1024, "status paths exceed 16 MiB");
        record.bytes.hash(&mut fingerprint);
        let bytes = &record.bytes;
        let fields = match bytes.first() {
            Some(b'1') => 8,
            Some(b'2') => 9,
            Some(b'u') => 10,
            Some(b'?') => 1,
            _ => 0,
        };
        if fields > 0
            && let Some(path) = bytes.splitn(fields + 1, |b| *b == b' ').nth(fields)
        {
            let path = root.join(crate::diff::path_from_bytes(path.to_vec())?);
            if let Ok(metadata) = std::fs::symlink_metadata(&path) {
                metadata.len().hash(&mut fingerprint);
                metadata.modified().ok().hash(&mut fingerprint);
            }
            if parents.len() < 128
                && let Some(parent) = path.parent()
                && !parents.iter().any(|p| p == parent)
            {
                parents.push(parent.to_owned());
            }
        }
        match bytes.first() {
            Some(b'1' | b'2' | b'u') => {
                ensure!(bytes.len() >= 5, "truncated Git status record");
                status.staged |= bytes[2] != b'.';
                status.unstaged |= bytes[3] != b'.';
                status.unmerged |= bytes[0] == b'u';
                if bytes[0] == b'2' {
                    stream.record(0)?.context("missing status rename source")?;
                }
            }
            Some(b'?') => status.unstaged = true,
            _ => {}
        }
    }
    Ok((status, fingerprint.finish(), parents))
}

// HEAD and refs are read by separate Git commands. Include both observations so
// a commit between those reads cannot suppress the next HEAD reconciliation.
fn refs_fingerprint(repository: &Repository, refs: &[u8]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    refs.hash(&mut hash);
    repository.branch.hash(&mut hash);
    repository.head.hash(&mut hash);
    hash.finish()
}

pub fn branches(root: &Path, cancel: &Cancellation) -> Result<Vec<String>> {
    let mut stream = GitStream::spawn(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        cancel.clone(),
        &[0],
    )?;
    let mut branches = Vec::new();
    let mut bytes = 0;
    while let Some(record) = stream.record(b'\n')? {
        bytes += record.bytes.len();
        ensure!(bytes <= 2 * 1024 * 1024, "branch names exceed 2 MiB");
        branches.push(String::from_utf8(record.bytes)?);
    }
    Ok(branches)
}

pub fn switch_branch(root: &Path, branch: &str, cancel: &Cancellation) -> Result<()> {
    // Validate against local names as well as Git's branch syntax. No shell, force,
    // stash or discard. --no-guess prevents accidentally creating tracking branches.
    ensure!(
        branches(root, cancel)?.iter().any(|name| name == branch),
        "local branch no longer exists: {branch}"
    );
    run_bytes(root, &["switch", "--no-guess", "--", branch], cancel, &[0])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Fixture;
    use std::fs;
    fn cancel() -> Cancellation {
        Cancellation::default()
    }

    #[test]
    fn staged_unstaged_untracked_and_rename_status() {
        let fixture = Fixture::new();
        fixture.commit("base");
        assert_eq!(
            local_status(fixture.root(), &cancel()).unwrap(),
            LocalStatus::default()
        );
        fs::write(fixture.root().join("untracked\nfile"), "hello").unwrap();
        assert!(local_status(fixture.root(), &cancel()).unwrap().unstaged);
        fixture.git(&["add", "."]);
        fs::write(fixture.root().join("file.txt"), "changed").unwrap();
        let status = local_status(fixture.root(), &cancel()).unwrap();
        assert!(status.staged && status.unstaged && !status.unmerged);
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "tracked"]);
        fixture.git(&["mv", "file.txt", "renamed\tλ"]);
        let status = local_status(fixture.root(), &cancel()).unwrap();
        assert!(status.staged && !status.unstaged);
    }

    #[test]
    fn refs_fingerprint_reconciles_head_when_a_commit_races_the_probe() {
        let fixture = Fixture::new();
        fixture.commit("base");
        let before = Repository::open(fixture.root(), &cancel()).unwrap();
        fixture.commit("new tip");
        let after = Repository::open(fixture.root(), &cancel()).unwrap();
        let refs = run_bytes(fixture.root(), &["show-ref", "--head"], &cancel(), &[0]).unwrap();
        // The first probe opened the old HEAD but read refs after the commit.
        assert_ne!(
            refs_fingerprint(&before, &refs),
            refs_fingerprint(&after, &refs)
        );
    }

    #[test]
    fn protected_switch_preserves_worktree_on_failure() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fixture.git(&["checkout", "-qb", "other"]);
        fixture.commit("other");
        fixture.git(&["checkout", "-q", "main"]);
        fs::write(fixture.root().join("file.txt"), "unsaved edits").unwrap();
        let error = switch_branch(fixture.root(), "other", &cancel()).unwrap_err();
        assert!(format!("{error:#}").contains("Git failed"));
        assert_eq!(
            fs::read_to_string(fixture.root().join("file.txt")).unwrap(),
            "unsaved edits"
        );
        assert_eq!(fixture.git(&["branch", "--show-current"]), "main");
        assert!(switch_branch(fixture.root(), "--detach", &cancel()).is_err());
        fs::write(fixture.root().join("file.txt"), "base").unwrap();
        switch_branch(fixture.root(), "other", &cancel()).unwrap();
        assert_eq!(fixture.git(&["branch", "--show-current"]), "other");
    }
    fn wait_snapshot(
        handle: &WorkspaceHandle,
        generation: WorkspaceGeneration,
        after: u64,
    ) -> WorkspaceSnapshot {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(snapshot) = handle.take_snapshot()
                && snapshot.generation == generation
                && snapshot.revision > after
            {
                return snapshot;
            }
            assert!(Instant::now() < until, "workspace worker timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn deleted_directories_watch_existing_ancestors_and_recover() {
        let fixture = Fixture::new();
        let directory = fixture.root().join("nested/deep");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("tracked.rs"), "base").unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", "nested file"]);
        fs::remove_dir_all(fixture.root().join("nested")).unwrap();
        let request = WorkspaceRequest {
            generation: WorkspaceGeneration(1),
            repository: fixture.root().into(),
            watch: true,
        };
        let engine = WorkspaceEngine::spawn(request.clone());
        let handle = engine.handle();
        let deleted = wait_snapshot(&handle, request.generation, 0);
        assert!(deleted.status.unstaged);
        assert!(deleted.error.is_none());
        assert!(
            deleted.watcher_error.is_none(),
            "{:?}",
            deleted.watcher_error
        );

        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("tracked.rs"), "edited").unwrap();
        handle.configure(request.clone());
        let recreated = wait_snapshot(&handle, request.generation, deleted.revision);
        assert!(recreated.status.unstaged);
        assert!(recreated.watcher_error.is_none());

        fs::remove_dir_all(fixture.root().join("nested")).unwrap();
        handle.configure(request.clone());
        let deleted_again = wait_snapshot(&handle, request.generation, recreated.revision);
        assert!(deleted_again.status.unstaged);
        assert!(deleted_again.watcher_error.is_none());
    }

    #[test]
    fn watcher_coalesces_edits_and_manual_refresh_works_when_disabled() {
        let fixture = Fixture::new();
        fixture.commit("base");
        let generation = WorkspaceGeneration(1);
        let engine = WorkspaceEngine::spawn(WorkspaceRequest {
            generation,
            repository: fixture.root().into(),
            watch: true,
        });
        let handle = engine.handle();
        let initial = wait_snapshot(&handle, generation, 0);
        assert!(!initial.status.unstaged);
        for i in 0..10 {
            fs::write(fixture.root().join("file.txt"), format!("edited {i}")).unwrap();
        }
        let changed = wait_snapshot(&handle, generation, initial.revision);
        assert!(changed.status.unstaged);
        handle.configure(WorkspaceRequest {
            generation,
            repository: fixture.root().into(),
            watch: false,
        });
        let disabled = wait_snapshot(&handle, generation, changed.revision);
        fs::write(fixture.root().join("file.txt"), "base").unwrap();
        handle.configure(WorkspaceRequest {
            generation,
            repository: fixture.root().into(),
            watch: false,
        });
        let refreshed = wait_snapshot(&handle, generation, disabled.revision);
        assert!(!refreshed.status.unstaged);
        let other = Fixture::new();
        other.commit("other");
        let generation = WorkspaceGeneration(2);
        handle.configure(WorkspaceRequest {
            generation,
            repository: other.root().into(),
            watch: false,
        });
        let switched = wait_snapshot(&handle, generation, refreshed.revision);
        // Git resolves symlinks, including macOS /tmp -> /private/tmp.
        assert_eq!(
            switched.repository.unwrap().root,
            other.root().canonicalize().unwrap()
        );
    }

    #[test]
    fn unmerged_status_is_reported_without_losing_local_edits() {
        let fixture = Fixture::new();
        fixture.commit("base");
        fixture.git(&["checkout", "-qb", "side"]);
        fixture.commit("side");
        fixture.git(&["checkout", "-q", "main"]);
        fixture.commit("main");
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.root())
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "merge",
                "--no-commit",
                "side",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let status = local_status(fixture.root(), &cancel()).unwrap();
        assert!(status.unmerged && status.staged && status.unstaged);
        assert!(
            fs::read_to_string(fixture.root().join("file.txt"))
                .unwrap()
                .contains("<<<<<<<")
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceGeneration(pub u64);
#[derive(Clone)]
pub struct WorkspaceRequest {
    pub generation: WorkspaceGeneration,
    pub repository: PathBuf,
    pub watch: bool,
}
#[derive(Clone, Debug)]
pub struct WorkspaceSnapshot {
    pub generation: WorkspaceGeneration,
    pub revision: u64,
    pub refs_fingerprint: u64,
    pub status_fingerprint: u64,
    pub repository: Option<Repository>,
    pub status: LocalStatus,
    pub error: Option<String>,
    pub watcher_error: Option<String>,
}
#[derive(Clone)]
pub enum Operation {
    Branches,
    Switch(String),
}
pub struct OperationResult {
    pub generation: WorkspaceGeneration,
    pub id: u64,
    pub result: Result<Vec<String>, String>,
}
struct WorkMailbox {
    generation: WorkspaceGeneration,
    request: Option<WorkspaceRequest>,
    operation: Option<(WorkspaceGeneration, u64, Operation)>,
    snapshot: Option<WorkspaceSnapshot>,
    result: Option<OperationResult>,
    cancel: Cancellation,
    stopped: bool,
}
struct WorkShared {
    mailbox: Mutex<WorkMailbox>,
    changed: Condvar,
}
#[derive(Clone)]
pub struct WorkspaceHandle {
    shared: Arc<WorkShared>,
}
pub struct WorkspaceEngine {
    handle: WorkspaceHandle,
    worker: Option<JoinHandle<()>>,
}
impl WorkspaceEngine {
    pub fn spawn(request: WorkspaceRequest) -> Self {
        let shared = Arc::new(WorkShared {
            mailbox: Mutex::new(WorkMailbox {
                generation: request.generation,
                request: Some(request),
                operation: None,
                snapshot: None,
                result: None,
                cancel: Cancellation::default(),
                stopped: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        Self {
            handle: WorkspaceHandle { shared },
            worker: Some(thread::spawn(move || workspace_worker(worker_shared))),
        }
    }
    pub fn handle(&self) -> WorkspaceHandle {
        self.handle.clone()
    }
}
impl Drop for WorkspaceEngine {
    fn drop(&mut self) {
        {
            let mut mailbox = self.handle.shared.mailbox.lock().unwrap();
            mailbox.stopped = true;
            mailbox.cancel.store(true, Ordering::Release);
        }
        self.handle.shared.changed.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl WorkspaceHandle {
    pub fn configure(&self, request: WorkspaceRequest) {
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        if mailbox.generation != request.generation {
            mailbox.cancel.store(true, Ordering::Release);
            mailbox.cancel = Cancellation::default();
            mailbox.generation = request.generation;
            mailbox.operation = None;
            mailbox.snapshot = None;
            mailbox.result = None;
        }
        mailbox.request = Some(request);
        self.shared.changed.notify_one();
    }
    pub fn operate(&self, generation: WorkspaceGeneration, id: u64, operation: Operation) -> bool {
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        if generation != mailbox.generation || mailbox.operation.is_some() {
            return false;
        }
        mailbox.operation = Some((generation, id, operation));
        self.shared.changed.notify_one();
        true
    }
    pub fn take_snapshot(&self) -> Option<WorkspaceSnapshot> {
        self.shared.mailbox.lock().unwrap().snapshot.take()
    }
    pub fn take_result(&self) -> Option<OperationResult> {
        self.shared.mailbox.lock().unwrap().result.take()
    }
}

fn workspace_worker(shared: Arc<WorkShared>) {
    let mut request: Option<WorkspaceRequest> = None;
    let dirty = Arc::new(AtomicBool::new(false));
    let watch_failed = Arc::new(AtomicBool::new(false));
    let mut watcher: Option<notify::RecommendedWatcher> = None;
    let mut watched = Vec::new();
    let mut watcher_error = None;
    let mut last_probe = Instant::now() - Duration::from_secs(10);
    let mut changed_at = None;
    let mut previous = None;
    let mut revision = 0;
    loop {
        let (configured, operation, cancel) = {
            let mut mailbox = shared.mailbox.lock().unwrap();
            if mailbox.request.is_none() && mailbox.operation.is_none() && !mailbox.stopped {
                mailbox = if request.as_ref().is_some_and(|request| request.watch) {
                    shared
                        .changed
                        .wait_timeout(mailbox, Duration::from_millis(100))
                        .unwrap()
                        .0
                } else {
                    // With watching disabled, configure/operations/shutdown are
                    // the only possible work. Each already notifies this condvar.
                    shared.changed.wait(mailbox).unwrap()
                };
            }
            if mailbox.stopped {
                break;
            }
            (
                mailbox.request.take(),
                mailbox.operation.take(),
                mailbox.cancel.clone(),
            )
        };
        let mut force = false;
        if let Some(configured) = configured {
            let reset = request.as_ref().is_none_or(|r| {
                r.generation != configured.generation || r.watch != configured.watch
            });
            request = Some(configured);
            force = true;
            if reset {
                watcher = None;
                watched.clear();
                previous = None;
                watcher_error = None;
                dirty.store(false, Ordering::Release);
                watch_failed.store(false, Ordering::Release);
                if request.as_ref().unwrap().watch {
                    let dirty = dirty.clone();
                    let failed = watch_failed.clone();
                    match notify::recommended_watcher(
                        move |event: notify::Result<notify::Event>| match event {
                            Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                                dirty.store(true, Ordering::Release);
                            }
                            Err(_) => {
                                failed.store(true, Ordering::Release);
                            }
                            _ => {}
                        },
                    ) {
                        Ok(created) => watcher = Some(created),
                        Err(error) => {
                            watcher_error = Some(format!(
                                "File watching unavailable: {error}; periodic refresh remains active."
                            ))
                        }
                    }
                }
            }
        }
        let Some(request) = &request else {
            continue;
        };
        if let Some((generation, id, operation)) = operation
            && generation == request.generation
        {
            let result = match operation {
                Operation::Branches => branches(&request.repository, &cancel),
                Operation::Switch(branch) => {
                    switch_branch(&request.repository, &branch, &cancel).map(|()| Vec::new())
                }
            }
            .map_err(|error| format!("{error:#}"));
            let mut mailbox = shared.mailbox.lock().unwrap();
            if mailbox.generation == generation {
                mailbox.result = Some(OperationResult {
                    generation,
                    id,
                    result,
                });
            }
            force = true;
        }
        if dirty.swap(false, Ordering::AcqRel) {
            changed_at = Some(Instant::now());
        }
        if watch_failed.swap(false, Ordering::AcqRel) {
            watcher_error = Some(
                "File watcher failed; periodic refresh remains active. F5 is available.".into(),
            );
            force = true;
        }
        let debounce = changed_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(350));
        if !force
            && !(request.watch && (debounce || last_probe.elapsed() >= Duration::from_secs(5)))
        {
            continue;
        }
        changed_at = None;
        last_probe = Instant::now();
        let result = (|| -> Result<_> {
            let repository = Repository::open(&request.repository, &cancel)?;
            let (status, fingerprint, parents) = probe_status(&repository.root, &cancel)?;
            let refs = run_bytes(
                &repository.root,
                &["show-ref", "--head", "--dereference"],
                &cancel,
                &[0, 1],
            )?;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            let refs_fingerprint = refs_fingerprint(&repository, &refs);
            refs_fingerprint.hash(&mut hash);
            fingerprint.hash(&mut hash);
            if let Some(watcher) = &mut watcher {
                let base = [
                    repository.root.clone(),
                    repository.git_dir.clone(),
                    repository.common_dir.clone(),
                    repository.common_dir.join("refs"),
                    repository.common_dir.join("refs/heads"),
                ];
                let mut desired = Vec::new();
                for path in base.into_iter().chain(parents) {
                    // Deleted tracked paths still appear in status. Watch their
                    // nearest surviving directory so recreation is observable.
                    if let Some(path) = path.ancestors().find(|path| path.is_dir())
                        && !desired.iter().any(|existing| existing == path)
                    {
                        desired.push(path.to_owned());
                    }
                }
                watched.retain(|path| {
                    if desired.contains(path) {
                        true
                    } else {
                        let _ = watcher.unwatch(path);
                        false
                    }
                });
                watcher_error = None;
                for path in desired {
                    if watched.len() >= 136 || watched.contains(&path) {
                        continue;
                    }
                    match watcher.watch(&path, RecursiveMode::NonRecursive) {
                        Ok(()) => watched.push(path),
                        Err(error) => {
                            watcher_error = Some(format!(
                                "Watch {}: {error}; periodic refresh remains active.",
                                path.display()
                            ))
                        }
                    }
                }
            }
            watcher_error.hash(&mut hash);
            Ok((
                repository,
                status,
                hash.finish(),
                refs_fingerprint,
                fingerprint,
            ))
        })();
        if cancel.load(Ordering::Acquire) {
            continue;
        }
        let snapshot = match result {
            Ok((repository, status, fingerprint, refs_fingerprint, status_fingerprint)) => {
                if previous == Some(fingerprint) && !force {
                    continue;
                }
                previous = Some(fingerprint);
                revision += 1;
                WorkspaceSnapshot {
                    generation: request.generation,
                    revision,
                    refs_fingerprint,
                    status_fingerprint,
                    repository: Some(repository),
                    status,
                    error: None,
                    watcher_error: watcher_error.clone(),
                }
            }
            Err(error) => WorkspaceSnapshot {
                generation: request.generation,
                revision,
                refs_fingerprint: 0,
                status_fingerprint: 0,
                repository: None,
                status: LocalStatus::default(),
                error: Some(format!("{error:#}")),
                watcher_error: watcher_error.clone(),
            },
        };
        let mut mailbox = shared.mailbox.lock().unwrap();
        if mailbox.generation == request.generation {
            mailbox.snapshot = Some(snapshot);
        }
    }
}

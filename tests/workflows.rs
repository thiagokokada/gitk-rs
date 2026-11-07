//! Public-service integration tests: real Git, worker threads, pipes, temporary
//! documents and generation changes. No display server or user repository needed.
#[path = "../src/test_support.rs"]
mod test_support;

use gitk_rs::{
    diff::{
        CopyRequest, DiffEngine, DiffPage, DiffSelection, DiffSettings, DiffSource, LineKind,
        SelectionId, TextPoint, TextSelection,
    },
    history::{Generation, HistoryEngine, Request, Restore},
    workspace::{Operation, WorkspaceEngine, WorkspaceGeneration, WorkspaceRequest},
};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use test_support::Fixture;

#[test]
fn restoration_uses_commit_ids_and_handles_unreachable_branch_commits() {
    let fixture = Fixture::new();
    let root = fixture.commit("root");
    let tip = fixture.commit("main tip");
    fixture.git(&["checkout", "-qb", "side", &root]);
    let side = fixture.commit("side only");
    fixture.git(&["checkout", "-q", "main"]);
    let mut request = history_request(&fixture);
    request.restore = Some(Restore {
        selected: Some(gitk_rs::git::CommitId::parse(&root).unwrap()),
        top: Some(gitk_rs::git::CommitId::parse(&tip).unwrap()),
    });
    let engine = HistoryEngine::spawn(request.clone());
    let handle = engine.handle();
    let restored = wait(|| {
        handle
            .take_snapshot()
            .and_then(|s| s.restored)
            .filter(|r| !r.pending)
    });
    let (position, entry) = restored.selected.unwrap();
    assert_eq!(position, 1);
    assert_eq!(entry.commit.id.to_hex(), root);
    assert_eq!(restored.top, Some(0));
    request.id += 1;
    request.restore.as_mut().unwrap().selected =
        Some(gitk_rs::git::CommitId::parse(&side).unwrap());
    handle.request(request.clone());
    let restored = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.request_id == request.id)
            .and_then(|s| s.restored)
            .filter(|r| !r.pending)
    });
    assert!(restored.selected.is_none());
    assert_eq!(restored.top, Some(0));
}

fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = poll() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not complete its integration workflow"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn history_request(fixture: &Fixture) -> Request {
    Request {
        generation: Generation(1),
        id: 1,
        refs_revision: 0,
        repository: fixture.root().into(),
        range: 0..8,
        query: String::new(),
        batch: Some(7),
        graph_columns: 200,
        anchor: None,
        restore: None,
    }
}
fn diff_page(id: u64) -> DiffPage {
    DiffPage {
        selection: SelectionId(id),
        id: 1,
        range: 0..32,
        settings: DiffSettings {
            syntax: true,
            dark: true,
        },
    }
}

#[test]
fn history_selection_highlighting_and_copy_across_unloaded_lines() {
    let fixture = Fixture::new();
    let source = (0..400)
        .map(|i| format!("\tlet value_{i} = {i};\n"))
        .collect::<String>();
    fs::write(fixture.root().join("main.rs"), &source).unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "root source"]);
    let history = HistoryEngine::spawn(history_request(&fixture));
    let handle = history.handle();
    let snapshot = wait(|| handle.take_snapshot().filter(|s| s.eof));
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.rows.len(), 1);
    let commit = Arc::new(snapshot.rows[0].commit.clone());
    let diff = DiffEngine::spawn();
    let handle = diff.handle();
    handle.select(
        DiffSelection {
            options: Default::default(),
            id: SelectionId(1),
            repository: fixture.root().into(),
            source: DiffSource::Commit(commit),
        },
        diff_page(1),
    );
    let snapshot = wait(|| handle.take_snapshot().filter(|s| !s.highlighting));
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.files[0].path, std::path::Path::new("main.rs"));
    assert_eq!(snapshot.files[0].added, Some(400));
    assert_eq!((snapshot.added, snapshot.removed), (400, 0));
    assert!(snapshot.count > 400);
    assert!(snapshot.lines.len() < snapshot.count);
    let first = snapshot
        .lines
        .iter()
        .position(|line| line.kind == LineKind::Added)
        .unwrap()
        + snapshot.start;
    assert!(
        snapshot
            .lines
            .iter()
            .any(|line| line.kind == LineKind::Added && line.spans.len() > 1)
    );
    handle.copy(CopyRequest {
        selection: SelectionId(1),
        strip_markers: true,
        range: TextSelection {
            anchor: TextPoint {
                line: first,
                byte: 0,
            },
            cursor: TextPoint {
                line: snapshot.count - 1,
                byte: snapshot.last_line_len,
            },
        },
    });
    let copied = wait(|| handle.take_copy()).result.unwrap();
    assert_eq!(copied, source.trim_end_matches('\n'));
}

fn history_fixture(count: usize) -> Fixture {
    let fixture = Fixture::new();
    // One deterministic fast-import avoids spawning Git once per commit.
    let mut input = String::new();
    for i in 1..=count {
        let message = if i == 1 {
            "root-only needle".into()
        } else {
            format!("commit {i}")
        };
        input.push_str(&format!("commit refs/heads/main\nmark :{i}\ncommitter Fixture <fixture@example.test> {} +0000\ndata {}\n{message}\n", 1700000000 + i, message.len()));
        if i > 1 {
            input.push_str(&format!("from :{}\n", i - 1));
        }
        input.push('\n');
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(fixture.root())
        .args(["fast-import", "--quiet"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let imported = child.wait_with_output().unwrap();
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    fixture
}

#[test]
fn default_history_streams_to_eof_without_more_viewport_requests() {
    let fixture = history_fixture(2500);
    let config = gitk_rs::cli::parse([]).unwrap();
    assert_eq!(config.batch, None);
    let mut request = history_request(&fixture);
    request.batch = config.batch;
    let engine = HistoryEngine::spawn(request);
    let handle = engine.handle();
    let snapshot = wait(|| handle.take_snapshot().filter(|s| s.eof));
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.loaded, 2500);
    assert_eq!(snapshot.visible, 2500);
    assert_eq!(snapshot.rows.len(), 8);
    assert_eq!(snapshot.rows[0].commit.summary(), "commit 2500");
}

#[test]
fn large_cli_limit_prefetches_beyond_worker_chunk_size_and_pauses() {
    let fixture = history_fixture(12_050);
    let config = gitk_rs::cli::parse(["--limit".into(), "10001".into()]).unwrap();
    let mut request = history_request(&fixture);
    request.batch = config.batch;
    let engine = HistoryEngine::spawn(request.clone());
    let handle = engine.handle();
    let snapshot = wait(|| handle.take_snapshot().filter(|s| s.loaded >= 10_001));
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.loaded, 10_001);
    assert!(!snapshot.eof);
    thread::sleep(Duration::from_millis(100));
    if let Some(paused) = handle.take_snapshot() {
        assert_eq!(paused.loaded, 10_001);
    }

    // A million-commit request on a smaller repository must reach real Git EOF.
    let config = gitk_rs::cli::parse(["--limit".into(), "1000000".into()]).unwrap();
    request.id += 1;
    request.batch = config.batch;
    handle.request(request.clone());
    let snapshot = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.request_id == request.id && s.eof)
    });
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.loaded, 12_050);
    assert_eq!(snapshot.visible, 12_050);
    request.id += 1;
    request.range = 12_049..12_050;
    handle.request(request.clone());
    let snapshot = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.request_id == request.id)
    });
    assert_eq!(snapshot.rows[0].commit.summary(), "root-only needle");
}

#[test]
fn demand_pauses_history_then_filter_streams_further_and_generation_switches() {
    let fixture = history_fixture(96);
    let mut request = history_request(&fixture);
    let engine = HistoryEngine::spawn(request.clone());
    let handle = engine.handle();
    let snapshot = wait(|| handle.take_snapshot().filter(|s| s.loaded >= 77));
    assert_eq!(snapshot.loaded, 77);
    assert!(!snapshot.eof);
    assert_eq!(snapshot.rows.len(), 8);
    thread::sleep(Duration::from_millis(100));
    if let Some(paused) = handle.take_snapshot() {
        assert_eq!(
            paused.loaded, 77,
            "history kept prefetching after demand was satisfied"
        );
    }
    request.id += 1;
    request.query = " ROOT-ONLY NEEDLE ".into();
    handle.request(request.clone());
    let found = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.request_id == request.id && s.eof)
    });
    assert_eq!(found.loaded, 96);
    assert_eq!(found.visible, 1);
    assert_eq!(found.rows[0].commit.summary(), "root-only needle");
    let other = Fixture::new();
    other.commit("another repository");
    request.generation = Generation(2);
    request.id += 1;
    request.query.clear();
    request.repository = other.root().into();
    handle.request(request.clone());
    let switched = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.generation == Generation(2) && s.eof)
    });
    assert_eq!(switched.loaded, 1);
    assert_eq!(switched.rows[0].commit.summary(), "another repository");
}

#[test]
fn protected_branch_operations_reconcile_status_through_the_worker() {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.git(&["checkout", "-qb", "side"]);
    fixture.commit("side");
    fixture.git(&["checkout", "-q", "main"]);
    fs::write(fixture.root().join("file.txt"), "unsaved edits").unwrap();
    let generation = WorkspaceGeneration(1);
    let engine = WorkspaceEngine::spawn(WorkspaceRequest {
        generation,
        repository: fixture.root().into(),
        watch: false,
    });
    let handle = engine.handle();
    let initial = wait(|| handle.take_snapshot());
    assert!(initial.error.is_none());
    assert!(initial.status.unstaged);
    assert!(handle.operate(generation, 1, Operation::Branches));
    let listed = wait(|| handle.take_result().filter(|r| r.id == 1))
        .result
        .unwrap();
    assert_eq!(listed, ["main", "side"]);
    assert!(handle.operate(generation, 2, Operation::Switch("side".into())));
    let failed = wait(|| handle.take_result().filter(|r| r.id == 2))
        .result
        .unwrap_err();
    assert!(failed.contains("Git failed"));
    assert_eq!(fixture.git(&["branch", "--show-current"]), "main");
    assert_eq!(
        fs::read_to_string(fixture.root().join("file.txt")).unwrap(),
        "unsaved edits"
    );
    fs::write(fixture.root().join("file.txt"), "base").unwrap();
    assert!(handle.operate(generation, 3, Operation::Switch("side".into())));
    assert!(
        wait(|| handle.take_result().filter(|r| r.id == 3))
            .result
            .is_ok()
    );
    let changed = wait(|| {
        handle
            .take_snapshot()
            .filter(|s| s.repository.as_ref().is_some_and(|r| r.branch == "side"))
    });
    assert!(!changed.status.unstaged);
    assert_eq!(
        fs::read_to_string(fixture.root().join("file.txt")).unwrap(),
        "side"
    );
}

#[test]
fn opening_a_subdirectory_still_includes_untracked_files_at_the_repository_root() {
    let fixture = Fixture::new();
    fixture.commit("base");
    let nested = fixture.root().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("inside.rs"), "fn main() {}\n").unwrap();
    fs::write(fixture.root().join("outside.nix"), "{ answer = 42; }\n").unwrap();
    let engine = DiffEngine::spawn();
    let handle = engine.handle();
    handle.select(
        DiffSelection {
            options: Default::default(),
            id: SelectionId(1),
            repository: nested,
            source: DiffSource::Local { staged: false },
        },
        diff_page(1),
    );
    let snapshot = wait(|| handle.take_snapshot().filter(|s| !s.highlighting));
    assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
    let paths = snapshot
        .files
        .iter()
        .map(|file| file.path.as_path())
        .collect::<Vec<_>>();
    assert!(
        paths.contains(&std::path::Path::new("outside.nix")),
        "root-level untracked file is missing"
    );
    assert!(
        paths.contains(&std::path::Path::new("nested/inside.rs")),
        "file paths are not repository-relative"
    );
}

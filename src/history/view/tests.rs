use super::*;
use crate::{
    git::{Commit, Signature},
    graph::GraphRow,
    history::{Generation, HistorySource, ReloadGeneration, Restored},
};
use std::{collections::HashMap, time::Duration};

fn state() -> HistoryViewState {
    HistoryViewState::new(Request {
        generation: Generation(1),
        source: HistorySource {
            repository: PathBuf::from("repository"),
            graph_columns: 256,
            git_workers: 4,
            reload_generation: ReloadGeneration::default(),
        },
        id: 1,
        refs_revision: 0,
        range: 0..128,
        query: String::new(),
        regex: false,
        batch: None,
        anchor: None,
        restore: None,
    })
}

fn page(state: &HistoryViewState, start: usize, count: usize, visible: usize) -> Snapshot {
    let rows = (start..start + count)
        .map(|ordinal| {
            let signature = Signature {
                name: "Author".into(),
                email: "author@example.test".into(),
                time: "0 +0000".into(),
            };
            Arc::new(Entry {
                ordinal,
                commit: Commit {
                    id: CommitId::parse(&format!("{:040x}", ordinal + 1)).unwrap(),
                    parents: Box::default(),
                    author: signature.clone(),
                    committer: signature,
                    message: format!("commit {ordinal}").into(),
                },
                graph: GraphRow::default(),
            })
        })
        .collect();
    Snapshot {
        generation: state.request.generation,
        source: state.request.source.clone(),
        request_id: state.request.id,
        start,
        rows,
        visible,
        loaded: visible,
        eof: true,
        load_duration: Duration::ZERO,
        filtering: false,
        branch: "main".into(),
        refs: Arc::new(HashMap::new()),
        error: None,
        query_error: None,
        anchor_position: None,
        restored: None,
    }
}

fn loaded() -> HistoryViewState {
    let mut state = state();
    let snapshot = page(&state, 0, 10, 10);
    state.consume_snapshot(snapshot).unwrap();
    state
}

#[test]
fn obsolete_snapshots_leave_selection_rows_and_viewport_untouched() {
    let mut state = loaded();
    let original = state.cached(0).unwrap();
    state.viewport = 3..8;
    for obsolete in 0..4 {
        let mut snapshot = page(&state, 0, 2, 2);
        match obsolete {
            0 => snapshot.request_id += 1,
            1 => snapshot.generation.0 += 1,
            2 => snapshot.source.repository = PathBuf::from("another-repository"),
            3 => snapshot.source.reload_generation.0 += 1,
            _ => unreachable!(),
        }
        assert!(state.consume_snapshot(snapshot).is_none());
        assert!(Arc::ptr_eq(&original, &state.cached(0).unwrap()));
        assert!(Arc::ptr_eq(&original, &state.selected.as_ref().unwrap().1));
        assert_eq!(state.viewport, 3..8);
    }
}

#[test]
fn pending_restoration_keeps_the_old_page_until_bookmarks_are_ready() {
    let mut state = loaded();
    state.viewport = 5..10;
    let original = state.cached(5).unwrap();
    let reload = state.reload(PathBuf::from("repository"), false);
    assert!(!reload.clear_diff);
    let mut snapshot = page(&state, 0, 1, 1);
    snapshot.eof = false;
    snapshot.restored = Some(Restored {
        selected: None,
        top: None,
        pending: true,
    });
    assert!(state.consume_snapshot(snapshot).is_none());
    assert!(state.request.restore.is_some());
    assert!(Arc::ptr_eq(&original, &state.cached(5).unwrap()));
    assert_eq!(state.viewport, 5..10);
}

#[test]
fn restored_viewport_is_bookmarked_before_the_ui_applies_its_scroll() {
    let mut state = loaded();
    state.update_local_rows(&LocalStatus {
        unstaged: true,
        ..Default::default()
    });
    state.viewport = 6..11;
    state.reload(PathBuf::from("repository"), false);
    let mut snapshot = page(&state, 0, 20, 20);
    let selected = snapshot.rows[7].clone();
    let top = snapshot.rows[9].clone();
    snapshot.restored = Some(Restored {
        selected: Some((7, selected.clone())),
        top: Some(9),
        pending: false,
    });
    let update = state.consume_snapshot(snapshot).unwrap();
    assert_eq!(update.scroll, Some(ScrollTarget::Top(10)));
    assert_eq!(state.viewport, 10..15);
    assert_eq!(state.selected.as_ref().unwrap().0, 8);
    assert!(Arc::ptr_eq(&selected, &state.selected.as_ref().unwrap().1));
    assert_eq!(update.request.unwrap().anchor, Some(selected.ordinal));
    // No UI scroll/layout has happened between these reloads.
    let second = state.reload(PathBuf::from("repository"), false);
    assert_eq!(
        second.request.restore.unwrap(),
        Restore {
            selected: Some(selected.commit.id.clone()),
            top: Some(top.commit.id.clone())
        }
    );
}

#[test]
fn local_row_changes_preserve_commit_identity_and_shift_pending_navigation() {
    let mut state = loaded();
    state.select_row(4).unwrap();
    let selected = state.selected.as_ref().unwrap().1.clone();
    let top = state.cached(6).unwrap();
    state.viewport = 6..10;
    state.pending_selection = Some(8);
    let update = state.update_local_rows(&LocalStatus {
        unstaged: true,
        staged: true,
        ..Default::default()
    });
    assert_eq!(update.top, Some(8));
    assert_eq!(state.viewport, 8..12);
    assert_eq!(state.selected.as_ref().unwrap().0, 6);
    assert_eq!(state.pending_selection, Some(10));
    assert!(Arc::ptr_eq(&top, &state.cached(8).unwrap()));
    let reload = state.reload(PathBuf::from("repository"), false);
    assert_eq!(
        reload.request.restore.unwrap(),
        Restore {
            selected: Some(selected.commit.id.clone()),
            top: Some(top.commit.id.clone())
        }
    );
    state.update_local_rows(&LocalStatus::default());
    assert_eq!(state.viewport, 6..10);
    assert_eq!(state.selected.as_ref().unwrap().0, 4);
}

#[test]
fn disappearing_local_selection_waits_for_an_unloaded_first_commit() {
    let mut state = state();
    let snapshot = page(&state, 5, 5, 10);
    state.consume_snapshot(snapshot).unwrap();
    state.update_local_rows(&LocalStatus {
        staged: true,
        ..Default::default()
    });
    state.select_row(0).unwrap();
    let update = state.update_local_rows(&LocalStatus::default());
    assert!(update.selection.is_none());
    assert!(state.local_selection.is_none());
    assert_eq!(state.pending_selection, Some(0));
    let snapshot = page(&state, 0, 5, 10);
    let update = state.consume_snapshot(snapshot).unwrap();
    assert!(matches!(update.selection, Some(Selection::Commit(_))));
    assert_eq!(state.selected.as_ref().unwrap().0, 0);
    assert!(state.pending_selection.is_none());
}

#[test]
fn filtering_retains_the_selected_commit_until_its_position_is_known() {
    let mut state = loaded();
    state.select_row(4).unwrap();
    let selected = state.selected.as_ref().unwrap().1.clone();
    state.begin_filter("needle".into());
    state.next_request();
    let mut snapshot = page(&state, 0, 0, 0);
    snapshot.filtering = true;
    snapshot.eof = false;
    state.consume_snapshot(snapshot).unwrap();
    assert!(state.preserve_filter_selection);
    assert!(Arc::ptr_eq(&selected, &state.selected.as_ref().unwrap().1));
    state.update_local_rows(&LocalStatus {
        unstaged: true,
        ..Default::default()
    });
    let mut snapshot = page(&state, 0, 2, 2);
    snapshot.anchor_position = Some(1);
    let update = state.consume_snapshot(snapshot).unwrap();
    assert_eq!(update.scroll, Some(ScrollTarget::Center(2)));
    assert_eq!(state.selected.as_ref().unwrap().0, 2);
    assert!(Arc::ptr_eq(&selected, &state.selected.as_ref().unwrap().1));
    assert!(!state.preserve_filter_selection);
}

#[test]
fn a_filter_that_excludes_the_selection_chooses_its_first_loaded_match() {
    let mut state = loaded();
    state.select_row(4).unwrap();
    state.begin_filter("needle".into());
    state.next_request();
    let snapshot = page(&state, 0, 2, 2);
    let first = snapshot.rows[0].clone();
    let update = state.consume_snapshot(snapshot).unwrap();
    assert!(matches!(update.selection, Some(Selection::Commit(_))));
    assert!(Arc::ptr_eq(&first, &state.selected.as_ref().unwrap().1));
    assert!(!state.preserve_filter_selection);
    state.begin_filter("no matches".into());
    state.next_request();
    let mut snapshot = page(&state, 0, 0, 0);
    snapshot.loaded = 10;
    assert!(state.consume_snapshot(snapshot).unwrap().clear_diff);
    assert!(state.selected.is_none());
}

#[test]
fn empty_filtered_history_preserves_local_selection_and_refreshes_local_diff() {
    let mut state = loaded();
    let status = LocalStatus {
        unstaged: true,
        ..Default::default()
    };
    state.update_local_rows(&status);
    state.select_row(0).unwrap();
    state.begin_filter("no matches".into());
    state.next_request();
    let snapshot = page(&state, 0, 0, 0);
    assert!(!state.consume_snapshot(snapshot).unwrap().clear_diff);
    assert_eq!(state.local_selection, Some(LocalChangeKind::Unstaged));
    assert!(matches!(
        state.update_local_rows(&status).selection,
        Some(Selection::Local(LocalChangeKind::Unstaged))
    ));
}

#[test]
fn forced_reload_releases_rows_after_capturing_bookmarks_and_anchor() {
    let mut state = loaded();
    state.select_row(3).unwrap();
    state.viewport = 5..10;
    let selected = state.selected.as_ref().unwrap().1.clone();
    let top = state.cached(5).unwrap();
    let update = state.reload(PathBuf::from("repository"), true);
    assert!(update.clear_diff);
    assert_eq!(update.request.anchor, Some(3));
    assert_eq!(update.request.source.reload_generation.0, 1);
    assert_eq!(
        update.request.restore.unwrap(),
        Restore {
            selected: Some(selected.commit.id.clone()),
            top: Some(top.commit.id.clone())
        }
    );
    assert!(state.snapshot.is_none());
    assert!(state.selected.is_none());
    // A second force reload before any new page must retain the bookmarks.
    let second = state.reload(PathBuf::from("repository"), true);
    assert_eq!(second.request.source.reload_generation.0, 2);
    assert_eq!(
        second.request.restore.unwrap().top,
        Some(top.commit.id.clone())
    );
}

#[test]
fn forced_reload_refreshes_local_selection_while_preserving_the_top_commit() {
    let mut state = loaded();
    state.update_local_rows(&LocalStatus {
        unstaged: true,
        ..Default::default()
    });
    state.select_row(0).unwrap();
    state.viewport = 6..11;
    let top = state.cached(6).unwrap();
    let update = state.reload(PathBuf::from("repository"), true);
    assert!(matches!(
        update.selection,
        Some(Selection::Local(LocalChangeKind::Unstaged))
    ));
    assert_eq!(state.local_selection, Some(LocalChangeKind::Unstaged));
    assert_eq!(
        update.request.restore.unwrap().top,
        Some(top.commit.id.clone())
    );
    assert!(state.snapshot.is_none());
}

#[test]
fn navigation_stops_at_visible_rows_and_uses_pending_selection() {
    let mut state = loaded();
    assert_eq!(state.navigation_index(isize::MIN), Some(0));
    assert_eq!(state.navigation_index(isize::MAX), Some(9));
    assert!(state.select_row(10).is_none());
    state.pending_selection = Some(7);
    assert_eq!(state.navigation_index(1), Some(8));
    let snapshot = page(&state, 0, 0, 0);
    state.consume_snapshot(snapshot).unwrap();
    assert_eq!(state.navigation_index(1), None);
    assert!(state.select_row(0).is_none());
    state.update_local_rows(&LocalStatus {
        staged: true,
        ..Default::default()
    });
    assert_eq!(state.navigation_index(1), Some(0));
    assert!(matches!(
        state.select_row(0),
        Some(RowSelection::Ready(Selection::Local(
            LocalChangeKind::Staged
        )))
    ));
}

#[test]
fn viewport_demand_is_bounded_and_does_not_replace_an_active_restore() {
    let mut state = loaded();
    state.update_local_rows(&LocalStatus {
        staged: true,
        ..Default::default()
    });
    assert!(state.ensure_rows(0..1).is_none());
    assert!(state.ensure_rows(1..8).is_none());
    let request = state.ensure_rows(1601..1621).unwrap();
    assert_eq!(request.range, 1408..1920);
    assert!(state.ensure_rows(1601..1621).is_none());
    state.request.restore = Some(Restore {
        selected: None,
        top: None,
    });
    assert!(state.ensure_rows(3000..3020).is_none());
    assert_eq!(state.request.range, 1408..1920);
    assert_eq!(state.viewport, 3000..3020);
}

#[test]
fn explicit_selection_updates_reload_bookmarks_and_repository_reset_drops_them() {
    let mut state = loaded();
    state.reload(PathBuf::from("repository"), false);
    state.select_row(4).unwrap();
    assert!(state.update_restore_selection());
    assert_eq!(
        state.request.restore.as_ref().unwrap().selected,
        Some(state.cached(4).unwrap().commit.id.clone())
    );
    state.reset_repository();
    let update = state.reload(PathBuf::from("another-repository"), false);
    assert!(update.clear_diff);
    assert!(update.request.restore.is_none());
    assert!(update.request.anchor.is_none());
    assert_eq!(
        update.request.source.repository,
        PathBuf::from("another-repository")
    );
    assert_eq!(state.pending_selection, Some(0));
    assert!(state.snapshot.is_none());
}

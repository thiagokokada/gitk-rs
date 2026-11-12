//! Selection and viewport transitions, independent of the windowing backend.

#[cfg(test)]
mod tests;

use super::{Entry, Request, Restore, Snapshot};
use crate::git::CommitId;
use crate::workspace::LocalStatus;
use std::{ops::Range, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalChangeKind {
    Unstaged,
    Staged,
}

pub enum Selection {
    Commit(Arc<Entry>),
    Local(LocalChangeKind),
}

pub enum RowSelection {
    Ready(Selection),
    Pending,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScrollTarget {
    Top(usize),
    Center(usize),
}

#[derive(Default)]
pub struct SnapshotUpdate {
    pub clear_diff: bool,
    pub selection: Option<Selection>,
    pub scroll: Option<ScrollTarget>,
    pub request: Option<Request>,
}

pub struct LocalRowsUpdate {
    pub selection: Option<Selection>,
    pub top: Option<usize>,
    pub reload: bool,
}

pub struct ReloadUpdate {
    pub clear_diff: bool,
    pub selection: Option<Selection>,
    pub request: Request,
    pub top: Option<usize>,
}

pub struct HistoryViewState {
    pub request: Request,
    pub snapshot: Option<Snapshot>,
    pub selected: Option<(usize, Arc<Entry>)>,
    pub pending_selection: Option<usize>,
    pub local_rows: Vec<LocalChangeKind>,
    pub local_selection: Option<LocalChangeKind>,
    pub reload_selection: Option<CommitId>,
    pub preserve_filter_selection: bool,
    pub viewport: Range<usize>,
    // A vanished local row should lead to the newest commit, not bookmark the
    // old HEAD while status and history are being reconciled independently.
    follow_history_top: bool,
}

impl HistoryViewState {
    pub fn new(request: Request) -> Self {
        Self {
            request,
            snapshot: None,
            selected: None,
            pending_selection: Some(0),
            local_rows: Vec::new(),
            local_selection: None,
            reload_selection: None,
            preserve_filter_selection: false,
            viewport: 0..10,
            follow_history_top: false,
        }
    }

    pub fn next_request(&mut self) -> Request {
        self.request.anchor = self.selected.as_ref().map(|s| s.1.ordinal);
        self.request.id += 1;
        self.request.clone()
    }

    pub fn begin_filter(&mut self, query: String) {
        self.follow_history_top = false;
        self.request.query = query;
        self.request.range = 0..128;
        self.pending_selection = None;
        self.preserve_filter_selection = self.local_selection.is_none();
        if let Some(restore) = &mut self.request.restore {
            restore.top = None;
        }
    }

    pub fn consume_snapshot(&mut self, snapshot: Snapshot) -> Option<SnapshotUpdate> {
        if snapshot.generation != self.request.generation
            || snapshot.source != self.request.source
            || snapshot.request_id != self.request.id
            || snapshot.error.is_none() && snapshot.restored.as_ref().is_some_and(|r| r.pending)
        {
            // Keep the current rows and viewport until bookmarks have been found.
            return None;
        }
        let mut update = SnapshotUpdate {
            clear_diff: self.local_selection.is_none()
                && (snapshot.eof && snapshot.visible == 0
                    || snapshot.error.is_some() && self.selected.is_none()),
            ..Default::default()
        };
        let restored = snapshot.restored.clone();
        self.snapshot = Some(snapshot);
        if self.request.restore.is_some() {
            self.request.restore = None;
            self.reload_selection = None;
            self.preserve_filter_selection = false;
            self.pending_selection = None;
            if self.local_selection.is_some() {
                self.selected = None;
            } else if let Some((index, entry)) = restored.as_ref().and_then(|r| r.selected.clone())
            {
                self.select_commit(index + self.local_rows.len(), entry.clone());
                update.selection = Some(Selection::Commit(entry));
            } else {
                self.selected = None;
                self.pending_selection = Some(self.local_rows.len());
            }
            let top = restored
                .as_ref()
                .and_then(|r| r.top)
                .or_else(|| {
                    self.selected
                        .as_ref()
                        .map(|s| s.0.saturating_sub(self.local_rows.len()))
                })
                .unwrap_or(0)
                + self.local_rows.len();
            // A second reload can arrive before the UI applies this scroll.
            self.set_top(top);
            update.scroll = Some(ScrollTarget::Top(top));
            let start = self.snapshot.as_ref().unwrap().start;
            self.request.range = start..start + 512;
            update.request = Some(self.next_request());
        }
        if self.preserve_filter_selection {
            let snapshot = self.snapshot.as_ref().unwrap();
            if let Some(index) = snapshot.anchor_position
                && let Some((_, entry)) = &self.selected
            {
                let index = index + self.local_rows.len();
                let entry = entry.clone();
                self.select_commit(index, entry.clone());
                update.selection = Some(Selection::Commit(entry));
                update.scroll = Some(ScrollTarget::Center(index));
                self.preserve_filter_selection = false;
            } else if !snapshot.filtering && snapshot.visible > 0 {
                self.selected = None;
                self.pending_selection = Some(self.local_rows.len());
                self.preserve_filter_selection = false;
            } else if snapshot.eof && snapshot.visible == 0 {
                self.selected = None;
                self.preserve_filter_selection = false;
            }
        }
        if let Some(selection) = self.resolve_pending() {
            update.selection = Some(selection);
        }
        Some(update)
    }

    pub fn update_local_rows(&mut self, status: &LocalStatus) -> LocalRowsUpdate {
        let old_count = self.local_rows.len();
        let old_top = self.viewport.start;
        self.local_rows.clear();
        if status.unstaged {
            self.local_rows.push(LocalChangeKind::Unstaged);
        }
        if status.staged {
            self.local_rows.push(LocalChangeKind::Staged);
        }
        if let Some((index, _)) = &mut self.selected {
            *index = index.saturating_sub(old_count) + self.local_rows.len();
        }
        if let Some(index) = &mut self.pending_selection {
            *index = index.saturating_sub(old_count) + self.local_rows.len();
        }
        let top = (old_count != self.local_rows.len() && old_top > old_count).then(|| {
            let top = old_top - old_count + self.local_rows.len();
            // Bookmarks must already use the new local-row count before scrolling.
            self.set_top(top);
            top
        });
        let mut selection = None;
        let mut reload = false;
        if let Some(kind) = self.local_selection {
            if self.local_rows.contains(&kind) {
                selection = Some(Selection::Local(kind));
            } else {
                self.local_selection = None;
                self.selected = None;
                self.pending_selection = Some(self.local_rows.len());
                self.follow_history_top = true;
                reload = true;
            }
        }
        if !reload
            && self.snapshot.as_ref().is_some_and(|s| s.visible == 0)
            && self.selected.is_none()
            && self.local_selection.is_none()
            && let Some(&kind) = self.local_rows.first()
        {
            self.select_local(kind);
            selection = Some(Selection::Local(kind));
        }
        LocalRowsUpdate {
            selection,
            top,
            reload,
        }
    }

    fn select_local(&mut self, kind: LocalChangeKind) {
        self.selected = None;
        self.local_selection = Some(kind);
        self.pending_selection = None;
    }

    fn select_commit(&mut self, index: usize, entry: Arc<Entry>) {
        self.local_selection = None;
        self.pending_selection = None;
        self.selected = Some((index, entry));
    }

    pub fn select_row(&mut self, index: usize) -> Option<RowSelection> {
        let count = self.local_rows.len() + self.snapshot.as_ref().map_or(0, |s| s.visible);
        if index >= count {
            return None;
        }
        self.follow_history_top = false;
        if let Some(&kind) = self.local_rows.get(index) {
            self.select_local(kind);
            Some(RowSelection::Ready(Selection::Local(kind)))
        } else if let Some(entry) = self.cached(index) {
            self.select_commit(index, entry.clone());
            Some(RowSelection::Ready(Selection::Commit(entry)))
        } else {
            self.pending_selection = Some(index);
            Some(RowSelection::Pending)
        }
    }

    fn resolve_pending(&mut self) -> Option<Selection> {
        let index = self.pending_selection?;
        if let Some(&kind) = self.local_rows.get(index) {
            self.select_local(kind);
            Some(Selection::Local(kind))
        } else if let Some(entry) = self.cached(index) {
            self.select_commit(index, entry.clone());
            Some(Selection::Commit(entry))
        } else {
            None
        }
    }

    pub fn cached(&self, index: usize) -> Option<Arc<Entry>> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .rows
            .get(
                index
                    .checked_sub(self.local_rows.len())?
                    .checked_sub(snapshot.start)?,
            )
            .cloned()
    }

    pub fn ensure_rows(&mut self, range: Range<usize>) -> Option<Request> {
        self.viewport = range.clone();
        if self.request.restore.is_some() {
            return None;
        }
        let range = range.start.saturating_sub(self.local_rows.len())
            ..range.end.saturating_sub(self.local_rows.len());
        if range.is_empty()
            || self
                .snapshot
                .as_ref()
                .is_some_and(|s| range.start >= s.start && range.end <= s.start + s.rows.len())
        {
            return None;
        }
        let start = (range.start / 128).saturating_sub(1) * 128;
        let requested = start..start + 512;
        if self.request.range == requested {
            return None;
        }
        self.request.range = requested;
        Some(self.next_request())
    }

    pub fn navigation_index(&self, delta: isize) -> Option<usize> {
        let snapshot = self.snapshot.as_ref()?;
        let maximum = (snapshot.visible + self.local_rows.len()).checked_sub(1)?;
        Some(
            self.pending_selection
                .or_else(|| self.selected.as_ref().map(|s| s.0))
                .or_else(|| {
                    self.local_selection
                        .and_then(|kind| self.local_rows.iter().position(|row| *row == kind))
                })
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(maximum),
        )
    }

    pub fn update_restore_selection(&mut self) -> bool {
        if self.request.restore.is_some() {
            if let Some((_, entry)) = &self.selected {
                self.request.restore.as_mut().unwrap().selected = Some(entry.commit.id.clone());
                self.reload_selection = Some(entry.commit.id.clone());
            } else {
                self.request.restore = None;
                self.reload_selection = None;
            }
            true
        } else {
            self.preserve_filter_selection
        }
    }

    pub fn reload(&mut self, repository: PathBuf, force: bool) -> ReloadUpdate {
        let scroll_top = (self.follow_history_top && self.local_selection.is_none()).then(|| {
            self.selected = None;
            self.snapshot = None;
            self.request.restore = None;
            self.reload_selection = None;
            self.set_top(0);
            0
        });
        if force {
            self.request.source.reload_generation.0 += 1;
        }
        self.request.source.repository = repository;
        let index = self.viewport.start.saturating_sub(self.local_rows.len());
        let previous_restore = self.request.restore.as_ref();
        let selected = self
            .selected
            .as_ref()
            .map(|s| s.1.commit.id.clone())
            .or_else(|| previous_restore.and_then(|r| r.selected.clone()));
        let top = self
            .cached(self.viewport.start)
            .map(|e| e.commit.id.clone())
            .or_else(|| previous_restore.and_then(|r| r.top.clone()));
        self.reload_selection = selected.clone().or(top.clone());
        self.request.restore = self
            .reload_selection
            .as_ref()
            .map(|_| Restore { selected, top });
        self.request.generation.0 += 1;
        let start = (index / 128).saturating_sub(1) * 128;
        self.request.range = start..start + 512;
        self.pending_selection = None;
        let clear_diff = self.request.restore.is_none() || force;
        if self.request.restore.is_none() {
            self.snapshot = None;
            self.selected = None;
            self.pending_selection = self
                .local_selection
                .is_none()
                .then_some(self.local_rows.len());
        }
        self.preserve_filter_selection = false;
        // Preserve the request anchor before a forced reload releases its rows.
        let request = self.next_request();
        let selection = if force {
            self.snapshot = None;
            self.selected = None;
            self.local_selection.map(|kind| {
                self.select_local(kind);
                Selection::Local(kind)
            })
        } else {
            None
        };
        ReloadUpdate {
            clear_diff,
            selection,
            request,
            top: scroll_top,
        }
    }

    pub fn reset_repository(&mut self) {
        self.follow_history_top = false;
        self.local_rows.clear();
        self.selected = None;
        self.local_selection = None;
        self.reload_selection = None;
        self.request.restore = None;
        self.snapshot = None;
    }

    fn set_top(&mut self, top: usize) {
        self.viewport = top..top.saturating_add(self.viewport.len());
    }
}

#[cfg(test)]
mod tests;

mod colors;
mod dialogs;
mod diff_view;
mod font_preferences;

use colors::Palette;
use font_preferences::{FontCatalogGeneration, FontChoices};
use gitk_rs::{
    cli::{Config, Theme},
    diff::{DiffEngine, DiffHandle, DiffSettings},
    graph::{GraphLayout, GraphRow, text_graph},
    history::{
        Entry, Generation, HistoryEngine, HistoryHandle, RefKind, Request,
        Restore as HistoryRestore, Snapshot,
    },
    preferences::Preferences,
    timing::{Phase, Timing, TimingGuard},
    workspace::{
        Operation, WorkspaceEngine, WorkspaceGeneration, WorkspaceHandle, WorkspaceRequest,
        WorkspaceSnapshot,
    },
};
use gpui_kit::component::{
    ThemeMode,
    button::Button,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
    resizable::{resizable_panel, v_resizable},
    scroll::{ScrollableElement, Scrollbar, ScrollbarMode},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tooltip::Tooltip,
};
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, Focusable, KeyBinding, PathBuilder,
    ScrollHandle, ScrollStrategy, SharedString, Subscription, Task, TestSupportExt,
    UniformListDecoration, UniformListScrollHandle, Window, WindowBounds, WindowOptions, actions,
    canvas, div, fill, point, prelude::*, px, rgb, size, uniform_list,
};
use std::{
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

actions!(
    gitk,
    [
        Quit,
        Previous,
        Next,
        First,
        Last,
        Reload,
        FocusFilter,
        BlurFilter,
        HistoryPageUp,
        HistoryPageDown,
        ToggleWatch,
        OpenRepository,
        OpenRepositoryPath,
        OpenBranches,
        Help,
        HelpScrollUp,
        HelpScrollDown,
        HelpPageUp,
        HelpPageDown,
        HelpTop,
        HelpBottom,
        About,
        CloseDialog,
        BranchUp,
        BranchDown,
        BranchAccept,
        FontPreferences,
        ApplyPreferences,
        ToggleTheme,
        CopyCommit
    ]
);

struct WindowServices {
    history: HistoryHandle,
    workspace: WorkspaceHandle,
    diff: DiffHandle,
}

struct HistoryViewport(Entity<HistoryWindow>);

impl UniformListDecoration for HistoryViewport {
    fn compute(
        &self,
        range: Range<usize>,
        _: Bounds<gpui_kit::Pixels>,
        _: gpui_kit::Point<gpui_kit::Pixels>,
        _: gpui_kit::Pixels,
        _: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.0.update(cx, |view, cx| {
            let layout = view.graph_layout.clone();
            view.ensure_rows(range);
            if view.graph_layout != layout {
                cx.notify();
            }
        });
        div().into_any_element()
    }
}

enum DialogKind {
    Help,
    About,
    Branches,
    Preferences,
    OpenRepository,
}

struct HistoryWindow {
    config: Config,
    preferences: Preferences,
    ui_font_select: Entity<SelectState<FontChoices>>,
    fixed_font_select: Entity<SelectState<FontChoices>>,
    _font_subscriptions: Vec<Subscription>,
    font_catalog: Option<Vec<SharedString>>,
    font_catalog_generation: FontCatalogGeneration,
    _font_catalog_task: Option<Task<()>>,
    font_size_input: Entity<InputState>,
    reload_selection: Option<gitk_rs::git::CommitId>,
    focus: FocusHandle,
    timing: Timing,
    history: HistoryHandle,
    workspace: WorkspaceHandle,
    workspace_generation: WorkspaceGeneration,
    workspace_snapshot: Option<WorkspaceSnapshot>,
    local_rows: Vec<bool>,
    local_selection: Option<bool>,
    dialog: Option<DialogKind>,
    dialog_focus: FocusHandle,
    restore_focus: bool,
    restore_repository_focus: bool,
    repository_prompt_pending: bool,
    branches: Vec<String>,
    filtered_branches: Vec<usize>,
    branch_filter: Entity<InputState>,
    _branch_subscription: Subscription,
    repository_input: Entity<InputState>,
    _repository_subscription: Subscription,
    branch_index: usize,
    branch_scroll: UniformListScrollHandle,
    help_scroll: ScrollHandle,
    operation_id: u64,
    pending_operation: Option<(u64, bool)>,
    operation_message: String,
    request: Request,
    snapshot: Option<Snapshot>,
    selected: Option<(usize, Arc<Entry>)>,
    pending_selection: Option<usize>,
    filter: Entity<InputState>,
    detail: SharedString,
    diff: Entity<diff_view::DiffPane>,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
    _history_task: Task<()>,
    filter_task: Option<Task<()>>,
    preserve_filter_selection: bool,
    viewport: Range<usize>,
    graph_layout: GraphLayout,
    graph_width: f32,
    graph_resize: Option<(gpui_kit::Pixels, f32)>,
}

impl HistoryWindow {
    fn new(
        config: Config,
        timing: Timing,
        services: WindowServices,
        preferences: Preferences,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        apply_fonts(&preferences, cx);
        let WindowServices {
            history,
            workspace,
            diff: diff_handle,
        } = services;
        let ui_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![SharedString::from(preferences.ui_font.clone())]),
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let fixed_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![SharedString::from(preferences.fixed_font.clone())]),
                Some(gpui_kit::component::IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let font_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preferences.font_size.to_string())
                .step(1.)
                .min(8.)
                .max(24.)
        });
        let mut font_subscriptions = Vec::new();
        for selector in [&ui_font_select, &fixed_font_select] {
            font_subscriptions.push(
                cx.subscribe(selector, |_, _, _: &SelectEvent<FontChoices>, cx| {
                    cx.notify()
                }),
            );
        }
        font_subscriptions.push(
            cx.subscribe(&font_size_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );
        let branch_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter local branches"));
        let branch_subscription = cx.subscribe_in(
            &branch_filter,
            window,
            |this, _, event, _, cx| match event {
                InputEvent::Change => this.filter_branches(cx),
                InputEvent::PressEnter { .. } => this.switch_branch(this.branch_index, cx),
                _ => {}
            },
        );
        let repository_input = cx.new(|cx| InputState::new(window, cx));
        let repository_subscription =
            cx.subscribe_in(&repository_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && matches!(this.dialog, Some(DialogKind::OpenRepository))
                {
                    this.apply_repository_path(cx);
                }
            });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Hash, author or commit message"));
        let subscription = cx.subscribe_in(&filter, window, |this, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.filter_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.request.query = query;
                        this.request.range = 0..128;
                        this.pending_selection = None;
                        this.preserve_filter_selection = this.local_selection.is_none();
                        if let Some(restore) = &mut this.request.restore {
                            restore.top = None;
                        }
                        this.detail = "Filtering commits…".into();
                        this.submit();
                        this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    });
                }));
            }
        });
        let task = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        // Apply HEAD changes before accepting history so a second
                        // reload cannot bookmark new rows with the old viewport.
                        this.consume_workspace(cx);
                        this.consume_snapshot(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let diff = cx.new(|cx| {
            diff_view::DiffPane::new(
                diff_handle,
                DiffSettings {
                    syntax: config.syntax,
                    dark: config.theme == Theme::Dark,
                },
                cx,
            )
        });
        diff.update(cx, |diff, cx| {
            diff.set_font(
                preferences.fixed_font.clone().into(),
                preferences.font_size,
                cx,
            )
        });
        Self {
            preferences,
            ui_font_select,
            fixed_font_select,
            _font_subscriptions: font_subscriptions,
            font_catalog: None,
            font_catalog_generation: FontCatalogGeneration(0),
            _font_catalog_task: None,
            font_size_input,
            reload_selection: None,
            diff,
            workspace,
            workspace_generation: WorkspaceGeneration(1),
            workspace_snapshot: None,
            local_rows: Vec::new(),
            local_selection: None,
            dialog: None,
            dialog_focus: cx.focus_handle(),
            restore_focus: false,
            restore_repository_focus: false,
            repository_prompt_pending: false,
            branches: Vec::new(),
            filtered_branches: Vec::new(),
            branch_filter,
            _branch_subscription: branch_subscription,
            repository_input,
            _repository_subscription: repository_subscription,
            branch_index: 0,
            branch_scroll: UniformListScrollHandle::new(),
            help_scroll: ScrollHandle::new(),
            operation_id: 0,
            pending_operation: None,
            operation_message: String::new(),
            config,
            focus,
            timing,
            history,
            request,
            snapshot: None,
            selected: None,
            pending_selection: Some(0),
            filter,
            detail: "Loading commits…".into(),
            scroll: UniformListScrollHandle::new(),
            _subscription: subscription,
            _history_task: task,
            filter_task: None,
            preserve_filter_selection: false,
            viewport: 0..10,
            graph_layout: GraphLayout::default(),
            graph_width: 280.,
            graph_resize: None,
        }
    }

    fn submit(&mut self) {
        self.request.anchor = self.selected.as_ref().map(|s| s.1.ordinal);
        self.request.id += 1;
        self.history.request(self.request.clone());
    }

    fn consume_snapshot(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.history.take_snapshot() else {
            return;
        };
        if snapshot.generation != self.request.generation || snapshot.request_id != self.request.id
        {
            return;
        }
        if snapshot.error.is_none() && snapshot.restored.as_ref().is_some_and(|r| r.pending) {
            // Keep the current viewport until its commit IDs have been located.
            // Publishing the partial row count here would clamp a deep scroll.
            return;
        }
        if self.config.verbose {
            eprintln!(
                "gitk-rs: history generation={} branch={} loaded={} visible={}",
                snapshot.generation.0, snapshot.branch, snapshot.loaded, snapshot.visible
            );
        }
        if let Some(error) = &snapshot.error {
            self.detail = error.clone().into();
        } else if snapshot.eof && snapshot.visible == 0 {
            self.detail = if snapshot.loaded == 0 {
                "This repository has no commits."
            } else {
                "No matching commits."
            }
            .into();
        }
        if self.local_selection.is_none()
            && (snapshot.eof && snapshot.visible == 0
                || snapshot.error.is_some() && self.selected.is_none())
        {
            self.diff
                .update(cx, |diff, cx| diff.clear(&self.detail, cx));
        }
        let restoring = self.request.restore.is_some();
        let restored = snapshot.restored.clone();
        self.snapshot = Some(snapshot);
        if restoring {
            self.request.restore = None;
            self.reload_selection = None;
            self.preserve_filter_selection = false;
            self.pending_selection = None;
            if let Some((index, entry)) = restored.as_ref().and_then(|r| r.selected.clone()) {
                self.set_selection(index + self.local_rows.len(), entry, cx);
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
                .unwrap_or(0);
            let top = top + self.local_rows.len();
            // A second reload can arrive before layout applies the deferred scroll.
            // Bookmark indices must already address the restored snapshot's rows.
            self.viewport = top..top.saturating_add(self.viewport.len());
            self.scroll.scroll_to_item_strict(top, ScrollStrategy::Top);
            let start = self.snapshot.as_ref().unwrap().start;
            self.request.range = start..start + 512;
            self.submit();
        }
        if self.preserve_filter_selection {
            let snapshot = self.snapshot.as_ref().unwrap();
            if let Some(index) = snapshot.anchor_position
                && let Some((_, entry)) = &self.selected
            {
                self.set_selection(index + self.local_rows.len(), entry.clone(), cx);
                self.scroll
                    .scroll_to_item(index + self.local_rows.len(), ScrollStrategy::Center);
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
        if let Some(staged) = self
            .pending_selection
            .and_then(|i| self.local_rows.get(i))
            .copied()
        {
            self.select_local(staged, cx);
        }
        if let Some(index) = self.pending_selection
            && let Some(entry) = self.cached(index)
        {
            self.pending_selection = None;
            self.set_selection(index, entry, cx);
        }
        cx.notify();
    }

    fn refresh_workspace(&self) {
        self.workspace.configure(WorkspaceRequest {
            generation: self.workspace_generation,
            repository: self.config.repository.clone(),
            watch: self.config.watch,
        });
    }

    fn consume_workspace(&mut self, cx: &mut Context<Self>) {
        if let Some(result) = self.workspace.take_result()
            && result.generation == self.workspace_generation
            && self.pending_operation.is_some_and(|p| p.0 == result.id)
        {
            let switching = self.pending_operation.take().unwrap().1;
            match result.result {
                Ok(branches) if !switching => {
                    self.branches = branches;
                    self.filter_branches(cx);
                    self.branch_index = 0;
                    self.operation_message.clear();
                }
                Ok(_) => {
                    self.dialog = None;
                    self.restore_focus = true;
                    self.operation_message = "branch switched".into();
                    // The forced worker probe will supply fresh state. Avoid a
                    // second HEAD-change reload losing the pending selection.
                    self.workspace_snapshot = None;
                    self.reload(cx);
                }
                Err(error) => self.operation_message = error,
            }
            cx.notify();
        }
        let Some(snapshot) = self.workspace.take_snapshot() else {
            return;
        };
        if snapshot.generation != self.workspace_generation {
            return;
        }
        if self.config.verbose {
            eprintln!("gitk-rs: repository refresh {}", snapshot.revision);
        }
        let refs_changed = self
            .workspace_snapshot
            .as_ref()
            .is_some_and(|old| old.refs_fingerprint != snapshot.refs_fingerprint);
        let head_changed = self
            .workspace_snapshot
            .as_ref()
            .and_then(|s| s.repository.as_ref())
            .zip(snapshot.repository.as_ref())
            .is_some_and(|(old, new)| old.head != new.head || old.branch != new.branch);
        let old_count = self.local_rows.len();
        let old_top = self.viewport.start;
        self.local_rows.clear();
        if snapshot.status.unstaged {
            self.local_rows.push(false);
        }
        if snapshot.status.staged {
            self.local_rows.push(true);
        }
        if let Some((index, _)) = &mut self.selected {
            *index = index.saturating_sub(old_count) + self.local_rows.len();
        }
        if let Some(index) = &mut self.pending_selection {
            *index = index.saturating_sub(old_count) + self.local_rows.len();
        }
        if old_count != self.local_rows.len() && old_top > old_count {
            let top = old_top - old_count + self.local_rows.len();
            // HEAD may change before layout applies this scroll. Bookmarks
            // must already address the current local-row count.
            self.viewport = top..top.saturating_add(self.viewport.len());
            self.scroll.scroll_to_item_strict(top, ScrollStrategy::Top);
        }
        if let Some(staged) = self.local_selection {
            if self.local_rows.contains(&staged) {
                self.diff.update(cx, |diff, cx| {
                    diff.load_local(
                        self.config.repository.clone(),
                        staged,
                        snapshot.status_fingerprint,
                        cx,
                    )
                });
            } else {
                self.local_selection = None;
                let index = self.local_rows.len();
                if let Some(entry) = self.cached(index) {
                    self.set_selection(index, entry, cx);
                } else {
                    self.pending_selection = Some(index);
                }
            }
        }
        if let Some(error) = &snapshot.error {
            self.operation_message = error.clone();
        }
        self.workspace_snapshot = Some(snapshot);
        if self.snapshot.as_ref().is_some_and(|s| s.visible == 0)
            && self.selected.is_none()
            && self.local_selection.is_none()
            && !self.local_rows.is_empty()
        {
            self.select_local(self.local_rows[0], cx);
        }
        if head_changed {
            self.reload(cx);
        } else if refs_changed {
            self.request.refs_revision += 1;
            self.submit();
        }
        cx.notify();
    }

    fn select_local(&mut self, staged: bool, cx: &mut Context<Self>) {
        self.selected = None;
        self.local_selection = Some(staged);
        self.pending_selection = None;
        let revision = self
            .workspace_snapshot
            .as_ref()
            .map_or(0, |s| s.status_fingerprint);
        self.diff.update(cx, |diff, cx| {
            diff.load_local(self.config.repository.clone(), staged, revision, cx)
        });
    }

    fn cached(&self, index: usize) -> Option<Arc<Entry>> {
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

    fn ensure_rows(&mut self, range: Range<usize>) {
        let entries: Vec<_> = range
            .clone()
            .filter_map(|index| self.cached(index))
            .collect();
        self.graph_layout =
            GraphLayout::new(entries.iter().map(|entry| &entry.graph), self.graph_width);
        self.viewport = range.clone();
        if self.request.restore.is_some() {
            return;
        }
        let range = range.start.saturating_sub(self.local_rows.len())
            ..range.end.saturating_sub(self.local_rows.len());
        if range.is_empty() {
            return;
        }
        let covered = self
            .snapshot
            .as_ref()
            .is_some_and(|s| range.start >= s.start && range.end <= s.start + s.rows.len());
        if covered {
            return;
        }
        let start = (range.start / 128).saturating_sub(1) * 128;
        let requested = start..start + 512;
        if self.request.range != requested {
            self.request.range = requested;
            self.submit();
        }
    }

    fn set_selection(&mut self, index: usize, entry: Arc<Entry>, cx: &mut Context<Self>) {
        if self.config.verbose {
            eprintln!(
                "gitk-rs: selection generation={} commit={}",
                self.request.generation.0,
                entry.commit.id.to_hex()
            );
        }
        self.diff.update(cx, |diff, cx| {
            diff.load(
                self.config.repository.clone(),
                Arc::new(entry.commit.clone()),
                cx,
            )
        });
        self.local_selection = None;
        self.selected = Some((index, entry));
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        if let Some(staged) = self.local_rows.get(index).copied() {
            self.selected = None;
            self.local_selection = Some(staged);
            self.pending_selection = None;
            let revision = self
                .workspace_snapshot
                .as_ref()
                .map_or(0, |s| s.status_fingerprint);
            self.diff.update(cx, |diff, cx| {
                diff.load_local(self.config.repository.clone(), staged, revision, cx)
            });
        } else if let Some(entry) = self.cached(index) {
            self.set_selection(index, entry, cx);
        } else {
            self.pending_selection = Some(index);
            self.ensure_rows(index..index + 1);
        }
        if self.request.restore.is_some() {
            if let Some((_, entry)) = &self.selected {
                self.request.restore.as_mut().unwrap().selected = Some(entry.commit.id.clone());
                self.reload_selection = Some(entry.commit.id.clone());
            } else {
                self.request.restore = None;
                self.reload_selection = None;
            }
            self.submit();
        } else if self.preserve_filter_selection {
            self.submit();
        }
        self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let previous_focus = window.focused(cx);
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let maximum =
            (snapshot.visible + self.local_rows.len()).saturating_sub(usize::from(snapshot.eof));
        let index = self
            .pending_selection
            .or_else(|| self.selected.as_ref().map(|s| s.0))
            .or_else(|| {
                self.local_selection
                    .and_then(|staged| self.local_rows.iter().position(|s| *s == staged))
            })
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(maximum);
        self.select(index, window, cx);
        if let Some(focus) = previous_focus {
            focus.focus(window, cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.request.repository = self.config.repository.clone();
        let index = self.viewport.start.saturating_sub(self.local_rows.len());
        let selected = self.selected.as_ref().map(|s| s.1.commit.id.clone());
        let top = self
            .cached(self.viewport.start)
            .map(|e| e.commit.id.clone());
        self.reload_selection = selected.clone().or(top.clone());
        self.request.restore = self
            .reload_selection
            .as_ref()
            .map(|_| HistoryRestore { selected, top });
        self.request.generation.0 += 1;
        let start = (index / 128).saturating_sub(1) * 128;
        self.request.range = start..start + 512;
        self.pending_selection = None;
        if self.request.restore.is_none() {
            self.snapshot = None;
            self.selected = None;
            self.pending_selection = self
                .local_selection
                .is_none()
                .then_some(self.local_rows.len());
            self.diff
                .update(cx, |diff, cx| diff.clear("Loading commits…", cx));
        }
        self.preserve_filter_selection = false;
        self.detail = "Loading commits…".into();
        self.submit();
        self.refresh_workspace();
        cx.notify();
    }

    fn toggle_auto_reload(&mut self, cx: &mut Context<Self>) {
        self.config.watch = !self.config.watch;
        self.reload(cx);
    }

    fn status_text(&self) -> String {
        let mut status = self.snapshot.as_ref().map_or_else(
            || "loading commits…".into(),
            |s| {
                if let Some(error) = &s.error {
                    return error.clone();
                }
                format!(
                    "showing {} / {} loaded commits on {}{}{}",
                    s.visible,
                    s.loaded,
                    s.branch,
                    if s.eof {
                        " — end of history"
                    } else {
                        " — more available"
                    },
                    if s.filtering { " — filtering…" } else { "" }
                )
            },
        );
        if !self.operation_message.is_empty() && self.dialog.is_none() {
            status.push_str("  — ");
            status.push_str(&self.operation_message);
        }
        if let Some(error) = self
            .workspace_snapshot
            .as_ref()
            .and_then(|s| s.watcher_error.as_deref())
        {
            status.push_str("  — ");
            status.push_str(error);
        }
        status
    }

    fn row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = Palette::for_theme(self.config.theme);
        if let Some(staged) = self.local_rows.get(index).copied() {
            let selected = self.local_selection == Some(staged);
            let unmerged = self
                .workspace_snapshot
                .as_ref()
                .is_some_and(|s| s.status.unmerged);
            return div()
                .id(("local", index))
                .w_full()
                .h(px((self.preferences.font_size * 2.).max(26.)))
                .flex()
                .items_center()
                .px_2()
                .bg(rgb(if selected {
                    palette.selection
                } else if staged {
                    palette.staged
                } else {
                    palette.unstaged
                }))
                .text_color(rgb(palette.local_foreground))
                .cursor_pointer()
                .child(if staged {
                    "●  Local changes checked into index but not committed"
                } else if unmerged {
                    "●  Unstaged changes (merge conflicts)"
                } else {
                    "●  Local uncommitted changes, not checked in to index"
                })
                .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
                .into_any_element();
        }
        let Some(entry) = self.cached(index) else {
            return div()
                .id(("loading", index))
                .h(px((self.preferences.font_size * 2.).max(26.)))
                .px_2()
                .child("Loading more commits…")
                .into_any_element();
        };
        let is_selected = self
            .selected
            .as_ref()
            .is_some_and(|s| s.1.commit.id == entry.commit.id);
        let background = if is_selected {
            colors::SOLID_SELECTION
        } else {
            palette.row
        };
        let labels = self
            .snapshot
            .as_ref()
            .and_then(|s| s.refs.get(&entry.commit.id))
            .cloned()
            .unwrap_or_default();
        let text_limit = (((self.graph_width / (self.preferences.font_size * 0.65) - 4.) / 2.)
            .floor() as usize)
            .clamp(1, 12);
        let graph_text = self
            .config
            .text_graph
            .then(|| text_graph(&entry.graph, text_limit));
        let width = graph_text
            .as_ref()
            .map_or(self.graph_layout.width(), |text| {
                text.chars().count() as f32 * self.preferences.font_size * 0.65 + 12.
            });
        let graph: AnyElement = if let Some(text) = graph_text {
            div()
                .font_family(SharedString::from(self.preferences.fixed_font.clone()))
                .child(text)
                .into_any_element()
        } else {
            let graph = entry.graph.clone();
            let layout = self.graph_layout.clone();
            let filtered = !self.request.query.trim().is_empty();
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| paint_graph(&graph, filtered, &layout, bounds, window),
            )
            .size_full()
            .into_any_element()
        };
        let badges = div()
            .absolute()
            .top(px(3.))
            .left(px(width))
            .flex()
            .gap_1()
            .children(labels.into_iter().map(|label| {
                let color = match label.kind {
                    RefKind::Head => colors::HEAD_BADGE,
                    RefKind::Branch => colors::BRANCH_BADGE,
                    RefKind::Remote => colors::REMOTE_BADGE,
                    RefKind::Tag => colors::TAG_BADGE,
                };
                div()
                    .px_1()
                    .text_size(px(11.))
                    .bg(rgb(color))
                    .text_color(rgb(colors::ON_SOLID))
                    .child(label.name)
            }));
        div()
            .id(("commit", index))
            .h(px((self.preferences.font_size * 2.).max(26.)))
            .w_full()
            .flex()
            .items_center()
            .bg(rgb(background))
            .when(is_selected, |row| row.text_color(rgb(colors::ON_SOLID)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
            .child(
                div()
                    .relative()
                    .w(px(self.graph_width))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .child(graph)
                    .child(badges),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .child(
                        div()
                            .w(px(self.preferences.font_size * 5.))
                            .flex_shrink_0()
                            .font_family(SharedString::from(self.preferences.fixed_font.clone()))
                            .child(entry.commit.id.to_hex()[..7].to_owned()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(entry.commit.summary().to_owned()),
                    ),
            )
            .child(
                div()
                    .w(px(280.))
                    .flex_shrink_0()
                    .truncate()
                    .px_2()
                    .child(format!(
                        "{} <{}>",
                        entry.commit.author.name, entry.commit.author.email
                    )),
            )
            .child(
                div().w(px(145.)).flex_shrink_0().truncate().px_2().child(
                    entry
                        .commit
                        .committer
                        .time
                        .chars()
                        .take(16)
                        .collect::<String>()
                        .replace('T', " "),
                ),
            )
            .context_menu({
                let reference = format!("{} {}", entry.commit.id, entry.commit.summary());
                move |menu, _, _| {
                    menu.item(PopupMenuItem::new("Copy commit reference").on_click({
                        let reference = reference.clone();
                        move |_, _, cx| {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                                reference.clone(),
                            ))
                        }
                    }))
                }
            })
            .into_any_element()
    }
}

fn apply_fonts(preferences: &Preferences, cx: &mut App) {
    let theme = gpui_kit::component::Theme::global_mut(cx);
    theme.font_family = preferences.ui_font.clone().into();
    theme.font_size = px(preferences.font_size);
    theme.mono_font_family = preferences.fixed_font.clone().into();
    theme.mono_font_size = px(preferences.font_size);
}

pub(crate) fn fixed_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

const IDLE_LANES_TOOLTIP: &str = "Idle lanes are history lines with no commit, branch, or merge junction in the visible rows. They are grouped into a dashed column to save space; commit dots and junctions stay separate.";

// Give tooltip text a concrete wrapping width; max-width on a flex popup alone
// does not constrain its content. Leave room for popup padding and viewport edges.
fn wrapped_tooltip(
    text: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyView {
    let text = text.into();
    let width = (window.viewport_size().width - px(64.)).clamp(px(40.), px(480.));
    let content_width = width - px(18.);
    let max_height = (window.viewport_size().height - px(64.)).max(px(24.));
    Tooltip::element(move |_, _| {
        div()
            .id("tooltip-layout")
            .test_support()
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .w(content_width)
            .child(
                div()
                    .w_full()
                    .max_h(max_height)
                    .child(
                        div()
                            .id("tooltip-text")
                            .test_support()
                            .w_full()
                            .pr_3()
                            .whitespace_normal()
                            .child(text.clone()),
                    )
                    .overflow_y_scrollbar()
                    .scroll_bounce(false),
            )
    })
    .w(width)
    .whitespace_normal()
    .build(window, cx)
}

fn paint_graph(
    graph: &GraphRow,
    filtered: bool,
    layout: &GraphLayout,
    bounds: Bounds<gpui_kit::Pixels>,
    window: &mut Window,
) {
    let colors = colors::GRAPH_LANES;
    let x = |lane: u16| bounds.left() + px(layout.x(lane));
    let radius = (layout.spacing * 0.4).clamp(2., 3.5);
    let stroke = 1.5;
    let middle = bounds.top() + bounds.size.height / 2.;
    if !filtered {
        let mut folded = [false; 200];
        for edge in graph.edges.iter() {
            if layout.is_collapsed(edge.from) && layout.is_collapsed(edge.to) {
                let column = layout.column(edge.from);
                if !folded[column] {
                    folded[column] = true;
                    for part in 0..3 {
                        let top = bounds.top() + bounds.size.height * (part as f32 / 3.);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(x(edge.from), top),
                                size(px(1.), bounds.size.height / 6.),
                            ),
                            rgb(colors::GRAPH_FOLDED),
                        ));
                    }
                }
                continue;
            }
            let mut path = PathBuilder::stroke(px(stroke));
            path.move_to(point(
                x(edge.from),
                if edge.parent { middle } else { bounds.top() },
            ));
            path.line_to(point(x(edge.to), bounds.bottom()));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(colors[edge.color as usize % colors.len()]));
            }
        }
        if graph.incoming
            && let Some(node) = graph.node
        {
            let mut path = PathBuilder::stroke(px(stroke));
            path.move_to(point(x(node), bounds.top()));
            path.line_to(point(x(node), middle));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(colors[graph.color as usize % colors.len()]));
            }
        }
    }
    if let Some(node) = graph.node {
        let mut quad = fill(
            Bounds::new(
                point(x(node) - px(radius), middle - px(radius)),
                size(px(radius * 2.), px(radius * 2.)),
            ),
            rgb(colors[graph.color as usize % colors.len()]),
        );
        quad.corner_radii = px(radius).into();
        window.paint_quad(quad);
    }
    if graph.overflow {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.right() - px(6.), middle - px(2.)),
                size(px(5.), px(4.)),
            ),
            rgb(colors::GRAPH_OVERFLOW),
        ));
    }
}

impl Render for HistoryWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.restore_focus)
            || self.dialog.is_none() && self.dialog_focus.is_focused(window)
        {
            self.focus.focus(window, cx);
        }
        if std::mem::take(&mut self.restore_repository_focus) {
            self.repository_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        let started = Instant::now();
        let timing = self.timing.clone();
        let history_ready = self.snapshot.as_ref().is_some_and(|s| !s.rows.is_empty());
        let selection_ready = self.selected.is_some();
        let palette = Palette::for_theme(self.config.theme);
        let section = || {
            div()
                .bg(rgb(palette.panel))
                .border_1()
                .border_color(rgb(palette.border))
        };
        let status = self.status_text();
        let count = self.snapshot.as_ref().map_or(0, |s| {
            self.local_rows.len() + s.visible + usize::from(!s.eof && s.error.is_none())
        });
        div()
            .relative()
            .on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(gpui_kit::MouseButton::Left)
                    && let Some((start, width)) = this.graph_resize {
                    let maximum = (f32::from(window.viewport_size().width) - 540.).clamp(160., 720.);
                    this.graph_width = (width + f32::from(event.position.x - start)).clamp(160., maximum);
                    this.ensure_rows(this.viewport.clone());
                    cx.notify();
                }
            }))
            .on_mouse_up(gpui_kit::MouseButton::Left, cx.listener(|this, _, _, _| this.graph_resize = None))
            .track_focus(&self.focus)
            .key_context("GitHistory")
            .on_action(|_: &Quit, _, cx| cx.quit())
            .on_action(cx.listener(|this, _: &Next, window, cx| this.move_selection(1, window, cx)))
            .on_action(
                cx.listener(|this, _: &Previous, window, cx| this.move_selection(-1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &First, window, cx| this.select(0, window, cx)))
            .on_action(cx.listener(|this, _: &Last, window, cx| {
                if let Some(s) = &this.snapshot {
                    this.select(
                        (s.visible + this.local_rows.len()).saturating_sub(1),
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &diff_view::PageDown, _, cx| {
                this.diff.update(cx, |diff, cx| diff.scroll_page(true, cx))
            }))
            .on_action(cx.listener(|this, _: &diff_view::PageUp, _, cx| {
                this.diff.update(cx, |diff, cx| diff.scroll_page(false, cx))
            }))
            .on_action(cx.listener(|this, _: &diff_view::LinesDown, _, cx| {
                this.diff.update(cx, |diff, cx| diff.scroll_lines(18, cx))
            }))
            .on_action(cx.listener(|this, _: &diff_view::LinesUp, _, cx| {
                this.diff.update(cx, |diff, cx| diff.scroll_lines(-18, cx))
            }))
            .on_action(cx.listener(|this, _: &FontPreferences, window, cx| {
                this.open_font_preferences(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ApplyPreferences, window, cx| {
                this.apply_preferences(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                this.config.theme = if this.config.theme == Theme::Dark {
                    Theme::Light
                } else {
                    Theme::Dark
                };
                gpui_kit::component::Theme::change(
                    if this.config.theme == Theme::Dark {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    },
                    Some(window),
                    cx,
                );
                apply_fonts(&this.preferences, cx);
                this.diff.update(cx, |diff, cx| {
                    diff.set_theme(this.config.theme == Theme::Dark, cx)
                });
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyCommit, _, cx| {
                if let Some((_, entry)) = &this.selected {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(format!(
                        "{} {}",
                        entry.commit.id,
                        entry.commit.summary()
                    )));
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleWatch, _, cx| {
                this.toggle_auto_reload(cx);
            }))
            .on_action(cx.listener(|this, _: &OpenRepository, _, cx| this.open_repository(cx)))
            .on_action(cx.listener(|this, _: &OpenRepositoryPath, window, cx| {
                this.open_repository_path(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenBranches, window, cx| this.open_branches(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Help, window, cx| {
                this.help_scroll.set_offset(point(px(0.), px(0.)));
                this.dialog = Some(DialogKind::Help);
                this.dialog_focus.focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &HelpScrollUp, _, cx| {
                this.scroll_shortcuts(px(-this.preferences.font_size * 1.5), cx);
            }))
            .on_action(cx.listener(|this, _: &HelpScrollDown, _, cx| {
                this.scroll_shortcuts(px(this.preferences.font_size * 1.5), cx);
            }))
            .on_action(cx.listener(|this, _: &HelpPageUp, _, cx| {
                this.scroll_shortcuts(-this.help_scroll.bounds().size.height, cx);
            }))
            .on_action(cx.listener(|this, _: &HelpPageDown, _, cx| {
                this.scroll_shortcuts(this.help_scroll.bounds().size.height, cx);
            }))
            .on_action(cx.listener(|this, _: &HelpTop, _, cx| {
                this.scroll_shortcuts(-this.help_scroll.max_offset().y, cx);
            }))
            .on_action(cx.listener(|this, _: &HelpBottom, _, cx| {
                this.scroll_shortcuts(this.help_scroll.max_offset().y, cx);
            }))
            .on_action(cx.listener(|this, _: &About, window, cx| {
                this.dialog = Some(DialogKind::About);
                this.dialog_focus.focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseDialog, window, cx| {
                this.dialog = None;
                this.focus.focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BranchUp, _, cx| {
                this.branch_index = this.branch_index.saturating_sub(1);
                this.branch_scroll
                    .scroll_to_item(this.branch_index, ScrollStrategy::Center);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BranchDown, _, cx| {
                this.branch_index =
                    (this.branch_index + 1).min(this.filtered_branches.len().saturating_sub(1));
                this.branch_scroll
                    .scroll_to_item(this.branch_index, ScrollStrategy::Center);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &BranchAccept, _, cx| {
                this.switch_branch(this.branch_index, cx)
            }))
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload(cx)))
            .on_action(cx.listener(|this, _: &HistoryPageUp, _, cx| {
                this.scroll.scroll_to_item(
                    this.viewport.start.saturating_sub(this.viewport.len()),
                    ScrollStrategy::Top,
                );
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &HistoryPageDown, _, cx| {
                let maximum = this
                    .snapshot
                    .as_ref()
                    .map_or(0, |s| s.visible.saturating_sub(1));
                this.scroll
                    .scroll_to_item(this.viewport.end.min(maximum), ScrollStrategy::Top);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.update(cx, |input, cx| input.focus(window, cx))
            }))
            .on_action(cx.listener(|this, _: &BlurFilter, window, cx| this.focus.focus(window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .text_size(px(self.preferences.font_size))
            .font_family(gpui_kit::SharedString::from(self.preferences.ui_font.clone()))
            .bg(rgb(palette.background))
            .text_color(rgb(palette.foreground))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(format!("Repository: {}", self.config.repository.display())),
                    )
                    .child(
                        Button::new("application-menu")
                            .label("Menu")
                            .dropdown_menu({
                                let focus = self.focus.clone();
                                let watch = self.config.watch;
                                let dark = self.config.theme == Theme::Dark;
                                move |menu, _, _| {
                                    menu.action_context(focus.clone())
                                        .menu("Open repository…", Box::new(OpenRepository))
                                        .menu("Switch local branch…", Box::new(OpenBranches))
                                        .menu("Reload", Box::new(Reload))
                                        .menu_with_check(
                                            "Automatic reload",
                                            watch,
                                            Box::new(ToggleWatch),
                                        )
                                        .separator()
                                        .menu("Copy commit reference", Box::new(CopyCommit))
                                        .menu("Font preferences…", Box::new(FontPreferences))
                                        .menu_with_check("Dark theme", dark, Box::new(ToggleTheme))
                                        .separator()
                                        .menu("Keyboard shortcuts", Box::new(Help))
                                        .menu("About", Box::new(About))
                                        .menu("Quit", Box::new(Quit))
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child("Filter:")
                    .child(div().flex_1().key_context("HistoryFilter").child(Input::new(&self.filter)))
                    .child(
                        div()
                            .id("clear-filter")
                            .px_3()
                            .py_2()
                            .bg(rgb(palette.panel))
                            .cursor_pointer()
                            .child("Clear")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.filter
                                    .update(cx, |input, cx| input.set_value("", window, cx))
                            })),
                    )
                    .child(
                        div()
                            .id("reload")
                            .test_support()
                            .px_3()
                            .py_2()
                            .bg(rgb(palette.panel))
                            .cursor_pointer()
                            .child(if self.config.watch {
                                "Reload (Auto On)"
                            } else {
                                "Reload (Auto Off)"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_auto_reload(cx))),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    v_resizable("main-split")
                        .child(
                            resizable_panel().size(px(265.)).child(
                                section()
                                    .size_full()
                                    // Keep the scrollbar above Kit's overlapping resize handle.
                                    .pb_1()
                                    .flex()
                                    .flex_col()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .flex()
                                            .h(px((self.preferences.font_size * 2.).max(26.)))
                                            .items_center()
                                            .bg(rgb(palette.border))
                                            .child(
                                                div()
                                                    .id("graph-heading")
                                                    .test_support()
                                                    .relative()
                                                    .w(px(self.graph_width))
                                                    .flex_shrink_0()
                                                    .px_2()
                                                    .child(if self.graph_layout.has_collapsed() { "Graph · … idle lanes" } else { "Graph" })
                                                    .when(self.graph_layout.has_collapsed(), |heading| {
                                                        heading.hoverable_tooltip(|window, cx| {
                                                            wrapped_tooltip(IDLE_LANES_TOOLTIP, window, cx)
                                                        })
                                                    })
                                                    .child(div().id("graph-divider").test_support()
                                                        .absolute().right_0().top_0().bottom_0().w(px(7.))
                                                        .cursor_col_resize()
                                                        .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                                                            this.graph_resize = Some((event.position.x, this.graph_width));
                                                            cx.stop_propagation();
                                                        }))),
                                            )
                                            .child(div().flex_1().px_2().child("Commit"))
                                            .child(
                                                div()
                                                    .w(px(280.))
                                                    .flex_shrink_0()
                                                    .px_2()
                                                    .child("Author"),
                                            )
                                            .child(
                                                div()
                                                    .w(px(145.))
                                                    .flex_shrink_0()
                                                    .px_2()
                                                    .child("Date"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .relative()
                                            .flex_1()
                                            .min_h_0()
                                            .child(
                                                uniform_list(
                                                    "history",
                                                    count,
                                                    cx.processor(
                                                        |this, range: Range<usize>, _, cx| {
                                                            range
                                                                .map(|index| this.row(index, cx))
                                                                .collect::<Vec<_>>()
                                                        },
                                                    ),
                                                )
                                                .with_width_from_item(Some(self.viewport.start))
                                                .with_decoration(HistoryViewport(cx.entity()))
                                                .track_scroll(&self.scroll)
                                                .size_full(),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top_0()
                                                    .left_0()
                                                    .size_full()
                                                    .id("history-scrollbar-viewport")
                                                    .test_support()
                                                    .child(
                                                        Scrollbar::vertical(&self.scroll)
                                                            .mode(ScrollbarMode::Always)
                                                            .id("history-scrollbar"),
                                                    ),
                                            ),
                                    ),
                            ),
                        )
                        .child(resizable_panel().child(self.diff.clone())),
                ),
            )
            .child(
                section()
                    .id("history-status")
                    .test_support()
                    .flex_shrink_0()
                    .truncate()
                    .px_2()
                    .py_1()
                    .child(status.clone())
                    .hoverable_tooltip(move |window, cx| {
                        wrapped_tooltip(status.clone(), window, cx)
                    }),
            )
            .when(self.dialog.is_some(), |root| {
                root.child(self.dialog_element(cx))
            })
            .when(timing.enabled(), |root| {
                root.child(
                    canvas(
                        |_, _, _| (),
                        move |_, (), _, _| {
                            timing.mark(Phase::WindowPainted);
                            if history_ready {
                                timing.mark(Phase::HistoryPainted);
                            }
                            if selection_ready {
                                timing.mark(Phase::SelectionPainted);
                            }
                            timing.frame(started);
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
            })
    }
}

fn bind_keys(cx: &mut App) {
    diff_view::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new(
            "ctrl-c",
            CopyCommit,
            Some("GitHistory && !Input && !Diff && !Dialog"),
        ),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("ctrl-,", FontPreferences, Some("GitHistory && !Dialog")),
        KeyBinding::new("ctrl-enter", ApplyPreferences, Some("Dialog")),
        KeyBinding::new(
            "alt-t",
            ToggleTheme,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new(
            "alt-r",
            ToggleWatch,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new(
            "ctrl-a",
            gpui_kit::component::input::MoveHome,
            Some("Input"),
        ),
        KeyBinding::new("ctrl-e", gpui_kit::component::input::MoveEnd, Some("Input")),
        KeyBinding::new(
            "ctrl-b",
            gpui_kit::component::input::MoveLeft,
            Some("Input"),
        ),
        KeyBinding::new(
            "ctrl-f",
            gpui_kit::component::input::MoveRight,
            Some("Input"),
        ),
        KeyBinding::new(
            "ctrl-w",
            gpui_kit::component::input::DeleteToPreviousWordStart,
            Some("Input"),
        ),
        KeyBinding::new(
            "ctrl-u",
            gpui_kit::component::input::DeleteToBeginningOfLine,
            Some("Input"),
        ),
        KeyBinding::new(
            "ctrl-k",
            gpui_kit::component::input::DeleteToEndOfLine,
            Some("Input"),
        ),
        KeyBinding::new("ctrl-d", gpui_kit::component::input::Delete, Some("Input")),
        KeyBinding::new("ctrl-o", OpenRepository, Some("GitHistory && !Dialog")),
        KeyBinding::new(
            "ctrl-shift-o",
            OpenRepositoryPath,
            Some("GitHistory && !Dialog"),
        ),
        KeyBinding::new(
            "ctrl-b",
            OpenBranches,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new("f1", Help, Some("GitHistory && !Dialog")),
        KeyBinding::new("?", Help, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("up", HelpScrollUp, Some("HelpDialog")),
        KeyBinding::new("down", HelpScrollDown, Some("HelpDialog")),
        KeyBinding::new("pageup", HelpPageUp, Some("HelpDialog")),
        KeyBinding::new("pagedown", HelpPageDown, Some("HelpDialog")),
        KeyBinding::new("home", HelpTop, Some("HelpDialog")),
        KeyBinding::new("end", HelpBottom, Some("HelpDialog")),
        KeyBinding::new("escape", CloseDialog, Some("Dialog")),
        KeyBinding::new("escape", CloseDialog, Some("Dialog > Input && !Select")),
        KeyBinding::new("escape", BlurFilter, Some("Input && !Dialog")),
        KeyBinding::new("up", BranchUp, Some("BranchDialog")),
        KeyBinding::new("down", BranchDown, Some("BranchDialog")),
        KeyBinding::new("down", BranchDown, Some("BranchDialog > Input")),
        KeyBinding::new("up", BranchUp, Some("BranchDialog > Input")),
        KeyBinding::new("alt-j", BranchDown, Some("BranchDialog > Input")),
        KeyBinding::new("alt-k", BranchUp, Some("BranchDialog > Input")),
        KeyBinding::new("alt-j", Next, Some("GitHistory && !Dialog")),
        KeyBinding::new("alt-k", Previous, Some("GitHistory && !Dialog")),
        KeyBinding::new("ctrl-n", BranchDown, Some("BranchDialog > Input")),
        KeyBinding::new("ctrl-p", BranchUp, Some("BranchDialog > Input")),
        KeyBinding::new("enter", BranchAccept, Some("BranchDialog")),
        KeyBinding::new("f5", Reload, None),
        KeyBinding::new(
            "ctrl-pageup",
            HistoryPageUp,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new(
            "ctrl-pagedown",
            HistoryPageDown,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new("/", FocusFilter, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("escape", BlurFilter, Some("GitHistory && !Dialog")),
        KeyBinding::new("down", Next, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("j", Next, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("n", Next, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("up", Previous, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("k", Previous, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("p", Previous, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("home", First, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("g g", First, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("end", Last, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("shift-g", Last, Some("GitHistory && !Input && !Dialog")),
    ]);
    // Keep Control bindings available on macOS, including Emacs input editing.
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new(
            "cmd-c",
            CopyCommit,
            Some("GitHistory && !Input && !Diff && !Dialog"),
        ),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", FontPreferences, Some("GitHistory && !Dialog")),
        KeyBinding::new("cmd-enter", ApplyPreferences, Some("Dialog")),
        KeyBinding::new("cmd-o", OpenRepository, Some("GitHistory && !Dialog")),
        KeyBinding::new(
            "cmd-shift-o",
            OpenRepositoryPath,
            Some("GitHistory && !Dialog"),
        ),
        KeyBinding::new(
            "cmd-b",
            OpenBranches,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new(
            "cmd-pageup",
            HistoryPageUp,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new(
            "cmd-pagedown",
            HistoryPageDown,
            Some("GitHistory && !Input && !Dialog"),
        ),
    ]);
}

pub fn run(config: Config, launched: Instant) {
    let preferences = Preferences::load().unwrap_or_else(|error| {
        eprintln!("gitk-rs: preferences: {error:#}");
        Preferences::default()
    });
    let guard = TimingGuard::start(config.timings, launched);
    let timing = guard.timing();
    let request = Request {
        generation: Generation(1),
        id: 0,
        refs_revision: 0,
        repository: config.repository.clone(),
        range: 0..128,
        query: String::new(),
        batch: config.batch,
        graph_columns: config.graph_columns,
        anchor: None,
        restore: None,
    };
    let engine = HistoryEngine::spawn(request.clone());
    let history = engine.handle();
    let workspace_engine = WorkspaceEngine::spawn(WorkspaceRequest {
        generation: WorkspaceGeneration(1),
        repository: config.repository.clone(),
        watch: config.watch,
    });
    let workspace = workspace_engine.handle();
    let diff_engine = DiffEngine::spawn();
    let diff_handle = diff_engine.handle();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            #[cfg(target_os = "macos")]
            if let Err(error) = crate::macos::set_application_icon() {
                eprintln!("gitk-rs: unable to set application icon: {error:#}");
            }
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(
                match config.theme {
                    Theme::Dark => ThemeMode::Dark,
                    Theme::Light => ThemeMode::Light,
                },
                None,
                cx,
            );
            bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    // Native X11 close still holds its client borrow during App::defer.
                    cx.spawn(async |cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(1))
                            .await;
                        cx.update(|cx| {
                            if cx.windows().is_empty() {
                                cx.quit();
                            }
                        });
                    })
                    .detach();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1280.), px(850.)), cx);
            if let Err(error) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    window.set_window_title("gitk-rs");
                    window.set_app_id("gitk-rs");
                    timing.mark(Phase::WindowCreated);
                    cx.new(|cx| {
                        HistoryWindow::new(
                            config,
                            timing,
                            WindowServices {
                                history,
                                workspace,
                                diff: diff_handle,
                            },
                            preferences,
                            request,
                            window,
                            cx,
                        )
                    })
                },
            ) {
                eprintln!("gitk-rs: unable to open window: {error}");
                cx.quit();
            }
            cx.activate(true);
        });
    drop(engine);
    drop(diff_engine);
    drop(workspace_engine);
}

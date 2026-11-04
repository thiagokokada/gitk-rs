mod diff_view;

use gitk_rs::{
    cli::{Config, Theme},
    diff::{DiffEngine, DiffHandle, DiffSettings},
    graph::GraphRow,
    history::{Entry, Generation, HistoryEngine, HistoryHandle, RefKind, Request, Snapshot},
    timing::{Phase, Timing, TimingGuard},
};
use gpui::{
    AnyElement, App, Application, Bounds, Context, Entity, FocusHandle, KeyBinding, PathBuilder,
    ScrollStrategy, SharedString, Subscription, Task, UniformListScrollHandle, Window,
    WindowBounds, WindowOptions, actions, canvas, div, fill, point, prelude::*, px, rgb, size,
    uniform_list,
};
use gpui_component::{
    Root, ThemeMode,
    input::{Input, InputEvent, InputState},
    scroll::Scrollbar,
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
        HistoryPageDown
    ]
);

struct HistoryWindow {
    config: Config,
    focus: FocusHandle,
    timing: Timing,
    history: HistoryHandle,
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
}

impl HistoryWindow {
    fn new(
        config: Config,
        timing: Timing,
        history: HistoryHandle,
        diff_handle: DiffHandle,
        request: Request,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
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
                        this.preserve_filter_selection = true;
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
                    .update(cx, |this, cx| this.consume_snapshot(cx))
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
        Self {
            diff,
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
        if snapshot.eof && snapshot.visible == 0
            || snapshot.error.is_some() && self.selected.is_none()
        {
            self.diff
                .update(cx, |diff, cx| diff.clear(&self.detail, cx));
        }
        self.snapshot = Some(snapshot);
        if self.preserve_filter_selection {
            let snapshot = self.snapshot.as_ref().unwrap();
            if let Some(index) = snapshot.anchor_position
                && let Some((_, entry)) = &self.selected
            {
                self.set_selection(index, entry.clone(), cx);
                self.scroll.scroll_to_item(index, ScrollStrategy::Center);
                self.preserve_filter_selection = false;
            } else if !snapshot.filtering && snapshot.visible > 0 {
                self.selected = None;
                self.pending_selection = Some(0);
                self.preserve_filter_selection = false;
            } else if snapshot.eof && snapshot.visible == 0 {
                self.selected = None;
                self.preserve_filter_selection = false;
            }
        }
        if let Some(index) = self.pending_selection
            && let Some(entry) = self.cached(index)
        {
            self.pending_selection = None;
            self.set_selection(index, entry, cx);
        }
        cx.notify();
    }

    fn cached(&self, index: usize) -> Option<Arc<Entry>> {
        let snapshot = self.snapshot.as_ref()?;
        snapshot
            .rows
            .get(index.checked_sub(snapshot.start)?)
            .cloned()
    }

    fn ensure_rows(&mut self, range: Range<usize>) {
        self.viewport = range.clone();
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
        self.diff.update(cx, |diff, cx| {
            diff.load(
                self.config.repository.clone(),
                Arc::new(entry.commit.clone()),
                self.request.generation.0,
                cx,
            )
        });
        self.selected = Some((index, entry));
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        if let Some(entry) = self.cached(index) {
            self.set_selection(index, entry, cx);
        } else {
            self.pending_selection = Some(index);
            self.ensure_rows(index..index + 1);
        }
        self.scroll.scroll_to_item(index, ScrollStrategy::Center);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let maximum = snapshot.visible.saturating_sub(usize::from(snapshot.eof));
        let index = self
            .pending_selection
            .or_else(|| self.selected.as_ref().map(|s| s.0))
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(maximum);
        self.select(index, window, cx);
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.request.generation.0 += 1;
        self.request.range = 0..128;
        self.snapshot = None;
        self.selected = None;
        self.pending_selection = Some(0);
        self.preserve_filter_selection = false;
        self.detail = "Loading commits…".into();
        self.diff
            .update(cx, |diff, cx| diff.clear("Loading commits…", cx));
        self.submit();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(entry) = self.cached(index) else {
            return div()
                .id(("loading", index))
                .h(px(26.))
                .px_2()
                .child("Loading more commits…")
                .into_any_element();
        };
        let is_selected = self
            .selected
            .as_ref()
            .is_some_and(|s| s.1.commit.id == entry.commit.id);
        let background = if is_selected {
            0x285b8e
        } else if self.config.theme == Theme::Dark {
            0x252930
        } else {
            0xffffff
        };
        let labels = self
            .snapshot
            .as_ref()
            .and_then(|s| s.refs.get(&entry.commit.id))
            .cloned()
            .unwrap_or_default();
        let width = entry.graph.width as f32 * 12. + 16.;
        let graph: AnyElement = if self.config.text_graph {
            let text: String = (0..entry.graph.width)
                .map(|lane| {
                    if Some(lane) == entry.graph.node {
                        "● "
                    } else {
                        "│ "
                    }
                })
                .collect();
            div()
                .font_family(fixed_font())
                .child(text)
                .into_any_element()
        } else {
            let graph = entry.graph.clone();
            let filtered = !self.request.query.trim().is_empty();
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| paint_graph(&graph, filtered, bounds, window),
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
                    RefKind::Head => 0x735a12,
                    RefKind::Branch => 0x24643a,
                    RefKind::Remote => 0x2e527c,
                    RefKind::Tag => 0x654b7e,
                };
                div()
                    .px_1()
                    .text_size(px(11.))
                    .bg(rgb(color))
                    .text_color(rgb(0xffffff))
                    .child(label.name)
            }));
        div()
            .id(("commit", index))
            .h(px(26.))
            .w_full()
            .flex()
            .items_center()
            .bg(rgb(background))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
            .child(
                div()
                    .relative()
                    .w(px(280.))
                    .flex_shrink_0()
                    .h_full()
                    .overflow_hidden()
                    .child(graph)
                    .child(badges),
            )
            .child(div().flex_1().min_w_0().truncate().px_2().child(format!(
                "{}  {}",
                &entry.commit.id.as_str()[..7],
                entry.commit.summary()
            )))
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
            .into_any_element()
    }
}

pub(crate) fn fixed_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

fn paint_graph(
    graph: &GraphRow,
    filtered: bool,
    bounds: Bounds<gpui::Pixels>,
    window: &mut Window,
) {
    let colors = [0x80bfff, 0x83d49b, 0xe5b978, 0xd49fe8, 0x73d5d1, 0xef9090];
    let x = |lane: u16| bounds.left() + px(9. + lane as f32 * 12.);
    let middle = bounds.top() + bounds.size.height / 2.;
    if !filtered {
        for edge in &graph.edges {
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(point(
                x(edge.from),
                if edge.parent { middle } else { bounds.top() },
            ));
            path.line_to(point(x(edge.to), bounds.bottom()));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(colors[edge.from as usize % colors.len()]));
            }
        }
        if graph.incoming
            && let Some(node) = graph.node
        {
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(point(x(node), bounds.top()));
            path.line_to(point(x(node), middle));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(colors[node as usize % colors.len()]));
            }
        }
    }
    if let Some(node) = graph.node {
        let mut quad = fill(
            Bounds::new(
                point(x(node) - px(3.5), middle - px(3.5)),
                size(px(7.), px(7.)),
            ),
            rgb(colors[node as usize % colors.len()]),
        );
        quad.corner_radii = px(3.5).into();
        window.paint_quad(quad);
    }
    if graph.overflow {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.right() - px(6.), middle - px(2.)),
                size(px(5.), px(4.)),
            ),
            rgb(0xe5b978),
        ));
    }
}

impl Render for HistoryWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        let timing = self.timing.clone();
        let history_ready = self.snapshot.as_ref().is_some_and(|s| !s.rows.is_empty());
        let selection_ready = self.selected.is_some();
        let (background, foreground, panel, border) = match self.config.theme {
            Theme::Dark => (0x202329, 0xe0e4eb, 0x292d35, 0x454b57),
            Theme::Light => (0xf5f6f8, 0x202329, 0xffffff, 0xc4c9d2),
        };
        let section = || div().bg(rgb(panel)).border_1().border_color(rgb(border));
        let status = self.snapshot.as_ref().map_or_else(
            || "Loading commits…".into(),
            |s| {
                if let Some(error) = &s.error {
                    return error.clone();
                }
                format!(
                    "Showing {} / {} loaded commits on {}{}{}",
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
        let count = self
            .snapshot
            .as_ref()
            .map_or(0, |s| s.visible + usize::from(!s.eof && s.error.is_none()));
        div()
            .relative()
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
                    this.select(s.visible.saturating_sub(1), window, cx);
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
            .on_action(cx.listener(|this, _: &BlurFilter, window, _| this.focus.focus(window)))
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .text_size(px(13.))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .child(
                div()
                    .truncate()
                    .child(format!("Repository: {}", self.config.repository.display())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child("Filter:")
                    .child(div().flex_1().child(Input::new(&self.filter)))
                    .child(
                        div()
                            .id("clear-filter")
                            .px_3()
                            .py_2()
                            .bg(rgb(panel))
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
                            .px_3()
                            .py_2()
                            .bg(rgb(panel))
                            .cursor_pointer()
                            .child("Reload")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            )
            .child(
                section()
                    .h(px(265.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .h(px(26.))
                            .items_center()
                            .bg(rgb(border))
                            .child(div().w(px(280.)).flex_shrink_0().px_2().child("Graph"))
                            .child(div().flex_1().px_2().child("Commit"))
                            .child(div().w(px(280.)).flex_shrink_0().px_2().child("Author"))
                            .child(div().w(px(145.)).flex_shrink_0().px_2().child("Date")),
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
                                    cx.processor(|this, range: Range<usize>, _, cx| {
                                        this.ensure_rows(range.clone());
                                        range.map(|index| this.row(index, cx)).collect::<Vec<_>>()
                                    }),
                                )
                                .track_scroll(self.scroll.clone())
                                .size_full(),
                            )
                            .child(Scrollbar::vertical(&self.scroll).id("history-scrollbar")),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.diff.clone()))
            .child(section().px_2().py_1().child(status))
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

pub fn run(config: Config, launched: Instant) {
    let guard = TimingGuard::start(config.timings, launched);
    let timing = guard.timing();
    let request = Request {
        generation: Generation(1),
        id: 0,
        repository: config.repository.clone(),
        range: 0..128,
        query: String::new(),
        batch: config.batch,
        graph_columns: config.graph_columns,
        anchor: None,
    };
    let engine = HistoryEngine::spawn(request.clone());
    let history = engine.handle();
    let diff_engine = DiffEngine::spawn();
    let diff_handle = diff_engine.handle();
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        diff_view::bind_keys(cx);
        gpui_component::Theme::change(
            match config.theme {
                Theme::Dark => ThemeMode::Dark,
                Theme::Light => ThemeMode::Light,
            },
            None,
            cx,
        );
        cx.bind_keys([
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("f5", Reload, None),
            KeyBinding::new("ctrl-pageup", HistoryPageUp, Some("GitHistory && !Input")),
            KeyBinding::new(
                "ctrl-pagedown",
                HistoryPageDown,
                Some("GitHistory && !Input"),
            ),
            KeyBinding::new("/", FocusFilter, Some("GitHistory && !Input")),
            KeyBinding::new("escape", BlurFilter, Some("GitHistory")),
            KeyBinding::new("down", Next, Some("GitHistory && !Input")),
            KeyBinding::new("j", Next, Some("GitHistory && !Input")),
            KeyBinding::new("n", Next, Some("GitHistory && !Input")),
            KeyBinding::new("up", Previous, Some("GitHistory && !Input")),
            KeyBinding::new("k", Previous, Some("GitHistory && !Input")),
            KeyBinding::new("p", Previous, Some("GitHistory && !Input")),
            KeyBinding::new("home", First, Some("GitHistory && !Input")),
            KeyBinding::new("end", Last, Some("GitHistory && !Input")),
        ]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                // Native X11 close still holds its client borrow during App::defer.
                cx.spawn(async |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                    let _ = cx.update(|cx| {
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
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("gitk-rs");
                timing.mark(Phase::WindowCreated);
                let view = cx.new(|cx| {
                    HistoryWindow::new(config, timing, history, diff_handle, request, window, cx)
                });
                cx.new(|cx| Root::new(view, window, cx))
            },
        ) {
            eprintln!("gitk-rs: unable to open window: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
    drop(engine);
    drop(diff_engine);
}

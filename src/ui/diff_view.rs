#[cfg(test)]
mod tests;

use super::fixed_font;
use gitk_rs::{
    diff::{
        CopyRequest, DiffHandle, DiffPage, DiffSelection, DiffSettings, DiffSnapshot, DiffSource,
        LineKind, SelectionId, TextPoint, TextSelection, selection::DisplayText,
    },
    git::Commit,
};
use gpui::{prelude::*, *};
use gpui_component::{
    menu::{ContextMenuExt, PopupMenuItem},
    resizable::{h_resizable, resizable_panel},
    scroll::Scrollbar,
};
use std::{
    cell::RefCell, collections::HashMap, ops::Range, path::PathBuf, rc::Rc, sync::Arc,
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    diff,
    [
        Copy, CopyCode, SelectAll, PageDown, PageUp, LinesDown, LinesUp
    ]
);

#[derive(Clone)]
struct PaintedLine {
    bounds: Bounds<Pixels>,
    shaped: ShapedLine,
    display: DisplayText,
}

pub(super) struct DiffPane {
    handle: DiffHandle,
    focus: FocusHandle,
    settings: DiffSettings,
    font: SharedString,
    font_size: f32,
    identity: Option<(String, u64)>,
    page: DiffPage,
    snapshot: Option<DiffSnapshot>,
    message: String,
    scroll: UniformListScrollHandle,
    files_scroll: UniformListScrollHandle,
    selection: Option<TextSelection>,
    dragging: bool,
    viewport: Range<usize>,
    layouts: Rc<RefCell<HashMap<usize, PaintedLine>>>,
    _poll: Task<()>,
}

impl DiffPane {
    pub(super) fn new(handle: DiffHandle, settings: DiffSettings, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if let Some(snapshot) = this.handle.take_snapshot()
                            && snapshot.selection == this.page.selection
                            && snapshot.request == this.page.id
                        {
                            this.message = snapshot
                                .error
                                .clone()
                                .or_else(|| snapshot.notice.clone())
                                .unwrap_or_default();
                            this.snapshot = Some(snapshot);
                            cx.notify();
                        }
                        if let Some(copy) = this.handle.take_copy()
                            && copy.selection == this.page.selection
                        {
                            match copy.result {
                                Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                                Err(error) => {
                                    this.message = error;
                                    cx.notify();
                                }
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            handle,
            focus: cx.focus_handle(),
            settings,
            font: fixed_font().into(),
            font_size: 13.,
            identity: None,
            page: DiffPage {
                selection: SelectionId(0),
                id: 0,
                range: 0..128,
                settings,
            },
            snapshot: None,
            message: "Select a commit".into(),
            scroll: UniformListScrollHandle::new(),
            files_scroll: UniformListScrollHandle::new(),
            selection: None,
            dragging: false,
            viewport: 0..1,
            layouts: Rc::new(RefCell::new(HashMap::new())),
            _poll: poll,
        }
    }

    pub(super) fn set_font(&mut self, font: SharedString, font_size: f32, cx: &mut Context<Self>) {
        self.font = font;
        self.font_size = font_size;
        cx.notify();
    }
    pub(super) fn set_theme(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.settings.dark = dark;
        self.page.settings = self.settings;
        self.page.id += 1;
        if let Some(snapshot) = &mut self.snapshot {
            for line in &mut snapshot.lines {
                line.spans.clear();
            }
        }
        self.handle.page(self.page.clone());
        cx.notify();
    }

    pub(super) fn load(
        &mut self,
        repository: PathBuf,
        commit: Arc<Commit>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let identity = (commit.id.as_str().to_owned(), generation);
        if self.identity.as_ref() == Some(&identity) {
            return;
        }
        self.clear("Loading diff…", cx);
        self.identity = Some(identity);
        self.handle.select(
            DiffSelection {
                id: self.page.selection,
                repository,
                source: DiffSource::Commit(commit),
            },
            self.page.clone(),
        );
    }

    pub(super) fn load_local(
        &mut self,
        repository: PathBuf,
        staged: bool,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        let identity = (
            if staged { "staged" } else { "unstaged" }.to_owned(),
            revision,
        );
        if self.identity.as_ref() == Some(&identity) {
            return;
        }
        self.clear("Loading local changes…", cx);
        self.identity = Some(identity);
        self.handle.select(
            DiffSelection {
                id: self.page.selection,
                repository,
                source: DiffSource::Local { staged },
            },
            self.page.clone(),
        );
    }

    pub(super) fn clear(&mut self, message: &str, cx: &mut Context<Self>) {
        self.page.selection.0 += 1;
        self.page.id += 1;
        self.page.range = 0..128;
        self.handle.clear(self.page.selection);
        self.identity = None;
        self.snapshot = None;
        self.message = message.into();
        self.selection = None;
        self.dragging = false;
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.files_scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn ensure(&mut self, range: Range<usize>) {
        self.viewport = range.clone();
        let covered = self
            .snapshot
            .as_ref()
            .is_some_and(|s| range.start >= s.start && range.end <= s.start + s.lines.len());
        if covered || range.is_empty() {
            return;
        }
        let start = (range.start / 64).saturating_sub(1) * 64;
        let range = start..start + 256;
        if range != self.page.range {
            self.page.range = range;
            self.page.id += 1;
            self.handle.page(self.page.clone());
        }
    }

    fn copy(&self, strip_markers: bool) {
        if let Some(range) = self.selection {
            self.handle.copy(CopyRequest {
                selection: self.page.selection,
                range,
                strip_markers,
            });
        }
    }

    pub(super) fn scroll_page(&mut self, down: bool, cx: &mut Context<Self>) {
        let delta = self.viewport.len().saturating_sub(2) as isize;
        self.scroll_lines(if down { delta } else { -delta }, cx);
    }

    pub(super) fn scroll_lines(&mut self, delta: isize, cx: &mut Context<Self>) {
        let max = self
            .snapshot
            .as_ref()
            .map_or(0, |s| s.count.saturating_sub(1));
        self.scroll.scroll_to_item_strict(
            self.viewport.start.saturating_add_signed(delta).min(max),
            ScrollStrategy::Top,
        );
        cx.notify();
    }

    fn jump(&mut self, line: usize, cx: &mut Context<Self>) {
        self.scroll.scroll_to_item_strict(line, ScrollStrategy::Top);
        self.ensure(line..line + 1);
        cx.notify();
    }

    fn hit(&self, line: usize, position: Point<Pixels>) -> TextPoint {
        let byte = self.layouts.borrow().get(&line).map_or(0, |layout| {
            layout.display.source_index(
                layout
                    .shaped
                    .closest_index_for_x(position.x - layout.bounds.left()),
            )
        });
        TextPoint { line, byte }
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(line) = self
            .snapshot
            .as_ref()
            .and_then(|s| index.checked_sub(s.start).and_then(|i| s.lines.get(i)))
        else {
            return div()
                .h(px(self.font_size * 1.55))
                .child("Loading…")
                .into_any_element();
        };
        let dark = self.settings.dark;
        let background = match (dark, line.kind) {
            (true, LineKind::Added) => 0x184b32,
            (true, LineKind::Removed) => 0x5f2639,
            (true, LineKind::Hunk) => 0x26384c,
            (true, LineKind::Header) => 0x343a44,
            (false, LineKind::Added) => 0xd9f5e0,
            (false, LineKind::Removed) => 0xffdddd,
            (false, LineKind::Hunk) => 0xddeafa,
            (false, LineKind::Header) => 0xe6e9ed,
            (true, _) => 0x252930,
            (false, _) => 0xffffff,
        };
        let display = DisplayText::new(&line.text);
        let spans = line.spans.clone();
        let selection = self
            .selection
            .and_then(|selection| selection.on_line(index, line.text.len()));
        let layouts = self.layouts.clone();
        let font_size = self.font_size;
        let line_height = px(font_size * 1.55);
        let width =
            self.snapshot.as_ref().map_or(100, |s| s.max_width).max(100) as f32 * font_size * 0.65
                + 24.;
        div()
            .id(("diff-line", index))
            .h(px(self.font_size * 1.55))
            .w(px(width))
            .min_w_full()
            .bg(rgb(background))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window);
                    let point = this.hit(index, event.position);
                    if event.click_count >= 3 {
                        let len = this
                            .layouts
                            .borrow()
                            .get(&index)
                            .map_or(0, |l| l.display.source_index(l.display.text.len()));
                        this.selection = Some(TextSelection {
                            anchor: TextPoint {
                                line: index,
                                byte: 0,
                            },
                            cursor: TextPoint {
                                line: index,
                                byte: len,
                            },
                        });
                    } else if event.click_count == 2 {
                        if let Some(layout) = this.layouts.borrow().get(&index) {
                            let at = layout.display.display_index(point.byte);
                            let word = layout
                                .display
                                .text
                                .split_word_bound_indices()
                                .find(|(start, word)| *start <= at && at < start + word.len());
                            if let Some((start, word)) = word {
                                this.selection = Some(TextSelection {
                                    anchor: TextPoint {
                                        line: index,
                                        byte: layout.display.source_index(start),
                                    },
                                    cursor: TextPoint {
                                        line: index,
                                        byte: layout.display.source_index(start + word.len()),
                                    },
                                });
                            }
                        }
                    } else if event.modifiers.shift
                        && let Some(selection) = &mut this.selection
                    {
                        selection.cursor = point;
                    } else {
                        this.selection = Some(TextSelection {
                            anchor: point,
                            cursor: point,
                        });
                    }
                    this.dragging = event.click_count == 1;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if this.dragging && event.pressed_button == Some(MouseButton::Left) {
                    let point = this.hit(index, event.position);
                    if let Some(selection) = &mut this.selection {
                        selection.cursor = point;
                    }
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let mut runs = Vec::new();
                        let mut offset = 0;
                        let font = window.text_style().font();
                        let default = if dark { 0xe0e4eb } else { 0x202329 };
                        let mut push = |start: usize, end: usize, color| {
                            if end > start {
                                runs.push(TextRun {
                                    len: end - start,
                                    font: font.clone(),
                                    color: rgb(color).into(),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                });
                            }
                        };
                        for span in &spans {
                            let start = display.display_index(span.range.start);
                            let end = display.display_index(span.range.end);
                            push(offset, start, default);
                            push(start, end, span.color);
                            offset = end;
                        }
                        push(offset, display.text.len(), default);
                        let shaped = window.text_system().shape_line(
                            display.text.clone().into(),
                            px(font_size),
                            &runs,
                            None,
                        );
                        let layout = PaintedLine {
                            bounds,
                            shaped,
                            display: display.clone(),
                        };
                        layouts.borrow_mut().insert(index, layout.clone());
                        layout
                    },
                    move |bounds, layout, window, cx| {
                        if let Some(selection) = &selection {
                            let start = layout
                                .shaped
                                .x_for_index(layout.display.display_index(selection.start));
                            let end = layout
                                .shaped
                                .x_for_index(layout.display.display_index(selection.end));
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.left() + start, bounds.top()),
                                    size(end - start, line_height),
                                ),
                                rgba(0x4285f4aa),
                            ));
                        }
                        let _ = layout.shaped.paint(bounds.origin, line_height, window, cx);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

impl Render for DiffPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.layouts.borrow_mut().clear();
        let count = self.snapshot.as_ref().map_or(0, |s| s.count);
        let files = self
            .snapshot
            .as_ref()
            .map(|s| s.files.clone())
            .unwrap_or_default();
        let border = if self.settings.dark {
            0x454b57
        } else {
            0xc4c9d2
        };
        div()
            .size_full()
            .flex()
            .gap_2()
            .track_focus(&self.focus)
            .key_context("Diff")
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_action(cx.listener(|this, _: &Copy, _, _| this.copy(false)))
            .on_action(cx.listener(|this, _: &CopyCode, _, _| this.copy(true)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| {
                if let Some(s) = &this.snapshot {
                    this.selection = Some(TextSelection {
                        anchor: TextPoint::default(),
                        cursor: TextPoint {
                            line: s.count.saturating_sub(1),
                            byte: s.last_line_len,
                        },
                    });
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &PageDown, _, cx| {
                this.scroll_lines(this.viewport.len().saturating_sub(2) as isize, cx)
            }))
            .on_action(cx.listener(|this, _: &PageUp, _, cx| {
                this.scroll_lines(-(this.viewport.len().saturating_sub(2) as isize), cx)
            }))
            .on_action(cx.listener(|this, _: &LinesDown, _, cx| this.scroll_lines(18, cx)))
            .on_action(cx.listener(|this, _: &LinesUp, _, cx| this.scroll_lines(-18, cx)))
            .child(
                h_resizable("diff-split")
                    .child(
                        resizable_panel().child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .flex()
                                .flex_col()
                                .border_1()
                                .border_color(rgb(border))
                                .font_family(self.font.clone())
                                .child(
                                    div()
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_hidden()
                                        .child(
                                            uniform_list(
                                                "diff-lines",
                                                count,
                                                cx.processor(|this, range: Range<usize>, _, cx| {
                                                    this.ensure(range.clone());
                                                    range
                                                        .map(|i| this.row(i, cx))
                                                        .collect::<Vec<_>>()
                                                }),
                                            )
                                            .track_scroll(self.scroll.clone())
                                            .with_horizontal_sizing_behavior(
                                                ListHorizontalSizingBehavior::Unconstrained,
                                            )
                                            .size_full(),
                                        )
                                        .child(Scrollbar::new(&self.scroll).id("diff-scrollbar")),
                                )
                                .when(!self.message.is_empty(), |pane| {
                                    pane.child(div().px_2().py_1().child(self.message.clone()))
                                }),
                        ),
                    )
                    .child(
                        resizable_panel().size(px(280.)).child(
                            div()
                                .w_full()
                                .h_full()
                                .flex_shrink_0()
                                .border_1()
                                .border_color(rgb(border))
                                .relative()
                                .child(
                                    uniform_list(
                                        "diff-files",
                                        files.len() + 1,
                                        cx.processor(move |this, range: Range<usize>, _, cx| {
                                            range
                                                .map(|i| {
                                                    let (label, line) = if i == 0 {
                                                        ("Commit".into(), Some(0))
                                                    } else {
                                                        let file = &files[i - 1];
                                                        let stats = match (file.added, file.removed)
                                                        {
                                                            (Some(a), Some(r)) => {
                                                                format!("+{a} -{r}")
                                                            }
                                                            _ => "binary".into(),
                                                        };
                                                        let path = file
                                                            .path
                                                            .to_string_lossy()
                                                            .escape_debug()
                                                            .to_string();
                                                        (
                                                            format!("{path}  {stats}"),
                                                            file.start_line,
                                                        )
                                                    };
                                                    let path = files
                                                        .get(i.saturating_sub(1))
                                                        .filter(|_| i > 0)
                                                        .map(|f| {
                                                            f.path.to_string_lossy().into_owned()
                                                        });
                                                    div()
                                                        .id(("diff-file", i))
                                                        .debug_selector(move || {
                                                            format!("diff-file-{i}")
                                                        })
                                                        .h(px((this.font_size * 1.9).max(24.)))
                                                        .px_2()
                                                        .truncate()
                                                        .cursor_pointer()
                                                        .child(label)
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                if let Some(line) = line {
                                                                    this.jump(line, cx);
                                                                }
                                                            },
                                                        ))
                                                        .context_menu(move |menu, _, _| {
                                                            if let Some(path) = path.clone() {
                                                                menu.item(
                                                                    PopupMenuItem::new(
                                                                        "Copy file path",
                                                                    )
                                                                    .on_click(move |_, _, cx| {
                                                                        cx.write_to_clipboard(
                                                                    ClipboardItem::new_string(
                                                                        path.clone(),
                                                                    ),
                                                                )
                                                                    }),
                                                                )
                                                            } else {
                                                                menu
                                                            }
                                                        })
                                                })
                                                .collect::<Vec<_>>()
                                        }),
                                    )
                                    .track_scroll(self.files_scroll.clone())
                                    .size_full(),
                                )
                                .child(
                                    Scrollbar::vertical(&self.files_scroll).id("files-scrollbar"),
                                ),
                        ),
                    ),
            )
            .context_menu({
                let focus = self.focus.clone();
                move |menu, _, _| {
                    menu.action_context(focus.clone())
                        .menu("Copy selection", Box::new(Copy))
                        .menu("Copy without diff markers", Box::new(CopyCode))
                        .menu("Select all", Box::new(SelectAll))
                }
            })
    }
}

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-c", Copy, Some("Diff")),
        KeyBinding::new("ctrl-shift-c", CopyCode, Some("Diff")),
        KeyBinding::new("ctrl-a", SelectAll, Some("Diff")),
        KeyBinding::new("space", PageDown, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("backspace", PageUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("delete", PageUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("shift-b", PageUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("shift-u", LinesUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new(
            "shift-d",
            LinesDown,
            Some("GitHistory && !Input && !Dialog"),
        ),
        KeyBinding::new("b", PageUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("u", LinesUp, Some("GitHistory && !Input && !Dialog")),
        KeyBinding::new("d", LinesDown, Some("GitHistory && !Input && !Dialog")),
    ]);
}

use super::*;
use gitk_rs::search::{QualifierCompletion, complete_qualifier};
use gpui_kit::component::input::{Escape, IndentInline, MoveDown, MoveUp};
use gpui_kit::{EntityInputHandler as _, MouseButton, deferred};

const COMPLETION_MENU_WIDTH: f32 = 420.;

impl HistoryWindow {
    fn filter_completion(&self, window: &Window, cx: &App) -> Option<QualifierCompletion> {
        let input = self.filter.read(cx);
        if self.dialog.is_some()
            || self.filter_completion_dismissed
            || !input.focus_handle(cx).is_focused(window)
            || !input.selected_range().is_empty()
        {
            return None;
        }
        complete_qualifier(input.value().as_ref(), input.cursor())
    }

    pub(super) fn complete_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(completion) = self.filter_completion(window, cx) else {
            return;
        };
        let index = self
            .filter_completion_index
            .min(completion.candidates.len() - 1);
        let text = format!("{}:", completion.candidates[index]);
        self.filter.update(cx, |input, cx| {
            let value = input.value();
            let start = value[..completion.range.start].encode_utf16().count();
            let end = value[..completion.range.end].encode_utf16().count();
            input.replace_text_in_range(Some(start..end), &text, window, cx);
            input.focus(window, cx);
        });
        self.filter_completion_index = 0;
        cx.notify();
    }

    pub(super) fn move_filter_completion(
        &mut self,
        delta: isize,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(completion) = self.filter_completion(window, cx) {
            let count = completion.candidates.len();
            self.filter_completion_index = (self.filter_completion_index.min(count - 1) as isize
                + delta)
                .rem_euclid(count as isize) as usize;
            cx.notify();
        }
    }

    pub(super) fn filter_input(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let completion = self.filter_completion(window, cx);
        let palette = Palette::for_theme(self.config.theme);
        div()
            .flex_1()
            .min_w_0()
            .relative()
            .key_context("HistoryFilter")
            // Input owns these bindings. Capture its actions before its editing
            // handlers, and leave normal input behavior alone without suggestions.
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.filter_completion(window, cx).is_some() {
                    this.complete_filter(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.filter_completion(window, cx).is_some() {
                    this.move_filter_completion(1, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.filter_completion(window, cx).is_some() {
                    this.move_filter_completion(-1, window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.dismiss_filter_completion(window, cx)
            }))
            .capture_action(cx.listener(|this, _: &BlurFilter, window, cx| {
                this.dismiss_filter_completion(window, cx)
            }))
            .child(Input::new(&self.filter).cleanable(true))
            .when_some(completion, |container, completion| {
                let selected = self
                    .filter_completion_index
                    .min(completion.candidates.len() - 1);
                container.child(
                    deferred(
                        div()
                            .absolute()
                            .top_full()
                            .left_0()
                            .w(px(COMPLETION_MENU_WIDTH))
                            .max_w_full()
                            .id("filter-completions")
                            .test_support()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .rounded_md()
                            .shadow_md()
                            .p_1()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .text_size(px(12.))
                                    .child("Filter tags · ↑/↓ to choose · Tab to complete"),
                            )
                            .children(completion.candidates.into_iter().enumerate().map(
                                |(index, qualifier)| {
                                    div()
                                        .id(SharedString::from(format!(
                                            "complete-{}",
                                            qualifier.name()
                                        )))
                                        .test_support()
                                        .px_2()
                                        .py_1()
                                        .cursor_pointer()
                                        .bg(rgb(if index == selected {
                                            palette.selection
                                        } else {
                                            palette.panel
                                        }))
                                        .child(format!("{qualifier}:"))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .child(qualifier.help_description()),
                                        )
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, window, cx| {
                                                window.prevent_default();
                                                this.filter_completion_index = index;
                                                this.complete_filter(window, cx);
                                            }),
                                        )
                                },
                            )),
                    )
                    .with_priority(1),
                )
            })
            .into_any_element()
    }

    fn dismiss_filter_completion(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.filter_completion(window, cx).is_some() {
            self.filter_completion_dismissed = true;
            cx.stop_propagation();
            cx.notify();
        }
    }
}

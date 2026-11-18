use super::*;
use gitk_rs::git::{FileHistory, HistoryScope};

pub(super) struct FileHistoryReturn {
    pub(super) scope: HistoryScope,
    pub(super) diff_mode: Option<gitk_rs::diff::ComparisonMode>,
    pub(super) local_selection: Option<LocalChangeKind>,
}

impl HistoryWindow {
    pub(super) fn open_file_history(
        &mut self,
        request: &diff_view::OpenFileHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if request.repository != self.config.repository
            || request.selection != self.diff.read(cx).selection_id()
        {
            return;
        }
        if !matches!(
            self.history_state.request.source.scope,
            HistoryScope::File(_)
        ) {
            self.file_history_return = Some(FileHistoryReturn {
                scope: self.history_state.request.source.scope.clone(),
                diff_mode: self.comparison_diff,
                local_selection: self.history_state.local_selection,
            });
        }
        self.set_history_scope(
            HistoryScope::File(FileHistory {
                path: request.path.clone(),
                revision: request.revision.clone(),
                follow_renames: self.follow_file_renames,
            }),
            window,
            cx,
        );
    }

    pub(super) fn file_history_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let HistoryScope::File(history) = &self.history_state.request.source.scope else {
            return div().into_any_element();
        };
        let label = self.history_state.request.source.scope.label();
        div()
            .id("file-history-controls")
            .test_support()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("file-history-label")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(label.clone())
                    .hoverable_tooltip(move |window, cx| {
                        wrapped_tooltip(label.clone(), window, cx)
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(
                        Checkbox::new("follow-file-renames")
                            .label("Follow renames")
                            .tooltip("Follow Git-detected renames into this file's earlier names.")
                            .checked(history.follow_renames)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                if let HistoryScope::File(history) =
                                    &mut this.history_state.request.source.scope
                                {
                                    history.follow_renames = *checked;
                                    this.follow_file_renames = *checked;
                                    this.layout_handle.save_follow_renames(*checked);
                                    this.reload(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("exit-file-history")
                            .label("Exit file history")
                            .tooltip("Return to the previous history scope.")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let previous =
                                    this.file_history_return
                                        .take()
                                        .unwrap_or(FileHistoryReturn {
                                            scope: HistoryScope::Current,
                                            diff_mode: None,
                                            local_selection: None,
                                        });
                                this.set_history_scope(previous.scope, window, cx);
                                this.comparison_diff = previous.diff_mode;
                                if let Some(kind) = previous.local_selection
                                    && let Some(index) = this
                                        .history_state
                                        .local_rows
                                        .iter()
                                        .position(|row| *row == kind)
                                {
                                    this.select(index, window, cx);
                                }
                            })),
                    ),
            )
            .into_any_element()
    }
}

use super::*;

pub(super) struct BranchDialog {
    pub(super) branches: Vec<String>,
    pub(super) filtered: Vec<usize>,
    pub(super) filter: Entity<InputState>,
    _subscription: Subscription,
    pub(super) index: usize,
    pub(super) scroll: UniformListScrollHandle,
}

impl BranchDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<HistoryWindow>) -> Self {
        let branch_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search refs"));
        let branch_subscription = cx.subscribe_in(
            &branch_filter,
            window,
            |this, _, event, _, cx| match event {
                InputEvent::Change => {
                    this.branch_dialog.filter(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.switch_branch(this.branch_dialog.index, cx),
                _ => {}
            },
        );
        Self {
            branches: Vec::new(),
            filtered: Vec::new(),
            filter: branch_filter,
            _subscription: branch_subscription,
            index: 0,
            scroll: UniformListScrollHandle::new(),
        }
    }

    pub(super) fn render(
        &self,
        body: gpui_kit::Stateful<gpui_kit::Div>,
        theme: Theme,
        operation_message: &str,
        pending: bool,
        cx: &mut Context<HistoryWindow>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let palette = Palette::for_theme(theme);
        body.overflow_hidden()
            .child(Input::new(&self.filter).cleanable(true))
            .child(
                div()
                    .relative()
                    .h(px(300.))
                    .min_h_0()
                    .child(
                        uniform_list(
                            "branches",
                            self.filtered.len(),
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                range
                                    .map(|i| {
                                        div()
                                            .id(("branch", i))
                                            .h(px((this.preferences.font_size * 2.).max(26.)))
                                            .px_2()
                                            .bg(rgb(if i == this.branch_dialog.index {
                                                colors::SOLID_SELECTION
                                            } else {
                                                Palette::for_theme(this.config.theme).panel
                                            }))
                                            .when(i == this.branch_dialog.index, |row| {
                                                row.text_color(rgb(colors::ON_SOLID))
                                            })
                                            .child(
                                                this.branch_dialog.branches
                                                    [this.branch_dialog.filtered[i]]
                                                    .clone(),
                                            )
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.switch_branch(i, cx)
                                            }))
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .size_full()
                        .track_scroll(&self.scroll),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .id("branch-scrollbar-viewport")
                            .test_support()
                            .child(
                                Scrollbar::vertical(&self.scroll)
                                    .mode(ScrollbarMode::Always)
                                    .id("branch-scrollbar"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("branch-operation-status")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(pending, |status| {
                        status.child(
                            Spinner::new()
                                .icon(gpui_kit::assets::IconName::LoaderCircle)
                                .color(rgb(palette.foreground).into()),
                        )
                    })
                    .child(operation_message.to_owned()),
            )
    }

    pub(super) fn set_branches(&mut self, branches: Vec<String>, cx: &App) {
        self.branches = branches;
        self.filter(cx);
    }

    pub(super) fn filter(&mut self, cx: &App) {
        let query = self.filter.read(cx).value().trim().to_lowercase();
        self.filtered = self
            .branches
            .iter()
            .enumerate()
            .filter_map(|(i, branch)| branch.to_lowercase().contains(&query).then_some(i))
            .collect();
        self.index = 0;
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    pub(super) fn reset(&mut self, window: &mut Window, cx: &mut Context<HistoryWindow>) {
        self.branches.clear();
        self.filtered.clear();
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.index = 0;
        self.filter.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn selected(&self, index: usize) -> Option<String> {
        self.filtered
            .get(index)
            .and_then(|i| self.branches.get(*i))
            .cloned()
    }

    pub(super) fn move_selection(&mut self, down: bool) {
        self.index = if down {
            (self.index + 1).min(self.filtered.len().saturating_sub(1))
        } else {
            self.index.saturating_sub(1)
        };
        self.scroll
            .scroll_to_item(self.index, ScrollStrategy::Center);
    }
}

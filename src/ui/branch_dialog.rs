use super::*;

#[derive(Clone, Copy)]
pub(super) enum BranchChoice {
    Current,
    AllBranches,
    Reference(usize),
}

impl BranchChoice {
    fn label(self, branches: &[String], browse: bool) -> String {
        match self {
            Self::Current => "Current branch".into(),
            Self::AllBranches => "All branches and tags".into(),
            Self::Reference(index) if browse => gitk_rs::git::reference_label(&branches[index]),
            Self::Reference(index) => branches[index].clone(),
        }
    }
}

pub(super) struct BranchDialog {
    pub(super) branches: Vec<String>,
    pub(super) filtered: Vec<BranchChoice>,
    pub(super) include_scopes: bool,
    pub(super) filter: Entity<InputState>,
    _subscription: Subscription,
    pub(super) index: usize,
    pub(super) scroll: UniformListScrollHandle,
}

impl BranchDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<HistoryWindow>) -> Self {
        let branch_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter references"));
        let branch_subscription = cx.subscribe_in(
            &branch_filter,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    this.branch_dialog.filter(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.switch_branch(this.branch_dialog.index, window, cx)
                }
                _ => {}
            },
        );
        Self {
            branches: Vec::new(),
            filtered: Vec::new(),
            include_scopes: false,
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
        font_size: f32,
        cx: &mut Context<HistoryWindow>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let palette = Palette::for_theme(theme);
        let status_message = self.status_message(operation_message, pending);
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
                                            .child(this.branch_dialog.filtered[i].label(
                                                &this.branch_dialog.branches,
                                                matches!(
                                                    this.dialog,
                                                    Some(DialogKind::BrowseReferences)
                                                ),
                                            ))
                                            .test_support()
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.switch_branch(i, window, cx)
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
                    .flex_shrink_0()
                    .h(px((font_size * 1.5).max(24.)))
                    .overflow_hidden()
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(status_message.clone()),
                    )
                    .hoverable_tooltip(move |window, cx| {
                        wrapped_tooltip(status_message.clone(), window, cx)
                    }),
            )
    }

    pub(super) fn status_message(&self, operation_message: &str, pending: bool) -> String {
        if !operation_message.is_empty() {
            return operation_message.to_owned();
        }
        if pending {
            return "Loading references…".into();
        }
        if self.filtered.is_empty() {
            return "No matching entries".into();
        }
        let total = self.branches.len() + if self.include_scopes { 2 } else { 0 };
        if self.filtered.len() == total {
            format!("{} entries · Enter to select", self.filtered.len())
        } else {
            format!(
                "{} of {total} entries · Enter to select",
                self.filtered.len()
            )
        }
    }

    pub(super) fn set_branches(&mut self, branches: Vec<String>, cx: &App) {
        self.branches = branches;
        self.filter(cx);
    }

    pub(super) fn filter(&mut self, cx: &App) {
        let query = self.filter.read(cx).value().trim().to_lowercase();
        self.filtered = [BranchChoice::Current, BranchChoice::AllBranches]
            .into_iter()
            .filter(|choice| {
                self.include_scopes
                    && choice
                        .label(&self.branches, true)
                        .to_lowercase()
                        .contains(&query)
            })
            .chain(self.branches.iter().enumerate().filter_map(|(i, branch)| {
                branch
                    .to_lowercase()
                    .contains(&query)
                    .then_some(BranchChoice::Reference(i))
            }))
            .collect();
        self.index = 0;
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
    }

    pub(super) fn reset(
        &mut self,
        include_scopes: bool,
        window: &mut Window,
        cx: &mut Context<HistoryWindow>,
    ) {
        self.include_scopes = include_scopes;
        self.branches.clear();
        self.filtered.clear();
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.filter(cx);
        self.filter.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn selected(&self, index: usize) -> Option<String> {
        self.filtered
            .get(index)
            .and_then(|choice| match choice {
                BranchChoice::Reference(i) => self.branches.get(*i),
                _ => None,
            })
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

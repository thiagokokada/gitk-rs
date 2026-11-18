use super::*;
use gitk_rs::{diff::ComparisonMode, git::HistoryScope};
use gpui_kit::base::Disableable;

#[derive(Clone)]
pub(super) struct ComparisonBase {
    pub(super) id: gitk_rs::git::CommitId,
    pub(super) subject: String,
}

impl ComparisonBase {
    pub(super) fn label(&self) -> String {
        format!("{} · {}", &self.id.to_hex()[..7], self.subject)
    }
}

#[derive(Clone, Copy)]
pub(super) enum ComparisonSide {
    Base,
    Target,
}

pub(super) struct ComparisonDialog {
    pub(super) base: Entity<InputState>,
    pub(super) target: Entity<InputState>,
    pub(super) destination: Option<ComparisonSide>,
    _subscriptions: Vec<Subscription>,
}

impl ComparisonDialog {
    pub(super) fn new(window: &mut Window, cx: &mut Context<HistoryWindow>) -> Self {
        let base = cx.new(|cx| InputState::new(window, cx).placeholder("Branch, tag, or commit"));
        let target = cx.new(|cx| InputState::new(window, cx).placeholder("Branch, tag, or commit"));
        let subscriptions = [&base, &target]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |this, _, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                    if matches!(event, InputEvent::PressEnter { .. })
                        && matches!(this.dialog, Some(DialogKind::Compare))
                    {
                        this.apply_comparison(window, cx);
                    }
                })
            })
            .collect();
        Self {
            base,
            target,
            destination: None,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn render(
        &self,
        body: gpui_kit::Stateful<gpui_kit::Div>,
        theme: Theme,
        cx: &mut Context<HistoryWindow>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let palette = Palette::for_theme(theme);
        let can_compare = !self.base.read(cx).value().trim().is_empty()
            && !self.target.read(cx).value().trim().is_empty();
        body.child("Show commits reachable from Target that are missing from Base.")
            .child("Base")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.base)))
                    .child(
                        Button::new("choose-comparison-base")
                            .label("Choose…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_comparison_reference(ComparisonSide::Base, window, cx)
                            })),
                    ),
            )
            .child("Target")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.target)))
                    .child(
                        Button::new("choose-comparison-target")
                            .label("Choose…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_comparison_reference(ComparisonSide::Target, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("apply-comparison")
                    .test_support()
                    .w_full()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .bg(rgb(palette.dialog_button))
                    .child("Compare")
                    .when(can_compare, |button| {
                        button.cursor_pointer().on_click(
                            cx.listener(|this, _, window, cx| this.apply_comparison(window, cx)),
                        )
                    })
                    .when(!can_compare, |button| button.opacity(0.5)),
            )
    }
}

impl HistoryWindow {
    pub(super) fn comparison_base_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(base) = &self.comparison_base else {
            return div().into_any_element();
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("comparison-base-indicator")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("Comparison base: {}", base.label())),
            )
            .child(
                Button::new("clear-comparison-base")
                    .label("Clear")
                    .tooltip("Clear the marked base for future comparisons.")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.comparison_base = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn open_comparison(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (base, target) = match &self.history_state.request.source.scope {
            HistoryScope::Compare { base, target } => (base.clone(), target.clone()),
            HistoryScope::Reference(target) => ("HEAD".into(), target.clone()),
            _ => ("HEAD".into(), "HEAD".into()),
        };
        self.comparison_dialog.destination = None;
        self.operation_message.clear();
        self.comparison_dialog
            .base
            .update(cx, |input, cx| input.set_value(base, window, cx));
        self.comparison_dialog
            .target
            .update(cx, |input, cx| input.set_value(target, window, cx));
        self.dialog = Some(DialogKind::Compare);
        self.comparison_dialog
            .target
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn choose_comparison_reference(
        &mut self,
        side: ComparisonSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_history_references(window, cx);
        self.branch_dialog.include_scopes = false;
        self.branch_dialog.filter(cx);
        self.comparison_dialog.destination = Some(side);
    }

    fn apply_comparison(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .comparison_dialog
            .base
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let target = self
            .comparison_dialog
            .target
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if base.is_empty() || target.is_empty() {
            return;
        }
        self.set_history_scope(HistoryScope::Compare { base, target }, window, cx);
    }

    pub(super) fn comparison_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("comparison-direction")
                    .test_support()
                    .child(self.history_state.request.source.scope.label()),
            )
            .child(Button::new("swap-comparison").label("Swap base and target")
                .tooltip("Reverse Base and Target and refresh the comparison, keeping the aggregate diff mode.")
                .on_click(cx.listener(|this, _, window, cx| {
                    if let HistoryScope::Compare { base, target } = &this.history_state.request.source.scope {
                        let scope = HistoryScope::Compare { base: target.clone(), target: base.clone() };
                        let mode = this.comparison_diff.unwrap_or(ComparisonMode::Endpoints);
                        this.set_history_scope(scope, window, cx);
                        this.comparison_diff = Some(mode);
                    }
                })))
            .child(
                Button::new("comparison-selected-commit")
                    .label("Selected commit diff")
                    .tooltip("Show the changes introduced by the highlighted commit, compared with its first parent.")
                    .disabled(self.comparison_diff.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.comparison_diff = None;
                        if let Some((_, entry)) = &this.history_state.selected {
                            this.load_selection(Selection::Commit(entry.clone()), cx);
                        } else {
                            this.diff
                                .update(cx, |diff, cx| diff.clear("No commit selected.", cx));
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("comparison-endpoints")
                    .label("Base → Target")
                    .tooltip("Compare the trees at Base and Target. Includes differences unique to either side; filters only narrow the commit list.")
                    .disabled(self.comparison_diff == Some(ComparisonMode::Endpoints))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_comparison_diff(ComparisonMode::Endpoints, cx)
                    })),
            )
            .child(
                Button::new("comparison-merge-base")
                    .label("Merge base → Target")
                    .tooltip("Compare Target with its common ancestor with Base. Filters only narrow the commit list.")
                    .disabled(self.comparison_diff == Some(ComparisonMode::SinceMergeBase))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_comparison_diff(ComparisonMode::SinceMergeBase, cx)
                    })),
            )
            .child(Button::new("exit-comparison").label("Exit comparison")
                .tooltip("Return to current branch history.")
                .on_click(cx.listener(|this, _, window, cx| this.set_history_scope(HistoryScope::Current, window, cx))))
            .into_any_element()
    }

    fn set_comparison_diff(&mut self, mode: ComparisonMode, cx: &mut Context<Self>) {
        self.comparison_diff = Some(mode);
        self.load_comparison(cx);
        cx.notify();
    }

    pub(super) fn load_comparison(&mut self, cx: &mut Context<Self>) {
        if let Some(mode) = self.comparison_diff
            && let Some(snapshot) = &self.history_state.snapshot
            && snapshot.generation == self.history_state.request.generation
            && snapshot.error.is_none()
            && let Some(endpoints) = &snapshot.comparison
        {
            let endpoints = endpoints.clone();
            self.diff.update(cx, |diff, cx| {
                diff.load_comparison(self.config.repository.clone(), endpoints, mode, cx)
            });
        }
    }
}

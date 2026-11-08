use super::*;
use gpui_kit::component::checkbox::Checkbox;

#[derive(Clone, Copy)]
enum PreferenceToggle {
    LightTheme,
    Syntax,
    Watch,
    TextGraph,
    OnDemand,
}

pub(super) type FontChoices = SearchableVec<SharedString>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FontCatalogGeneration(pub u64);

fn font_choices(mut installed: Vec<SharedString>, ui: &str, fixed: &str) -> Vec<SharedString> {
    // Virtual system aliases and saved fonts may not be in the installed list.
    installed.extend([
        SharedString::from(ui.to_owned()),
        SharedString::from(fixed.to_owned()),
    ]);
    installed.sort_unstable();
    installed.dedup();
    installed
}

pub(super) fn preview_size(size: f32, fallback: f32) -> f32 {
    if size.is_finite() {
        size.clamp(8., 24.)
    } else {
        fallback
    }
}

impl HistoryWindow {
    pub(super) fn preference_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let draft = &self.preferences_draft;
        let mut form = div()
            .flex()
            .flex_col()
            .gap_2()
            .child("Application settings");
        for (id, label, checked, setting) in [
            (
                "light-theme-setting",
                "Light theme",
                draft.theme == Theme::Light,
                PreferenceToggle::LightTheme,
            ),
            (
                "syntax-setting",
                "Syntax highlighting",
                draft.syntax,
                PreferenceToggle::Syntax,
            ),
            (
                "watch-setting",
                "Automatically reload repository changes",
                draft.watch,
                PreferenceToggle::Watch,
            ),
            (
                "text-graph-setting",
                "Use a text graph",
                draft.text_graph,
                PreferenceToggle::TextGraph,
            ),
            (
                "on-demand-setting",
                "Load history on demand",
                draft.batch.is_some(),
                PreferenceToggle::OnDemand,
            ),
        ] {
            form = form.child(Checkbox::new(id).label(label).checked(checked).on_click(
                cx.listener(move |this, checked: &bool, _, cx| {
                    this.toggle_preference(setting, *checked);
                    cx.notify();
                }),
            ));
        }
        form = form
            .child("History batch size (when loading on demand)")
            .child(
                div()
                    .id("history-batch-input")
                    .test_support()
                    .h_8()
                    .flex_shrink_0()
                    .child(
                        Input::new(&self.batch_input)
                            .disabled(draft.batch.is_none())
                            .size_full(),
                    ),
            )
            .child(format!("Logical graph columns (1–{MAX_GRAPH_LANES})"))
            .child(
                div()
                    .id("graph-columns-input")
                    .test_support()
                    .h_8()
                    .flex_shrink_0()
                    .child(Input::new(&self.graph_columns_input).size_full()),
            );
        form.into_any_element()
    }

    fn toggle_preference(&mut self, setting: PreferenceToggle, checked: bool) {
        match setting {
            PreferenceToggle::LightTheme => {
                self.preferences_draft.theme = if checked { Theme::Light } else { Theme::Dark }
            }
            PreferenceToggle::Syntax => self.preferences_draft.syntax = checked,
            PreferenceToggle::Watch => self.preferences_draft.watch = checked,
            PreferenceToggle::TextGraph => self.preferences_draft.text_graph = checked,
            PreferenceToggle::OnDemand => self.preferences_draft.batch = checked.then_some(1000),
        }
    }

    pub(super) fn draft_preferences(&self, cx: &App) -> Preferences {
        Preferences {
            ui_font: self
                .ui_font_select
                .read(cx)
                .selected_value()
                .map_or_else(String::new, ToString::to_string),
            fixed_font: self
                .fixed_font_select
                .read(cx)
                .selected_value()
                .map_or_else(String::new, ToString::to_string),
            font_size: self
                .font_size_input
                .read(cx)
                .value()
                .parse()
                .unwrap_or(f32::NAN),
            graph_columns: self
                .graph_columns_input
                .read(cx)
                .value()
                .parse()
                .unwrap_or(0),
            batch: self
                .preferences_draft
                .batch
                .map(|_| self.batch_input.read(cx).value().parse().unwrap_or(0)),
            ..self.preferences_draft.clone()
        }
    }

    fn populate_font_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft_preferences(cx);
        let choices = font_choices(
            self.font_catalog.clone().unwrap_or_default(),
            &draft.ui_font,
            &draft.fixed_font,
        );
        for selector in [&self.ui_font_select, &self.fixed_font_select] {
            selector.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(choices.clone()), window, cx)
            });
        }
    }

    pub(super) fn accept_font_catalog(
        &mut self,
        generation: FontCatalogGeneration,
        fonts: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.font_catalog_generation != generation
            || !matches!(self.dialog, Some(DialogKind::Preferences))
        {
            return;
        }
        self.font_catalog = Some(fonts);
        self.populate_font_choices(window, cx);
        cx.notify();
    }

    pub(super) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preferences_draft = self.preferences.with_config(&self.config);
        self.graph_columns_input.update(cx, |input, cx| {
            input.set_value(self.config.graph_columns.to_string(), window, cx)
        });
        self.batch_input.update(cx, |input, cx| {
            input.set_value(self.config.batch.unwrap_or(1000).to_string(), window, cx)
        });
        self.dialog = Some(DialogKind::Preferences);
        self.operation_message.clear();
        self.font_catalog_generation.0 += 1;
        let generation = self.font_catalog_generation;
        let choices = font_choices(
            self.font_catalog.clone().unwrap_or_default(),
            &self.preferences.ui_font,
            &self.preferences.fixed_font,
        );
        for (selector, value) in [
            (&self.ui_font_select, &self.preferences.ui_font),
            (&self.fixed_font_select, &self.preferences.fixed_font),
        ] {
            selector.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(choices.clone()), window, cx);
                state.set_selected_value(&SharedString::from(value.clone()), window, cx);
            });
        }
        self.font_size_input.update(cx, |input, cx| {
            input.set_value(self.preferences.font_size.to_string(), window, cx)
        });
        self.ui_font_select
            .update(cx, |selector, cx| selector.focus(window, cx));
        self._font_catalog_task = None;
        if self.font_catalog.is_none() {
            // Enumerating system fonts can scan font directories; keep it off the UI thread.
            let text_system = cx.text_system().clone();
            let fonts = cx.background_executor().spawn(async move {
                text_system
                    .all_font_names()
                    .into_iter()
                    .map(SharedString::from)
                    .collect::<Vec<_>>()
            });
            self._font_catalog_task = Some(cx.spawn_in(window, async move |this, cx| {
                let fonts = fonts.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.accept_font_catalog(generation, fonts, window, cx);
                });
            }));
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_saved_and_virtual_fonts_without_duplicates() {
        let names = font_choices(
            vec!["Menlo".into(), "Arial".into(), "Menlo".into()],
            ".SystemUIFont",
            "Saved Custom Mono",
        );
        assert_eq!(
            names.iter().map(|name| name.as_ref()).collect::<Vec<_>>(),
            vec![".SystemUIFont", "Arial", "Menlo", "Saved Custom Mono"]
        );
    }

    #[test]
    fn invalid_draft_sizes_cannot_break_the_preview() {
        assert_eq!(preview_size(f32::NAN, 13.), 13.);
        assert_eq!(preview_size(f32::INFINITY, 13.), 13.);
        assert_eq!(preview_size(0., 13.), 8.);
        assert_eq!(preview_size(100., 13.), 24.);
        assert_eq!(preview_size(16., 13.), 16.);
    }
}

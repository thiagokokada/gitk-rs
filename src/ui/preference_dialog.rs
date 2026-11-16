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

#[derive(Clone, Copy, PartialEq, Eq)]
struct PreferencesSaveGeneration(u64);

pub(super) struct PreferencesDialog {
    pub(super) draft: Preferences,
    pub(super) graph_columns_input: Entity<InputState>,
    pub(super) batch_input: Entity<InputState>,
    pub(super) git_workers_input: Entity<InputState>,
    pub(super) path: Option<std::path::PathBuf>,
    save_generation: PreferencesSaveGeneration,
    pub(super) save_pending: bool,
    pub(super) ui_font_select: Entity<SelectState<FontChoices>>,
    pub(super) fixed_font_select: Entity<SelectState<FontChoices>>,
    _font_subscriptions: Vec<Subscription>,
    pub(super) font_catalog: Option<Vec<SharedString>>,
    pub(super) font_catalog_generation: FontCatalogGeneration,
    _font_catalog_task: Option<Task<()>>,
    pub(super) font_size_input: Entity<InputState>,
}

impl PreferencesDialog {
    pub(super) fn new(
        preferences: &Preferences,
        window: &mut Window,
        cx: &mut Context<HistoryWindow>,
    ) -> Self {
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
        let graph_columns_input = cx.new(|cx| InputState::new(window, cx));
        let batch_input = cx.new(|cx| InputState::new(window, cx));
        let git_workers_input = cx.new(|cx| InputState::new(window, cx));
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
        for input in [&graph_columns_input, &batch_input, &git_workers_input] {
            font_subscriptions.push(cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()));
        }
        Self {
            draft: preferences.clone(),
            graph_columns_input,
            batch_input,
            git_workers_input,
            path: None,
            save_generation: PreferencesSaveGeneration(0),
            save_pending: false,
            ui_font_select,
            fixed_font_select,
            _font_subscriptions: font_subscriptions,
            font_catalog: None,
            font_catalog_generation: FontCatalogGeneration(0),
            _font_catalog_task: None,
            font_size_input,
        }
    }
    pub(super) fn begin(
        &mut self,
        preferences: &Preferences,
        config: &Config,
        window: &mut Window,
        cx: &mut Context<HistoryWindow>,
    ) {
        self.draft = preferences.with_config(config);
        self.graph_columns_input.update(cx, |input, cx| {
            input.set_value(config.graph_columns.to_string(), window, cx)
        });
        self.git_workers_input.update(cx, |input, cx| {
            input.set_value(config.git_workers.to_string(), window, cx)
        });
        self.batch_input.update(cx, |input, cx| {
            input.set_value(config.batch.unwrap_or(1000).to_string(), window, cx)
        });
        self.font_catalog_generation.0 += 1;
        let choices = font_choices(
            self.font_catalog.clone().unwrap_or_default(),
            &preferences.ui_font,
            &preferences.fixed_font,
        );
        for (selector, value) in [
            (&self.ui_font_select, &preferences.ui_font),
            (&self.fixed_font_select, &preferences.fixed_font),
        ] {
            selector.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(choices.clone()), window, cx);
                state.set_selected_value(&SharedString::from(value.clone()), window, cx);
            });
        }
        self.font_size_input.update(cx, |input, cx| {
            input.set_value(preferences.font_size.to_string(), window, cx)
        });
        self.ui_font_select
            .update(cx, |selector, cx| selector.focus(window, cx));
        let generation = self.font_catalog_generation;
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
    }

    pub(super) fn save(&mut self, preferences: Preferences, cx: &mut Context<HistoryWindow>) {
        self.save_pending = true;
        self.save_generation.0 += 1;
        let generation = self.save_generation;
        let path = self.path.clone();
        let save = cx.background_executor().spawn(async move {
            path.as_ref()
                .map_or_else(|| preferences.save(), |path| preferences.save_to(path))
                .map_err(|error| {
                    eprintln!("gitk-rs: save preferences: {error:#}");
                    format!("{error:#}")
                })
        });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                if this.preference_dialog.save_generation != generation {
                    return;
                }
                this.preference_dialog.save_pending = false;
                this.operation_message = result.err().map_or_else(
                    || "Preferences saved".into(),
                    |error| format!("Save preferences: {error}"),
                );
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render(
        &self,
        body: gpui_kit::Stateful<gpui_kit::Div>,
        theme: Theme,
        preferences: &Preferences,
        operation_message: &str,
        cx: &mut Context<HistoryWindow>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let palette = Palette::for_theme(theme);
        let draft = self.draft_preferences(cx);
        let options = self.options(cx);
        let preview_size = preview_size(draft.font_size, preferences.font_size);
        body.h(px(640.))
            .overflow_hidden()
            .child(
                div()
                    .id("preferences-scroll-viewport")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .pr_3()
                            .child("UI font family")
                            .child(
                                div().h_8().flex_shrink_0().child(
                                    Select::new(&self.ui_font_select)
                                        .id("ui-font-select")
                                        .disabled(self.font_catalog.is_none())
                                        .accessibility_label("UI font family")
                                        .search_placeholder("Search installed fonts…")
                                        .menu_max_h(px(220.))
                                        .w_full(),
                                ),
                            )
                            .child("Fixed font family")
                            .child(
                                div().h_8().flex_shrink_0().child(
                                    Select::new(&self.fixed_font_select)
                                        .id("fixed-font-select")
                                        .disabled(self.font_catalog.is_none())
                                        .accessibility_label("Fixed font family")
                                        .search_placeholder("Search installed fonts…")
                                        .menu_max_h(px(220.))
                                        .w_full(),
                                ),
                            )
                            .when(self.font_catalog.is_none(), |form| {
                                form.child("Loading installed fonts…")
                            })
                            .child("Font size (8–24 pixels)")
                            .child(
                                div()
                                    .id("font-size-input")
                                    .test_support()
                                    .h_8()
                                    .flex_shrink_0()
                                    .track_focus(&self.font_size_input.read(cx).focus_handle(cx))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(palette.border))
                                    .focus(|input| input.border_color(rgb(colors::SOLID_SELECTION)))
                                    .bg(rgb(palette.row))
                                    .text_color(rgb(palette.foreground))
                                    .child(
                                        gpui_kit::base::NumberInput::new(&self.font_size_input)
                                            .accessibility_label("Font size in pixels")
                                            .controls_right()
                                            .size_full()
                                            .input(
                                                Input::new(&self.font_size_input)
                                                    .appearance(false)
                                                    .h_full()
                                                    .w_full()
                                                    .text_align(gpui_kit::TextAlign::Left),
                                            )
                                            .increment_button(move |button| {
                                                button
                                                    .accessibility_label("Increase font size")
                                                    .w_8()
                                                    .text_size(px(12.))
                                                    .line_height(px(12.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .hover(move |button| {
                                                        button.bg(rgb(palette.dialog_button))
                                                    })
                                                    .child("+")
                                            })
                                            .decrement_button(move |button| {
                                                button
                                                    .accessibility_label("Decrease font size")
                                                    .w_8()
                                                    .text_size(px(12.))
                                                    .line_height(px(12.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .hover(move |button| {
                                                        button.bg(rgb(palette.dialog_button))
                                                    })
                                                    .child("−")
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .p_3()
                                    .bg(rgb(palette.row))
                                    .border_1()
                                    .border_color(rgb(palette.border))
                                    .child(
                                        div()
                                            .font_family(system_fonts::resolve(&draft.ui_font, cx))
                                            .child("Interface preview"),
                                    )
                                    .child(
                                        div()
                                            .id("ui-font-preview")
                                            .flex()
                                            .flex_col()
                                            .test_support()
                                            .font_family(system_fonts::resolve(&draft.ui_font, cx))
                                            .text_size(px(preview_size))
                                            .line_height(px(preview_size * 1.5))
                                            .child("The quick brown fox jumps over the lazy dog.")
                                            .child("AaBbCc 0123456789"),
                                    )
                                    .child(
                                        div()
                                            .font_family(system_fonts::resolve(
                                                &draft.fixed_font,
                                                cx,
                                            ))
                                            .child("Fixed-width preview"),
                                    )
                                    .child(
                                        div()
                                            .id("fixed-font-preview")
                                            .flex()
                                            .flex_col()
                                            .test_support()
                                            .font_family(system_fonts::resolve(
                                                &draft.fixed_font,
                                                cx,
                                            ))
                                            .text_size(px(preview_size))
                                            .line_height(px(preview_size * 1.5))
                                            .child("fn main() { let value = 42; }")
                                            .child("0123456789  []{}()  != ->"),
                                    ),
                            )
                            .child(options)
                            .overflow_y_scrollbar()
                            .id("font-preferences-fields")
                            .scroll_bounce(false),
                    ),
            )
            .when(!operation_message.is_empty(), |body| {
                body.child(operation_message.to_owned())
            })
            .child(
                div()
                    .id("apply-preferences")
                    .flex_shrink_0()
                    .test_support()
                    .px_3()
                    .py_2()
                    .bg(rgb(palette.dialog_button))
                    .cursor_pointer()
                    .child(if self.save_pending {
                        "Saving…"
                    } else {
                        "Apply and save"
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.apply_preferences(window, cx)),
                    ),
            )
    }

    fn options(&self, cx: &mut Context<HistoryWindow>) -> AnyElement {
        let draft = &self.draft;
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
                    this.preference_dialog.toggle_preference(setting, *checked);
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
            .child(format!("Git workers (1–{})", gitk_rs::git::MAX_GIT_WORKERS))
            .child(
                div()
                    .id("git-workers-input")
                    .test_support()
                    .h_8()
                    .flex_shrink_0()
                    .child(Input::new(&self.git_workers_input).size_full()),
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
                self.draft.theme = if checked { Theme::Light } else { Theme::Dark }
            }
            PreferenceToggle::Syntax => self.draft.syntax = checked,
            PreferenceToggle::Watch => self.draft.watch = checked,
            PreferenceToggle::TextGraph => self.draft.text_graph = checked,
            PreferenceToggle::OnDemand => self.draft.batch = checked.then_some(1000),
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
            git_workers: self.git_workers_input.read(cx).value().parse().unwrap_or(0),
            graph_columns: self
                .graph_columns_input
                .read(cx)
                .value()
                .parse()
                .unwrap_or(0),
            batch: self
                .draft
                .batch
                .map(|_| self.batch_input.read(cx).value().parse().unwrap_or(0)),
            ..self.draft.clone()
        }
    }

    fn populate_font_choices(&mut self, window: &mut Window, cx: &mut Context<HistoryWindow>) {
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
}

fn font_choices(mut installed: Vec<SharedString>, ui: &str, fixed: &str) -> Vec<SharedString> {
    // Virtual system aliases and saved fonts may not be in the installed list.
    installed.extend([
        gitk_rs::preferences::SYSTEM_UI_FONT.into(),
        gitk_rs::preferences::SYSTEM_MONOSPACE_FONT.into(),
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
    pub(super) fn accept_font_catalog(
        &mut self,
        generation: FontCatalogGeneration,
        fonts: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preference_dialog.font_catalog_generation != generation
            || !matches!(self.dialog, Some(DialogKind::Preferences))
        {
            return;
        }
        self.preference_dialog.font_catalog = Some(fonts);
        self.preference_dialog.populate_font_choices(window, cx);
        cx.notify();
    }

    pub(super) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(DialogKind::Preferences);
        self.operation_message.clear();
        self.preference_dialog
            .begin(&self.preferences, &self.config, window, cx);
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
            vec![
                ".SystemUIFont",
                "Arial",
                "Menlo",
                "Saved Custom Mono",
                "System Monospace",
                "System UI"
            ]
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

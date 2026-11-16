use super::*;
use gitk_rs::search::Qualifier;

impl HistoryWindow {
    pub(super) fn open_repository(&mut self, cx: &mut Context<Self>) {
        if self.repository_prompt_pending {
            return;
        }
        self.repository_prompt_pending = true;
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git repository".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |this, cx| {
                this.repository_prompt_pending = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.change_repository(path, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.operation_message = format!(
                            "Desktop picker unavailable: {error:#}; enter a repository path below"
                        );
                        this.dialog = Some(DialogKind::OpenRepository);
                        this.restore_repository_focus = true;
                    }
                    Err(error) => {
                        this.operation_message = format!(
                            "Desktop picker unavailable: {error}; enter a repository path below"
                        );
                        this.dialog = Some(DialogKind::OpenRepository);
                        this.restore_repository_focus = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn change_repository(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.config.repository = std::path::absolute(&path).unwrap_or(path);
        self.workspace_generation.0 += 1;
        self.workspace_snapshot = None;
        self.history_state.reset_repository();
        self.pending_operation = None;
        self.operation_message.clear();
        self.dialog = None;
        self.restore_focus = true;
        self.reload(cx);
    }
    pub(super) fn open_repository_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(DialogKind::OpenRepository);
        self.operation_message.clear();
        self.repository_input.update(cx, |input, cx| {
            input.set_value(
                self.config.repository.to_string_lossy().into_owned(),
                window,
                cx,
            );
            input.focus(window, cx);
        });
        cx.notify();
    }
    pub(super) fn apply_repository_path(&mut self, cx: &mut Context<Self>) {
        let path = self.repository_input.read(cx).value().to_string();
        if path.is_empty() {
            self.operation_message = "Enter a repository directory".into();
            cx.notify();
            return;
        }
        self.change_repository(path.into(), cx);
    }

    pub(super) fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(DialogKind::Branches);
        self.dialog_focus.focus(window, cx);
        self.branch_dialog.reset(window, cx);
        self.operation_message = "Loading local branches…".into();
        self.operation_id.value += 1;
        if self.workspace.operate(
            self.workspace_generation,
            self.operation_id,
            Operation::Branches,
        ) {
            self.pending_operation = Some(self.operation_id);
        } else {
            self.operation_message = "A repository operation is already pending".into();
        }
        cx.notify();
    }
    pub(super) fn switch_branch(&mut self, index: usize, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(DialogKind::Branches)) || self.pending_operation.is_some() {
            return;
        }
        let Some(branch) = self.branch_dialog.selected(index) else {
            return;
        };
        self.operation_id.value += 1;
        self.operation_message = format!("Switching to {branch}…");
        if self.config.verbose {
            eprintln!("gitk-rs: {}", self.operation_message);
        }
        if self.workspace.operate(
            self.workspace_generation,
            self.operation_id,
            Operation::Switch(branch),
        ) {
            self.pending_operation = Some(self.operation_id);
        }
        cx.notify();
    }
    pub(super) fn scroll_shortcuts(&mut self, delta: gpui_kit::Pixels, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(DialogKind::Help)) {
            return;
        }
        let mut offset = self.help_scroll.offset();
        offset.y = (offset.y - delta).clamp(-self.help_scroll.max_offset().y, px(0.));
        self.help_scroll.set_offset(offset);
        cx.notify();
    }

    pub(super) fn dialog_element(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = match self.dialog {
            Some(DialogKind::Branches) => "Switch local branch",
            Some(DialogKind::Help) => "Help",
            Some(DialogKind::Preferences) => "Preferences",
            Some(DialogKind::OpenRepository) => "Open repository by path",
            _ => "About gitk-rs",
        };
        let palette = Palette::for_theme(self.config.theme);
        let mut body = div()
            .id("dialog-card")
            .max_h(gpui_kit::relative(0.9))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .w(px(620.))
            .p_4()
            .bg(rgb(palette.panel))
            .text_color(rgb(palette.foreground));
        body = body.child(div().flex_shrink_0().text_size(px(18.)).child(title));
        match self.dialog {
            Some(DialogKind::Branches) => {
                body = self.branch_dialog.render(
                    body,
                    self.config.theme,
                    &self.operation_message,
                    self.pending_operation.is_some(),
                    cx,
                );
            }
            Some(DialogKind::OpenRepository) => {
                body = body
                    .child("Repository directory")
                    .child(Input::new(&self.repository_input))
                    .child(self.operation_message.clone())
                    .child(
                        Button::new("open-path")
                            .label("Open")
                            .on_click(cx.listener(|this, _, _, cx| this.apply_repository_path(cx))),
                    )
            }
            Some(DialogKind::Preferences) => {
                body = self.preference_dialog.render(
                    body,
                    self.config.theme,
                    &self.preferences,
                    &self.operation_message,
                    cx,
                );
            }
            Some(DialogKind::Help) => {
                body = body.h(gpui_kit::relative(0.9)).overflow_hidden().child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(
                            div()
                                .id("shortcut-groups")
                                .test_support()
                                .size_full()
                                .pr_3()
                                .overflow_y_scroll()
                                .track_scroll(&self.help_scroll)
                                .flex()
                                .flex_col()
                                .gap_4()
                                .children(SHORTCUT_GROUPS.iter().map(|group| {
                                    div()
                                        .id(group.title)
                                        .test_support()
                                        .flex_shrink_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .font_weight(gpui_kit::FontWeight::BOLD)
                                                .child(group.title),
                                        )
                                        .children(
                                            group
                                                .qualifiers
                                                .iter()
                                                .map(|qualifier| {
                                                    (
                                                        SharedString::from(qualifier.syntax()),
                                                        qualifier.help_description(),
                                                    )
                                                })
                                                .chain(group.shortcuts.iter().map(
                                                    |&(keys, description)| {
                                                        (SharedString::from(keys), description)
                                                    },
                                                ))
                                                .map(|(keys, description)| {
                                                    div()
                                                        .w_full()
                                                        .min_w_0()
                                                        .flex_shrink_0()
                                                        .flex()
                                                        .items_start()
                                                        .gap_3()
                                                        .child(
                                                            div()
                                                                .w(px(235.))
                                                                .flex_shrink_0()
                                                                .child(keys.clone()),
                                                        )
                                                        .child(
                                                            div()
                                                                .id(keys)
                                                                .test_support()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .whitespace_normal()
                                                                .child(description),
                                                        )
                                                }),
                                        )
                                })),
                        )
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .id("help-scrollbar-viewport")
                                .test_support()
                                .child(
                                    Scrollbar::vertical(&self.help_scroll)
                                        .mode(ScrollbarMode::Always)
                                        .id("help-scrollbar"),
                                ),
                        ),
                );
            }
            _ => {
                body = body
                    .child(format!("gitk-rs {}", gitk_rs::VERSION))
                    .child("A lightweight Git history explorer, written in Rust with GPUI.")
                    .child("MIT licensed.")
            }
        }
        body = body.child(
            div()
                .id("close-dialog")
                .flex_shrink_0()
                .test_support()
                .px_3()
                .py_2()
                .bg(rgb(palette.dialog_button))
                .cursor_pointer()
                .child("Close (Escape)")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dialog = None;
                    this.focus.focus(window, cx);
                    cx.notify();
                })),
        );
        div()
            .id("dialog-overlay")
            .test_support()
            .absolute()
            .inset_0()
            // Modal hit testing must include scroll events, including at list boundaries.
            .occlude()
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .bg(gpui_kit::rgba(colors::DIALOG_OVERLAY))
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.dialog_focus)
            .key_context(match self.dialog {
                Some(DialogKind::Branches) => "Dialog BranchDialog",
                Some(DialogKind::Help) => "Dialog HelpDialog",
                _ => "Dialog",
            })
            // Select handles Ctrl/Cmd+Enter as secondary Confirm before the
            // window's Apply action can bubble. Capture it at the preferences
            // boundary while preserving plain Enter for selecting a font.
            .when(
                matches!(self.dialog, Some(DialogKind::Preferences)),
                |dialog| {
                    dialog.capture_action(cx.listener(
                        |this, confirm: &gpui_kit::base::actions::Confirm, window, cx| {
                            if confirm.secondary {
                                this.apply_preferences(window, cx);
                                cx.stop_propagation();
                            }
                        },
                    ))
                },
            )
            .child(body)
            .into_any_element()
    }
    pub(super) fn apply_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(DialogKind::Preferences)) {
            return;
        }
        if self.preference_dialog.save_pending {
            return;
        }
        let preferences = self.preference_dialog.draft_preferences(cx);
        if let Err(error) = preferences.validate() {
            self.operation_message = format!("Invalid preferences: {error}");
            cx.notify();
            return;
        }
        let previous = self.config.clone();
        let previous_source = self.history_state.request.source.clone();
        self.config.apply_preferences(&preferences);
        self.history_state.request.batch = self.config.batch;
        self.history_state.request.source.graph_columns = self.config.graph_columns;
        self.history_state.request.source.git_workers = self.config.git_workers;
        if previous.watch != self.config.watch {
            self.workspace_generation.0 += 1;
            self.refresh_workspace();
        }
        if previous.theme != self.config.theme {
            apply_theme(self.config.theme, window, cx);
        }
        self.diff.update(cx, |diff, cx| {
            diff.set_settings(
                DiffSettings {
                    syntax: self.config.syntax,
                    dark: self.config.theme == Theme::Dark,
                },
                cx,
            );
            diff.set_font(
                system_fonts::resolve(&preferences.fixed_font, cx),
                preferences.font_size,
                cx,
            )
        });
        apply_fonts(&preferences, cx);
        self.refresh_filter_highlights(cx);
        window.refresh();
        self.preferences = preferences.clone();
        if previous_source != self.history_state.request.source {
            self.reload(cx);
        } else if previous.batch != self.config.batch {
            self.submit();
        }
        self.dialog = None;
        self.focus.focus(window, cx);
        self.preference_dialog.save(preferences, cx);
        cx.notify();
    }
}

struct ShortcutGroup {
    title: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
    qualifiers: &'static [Qualifier],
}

// Only application shortcuts gain Command aliases; Control input editing stays literal.
macro_rules! command_shortcut {
    ("Enter") => {
        if cfg!(target_os = "macos") {
            "Cmd+Return"
        } else {
            "Ctrl+Enter"
        }
    };
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("Cmd+", $key)
        } else {
            concat!("Ctrl+", $key)
        }
    };
    ($first:literal, $second:literal) => {
        if cfg!(target_os = "macos") {
            concat!("Cmd+", $first, " / Cmd+", $second)
        } else {
            concat!("Ctrl+", $first, " / Ctrl+", $second)
        }
    };
}

const ENTER_KEY: &str = if cfg!(target_os = "macos") {
    "Return"
} else {
    "Enter"
};

macro_rules! option_shortcut {
    ($first:literal, $second:literal) => {
        if cfg!(target_os = "macos") {
            concat!("Option+", $first, " / Option+", $second)
        } else {
            concat!("Alt+", $first, " / Alt+", $second)
        }
    };
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("Option+", $key)
        } else {
            concat!("Alt+", $key)
        }
    };
}

const SHORTCUT_GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        qualifiers: &[],
        title: "Commit list",
        shortcuts: &[
            ("Up / p / k", "Move up one commit"),
            ("Down / n / j", "Move down one commit"),
            (option_shortcut!("J", "K"), "Select next / previous commit"),
            ("Home / gg", "Jump to the first commit"),
            ("End / G", "Jump to the last loaded commit"),
            (
                command_shortcut!("PageUp", "PageDown"),
                "Scroll history up / down a page",
            ),
            (command_shortcut!("C"), "Copy commit hash and summary"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "Diff view",
        shortcuts: &[
            ("Delete / Backspace / b / B", "Scroll diff up one page"),
            ("Space", "Scroll diff down one page"),
            ("u / U", "Scroll diff up 18 lines"),
            ("d / D", "Scroll diff down 18 lines"),
            ("Drag / Shift-click", "Select text / extend selection"),
            ("Double-click", "Select a word"),
            (command_shortcut!("A"), "Select all diff text"),
            (command_shortcut!("C"), "Copy selected text"),
            (command_shortcut!("Shift+C"), "Copy without +/- markers"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "General",
        shortcuts: &[
            ("/", "Focus the filter box"),
            (
                ENTER_KEY,
                "Apply filter and return to history (in filter box)",
            ),
            (
                "Escape",
                "Dismiss filter suggestions, then leave the filter box",
            ),
            ("F5", "Reload commits"),
            (REGEX_SHORTCUT, "Toggle regex search"),
            ("F1 / ?", "Show help (? outside text inputs)"),
            (command_shortcut!("O"), "Open repository"),
            (command_shortcut!("Shift+O"), "Open repository by path"),
            (
                command_shortcut!("B"),
                "Switch local branch (outside text inputs)",
            ),
            (command_shortcut!(","), "Open preferences"),
            (option_shortcut!("T"), "Toggle theme (outside text inputs)"),
            (command_shortcut!("Q"), "Quit gitk-rs"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "Filter box and text inputs",
        shortcuts: &[
            (
                "Tab / Up / Down",
                "Complete filter tag / choose suggestion (when shown)",
            ),
            ("Ctrl+A / Ctrl+E", "Move to start / end of line"),
            ("Ctrl+B / Ctrl+F", "Move back / forward one character"),
            ("Ctrl+W", "Delete word before cursor"),
            ("Ctrl+U", "Delete to start of line"),
            ("Ctrl+K", "Delete to end of line"),
            ("Ctrl+D", "Delete character at cursor"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "Branch dialog",
        shortcuts: &[
            (
                if cfg!(target_os = "macos") {
                    "Up / Ctrl+P / Option+K"
                } else {
                    "Up / Ctrl+P / Alt+K"
                },
                "Select previous branch",
            ),
            (
                if cfg!(target_os = "macos") {
                    "Down / Ctrl+N / Option+J"
                } else {
                    "Down / Ctrl+N / Alt+J"
                },
                "Select next branch",
            ),
            (ENTER_KEY, "Switch to selected branch"),
            ("Type in filter", "Filter local branches"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "Help dialog",
        shortcuts: &[
            ("Up / Down", "Scroll up / down one line"),
            ("PageUp / PageDown", "Scroll up / down one page"),
            ("Home / End", "Jump to the top / bottom"),
        ],
    },
    ShortcutGroup {
        qualifiers: &[],
        title: "Dialogs",
        shortcuts: &[
            ("Escape", "Close the dialog"),
            (ENTER_KEY, "Open repository in the path input"),
            (command_shortcut!("Enter"), "Apply and save preferences"),
        ],
    },
    ShortcutGroup {
        qualifiers: &Qualifier::ALL,
        title: "Commit search",
        shortcuts: &[],
    },
];

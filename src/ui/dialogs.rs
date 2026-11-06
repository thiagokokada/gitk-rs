use super::*;

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
                            "Desktop picker unavailable: {error:#}. Enter a repository path below."
                        );
                        this.dialog = Some(DialogKind::OpenRepository);
                        this.restore_repository_focus = true;
                    }
                    Err(error) => {
                        this.operation_message = format!(
                            "Desktop picker unavailable: {error}. Enter a repository path below."
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
        self.config.repository = path;
        self.workspace_generation.0 += 1;
        self.workspace_snapshot = None;
        self.local_rows.clear();
        self.pending_operation = None;
        self.operation_message.clear();
        self.selected = None;
        self.local_selection = None;
        self.reload_selection = None;
        self.request.restore = None;
        self.snapshot = None;
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
            self.operation_message = "Enter a repository directory.".into();
            cx.notify();
            return;
        }
        self.change_repository(path.into(), cx);
    }

    pub(super) fn open_branches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(DialogKind::Branches);
        self.dialog_focus.focus(window, cx);
        self.branches.clear();
        self.filtered_branches.clear();
        self.branch_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.branch_index = 0;
        self.branch_filter
            .update(cx, |input, cx| input.focus(window, cx));
        self.operation_message = "Loading local branches…".into();
        self.operation_id += 1;
        if self.workspace.operate(
            self.workspace_generation,
            self.operation_id,
            Operation::Branches,
        ) {
            self.pending_operation = Some((self.operation_id, false));
        } else {
            self.operation_message = "A repository operation is already pending.".into();
        }
        cx.notify();
    }
    pub(super) fn switch_branch(&mut self, index: usize, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(DialogKind::Branches)) || self.pending_operation.is_some() {
            return;
        }
        let Some(branch) = self
            .filtered_branches
            .get(index)
            .and_then(|i| self.branches.get(*i))
            .cloned()
        else {
            return;
        };
        self.operation_id += 1;
        self.operation_message = format!("Switching to {branch}…");
        if self.config.verbose {
            eprintln!("gitk-rs: {}", self.operation_message);
        }
        if self.workspace.operate(
            self.workspace_generation,
            self.operation_id,
            Operation::Switch(branch),
        ) {
            self.pending_operation = Some((self.operation_id, true));
        }
        cx.notify();
    }
    pub(super) fn filter_branches(&mut self, cx: &mut Context<Self>) {
        let query = self.branch_filter.read(cx).value().trim().to_lowercase();
        self.filtered_branches = self
            .branches
            .iter()
            .enumerate()
            .filter_map(|(i, branch)| branch.to_lowercase().contains(&query).then_some(i))
            .collect();
        self.branch_index = 0;
        self.branch_scroll
            .scroll_to_item_strict(0, ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn dialog_element(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = match self.dialog {
            Some(DialogKind::Branches) => "Switch local branch",
            Some(DialogKind::Help) => "Keyboard shortcuts",
            Some(DialogKind::Preferences) => "Font preferences",
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
                body = body
                    .overflow_hidden()
                    .child(Input::new(&self.branch_filter))
                    .child(
                        div()
                            .relative()
                            .h(px(300.))
                            .min_h_0()
                            .child(
                                uniform_list(
                                    "branches",
                                    self.filtered_branches.len(),
                                    cx.processor(|this, range: Range<usize>, _, cx| {
                                        range
                                            .map(|i| {
                                                div()
                                                    .id(("branch", i))
                                                    .h(px(
                                                        (this.preferences.font_size * 2.).max(26.)
                                                    ))
                                                    .px_2()
                                                    .bg(rgb(if i == this.branch_index {
                                                        colors::SOLID_SELECTION
                                                    } else {
                                                        Palette::for_theme(this.config.theme).panel
                                                    }))
                                                    .when(i == this.branch_index, |row| {
                                                        row.text_color(rgb(colors::ON_SOLID))
                                                    })
                                                    .child(
                                                        this.branches[this.filtered_branches[i]]
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
                                .track_scroll(&self.branch_scroll),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .id("branch-scrollbar-viewport")
                                    .test_support()
                                    .child(
                                        Scrollbar::vertical(&self.branch_scroll)
                                            .mode(ScrollbarMode::Always)
                                            .id("branch-scrollbar"),
                                    ),
                            ),
                    )
                    .child(self.operation_message.clone());
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
                body = body
                    .child("UI font family")
                    .child(Input::new(&self.ui_font_input))
                    .child("Fixed font family")
                    .child(Input::new(&self.fixed_font_input))
                    .child("Font size (8–24 pixels)")
                    .child(Input::new(&self.font_size_input))
                    .child(self.operation_message.clone())
                    .child(
                        Button::new("apply-preferences")
                            .label("Apply and save")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_preferences(window, cx)
                            })),
                    )
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
                                        .children(group.shortcuts.iter().map(
                                            |&(keys, description)| {
                                                div()
                                                    .flex()
                                                    .gap_3()
                                                    .child(
                                                        div()
                                                            .w(px(235.))
                                                            .flex_shrink_0()
                                                            .child(keys),
                                                    )
                                                    .child(div().flex_1().child(description))
                                            },
                                        ))
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
                body = body.child(format!(
                    "gitk-rs {}\nRust/GPUI port of gitk-go.\nMIT licensed.",
                    env!("CARGO_PKG_VERSION")
                ))
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
            .key_context(if matches!(self.dialog, Some(DialogKind::Branches)) {
                "Dialog BranchDialog"
            } else {
                "Dialog"
            })
            .child(body)
            .into_any_element()
    }
    pub(super) fn apply_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.dialog, Some(DialogKind::Preferences)) {
            return;
        }
        let preferences = Preferences {
            ui_font: self.ui_font_input.read(cx).value().to_string(),
            fixed_font: self.fixed_font_input.read(cx).value().to_string(),
            font_size: self.font_size_input.read(cx).value().parse().unwrap_or(0.),
        };
        if let Err(error) = preferences.validate() {
            self.operation_message = error.to_string();
            cx.notify();
            return;
        }
        self.diff.update(cx, |diff, cx| {
            diff.set_font(
                preferences.fixed_font.clone().into(),
                preferences.font_size,
                cx,
            )
        });
        apply_fonts(&preferences, cx);
        window.refresh();
        self.preferences = preferences.clone();
        self.dialog = None;
        self.focus.focus(window, cx);
        let save = cx
            .background_executor()
            .spawn(async move { preferences.save().map_err(|e| format!("{e:#}")) });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |this, cx| {
                this.operation_message = result.err().map_or_else(
                    || "Preferences saved.".into(),
                    |error| format!("Save preferences: {error}"),
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

struct ShortcutGroup {
    title: &'static str,
    shortcuts: &'static [(&'static str, &'static str)],
}

// Only application shortcuts gain Command aliases; Control input editing stays literal.
macro_rules! command_shortcut {
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("Ctrl/Cmd+", $key)
        } else {
            concat!("Ctrl+", $key)
        }
    };
}

const SHORTCUT_GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Commit list",
        shortcuts: &[
            ("Up / p / k", "Move up one commit"),
            ("Down / n / j", "Move down one commit"),
            ("Home / gg", "Jump to the first commit"),
            ("End / G", "Jump to the last loaded commit"),
            (
                command_shortcut!("PageUp / PageDown"),
                "Scroll history up / down a page",
            ),
            (
                command_shortcut!("C"),
                "Copy selected commit hash and summary (history focused)",
            ),
        ],
    },
    ShortcutGroup {
        title: "Diff view",
        shortcuts: &[
            ("Delete / Backspace / b / B", "Scroll diff up one page"),
            ("Space", "Scroll diff down one page"),
            ("u / U", "Scroll diff up 18 lines"),
            ("d / D", "Scroll diff down 18 lines"),
            ("Drag / Shift-click", "Select text / extend selection"),
            ("Double-click", "Select a word"),
            (command_shortcut!("A"), "Select all (diff focused)"),
            (command_shortcut!("C"), "Copy selection (diff focused)"),
            (
                command_shortcut!("Shift+C"),
                "Copy without +/- markers (diff focused)",
            ),
        ],
    },
    ShortcutGroup {
        title: "General",
        shortcuts: &[
            ("/", "Focus the filter box"),
            ("Escape", "Leave the filter box"),
            ("F5", "Reload commits"),
            ("F1 / ?", "Show keyboard shortcuts (? outside text inputs)"),
            (command_shortcut!("O"), "Open repository"),
            (command_shortcut!("Shift+O"), "Open repository by path"),
            (
                command_shortcut!("B"),
                "Switch local branch (outside text inputs)",
            ),
            (command_shortcut!(","), "Open font preferences"),
            ("Alt+T", "Toggle theme (outside text inputs)"),
            ("Alt+R", "Toggle automatic reload (outside text inputs)"),
            (command_shortcut!("Q"), "Quit gitk-rs"),
        ],
    },
    ShortcutGroup {
        title: "Filter box and text inputs",
        shortcuts: &[
            ("Ctrl+A / Ctrl+E", "Move to start / end of line"),
            ("Ctrl+B / Ctrl+F", "Move back / forward one character"),
            ("Ctrl+W", "Delete word before cursor"),
            ("Ctrl+U", "Delete to start of line"),
            ("Ctrl+K", "Delete to end of line"),
            ("Ctrl+D", "Delete character at cursor"),
        ],
    },
    ShortcutGroup {
        title: "Branch dialog",
        shortcuts: &[
            ("Up / Ctrl+P", "Select previous branch"),
            ("Down / Ctrl+N", "Select next branch"),
            ("Enter", "Switch to selected branch"),
            ("Type in filter", "Filter local branches"),
        ],
    },
    ShortcutGroup {
        title: "Dialogs",
        shortcuts: &[
            ("Escape", "Close the dialog"),
            ("Enter", "Open repository in the path input"),
            (
                command_shortcut!("Enter"),
                "Apply and save font preferences",
            ),
        ],
    },
    ShortcutGroup {
        title: "Graph",
        shortcuts: &[
            ("Drag header edge", "Resize the graph column"),
            ("Dashed lanes", "Indicate folded idle branches"),
        ],
    },
];

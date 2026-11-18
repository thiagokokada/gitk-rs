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
        self.history_state.request.source.scope = Default::default();
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
    pub(super) fn history_scope_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child("History:")
            .child(
                Button::new("history-scope")
                    .label(self.history_state.request.source.scope.label())
                    .dropdown_menu(move |menu, _, _| {
                        let current = view.clone();
                        let all = view.clone();
                        let reference = view.clone();
                        menu.item(PopupMenuItem::new("Current branch").on_click(
                            move |_, window, cx| {
                                let _ = current.update(cx, |this, cx| {
                                    this.set_history_scope(
                                        gitk_rs::git::HistoryScope::Current,
                                        window,
                                        cx,
                                    )
                                });
                            },
                        ))
                        .item(PopupMenuItem::new("All branches and tags").on_click(
                            move |_, window, cx| {
                                let _ = all.update(cx, |this, cx| {
                                    this.set_history_scope(
                                        gitk_rs::git::HistoryScope::AllBranches,
                                        window,
                                        cx,
                                    )
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Choose branch or tag…").on_click(
                                move |_, window, cx| {
                                    let _ = reference.update(cx, |this, cx| {
                                        this.open_history_references(window, cx)
                                    });
                                },
                            ),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn set_history_scope(
        &mut self,
        scope: gitk_rs::git::HistoryScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_history_scope(scope, cx);
        self.focus.focus(window, cx);
    }

    fn apply_history_scope(&mut self, scope: gitk_rs::git::HistoryScope, cx: &mut Context<Self>) {
        self.history_state.request.source.scope = scope;
        self.history_state.reset_repository();
        if let Some(snapshot) = &self.workspace_snapshot {
            self.history_state.update_local_rows(&snapshot.status);
        }
        self.history_state.viewport = 0..10;
        self.history_state.request.range = 0..128;
        self.history_state.request.anchor = None;
        self.dialog = None;
        self.restore_focus = true;
        self.scroll.scroll_to_item_strict(0, ScrollStrategy::Top);
        self.reload(cx);
    }

    pub(super) fn open_history_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(DialogKind::BrowseReferences);
        self.dialog_focus.focus(window, cx);
        self.branch_dialog.reset(window, cx);
        self.operation_message = "Loading branches and tags…".into();
        self.operation_id.value += 1;
        if self.workspace.operate(
            self.workspace_generation,
            self.operation_id,
            Operation::References,
        ) {
            self.pending_operation = Some(self.operation_id);
        } else {
            self.operation_message = "A repository operation is already pending".into();
        }
        cx.notify();
    }

    pub(super) fn switch_branch(&mut self, index: usize, cx: &mut Context<Self>) {
        if matches!(self.dialog, Some(DialogKind::BrowseReferences)) {
            if self.pending_operation.is_none()
                && let Some(name) = self.branch_dialog.selected(index)
            {
                self.apply_history_scope(gitk_rs::git::HistoryScope::Reference(name), cx);
            }
            return;
        }
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
            Some(DialogKind::BrowseReferences) => "Browse branch or tag",
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
            Some(DialogKind::Branches | DialogKind::BrowseReferences) => {
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
                                .children(shortcuts::GROUPS.iter().map(|group| {
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
                                                .chain(group.shortcuts.iter().map(|shortcut| {
                                                    (
                                                        SharedString::from(shortcut.label()),
                                                        shortcut.description,
                                                    )
                                                }))
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
                Some(DialogKind::Branches | DialogKind::BrowseReferences) => "Dialog BranchDialog",
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

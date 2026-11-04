use super::*;

impl HistoryWindow {
    pub(super) fn open_repository(&mut self, cx: &mut Context<Self>) {
        if self.repository_prompt_pending {
            return;
        }
        self.repository_prompt_pending = true;
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
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
        self.dialog_focus.focus(window);
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
        let (panel, foreground, button) = if self.config.theme == Theme::Dark {
            (0x292d35, 0xe0e4eb, 0x454b57)
        } else {
            (0xffffff, 0x202329, 0xdde2e8)
        };
        let mut body = div()
            .id("dialog-card")
            .max_h(gpui::relative(0.9))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .w(px(620.))
            .p_4()
            .bg(rgb(panel))
            .text_color(rgb(foreground));
        body = body.child(div().text_size(px(18.)).child(title));
        match self.dialog {
            Some(DialogKind::Branches) => {
                body = body.child("Git preserves ordinary checkout protections. Unsaved changes are never stashed or discarded automatically.")
                    .child(Input::new(&self.branch_filter)).child(uniform_list("branches", self.filtered_branches.len(), cx.processor(|this, range: Range<usize>, _, cx| {
                        range.map(|i| div().id(("branch", i)).h(px((this.preferences.font_size * 2.).max(26.))).px_2().bg(rgb(if i == this.branch_index { 0x285b8e } else if this.config.theme == Theme::Dark { 0x292d35 } else { 0xffffff }))
                            .when(i == this.branch_index, |row| row.text_color(rgb(0xffffff))).child(this.branches[this.filtered_branches[i]].clone()).cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.switch_branch(i, cx)))).collect::<Vec<_>>()
                    })).h(px(300.)).track_scroll(self.branch_scroll.clone()))
                    .child(self.operation_message.clone());
            }
            Some(DialogKind::OpenRepository) => body = body.child("Repository directory").child(Input::new(&self.repository_input)).child(self.operation_message.clone())
                .child(Button::new("open-path").label("Open").on_click(cx.listener(|this, _, _, cx| this.apply_repository_path(cx)))),
            Some(DialogKind::Preferences) => body = body.child("UI font family").child(Input::new(&self.ui_font_input))
                .child("Fixed font family").child(Input::new(&self.fixed_font_input))
                .child("Font size (8–24 pixels)").child(Input::new(&self.font_size_input))
                .child(self.operation_message.clone())
                .child(Button::new("apply-preferences").label("Apply and save").on_click(cx.listener(|this, _, window, cx| this.apply_preferences(window, cx)))),
            Some(DialogKind::Help) => body = body.child("Arrows / j,k / n,p: select commit\nHome / End: first / last loaded\nCtrl+PageUp / PageDown: scroll history\nSpace: page diff down; Backspace / Delete / b: up\nu / d: scroll diff 18 lines\nDrag: select; Shift-click: extend; double-click: word\nCtrl+A in diff: select all; Ctrl+C: copy\nCtrl+Shift+C: copy without diff markers\n/: filter; Escape: leave filter / close dialog\nCtrl+O: open repository; Ctrl+B: local branches\nF5: reload; F1: help; Ctrl+Q: quit\nCtrl+,: fonts; Ctrl+Enter: apply preferences\nAlt+T: theme; Alt+R: automatic reload\nGraph: drag header edge to resize; dashed lanes fold idle branches\nFilter Ctrl+A/E: start/end; B/F: character\nFilter Ctrl+W/U/K/D: delete word/start/end/forward"),
            _ => body = body.child(format!("gitk-rs {}\nRust/GPUI port of gitk-go.\nMIT licensed. 213 offline syntaxes.", env!("CARGO_PKG_VERSION"))),
        }
        body = body.child(
            div()
                .id("close-dialog")
                .px_3()
                .py_2()
                .bg(rgb(button))
                .cursor_pointer()
                .child("Close (Escape)")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.dialog = None;
                    this.focus.focus(window);
                    cx.notify();
                })),
        );
        div()
            .absolute()
            .inset_0()
            .bg(gpui::rgba(0x00000099))
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
        self.focus.focus(window);
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

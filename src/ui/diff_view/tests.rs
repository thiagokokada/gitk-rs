use crate::ui::LocalChangeKind;
use crate::ui::tests::{Fixture, open, selected, until};
use gpui_kit::{
    AppContext, Modifiers, TestAppContext, point, px,
    test::{TestQueryExt, TestWindowExt},
};

#[gpui_kit::test]
fn ref_refresh_preserves_local_diff_page_and_text_selection(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    let source: String = (0..600)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect();
    std::fs::write(fixture.root().join("untracked.rs"), source).unwrap();
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).history_state.local_rows.is_empty())
    });
    cx.simulate_keystrokes("home");
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.count > 600)
        })
    });
    pane.update(cx, |pane, cx| pane.jump(400, cx));
    until(cx, |cx| {
        cx.update(|_, cx| pane.read(cx).viewport.start >= 400)
    });
    cx.update(|window, cx| pane.read(cx).focus.clone().focus(window, cx));
    cx.simulate_keystrokes("ctrl-a");
    let selection = cx.update(|_, cx| pane.read(cx).selection.unwrap().ordered());
    let identity = cx.update(|_, cx| pane.read(cx).page.selection);
    let revision = cx.update(|_, cx| {
        app.view
            .read(cx)
            .workspace_snapshot
            .as_ref()
            .unwrap()
            .revision
    });
    fixture.git(&["branch", "new-label"]);
    app.view.update(cx, |view, _| view.refresh_workspace());
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .workspace_snapshot
                .as_ref()
                .unwrap()
                .revision
                > revision
        })
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).page.selection), identity);
    assert!(cx.update(|_, cx| pane.read(cx).viewport.start >= 400));
    assert_eq!(
        cx.update(|_, cx| pane.read(cx).selection.unwrap().ordered()),
        selection
    );
}

#[gpui_kit::test]
fn removing_the_selected_local_row_returns_to_a_commit(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let head = fixture.commit("base");
    std::fs::write(fixture.root().join("file.txt"), "edited").unwrap();
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).history_state.local_rows.is_empty())
    });
    cx.simulate_keystrokes("home");
    assert!(
        cx.update(|_, cx| app.view.read(cx).history_state.local_selection
            == Some(LocalChangeKind::Unstaged))
    );
    std::fs::write(fixture.root().join("file.txt"), "base").unwrap();
    app.view.update(cx, |view, _| view.refresh_workspace());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.local_rows.is_empty()
                && view
                    .history_state
                    .selected
                    .as_ref()
                    .is_some_and(|s| s.1.commit.id.to_hex() == head)
        })
    });
}

#[gpui_kit::test]
fn file_scrollbar_reaches_and_selects_offscreen_files(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    for i in 0..80 {
        std::fs::write(
            fixture.root().join(format!("file-{i:03}.nix")),
            "{ a = 1; }\n",
        )
        .unwrap();
    }
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "many files"]);
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 80)
        })
    });
    let scrollbar = cx.update(|window, _| window.find("files-scrollbar-viewport").bounds());
    cx.simulate_click(
        point(scrollbar.right() - px(8.), scrollbar.bottom() - px(2.)),
        Modifiers::default(),
    );
    until(cx, |cx| {
        cx.update(|window, _| window.try_find(("diff-file", 80_usize)).is_some())
    });
    let last = cx.update(|window, _| window.find(("diff-file", 80_usize)).bounds());
    cx.simulate_click(last.center(), Modifiers::default());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let pane = pane.read(cx);
            pane.selected_file == Some(80)
                && pane.viewport.start > 200
                && pane
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.request == pane.page.id)
        })
    });
}

#[gpui_kit::test]
fn file_click_and_copy_use_the_full_diff_not_just_visible_rows(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let source: String = (0..400)
        .map(|i| format!("\tlet value_{i} = {i};\n"))
        .collect();
    std::fs::write(fixture.root().join("a.rs"), &source).unwrap();
    std::fs::write(fixture.root().join("b.nix"), "{ answer = 42; }\n").unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "two files"]);
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 2 && !s.highlighting && s.error.is_none())
        })
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).selected_file), Some(0));
    let bounds = cx.update(|window, _| window.find(("diff-file", 2_usize)).bounds());
    cx.simulate_click(bounds.center(), Modifiers::default());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let pane = pane.read(cx);
            pane.viewport.start > 200
                && pane.snapshot.as_ref().is_some_and(|s| {
                    s.request == pane.page.id
                        && !s.highlighting
                        && s.start <= pane.viewport.start
                        && s.start + s.lines.len() >= pane.viewport.end
                })
        })
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).selected_file), Some(2));
    // Focus the real diff context, then dispatch its production key bindings.
    cx.update(|window, cx| pane.read(cx).focus.clone().focus(window, cx));
    let modifiers: &[&str] = if cfg!(target_os = "macos") {
        &["ctrl", "cmd"]
    } else {
        &["ctrl"]
    };
    for modifier in modifiers {
        pane.update(cx, |pane, cx| {
            pane.selection = None;
            cx.notify();
        });
        cx.simulate_keystrokes(&format!("{modifier}-a {modifier}-c"));
        until(cx, |cx| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|s| s.contains("+\tlet value_399 = 399;"))
        });
        cx.simulate_keystrokes(&format!("{modifier}-shift-c"));
        until(cx, |cx| {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .is_some_and(|s| {
                    s.contains("\n\tlet value_399 = 399;") && !s.contains("+\tlet value_399")
                })
        });
        let copied = cx.read_from_clipboard().unwrap().text().unwrap();
        assert!(
            copied.contains(&source),
            "copy preserves tabs and crosses unloaded pages"
        );
        assert!(copied.contains("{ answer = 42; }"));
    }

    // The last file is short: its header cannot stay at the viewport top.
    // Painting must preserve the explicitly clicked file until scrolling moves.
    for _ in 0..3 {
        cx.refresh().unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| pane.read(cx).selected_file), Some(2));
    }
    let bounds = cx.update(|window, _| window.find(("diff-file", 0_usize)).bounds());
    cx.simulate_click(bounds.center(), Modifiers::default());
    until(cx, |cx| {
        cx.update(|_, cx| pane.read(cx).viewport.start == 0)
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).selected_file), Some(0));
    pane.update(cx, |pane, cx| pane.scroll_lines(20, cx));
    until(cx, |cx| {
        cx.update(|_, cx| pane.read(cx).selected_file == Some(1))
    });
    cx.update(|window, cx| app.view.read(cx).focus.clone().focus(window, cx));
    let identity = cx.update(|_, cx| pane.read(cx).page.selection);
    let generation = cx.update(|_, cx| app.view.read(cx).history_state.request.generation);
    cx.simulate_keystrokes("f5");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.request.generation != generation
                && view.history_state.reload_selection.is_none()
        })
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).selected_file), Some(1));
    assert_eq!(cx.update(|_, cx| pane.read(cx).page.selection), identity);
    let next = fixture.commit("new commit");
    cx.simulate_keystrokes("f5");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.rows.first().is_some_and(|e| e.commit.id.to_hex() == next))
        })
    });
    cx.simulate_keystrokes("home");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let pane = pane.read(cx);
            pane.selected_file == Some(0)
                && pane.snapshot.as_ref().is_some_and(|s| {
                    s.files.len() == 1 && s.files[0].path == std::path::Path::new("file.txt")
                })
        })
    });
}

#[gpui_kit::test]
fn staged_and_unstaged_diff_pages_settle_after_scrolling(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    let source: String = (0..600)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect();
    std::fs::write(fixture.root().join("changes.rs"), &source).unwrap();
    fixture.git(&["add", "changes.rs"]);
    std::fs::write(
        fixture.root().join("changes.rs"),
        source.replace("let value_", "const VALUE_"),
    )
    .unwrap();
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.local_rows == [LocalChangeKind::Unstaged, LocalChangeKind::Staged]
                && view.history_state.selected.is_some()
        })
    });
    for keys in ["home", "j"] {
        cx.simulate_keystrokes(keys);
        until(cx, |cx| {
            cx.update(|_, cx| {
                let pane = pane.read(cx);
                pane.snapshot
                    .as_ref()
                    .is_some_and(|s| s.count > 600 && s.error.is_none() && !s.highlighting)
            })
        });
        pane.update(cx, |pane, cx| pane.jump(500, cx));
        until(cx, |cx| {
            cx.update(|_, cx| {
                let pane = pane.read(cx);
                pane.viewport.start >= 500
                    && pane.snapshot.as_ref().is_some_and(|s| {
                        s.request == pane.page.id
                            && !s.highlighting
                            && s.start <= pane.viewport.start
                            && s.start + s.lines.len() >= pane.viewport.end
                    })
            })
        });
        let request = cx.update(|_, cx| pane.read(cx).page.id);
        for _ in 0..12 {
            cx.background_executor
                .advance_clock(std::time::Duration::from_millis(20));
            cx.run_until_parked();
            cx.refresh().unwrap();
            cx.run_until_parked();
            assert_eq!(cx.update(|_, cx| pane.read(cx).page.id), request);
        }
    }
}

#[gpui_kit::test]
fn alt_commit_navigation_works_with_history_and_diff_focus(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let older = fixture.commit("older");
    let newer = fixture.commit("newer");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&newer));
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    for focus_diff in [false, true] {
        if focus_diff {
            cx.update(|window, cx| pane.read(cx).focus.clone().focus(window, cx));
        }
        let focus = cx.update(|window, cx| window.focused(cx).unwrap());
        cx.simulate_keystrokes("alt-j");
        assert_eq!(selected(&app.view, cx).as_deref(), Some(older.as_str()));
        cx.simulate_keystrokes("alt-k");
        assert_eq!(selected(&app.view, cx).as_deref(), Some(newer.as_str()));
        assert!(cx.update(|window, _| focus.is_focused(window)));
    }
}

#[gpui_kit::test]
fn diff_option_changes_replace_generation_and_reject_old_pages(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.commit("changed");
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| cx.update(|_, cx| pane.read(cx).snapshot.is_some()));
    let radio = cx.update(|window, _| window.find("old-mode").bounds());
    let words = cx.update(|window, _| window.find("word-diff").bounds());
    let toolbar = cx.update(|window, _| window.find("diff-options").bounds());
    let padding = cx.update(|window, _| window.rem_size() * 0.5);
    let content = cx.update(|window, _| window.find("diff-content").bounds());
    assert!(
        (content.top() - toolbar.bottom()).abs() <= px(1.),
        "extra gap below toolbar"
    );
    assert!(
        toolbar.size.height + px(1.) >= words.size.height + padding,
        "toolbar {toolbar:?}; word control {words:?}"
    );
    assert!(
        (toolbar.center().y - radio.center().y).abs() <= px(1.),
        "toolbar {toolbar:?}; radio {radio:?}"
    );
    assert!((radio.center().y - words.center().y).abs() <= px(1.));
    cx.simulate_click(radio.center(), Modifiers::default());
    assert_eq!(
        cx.update(|_, cx| pane.read(cx).options.version),
        super::DiffVersion::Old
    );
    let old = cx.update(|_, cx| pane.read(cx).page.selection);
    pane.update(cx, |pane, cx| {
        pane.change_options(
            super::DiffOptions {
                version: super::DiffVersion::Old,
                context: 0,
                words: super::WordDiff::Markup,
                ignore_space: true,
            },
            cx,
        )
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let pane = pane.read(cx);
            pane.snapshot
                .as_ref()
                .is_some_and(|s| s.selection != old && s.selection == pane.page.selection)
        })
    });
    assert!(cx.update(|_, cx| pane.read(cx).selection.is_none()));
    assert!(cx.update(|_, cx| pane.read(cx).current.as_ref().unwrap().options.ignore_space));
}

#[gpui_kit::test]
fn word_mode_changes_preserve_the_current_file_and_scroll_position(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    for word in ["old", "new"] {
        let source: String = (0..600)
            .map(|i| format!("let value_{i} = \"{word}\";\n"))
            .collect();
        std::fs::write(fixture.root().join("long.rs"), source).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-qm", word]);
    }
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.count > 1000)
        })
    });
    pane.update(cx, |pane, cx| pane.jump(700, cx));
    until(cx, |cx| {
        cx.update(|_, cx| pane.read(cx).viewport.start >= 700)
    });
    for words in [
        super::WordDiff::Markup,
        super::WordDiff::Color,
        super::WordDiff::Lines,
    ] {
        pane.update(cx, |pane, cx| {
            pane.change_options(
                super::DiffOptions {
                    words,
                    ..pane.options
                },
                cx,
            )
        });
        until(cx, |cx| {
            cx.update(|_, cx| {
                let pane = pane.read(cx);
                pane.restore_position.is_none()
                    && pane.viewport.start > 250
                    && pane.selected_file == Some(1)
                    && pane.snapshot.as_ref().is_some_and(|s| {
                        s.request == pane.page.id && s.selection == pane.page.selection
                    })
            })
        });
    }
}

#[gpui_kit::test]
fn opening_a_non_repository_shows_one_error_and_recovers(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    let invalid = tempfile::tempdir().unwrap();
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    app.view.update(cx, |view, cx| {
        view.change_repository(invalid.path().to_owned(), cx)
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_some())
                && view
                    .workspace_snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_some())
        })
    });
    let status = cx.update(|_, cx| app.view.read(cx).status_text());
    assert_eq!(
        status.matches("not a git repository").count(),
        1,
        "{status}"
    );
    assert!(cx.update(|_, cx| app.view.read(cx).operation_message.is_empty()));
    assert_eq!(
        cx.update(|_, cx| pane.read(cx).message.clone()),
        "No commit selected."
    );
    app.view.update(cx, |view, cx| {
        view.change_repository(fixture.root().to_owned(), cx)
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_none() && s.loaded > 0)
                && view
                    .workspace_snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_none())
                && pane
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_none())
        })
    });
    assert!(
        !cx.update(|_, cx| app.view.read(cx).status_text())
            .contains("not a git repository")
    );
}

#[gpui_kit::test]
fn context_input_supports_keyboard_stepping_and_clamps_at_zero(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("diff-context-input").is_some())
    });
    cx.update(|window, cx| window.click("diff-context-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("7");
    cx.simulate_keystrokes("up");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).diff.read(cx).options.context),
        8
    );
    cx.simulate_keystrokes("down");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).diff.read(cx).options.context),
        7
    );
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("0");
    cx.simulate_keystrokes("down");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).diff.read(cx).options.context),
        0
    );
}

#[gpui_kit::test]
fn diff_options_restore_and_save_independently_of_preferences(cx: &mut TestAppContext) {
    use gitk_rs::diff::{DiffEngine, DiffSettings, options};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.toml");
    let saved = super::DiffOptions {
        context: 19,
        version: super::DiffVersion::Old,
        words: super::WordDiff::Color,
        ignore_space: true,
    };
    options::save_to(saved, &path).unwrap();
    let engine = DiffEngine::with_options_file(Some(path.clone()));
    let fixture = Fixture::new();
    fixture.commit("base");
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| {
        cx.new(|cx| {
            super::DiffPane::new(
                engine.handle(),
                DiffSettings {
                    syntax: true,
                    dark: true,
                },
                false,
                cx,
            )
        })
    });
    app.view.update(cx, |view, cx| {
        view.diff = pane.clone();
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| pane.read(cx).context_input.is_some())
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).options), saved);
    assert_eq!(
        cx.update(|_, cx| pane
            .read(cx)
            .context_input
            .as_ref()
            .unwrap()
            .read(cx)
            .value()
            .to_string()),
        "19"
    );
    let changed = super::DiffOptions {
        context: 23,
        words: super::WordDiff::Markup,
        ..saved
    };
    pane.update(cx, |pane, cx| pane.change_options(changed, cx));
    drop(engine);
    assert_eq!(options::load_from(&path).unwrap(), changed);
    assert!(!root.path().join("preferences.toml").exists());
    let permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();
    let engine = DiffEngine::with_options_file(Some(path.clone()));
    pane.update(cx, |pane, cx| {
        pane.handle = engine.handle();
        pane.change_options(
            super::DiffOptions {
                context: 31,
                ..changed
            },
            cx,
        );
    });
    drop(engine);
    assert_eq!(cx.update(|_, cx| pane.read(cx).options.context), 31);
    assert_eq!(options::load_from(&path).unwrap(), changed);
    std::fs::set_permissions(&path, permissions).unwrap();
}

#[gpui_kit::test]
fn file_context_menu_keeps_copy_after_open_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let path = "file with # and %.txt";
    std::fs::write(fixture.root().join(path), "content\n").unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "file menu"]);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .diff
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.files.len() == 1)
        })
    });
    let bounds = cx.update(|window, _| window.find(("diff-file", 1_usize)).bounds());
    cx.simulate_mouse_down(
        bounds.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        bounds.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("popup-menu").is_some())
    });
    cx.update(|window, _| {
        assert!(window.find_by_label("Open file").visible());
        assert!(window.find_by_label("Copy file path").visible());
    });
    // Open file is first; Copy file path remains the second keyboard item.
    // The headless GPUI platform does not implement open_with_system.
    cx.simulate_keystrokes("down down enter");
    until(cx, |cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref()
            == Some(path)
    });
}

#[gpui_kit::test]
fn comparison_picker_direction_and_diff_modes_work_without_checkout(cx: &mut TestAppContext) {
    use crate::ui::DialogKind;
    use gitk_rs::{diff::ComparisonMode, git::HistoryScope};
    let fixture = Fixture::new();
    fixture.commit("shared");
    fixture.git(&["checkout", "-qb", "feature"]);
    let feature = fixture.commit("feature change");
    fixture.git(&["checkout", "-q", "main"]);
    std::fs::write(fixture.root().join("main.txt"), "main only\n").unwrap();
    fixture.git(&["add", "main.txt"]);
    fixture.git(&["commit", "-qm", "main change"]);
    let main = fixture.git(&["rev-parse", "HEAD"]);
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof)
        })
    });
    cx.update(|window, cx| window.click("open-comparison", cx));
    cx.update(|window, cx| window.click("choose-comparison-target", cx));
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).pending_operation.is_none())
    });
    cx.simulate_input("feature");
    cx.simulate_keystrokes("enter");
    until(cx, |cx| {
        cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Compare)))
    });
    cx.update(|window, _| {
        let close = window.find("close-dialog").bounds();
        let compare = window.find("apply-comparison").bounds();
        assert_eq!(compare.size.width, close.size.width);
        assert_eq!(compare.size.height, close.size.height);
        assert!(compare.bottom() <= close.top());
    });
    cx.update(|window, cx| window.click("apply-comparison", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_none() && s.files.len() == 1)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(feature.as_str()));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).comparison_diff),
        Some(ComparisonMode::SinceMergeBase)
    );
    cx.update(|window, cx| window.click("comparison-endpoints", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_none() && s.files.len() == 2)
        })
    });
    cx.update(|window, cx| window.click("comparison-merge-base", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_none() && s.files.len() == 1)
        })
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).comparison_diff),
        Some(ComparisonMode::SinceMergeBase)
    );
    cx.update(|window, cx| window.click("comparison-selected-commit", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            matches!(
                pane.read(cx).current.as_ref().map(|s| &s.source),
                Some(super::DiffSource::Commit(_))
            ) && pane.read(cx).snapshot.is_some()
        })
    });
    cx.update(|window, cx| window.click("comparison-endpoints", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 2)
        })
    });
    // Reverse the comparison directly, preserving the chosen aggregate diff mode.
    cx.update(|window, cx| window.click("comparison-merge-base", cx));
    cx.update(|window, cx| window.click("swap-comparison", cx));
    until(cx, |cx| {
        selected(&app.view, cx).as_deref() == Some(main.as_str())
    });
    assert!(cx.update(|_, cx| matches!(&app.view.read(cx).history_state.request.source.scope, HistoryScope::Compare { base, target } if base == "refs/heads/feature" && target == "HEAD")));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).comparison_diff),
        Some(ComparisonMode::SinceMergeBase)
    );
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx).snapshot.as_ref().is_some_and(|s| {
                s.files.len() == 1 && s.files[0].path == std::path::Path::new("main.txt")
            })
        })
    });
    // Canceling a reference picker returns to the comparison form with edits retained.
    cx.update(|window, cx| window.click("open-comparison", cx));
    cx.update(|window, cx| window.click("choose-comparison-base", cx));
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Compare))));
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
    // Equal endpoints show an empty range but retain the valid aggregate diff.
    cx.update(|window, cx| {
        app.view.update(cx, |view, cx| {
            view.set_history_scope(
                HistoryScope::Compare {
                    base: "main".into(),
                    target: "main".into(),
                },
                window,
                cx,
            )
        })
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 0)
                && pane
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_none() && s.files.is_empty())
        })
    });
    assert!(cx.update(|_, cx| app.view.read(cx).detail.contains("no commits missing")));
    cx.update(|window, cx| window.click("exit-comparison", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.request.source.scope == HistoryScope::Current
                && view.comparison_diff.is_none()
                && view
                    .history_state
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.eof && s.loaded == 2)
                && matches!(
                    pane.read(cx).current.as_ref().map(|s| &s.source),
                    Some(super::DiffSource::Commit(_))
                )
        })
    });
    assert_eq!(fixture.git(&["branch", "--show-current"]), "main");
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), main);
}

#[gpui_kit::test]
fn commit_context_menu_compares_marked_base_and_clears_it_on_repository_change(
    cx: &mut TestAppContext,
) {
    use gitk_rs::git::HistoryScope;
    let fixture = Fixture::new();
    fixture.commit("shared root");
    let base = fixture.commit("base revision");
    let target = fixture.commit("target revision");
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof)
                && pane.read(cx).snapshot.is_some()
        })
    });
    let open_menu = |cx: &mut gpui_kit::VisualTestContext, row: usize| {
        let bounds = cx.update(|window, _| window.find(("commit", row)).bounds());
        cx.simulate_mouse_down(
            bounds.center(),
            gpui_kit::MouseButton::Right,
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            bounds.center(),
            gpui_kit::MouseButton::Right,
            Modifiers::default(),
        );
        until(cx, |cx| {
            cx.update(|window, _| window.try_find("popup-menu").is_some())
        });
    };
    // Mark a nonselected row. This must not replace the currently displayed diff.
    let selection = cx.update(|_, cx| pane.read(cx).selection_id());
    open_menu(cx, 1);
    let set_base = cx.update(|window, _| window.find_by_label("Set as comparison base").bounds());
    cx.simulate_click(set_base.center(), Modifiers::default());
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .comparison_base
                .as_ref()
                .is_some_and(|b| b.id.to_hex() == base)
        })
    });
    assert_eq!(cx.update(|_, cx| pane.read(cx).selection_id()), selection);
    cx.update(|window, cx| {
        assert!(window.find("comparison-base-indicator").visible());
        assert_eq!(
            app.view.read(cx).comparison_base.as_ref().unwrap().label(),
            format!("{} · base revision", &base[..7])
        );
    });
    // The target row's menu identifies the pinned base and compares exact commits.
    open_menu(cx, 0);
    let compare = cx.update(|window, _| {
        window
            .find_by_label(&format!("Compare with base {} · base revision", &base[..7]))
            .bounds()
    });
    cx.simulate_click(compare.center(), Modifiers::default());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.snapshot.as_ref().is_some_and(|s| {
                s.eof
                    && s.loaded == 1
                    && s.comparison
                        .as_ref()
                        .is_some_and(|e| e.base.to_hex() == base && e.target.to_hex() == target)
            }) && matches!(
                pane.read(cx).current.as_ref().map(|s| &s.source),
                Some(super::DiffSource::Comparison {
                    mode: gitk_rs::diff::ComparisonMode::Endpoints,
                    ..
                })
            ) && pane
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.error.is_none())
        })
    });
    // Clearing the marker affects future comparisons, keeping this comparison usable.
    cx.update(|window, cx| window.click("clear-comparison-base", cx));
    assert!(cx.update(|_, cx| app.view.read(cx).comparison_base.is_none()));
    assert!(cx.update(|_, cx| matches!(
        app.view.read(cx).history_state.request.source.scope,
        HistoryScope::Compare { .. }
    )));
    cx.update(|window, cx| window.click("exit-comparison", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 3)
        })
    });
    open_menu(cx, 1);
    cx.simulate_keystrokes("down down enter");
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).comparison_base.is_some())
    });
    let other = Fixture::new();
    let other_head = other.commit("another repository");
    app.view.update(cx, |view, cx| {
        view.change_repository(other.root().into(), cx)
    });
    until(cx, |cx| {
        selected(&app.view, cx).as_deref() == Some(other_head.as_str())
    });
    assert!(cx.update(|_, cx| app.view.read(cx).comparison_base.is_none()));
    assert_eq!(fixture.git(&["branch", "--show-current"]), "main");
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), target);
    assert!(fixture.git(&["status", "--porcelain=v1"]).is_empty());
}

#[gpui_kit::test]
fn long_comparison_base_subject_stays_inside_the_context_menu(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let subject = "Very long base subject with Unicode 日本語 λ ".repeat(12);
    let base = fixture.commit(&subject);
    let target = fixture.commit("comparison target");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof)
        })
    });
    app.view.update(cx, |view, cx| {
        view.comparison_base = Some(crate::ui::comparison_dialog::ComparisonBase {
            id: gitk_rs::git::CommitId::parse(&base).unwrap(),
            subject: subject.clone(),
        });
        cx.notify();
    });
    cx.simulate_resize(gpui_kit::size(px(680.), px(650.)));
    let row = cx.update(|window, _| window.find(("commit", 0_usize)).bounds());
    cx.simulate_mouse_down(
        row.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        row.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("popup-menu").is_some())
    });
    cx.update(|window, _| {
        let menu = window.find("popup-menu").bounds();
        let label = window.find("compare-with-base-menu-label").bounds();
        assert!(label.left() >= menu.left());
        assert!(label.right() <= menu.right());
        assert!(menu.left() >= px(0.));
        assert!(menu.right() <= window.viewport_size().width);
        assert!(menu.size.width <= px(480.));
    });
    cx.simulate_keystrokes("down down down enter");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| {
                    s.eof
                        && s.comparison
                            .as_ref()
                            .is_some_and(|e| e.base.to_hex() == base && e.target.to_hex() == target)
                })
        })
    });
}

fn open_file_history_from_menu(cx: &mut gpui_kit::VisualTestContext, index: usize) {
    let bounds = cx.update(|window, _| window.find(("diff-file", index)).bounds());
    cx.simulate_mouse_down(
        bounds.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        bounds.center(),
        gpui_kit::MouseButton::Right,
        Modifiers::default(),
    );
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("popup-menu").is_some())
    });
    let action = cx.update(|window, _| window.find_by_label("Open file history").bounds());
    cx.simulate_click(action.center(), Modifiers::default());
}

#[gpui_kit::test]
fn changed_file_history_follows_old_names_and_returns_to_previous_scope(cx: &mut TestAppContext) {
    use gitk_rs::git::{CommitId, HistoryScope};
    let fixture = Fixture::new();
    let old = "z old [1].rs";
    let new = "z new [1].rs";
    let source: String = (0..180)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect();
    std::fs::write(fixture.root().join(old), &source).unwrap();
    std::fs::write(fixture.root().join("a unrelated.rs"), "first\n").unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "create file"]);
    let root = fixture.git(&["rev-parse", "HEAD"]);
    std::fs::write(fixture.root().join(old), format!("{source}// old edit\n")).unwrap();
    fixture.git(&["commit", "-qam", "edit old name"]);
    fixture.commit("unrelated change");
    fixture.git(&["mv", "--", old, new]);
    fixture.git(&["commit", "-qm", "rename"]);
    std::fs::write(fixture.root().join(new), format!("{source}// new edit\n")).unwrap();
    std::fs::write(fixture.root().join("a unrelated.rs"), "second\n").unwrap();
    fixture.git(&["commit", "-qam", "edit new name"]);
    let head = fixture.git(&["rev-parse", "HEAD"]);
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    cx.update(|window, cx| {
        app.view.update(cx, |view, cx| {
            view.set_history_scope(HistoryScope::AllBranches, window, cx)
        })
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 5)
                && pane
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.files.len() == 2)
        })
    });
    open_file_history_from_menu(cx, 2);
    until(cx, |cx| {
        cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(&view.history_state.request.source.scope, HistoryScope::File(h) if h.path == std::path::Path::new(new) && h.revision.as_ref().is_some_and(|id| id.to_hex() == head) && !h.follow_renames)
            && view.history_state.snapshot.as_ref().is_some_and(|s| s.eof && s.loaded == 2)
            && pane.read(cx).selected_file == Some(2)
    })
    });
    cx.update(|window, _| {
        let file = window.find("file-history-label").bounds();
        let follow = window.find("follow-file-renames").bounds();
        let exit = window.find("exit-file-history").bounds();
        let row = window.find("file-history-controls").bounds();
        assert!(file.top() >= window.find("history-scope").bounds().bottom());
        assert!(file.right() <= follow.left());
        assert!((file.center().y - exit.center().y).abs() < px(1.));
        assert!((follow.center().y - exit.center().y).abs() < px(1.));
        assert!((exit.right() - row.right()).abs() < px(1.));
    });
    cx.update(|window, cx| window.click("follow-file-renames", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 4)
        })
    });
    cx.update(|window, cx| app.view.update(cx, |view, cx| view.select(3, window, cx)));
    until(cx, |cx| {
        cx.update(|_, cx| {
        let pane = pane.read(cx);
        matches!(&pane.identity, Some(super::DiffIdentity::File(_, id, path)) if id.to_hex() == root && path.as_ref() == std::path::Path::new(old))
            && pane.snapshot.as_ref().is_some_and(|s| pane.selected_file.is_some_and(|index| index > 0 && s.files[index - 1].path == std::path::Path::new(old)))
            && pane.viewport.start > 0
    })
    });
    cx.update(|window, cx| window.click("exit-file-history", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.history_state.request.source.scope == HistoryScope::AllBranches
                && view
                    .history_state
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.eof && s.loaded == 5)
                && pane.read(cx).snapshot.is_some()
        })
    });
    let stale = cx.update(|_, cx| super::OpenFileHistory {
        selection: pane.read(cx).selection_id(),
        repository: fixture.root().into(),
        revision: Some(CommitId::parse(&head).unwrap()),
        path: new.into(),
    });
    cx.update(|window, cx| app.view.update(cx, |view, cx| view.select(4, window, cx)));
    until(cx, |cx| {
        cx.update(|_, cx| matches!(pane.read(cx).current.as_ref().map(|s| &s.source), Some(super::DiffSource::Commit(commit)) if commit.id.to_hex() == root) && pane.read(cx).snapshot.is_some())
    });
    cx.update(|window, cx| {
        app.view
            .update(cx, |view, cx| view.open_file_history(&stale, window, cx))
    });
    assert!(
        cx.update(|_, cx| app.view.read(cx).history_state.request.source.scope
            == HistoryScope::AllBranches)
    );
    open_file_history_from_menu(cx, 2);
    until(cx, |cx| {
        cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(&view.history_state.request.source.scope, HistoryScope::File(h) if h.revision.as_ref().is_some_and(|id| id.to_hex() == root) && h.path == std::path::Path::new(old))
            && view.history_state.snapshot.as_ref().is_some_and(|s| s.eof && s.loaded == 1)
    })
    });
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), head);
    assert!(fixture.git(&["status", "--porcelain=v1"]).is_empty());
}

#[gpui_kit::test]
fn untracked_file_history_in_an_unborn_repository_returns_to_local_changes(
    cx: &mut TestAppContext,
) {
    use gitk_rs::git::HistoryScope;
    let fixture = Fixture::new();
    std::fs::write(fixture.root().join("new.rs"), "let value = 1;\n").unwrap();
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).history_state.local_rows.is_empty())
    });
    cx.simulate_keystrokes("home");
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 1)
        })
    });
    open_file_history_from_menu(cx, 1);
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            matches!(
                view.history_state.request.source.scope,
                HistoryScope::File(_)
            ) && view
                .history_state
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 0 && s.error.is_none())
                && view.detail.contains("No commits found for this file")
        })
    });
    cx.update(|window, cx| window.click("exit-file-history", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            matches!(
                pane.read(cx).current.as_ref().map(|s| &s.source),
                Some(super::DiffSource::Local { .. })
            ) && pane
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 1)
        })
    });
    assert_eq!(
        std::fs::read_to_string(fixture.root().join("new.rs")).unwrap(),
        "let value = 1;\n"
    );
}

#[gpui_kit::test]
fn file_history_from_aggregate_diff_uses_target_and_restores_comparison(cx: &mut TestAppContext) {
    use gitk_rs::{diff::ComparisonMode, git::HistoryScope};
    let fixture = Fixture::new();
    fixture.commit("root");
    fixture.git(&["checkout", "-qb", "feature"]);
    let feature = fixture.commit("feature change");
    fixture.git(&["checkout", "-q", "main"]);
    std::fs::write(fixture.root().join("other.txt"), "main only").unwrap();
    fixture.git(&["add", "."]);
    fixture.git(&["commit", "-qm", "main change"]);
    let (app, cx) = open(&fixture, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    cx.update(|window, cx| {
        app.view.update(cx, |view, cx| {
            view.set_history_scope(
                HistoryScope::Compare {
                    base: "main".into(),
                    target: "feature".into(),
                },
                window,
                cx,
            )
        })
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 2)
        })
    });
    cx.update(|window, cx| window.click("comparison-merge-base", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.files.len() == 1)
        })
    });
    open_file_history_from_menu(cx, 1);
    until(cx, |cx| {
        cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(&view.history_state.request.source.scope, HistoryScope::File(h) if h.revision.as_ref().is_some_and(|id| id.to_hex() == feature))
            && view.history_state.snapshot.as_ref().is_some_and(|s| s.eof && s.loaded == 2)
    })
    });
    cx.update(|window, cx| window.click("exit-file-history", cx));
    until(cx, |cx| {
        cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(&view.history_state.request.source.scope, HistoryScope::Compare { base, target } if base == "main" && target == "feature")
            && view.comparison_diff == Some(ComparisonMode::SinceMergeBase)
            && matches!(pane.read(cx).current.as_ref().map(|s| &s.source), Some(super::DiffSource::Comparison { endpoints, .. }) if endpoints.target.to_hex() == feature)
            && pane.read(cx).snapshot.as_ref().is_some_and(|s| s.files.len() == 1)
    })
    });
}

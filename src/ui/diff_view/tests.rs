use crate::ui::tests::{Fixture, open, selected, until};
use gpui_kit::{Modifiers, TestAppContext, point, px, test::TestWindowExt};

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
        cx.update(|_, cx| !app.view.read(cx).local_rows.is_empty())
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
        cx.update(|_, cx| !app.view.read(cx).local_rows.is_empty())
    });
    cx.simulate_keystrokes("home");
    assert!(cx.update(|_, cx| app.view.read(cx).local_selection == Some(false)));
    std::fs::write(fixture.root().join("file.txt"), "base").unwrap();
    app.view.update(cx, |view, _| view.refresh_workspace());
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.local_rows.is_empty()
                && view
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
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    cx.simulate_keystrokes("f5");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.request.generation != generation && view.reload_selection.is_none()
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
            view.local_rows == [false, true] && view.selected.is_some()
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
            view.snapshot.as_ref().is_some_and(|s| s.error.is_some())
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
            view.snapshot
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

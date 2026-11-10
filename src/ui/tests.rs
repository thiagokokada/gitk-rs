use super::*;
use gpui_kit::{InputEvent as _, TestAppContext, VisualTestContext, test::TestWindowExt};
#[path = "../test_support.rs"]
mod test_support;
pub(super) use test_support::Fixture;

pub(super) struct Harness {
    pub view: Entity<HistoryWindow>,
    _history: HistoryEngine,
    _workspace: WorkspaceEngine,
    _diff: DiffEngine,
    _preferences_directory: tempfile::TempDir,
}

pub(super) fn open<'a>(
    fixture: &Fixture,
    cx: &'a mut TestAppContext,
) -> (Harness, &'a mut VisualTestContext) {
    // Preserve partial-loading fixtures for scrolling/restoration tests.
    open_with_batch(fixture, Some(1000), cx)
}

fn open_with_batch<'a>(
    fixture: &Fixture,
    batch: Option<usize>,
    cx: &'a mut TestAppContext,
) -> (Harness, &'a mut VisualTestContext) {
    open_with_state(fixture, batch, None, cx)
}

fn open_with_state<'a>(
    fixture: &Fixture,
    batch: Option<usize>,
    state_path: Option<std::path::PathBuf>,
    cx: &'a mut TestAppContext,
) -> (Harness, &'a mut VisualTestContext) {
    let config = Config {
        repository: fixture.root().into(),
        watch: false,
        batch,
        ..Config::default()
    };
    let request = Request {
        generation: Generation(1),
        id: 0,
        refs_revision: 0,
        repository: config.repository.clone(),
        range: 0..128,
        query: String::new(),
        regex: false,
        batch: config.batch,
        graph_columns: config.graph_columns,
        anchor: None,
        restore: None,
    };
    let history = HistoryEngine::spawn(request.clone());
    let workspace = WorkspaceEngine::spawn(WorkspaceRequest {
        generation: WorkspaceGeneration(1),
        repository: config.repository.clone(),
        watch: false,
    });
    let restoring_layout = state_path.is_some();
    let diff = DiffEngine::with_options_file(state_path);
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(ThemeMode::Dark, None, cx);
        bind_keys(cx);
    });
    let mut view = None;
    let preferences_directory = tempfile::tempdir().unwrap();
    let preferences_path = preferences_directory.path().join("preferences.toml");
    let build = |window: &mut Window, cx: &mut Context<gpui_kit::base::Root>| {
        let entity = cx.new(|cx| {
            let preferences = Preferences::default().with_config(&config);
            let mut view = HistoryWindow::new(
                config,
                TimingGuard::start(false, Instant::now()).timing(),
                WindowServices {
                    history: history.handle(),
                    workspace: workspace.handle(),
                    diff: diff.handle(),
                },
                preferences,
                request,
                window,
                cx,
            );
            // Even a failing validation regression must never write user settings.
            view.preferences_path = Some(preferences_path.clone());
            view
        });
        view = Some(entity.clone());
        gpui_kit::base::Root::new(entity, window, cx)
    };
    let cx = if restoring_layout {
        // Start at the target size: resizing an already rendered maximized
        // window legitimately scales GPUI's split panes proportionally.
        let handle = cx.open_window(size(px(1280.), px(850.)), build);
        VisualTestContext::from_window(handle.into(), cx).into_mut()
    } else {
        cx.add_window_view(build).1
    };
    cx.simulate_resize(size(px(1280.), px(850.)));
    (
        Harness {
            view: view.unwrap(),
            _history: history,
            _workspace: workspace,
            _diff: diff,
            _preferences_directory: preferences_directory,
        },
        cx,
    )
}

// Git workers use real OS threads; GPUI tasks and debounce timers use its test clock.
// Bound the external wait and advance the clock rather than sleeping for UI timers.
#[track_caller]
pub(super) fn until(
    cx: &mut VisualTestContext,
    mut ready: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "UI condition did not become ready"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn selected(view: &Entity<HistoryWindow>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .selected
            .as_ref()
            .map(|(_, entry)| entry.commit.id.to_hex())
    })
}

fn scroll_preferences_to(id: &'static str, cx: &mut VisualTestContext) {
    cx.refresh().unwrap();
    cx.update(|window, cx| {
        let form = window.find("preferences-scroll-viewport").bounds();
        let target = window.find(id).bounds();
        window.scroll(
            "preferences-scroll-viewport",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), form.center().y - target.center().y)),
            cx,
        );
    });
    cx.refresh().unwrap();
}

fn history_fixture(count: usize) -> Fixture {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let fixture = Fixture::new();
    let mut input = String::new();
    for i in 1..=count {
        let message = format!("commit {i}");
        input.push_str(&format!("commit refs/heads/main\nmark :{i}\ncommitter Fixture <fixture@example.test> {} +0000\ndata {}\n{message}\n", 1700000000 + i, message.len()));
        if i > 1 {
            input.push_str(&format!("from :{}\n", i - 1));
        }
        input.push('\n');
    }
    let mut child = Command::new("git")
        .arg("-C")
        .arg(fixture.root())
        .args(["fast-import", "--quiet"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fixture
}

#[gpui_kit::test]
fn default_streaming_loads_history_and_keeps_navigation_and_search_working(
    cx: &mut TestAppContext,
) {
    let fixture = history_fixture(2500);
    let tip = fixture.git(&["rev-parse", "HEAD"]);
    let (app, cx) = open_with_batch(&fixture, Config::default().batch, cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().is_some_and(|s| s.eof))
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().unwrap().loaded),
        2500
    );
    assert_eq!(selected(&app.view, cx).as_deref(), Some(tip.as_str()));
    cx.simulate_keystrokes("alt-j");
    assert_ne!(selected(&app.view, cx).as_deref(), Some(tip.as_str()));
    cx.simulate_keystrokes("alt-k");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(tip.as_str()));
    cx.simulate_keystrokes("/");
    cx.simulate_input("commit 1234");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.visible == 1 && s.rows[0].commit.summary() == "commit 1234")
        })
    });
}

#[gpui_kit::test]
fn history_pages_settle_after_scrolling_beyond_the_first_page(cx: &mut TestAppContext) {
    let fixture = history_fixture(2500);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded == 1000)
        })
    });
    let scrollbar = cx.update(|window, _| window.find("history-scrollbar-viewport").bounds());
    assert!(scrollbar.size.height > px(50.));
    cx.simulate_click(
        point(scrollbar.right() - px(8.), scrollbar.bottom() - px(2.)),
        gpui_kit::Modifiers::default(),
    );
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).viewport.start > 500)
    });
    for index in [800, 1600, 100] {
        app.view.update(cx, |view, cx| {
            view.scroll
                .scroll_to_item_strict(index, ScrollStrategy::Top);
            cx.notify();
        });
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                view.viewport.contains(&index)
                    && view.snapshot.as_ref().is_some_and(|s| {
                        s.request_id == view.request.id
                            && s.start <= view.viewport.start
                            && s.start + s.rows.len() >= view.viewport.end
                    })
            })
        });
        let request = cx.update(|_, cx| app.view.read(cx).request.id);
        for _ in 0..12 {
            cx.background_executor
                .advance_clock(Duration::from_millis(20));
            cx.run_until_parked();
            cx.refresh().unwrap();
            cx.run_until_parked();
            assert_eq!(cx.update(|_, cx| app.view.read(cx).request.id), request);
        }
    }
}

#[gpui_kit::test]
fn unchanged_head_reload_retains_rows_and_selection(cx: &mut TestAppContext) {
    let fixture = history_fixture(20);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().is_some_and(|s| s.eof))
    });
    app.view
        .update_in(cx, |view, window, cx| view.select(8, window, cx));
    let (before, generation) = cx.update(|_, cx| {
        let view = app.view.read(cx);
        (view.cached(8).unwrap(), view.request.generation)
    });
    cx.simulate_keystrokes("f5");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.snapshot
                .as_ref()
                .is_some_and(|s| s.generation != generation)
                && view.reload_selection.is_none()
        })
    });
    cx.update(|_, cx| {
        let view = app.view.read(cx);
        assert!(Arc::ptr_eq(&before, &view.cached(8).unwrap()));
        assert_eq!(
            view.selected.as_ref().unwrap().1.commit.id,
            before.commit.id
        );
    });
}

#[gpui_kit::test]
fn consecutive_reload_before_layout_preserves_the_restored_top_commit(cx: &mut TestAppContext) {
    let fixture = history_fixture(100);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).cached(50).is_some())
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(50, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).viewport.start == 50)
    });
    let top = cx.update(|_, cx| app.view.read(cx).cached(50).unwrap().commit.id.clone());
    fixture.append_history(600, false, "new tip");
    app.view.update(cx, |view, cx| view.reload(cx));
    // Receive the restored rows without running layout or workspace polling.
    // This reproduces a second HEAD update arriving before the next frame.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if app.view.update(cx, |view, cx| {
            view.consume_snapshot(cx);
            view.request.restore.is_none()
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "restore did not complete");
        std::thread::sleep(Duration::from_millis(10));
    }
    app.view.update(cx, |view, cx| {
        view.reload(cx);
        assert_eq!(view.request.restore.as_ref().unwrap().top, Some(top));
    });
}

#[gpui_kit::test]
fn deep_reload_keeps_selected_commit_and_top_visible_commit(cx: &mut TestAppContext) {
    let fixture = history_fixture(2500);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded >= 1000)
        })
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(800, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded >= 2000)
        })
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(1600, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).cached(1600).is_some())
    });
    app.view
        .update_in(cx, |view, window, cx| view.select(1620, window, cx));
    let chosen = selected(&app.view, cx).unwrap();
    let author = cx.update(|_, cx| {
        app.view
            .read(cx)
            .selected
            .as_ref()
            .unwrap()
            .1
            .commit
            .author
            .name
            .clone()
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(1800, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view.read(cx).viewport.start == 1800 && app.view.read(cx).cached(1800).is_some()
        })
    });
    let top = cx.update(|_, cx| app.view.read(cx).cached(1800).unwrap().commit.id.clone());
    fixture.commit("new tip");
    cx.simulate_keystrokes("f5");
    // Restoring the snapshot schedules a scroll; wait for layout to apply it.
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.reload_selection.is_none()
                && view
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.generation == view.request.generation)
                && view
                    .cached(view.viewport.start)
                    .is_some_and(|entry| entry.commit.id == top)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
    cx.update(|_, cx| {
        assert!(Arc::ptr_eq(
            &author,
            &app.view
                .read(cx)
                .selected
                .as_ref()
                .unwrap()
                .1
                .commit
                .author
                .name
        ));
    });
    assert_eq!(
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(view.viewport.start)
                .map(|e| e.commit.id.clone())
        }),
        Some(top.clone())
    );
    // Branch switching and watcher-driven HEAD changes use the same bookmarks.
    fixture.git(&["checkout", "-qb", "side"]);
    fixture.commit("side tip");
    fixture.git(&["checkout", "-q", "main"]);
    cx.simulate_keystrokes("ctrl-b");
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).branches.is_empty())
    });
    cx.simulate_input("side");
    cx.simulate_keystrokes("enter");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.reload_selection.is_none()
                && view.workspace_snapshot.is_some()
                && view.snapshot.as_ref().is_some_and(|s| s.branch == "side")
                && view
                    .cached(view.viewport.start)
                    .is_some_and(|entry| entry.commit.id == top)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
    assert_eq!(
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(view.viewport.start).unwrap().commit.id.clone()
        }),
        top
    );
    let top = cx.update(|_, cx| {
        let view = app.view.read(cx);
        view.cached(view.viewport.start).unwrap().commit.id.clone()
    });
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    let revision = cx.update(|_, cx| {
        app.view
            .read(cx)
            .workspace_snapshot
            .as_ref()
            .unwrap()
            .revision
    });
    app.view.update(cx, |view, _| {
        view.config.watch = true;
        view.refresh_workspace();
    });
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
    fixture.append_history(600, false, "automatic tip");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.request.generation != generation
                && view.reload_selection.is_none()
                && view
                    .cached(view.viewport.start)
                    .is_some_and(|entry| entry.commit.id == top)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
    cx.update(|_, cx| {
        assert!(Arc::ptr_eq(
            &author,
            &app.view
                .read(cx)
                .selected
                .as_ref()
                .unwrap()
                .1
                .commit
                .author
                .name
        ));
    });
    assert_eq!(
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(view.viewport.start).unwrap().commit.id.clone()
        }),
        top
    );
}

#[gpui_kit::test]
fn filtering_and_new_local_rows_preserve_commit_identity(cx: &mut TestAppContext) {
    let fixture = history_fixture(2500);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded >= 1000)
        })
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(800, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).cached(850).is_some())
    });
    app.view
        .update_in(cx, |view, window, cx| view.select(850, window, cx));
    let chosen = selected(&app.view, cx).unwrap();
    cx.simulate_keystrokes("/");
    cx.simulate_input("commit 16");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            !view.preserve_filter_selection
                && view.snapshot.as_ref().is_some_and(|s| s.visible < 200)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
    cx.simulate_keystrokes("escape");
    let top = cx.update(|_, cx| {
        let view = app.view.read(cx);
        view.cached(view.viewport.start).unwrap().commit.id.clone()
    });
    std::fs::write(fixture.root().join("untracked"), "local").unwrap();
    app.view.update(cx, |view, _| view.refresh_workspace());
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).local_rows.len() == 1)
    });
    assert_eq!(
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(view.viewport.start).unwrap().commit.id.clone()
        }),
        top
    );
    cx.simulate_keystrokes("/ ctrl-a ctrl-k escape");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.request.query.is_empty()
                && !view.preserve_filter_selection
                && view
                    .selected
                    .as_ref()
                    .is_some_and(|s| view.cached(s.0).is_some())
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
    cx.update(|_, cx| {
        let view = app.view.read(cx);
        let selected = view.selected.as_ref().unwrap();
        assert_eq!(selected.0, selected.1.ordinal + 1);
    });
}

#[gpui_kit::test]
fn status_messages_keep_one_line_at_narrow_widths(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    let height = cx.update(|window, _| window.find("history-status").bounds().size.height);
    for message in [
        "branch switched".to_owned(),
        "preferences saved".to_owned(),
        "Git failed: /CaseSensitive/Repository.git".to_owned(),
        "Long operation message ".repeat(30),
    ] {
        cx.update(|_, cx| {
            app.view.update(cx, |view, cx| {
                view.operation_message = message.clone();
                cx.notify();
            });
        });
        cx.update(|_, cx| {
            let status = app.view.read(cx).status_text();
            assert!(status.starts_with("showing "));
            assert!(status.ends_with(&message));
        });
        cx.simulate_resize(size(px(740.), px(850.)));
        cx.refresh().unwrap();
        assert_eq!(
            cx.update(|window, _| window.find("history-status").bounds().size.height),
            height,
        );
    }
}

#[gpui_kit::test]
fn graph_column_can_be_resized_without_changing_selection(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    let divider = cx.update(|window, _| window.find("graph-divider").bounds());
    let origin = divider.center();
    cx.simulate_mouse_down(
        origin,
        gpui_kit::MouseButton::Left,
        gpui_kit::Modifiers::default(),
    );
    cx.simulate_mouse_move(
        origin + point(px(140.), px(0.)),
        Some(gpui_kit::MouseButton::Left),
        gpui_kit::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        origin + point(px(140.), px(0.)),
        gpui_kit::MouseButton::Left,
        gpui_kit::Modifiers::default(),
    );
    assert_eq!(cx.update(|_, cx| app.view.read(cx).graph_width), 420.);
    assert_eq!(selected(&app.view, cx).as_deref(), Some(tip.as_str()));
    assert!(
        cx.update(|window, _| window.find("graph-divider").bounds())
            .center()
            .x
            > origin.x
    );
}

#[gpui_kit::test]
fn preferences_toggle_watching_and_refresh_history(cx: &mut TestAppContext) {
    let settings = tempfile::tempdir().unwrap();
    let fixture = Fixture::new();
    let head = fixture.commit("base");
    let (app, cx) = open(&fixture, cx);
    app.view.update(cx, |view, _| {
        view.preferences_path = Some(settings.path().join("preferences.toml"));
    });
    until(cx, |cx| {
        selected(&app.view, cx).as_deref() == Some(head.as_str())
    });
    for enabled in [true, false, true] {
        cx.simulate_keystrokes("ctrl-,");
        scroll_preferences_to("watch-setting", cx);
        cx.update(|window, cx| window.click("watch-setting", cx));
        cx.update(|window, cx| window.click("apply-preferences", cx));
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                view.config.watch == enabled
                    && !view.preferences_save_pending
                    && view.reload_selection.is_none()
            })
        });
        assert_eq!(selected(&app.view, cx).as_deref(), Some(head.as_str()));
        if enabled {
            std::fs::write(fixture.root().join("file.txt"), "changed").unwrap();
            until(cx, |cx| {
                cx.update(|_, cx| !app.view.read(cx).local_rows.is_empty())
            });
        }
    }
}

#[gpui_kit::test]
fn keyboard_navigation_and_filter_focus(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let older = fixture.commit("older needle");
    let newer = fixture.commit("newer tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&newer));
    cx.simulate_keystrokes("j");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(older.as_str()));
    cx.simulate_keystrokes("/");
    cx.simulate_input("NEEDLE");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.visible == 1)
        })
    });
    cx.simulate_keystrokes("ctrl-b");
    assert!(
        cx.update(|_, cx| app.view.read(cx).dialog.is_none()),
        "input editing must not open the branch dialog"
    );
    cx.simulate_keystrokes("ctrl-a ctrl-k escape");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.visible == 2)
        })
    });
    cx.simulate_keystrokes("home");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(newer.as_str()));
}

#[gpui_kit::test]
fn branch_protection_and_selected_ancestor_survive_switch(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let ancestor = fixture.commit("ancestor");
    fixture.git(&["checkout", "-qb", "side"]);
    let tip = fixture.commit("side tip");
    fixture.git(&["checkout", "-q", "main"]);
    std::fs::write(fixture.root().join("file.txt"), "unsaved").unwrap();
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        selected(&app.view, cx).as_deref() == Some(&ancestor)
    });
    cx.simulate_keystrokes("ctrl-b");
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).branches.is_empty())
    });
    cx.simulate_input("side");
    cx.simulate_keystrokes("enter");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let v = app.view.read(cx);
            v.pending_operation.is_none() && !v.operation_message.is_empty()
        })
    });
    assert_eq!(fixture.git(&["branch", "--show-current"]), "main");
    let error = cx.update(|_, cx| app.view.read(cx).operation_message.clone());
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).operation_message.clone()),
        error
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root().join("file.txt")).unwrap(),
        "unsaved"
    );
    cx.simulate_keystrokes("escape");
    std::fs::write(fixture.root().join("file.txt"), "ancestor").unwrap();
    cx.simulate_keystrokes("ctrl-b");
    until(cx, |cx| {
        cx.update(|_, cx| {
            !app.view.read(cx).branches.is_empty() && app.view.read(cx).pending_operation.is_none()
        })
    });
    cx.simulate_input("side");
    cx.simulate_keystrokes("enter");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let v = app.view.read(cx);
            v.dialog.is_none()
                && v.snapshot.as_ref().is_some_and(|s| s.branch == "side")
                && v.reload_selection.is_none()
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(ancestor.as_str()));
    assert!(cx.update(|_, cx| app.view.read(cx).status_text().ends_with("branch switched")));
    cx.simulate_keystrokes("home");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(tip.as_str()));
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| app.view.read(cx).operation_message.is_empty()));
}

#[gpui_kit::test]
fn end_of_history_notice_expires_without_changing_eof_or_repeating(cx: &mut TestAppContext) {
    let fixture = history_fixture(20);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).status_text().contains("end of history"))
    });
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.simulate_keystrokes("f5");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.end_of_history_generation != Some(generation)
                && view.status_text().contains("end of history")
        })
    });
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| app.view.read(cx).status_text().contains("end of history")));
    cx.background_executor.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    app.view.update(cx, |view, _| {
        assert!(view.snapshot.as_ref().unwrap().eof);
        assert!(!view.status_text().contains("end of history"));
        assert!(!view.status_text().contains("more available"));
        view.submit();
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.snapshot
                .as_ref()
                .is_some_and(|s| s.request_id == view.request.id)
        })
    });
    assert!(cx.update(|_, cx| !app.view.read(cx).status_text().contains("end of history")));
}

#[gpui_kit::test]
fn branch_switch_message_timer_preserves_newer_operations(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("root");
    fixture.git(&["branch", "side"]);
    let (app, cx) = open(&fixture, cx);
    for branch in ["side", "main"] {
        cx.simulate_keystrokes("ctrl-b");
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                !view.branches.is_empty() && view.pending_operation.is_none()
            })
        });
        cx.simulate_input(branch);
        cx.simulate_keystrokes("enter");
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                view.dialog.is_none()
                    && view.operation_message == "branch switched"
                    && view.pending_operation.is_none()
            })
        });
        if branch == "side" {
            cx.background_executor.advance_clock(Duration::from_secs(2));
            cx.run_until_parked();
        }
    }
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| app.view.read(cx).operation_message == "branch switched"));
    app.view.update(cx, |view, _| {
        view.operation_message = "newer operation failed".into()
    });
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| app.view.read(cx).operation_message == "newer operation failed"));
}

#[gpui_kit::test]
fn repository_path_dialog_replaces_history_and_restores_navigation(cx: &mut TestAppContext) {
    let first = Fixture::new();
    let old = first.commit("old repository");
    let second = Fixture::new();
    let base = second.commit("new base");
    let tip = second.commit("new tip");
    let (app, cx) = open(&first, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&old));
    cx.simulate_keystrokes("ctrl-shift-o ctrl-a ctrl-k");
    cx.simulate_keystrokes("enter");
    assert!(
        cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::OpenRepository)))
    );
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|_, cx| {
        app.view
            .read(cx)
            .status_text()
            .ends_with("enter a repository directory")
    }));
    cx.simulate_keystrokes("ctrl-shift-o ctrl-a ctrl-k");
    cx.simulate_input(second.root().to_str().unwrap());
    cx.simulate_keystrokes("enter");
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
    cx.simulate_keystrokes("j");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(base.as_str()));
}

#[gpui_kit::test]
fn workspace_changes_before_layout_preserve_the_top_commit(cx: &mut TestAppContext) {
    let fixture = history_fixture(100);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(60).is_some() && view.workspace_snapshot.is_some()
        })
    });
    app.view
        .update_in(cx, |view, window, cx| view.select(55, window, cx));
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(50, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).viewport.start == 50)
    });
    let top = cx.update(|_, cx| app.view.read(cx).cached(50).unwrap().commit.id.clone());
    let chosen = selected(&app.view, cx).unwrap();
    std::fs::write(fixture.root().join("transient.txt"), "not committed yet").unwrap();
    app.view.update(cx, |view, _| view.refresh_workspace());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if app.view.update(cx, |view, cx| {
            view.consume_workspace(cx);
            if view.local_rows.len() == 1 {
                // Assert before this update returns and GPUI can run layout.
                assert_eq!(view.cached(view.viewport.start).unwrap().commit.id, top);
                true
            } else {
                false
            }
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "workspace did not observe local row"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Layout must preserve the same bookmark before the next HEAD update.
    assert_eq!(
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.cached(view.viewport.start).unwrap().commit.id.clone()
        }),
        top
    );
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    fixture.commit("committed transient row");
    app.view.update(cx, |view, _| view.refresh_workspace());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if app.view.update(cx, |view, cx| {
            view.consume_workspace(cx);
            view.request.generation != generation
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "workspace did not observe HEAD change"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.reload_selection.is_none()
                && view
                    .cached(view.viewport.start)
                    .is_some_and(|entry| entry.commit.id == top)
        })
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(chosen.as_str()));
}

#[gpui_kit::test]
fn shortcuts_are_grouped_scrollable_and_restore_focus(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let older = fixture.commit("older");
    let newer = fixture.commit("newer");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&newer));

    for theme in [Theme::Dark, Theme::Light] {
        app.view.update(cx, |view, cx| {
            view.config.theme = theme;
            view.preferences.font_size = 24.;
            cx.notify();
        });
        cx.simulate_resize(size(px(800.), px(600.)));
        cx.simulate_keystrokes("f1");
        cx.refresh().unwrap();
        cx.update(|window, _| {
            let commits = window.find("Commit list").bounds();
            let diff = window.find("Diff view").bounds();
            assert!(commits.size.height > px(0.));
            assert!(
                diff.origin.y > commits.bottom(),
                "groups need visible spacing"
            );
            let viewport = window.find("shortcut-groups").bounds();
            let close = window.find("close-dialog");
            assert!(close.visible());
            assert!(close.bounds().origin.y >= viewport.bottom());
            assert!(close.bounds().bottom() <= window.bounds().size.height);
            assert!(
                !window
                    .find(SharedString::from(
                        gitk_rs::search::Qualifier::Merge.syntax()
                    ))
                    .visible()
            );
        });
        let end = cx.update(|_, cx| -app.view.read(cx).help_scroll.max_offset().y);
        cx.update(|window, cx| {
            window.scroll(
                "shortcut-groups",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), end)),
                cx,
            );
        });
        cx.refresh().unwrap();
        cx.update(|window, _| {
            assert!(
                window
                    .find(SharedString::from(
                        gitk_rs::search::Qualifier::Merge.syntax()
                    ))
                    .visible(),
                "last help entry must be reachable"
            );
            assert!(window.find("close-dialog").visible());
        });
        cx.simulate_keystrokes("escape");
        assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
    }
    cx.simulate_keystrokes("j");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(older.as_str()));
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn command_aliases_copy_commits_and_page_history(cx: &mut TestAppContext) {
    let fixture = history_fixture(300);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded == 300)
                && app.view.read(cx).selected.is_some()
        })
    });
    for modifier in ["ctrl", "cmd"] {
        let commit = selected(&app.view, cx).unwrap();
        cx.update(|_, cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into()))
        });
        cx.simulate_keystrokes(&format!("{modifier}-c"));
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .starts_with(&commit)
        );
        cx.simulate_keystrokes(&format!("{modifier}-pagedown"));
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).viewport.start > 0)
        });
        assert_eq!(selected(&app.view, cx).as_deref(), Some(commit.as_str()));
        cx.simulate_keystrokes(&format!("{modifier}-pageup"));
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).viewport.start == 0)
        });
    }
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn command_aliases_open_repositories_and_respect_input_and_dialog_focus(cx: &mut TestAppContext) {
    let first = Fixture::new();
    let old = first.commit("old");
    let second = Fixture::new();
    let new = second.commit("new");
    let (app, cx) = open(&first, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&old));

    cx.simulate_keystrokes("cmd-o");
    assert!(cx.update(|_, cx| app.view.read(cx).repository_prompt_pending));
    cx.simulate_path_prompt_response(|_| Some(vec![second.root().into()]));
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&new));

    cx.simulate_keystrokes("cmd-shift-o");
    assert!(
        cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::OpenRepository)))
    );
    cx.simulate_keystrokes("cmd-a cmd-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        second.root().to_str().unwrap()
    );
    cx.simulate_keystrokes("cmd-b cmd-o");
    assert!(
        cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::OpenRepository)))
    );
    assert!(!cx.update(|_, cx| app.view.read(cx).repository_prompt_pending));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input(first.root().to_str().unwrap());
    cx.simulate_keystrokes("enter");
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&old));

    cx.simulate_keystrokes("/ ctrl-b cmd-b");
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
    cx.simulate_keystrokes("escape cmd-b");
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).branches.is_empty())
    });
    assert!(cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Branches))));
    cx.simulate_keystrokes("escape cmd-,");
    assert!(cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Preferences))));
    // Invalid preferences exercise Cmd+Enter without writing user preferences.
    app.view.update_in(cx, |view, window, cx| {
        view.font_size_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.simulate_keystrokes("ctrl-a ctrl-k cmd-enter");
    assert!(cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(view.dialog, Some(DialogKind::Preferences)) && !view.operation_message.is_empty()
    }));
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
}

#[gpui_kit::test]
fn vim_history_jumps_and_help_respect_focus(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let oldest = fixture.commit("oldest");
    let middle = fixture.commit("middle");
    let newest = fixture.commit("newest");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&newest));

    cx.simulate_keystrokes("j g");
    assert_eq!(
        selected(&app.view, cx).as_deref(),
        Some(middle.as_str()),
        "a single g must wait for the second key"
    );
    cx.simulate_keystrokes("g");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(newest.as_str()));
    cx.simulate_keystrokes("shift-g");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(oldest.as_str()));
    cx.simulate_keystrokes("g g");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(newest.as_str()));

    for keys in ["?", "shift-/->?"] {
        cx.simulate_keystrokes(keys);
        assert!(cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Help))));
        cx.simulate_keystrokes("shift-g g g ?");
        assert_eq!(selected(&app.view, cx).as_deref(), Some(newest.as_str()));
        assert!(cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Help))));
        cx.simulate_keystrokes("escape");
    }

    cx.simulate_keystrokes("/");
    cx.simulate_input("ggG?");
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).filter.read(cx).value().to_string()),
        "ggG?"
    );
    cx.simulate_keystrokes("ctrl-a ctrl-k escape");
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.visible == 3)
        })
    });
    cx.simulate_keystrokes("shift-g");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(oldest.as_str()));
}

#[gpui_kit::test]
fn dialog_scrollbars_and_wheel_events_are_isolated_from_history(cx: &mut TestAppContext) {
    let fixture = history_fixture(200);
    for i in 0..60 {
        fixture.git(&["branch", &format!("branch-{i:03}")]);
    }
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded == 200)
        })
    });
    for (keys, viewport) in [
        ("f1", "help-scrollbar-viewport"),
        ("ctrl-b", "branch-scrollbar-viewport"),
    ] {
        app.view.update(cx, |view, cx| {
            view.scroll.scroll_to_item_strict(40, ScrollStrategy::Top);
            cx.notify();
        });
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).viewport.start == 40)
        });
        let history_offset =
            cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset());
        let commit = selected(&app.view, cx);
        cx.simulate_keystrokes(keys);
        if keys == "ctrl-b" {
            until(cx, |cx| {
                cx.update(|_, cx| app.view.read(cx).branches.len() == 61)
            });
        }
        cx.refresh().unwrap();
        let handle = cx.update(|_, cx| {
            let view = app.view.read(cx);
            if keys == "f1" {
                view.help_scroll.clone()
            } else {
                view.branch_scroll.0.borrow().base_handle.clone()
            }
        });
        assert_eq!(handle.offset().y, px(0.));
        cx.update(|window, cx| {
            window.scroll(
                viewport,
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-140.))),
                cx,
            )
        });
        cx.refresh().unwrap();
        assert!(
            handle.offset().y < px(0.),
            "dialog must scroll on wheel events"
        );
        assert_eq!(
            cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset()),
            history_offset
        );
        // Repeated scrolling at either boundary must also stay inside the modal.
        for delta in [-10000., -10000., 10000., 10000.] {
            cx.update(|window, cx| {
                window.scroll(
                    viewport,
                    gpui_kit::ScrollDelta::Pixels(point(px(0.), px(delta))),
                    cx,
                )
            });
            cx.refresh().unwrap();
            assert_eq!(
                cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset()),
                history_offset
            );
        }
        let bounds = cx.update(|window, _| window.find(viewport).bounds());
        cx.simulate_click(
            point(bounds.right() - px(8.), bounds.bottom() - px(4.)),
            gpui_kit::Modifiers::default(),
        );
        cx.refresh().unwrap();
        assert!(
            handle.offset().y < px(0.),
            "visible scrollbar track must be clickable"
        );
        // Wheel events on the dimmed backdrop must not reach history either.
        cx.update(|window, cx| {
            let bounds = window.find("history-scrollbar-viewport").bounds();
            window.dispatch_event(
                gpui_kit::ScrollEvent {
                    position: bounds.origin + point(px(10.), px(10.)),
                    delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-140.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        });
        cx.refresh().unwrap();
        assert_eq!(
            cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset()),
            history_offset
        );
        assert_eq!(selected(&app.view, cx), commit);
        cx.simulate_keystrokes("escape");
    }
    let before = cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset());
    cx.update(|window, cx| {
        window.scroll(
            "history-scrollbar-viewport",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-140.))),
            cx,
        )
    });
    cx.refresh().unwrap();
    assert!(
        cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset().y < before.y),
        "history must scroll again after closing the dialog"
    );
}

struct TooltipProbe {
    status: String,
}

impl Render for TooltipProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .child(
                div()
                    .id("graph-heading")
                    .test_support()
                    .w(px(280.))
                    .h(px(26.))
                    .child("Graph · … idle lanes")
                    .hoverable_tooltip(|window, cx| {
                        wrapped_tooltip(IDLE_LANES_TOOLTIP, window, cx)
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("history-status")
                    .test_support()
                    .truncate()
                    .px_2()
                    .py_1()
                    .child(status.clone())
                    .hoverable_tooltip(move |window, cx| {
                        wrapped_tooltip(status.clone(), window, cx)
                    }),
            )
    }
}

#[gpui_kit::test]
fn status_and_idle_lane_tooltips_wrap_inside_the_window(cx: &mut TestAppContext) {
    // Exercise the tooltip renderer and edge placement without asynchronous Git
    // snapshots replacing the graph data while a delayed hover is pending.
    cx.update(gpui_kit::init);
    let mut probe = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|_| TooltipProbe {
            status: "branch switched; details for /CaseSensitive/Repository.git ".repeat(6),
        });
        probe = Some(view.clone());
        gpui_kit::base::Root::new(view, window, cx)
    });
    let probe = probe.unwrap();
    for dimensions in [size(px(1280.), px(850.)), size(px(740.), px(500.))] {
        cx.simulate_resize(dimensions);
        cx.refresh().unwrap();
        for trigger in ["history-status", "graph-heading"] {
            cx.simulate_mouse_move(point(px(1.), px(1.)), None, gpui_kit::Modifiers::default());
            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            cx.refresh().unwrap();
            cx.update(|window, cx| window.hover(trigger, cx));
            until(cx, |cx| {
                cx.update(|window, _| window.try_find("tooltip-layout").is_some())
            });
            // Finish placement and entrance animation before checking geometry.
            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            cx.refresh().unwrap();
            cx.update(|window, _| {
                let popup = window.find("tooltip-layout").bounds();
                let text = window.find("tooltip-text").bounds();
                assert!(popup.left() >= px(4.) && popup.top() >= px(4.));
                assert!(popup.right() <= dimensions.width - px(4.));
                assert!(popup.bottom() <= dimensions.height - px(4.));
                assert!(
                    text.size.height > px(24.),
                    "long tooltip must wrap onto multiple lines"
                );
                assert!(text.right() <= popup.right());
                assert!(text.bottom() <= popup.bottom());
            });
        }
    }
    // Very long diagnostics stay within the viewport and can be read by scrolling.
    cx.simulate_mouse_move(point(px(1.), px(1.)), None, gpui_kit::Modifiers::default());
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    probe.update(cx, |view, cx| {
        view.status = "long diagnostic with /CaseSensitive/Repository.git details ".repeat(100);
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.update(|window, cx| window.hover("history-status", cx));
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("tooltip-layout").is_some())
    });
    let before = cx.update(|window, _| {
        let popup = window.find("tooltip-layout").bounds();
        let text = window.find("tooltip-text").bounds();
        assert!(popup.top() >= px(4.) && popup.bottom() <= window.viewport_size().height - px(4.));
        assert!(text.size.height > popup.size.height);
        text.origin.y
    });
    cx.update(|window, cx| {
        window.scroll(
            "tooltip-layout",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    cx.refresh().unwrap();
    cx.update(|window, _| assert!(window.find("tooltip-text").bounds().origin.y < before));
}

#[gpui_kit::test]
fn alt_list_navigation_preserves_filter_focus_and_emacs_editing(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let oldest = fixture.commit("match oldest");
    let middle = fixture.commit("match middle");
    let newest = fixture.commit("match newest");
    fixture.git(&["branch", "match-a"]);
    fixture.git(&["branch", "match-b"]);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&newest));
    // macOS Option keys may also carry printable characters; bindings must consume them.
    let (down, up) = if cfg!(target_os = "macos") {
        ("alt-j->\u{2206}", "alt-k->\u{02da}")
    } else {
        ("alt-j", "alt-k")
    };
    cx.simulate_keystrokes("/");
    cx.simulate_input("match");
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.request.query == "match"
                && view.snapshot.as_ref().is_some_and(|snapshot| {
                    !snapshot.filtering
                        && snapshot.generation == view.request.generation
                        && snapshot.request_id == view.request.id
                })
        })
    });
    cx.simulate_keystrokes(down);
    assert_eq!(selected(&app.view, cx).as_deref(), Some(middle.as_str()));
    cx.simulate_keystrokes(down);
    assert_eq!(selected(&app.view, cx).as_deref(), Some(oldest.as_str()));
    cx.simulate_keystrokes(up);
    assert_eq!(selected(&app.view, cx).as_deref(), Some(middle.as_str()));
    assert!(cx.update(|window, cx| {
        app.view
            .read(cx)
            .filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).filter.read(cx).value().to_string()),
        "match"
    );
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).filter.read(cx).value().to_string()),
        ""
    );
    cx.simulate_keystrokes("escape ctrl-b");
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).branches.is_empty())
    });
    cx.simulate_input("match");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).filtered_branches.len()),
        2
    );
    cx.simulate_keystrokes(down);
    assert_eq!(cx.update(|_, cx| app.view.read(cx).branch_index), 1);
    cx.simulate_keystrokes(up);
    assert_eq!(cx.update(|_, cx| app.view.read(cx).branch_index), 0);
    assert!(cx.update(|window, cx| {
        app.view
            .read(cx)
            .branch_filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).branch_filter.read(cx).value().to_string()),
        "match"
    );
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).branch_filter.read(cx).value().to_string()),
        ""
    );
    cx.simulate_keystrokes("escape ctrl-shift-o ctrl-a ctrl-k");
    cx.simulate_input("keep this path");
    cx.simulate_keystrokes("alt-j alt-k");
    assert_eq!(
        cx.update(|_, cx| app
            .view
            .read(cx)
            .repository_input
            .read(cx)
            .value()
            .to_string()),
        "keep this path"
    );
}

#[gpui_kit::test]
fn preferences_apply_shortcuts_work_from_initial_font_selector_focus(cx: &mut TestAppContext) {
    let settings = tempfile::tempdir().unwrap();
    let fixture = Fixture::new();
    fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    cx.simulate_keystrokes("ctrl-,");
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).font_catalog.is_some())
    });
    app.view.update_in(cx, |view, window, cx| {
        view.preferences_path = Some(settings.path().join("preferences.toml"));
        view.graph_columns_input
            .update(cx, |input, cx| input.set_value("0", window, cx));
    });
    cx.refresh().unwrap();
    let shortcuts: &[&str] = if cfg!(target_os = "macos") {
        &["ctrl-enter", "cmd-enter"]
    } else {
        &["ctrl-enter"]
    };
    for &shortcut in shortcuts {
        app.view
            .update(cx, |view, _| view.operation_message.clear());
        let contexts = cx.update(|window, _| window.context_stack());
        cx.simulate_keystrokes(shortcut);
        let message = cx.update(|_, cx| app.view.read(cx).operation_message.clone());
        assert!(
            message.contains("graph columns"),
            "{shortcut} did not apply from the initial selector focus: {contexts:?}; message={message:?}"
        );
    }
    cx.simulate_keystrokes("enter");
    cx.refresh().unwrap();
    assert_eq!(
        cx.update(|window, _| window.find("ui-font-select").expanded()),
        Some(true)
    );
}

#[gpui_kit::test]
fn application_preferences_replace_flags_apply_and_persist(cx: &mut TestAppContext) {
    let settings = tempfile::tempdir().unwrap();
    let path = settings.path().join("preferences.toml");
    let fixture = Fixture::new();
    fixture.commit("older");
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    let config = gitk_rs::cli::parse([
        fixture.root().as_os_str().to_owned(),
        "--mode=light".into(),
        "--graph-cols=128".into(),
        "--nosyntax".into(),
        "--text-graph".into(),
        "--nowatch".into(),
    ])
    .unwrap();
    app.view.update(cx, |view, _| {
        view.config = config;
        view.preferences_path = Some(path.clone());
    });
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    cx.simulate_keystrokes("ctrl-,");
    for id in [
        "light-theme-setting",
        "syntax-setting",
        "watch-setting",
        "text-graph-setting",
        "on-demand-setting",
    ] {
        scroll_preferences_to(id, cx);
        cx.update(|window, cx| window.click(id, cx));
    }
    scroll_preferences_to("history-batch-input", cx);
    cx.update(|window, cx| window.click("history-batch-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("73");
    scroll_preferences_to("graph-columns-input", cx);
    cx.update(|window, cx| window.click("graph-columns-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("1025");
    cx.update(|window, cx| window.click("apply-preferences", cx));
    assert!(cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(view.dialog, Some(DialogKind::Preferences))
            && view.operation_message.contains("graph columns")
            && view.config.graph_columns == 128
            && view.request.generation == generation
    }));
    assert!(!path.exists());
    scroll_preferences_to("graph-columns-input", cx);
    cx.update(|window, cx| window.click("graph-columns-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("512");
    cx.update(|window, cx| window.click("apply-preferences", cx));
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).preferences_save_pending)
            && selected(&app.view, cx).as_deref() == Some(&tip)
    });
    let saved = Preferences::load_from(&path).unwrap();
    assert_eq!(saved.theme, Theme::Dark);
    assert_eq!(saved.batch, Some(73));
    assert_eq!(saved.graph_columns, 512);
    assert!(saved.syntax && saved.watch);
    assert!(!saved.text_graph);
    cx.update(|_, cx| {
        let view = app.view.read(cx);
        assert_eq!(view.preferences, saved);
        assert_eq!(saved.with_config(&view.config), saved);
        assert_eq!(view.request.batch, Some(73));
        assert_eq!(view.request.graph_columns, 512);
        assert!(view.request.generation.0 > generation.0);
    });
    let restarted = gitk_rs::cli::parse_with_preferences([], &saved).unwrap();
    assert_eq!(saved.with_config(&restarted), saved);

    cx.simulate_keystrokes("ctrl-,");
    scroll_preferences_to("on-demand-setting", cx);
    cx.update(|window, cx| window.click("on-demand-setting", cx));
    cx.update(|window, cx| window.click("apply-preferences", cx));
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).preferences_save_pending)
    });
    assert_eq!(Preferences::load_from(&path).unwrap().batch, None);
    assert!(cx.update(|_, cx| app.view.read(cx).request.batch.is_none()));
}

#[gpui_kit::test]
fn application_preferences_cancel_invalid_values_and_report_save_failure(cx: &mut TestAppContext) {
    let settings = tempfile::tempdir().unwrap();
    let path = settings.path().join("preferences.toml");
    let fixture = Fixture::new();
    let older = fixture.commit("older");
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    let original = cx.update(|_, cx| app.view.read(cx).preferences.clone());
    original.save_to(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    app.view
        .update(cx, |view, _| view.preferences_path = Some(path.clone()));
    let generation = cx.update(|_, cx| app.view.read(cx).request.generation);
    cx.simulate_keystrokes("ctrl-,");
    scroll_preferences_to("syntax-setting", cx);
    cx.update(|window, cx| window.click("syntax-setting", cx));
    scroll_preferences_to("history-batch-input", cx);
    cx.update(|window, cx| window.click("history-batch-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("0");
    cx.update(|window, cx| window.click("apply-preferences", cx));
    cx.update(|_, cx| {
        let view = app.view.read(cx);
        assert!(view.operation_message.contains("batch size"));
        assert!(view.config.syntax);
        assert_eq!(view.preferences, original);
        assert_eq!(view.request.generation, generation);
    });
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    cx.simulate_keystrokes("escape ctrl-,");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx)),
        original
    );
    // A directory is a deterministic failed destination, even when running as root.
    app.view.update(cx, |view, _| {
        view.preferences_path = Some(settings.path().into())
    });
    scroll_preferences_to("text-graph-setting", cx);
    cx.update(|window, cx| window.click("text-graph-setting", cx));
    cx.update(|window, cx| window.click("apply-preferences", cx));
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).preferences_save_pending)
    });
    assert!(cx.update(|_, cx| {
        let view = app.view.read(cx);
        view.dialog.is_none()
            && view.config.text_graph
            && view.operation_message.contains("save preferences:")
    }));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    cx.simulate_keystrokes("alt-j");
    assert_eq!(selected(&app.view, cx).as_deref(), Some(older.as_str()));
}

#[gpui_kit::test]
fn searchable_font_selectors_preview_drafts_and_cancel_restores_preferences(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| selected(&app.view, cx).as_deref() == Some(&tip));
    let original = cx.update(|_, cx| app.view.read(cx).preferences.clone());
    app.view.update(cx, |view, _| {
        view.font_catalog = Some(vec![
            original.ui_font.clone().into(),
            original.fixed_font.clone().into(),
            ".SystemUIFont".into(),
        ])
    });
    cx.simulate_keystrokes("ctrl-,");
    cx.refresh().unwrap();
    for (id, query, is_ui) in [
        ("ui-font-select", original.fixed_font.as_str(), true),
        ("fixed-font-select", ".SystemUIFont", false),
    ] {
        cx.update(|window, cx| window.click(id, cx));
        assert_eq!(
            cx.update(|window, _| window.find(id).expanded()),
            Some(true)
        );
        cx.simulate_input(query);
        cx.background_executor
            .advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes("enter");
        until(cx, |cx| {
            cx.update(|_, cx| {
                let draft = app.view.read(cx).draft_preferences(cx);
                if is_ui {
                    draft.ui_font == query
                } else {
                    draft.fixed_font == query
                }
            })
        });
        assert!(
            cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Preferences)))
        );
    }
    let small = cx.update(|window, _| {
        (
            window.find("ui-font-preview").bounds().size.height,
            window.find("fixed-font-preview").bounds().size.height,
        )
    });
    cx.update(|window, cx| window.click("font-size-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("24");
    cx.refresh().unwrap();
    cx.update(|window, _| {
        assert!(window.find("ui-font-preview").bounds().size.height > small.0);
        assert!(window.find("fixed-font-preview").bounds().size.height > small.1);
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).preferences.clone()),
        original,
        "preview must not apply or save the draft"
    );
    assert!(cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).validate().is_ok()));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("NaN");
    cx.refresh().unwrap();
    assert!(cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).validate().is_err()));
    cx.update(|window, cx| window.click("apply-preferences", cx));
    assert!(cx.update(|_, cx| {
        let view = app.view.read(cx);
        matches!(view.dialog, Some(DialogKind::Preferences))
            && view.preferences == original
            && view.operation_message.contains("font size")
    }));
    cx.simulate_keystrokes("escape ctrl-,");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx)),
        original
    );
    cx.update(|window, cx| window.click("ui-font-select", cx));
    cx.simulate_input("no matching font");
    cx.simulate_keystrokes("escape");
    assert!(
        cx.update(|_, cx| matches!(app.view.read(cx).dialog, Some(DialogKind::Preferences))),
        "Escape must close the font dropdown before the preferences dialog"
    );
    assert_eq!(
        cx.update(|window, _| window.find("ui-font-select").expanded()),
        Some(false)
    );
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|_, cx| app.view.read(cx).dialog.is_none()));
}

#[gpui_kit::test]
fn font_size_steps_with_buttons_and_keyboard_within_bounds(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    let original = cx.update(|_, cx| app.view.read(cx).preferences.clone());
    cx.simulate_keystrokes("ctrl-,");
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).font_catalog.is_some())
    });
    cx.update(|window, cx| window.click("font-size-input", cx));
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("12");
    cx.refresh().unwrap();
    let height = cx.update(|window, _| window.find("ui-font-preview").bounds().size.height);
    cx.update(|window, cx| window.within("font-size-input").click("increment", cx));
    cx.refresh().unwrap();
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).font_size),
        13.
    );
    assert!(cx.update(|window, _| window.find("ui-font-preview").bounds().size.height) > height);
    cx.update(|window, cx| window.within("font-size-input").click("decrement", cx));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).font_size),
        12.
    );
    cx.simulate_keystrokes("up");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).font_size),
        13.
    );
    cx.simulate_keystrokes("down");
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).font_size),
        12.
    );
    for (value, key, button) in [("24", "up", "increment"), ("8", "down", "decrement")] {
        cx.simulate_keystrokes("ctrl-a ctrl-k");
        cx.simulate_input(value);
        cx.simulate_keystrokes(key);
        cx.refresh().unwrap();
        cx.update(|window, cx| window.within("font-size-input").click(button, cx));
        assert_eq!(
            cx.update(|_, cx| app.view.read(cx).draft_preferences(cx).font_size),
            value.parse::<f32>().unwrap()
        );
    }
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).preferences.clone()),
        original
    );
    cx.simulate_keystrokes("escape");
}

#[gpui_kit::test]
fn font_catalog_loading_rejects_closed_and_superseded_preferences(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    cx.simulate_keystrokes("ctrl-,");
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).font_catalog.is_some())
    });
    let old = cx.update(|_, cx| app.view.read(cx).font_catalog_generation);
    cx.simulate_keystrokes("escape ctrl-,");
    let current = cx.update(|_, cx| app.view.read(cx).font_catalog_generation);
    assert_ne!(old, current);
    let original = cx.update(|_, cx| app.view.read(cx).draft_preferences(cx));
    app.view.update_in(cx, |view, window, cx| {
        view.accept_font_catalog(old, vec!["stale font".into()], window, cx)
    });
    assert!(!cx.update(|_, cx| {
        app.view
            .read(cx)
            .font_catalog
            .as_ref()
            .unwrap()
            .iter()
            .any(|name| name.as_ref() == "stale font")
    }));
    app.view.update_in(cx, |view, window, cx| {
        view.accept_font_catalog(current, vec!["Newly installed font".into()], window, cx)
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).draft_preferences(cx)),
        original
    );
    cx.simulate_keystrokes("escape");
    app.view.update_in(cx, |view, window, cx| {
        view.accept_font_catalog(current, vec!["closed font".into()], window, cx)
    });
    assert!(!cx.update(|_, cx| {
        app.view
            .read(cx)
            .font_catalog
            .as_ref()
            .unwrap()
            .iter()
            .any(|name| name.as_ref() == "closed font")
    }));
}

#[gpui_kit::test]
fn font_preferences_keep_actions_visible_with_large_fonts_in_a_small_window(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    app.view.update_in(cx, |view, window, cx| {
        view.preferences.font_size = 24.;
        apply_fonts(&view.preferences, cx);
        view.font_catalog = Some(vec![
            view.preferences.ui_font.clone().into(),
            view.preferences.fixed_font.clone().into(),
        ]);
        window.refresh();
    });
    cx.simulate_resize(size(px(800.), px(500.)));
    cx.simulate_keystrokes("ctrl-,");
    cx.refresh().unwrap();
    cx.update(|window, _| {
        let apply = window.find("apply-preferences");
        let close = window.find("close-dialog");
        assert!(apply.visible() && close.visible());
        assert!(close.bounds().bottom() <= window.viewport_size().height);
        assert!(
            window.find("ui-font-select").bounds().bottom()
                <= window.find("fixed-font-select").bounds().top()
        );
    });
    cx.update(|window, cx| {
        window.scroll(
            "ui-font-select",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-1000.))),
            cx,
        )
    });
    cx.refresh().unwrap();
    cx.update(|window, _| {
        assert!(window.find("apply-preferences").visible());
        assert!(window.find("close-dialog").visible());
    });
    scroll_preferences_to("graph-columns-input", cx);
    cx.update(|window, _| assert!(window.find("graph-columns-input").visible()));
    scroll_preferences_to("fixed-font-preview", cx);
    cx.update(|window, _| {
        assert!(window.find("fixed-font-preview").visible());
        assert!(
            window.find("fixed-font-preview").bounds().bottom()
                <= window.find("apply-preferences").bounds().top()
        );
        assert!(window.find("close-dialog").visible());
    });
}

#[gpui_kit::test]
fn shortcuts_descriptions_wrap_without_clipping(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    app.view.update(cx, |view, cx| {
        view.preferences.ui_font = gitk_rs::preferences::SYSTEM_MONOSPACE_FONT.into();
        view.preferences.font_size = 24.;
        cx.notify();
    });
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.simulate_keystrokes("f1");
    cx.refresh().unwrap();
    cx.update(|window, _| {
        let viewport = window.find("shortcut-groups").bounds();
        let group = window.within("Commit list");
        let single_line = group.find("Up / p / k").bounds();
        let copy = group
            .find(if cfg!(target_os = "macos") {
                "Ctrl/Cmd+C"
            } else {
                "Ctrl+C"
            })
            .bounds();
        assert!(
            copy.size.height > single_line.size.height,
            "long descriptions must wrap rather than overflow horizontally"
        );
        assert!(copy.right() <= viewport.right());
        assert!(copy.left() >= viewport.left());
        assert!(
            copy.bottom() < window.find("Diff view").bounds().top(),
            "the following group must stay below wrapped descriptions"
        );
    });
}

#[gpui_kit::test]
fn shortcuts_keyboard_scrolls_lines_pages_and_boundaries_without_moving_history(
    cx: &mut TestAppContext,
) {
    let fixture = history_fixture(200);
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.loaded == 200)
        })
    });
    app.view.update(cx, |view, cx| {
        view.scroll.scroll_to_item_strict(40, ScrollStrategy::Top);
        cx.notify();
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).viewport.start == 40)
    });
    let history = cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset());
    let commit = selected(&app.view, cx);
    cx.simulate_keystrokes("f1");
    cx.refresh().unwrap();
    cx.simulate_keystrokes("down");
    cx.refresh().unwrap();
    let line = cx.update(|_, cx| app.view.read(cx).help_scroll.offset().y);
    assert!(line < px(0.));
    cx.simulate_keystrokes("up");
    cx.refresh().unwrap();
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).help_scroll.offset().y),
        px(0.)
    );
    cx.simulate_keystrokes("pagedown");
    cx.refresh().unwrap();
    assert!(cx.update(|_, cx| app.view.read(cx).help_scroll.offset().y < line));
    cx.simulate_keystrokes("pageup");
    cx.refresh().unwrap();
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).help_scroll.offset().y),
        px(0.)
    );
    cx.simulate_keystrokes("end");
    cx.refresh().unwrap();
    cx.update(|_, cx| {
        let scroll = &app.view.read(cx).help_scroll;
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
    });
    cx.simulate_keystrokes("down pagedown end");
    cx.refresh().unwrap();
    cx.update(|_, cx| {
        let scroll = &app.view.read(cx).help_scroll;
        assert_eq!(scroll.offset().y, -scroll.max_offset().y);
    });
    cx.simulate_keystrokes("home up pageup");
    cx.refresh().unwrap();
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).help_scroll.offset().y),
        px(0.)
    );
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset()),
        history
    );
    assert_eq!(selected(&app.view, cx), commit);
    cx.simulate_keystrokes("escape down");
    assert_ne!(
        selected(&app.view, cx),
        commit,
        "arrow keys must return to history navigation after closing help"
    );
}

#[gpui_kit::test]
fn graph_paints_a_folded_column_at_the_maximum_lane_limit(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("wide graph");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.request_id == view.request.id)
        })
    });
    let last = MAX_GRAPH_LANES as u16 - 1;
    app.view.update(cx, |view, cx| {
        let snapshot = view.snapshot.as_mut().unwrap();
        let entry = &snapshot.rows[0];
        let graph = GraphRow {
            node: Some(0),
            width: MAX_GRAPH_LANES as u16,
            edges: (0..=last)
                .map(|lane| gitk_rs::graph::Edge {
                    from: if lane == last { lane } else { 0 },
                    to: lane,
                    parent: lane != last,
                    color: (lane % 6) as u8,
                })
                .collect(),
            ..GraphRow::default()
        };
        snapshot.rows[0] = Arc::new(Entry {
            ordinal: entry.ordinal,
            commit: entry.commit.clone(),
            graph,
        });
        view.graph_layout = GraphLayout::new([&snapshot.rows[0].graph], view.graph_width);
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.update(|_, cx| {
        let layout = &app.view.read(cx).graph_layout;
        assert_eq!(layout.column(last), last as usize);
        assert!(layout.is_collapsed(last));
    });
}

#[gpui_kit::test]
fn repository_title_and_inline_filter_clear_controls(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("base");
    fixture.git(&["branch", "another"]);
    let (app, cx) = open(&fixture, cx);
    // Git resolves symlinks (including macOS /var -> /private/var) in its root.
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("repository");
    std::os::unix::fs::symlink(fixture.root(), &alias).unwrap();
    app.view
        .update(cx, |view, cx| view.change_repository(alias.join("."), cx));
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).workspace_snapshot.is_some())
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).title.clone()),
        format!(
            "{} — gitk-rs",
            fixture.root().canonicalize().unwrap().display()
        )
    );
    cx.update(|window, cx| {
        app.view.update(cx, |view, cx| {
            view.filter
                .update(cx, |input, cx| input.set_value("no matches", window, cx));
        })
    });
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("clean").is_some())
    });
    let clear = cx.update(|window, _| window.find("clean").bounds());
    cx.simulate_click(clear.center(), gpui_kit::Modifiers::default());
    assert!(cx.update(|_, cx| app.view.read(cx).filter.read(cx).value().is_empty()));
    assert!(cx.update(|window, _| window.try_find("clear-filter").is_none()));
    cx.update(|window, cx| {
        app.view
            .update(cx, |view, cx| view.open_branches(window, cx))
    });
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).branches.len() == 2)
    });
    cx.update(|window, cx| {
        app.view.update(cx, |view, cx| {
            view.branch_filter
                .update(cx, |input, cx| input.set_value("another", window, cx));
        })
    });
    until(cx, |cx| {
        cx.update(|window, _| window.try_find("clean").is_some())
    });
    let clear = cx.update(|window, _| window.find("clean").bounds());
    cx.simulate_click(clear.center(), gpui_kit::Modifiers::default());
    assert!(cx.update(|_, cx| app.view.read(cx).branch_filter.read(cx).value().is_empty()));
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).filtered_branches.len()),
        2
    );
}

#[gpui_kit::test]
fn repository_caption_updates_even_when_opening_the_directory_fails(cx: &mut TestAppContext) {
    let first = Fixture::new();
    first.commit("first");
    let second = Fixture::new();
    second.commit("second");
    let invalid = tempfile::tempdir().unwrap();
    let (app, cx) = open(&first, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view.read(cx).title == repository_title(&first.root().canonicalize().unwrap())
        })
    });
    app.view.update(cx, |view, cx| {
        view.change_repository(invalid.path().to_owned(), cx)
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.title == repository_title(invalid.path())
                && view
                    .workspace_snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_some())
        })
    });
    app.view.update(cx, |view, cx| {
        view.change_repository(second.root().to_owned(), cx)
    });
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view.read(cx).title == repository_title(&second.root().canonicalize().unwrap())
        })
    });
}

#[gpui_kit::test]
fn resized_columns_and_panes_are_saved_and_restored(cx: &mut TestAppContext) {
    use gitk_rs::diff::options::{self, Layout, State};
    let settings = tempfile::tempdir().unwrap();
    let path = settings.path().join("state.toml");
    let fixture = Fixture::new();
    fixture.commit("base");
    options::save_state_to(
        State {
            layout: Layout {
                graph_width: 320.,
                history_height: 300.,
                file_list_width: 310.,
            },
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    let saved = {
        let (app, cx) = open_with_state(&fixture, Some(1000), Some(path.clone()), cx);
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).selected.is_some())
        });
        assert_eq!(cx.update(|_, cx| app.view.read(cx).graph_width), 320.);
        assert_eq!(
            cx.update(|window, _| window.find("files-scrollbar-viewport").bounds().size.width)
                + px(2.),
            px(310.)
        );
        let graph = cx.update(|window, _| window.find("graph-divider").bounds().center());
        let history = cx.update(|window, _| {
            window
                .within("main-split")
                .find_all(("resizable-handle", 0_usize))
                .into_iter()
                .map(|element| element.bounds())
                .find(|bounds| bounds.size.width > bounds.size.height)
                .unwrap()
                .center()
        });
        for (origin, delta) in [
            (graph, point(px(70.), px(0.))),
            (history, point(px(0.), px(50.))),
        ] {
            cx.simulate_mouse_down(
                origin,
                gpui_kit::MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
            cx.simulate_mouse_move(
                origin + delta,
                Some(gpui_kit::MouseButton::Left),
                gpui_kit::Modifiers::default(),
            );
            // GPUI starts the drag on the first move; the next move resizes.
            cx.simulate_mouse_move(
                origin + delta,
                Some(gpui_kit::MouseButton::Left),
                gpui_kit::Modifiers::default(),
            );
            cx.simulate_mouse_up(
                origin + delta,
                gpui_kit::MouseButton::Left,
                gpui_kit::Modifiers::default(),
            );
        }
        let files = cx.update(|window, _| {
            window
                .within("diff-split")
                .find(("resizable-handle", 0_usize))
                .bounds()
                .center()
        });
        cx.simulate_mouse_down(
            files,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_move(
            files - point(px(60.), px(0.)),
            Some(gpui_kit::MouseButton::Left),
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_move(
            files - point(px(60.), px(0.)),
            Some(gpui_kit::MouseButton::Left),
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            files - point(px(60.), px(0.)),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        drop(app);
        let saved = options::load_state_from(&path).unwrap();
        assert_eq!(saved.layout.graph_width, 390.);
        assert!(saved.layout.history_height > 340., "{:?}", saved.layout);
        assert!(saved.layout.file_list_width > 360., "{:?}", saved.layout);
        saved
    };
    let (app, cx) = open_with_state(&fixture, Some(1000), Some(path.clone()), cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).selected.is_some())
    });
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).graph_width),
        saved.layout.graph_width
    );
    assert_eq!(
        cx.update(|_, cx| app.view.read(cx).history_height),
        saved.layout.history_height
    );
    let width = cx.update(|window, _| window.find("files-scrollbar-viewport").bounds().size.width);
    assert!((width + px(2.) - px(saved.layout.file_list_width)).abs() <= px(1.));
    assert!(!settings.path().join("preferences.toml").exists());
}

#[gpui_kit::test]
fn metadata_search_shows_badges_errors_and_recovers(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture.commit("needle");
    fixture.commit("other");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().is_some_and(|s| s.eof))
    });
    assert!(cx.update(|window, _| window.try_find("author:text").is_none()));
    cx.simulate_keystrokes("/");
    cx.simulate_input("author:fixture message:needle path:file.txt contents:needle");
    until(cx, |cx| {
        cx.update(|window, cx| {
            window.try_find("search-filter-0").is_some()
                && app.view.read(cx).snapshot.as_ref().is_some_and(|s| {
                    s.visible == 1 && s.rows[0].commit.summary() == "needle" && !s.filtering
                })
        })
    });
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("before:nope");
    until(cx, |cx| {
        cx.update(|window, cx| {
            window.try_find("search-error").is_some()
                && app
                    .view
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_some())
        })
    });
    cx.simulate_keystrokes("ctrl-a ctrl-k");
    cx.simulate_input("author:fixture");
    until(cx, |cx| {
        cx.update(|window, cx| {
            window.try_find("search-error").is_none()
                && app
                    .view
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.error.is_none() && s.visible == 2)
        })
    });
    cx.simulate_keystrokes("f1");
    cx.refresh().unwrap();
    cx.update(|window, _| {
        assert!(window.find("Commit search").bounds().size.height > px(0.));
        for qualifier in gitk_rs::search::Qualifier::ALL {
            assert!(
                window
                    .find(SharedString::from(qualifier.syntax()))
                    .bounds()
                    .size
                    .height
                    > px(0.)
            );
        }
    });
}

#[gpui_kit::test]
fn regex_toggle_refilters_reports_errors_and_restores_saved_state(cx: &mut TestAppContext) {
    use gitk_rs::diff::options::{self, Layout, State};
    let settings = tempfile::tempdir().unwrap();
    let path = settings.path().join("state.toml");
    options::save_state_to(
        State {
            layout: Layout {
                graph_width: 300.,
                ..Default::default()
            },
            diff: gitk_rs::diff::DiffOptions {
                context: 7,
                ..Default::default()
            },
            ..Default::default()
        },
        &path,
    )
    .unwrap();
    let fixture = Fixture::new();
    fixture.commit("fix all");
    fixture.commit("fix.*");
    {
        let (app, cx) = open_with_state(&fixture, None, Some(path.clone()), cx);
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().is_some_and(|s| s.eof))
        });
        assert!(!cx.update(|_, cx| app.view.read(cx).request.regex));
        cx.simulate_keystrokes("/");
        cx.simulate_input("message:fix.*");
        until(cx, |cx| {
            cx.update(|_, cx| {
                app.view
                    .read(cx)
                    .snapshot
                    .as_ref()
                    .is_some_and(|s| s.visible == 1 && !s.filtering)
            })
        });
        cx.simulate_keystrokes("alt-r");
        assert!(!cx.update(|_, cx| app.view.read(cx).config.watch));
        assert!(cx.update(|window, cx| {
            app.view
                .read(cx)
                .filter
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        }));
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                view.request.regex
                    && view.snapshot.as_ref().is_some_and(|s| {
                        s.request_id == view.request.id && s.visible == 2 && !s.filtering
                    })
            })
        });
        cx.simulate_keystrokes("f1 alt-r");
        assert!(cx.update(|_, cx| app.view.read(cx).request.regex));
        cx.simulate_keystrokes("escape / ctrl-a ctrl-k");
        cx.simulate_input("message:[");
        until(cx, |cx| {
            cx.update(|window, cx| {
                window.try_find("search-error").is_some()
                    && app.view.read(cx).snapshot.as_ref().is_some_and(|s| {
                        s.query_error
                            .as_deref()
                            .is_some_and(|error| error.contains("Invalid regular expression"))
                    })
            })
        });
        let toggle = cx.update(|window, _| window.find("search-regex").bounds().center());
        cx.simulate_click(toggle, gpui_kit::Modifiers::default());
        assert!(cx.update(|_, cx| app.view.read(cx).query_error(cx).is_none()));
        until(cx, |cx| {
            cx.update(|window, cx| {
                let view = app.view.read(cx);
                window.try_find("search-error").is_none()
                    && view.snapshot.as_ref().is_some_and(|s| {
                        s.request_id == view.request.id && s.error.is_none() && s.visible == 0
                    })
            })
        });
        cx.simulate_keystrokes("/ ctrl-a ctrl-k");
        cx.simulate_input("author:Fixture");
        cx.simulate_keystrokes("escape alt-r");
        until(cx, |cx| {
            cx.update(|_, cx| {
                let view = app.view.read(cx);
                view.request.regex
                    && view.snapshot.as_ref().is_some_and(|s| {
                        s.request_id == view.request.id && s.error.is_none() && s.visible == 2
                    })
            })
        });
    }
    let saved = options::load_state_from(&path).unwrap();
    assert!(saved.search.regex);
    assert_eq!(saved.diff.context, 7);
    assert_eq!(saved.layout.graph_width, 300.);
    assert!(!settings.path().join("preferences.toml").exists());
    {
        let (app, cx) = open_with_state(&fixture, None, Some(path.clone()), cx);
        until(cx, |cx| {
            cx.update(|_, cx| app.view.read(cx).snapshot.as_ref().is_some_and(|s| s.eof))
        });
        assert!(cx.update(|_, cx| app.view.read(cx).request.regex));
        let toggle = cx.update(|window, _| window.find("search-regex").bounds().center());
        cx.simulate_click(toggle, gpui_kit::Modifiers::default());
        assert!(!cx.update(|_, cx| app.view.read(cx).request.regex));
    }
    assert!(!options::load_state_from(&path).unwrap().search.regex);
}

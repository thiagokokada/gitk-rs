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
}

pub(super) fn open<'a>(
    fixture: &Fixture,
    cx: &'a mut TestAppContext,
) -> (Harness, &'a mut VisualTestContext) {
    let config = Config {
        repository: fixture.root().into(),
        watch: false,
        ..Config::default()
    };
    let request = Request {
        generation: Generation(1),
        id: 0,
        refs_revision: 0,
        repository: config.repository.clone(),
        range: 0..128,
        query: String::new(),
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
    let diff = DiffEngine::spawn();
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::change(ThemeMode::Dark, None, cx);
        bind_keys(cx);
    });
    let mut view = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| {
            HistoryWindow::new(
                config,
                TimingGuard::start(false, Instant::now()).timing(),
                WindowServices {
                    history: history.handle(),
                    workspace: workspace.handle(),
                    diff: diff.handle(),
                },
                Preferences::default(),
                request,
                window,
                cx,
            )
        });
        view = Some(entity.clone());
        gpui_kit::base::Root::new(entity, window, cx)
    });
    cx.simulate_resize(size(px(1280.), px(850.)));
    (
        Harness {
            view: view.unwrap(),
            _history: history,
            _workspace: workspace,
            _diff: diff,
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

fn selected(view: &Entity<HistoryWindow>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .selected
            .as_ref()
            .map(|(_, entry)| entry.commit.id.as_str().to_owned())
    })
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
    fixture.commit("new tip");
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
    fixture.commit("automatic tip");
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
fn reload_control_toggles_watching_and_refreshes_history(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let head = fixture.commit("base");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        selected(&app.view, cx).as_deref() == Some(head.as_str())
    });
    assert!(!cx.update(|_, cx| app.view.read(cx).config.watch));
    let button = cx.update(|window, _| window.find("reload").bounds());
    cx.simulate_click(button.center(), gpui_kit::Modifiers::default());
    assert!(cx.update(|_, cx| app.view.read(cx).config.watch));
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).reload_selection.is_none())
    });
    std::fs::write(fixture.root().join("file.txt"), "changed").unwrap();
    until(cx, |cx| {
        cx.update(|_, cx| !app.view.read(cx).local_rows.is_empty())
    });
    let button = cx.update(|window, _| window.find("reload").bounds());
    cx.simulate_click(button.center(), gpui_kit::Modifiers::default());
    assert!(!cx.update(|_, cx| app.view.read(cx).config.watch));
    until(cx, |cx| {
        cx.update(|_, cx| app.view.read(cx).reload_selection.is_none())
    });
    assert_eq!(selected(&app.view, cx).as_deref(), Some(head.as_str()));
    cx.simulate_keystrokes("alt-r");
    assert!(cx.update(|_, cx| app.view.read(cx).config.watch));
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
            assert!(!window.find("Graph").visible());
        });
        cx.update(|window, cx| {
            window.scroll(
                "shortcut-groups",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
                cx,
            );
        });
        cx.refresh().unwrap();
        cx.update(|window, _| {
            assert!(
                window.find("Graph").visible(),
                "last group must be reachable"
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

#[gpui_kit::test]
fn status_and_idle_lane_tooltips_wrap_inside_the_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let tip = fixture.commit("tip");
    let (app, cx) = open(&fixture, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            let view = app.view.read(cx);
            view.workspace_snapshot.is_some()
                && view
                    .selected
                    .as_ref()
                    .is_some_and(|(_, entry)| entry.commit.id.as_str() == tip)
                && view.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.eof
                        && snapshot.generation == view.request.generation
                        && snapshot.request_id == view.request.id
                })
        })
    });
    app.view.update(cx, |view, cx| {
        // A continuation-only lane exercises the production folded-lane heading.
        let entry = &mut view.snapshot.as_mut().unwrap().rows[0];
        let mut row = Entry {
            ordinal: entry.ordinal,
            commit: entry.commit.clone(),
            graph: entry.graph.clone(),
        };
        row.graph.width = 2;
        row.graph.edges = vec![gitk_rs::graph::Edge {
            from: 1,
            to: 1,
            parent: false,
            color: 1,
        }]
        .into_boxed_slice();
        *entry = Arc::new(row);
        view.operation_message =
            "branch switched; details for /CaseSensitive/Repository.git ".repeat(6);
        cx.notify();
    });
    for dimensions in [size(px(1280.), px(850.)), size(px(740.), px(500.))] {
        cx.simulate_resize(dimensions);
        cx.refresh().unwrap();
        assert!(cx.update(|_, cx| app.view.read(cx).graph_layout.has_collapsed()));
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
    app.view.update(cx, |view, cx| {
        view.operation_message =
            "long diagnostic with /CaseSensitive/Repository.git details ".repeat(100);
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

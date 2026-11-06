use super::*;
use gpui_kit::{TestAppContext, VisualTestContext, test::TestWindowExt};
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

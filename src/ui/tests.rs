use super::*;
use gpui::{TestAppContext, VisualTestContext};
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
    };
    let history = HistoryEngine::spawn(request.clone());
    let workspace = WorkspaceEngine::spawn(WorkspaceRequest {
        generation: WorkspaceGeneration(1),
        repository: config.repository.clone(),
        watch: false,
    });
    let diff = DiffEngine::spawn();
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(ThemeMode::Dark, None, cx);
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
        Root::new(entity, window, cx)
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

#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
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

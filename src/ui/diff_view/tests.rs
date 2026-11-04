use crate::ui::tests::{Fixture, open, until};
use gpui::{Modifiers, TestAppContext};

#[gpui::test]
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
    let bounds = cx
        .debug_bounds("diff-file-2")
        .expect("second file rendered");
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
    // Focus the real diff context, then dispatch its production key bindings.
    cx.update(|window, cx| pane.read(cx).focus.focus(window));
    cx.simulate_keystrokes("ctrl-a ctrl-c");
    until(cx, |cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|s| s.contains("+\tlet value_399 = 399;"))
    });
    cx.simulate_keystrokes("ctrl-shift-c");
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

#[gpui::test]
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

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
        cx.update(|_, cx| pane.read(cx).viewport.start > 200)
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

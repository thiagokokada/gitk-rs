use gitk_rs::cli::{Config, Theme};
use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, rgb, size,
};

actions!(gitk, [Quit]);

struct HistoryWindow {
    config: Config,
    focus: FocusHandle,
}

impl Render for HistoryWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground, panel, border) = match self.config.theme {
            Theme::Dark => (0x202329, 0xe0e4eb, 0x292d35, 0x454b57),
            Theme::Light => (0xf5f6f8, 0x202329, 0xffffff, 0xc4c9d2),
        };
        let section = || {
            div()
                .p_3()
                .bg(rgb(panel))
                .border_1()
                .border_color(rgb(border))
        };
        div()
            .track_focus(&self.focus)
            .on_action(|_: &Quit, _, cx| cx.quit())
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .child(div().child(format!("Repository: {}", self.config.repository.display())))
            .child(
                section()
                    .h(px(240.))
                    .child(
                        "Graph                 Commit                 Author                 Date",
                    )
                    .child(
                        div()
                            .mt_3()
                            .child("History loading is not implemented yet."),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .gap_2()
                    .child(
                        section()
                            .flex_1()
                            .child("Select a commit to view its details."),
                    )
                    .child(section().w(px(280.)).child("Changed files")),
            )
            .child(section().child("Development scaffold - Ctrl+Q to quit"))
    }
}

pub fn run(config: Config) {
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        if let Err(error) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("gitk-rs");
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    focus.focus(window);
                    HistoryWindow { config, focus }
                })
            },
        ) {
            eprintln!("gitk-rs: unable to open window: {error}");
            cx.quit();
        }
        cx.activate(true);
    });
}

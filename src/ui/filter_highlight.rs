use super::*;
use gitk_rs::search::filter_ranges;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{
    InputGroup, InputGroupAddon, InputGroupAddonAlignment, InputGroupButton, TextDecoration,
    Textarea,
};
use gpui_kit::{EntityInputHandler as _, FontWeight, HighlightStyle, Role};

impl HistoryWindow {
    pub(super) fn refresh_filter_highlights(&mut self, cx: &mut Context<Self>) {
        let palette = Palette::for_theme(self.config.theme);
        self.filter.update(cx, |input, cx| {
            let tag_style = HighlightStyle {
                color: Some(rgb(palette.filter_tag).into()),
                font_weight: Some(FontWeight::SEMIBOLD),
                ..Default::default()
            };
            let value_style = HighlightStyle {
                color: Some(rgb(palette.filter_value).into()),
                ..Default::default()
            };
            let decorations = filter_ranges(input.value().as_ref())
                .into_iter()
                .flat_map(|range| {
                    [
                        TextDecoration::new(range.tag, tag_style),
                        TextDecoration::new(range.value, value_style),
                    ]
                })
                .filter(|decoration| !decoration.range.is_empty())
                .collect();
            input.set_decorations(decorations, cx);
        });
    }

    pub(super) fn filter_control(&self, cx: &mut Context<Self>) -> AnyElement {
        // Textarea's decorations style editable ranges without turning tags into
        // atomic tokens. Keep the search field one row high and submit on Enter.
        let filter = self.filter.clone();
        InputGroup::new("history-filter")
            .small()
            .h_8()
            .justify_center()
            .input(
                Textarea::new(&self.filter)
                    // Override InputGroup's multiline minimum for this single row.
                    .min_h_0()
                    .h_auto()
                    .role(Role::TextInput)
                    .on_paste(move |clipboard, window, cx| {
                        let Some(text) = clipboard.text() else {
                            return false;
                        };
                        filter.update(cx, |input, cx| {
                            input.replace_text_in_range(
                                None,
                                &text.replace(['\r', '\n'], ""),
                                window,
                                cx,
                            );
                        });
                        true
                    }),
            )
            .when(!self.filter.read(cx).value().is_empty(), |group| {
                group.addon(
                    InputGroupAddon::new("filter-clear-addon")
                        .align(InputGroupAddonAlignment::InlineEnd)
                        .child(
                            InputGroupButton::new("clean")
                                .icon(gpui_kit::assets::IconName::X)
                                .accessibility_label("Clear filter")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.filter.update(cx, |input, cx| {
                                        input.set_value("", window, cx);
                                        input.focus(window, cx);
                                    });
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

use gpui_kit::{
    FontWeight, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, Window, div, px,
};

use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, dialog::DialogFooter, h_flex,
    v_flex,
};

pub(super) fn open_about_dialog(window: &mut Window, cx: &mut gpui_kit::App) {
    window.open_dialog(cx, |dialog, _window, cx| {
        let muted = cx.theme().muted_foreground;
        dialog
            .title("About")
            .child(
                v_flex()
                    .w(px(380.))
                    .gap_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(IconName::SquareTerminal).small())
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Oxidal"))
                            .child(
                                div()
                                    .text_color(muted)
                                    .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(cx.theme().muted)
                                    .text_xs()
                                    .child("Community Edition"),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Cross-platform SSH, SFTP and serial terminal client."),
                    )
                    .child(
                        h_flex()
                            .id("about-repo")
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(cx.theme().primary)
                            .child(Icon::new(IconName::Github).xsmall())
                            .child(div().underline().child("github.com/sh4den/Oxidal"))
                            .on_click(|_, _, _| {
                                let _ = open::that_detached("https://github.com/sh4den/Oxidal");
                            }),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Core maintainers"),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(Icon::new(IconName::User).xsmall())
                                    .child("𝑺𝒉𝒂𝒅𝒆𝒏")
                                    .child(
                                        div()
                                            .id("about-maintainer")
                                            .cursor_pointer()
                                            .underline()
                                            .text_color(cx.theme().primary)
                                            .child("@sh4den")
                                            .on_click(|_, _, _| {
                                                let _ = open::that_detached(
                                                    "https://github.com/sh4den",
                                                );
                                            }),
                                    ),
                            ),
                    ),
            )
            .footer(
                DialogFooter::new().child(Button::new("close-about").label("Close").on_click(
                    |_, window, cx| {
                        window.close_dialog(cx);
                    },
                )),
            )
    });
}

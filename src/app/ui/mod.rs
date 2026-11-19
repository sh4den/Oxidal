use crate::settings::{self, AppSettings};

use gpui_kit::{
    Context, IntoElement, ParentElement as _, Render, StyleRefinement, Styled as _, Window, div, px,
};

use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    resizable::{h_resizable, resizable_panel},
    v_flex,
};

use crate::app::OxidalApp;

mod about;
mod session_list;
mod sidebar;
mod title_bar;
mod welcome;
mod workspace;

pub(super) use sidebar::SidebarPane;

impl Render for OxidalApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child({
                let sidebar = (!self.sidebar_collapsed).then(|| {
                    self.sidebar_pane(cx)
                        .cached(StyleRefinement::default().size_full())
                        .into_any_element()
                });

                let content = h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar_rail(cx));
                match sidebar {
                    // Both modes share one split, so a width set on either side
                    // is the width the other opens at.
                    Some(sidebar) => content.child(
                        div().flex_1().min_w_0().h_full().child(
                            h_resizable("sidebar-split")
                                .with_state(&self.sidebar_state)
                                .child(
                                    resizable_panel()
                                        .size(px(cx.global::<AppSettings>().sidebar_width))
                                        .size_range(
                                            px(settings::SIDEBAR_MIN_WIDTH)
                                                ..px(settings::SIDEBAR_MAX_WIDTH),
                                        )
                                        .child(sidebar),
                                )
                                .child(self.render_workspace(cx).into_any_element())
                                // Fires once on mouse up, so this is a write per
                                // drag rather than per frame.
                                .on_resize(|state, _, cx| {
                                    let Some(width) = state.read(cx).sizes().first().copied()
                                    else {
                                        return;
                                    };
                                    let width = f32::from(width).clamp(
                                        settings::SIDEBAR_MIN_WIDTH,
                                        settings::SIDEBAR_MAX_WIDTH,
                                    );
                                    if cx.global::<AppSettings>().sidebar_width != width {
                                        cx.global_mut::<AppSettings>().sidebar_width = width;
                                        settings::save_settings(cx.global::<AppSettings>());
                                    }
                                }),
                        ),
                    ),
                    None => content.child(self.render_workspace(cx).into_any_element()),
                }
            })
    }
}

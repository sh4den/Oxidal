use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Root, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, actions, canvas, div, px, size,
};

actions!(dialog_window, [Confirm]);

const INITIAL_HEIGHT: f32 = 220.;
const MAX_HEIGHT: f32 = 760.;

type Callback = Rc<dyn Fn(&mut Window, &mut App)>;
type RenderFn = Box<dyn Fn(DialogContent, &mut Window, &mut App) -> DialogContent>;

#[derive(Default)]
pub struct DialogContent {
    children: Vec<AnyElement>,
    buttons: Vec<AnyElement>,
    on_ok: Option<Callback>,
    on_close: Option<Callback>,
}

impl DialogContent {
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn button(mut self, button: impl IntoElement) -> Self {
        self.buttons.push(button.into_any_element());
        self
    }

    pub fn on_ok(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_ok = Some(Rc::new(f));
        self
    }

    pub fn on_close(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}

pub fn open<F>(
    title: impl Into<SharedString>,
    width: Pixels,
    cx: &mut App,
    render: F,
) -> Option<WindowHandle<Root>>
where
    F: Fn(DialogContent, &mut Window, &mut App) -> DialogContent + 'static,
{
    open_with(title, width, cx, move |_, _| render)
}

pub fn open_with<I, F>(
    title: impl Into<SharedString>,
    width: Pixels,
    cx: &mut App,
    init: I,
) -> Option<WindowHandle<Root>>
where
    I: FnOnce(&mut Window, &mut App) -> F,
    F: Fn(DialogContent, &mut Window, &mut App) -> DialogContent + 'static,
{
    let title = title.into();
    let bounds = Bounds::centered(None, size(width, px(INITIAL_HEIGHT)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title),
                ..Default::default()
            }),
            is_minimizable: false,
            is_resizable: false,
            app_id: Some(crate::APP_ID.to_string()),
            ..Default::default()
        },
        |window, cx| {
            crate::settings::apply_window_appearance(window, cx);
            let focus_handle = cx.focus_handle();
            focus_handle.focus(window, cx);
            let render: RenderFn = Box::new(init(window, cx));
            let view = cx.new(|_| DialogWindow {
                render,
                width,
                focus_handle,
                on_ok: None,
                on_close: None,
                closed: false,
                body_height: Rc::new(Cell::new(px(0.))),
            });
            window.on_window_should_close(cx, {
                let view = view.downgrade();
                move |window, cx| {
                    let _ = view.update(cx, |dialog, cx| dialog.notify_close(window, cx));
                    true
                }
            });
            cx.new(|cx| Root::new(view, window, cx))
        },
    )
    .ok()
}

struct DialogWindow {
    render: RenderFn,
    width: Pixels,
    focus_handle: FocusHandle,
    on_ok: Option<Callback>,
    on_close: Option<Callback>,
    closed: bool,
    body_height: Rc<Cell<Pixels>>,
}

impl DialogWindow {
    fn notify_close(&mut self, window: &mut Window, cx: &mut App) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(on_close) = self.on_close.take() {
            on_close(window, cx);
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        self.notify_close(window, cx);
        window.remove_window();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut App) {
        if self.closed {
            return;
        }
        let Some(on_ok) = self.on_ok.clone() else {
            return;
        };
        self.closed = true;
        on_ok(window, cx);
        window.remove_window();
    }
}

impl Render for DialogWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = (self.render)(DialogContent::default(), window, cx);
        self.on_ok = content.on_ok;
        self.on_close = content.on_close;

        let width = self.width;
        let body_height = self.body_height.clone();
        let measured_body = body_height.clone();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .track_focus(&self.focus_handle)
            .key_context("DialogWindow")
            .on_action(cx.listener(|dialog, _: &Confirm, window, cx| dialog.confirm(window, cx)))
            .on_key_down(cx.listener(|dialog, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => dialog.dismiss(window, cx),
                    "enter" => dialog.confirm(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id("dialog-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .relative()
                            .w_full()
                            .p_4()
                            .gap_3()
                            .text_sm()
                            .children(content.children)
                            .child(
                                canvas(
                                    move |bounds, _, _| measured_body.set(bounds.size.height),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .flex_none()
                    .justify_end()
                    .gap_2()
                    .when(!content.buttons.is_empty(), |this| {
                        this.p_3().border_t_1().border_color(cx.theme().border)
                    })
                    .children(content.buttons)
                    .child(
                        canvas(
                            move |bounds, window, _| {
                                let wanted =
                                    (body_height.get() + bounds.size.height).min(px(MAX_HEIGHT));
                                if (wanted - window.viewport_size().height).abs() > px(0.5) {
                                    window.resize(size(width, wanted));
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    ),
            )
    }
}

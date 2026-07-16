use gpui::{
    actions, canvas, div, fill, hsla, point, prelude::FluentBuilder as _, px, size, Bounds,
    Context, FocusHandle, Font, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, PaintQuad, ParentElement as _, Pixels, Point, Render, ShapedLine, SharedString,
    StrikethroughStyle, Styled as _, TextAlign, TextRun, UnderlineStyle, Window,
};

use super::backend::{Backend, BackendEvent};
use super::grid::{default_bg, Grid};
use crate::settings::AppSettings;

actions!(terminal, [SendTab, SendTabPrev]);

fn cursor_fg() -> Hsla {
    hsla(0., 0., 0.08, 1.)
}

#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

pub struct TerminalView {
    grid: Grid,
    backend: Backend,
    focus_handle: FocusHandle,
    closed_message: Option<String>,
}

impl TerminalView {
    pub fn new(
        backend: Backend,
        rows: usize,
        cols: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let events = backend.events.clone();
        cx.spawn(async move |this, cx| loop {
            match events.recv().await {
                Ok(BackendEvent::Data(bytes)) => {
                    if this
                        .update(cx, |view: &mut Self, cx| {
                            let replies = view.grid.feed(&bytes);
                            if !replies.is_empty() {
                                view.backend.write_input(&replies);
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(BackendEvent::Closed(message)) => {
                    let _ = this.update(cx, |view: &mut Self, cx| {
                        view.closed_message =
                            Some(message.unwrap_or_else(|| "Connection closed".to_string()));
                        cx.notify();
                    });
                    break;
                }
                Err(_) => break,
            }
        })
        .detach();

        cx.observe_global::<AppSettings>(|_, cx| cx.notify()).detach();

        Self {
            grid: Grid::new(rows, cols),
            backend,
            focus_handle,
            closed_message: None,
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent) {
        if let Some(bytes) = translate_key(event, self.grid.application_cursor_keys) {
            self.backend.write_input(&bytes);
        }
    }

    fn resize(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        if rows == self.grid.rows && cols == self.grid.cols {
            return;
        }
        self.grid.resize(rows, cols);
        self.backend.resize(rows as u16, cols as u16);
        cx.notify();
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<AppSettings>().clone();
        let font_family = SharedString::from(settings.font_family.clone());
        let font_size = settings.font_size.clamp(8.0, 32.0);
        let line_height = font_size * 1.43;
        let closed_message = self.closed_message.clone();

        let measure = {
            let weak = cx.entity().downgrade();
            let font_family = font_family.clone();
            move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App| {
                let run = TextRun {
                    len: 1,
                    font: gpui::font(font_family.clone()),
                    color: hsla(0., 0., 0., 1.),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from("M"),
                    px(font_size),
                    &[run],
                    None,
                );
                let char_width = shaped.width();
                if char_width > px(0.) {
                    // Hard backstop: a pane this size is never real, so
                    // clamping here means a layout regression can misbehave
                    // visually but can no longer runaway-grow every frame
                    // into a crash.
                    let cols = ((bounds.size.width / char_width).floor() as usize).clamp(10, 500);
                    let rows =
                        ((bounds.size.height / px(line_height)).floor() as usize).clamp(4, 200);
                    let _ = weak.update(cx, |view, cx| view.resize(rows, cols, cx));
                }
                char_width
            }
        };

        let paint = {
            let entity = cx.entity().clone();
            let font_family = font_family.clone();
            move |bounds: Bounds<Pixels>,
                  char_width: Pixels,
                  window: &mut Window,
                  cx: &mut gpui::App| {
                if char_width <= px(0.) {
                    return;
                }
                let base_font = gpui::font(font_family.clone());
                let (quads, lines) = {
                    let view = entity.read(cx);
                    build_paint(
                        &view.grid,
                        bounds,
                        char_width,
                        px(line_height),
                        px(font_size),
                        &base_font,
                        window,
                    )
                };
                for quad in quads {
                    window.paint_quad(quad);
                }
                for (line, origin) in lines {
                    let _ =
                        line.paint(origin, px(line_height), TextAlign::default(), None, window, cx);
                }
            }
        };

        div()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                view.handle_key(event);
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &SendTab, _window, cx| {
                view.backend.write_input(b"\t");
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &SendTabPrev, _window, cx| {
                view.backend.write_input(b"\x1b[Z");
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _event, window, cx| {
                    view.focus_handle.focus(window, cx);
                }),
            )
            .size_full()
            // Without `min_w_0`/`min_h_0`, a flex item's default min-size is
            // its content's natural size — this keeps the pane clipping
            // instead of growing when a child measures wide.
            .min_w_0()
            .min_h_0()
            .bg(default_bg())
            .text_color(hsla(0., 0., 0.9, 1.))
            .p_2()
            .font_family(font_family)
            .text_size(px(font_size))
            .line_height(px(line_height))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(canvas(measure, paint).w_full().flex_1().min_h_0())
            .when_some(closed_message, |this, msg| {
                this.child(
                    div()
                        .mt_2()
                        .text_color(hsla(0., 0.6, 0.6, 1.))
                        .child(format!("[session ended: {}]", msg)),
                )
            })
    }
}

fn build_paint(
    grid: &Grid,
    bounds: Bounds<Pixels>,
    char_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
    base_font: &Font,
    window: &Window,
) -> (Vec<PaintQuad>, Vec<(ShapedLine, Point<Pixels>)>) {
    let mut quads = Vec::new();
    let mut lines = Vec::new();

    let cursor = grid
        .cursor_visible
        .then_some((grid.cursor_row, grid.cursor_col));

    for row in 0..grid.rows {
        let y = bounds.origin.y + line_height * row as f32;

        let cell_bg = |col: usize| -> Option<Hsla> {
            let cell = grid.cell(row, col);
            if cursor == Some((row, col)) {
                Some(cell.fg.as_fg())
            } else {
                cell.bg.as_bg()
            }
        };
        let mut col = 0;
        while col < grid.cols {
            let bg = cell_bg(col);
            let start = col;
            col += 1;
            while col < grid.cols && cell_bg(col) == bg {
                col += 1;
            }
            if let Some(color) = bg {
                let origin = point(bounds.origin.x + char_width * start as f32, y);
                quads.push(fill(
                    Bounds::new(origin, size(char_width * (col - start) as f32, line_height)),
                    color,
                ));
            }
        }

        let mut text = String::new();
        let mut style: Option<RunStyle> = None;
        let mut start_col = 0;
        let mut flush = |text: &mut String, style: Option<RunStyle>, start_col: usize| {
            let Some(style) = style else {
                text.clear();
                return;
            };
            if text.trim().is_empty() && !style.underline && !style.strike {
                text.clear();
                return;
            }
            let mut font = base_font.clone();
            if style.bold {
                font = font.bold();
            }
            if style.italic {
                font = font.italic();
            }
            let run = TextRun {
                len: text.len(),
                font,
                color: style.color,
                background_color: None,
                underline: style.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(style.color),
                    wavy: false,
                }),
                strikethrough: style.strike.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(style.color),
                }),
            };
            let shaped = window.text_system().shape_line(
                SharedString::from(std::mem::take(text)),
                font_size,
                &[run],
                None,
            );
            lines.push((
                shaped,
                point(bounds.origin.x + char_width * start_col as f32, y),
            ));
        };

        for col in 0..grid.cols {
            let cell = grid.cell(row, col);
            let is_cursor = cursor == Some((row, col));
            let mut color = if is_cursor {
                cell.bg.as_bg().unwrap_or(cursor_fg())
            } else {
                cell.fg.as_fg()
            };
            if cell.dim() {
                color.a *= 0.6;
            }
            let cell_style = RunStyle {
                color,
                bold: cell.bold(),
                italic: cell.italic(),
                underline: cell.underline(),
                strike: cell.strike(),
            };
            let ch = cell.ch();
            if style != Some(cell_style) || !ch.is_ascii() {
                flush(&mut text, style.take(), start_col);
                style = Some(cell_style);
                start_col = col;
            }
            text.push(ch);
            if !ch.is_ascii() {
                flush(&mut text, style.take(), start_col);
            }
        }
        flush(&mut text, style, start_col);
    }

    (quads, lines)
}

/// Translate a raw key event into the bytes a shell/PTY expects to receive.
/// `application_cursor_keys` mirrors DECCKM (`CSI ?1h`/`l`): full-screen TUIs
/// like vim and htop switch arrow/home/end keys to the `ESC O x` form while
/// they're active.
fn translate_key(event: &KeyDownEvent, application_cursor_keys: bool) -> Option<Vec<u8>> {
    let keystroke = &event.keystroke;

    if keystroke.modifiers.control && keystroke.key.len() == 1 {
        let c = keystroke.key.chars().next()?;
        if c.is_ascii_alphabetic() {
            let byte = (c.to_ascii_uppercase() as u8) - b'A' + 1;
            return Some(vec![byte]);
        }
    }

    let app_mode = application_cursor_keys;
    let bytes: &[u8] = match keystroke.key.as_str() {
        "enter" => b"\r",
        "backspace" => b"\x7f",
        "tab" => b"\t",
        "escape" => b"\x1b",
        "space" => b" ",
        "up" if app_mode => b"\x1bOA",
        "down" if app_mode => b"\x1bOB",
        "right" if app_mode => b"\x1bOC",
        "left" if app_mode => b"\x1bOD",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" if app_mode => b"\x1bOH",
        "end" if app_mode => b"\x1bOF",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "delete" => b"\x1b[3~",
        "pageup" => b"\x1b[5~",
        "pagedown" => b"\x1b[6~",
        _ => {
            if let Some(key_char) = &keystroke.key_char {
                return Some(key_char.as_bytes().to_vec());
            }
            return None;
        }
    };
    Some(bytes.to_vec())
}

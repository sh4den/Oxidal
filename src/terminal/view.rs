use gpui::{
    div, hsla, prelude::FluentBuilder as _, px, Context, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, ParentElement as _, Render, SharedString, Styled as _,
    StyledText, TextRun, Window,
};
use gpui_component::ElementExt as _;

use super::backend::{Backend, BackendEvent};
use super::grid::Grid;
use crate::settings::AppSettings;

/// Renders a live terminal grid (local shell, SSH, or serial session) and
/// forwards keyboard input to its backend.
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
                            view.grid.feed(&bytes);
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

    /// Resize the grid (and tell the backend, so the shell/TUI gets a proper
    /// resize notification) if the pane's measured size implies a different
    /// row/column count than we currently have.
    fn resize(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        if rows == self.grid.rows && cols == self.grid.cols {
            return;
        }
        self.grid.resize(rows, cols);
        self.backend.resize(rows as u16, cols as u16);
        cx.notify();
    }

    fn render_row(&self, row: usize, font_family: SharedString, line_height: f32) -> impl IntoElement {
        let mut text = String::new();
        let mut runs: Vec<TextRun> = Vec::new();

        for col in 0..self.grid.cols {
            let mut cell = self.grid.cell(row, col);
            let is_cursor = self.grid.cursor_visible
                && self.grid.cursor_row == row
                && self.grid.cursor_col == col;
            if is_cursor {
                let fg = cell.bg.unwrap_or(hsla(0., 0., 0.08, 1.));
                let bg = cell.fg;
                cell.fg = fg;
                cell.bg = Some(bg);
            }

            let byte_len = cell.ch.len_utf8();
            text.push(cell.ch);

            let font = gpui::font(font_family.clone());
            let font = if cell.bold { font.bold() } else { font };

            if let Some(last) = runs.last_mut() {
                let same_style: &TextRun = last;
                if same_style.color == cell.fg
                    && same_style.background_color == cell.bg
                    && same_style.font == font
                {
                    last.len += byte_len;
                    continue;
                }
            }

            runs.push(TextRun {
                len: byte_len,
                font,
                color: cell.fg,
                background_color: cell.bg,
                underline: None,
                strikethrough: None,
            });
        }

        // `w_full` + `overflow_hidden` keep a row's own (exactly `grid.cols`
        // characters wide) content from ever inflating an ancestor flex
        // container's intrinsic size — see the `min_w_0` note on the
        // container below for why that matters. Text wraps by default in
        // gpui, so without `whitespace_nowrap` any tiny rounding gap between
        // our column-width estimate and the real glyph advances would wrap
        // the last character onto a second line — on every row, since a
        // fixed-width row is right at that boundary by construction. A fixed
        // row height makes that (and anything else unexpected) just clip
        // instead of pushing every following row down.
        div()
            .w_full()
            .h(px(line_height))
            .overflow_hidden()
            .whitespace_nowrap()
            .child(StyledText::new(text).with_runs(runs))
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<AppSettings>().clone();
        let font_family = SharedString::from(settings.font_family.clone());
        let font_size = settings.font_size.clamp(8.0, 32.0);
        let line_height = font_size * 1.43;

        let rows = (0..self.grid.rows)
            .map(|row| self.render_row(row, font_family.clone(), line_height))
            .collect::<Vec<_>>();
        let closed_message = self.closed_message.clone();

        let weak = cx.entity().downgrade();
        let measure_font_family = font_family.clone();

        div()
            .track_focus(&self.focus_handle)
            .key_context("Terminal")
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                view.handle_key(event);
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
            // its content's natural size — so once a row's text got even
            // slightly wider than the visible pane, this container would
            // grow to fit it instead of clipping, `on_prepaint` would then
            // measure that *inflated* size and add even more columns next
            // frame, compounding every frame until it crashed.
            .min_w_0()
            .min_h_0()
            .bg(hsla(0., 0., 0.07, 1.))
            .text_color(hsla(0., 0., 0.9, 1.))
            .p_2()
            .font_family(font_family.clone())
            .text_size(px(font_size))
            .line_height(px(line_height))
            .overflow_hidden()
            .on_prepaint(move |bounds, window, cx| {
                let run = TextRun {
                    len: 1,
                    font: gpui::font(measure_font_family.clone()),
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
                if char_width <= px(0.) {
                    return;
                }
                // Hard backstop: a pane this size is never real, so clamping
                // here means a layout regression can misbehave visually but
                // can no longer runaway-grow every frame into a crash.
                let cols = ((bounds.size.width / char_width).floor() as usize).clamp(10, 500);
                let rows = ((bounds.size.height / px(line_height)).floor() as usize).clamp(4, 200);
                let _ = weak.update(cx, |view, cx| view.resize(rows, cols, cx));
            })
            .children(rows)
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

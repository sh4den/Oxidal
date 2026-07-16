use std::collections::VecDeque;

use gpui::{
    actions, canvas, div, fill, hsla, point, prelude::FluentBuilder as _, px, relative, size,
    AnyElement, Bounds, ClipboardItem, Context, Div, FocusHandle, Font, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle, Styled as _, TextAlign,
    TextRun, UnderlineStyle, Window,
};
use gpui_component::{Icon, IconName, Sizable as _};

use super::backend::{Backend, BackendEvent};
use super::grid::{default_bg, Grid};
use super::stats::RemoteStats;
use crate::settings::AppSettings;

const CPU_HISTORY_LEN: usize = 30;

actions!(terminal, [SendTab, SendTabPrev, CopySelection, PasteClipboard]);

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

#[derive(Clone, Copy)]
struct Selection {
    anchor: (usize, usize),
    head: (usize, usize),
    dragging: bool,
}

impl Selection {
    fn range(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

fn selection_bg() -> Hsla {
    hsla(215. / 360., 0.45, 0.32, 1.)
}

pub struct TerminalView {
    grid: Grid,
    backend: Backend,
    focus_handle: FocusHandle,
    closed_message: Option<String>,
    monitored: bool,
    stats: Option<RemoteStats>,
    cpu_history: VecDeque<f32>,
    selection: Option<Selection>,
    layout: Option<(Bounds<Pixels>, Pixels, Pixels)>,
}

impl TerminalView {
    pub fn new(
        backend: Backend,
        rows: usize,
        cols: usize,
        stats_rx: Option<async_channel::Receiver<RemoteStats>>,
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

        let monitored = stats_rx.is_some();
        if let Some(rx) = stats_rx {
            cx.spawn(async move |this, cx| {
                while let Ok(stats) = rx.recv().await {
                    if this
                        .update(cx, |view: &mut Self, cx| {
                            if let Some(cpu) = stats.cpu {
                                view.cpu_history.push_back(cpu);
                                while view.cpu_history.len() > CPU_HISTORY_LEN {
                                    view.cpu_history.pop_front();
                                }
                            }
                            view.stats = Some(stats);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }

        Self {
            grid: Grid::new(rows, cols),
            backend,
            focus_handle,
            closed_message: None,
            monitored,
            stats: None,
            cpu_history: VecDeque::new(),
            selection: None,
            layout: None,
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
        self.selection = None;
        cx.notify();
    }

    fn cell_at(&self, position: Point<Pixels>, clamp: bool) -> Option<(usize, usize)> {
        let (bounds, char_width, line_height) = self.layout?;
        if char_width <= px(0.) {
            return None;
        }
        if !clamp && !bounds.contains(&position) {
            return None;
        }
        let col = ((position.x - bounds.origin.x) / char_width).floor() as isize;
        let row = ((position.y - bounds.origin.y) / line_height).floor() as isize;
        Some((
            row.clamp(0, self.grid.rows as isize - 1) as usize,
            col.clamp(0, self.grid.cols as isize - 1) as usize,
        ))
    }

    fn send_mouse(&mut self, button: u8, row: usize, col: usize, press: bool, drag: bool) {
        if self.grid.mouse_mode == 0 {
            return;
        }
        let code = button + if drag { 32 } else { 0 };
        let bytes = if self.grid.mouse_sgr {
            format!(
                "\x1b[<{};{};{}{}",
                code,
                col + 1,
                row + 1,
                if press { 'M' } else { 'm' }
            )
            .into_bytes()
        } else {
            let cb = 32 + if press { code } else { 3 };
            let cx = 32 + (col + 1).min(223) as u8;
            let cy = 32 + (row + 1).min(223) as u8;
            vec![0x1b, b'[', b'M', cb, cx, cy]
        };
        self.backend.write_input(&bytes);
    }

    fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection.as_ref()?.range();
        let mut out = String::new();
        for row in start.0..=end.0.min(self.grid.rows - 1) {
            let from = if row == start.0 { start.1 } else { 0 };
            let to = if row == end.0 { end.1 } else { self.grid.cols - 1 };
            let mut line: String = (from..=to.min(self.grid.cols - 1))
                .map(|col| self.grid.cell(row, col).ch())
                .collect();
            while line.ends_with(' ') {
                line.pop();
            }
            if row != start.0 {
                out.push('\n');
            }
            out.push_str(&line);
        }
        Some(out)
    }

    fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.grid.bracketed_paste {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.backend.write_input(&bytes);
        } else {
            self.backend.write_input(text.as_bytes());
        }
    }

    fn end_drag(&mut self) {
        if let Some(selection) = self.selection.as_mut() {
            selection.dragging = false;
        }
        if self.selection.is_some_and(|s| s.anchor == s.head) {
            self.selection = None;
        }
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
                    let _ = weak.update(cx, |view, cx| {
                        view.layout = Some((bounds, char_width, px(line_height)));
                        view.resize(rows, cols, cx);
                    });
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
                    let selection = view.selection.map(|s| s.range());
                    build_paint(
                        &view.grid,
                        selection,
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
            .on_action(cx.listener(|view, _: &CopySelection, _window, cx| {
                if let Some(text) = view.selected_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }))
            .on_action(cx.listener(|view, _: &PasteClipboard, _window, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    view.paste(&text);
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    view.focus_handle.focus(window, cx);
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, false) {
                            view.send_mouse(0, row, col, true, false);
                        }
                        view.selection = None;
                    } else {
                        view.selection =
                            view.cell_at(event.position, false).map(|cell| Selection {
                                anchor: cell,
                                head: cell,
                                dragging: true,
                            });
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, false) {
                            view.send_mouse(1, row, col, true, false);
                        }
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, false) {
                            view.send_mouse(2, row, col, true, false);
                        }
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _window, cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, true) {
                            view.send_mouse(0, row, col, false, false);
                        }
                    }
                    view.end_drag();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, true) {
                            view.send_mouse(1, row, col, false, false);
                        }
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, true) {
                            view.send_mouse(2, row, col, false, false);
                        }
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _event: &MouseUpEvent, _window, cx| {
                    view.end_drag();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    let cell = view.cell_at(event.position, true);
                    if let (Some(cell), Some(selection)) = (cell, view.selection.as_mut()) {
                        if selection.dragging && selection.head != cell {
                            selection.head = cell;
                            cx.notify();
                        }
                    }
                }
                if view.grid.mouse_mode >= 1002 && !event.modifiers.shift {
                    let code = match event.pressed_button {
                        Some(MouseButton::Left) => 0,
                        Some(MouseButton::Middle) => 1,
                        Some(MouseButton::Right) => 2,
                        _ => return,
                    };
                    if let Some((row, col)) = view.cell_at(event.position, true) {
                        view.send_mouse(code, row, col, true, true);
                    }
                }
            }))
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, cx| {
                let Some((_, _, line_height)) = view.layout else {
                    return;
                };
                let dy = event.delta.pixel_delta(line_height).y;
                if dy == px(0.) {
                    return;
                }
                let lines = ((dy.abs() / line_height).ceil() as usize).clamp(1, 5);
                let up = dy > px(0.);
                if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                    if let Some((row, col)) = view.cell_at(event.position, true) {
                        let button = if up { 64 } else { 65 };
                        for _ in 0..lines {
                            view.send_mouse(button, row, col, true, false);
                        }
                    }
                } else if view.grid.alt_active() {
                    let seq: &[u8] = match (up, view.grid.application_cursor_keys) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    for _ in 0..lines {
                        view.backend.write_input(seq);
                    }
                }
                cx.notify();
            }))
            .size_full()
            // Without `min_w_0`/`min_h_0`, a flex item's default min-size is
            // its content's natural size — this keeps the pane clipping
            // instead of growing when a child measures wide.
            .min_w_0()
            .min_h_0()
            .bg(default_bg())
            .text_color(hsla(0., 0., 0.9, 1.))
            .font_family(font_family)
            .text_size(px(font_size))
            .line_height(px(line_height))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .p_2()
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
                    }),
            )
            .when(self.monitored, |this| this.child(self.render_monitor_bar()))
    }
}

impl TerminalView {
    fn render_monitor_bar(&self) -> AnyElement {
        let mut items: Vec<AnyElement> = Vec::new();
        if let Some(stats) = &self.stats {
            if let Some(sysname) = &stats.sysname {
                items.push(segment(IconName::Globe).child(sysname.clone()).into_any_element());
            }
            items.push(
                segment(IconName::Cpu)
                    .child(cpu_chart(&self.cpu_history))
                    .when_some(stats.cpu, |this, cpu| {
                        this.child(format!("{:.0}%", cpu * 100.))
                    })
                    .into_any_element(),
            );
            if let Some((used, total)) = stats.mem {
                let frac = used as f32 / total.max(1) as f32;
                items.push(
                    segment(IconName::MemoryStick)
                        .child(meter_bar(frac))
                        .child(format!("{}/{}", fmt_size(used), fmt_size(total)))
                        .into_any_element(),
                );
            }
            if let Some((rx, tx)) = stats.net {
                items.push(segment(IconName::ArrowUp).child(fmt_rate(tx)).into_any_element());
                items.push(segment(IconName::ArrowDown).child(fmt_rate(rx)).into_any_element());
            }
            if let Some(user) = &stats.user {
                items.push(segment(IconName::User).child(user.clone()).into_any_element());
            }
            if let Some((used, total)) = stats.disk {
                let frac = used as f32 / total.max(1) as f32;
                items.push(
                    segment(IconName::HardDrive)
                        .child(meter_bar(frac))
                        .child(format!("{:.0}%", frac * 100.))
                        .into_any_element(),
                );
            }
        }
        if items.is_empty() {
            items.push(
                div()
                    .text_color(hsla(0., 0., 0.45, 1.))
                    .child(Icon::new(IconName::Loader).xsmall())
                    .into_any_element(),
            );
        }

        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_4()
            .h(px(24.))
            .px_2()
            .text_xs()
            .text_color(hsla(0., 0., 0.75, 1.))
            .bg(hsla(0., 0., 0.11, 1.))
            .border_t_1()
            .border_color(hsla(0., 0., 0.2, 1.))
            .overflow_hidden()
            .children(items)
            .into_any_element()
    }
}

fn segment(icon: IconName) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(div().text_color(hsla(0., 0., 0.5, 1.)).child(Icon::new(icon).xsmall()))
}

fn cpu_chart(history: &VecDeque<f32>) -> AnyElement {
    div()
        .flex()
        .items_end()
        .justify_end()
        .gap(px(1.))
        .w(px(CPU_HISTORY_LEN as f32 * 3.))
        .h(px(14.))
        .rounded_sm()
        .overflow_hidden()
        .bg(hsla(0., 0., 0.16, 1.))
        .children(history.iter().map(|&value| {
            let value = value.clamp(0., 1.);
            div()
                .w(px(2.))
                .h(px((13. * value).max(1.)))
                .bg(meter_color(value))
        }))
        .into_any_element()
}

fn meter_bar(frac: f32) -> AnyElement {
    let frac = frac.clamp(0., 1.);
    div()
        .w(px(40.))
        .h(px(5.))
        .rounded_full()
        .overflow_hidden()
        .bg(hsla(0., 0., 0.22, 1.))
        .child(div().w(relative(frac)).h_full().bg(meter_color(frac)))
        .into_any_element()
}

fn meter_color(frac: f32) -> Hsla {
    if frac < 0.7 {
        hsla(120. / 360., 0.45, 0.45, 1.)
    } else if frac < 0.9 {
        hsla(40. / 360., 0.6, 0.5, 1.)
    } else {
        hsla(0., 0.55, 0.5, 1.)
    }
}

fn fmt_size(bytes: u64) -> String {
    const G: f64 = 1024. * 1024. * 1024.;
    const M: f64 = 1024. * 1024.;
    let bytes = bytes as f64;
    if bytes >= G {
        format!("{:.1}G", bytes / G)
    } else if bytes >= M {
        format!("{:.0}M", bytes / M)
    } else {
        format!("{:.0}K", bytes / 1024.)
    }
}

fn fmt_rate(bytes_per_sec: f64) -> String {
    format!("{}/s", fmt_size(bytes_per_sec.max(0.) as u64))
}

fn build_paint(
    grid: &Grid,
    selection: Option<((usize, usize), (usize, usize))>,
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
            } else if selection
                .is_some_and(|(start, end)| (row, col) >= start && (row, col) <= end)
            {
                Some(selection_bg())
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

use std::collections::VecDeque;
use std::time::Duration;

use gpui::{
    Anchor, AnyElement, App, Bounds, ClipboardItem, Context, Div, FocusHandle, Font, FontWeight,
    Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement as _, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle, Styled as _, TextAlign,
    TextRun, UnderlineStyle, Window, actions, canvas, div, fill, hsla, point,
    prelude::FluentBuilder as _, px, relative, size,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, hover_card::HoverCard};

use super::backend::{Backend, BackendEvent};
use super::grid::{Cell, Grid, default_bg};
use super::stats::{DiskInfo, RemoteStats};
use crate::settings::AppSettings;

const CPU_HISTORY_LEN: usize = 30;

actions!(
    terminal,
    [SendTab, SendTabPrev, CopySelection, CutSelection, PasteClipboard]
);

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
    scroll_offset: usize,
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
        cx.spawn(async move |this, cx| {
            loop {
                match events.recv().await {
                    Ok(BackendEvent::Data(bytes)) => {
                        if this
                            .update(cx, |view: &mut Self, cx| {
                                let top_before = view.grid.screen_top_line();
                                let replies = view.grid.feed(&bytes);
                                if !replies.is_empty() {
                                    view.backend.write_input(&replies);
                                }
                                if view.scroll_offset > 0 {
                                    let pushed = view.grid.screen_top_line() - top_before;
                                    view.scroll_offset = (view.scroll_offset + pushed)
                                        .min(view.grid.scrollback_len());
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
            }
        })
        .detach();

        cx.observe_global::<AppSettings>(|_, cx| cx.notify())
            .detach();

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
            scroll_offset: 0,
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent) {
        if event.keystroke.modifiers.shift && !self.grid.alt_active() {
            let page = self.grid.rows.max(1);
            match event.keystroke.key.as_str() {
                "pageup" => {
                    self.scroll_lines(page as isize);
                    return;
                }
                "pagedown" => {
                    self.scroll_lines(-(page as isize));
                    return;
                }
                _ => {}
            }
        }
        if let Some(bytes) = translate_key(event, self.grid.application_cursor_keys) {
            self.scroll_offset = 0;
            self.selection = None;
            self.backend.write_input(&bytes);
        }
    }

    fn scroll_lines(&mut self, delta: isize) {
        self.scroll_offset = self
            .scroll_offset
            .saturating_add_signed(delta)
            .min(self.grid.scrollback_len());
    }

    fn line_at(&self, visual_row: usize) -> usize {
        (self.grid.screen_top_line() + visual_row).saturating_sub(self.scroll_offset)
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
        let row = row.saturating_sub(self.scroll_offset);
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
        for line_id in start.0..=end.0 {
            let Some(cells) = self.grid.line_cells(line_id) else {
                continue;
            };
            let last = cells.len().saturating_sub(1);
            let from = if line_id == start.0 { start.1 } else { 0 };
            let to = if line_id == end.0 {
                end.1.min(last)
            } else {
                last
            };
            let mut line: String = cells
                .get(from..=to)
                .unwrap_or_default()
                .iter()
                .map(|cell| cell.ch())
                .collect();
            while line.ends_with(' ') {
                line.pop();
            }
            if line_id != start.0 {
                out.push('\n');
            }
            out.push_str(&line);
        }
        Some(out)
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.selected_text().filter(|text| !text.is_empty()) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.selection = None;
        true
    }

    fn paste(&mut self, text: &str) {
        self.scroll_offset = 0;
        self.selection = None;
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

        if self.grid.alt_active() {
            self.scroll_offset = 0;
        }
        self.scroll_offset = self.scroll_offset.min(self.grid.scrollback_len());
        let scroll_offset = self.scroll_offset;

        let surface_opacity = {
            let opacity = settings.opacity.clamp(0.3, 1.0);
            if opacity < 1.0 && cx.theme().mode.is_dark() {
                0.
            } else {
                opacity
            }
        };

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
                        view.scroll_offset.min(view.grid.scrollback_len()),
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
                    let _ = line.paint(
                        origin,
                        px(line_height),
                        TextAlign::default(),
                        None,
                        window,
                        cx,
                    );
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
                view.scroll_offset = 0;
                view.backend.write_input(b"\t");
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &SendTabPrev, _window, cx| {
                view.scroll_offset = 0;
                view.backend.write_input(b"\x1b[Z");
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CopySelection, _window, cx| {
                if !view.copy_selection(cx) {
                    view.scroll_offset = 0;
                    view.backend.write_input(b"\x03");
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CutSelection, _window, cx| {
                if !view.copy_selection(cx) {
                    view.scroll_offset = 0;
                    view.backend.write_input(b"\x18");
                }
                cx.notify();
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
                        view.selection = view.cell_at(event.position, false).map(|(row, col)| {
                            let cell = (view.line_at(row), col);
                            Selection {
                                anchor: cell,
                                head: cell,
                                dragging: true,
                            }
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
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                        if let Some((row, col)) = view.cell_at(event.position, false) {
                            view.send_mouse(2, row, col, true, false);
                        }
                        return;
                    }
                    view.focus_handle.focus(window, cx);
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        view.paste(&text);
                    }
                    cx.notify();
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
                    let cell = view
                        .cell_at(event.position, true)
                        .map(|(row, col)| (view.line_at(row), col));
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
                } else {
                    let lines = lines as isize;
                    view.scroll_lines(if up { lines } else { -lines });
                }
                cx.notify();
            }))
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(default_bg().opacity(surface_opacity))
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
                    .relative()
                    .child(canvas(measure, paint).w_full().flex_1().min_h_0())
                    .when(scroll_offset > 0, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top(px(6.))
                                .right(px(14.))
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(hsla(0., 0., 0.16, 0.9))
                                .text_xs()
                                .text_color(hsla(0., 0., 0.7, 1.))
                                .child(if scroll_offset == 1 {
                                    "1 line up".to_string()
                                } else {
                                    format!("{scroll_offset} lines up")
                                }),
                        )
                    })
                    .when_some(closed_message, |this, msg| {
                        this.child(
                            div()
                                .mt_2()
                                .text_color(hsla(0., 0.6, 0.6, 1.))
                                .child(format!("[session ended: {}]", msg)),
                        )
                    }),
            )
            .when(self.monitored, |this| {
                this.child(self.render_monitor_bar(surface_opacity, cx))
            })
    }
}

impl TerminalView {
    fn render_monitor_bar(&self, surface_opacity: f32, cx: &App) -> AnyElement {
        let mut items: Vec<AnyElement> = Vec::new();
        if let Some(stats) = &self.stats {
            if let Some(sysname) = &stats.sysname {
                items.push(
                    segment(IconName::Globe)
                        .child(sysname.clone())
                        .into_any_element(),
                );
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
                items.push(
                    segment_colored(IconName::ArrowUp, hsla(0.75, 0.6, 0.65, 1.))
                        .child(rate_cell(tx))
                        .into_any_element(),
                );
                items.push(
                    segment_colored(IconName::ArrowDown, hsla(0.36, 0.55, 0.5, 1.))
                        .child(rate_cell(rx))
                        .into_any_element(),
                );
            }
            if let Some((count, port)) = stats.connections {
                items.push(
                    segment(IconName::Network)
                        .child(format!("Connections: {count} (port {port})"))
                        .into_any_element(),
                );
            }
            if let Some(user) = &stats.user {
                items.push(
                    HoverCard::new("monitor-who")
                        .anchor(Anchor::BottomLeft)
                        .open_delay(Duration::from_millis(250))
                        .trigger(segment(IconName::User).child(user.clone()))
                        .child(who_details(&stats.who, cx))
                        .into_any_element(),
                );
            }
            if let Some((used, total)) = stats.disk {
                let frac = used as f32 / total.max(1) as f32;
                items.push(
                    HoverCard::new("monitor-disks")
                        .anchor(Anchor::BottomLeft)
                        .open_delay(Duration::from_millis(250))
                        .trigger(
                            segment(IconName::HardDrive)
                                .child(meter_bar(frac))
                                .child(format!("{:.0}%", frac * 100.)),
                        )
                        .child(disk_details(&stats.disks, cx))
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
            .bg(hsla(0., 0., 0.11, 1.).opacity(surface_opacity))
            .border_t_1()
            .border_color(hsla(0., 0., 0.2, 1.))
            .overflow_hidden()
            .children(items)
            .into_any_element()
    }
}

fn segment(icon: IconName) -> Div {
    segment_colored(icon, hsla(0., 0., 0.5, 1.))
}

fn segment_colored(icon: IconName, color: Hsla) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .child(div().text_color(color).child(Icon::new(icon).xsmall()))
}

fn rate_cell(bytes_per_sec: f64) -> Div {
    div()
        .flex_none()
        .w(px(72.))
        .whitespace_nowrap()
        .text_right()
        .child(fmt_rate(bytes_per_sec))
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

fn who_details(who: &[String], cx: &App) -> AnyElement {
    const MAX_ROWS: usize = 10;
    let muted = cx.theme().muted_foreground;

    let mut rows: Vec<AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_color(muted)
            .child(Icon::new(IconName::User).xsmall())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Logged in users"),
            )
            .into_any_element(),
    ];

    for line in who.iter().take(MAX_ROWS) {
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or_default().to_string();
        let tty = fields.next().unwrap_or_default().to_string();
        let rest = fields.collect::<Vec<_>>().join(" ");
        rows.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(div().flex_none().text_color(muted).child(tty))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_right()
                        .text_color(muted)
                        .child(rest),
                )
                .into_any_element(),
        );
    }

    if who.len() > MAX_ROWS {
        rows.push(
            div()
                .text_color(muted)
                .child(format!("+{} more sessions", who.len() - MAX_ROWS))
                .into_any_element(),
        );
    }
    if who.is_empty() {
        rows.push(
            div()
                .text_color(muted)
                .child("No sessions reported")
                .into_any_element(),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_2()
        .w(px(300.))
        .text_xs()
        .children(rows)
        .into_any_element()
}

fn disk_details(disks: &[DiskInfo], cx: &App) -> AnyElement {
    const MAX_ROWS: usize = 8;
    let muted = cx.theme().muted_foreground;
    let track = cx.theme().muted;

    let mut rows: Vec<AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_color(muted)
            .child(Icon::new(IconName::HardDrive).xsmall())
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Storage"))
            .into_any_element(),
    ];

    for disk in disks.iter().take(MAX_ROWS) {
        let frac = (disk.used as f32 / disk.total.max(1) as f32).clamp(0., 1.);
        rows.push(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(disk.mount.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(meter_color(frac))
                                .child(format!("{:.0}%", frac * 100.)),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(track)
                        .child(
                            div()
                                .w(relative(frac))
                                .h_full()
                                .rounded_full()
                                .bg(meter_color(frac)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_color(muted)
                        .child(div().flex_none().child(format!(
                            "{} of {}",
                            fmt_size(disk.used),
                            fmt_size(disk.total)
                        )))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_right()
                                .child(disk.filesystem.clone()),
                        ),
                )
                .into_any_element(),
        );
    }

    if disks.len() > MAX_ROWS {
        rows.push(
            div()
                .text_color(muted)
                .child(format!("+{} more filesystems", disks.len() - MAX_ROWS))
                .into_any_element(),
        );
    }
    if disks.is_empty() {
        rows.push(
            div()
                .text_color(muted)
                .child("No disk details reported")
                .into_any_element(),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .w(px(280.))
        .text_xs()
        .children(rows)
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
    let mbps = bytes_per_sec.max(0.) * 8. / 1_000_000.;
    if mbps >= 1000. {
        format!("{:.2} Gb/s", mbps / 1000.)
    } else if mbps >= 100. {
        format!("{mbps:.0} Mb/s")
    } else if mbps >= 10. {
        format!("{mbps:.1} Mb/s")
    } else {
        format!("{mbps:.2} Mb/s")
    }
}

fn build_paint(
    grid: &Grid,
    scroll_offset: usize,
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

    let top_line = grid.screen_top_line().saturating_sub(scroll_offset);
    let cursor = grid
        .cursor_visible
        .then_some((grid.screen_top_line() + grid.cursor_row, grid.cursor_col));

    for row in 0..grid.rows {
        let line_id = top_line + row;
        let Some(cells) = grid.line_cells(line_id) else {
            continue;
        };
        let y = bounds.origin.y + line_height * row as f32;
        let cell = |col: usize| -> Cell { cells.get(col).copied().unwrap_or(Cell::BLANK) };

        let cell_bg = |col: usize| -> Option<Hsla> {
            let cell = cell(col);
            if cursor == Some((line_id, col)) {
                Some(cell.fg.as_fg())
            } else if selection
                .is_some_and(|(start, end)| (line_id, col) >= start && (line_id, col) <= end)
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
            let cell = cell(col);
            let is_cursor = cursor == Some((line_id, col));
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

fn translate_key(event: &KeyDownEvent, application_cursor_keys: bool) -> Option<Vec<u8>> {
    let keystroke = &event.keystroke;

    if keystroke.modifiers.control && keystroke.key.len() == 1 {
        let c = keystroke.key.chars().next()?;
        if c.is_ascii_alphabetic() {
            let byte = (c.to_ascii_uppercase() as u8) - b'A' + 1;
            return Some(vec![byte]);
        }
    }

    let modifiers = &keystroke.modifiers;
    let modifier_code = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    if modifier_code > 1 {
        let final_byte = match keystroke.key.as_str() {
            "up" => Some('A'),
            "down" => Some('B'),
            "right" => Some('C'),
            "left" => Some('D'),
            "end" => Some('F'),
            "home" => Some('H'),
            _ => None,
        };
        if let Some(final_byte) = final_byte {
            return Some(format!("\x1b[1;{modifier_code}{final_byte}").into_bytes());
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

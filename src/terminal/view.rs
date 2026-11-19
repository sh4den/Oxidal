use gpui_kit::assets::IconName as Lucide;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, hover_card::HoverCard};
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, CursorStyle, Div,
    DragMoveEvent, Entity, FocusHandle, Font, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, ParentElement as _, Pixels, Point, Render, ScrollEvent, ScrollHandle, ShapedLine,
    SharedString, StatefulInteractiveElement as _, StrikethroughStyle, StyleRefinement,
    Styled as _, Task, TextAlign, TextRun, UnderlineStyle, Window, actions, canvas, div, fill,
    hsla, point, prelude::FluentBuilder as _, px, relative, rgb, size,
};

use super::backend::{Backend, BackendEvent};
use super::grid::{Cell, Color, Grid, TerminalPalette};
use super::stats::{self, DiskInfo, RemoteStats, TempInfo};
use crate::settings::AppSettings;

const CPU_HISTORY_LEN: usize = 30;
const FEED_BUDGET: Duration = Duration::from_millis(4);
const FEED_PAUSE: Duration = Duration::from_millis(4);
const AUTOSCROLL_TICK: Duration = Duration::from_millis(16);
const RESIZE_SETTLE: Duration = Duration::from_millis(50);
const AUTOSCROLL_MIN: f32 = 8.;
const AUTOSCROLL_MAX: f32 = 400.;
const AUTOSCROLL_RANGE: f32 = 8.;
const AUTOSCROLL_POWER: f32 = 2.;

const CLIPBOARD_SHARES_CONTROL_KEY: bool = !cfg!(target_os = "macos");

const WORD_MOTION_USES_ALT: bool = cfg!(target_os = "macos");

actions!(
    terminal,
    [
        SendTab,
        SendTabPrev,
        CopySelection,
        CutSelection,
        PasteClipboard
    ]
);

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

fn sanitize_paste(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\r');
            }
            '\n' => out.push('\r'),
            '\t' => out.push('\t'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

pub struct TerminalView {
    grid: Grid,
    backend: Backend,
    focus_handle: FocusHandle,
    closed_message: Option<String>,
    monitor: Option<Entity<MonitorBar>>,
    selection: Option<Selection>,
    layout: Option<(Bounds<Pixels>, Pixels, Pixels)>,
    scroll_offset: f32,
    drag_task: Option<Task<()>>,
    resize_task: Option<Task<()>>,
    last_mouse_report: Option<(u8, usize, usize)>,
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
                let Ok(event) = events.recv().await else {
                    break;
                };
                let closed = match event {
                    BackendEvent::Data(bytes) => {
                        match this.update(cx, |view: &mut Self, cx| view.feed(bytes, &events, cx)) {
                            Ok(closed) => closed,
                            Err(_) => break,
                        }
                    }
                    BackendEvent::Closed(message) => Some(message),
                };
                if let Some(message) = closed {
                    let _ = this.update(cx, |view: &mut Self, cx| {
                        view.closed_message =
                            Some(message.unwrap_or_else(|| "Connection closed".to_string()));
                        cx.notify();
                    });
                    break;
                }
                if !events.is_empty() {
                    cx.background_executor().timer(FEED_PAUSE).await;
                }
            }
        })
        .detach();

        cx.observe_global::<AppSettings>(|_, cx| cx.notify())
            .detach();

        let monitor = stats_rx.map(|rx| {
            cx.new(|cx| {
                cx.spawn(async move |this, cx| {
                    while let Ok(stats) = rx.recv().await {
                        if this
                            .update(cx, |bar: &mut MonitorBar, cx| {
                                if let Some(cpu) = stats.cpu {
                                    bar.cpu_history.push_back(cpu);
                                    while bar.cpu_history.len() > CPU_HISTORY_LEN {
                                        bar.cpu_history.pop_front();
                                    }
                                }
                                bar.stats = Some(stats);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
                cx.observe_global::<AppSettings>(|_, cx| cx.notify())
                    .detach();
                MonitorBar {
                    stats: None,
                    cpu_history: VecDeque::new(),
                    scroll: ScrollHandle::new(),
                    visible: Vec::new(),
                    dragging: None,
                    preview: None,
                }
            })
        });

        Self {
            grid: Grid::new(rows, cols),
            backend,
            focus_handle,
            closed_message: None,
            monitor,
            selection: None,
            layout: None,
            scroll_offset: 0.,
            drag_task: None,
            resize_task: None,
            last_mouse_report: None,
        }
    }

    fn feed(
        &mut self,
        mut bytes: Vec<u8>,
        events: &async_channel::Receiver<BackendEvent>,
        cx: &mut Context<Self>,
    ) -> Option<Option<String>> {
        let deadline = Instant::now() + FEED_BUDGET;
        let top_before = self.grid.screen_top_line();
        let mut closed = None;
        loop {
            let replies = self.grid.feed(&bytes);
            if !replies.is_empty() {
                self.backend.write_input(&replies);
            }
            if Instant::now() >= deadline {
                break;
            }
            match events.try_recv() {
                Ok(BackendEvent::Data(more)) => bytes = more,
                Ok(BackendEvent::Closed(message)) => {
                    closed = Some(message);
                    break;
                }
                Err(_) => break,
            }
        }
        if self.scroll_offset > 0. {
            let pushed = self.grid.screen_top_line() - top_before;
            self.scroll_offset =
                (self.scroll_offset + pushed as f32).min(self.grid.scrollback_len() as f32);
        }
        cx.notify();
        closed
    }

    fn handle_key(&mut self, event: &KeyDownEvent) {
        if event.keystroke.modifiers.shift && !self.grid.alt_active() {
            let page = self.grid.rows.max(1);
            match event.keystroke.key.as_str() {
                "pageup" => {
                    self.scroll_by(page as f32);
                    return;
                }
                "pagedown" => {
                    self.scroll_by(-(page as f32));
                    return;
                }
                _ => {}
            }
        }
        if let Some(bytes) = translate_key(event, self.grid.application_cursor_keys) {
            self.scroll_offset = 0.;
            self.selection = None;
            self.backend.write_input(&bytes);
        }
    }

    fn scroll_by(&mut self, lines: f32) {
        let max = self.grid.scrollback_len() as f32;
        self.scroll_offset = (self.scroll_offset + lines).clamp(0., max);
    }

    fn top_line(&self) -> f64 {
        self.grid.screen_top_line() as f64 - self.scroll_offset as f64
    }

    fn autoscroll_speed(&self, position: Point<Pixels>) -> f32 {
        let Some((bounds, _, line_height)) = self.layout else {
            return 0.;
        };
        if line_height <= px(0.) {
            return 0.;
        }
        let over = if position.y < bounds.origin.y {
            (bounds.origin.y - position.y) / line_height
        } else if position.y > bounds.bottom() {
            -((position.y - bounds.bottom()) / line_height)
        } else {
            return 0.;
        };
        let ratio = (over.abs() / AUTOSCROLL_RANGE).clamp(0., 1.);
        let speed =
            AUTOSCROLL_MIN + (AUTOSCROLL_MAX - AUTOSCROLL_MIN) * ratio.powf(AUTOSCROLL_POWER);
        if over > 0. { speed } else { -speed }
    }

    fn drag_scroll_tick(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.selection.is_some_and(|selection| selection.dragging) {
            return false;
        }
        let position = window.mouse_position();
        let speed = self.autoscroll_speed(position);
        let before = self.scroll_offset;
        if speed != 0. {
            self.scroll_by(speed * AUTOSCROLL_TICK.as_secs_f32());
        }
        let mut changed = self.scroll_offset != before;
        if let Some(cell) = self.cell_at(position, true)
            && let Some(selection) = self.selection.as_mut()
            && selection.head != cell
        {
            selection.head = cell;
            changed = true;
        }
        if changed {
            cx.notify();
        }
        true
    }

    fn start_drag_scroll(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.drag_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(AUTOSCROLL_TICK).await;
                let running = this
                    .update_in(cx, |view, window, cx| view.drag_scroll_tick(window, cx))
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        }));
    }

    fn resize(&mut self, rows: usize, cols: usize, cx: &mut Context<Self>) {
        if rows == self.grid.rows && cols == self.grid.cols {
            return;
        }
        self.grid.resize(rows, cols);
        self.resize_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let _ = this.update(cx, |view, _| view.backend.resize(rows as u16, cols as u16));
        }));
        self.selection = None;
        cx.notify();
    }

    fn cell_at(&self, position: Point<Pixels>, clamp: bool) -> Option<(usize, usize)> {
        let (bounds, char_width, line_height) = self.layout?;
        if char_width <= px(0.) || line_height <= px(0.) {
            return None;
        }
        if !clamp && !bounds.contains(&position) {
            return None;
        }
        let col = ((position.x - bounds.origin.x) / char_width).floor() as isize;
        let top = self.top_line();
        let offset = ((position.y - bounds.origin.y) / line_height) as f64;
        let first = top.floor().max(0.);
        let last = (top + self.grid.rows as f64).ceil() - 1.;
        let line = (top + offset).floor().clamp(first, last.max(first));
        Some((
            line.max(0.) as usize,
            col.clamp(0, self.grid.cols as isize - 1) as usize,
        ))
    }

    fn send_mouse(&mut self, button: u8, line: usize, col: usize, press: bool, drag: bool) {
        if self.grid.mouse_mode == 0 {
            return;
        }
        if !drag {
            self.last_mouse_report = None;
        }
        let bytes = self.mouse_report(button, line, col, press, drag);
        self.backend.write_input(&bytes);
    }

    fn mouse_report(
        &self,
        button: u8,
        line: usize,
        col: usize,
        press: bool,
        drag: bool,
    ) -> Vec<u8> {
        let row = line.saturating_sub(self.grid.screen_top_line());
        let code = button + if drag { 32 } else { 0 };
        if self.grid.mouse_sgr {
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
        }
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
        self.scroll_offset = 0.;
        self.selection = None;
        let text = sanitize_paste(text);
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
        self.drag_task = None;
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
        let (font_family, font_size, opacity) = {
            let settings = cx.global::<AppSettings>();
            (
                SharedString::from(settings.font_family.clone()),
                settings.font_size.clamp(8.0, 32.0),
                settings.opacity.clamp(0.3, 1.0),
            )
        };
        let line_height = font_size * 1.43;
        let closed_message = self.closed_message.clone();

        if self.grid.alt_active() {
            self.scroll_offset = 0.;
        }
        self.scroll_offset = self
            .scroll_offset
            .clamp(0., self.grid.scrollback_len() as f32);
        let scrolled_lines = self.scroll_offset.round() as usize;

        let surface_opacity = surface_opacity(opacity, cx);
        let (surface, surface_foreground) = {
            let palette = cx.global::<TerminalPalette>();
            (palette.background, palette.foreground)
        };
        self.grid.set_reported_colors(surface_foreground, surface);

        let measure = {
            let weak = cx.entity().downgrade();
            let font_family = font_family.clone();
            move |bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui_kit::App| {
                let run = TextRun {
                    len: 1,
                    font: gpui_kit::font(font_family.clone()),
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
                  cx: &mut gpui_kit::App| {
                if char_width <= px(0.) {
                    return;
                }
                let base_font = gpui_kit::font(font_family.clone());
                let (quads, lines) = {
                    let palette = cx.global::<TerminalPalette>().clone();
                    let view = entity.read(cx);
                    let selection = view.selection.map(|s| s.range());
                    build_paint(
                        &view.grid,
                        view.top_line(),
                        selection,
                        bounds,
                        char_width,
                        px(line_height),
                        px(font_size),
                        &base_font,
                        &palette,
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
                view.scroll_offset = 0.;
                view.backend.write_input(b"\t");
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &SendTabPrev, _window, cx| {
                view.scroll_offset = 0.;
                view.backend.write_input(b"\x1b[Z");
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CopySelection, _window, cx| {
                if !view.copy_selection(cx) && CLIPBOARD_SHARES_CONTROL_KEY {
                    view.scroll_offset = 0.;
                    view.backend.write_input(b"\x03");
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CutSelection, _window, cx| {
                if !view.copy_selection(cx) && CLIPBOARD_SHARES_CONTROL_KEY {
                    view.scroll_offset = 0.;
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
                        view.selection =
                            view.cell_at(event.position, false).map(|cell| Selection {
                                anchor: cell,
                                head: cell,
                                dragging: true,
                            });
                        if view.selection.is_some() {
                            view.start_drag_scroll(window, cx);
                        }
                    }
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseDownEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0
                        && !event.modifiers.shift
                        && let Some((row, col)) = view.cell_at(event.position, false)
                    {
                        view.send_mouse(1, row, col, true, false);
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
                    if !view.copy_selection(cx)
                        && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    {
                        view.paste(&text);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &MouseUpEvent, _window, cx| {
                    if view.grid.mouse_mode != 0
                        && !event.modifiers.shift
                        && let Some((row, col)) = view.cell_at(event.position, true)
                    {
                        view.send_mouse(0, row, col, false, false);
                    }
                    view.end_drag();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0
                        && !event.modifiers.shift
                        && let Some((row, col)) = view.cell_at(event.position, true)
                    {
                        view.send_mouse(1, row, col, false, false);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|view, event: &MouseUpEvent, _window, _cx| {
                    if view.grid.mouse_mode != 0
                        && !event.modifiers.shift
                        && let Some((row, col)) = view.cell_at(event.position, true)
                    {
                        view.send_mouse(2, row, col, false, false);
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
                    if let (Some(cell), Some(selection)) = (cell, view.selection.as_mut())
                        && selection.dragging
                        && selection.head != cell
                    {
                        selection.head = cell;
                        cx.notify();
                    }
                }
                if view.grid.mouse_mode >= 1002 && !event.modifiers.shift {
                    let code = match event.pressed_button {
                        Some(MouseButton::Left) => 0,
                        Some(MouseButton::Middle) => 1,
                        Some(MouseButton::Right) => 2,
                        _ => return,
                    };
                    if let Some((row, col)) = view.cell_at(event.position, true)
                        && view.last_mouse_report != Some((code, row, col))
                    {
                        view.last_mouse_report = Some((code, row, col));
                        view.send_mouse(code, row, col, true, true);
                    }
                }
            }))
            .on_scroll(cx.listener(|view, event: &ScrollEvent, _window, cx| {
                let Some((_, _, line_height)) = view.layout else {
                    return;
                };
                let dy = event.delta.pixel_delta(line_height).y;
                if dy == px(0.) {
                    return;
                }
                let scrolled = dy / line_height;
                let lines = ((dy.abs() / line_height).ceil() as usize).clamp(1, 5);
                let up = dy > px(0.);
                if view.grid.mouse_mode != 0 && !event.modifiers.shift {
                    if let Some((row, col)) = view.cell_at(event.position, true) {
                        let button = if up { 64 } else { 65 };
                        let report = view.mouse_report(button, row, col, true, false);
                        view.backend.write_input(&report.repeat(lines));
                    }
                } else if view.grid.alt_active() {
                    let seq: &[u8] = match (up, view.grid.application_cursor_keys) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    view.backend.write_input(&seq.repeat(lines));
                } else {
                    view.scroll_by(scrolled);
                }
                cx.notify();
            }))
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(surface.opacity(surface_opacity))
            .text_color(surface_foreground)
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
                    .when(scrolled_lines > 0, |this| {
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
                                .child(if scrolled_lines == 1 {
                                    "1 line up".to_string()
                                } else {
                                    format!("{scrolled_lines} lines up")
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
            .when_some(self.monitor.clone(), |this, monitor| {
                this.child(
                    monitor.cached(
                        StyleRefinement::default()
                            .flex_none()
                            .w_full()
                            .h(px(MONITOR_BAR_HEIGHT)),
                    ),
                )
            })
    }
}

const MONITOR_BAR_HEIGHT: f32 = 24.;

fn surface_opacity(opacity: f32, cx: &App) -> f32 {
    if opacity < 1.0 && cx.theme().mode.is_dark() {
        0.
    } else {
        opacity
    }
}

struct MonitorBar {
    stats: Option<RemoteStats>,
    cpu_history: VecDeque<f32>,
    scroll: ScrollHandle,
    visible: Vec<MonitorStat>,
    dragging: Option<MonitorStat>,
    preview: Option<Vec<MonitorStat>>,
}

impl Render for MonitorBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let opacity = cx.global::<AppSettings>().opacity.clamp(0.3, 1.0);
        let surface_opacity = surface_opacity(opacity, cx);
        if !cx.has_active_drag() {
            self.dragging = None;
            self.preview = None;
        }
        let order = monitor_order(cx);
        let mut items = self.items(cx);
        items.sort_by_key(|(stat, _)| order.iter().position(|o| o == stat));
        self.visible = items.iter().map(|(stat, _)| *stat).collect();
        if let Some(preview) = &self.preview {
            items.sort_by_key(|(stat, _)| preview.iter().position(|o| o == stat));
        }
        let dragging = self.dragging;
        let items: Vec<AnyElement> = if items.is_empty() {
            vec![
                div()
                    .px_2()
                    .text_color(hsla(0., 0., 0.45, 1.))
                    .child(Icon::new(IconName::Loader).xsmall())
                    .into_any_element(),
            ]
        } else {
            items
                .into_iter()
                .map(|(stat, element)| draggable_stat(stat, element, dragging == Some(stat), cx))
                .collect()
        };
        let max = self.scroll.max_offset().x;
        let overflowing = max > px(0.5);
        let mut current = self.scroll.offset();
        if current.x < -max {
            current.x = -max;
            self.scroll.set_offset(current);
        }
        let offset = current.x;
        let at_start = offset >= px(0.);
        let at_end = offset <= -max;

        div()
            .flex_none()
            .flex()
            .w_full()
            .items_center()
            .h(px(MONITOR_BAR_HEIGHT))
            .text_xs()
            .text_color(hsla(0., 0., 0.75, 1.))
            .bg(hsla(0., 0., 0.11, 1.).opacity(surface_opacity))
            .border_t_1()
            .border_color(hsla(0., 0., 0.2, 1.))
            .when(overflowing, |this| {
                this.child(scroll_arrow(
                    "monitor-scroll-left",
                    IconName::ChevronLeft,
                    !at_start,
                    cx.listener(|bar, _, _, cx| bar.scroll_by(MONITOR_SCROLL_STEP, cx)),
                ))
            })
            .child(
                div()
                    .id("monitor-scroll")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_hidden()
                    .track_scroll(&self.scroll)
                    .on_drag_move::<MonitorDrag>(cx.listener(
                        |bar, event: &DragMoveEvent<MonitorDrag>, _, cx| {
                            if !event.bounds.contains(&event.event.position)
                                && bar.preview.is_some()
                            {
                                bar.preview = None;
                                cx.notify();
                            }
                        },
                    ))
                    .on_drop(cx.listener(|bar, _: &MonitorDrag, _, cx| bar.drop(cx)))
                    .on_scroll(cx.listener(|bar, event: &ScrollEvent, window, cx| {
                        let delta = event.delta.pixel_delta(window.line_height());
                        let step = if delta.x == px(0.) { delta.y } else { delta.x };
                        bar.scroll_by(step, cx);
                    }))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .h_full()
                            .px_1()
                            .children(items),
                    ),
            )
            .when(overflowing, |this| {
                this.child(scroll_arrow(
                    "monitor-scroll-right",
                    IconName::ChevronRight,
                    !at_end,
                    cx.listener(|bar, _, _, cx| bar.scroll_by(-MONITOR_SCROLL_STEP, cx)),
                ))
            })
    }
}

const MONITOR_SCROLL_STEP: Pixels = px(160.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MonitorStat {
    Os,
    Cpu,
    Memory,
    Upload,
    Download,
    Connections,
    User,
    Uptime,
    Temperature,
    Disk,
}

impl MonitorStat {
    const ALL: [MonitorStat; 10] = [
        MonitorStat::Os,
        MonitorStat::Cpu,
        MonitorStat::Memory,
        MonitorStat::Upload,
        MonitorStat::Download,
        MonitorStat::Connections,
        MonitorStat::User,
        MonitorStat::Uptime,
        MonitorStat::Temperature,
        MonitorStat::Disk,
    ];

    fn key(self) -> &'static str {
        match self {
            MonitorStat::Os => "os",
            MonitorStat::Cpu => "cpu",
            MonitorStat::Memory => "memory",
            MonitorStat::Upload => "upload",
            MonitorStat::Download => "download",
            MonitorStat::Connections => "connections",
            MonitorStat::User => "user",
            MonitorStat::Uptime => "uptime",
            MonitorStat::Temperature => "temperature",
            MonitorStat::Disk => "disk",
        }
    }

    fn label(self) -> &'static str {
        match self {
            MonitorStat::Os => "System",
            MonitorStat::Cpu => "CPU",
            MonitorStat::Memory => "Memory",
            MonitorStat::Upload => "Upload",
            MonitorStat::Download => "Download",
            MonitorStat::Connections => "Connections",
            MonitorStat::User => "User",
            MonitorStat::Uptime => "Uptime",
            MonitorStat::Temperature => "Temperature",
            MonitorStat::Disk => "Disk",
        }
    }

    fn icon(self) -> Icon {
        match self {
            MonitorStat::Os => Icon::new(IconName::Globe),
            MonitorStat::Cpu => Icon::new(IconName::Cpu),
            MonitorStat::Memory => Icon::new(IconName::MemoryStick),
            MonitorStat::Upload => Icon::new(IconName::ArrowUp),
            MonitorStat::Download => Icon::new(IconName::ArrowDown),
            MonitorStat::Connections => Icon::new(IconName::Network),
            MonitorStat::User => Icon::new(IconName::User),
            MonitorStat::Uptime => Icon::new(Lucide::Clock),
            MonitorStat::Temperature => thermometer(),
            MonitorStat::Disk => Icon::new(IconName::HardDrive),
        }
    }
}

fn monitor_order(cx: &App) -> Vec<MonitorStat> {
    let saved = &cx.global::<AppSettings>().monitor_order;
    let mut order: Vec<MonitorStat> = saved
        .iter()
        .filter_map(|key| MonitorStat::ALL.into_iter().find(|stat| stat.key() == key))
        .collect();
    for stat in MonitorStat::ALL {
        if !order.contains(&stat) {
            order.push(stat);
        }
    }
    order
}

fn preview_step(
    mut order: Vec<MonitorStat>,
    dragged: MonitorStat,
    hovered: MonitorStat,
    past_middle: bool,
) -> Vec<MonitorStat> {
    let (Some(from), Some(to)) = (
        order.iter().position(|s| *s == dragged),
        order.iter().position(|s| *s == hovered),
    ) else {
        return order;
    };
    if (from < to && past_middle) || (from > to && !past_middle) {
        let stat = order.remove(from);
        order.insert(to, stat);
    }
    order
}

fn merged_order(saved: Vec<MonitorStat>, visible: &[MonitorStat]) -> Vec<MonitorStat> {
    let mut shown = visible.iter().copied();
    saved
        .into_iter()
        .map(|stat| {
            if visible.contains(&stat) {
                shown.next().unwrap_or(stat)
            } else {
                stat
            }
        })
        .collect()
}

fn save_monitor_order(order: Vec<MonitorStat>, cx: &mut App) {
    let keys: Vec<String> = order.iter().map(|s| s.key().to_string()).collect();
    if cx.global::<AppSettings>().monitor_order != keys {
        cx.global_mut::<AppSettings>().monitor_order = keys;
        crate::settings::save_settings(cx.global::<AppSettings>());
    }
}

#[derive(Clone, Copy)]
struct MonitorDrag {
    stat: MonitorStat,
}

impl Render for MonitorDrag {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accent = cx.theme().primary;
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .h(px(22.))
            .px_2p5()
            .rounded(px(6.))
            .bg(hsla(0., 0., 0.16, 0.96))
            .border_1()
            .border_color(accent)
            .shadow_md()
            .text_xs()
            .text_color(hsla(0., 0., 0.92, 1.))
            .child(div().text_color(accent).child(self.stat.icon().xsmall()))
            .child(self.stat.label())
    }
}

fn draggable_stat(
    stat: MonitorStat,
    element: AnyElement,
    dragged: bool,
    cx: &mut Context<MonitorBar>,
) -> AnyElement {
    let primary = cx.theme().primary;
    div()
        .id(("monitor-stat", stat as usize))
        .flex_none()
        .flex()
        .items_center()
        .h(px(18.))
        .px_1p5()
        .rounded(px(4.))
        .cursor(CursorStyle::OpenHand)
        .when(dragged, |this| this.opacity(0.45).bg(primary.opacity(0.18)))
        .when(!dragged, |this| {
            this.hover(|style| style.bg(hsla(0., 0., 1., 0.05)))
        })
        .on_drag(MonitorDrag { stat }, |drag, _, _, cx| cx.new(|_| *drag))
        .on_drag_move::<MonitorDrag>(cx.listener(
            move |bar, event: &DragMoveEvent<MonitorDrag>, _, cx| {
                let position = event.event.position;
                if !event.bounds.contains(&position) {
                    return;
                }
                let past_middle = position.x > event.bounds.center().x;
                let dragged = event.drag(cx).stat;
                bar.hover_stat(dragged, stat, past_middle, cx);
            },
        ))
        .on_drop(cx.listener(|bar, _: &MonitorDrag, _, cx| bar.drop(cx)))
        .child(element)
        .into_any_element()
}

fn scroll_arrow(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.))
        .h_full()
        .border_color(hsla(0., 0., 0.2, 1.))
        .when(id.ends_with("left"), |this| this.border_r_1())
        .when(id.ends_with("right"), |this| this.border_l_1())
        .text_color(if enabled {
            hsla(0., 0., 0.85, 1.)
        } else {
            hsla(0., 0., 0.35, 1.)
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(hsla(0., 0., 0.2, 1.)))
                .on_click(on_click)
        })
        .child(Icon::new(icon).xsmall())
}

impl MonitorBar {
    fn hover_stat(
        &mut self,
        dragged: MonitorStat,
        hovered: MonitorStat,
        past_middle: bool,
        cx: &mut Context<Self>,
    ) {
        let current = self.preview.clone().unwrap_or_else(|| self.visible.clone());
        let next = preview_step(current, dragged, hovered, past_middle);
        if self.dragging != Some(dragged) || self.preview.as_ref() != Some(&next) {
            self.dragging = Some(dragged);
            self.preview = Some(next);
            cx.notify();
        }
    }

    fn drop(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.preview.take() {
            save_monitor_order(merged_order(monitor_order(cx), &preview), cx);
        }
        self.dragging = None;
        cx.notify();
    }

    fn scroll_by(&mut self, step: Pixels, cx: &mut Context<Self>) {
        let max = self.scroll.max_offset().x;
        let mut offset = self.scroll.offset();
        let x = (offset.x + step).clamp(-max, px(0.));
        if x != offset.x {
            offset.x = x;
            self.scroll.set_offset(offset);
            cx.notify();
        }
    }

    fn items(&self, cx: &App) -> Vec<(MonitorStat, AnyElement)> {
        let mut items: Vec<(MonitorStat, AnyElement)> = Vec::new();
        if let Some(stats) = &self.stats {
            if let Some(sysname) = &stats.sysname {
                let os = match stats.os {
                    Some(logo) => segment_icon(
                        Icon::default().path(format!("icons/oxidal/os/{}.svg", logo.icon)),
                        logo_color(logo.color),
                    ),
                    None => segment(IconName::Globe),
                };
                items.push((
                    MonitorStat::Os,
                    os.child(sysname.clone()).into_any_element(),
                ));
            }
            items.push((
                MonitorStat::Cpu,
                segment(IconName::Cpu)
                    .child(cpu_chart(&self.cpu_history))
                    .when_some(stats.cpu, |this, cpu| {
                        this.child(format!("{:.0}%", cpu * 100.))
                    })
                    .into_any_element(),
            ));
            if let Some((used, total)) = stats.mem {
                let frac = used as f32 / total.max(1) as f32;
                items.push((
                    MonitorStat::Memory,
                    segment(IconName::MemoryStick)
                        .child(meter_bar(frac))
                        .child(format!("{}/{}", fmt_size(used), fmt_size(total)))
                        .into_any_element(),
                ));
            }
            if let Some((rx, tx)) = stats.net {
                items.push((
                    MonitorStat::Upload,
                    segment_colored(IconName::ArrowUp, hsla(0.75, 0.6, 0.65, 1.))
                        .child(rate_cell(tx))
                        .into_any_element(),
                ));
                items.push((
                    MonitorStat::Download,
                    segment_colored(IconName::ArrowDown, hsla(0.36, 0.55, 0.5, 1.))
                        .child(rate_cell(rx))
                        .into_any_element(),
                ));
            }
            if let Some((count, port)) = stats.connections {
                items.push((
                    MonitorStat::Connections,
                    segment(IconName::Network)
                        .child(format!("Connections: {count} (port {port})"))
                        .into_any_element(),
                ));
            }
            if let Some(user) = &stats.user {
                items.push((
                    MonitorStat::User,
                    HoverCard::new("monitor-who")
                        .anchor(Anchor::BottomLeft)
                        .open_delay(Duration::from_millis(250))
                        .trigger(segment(IconName::User).child(user.clone()))
                        .child(who_details(&stats.who, cx))
                        .into_any_element(),
                ));
            }
            if let Some(uptime) = stats.uptime {
                items.push((
                    MonitorStat::Uptime,
                    segment_icon(Icon::new(Lucide::Clock), hsla(0., 0., 0.5, 1.))
                        .child(fmt_uptime(uptime))
                        .into_any_element(),
                ));
            }
            if let Some(temps) = &stats.temps {
                let label = stats::headline_temp(temps)
                    .map(|t| format!("{:.0}°C", t.value))
                    .unwrap_or_else(|| "--".to_string());
                items.push((
                    MonitorStat::Temperature,
                    HoverCard::new("monitor-temps")
                        .anchor(Anchor::BottomLeft)
                        .open_delay(Duration::from_millis(250))
                        .trigger(segment_icon(thermometer(), hsla(0., 0., 0.5, 1.)).child(label))
                        .child(temp_details(temps, cx))
                        .into_any_element(),
                ));
            }
            if let Some((used, total)) = stats.disk {
                let frac = used as f32 / total.max(1) as f32;
                items.push((
                    MonitorStat::Disk,
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
                ));
            }
        }
        items
    }
}

fn segment(icon: IconName) -> Div {
    segment_colored(icon, hsla(0., 0., 0.5, 1.))
}

fn segment_colored(icon: IconName, color: Hsla) -> Div {
    segment_icon(Icon::new(icon), color)
}

fn segment_icon(icon: Icon, color: Hsla) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .child(div().text_color(color).child(icon.xsmall()))
}

fn logo_color(hex: u32) -> Hsla {
    let mut color: Hsla = rgb(hex).into();
    color.l = color.l.clamp(0.5, 0.75);
    color
}

fn fmt_uptime(secs: u64) -> String {
    let (days, hours, mins) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else {
        format!("{mins}m")
    }
}

fn thermometer() -> Icon {
    Icon::new(Lucide::Thermometer)
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

fn temp_details(temps: &[TempInfo], cx: &App) -> AnyElement {
    const MAX_ROWS: usize = 12;
    let muted = cx.theme().muted_foreground;

    let mut rows: Vec<AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .mb_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_color(muted)
            .child(thermometer().xsmall())
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Temperatures"),
            )
            .into_any_element(),
    ];

    let mut chip = "";
    for temp in temps.iter().take(MAX_ROWS) {
        if temp.chip != chip {
            chip = &temp.chip;
            rows.push(
                div()
                    .when(rows.len() > 1, |this| this.mt_1())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(muted)
                    .child(temp.chip.clone())
                    .into_any_element(),
            );
        }
        rows.push(
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
                        .child(temp.label.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(meter_color(temp.fraction()))
                        .child(format!("{:.1}°C", temp.value)),
                )
                .into_any_element(),
        );
    }

    if temps.len() > MAX_ROWS {
        rows.push(
            div()
                .mt_1()
                .text_color(muted)
                .child(format!("+{} more sensors", temps.len() - MAX_ROWS))
                .into_any_element(),
        );
    }
    if temps.is_empty() {
        rows.push(
            div()
                .text_color(muted)
                .child(
                    "No sensors reported. This usually means the kernel drivers are not loaded yet. Running sensors-detect on the host may fix it.",
                )
                .into_any_element(),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .w(px(260.))
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

struct RunFonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
}

impl RunFonts {
    fn new(base: &Font) -> Self {
        Self {
            regular: base.clone(),
            bold: base.clone().bold(),
            italic: base.clone().italic(),
            bold_italic: base.clone().bold().italic(),
        }
    }

    fn run(&self, style: RunStyle, len: usize) -> TextRun {
        let font = match (style.bold, style.italic) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        };
        TextRun {
            len,
            font: font.clone(),
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
        }
    }
}

#[derive(Default)]
struct ColorCache {
    last: Option<(Color, Hsla)>,
}

impl ColorCache {
    fn resolve(&mut self, color: Color, palette: &TerminalPalette) -> Hsla {
        if let Some((cached, hsla)) = self.last
            && cached == color
        {
            return hsla;
        }
        let hsla = color.as_fg(palette);
        self.last = Some((color, hsla));
        hsla
    }
}

fn build_paint(
    grid: &Grid,
    top: f64,
    selection: Option<((usize, usize), (usize, usize))>,
    bounds: Bounds<Pixels>,
    char_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
    base_font: &Font,
    palette: &TerminalPalette,
    window: &Window,
) -> (Vec<PaintQuad>, Vec<(ShapedLine, Point<Pixels>)>) {
    let mut quads = Vec::with_capacity(grid.rows + 1);
    let mut lines = Vec::with_capacity(grid.rows + 1);

    let first = top.floor().max(0.);
    let shift = line_height * (top - first) as f32;
    let top_line = first as usize;
    let cursor = grid
        .cursor_visible
        .then_some((grid.screen_top_line() + grid.cursor_row, grid.cursor_col));
    let fonts = RunFonts::new(base_font);
    let mut fg_cache = ColorCache::default();

    for row in 0..=grid.rows {
        let line_id = top_line + row;
        let Some(cells) = grid.line_cells(line_id) else {
            continue;
        };
        let y = bounds.origin.y + line_height * row as f32 - shift;
        let cell = |col: usize| -> Cell { cells.get(col).copied().unwrap_or(Cell::BLANK) };

        let cell_bg = |col: usize| -> Option<Hsla> {
            let cell = cell(col);
            if cursor == Some((line_id, col)) {
                Some(palette.cursor)
            } else if selection
                .is_some_and(|(start, end)| (line_id, col) >= start && (line_id, col) <= end)
            {
                Some(palette.selection)
            } else {
                cell.bg.as_bg(palette)
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

        let mut text = String::with_capacity(grid.cols);
        let mut runs: Vec<TextRun> = Vec::new();
        let mut style: Option<RunStyle> = None;
        let mut run_start = 0;
        let mut visible_end = 0;

        for col in 0..grid.cols {
            let cell = cell(col);
            let is_cursor = cursor == Some((line_id, col));
            let mut color = if is_cursor {
                palette.background
            } else {
                fg_cache.resolve(cell.fg, palette)
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
            if style != Some(cell_style) {
                if let Some(previous) = style {
                    runs.push(fonts.run(previous, text.len() - run_start));
                }
                style = Some(cell_style);
                run_start = text.len();
            }
            let ch = cell.ch();
            text.push(ch);
            if !ch.is_whitespace() || cell_style.underline || cell_style.strike {
                visible_end = text.len();
            }
        }
        if let Some(last) = style {
            runs.push(fonts.run(last, text.len() - run_start));
        }
        if visible_end == 0 {
            continue;
        }
        text.truncate(visible_end);
        let mut remaining = visible_end;
        runs.retain_mut(|run| {
            if remaining == 0 {
                return false;
            }
            run.len = run.len.min(remaining);
            remaining -= run.len;
            true
        });
        let shaped = window.text_system().shape_line(
            SharedString::from(text),
            font_size,
            &runs,
            Some(char_width),
        );
        lines.push((shaped, point(bounds.origin.x, y)));
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

    if WORD_MOTION_USES_ALT && modifiers.alt && !modifiers.control && !modifiers.platform {
        match keystroke.key.as_str() {
            "left" => return Some(b"\x1bb".to_vec()),
            "right" => return Some(b"\x1bf".to_vec()),
            _ => {}
        }
    }

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

#[cfg(test)]
#[path = "../../tests/terminal/view.rs"]
mod tests;

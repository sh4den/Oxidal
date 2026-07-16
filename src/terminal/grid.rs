use gpui::{hsla, Hsla};
use vte::{Params, Perform};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    DefaultFg,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn as_fg(self) -> Hsla {
        match self {
            Color::Default | Color::DefaultFg => default_fg(),
            Color::Indexed(i) => palette_256(i as u16),
            Color::Rgb(r, g, b) => rgb_to_hsla(r as f32 / 255., g as f32 / 255., b as f32 / 255.),
        }
    }

    pub fn as_bg(self) -> Option<Hsla> {
        match self {
            Color::Default => None,
            other => Some(other.as_fg()),
        }
    }
}

const CHAR_MASK: u32 = (1 << 24) - 1;
const FLAG_BOLD: u32 = 1 << 24;
const FLAG_ITALIC: u32 = 1 << 25;
const FLAG_UNDERLINE: u32 = 1 << 26;
const FLAG_STRIKE: u32 = 1 << 27;
const FLAG_DIM: u32 = 1 << 28;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    ch_flags: u32,
    pub fg: Color,
    pub bg: Color,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

impl Cell {
    const BLANK: Cell = Cell {
        ch_flags: ' ' as u32,
        fg: Color::Default,
        bg: Color::Default,
    };

    fn new(ch: char, fg: Color, bg: Color, flags: u32) -> Self {
        Self {
            ch_flags: ch as u32 | flags,
            fg,
            bg,
        }
    }

    fn blank(attrs: &Attrs) -> Self {
        Self::new(' ', attrs.fg, attrs.bg, 0)
    }

    pub fn ch(self) -> char {
        char::from_u32(self.ch_flags & CHAR_MASK).unwrap_or(' ')
    }

    pub fn bold(self) -> bool {
        self.ch_flags & FLAG_BOLD != 0
    }

    pub fn italic(self) -> bool {
        self.ch_flags & FLAG_ITALIC != 0
    }

    pub fn underline(self) -> bool {
        self.ch_flags & FLAG_UNDERLINE != 0
    }

    pub fn strike(self) -> bool {
        self.ch_flags & FLAG_STRIKE != 0
    }

    pub fn dim(self) -> bool {
        self.ch_flags & FLAG_DIM != 0
    }
}

#[derive(Clone, Copy, Default)]
struct Attrs {
    fg: Color,
    bg: Color,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    reverse: bool,
}

impl Attrs {
    fn flags(&self) -> u32 {
        let mut flags = 0;
        if self.bold {
            flags |= FLAG_BOLD;
        }
        if self.italic {
            flags |= FLAG_ITALIC;
        }
        if self.underline {
            flags |= FLAG_UNDERLINE;
        }
        if self.strike {
            flags |= FLAG_STRIKE;
        }
        if self.dim {
            flags |= FLAG_DIM;
        }
        flags
    }
}

fn default_fg() -> Hsla {
    hsla(0., 0., 0.85, 1.)
}

pub fn default_bg() -> Hsla {
    hsla(0., 0., 0.07, 1.)
}

/// ANSI 16-color palette rendered as approximate terminal colors.
fn ansi_color(code: u16, bright: bool) -> Hsla {
    let (h, s, l): (f32, f32, f32) = match code {
        0 => (0., 0., if bright { 0.45 } else { 0.15 }),
        1 => (0., 0.55, if bright { 0.62 } else { 0.48 }),
        2 => (120., 0.4, if bright { 0.6 } else { 0.42 }),
        3 => (48., 0.55, if bright { 0.65 } else { 0.5 }),
        4 => (215., 0.6, if bright { 0.68 } else { 0.55 }),
        5 => (285., 0.5, if bright { 0.7 } else { 0.55 }),
        6 => (185., 0.5, if bright { 0.65 } else { 0.5 }),
        7 => (0., 0., if bright { 0.95 } else { 0.75 }),
        _ => (0., 0., 0.85),
    };
    hsla(h / 360., s, l, 1.)
}

fn ansi_index(code: u16, bright: bool) -> u8 {
    code as u8 + if bright { 8 } else { 0 }
}

fn blank_grid(rows: usize, cols: usize, blank: Cell) -> Box<[Box<[Cell]>]> {
    (0..rows)
        .map(|_| vec![blank; cols].into_boxed_slice())
        .collect()
}

struct SavedPrimaryScreen {
    cells: Box<[Box<[Cell]>]>,
    cursor: (usize, usize),
}

pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    cells: Box<[Box<[Cell]>]>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub cursor_visible: bool,
    attrs: Attrs,
    saved_cursor: (usize, usize),
    /// Parked in an `Option` so `feed` can move it out and pass `self` as the
    /// `Perform` sink without building a throwaway parser on every call.
    parser: Option<vte::Parser>,
    /// Inclusive scroll region rows (DECSTBM). Defaults to the whole screen.
    scroll_top: usize,
    scroll_bottom: usize,
    autowrap: bool,
    /// DECCKM: arrow keys send `ESC O x` instead of `ESC [ x` when set.
    pub application_cursor_keys: bool,
    alt_screen: Option<SavedPrimaryScreen>,
    responses: Vec<u8>,
    last_printed: Option<char>,
    g0_graphics: bool,
    g1_graphics: bool,
    shift_out: bool,
}

impl Grid {
    pub fn new(rows: usize, cols: usize) -> Self {
        let attrs = Attrs::default();
        Self {
            rows,
            cols,
            cells: blank_grid(rows, cols, Cell::blank(&attrs)),
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            attrs,
            saved_cursor: (0, 0),
            parser: Some(vte::Parser::new()),
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            autowrap: true,
            application_cursor_keys: false,
            alt_screen: None,
            responses: Vec::new(),
            last_printed: None,
            g0_graphics: false,
            g1_graphics: false,
            shift_out: false,
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> Cell {
        self.cells[row][col]
    }

    /// Resize the grid to fill however much space is actually available,
    /// growing/shrinking rows at the bottom (keeping the most recent content
    /// visible) and columns on the right. Also resizes the stashed alt-screen
    /// buffer if one exists, so switching back stays consistent. The scroll
    /// region is reset to the full screen, since an old region may no longer
    /// make sense at the new size.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if rows == self.rows && cols == self.cols {
            return;
        }

        let old_cells = std::mem::take(&mut self.cells);
        self.cells = resized_cells(old_cells, rows, cols);
        if let Some(alt) = &mut self.alt_screen {
            let alt_cells = std::mem::take(&mut alt.cells);
            alt.cells = resized_cells(alt_cells, rows, cols);
            alt.cursor.0 = alt.cursor.0.min(rows - 1);
            alt.cursor.1 = alt.cursor.1.min(cols - 1);
        }

        self.rows = rows;
        self.cols = cols;
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.cursor_col = self.cursor_col.min(cols - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let Some(mut parser) = self.parser.take() else {
            return Vec::new();
        };
        for byte in bytes {
            parser.advance(self, *byte);
        }
        self.parser = Some(parser);
        std::mem::take(&mut self.responses)
    }

    fn blank_cell(&self) -> Cell {
        Cell::blank(&self.attrs)
    }

    fn line_feed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up_region(1);
        } else if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        }
    }

    /// Move `rows` within the scroll region up or down by `n`, blanking the
    /// rows that wrap around. Rotating the row handles recycles the existing
    /// row buffers, so scrolling never touches the allocator.
    fn scroll_region(&mut self, n: usize, up: bool) {
        if self.scroll_top > self.scroll_bottom || self.scroll_bottom >= self.rows {
            return;
        }
        let n = n.min(self.scroll_bottom - self.scroll_top + 1);
        let blank = self.blank_cell();
        let region = &mut self.cells[self.scroll_top..=self.scroll_bottom];
        if up {
            region.rotate_left(n);
            for row in region.iter_mut().rev().take(n) {
                row.fill(blank);
            }
        } else {
            region.rotate_right(n);
            for row in region.iter_mut().take(n) {
                row.fill(blank);
            }
        }
    }

    fn scroll_up_region(&mut self, n: usize) {
        self.scroll_region(n, true);
    }

    fn scroll_down_region(&mut self, n: usize) {
        self.scroll_region(n, false);
    }

    fn insert_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        let blank = self.blank_cell();
        let region = &mut self.cells[self.cursor_row..=self.scroll_bottom];
        region.rotate_right(n);
        for row in region.iter_mut().take(n) {
            row.fill(blank);
        }
    }

    fn delete_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        let blank = self.blank_cell();
        let region = &mut self.cells[self.cursor_row..=self.scroll_bottom];
        region.rotate_left(n);
        for row in region.iter_mut().rev().take(n) {
            row.fill(blank);
        }
    }

    fn insert_chars(&mut self, n: usize) {
        let blank = self.blank_cell();
        let (col, cols) = (self.cursor_col, self.cols);
        let n = n.min(cols - col);
        let row = &mut self.cells[self.cursor_row];
        row.copy_within(col..cols - n, col + n);
        row[col..col + n].fill(blank);
    }

    fn delete_chars(&mut self, n: usize) {
        let blank = self.blank_cell();
        let (col, cols) = (self.cursor_col, self.cols);
        let n = n.min(cols - col);
        let row = &mut self.cells[self.cursor_row];
        row.copy_within(col + n..cols, col);
        row[cols - n..].fill(blank);
    }

    fn erase_chars(&mut self, n: usize) {
        let blank = self.blank_cell();
        let col = self.cursor_col;
        let n = n.min(self.cols - col);
        self.cells[self.cursor_row][col..col + n].fill(blank);
    }

    fn set_scroll_region(&mut self, top: Option<u16>, bottom: Option<u16>) {
        let top = top.unwrap_or(1).max(1) as usize - 1;
        let bottom = bottom
            .map(|b| b as usize)
            .unwrap_or(self.rows)
            .min(self.rows)
            .saturating_sub(1);
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
        } else {
            self.scroll_top = 0;
            self.scroll_bottom = self.rows.saturating_sub(1);
        }
        self.cursor_row = self.scroll_top;
        self.cursor_col = 0;
    }

    /// Enter or leave the alternate screen buffer (used by full-screen TUIs
    /// like vim, htop, less). Also resets scroll region / cursor-key /
    /// autowrap modes so a TUI that exits uncleanly can't leave the shell
    /// in a broken state.
    fn set_alt_screen(&mut self, enable: bool) {
        if enable {
            if self.alt_screen.is_none() {
                let blank = blank_grid(self.rows, self.cols, self.blank_cell());
                let cells = std::mem::replace(&mut self.cells, blank);
                self.alt_screen = Some(SavedPrimaryScreen {
                    cells,
                    cursor: (self.cursor_row, self.cursor_col),
                });
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
        } else if let Some(saved) = self.alt_screen.take() {
            self.cells = saved.cells;
            self.cursor_row = saved.cursor.0;
            self.cursor_col = saved.cursor.1;
        }

        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        self.application_cursor_keys = false;
        self.autowrap = true;
        self.shift_out = false;
    }

    fn put_char(&mut self, ch: char) {
        let graphics = if self.shift_out {
            self.g1_graphics
        } else {
            self.g0_graphics
        };
        let ch = if graphics { dec_graphics(ch) } else { ch };
        self.last_printed = Some(ch);
        if self.cursor_col >= self.cols {
            if self.autowrap {
                self.cursor_col = 0;
                self.line_feed();
            } else {
                self.cursor_col = self.cols - 1;
            }
        }
        let (fg, bg) = if self.attrs.reverse {
            let bg = match self.attrs.fg {
                Color::Default => Color::DefaultFg,
                c => c,
            };
            (self.attrs.bg, bg)
        } else {
            (self.attrs.fg, self.attrs.bg)
        };
        self.cells[self.cursor_row][self.cursor_col] = Cell::new(ch, fg, bg, self.attrs.flags());
        self.cursor_col += 1;
    }

    fn reverse_line_feed(&mut self) {
        if self.cursor_row == self.scroll_top {
            self.scroll_down_region(1);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let blank = self.blank_cell();
        let (col, cols) = (self.cursor_col, self.cols);
        let row = &mut self.cells[self.cursor_row];
        match mode {
            0 => row[col.min(cols)..].fill(blank),
            1 => row[..=col.min(cols - 1)].fill(blank),
            2 => row.fill(blank),
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let blank = self.blank_cell();
        match mode {
            0 => {
                self.erase_line(0);
                for row in self.cells[self.cursor_row + 1..].iter_mut() {
                    row.fill(blank);
                }
            }
            1 => {
                self.erase_line(1);
                for row in self.cells[..self.cursor_row].iter_mut() {
                    row.fill(blank);
                }
            }
            2 | 3 => {
                for row in self.cells.iter_mut() {
                    row.fill(blank);
                }
            }
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        let mut buf = [0u16; 32];
        let mut len = 0;
        for p in params.iter() {
            if len == buf.len() {
                break;
            }
            buf[len] = p.first().copied().unwrap_or(0);
            len += 1;
        }
        let values = &buf[..len];

        if values.is_empty() {
            self.attrs = Attrs::default();
            return;
        }
        let mut i = 0;
        while i < values.len() {
            match values[i] {
                0 => self.attrs = Attrs::default(),
                1 => self.attrs.bold = true,
                2 => self.attrs.dim = true,
                3 => self.attrs.italic = true,
                4 => self.attrs.underline = true,
                9 => self.attrs.strike = true,
                22 => {
                    self.attrs.bold = false;
                    self.attrs.dim = false;
                }
                23 => self.attrs.italic = false,
                24 => self.attrs.underline = false,
                29 => self.attrs.strike = false,
                7 => self.attrs.reverse = true,
                27 => self.attrs.reverse = false,
                39 => self.attrs.fg = Color::Default,
                49 => self.attrs.bg = Color::Default,
                30..=37 => {
                    self.attrs.fg = Color::Indexed(ansi_index(values[i] - 30, self.attrs.bold))
                }
                90..=97 => self.attrs.fg = Color::Indexed(ansi_index(values[i] - 90, true)),
                40..=47 => self.attrs.bg = Color::Indexed(ansi_index(values[i] - 40, false)),
                100..=107 => self.attrs.bg = Color::Indexed(ansi_index(values[i] - 100, true)),
                38 | 48 => {
                    let is_fg = values[i] == 38;
                    let color = if i + 2 < values.len() && values[i + 1] == 5 {
                        i += 2;
                        Some(Color::Indexed(values[i].min(255) as u8))
                    } else if i + 4 < values.len() && values[i + 1] == 2 {
                        let rgb = Color::Rgb(
                            values[i + 2].min(255) as u8,
                            values[i + 3].min(255) as u8,
                            values[i + 4].min(255) as u8,
                        );
                        i += 4;
                        Some(rgb)
                    } else {
                        None
                    };
                    if let Some(color) = color {
                        if is_fg {
                            self.attrs.fg = color;
                        } else {
                            self.attrs.bg = color;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn clamp_cursor(&mut self) {
        self.cursor_row = self.cursor_row.min(self.rows - 1);
        self.cursor_col = self.cursor_col.min(self.cols.saturating_sub(1));
    }
}

fn resized_cells(
    cells: Box<[Box<[Cell]>]>,
    new_rows: usize,
    new_cols: usize,
) -> Box<[Box<[Cell]>]> {
    let skip = cells.len().saturating_sub(new_rows);
    let mut rows: Vec<Box<[Cell]>> = Vec::with_capacity(new_rows);
    for row in cells.into_vec().into_iter().skip(skip) {
        let mut new_row = vec![Cell::BLANK; new_cols];
        let keep = row.len().min(new_cols);
        new_row[..keep].copy_from_slice(&row[..keep]);
        rows.push(new_row.into_boxed_slice());
    }
    while rows.len() < new_rows {
        rows.push(vec![Cell::BLANK; new_cols].into_boxed_slice());
    }
    rows.into_boxed_slice()
}

fn palette_256(code: u16) -> Hsla {
    if code < 16 {
        return ansi_color(code % 8, code >= 8);
    }
    if code < 232 {
        let c = code - 16;
        let r = c / 36;
        let g = (c % 36) / 6;
        let b = c % 6;
        return rgb_to_hsla(r as f32 / 5., g as f32 / 5., b as f32 / 5.);
    }
    let level = (code - 232) as f32 / 23.;
    rgb_to_hsla(level, level, level)
}

fn rgb_to_hsla(r: f32, g: f32, b: f32) -> Hsla {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.;
    if (max - min).abs() < f32::EPSILON {
        return hsla(0., 0., l, 1.);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2. - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d) % 6.
    } else if max == g {
        (b - r) / d + 2.
    } else {
        (r - g) / d + 4.
    };
    let mut h = h * 60.;
    if h < 0. {
        h += 360.;
    }
    hsla(h / 360., s, l, 1.)
}

impl Perform for Grid {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' | 0x0b | 0x0c => self.line_feed(),
            b'\r' => self.cursor_col = 0,
            0x08 => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            b'\t' => {
                let next_tab = (self.cursor_col / 8 + 1) * 8;
                self.cursor_col = next_tab.min(self.cols - 1);
            }
            0x0e => self.shift_out = true,
            0x0f => self.shift_out = false,
            0x07 => {}
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let n = |default: u16| -> u16 {
            let first = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
            if first == 0 { default } else { first }
        };
        match action {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(n(1) as usize),
            'B' => self.cursor_row = (self.cursor_row + n(1) as usize).min(self.rows - 1),
            'C' => self.cursor_col = (self.cursor_col + n(1) as usize).min(self.cols - 1),
            'D' => self.cursor_col = self.cursor_col.saturating_sub(n(1) as usize),
            'H' | 'f' => {
                let mut it = params.iter();
                let row = it.next().and_then(|p| p.first().copied()).unwrap_or(1).max(1);
                let col = it.next().and_then(|p| p.first().copied()).unwrap_or(1).max(1);
                self.cursor_row = (row - 1) as usize;
                self.cursor_col = (col - 1) as usize;
                self.clamp_cursor();
            }
            'G' => {
                self.cursor_col = (n(1) as usize).saturating_sub(1);
                self.clamp_cursor();
            }
            'd' => {
                self.cursor_row = (n(1) as usize).saturating_sub(1);
                self.clamp_cursor();
            }
            'J' => self.erase_display(n(0).min(3)),
            'K' => self.erase_line(n(0).min(2)),
            'm' => self.sgr(params),
            's' => self.saved_cursor = (self.cursor_row, self.cursor_col),
            'u' => (self.cursor_row, self.cursor_col) = self.saved_cursor,
            'r' => {
                let mut it = params.iter();
                let top = it.next().and_then(|p| p.first().copied());
                let bottom = it.next().and_then(|p| p.first().copied());
                self.set_scroll_region(top, bottom);
            }
            'S' => self.scroll_up_region(n(1) as usize),
            'T' => self.scroll_down_region(n(1) as usize),
            'L' => self.insert_lines(n(1) as usize),
            'M' => self.delete_lines(n(1) as usize),
            '@' => self.insert_chars(n(1) as usize),
            'P' => self.delete_chars(n(1) as usize),
            'X' => self.erase_chars(n(1) as usize),
            'E' => {
                self.cursor_row = (self.cursor_row + n(1) as usize).min(self.rows - 1);
                self.cursor_col = 0;
            }
            'F' => {
                self.cursor_row = self.cursor_row.saturating_sub(n(1) as usize);
                self.cursor_col = 0;
            }
            'Z' => {
                for _ in 0..n(1) {
                    if self.cursor_col == 0 {
                        break;
                    }
                    self.cursor_col = (self.cursor_col - 1) / 8 * 8;
                }
            }
            'b' => {
                if let Some(ch) = self.last_printed {
                    for _ in 0..n(1) {
                        self.put_char(ch);
                    }
                }
            }
            'n' => {
                let private = intermediates.first() == Some(&b'?');
                match n(0) {
                    5 if !private => self.responses.extend_from_slice(b"\x1b[0n"),
                    6 => {
                        let row = self.cursor_row + 1;
                        let col = self.cursor_col.min(self.cols.saturating_sub(1)) + 1;
                        let reply = if private {
                            format!("\x1b[?{row};{col}R")
                        } else {
                            format!("\x1b[{row};{col}R")
                        };
                        self.responses.extend_from_slice(reply.as_bytes());
                    }
                    _ => {}
                }
            }
            'c' => match intermediates.first() {
                None if n(0) == 0 => self.responses.extend_from_slice(b"\x1b[?6c"),
                Some(&b'>') => self.responses.extend_from_slice(b"\x1b[>0;95;0c"),
                _ => {}
            },
            't' => {
                if intermediates.is_empty() && n(0) == 18 {
                    let reply = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.responses.extend_from_slice(reply.as_bytes());
                }
            }
            'h' | 'l' => {
                if intermediates.first() == Some(&b'?') {
                    let enable = action == 'h';
                    for param in params.iter() {
                        match param.first().copied().unwrap_or(0) {
                            1 => self.application_cursor_keys = enable,
                            7 => self.autowrap = enable,
                            25 => self.cursor_visible = enable,
                            47 | 1047 | 1049 => self.set_alt_screen(enable),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match intermediates.first() {
            Some(&b'(') => {
                self.g0_graphics = byte == b'0';
                return;
            }
            Some(&b')') => {
                self.g1_graphics = byte == b'0';
                return;
            }
            Some(_) => return,
            None => {}
        }
        match byte {
            b'7' => self.saved_cursor = (self.cursor_row, self.cursor_col),
            b'8' => (self.cursor_row, self.cursor_col) = self.saved_cursor,
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor_col = 0;
                self.line_feed();
            }
            b'M' => self.reverse_line_feed(),
            b'c' => {
                self.attrs = Attrs::default();
                self.alt_screen = None;
                self.scroll_top = 0;
                self.scroll_bottom = self.rows.saturating_sub(1);
                self.application_cursor_keys = false;
                self.autowrap = true;
                self.cursor_visible = true;
                self.g0_graphics = false;
                self.g1_graphics = false;
                self.shift_out = false;
                self.last_printed = None;
                let blank = self.blank_cell();
                for row in self.cells.iter_mut() {
                    row.fill(blank);
                }
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
        if params.len() < 2 || params[1] != b"?" {
            return;
        }
        let (code, color) = match params[0] {
            b"10" => ("10", default_fg()),
            b"11" => ("11", default_bg()),
            _ => return,
        };
        let c = (color.l * 255.) as u16;
        let terminator = if bell_terminated { "\x07" } else { "\x1b\\" };
        let reply =
            format!("\x1b]{code};rgb:{c:02x}{c:02x}/{c:02x}{c:02x}/{c:02x}{c:02x}{terminator}");
        self.responses.extend_from_slice(reply.as_bytes());
    }
}

fn dec_graphics(ch: char) -> char {
    match ch {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        '_' => ' ',
        other => other,
    }
}

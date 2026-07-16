use gpui::{hsla, Hsla};
use vte::{Params, Perform};

#[derive(Clone, Copy, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Hsla,
    pub bg: Option<Hsla>,
    pub bold: bool,
}

impl Cell {
    fn blank(attrs: &Attrs) -> Self {
        Self {
            ch: ' ',
            fg: attrs.fg,
            bg: attrs.bg,
            bold: attrs.bold,
        }
    }
}

#[derive(Clone, Copy)]
struct Attrs {
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    reverse: bool,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            fg: default_fg(),
            bg: None,
            bold: false,
            reverse: false,
        }
    }
}

fn default_fg() -> Hsla {
    hsla(0., 0., 0.85, 1.)
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

/// The primary screen's content and cursor, stashed away while an alternate
/// screen (vim, htop, less, ...) is active.
struct SavedPrimaryScreen {
    cells: Vec<Vec<Cell>>,
    cursor: (usize, usize),
}

/// A fixed-size character grid fed by a VTE parser. Supports the subset of
/// ANSI/VT100/xterm sequences needed by shells (cmd, PowerShell, bash) as
/// well as full-screen TUIs (vim, htop, less, nano): cursor movement, erase,
/// SGR colors, line wrapping, scroll regions, line/character insert-delete,
/// the alternate screen buffer, and application cursor-key mode.
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    cells: Vec<Vec<Cell>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub cursor_visible: bool,
    attrs: Attrs,
    saved_cursor: (usize, usize),
    parser: vte::Parser,
    /// Inclusive scroll region rows (DECSTBM). Defaults to the whole screen.
    scroll_top: usize,
    scroll_bottom: usize,
    autowrap: bool,
    /// DECCKM: arrow keys send `ESC O x` instead of `ESC [ x` when set.
    pub application_cursor_keys: bool,
    alt_screen: Option<SavedPrimaryScreen>,
}

impl Grid {
    pub fn new(rows: usize, cols: usize) -> Self {
        let attrs = Attrs::default();
        Self {
            rows,
            cols,
            cells: vec![vec![Cell::blank(&attrs); cols]; rows],
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            attrs,
            saved_cursor: (0, 0),
            parser: vte::Parser::new(),
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            autowrap: true,
            application_cursor_keys: false,
            alt_screen: None,
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
        self.cells = Self::resized_cells(old_cells, rows, cols);
        if let Some(alt) = &mut self.alt_screen {
            let alt_cells = std::mem::take(&mut alt.cells);
            alt.cells = Self::resized_cells(alt_cells, rows, cols);
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

    fn resized_cells(mut cells: Vec<Vec<Cell>>, new_rows: usize, new_cols: usize) -> Vec<Vec<Cell>> {
        let blank = Cell {
            ch: ' ',
            fg: default_fg(),
            bg: None,
            bold: false,
        };
        for row in &mut cells {
            if new_cols > row.len() {
                row.extend(std::iter::repeat(blank).take(new_cols - row.len()));
            } else {
                row.truncate(new_cols);
            }
        }
        let current_rows = cells.len();
        if new_rows > current_rows {
            for _ in 0..(new_rows - current_rows) {
                cells.push(vec![blank; new_cols]);
            }
        } else if new_rows < current_rows {
            cells.drain(0..current_rows - new_rows);
        }
        cells
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::replace(&mut self.parser, vte::Parser::new());
        for byte in bytes {
            parser.advance(self, *byte);
        }
        self.parser = parser;
    }

    fn blank_row(&self) -> Vec<Cell> {
        vec![Cell::blank(&self.attrs); self.cols]
    }

    fn line_feed(&mut self) {
        if self.cursor_row == self.scroll_bottom {
            self.scroll_up_region(1);
        } else if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        }
    }

    fn scroll_up_region(&mut self, n: usize) {
        let blank = self.blank_row();
        for _ in 0..n {
            if self.scroll_top <= self.scroll_bottom && self.scroll_bottom < self.rows {
                self.cells.remove(self.scroll_top);
                self.cells.insert(self.scroll_bottom, blank.clone());
            }
        }
    }

    fn scroll_down_region(&mut self, n: usize) {
        let blank = self.blank_row();
        for _ in 0..n {
            if self.scroll_top <= self.scroll_bottom && self.scroll_bottom < self.rows {
                self.cells.remove(self.scroll_bottom);
                self.cells.insert(self.scroll_top, blank.clone());
            }
        }
    }

    fn insert_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let blank = self.blank_row();
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        for _ in 0..n {
            self.cells.remove(self.scroll_bottom);
            self.cells.insert(self.cursor_row, blank.clone());
        }
    }

    fn delete_lines(&mut self, n: usize) {
        if self.cursor_row < self.scroll_top || self.cursor_row > self.scroll_bottom {
            return;
        }
        let blank = self.blank_row();
        let n = n.min(self.scroll_bottom - self.cursor_row + 1);
        for _ in 0..n {
            self.cells.remove(self.cursor_row);
            self.cells.insert(self.scroll_bottom, blank.clone());
        }
    }

    fn insert_chars(&mut self, n: usize) {
        let blank = Cell::blank(&self.attrs);
        let row = &mut self.cells[self.cursor_row];
        let n = n.min(self.cols - self.cursor_col);
        row.truncate(self.cols - n);
        for _ in 0..n {
            row.insert(self.cursor_col, blank);
        }
    }

    fn delete_chars(&mut self, n: usize) {
        let blank = Cell::blank(&self.attrs);
        let row = &mut self.cells[self.cursor_row];
        let n = n.min(self.cols - self.cursor_col);
        row.drain(self.cursor_col..self.cursor_col + n);
        row.extend(std::iter::repeat(blank).take(n));
    }

    fn erase_chars(&mut self, n: usize) {
        let blank = Cell::blank(&self.attrs);
        let n = n.min(self.cols - self.cursor_col);
        for c in &mut self.cells[self.cursor_row][self.cursor_col..self.cursor_col + n] {
            *c = blank;
        }
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

    fn set_alt_screen(&mut self, enable: bool) {
        if enable {
            if self.alt_screen.is_none() {
                let blank = self.blank_row_grid();
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
    }

    fn blank_row_grid(&self) -> Vec<Vec<Cell>> {
        vec![self.blank_row(); self.rows]
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            if self.autowrap {
                self.cursor_col = 0;
                self.line_feed();
            } else {
                self.cursor_col = self.cols - 1;
            }
        }
        self.cells[self.cursor_row][self.cursor_col] = Cell {
            ch,
            fg: if self.attrs.reverse {
                self.attrs.bg.unwrap_or(default_fg())
            } else {
                self.attrs.fg
            },
            bg: if self.attrs.reverse {
                Some(self.attrs.fg)
            } else {
                self.attrs.bg
            },
            bold: self.attrs.bold,
        };
        self.cursor_col += 1;
    }

    fn erase_line(&mut self, mode: u16) {
        let blank = Cell::blank(&self.attrs);
        let row = &mut self.cells[self.cursor_row];
        match mode {
            0 => {
                for c in &mut row[self.cursor_col..] {
                    *c = blank;
                }
            }
            1 => {
                for c in &mut row[..=self.cursor_col.min(self.cols - 1)] {
                    *c = blank;
                }
            }
            2 => {
                for c in row.iter_mut() {
                    *c = blank;
                }
            }
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for row in self.cursor_row + 1..self.rows {
                    self.cells[row] = self.blank_row();
                }
            }
            1 => {
                self.erase_line(1);
                for row in 0..self.cursor_row {
                    self.cells[row] = self.blank_row();
                }
            }
            2 | 3 => {
                self.cells = self.blank_row_grid();
            }
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        let values: Vec<u16> = params.iter().map(|p| p.first().copied().unwrap_or(0)).collect();
        if values.is_empty() {
            self.attrs = Attrs::default();
            return;
        }
        let mut i = 0;
        while i < values.len() {
            match values[i] {
                0 => self.attrs = Attrs::default(),
                1 => self.attrs.bold = true,
                22 => self.attrs.bold = false,
                7 => self.attrs.reverse = true,
                27 => self.attrs.reverse = false,
                39 => self.attrs.fg = default_fg(),
                49 => self.attrs.bg = None,
                30..=37 => self.attrs.fg = ansi_color(values[i] - 30, self.attrs.bold),
                90..=97 => self.attrs.fg = ansi_color(values[i] - 90, true),
                40..=47 => self.attrs.bg = Some(ansi_color(values[i] - 40, false)),
                100..=107 => self.attrs.bg = Some(ansi_color(values[i] - 100, true)),
                38 | 48 => {
                    let is_fg = values[i] == 38;
                    if i + 1 < values.len() && values[i + 1] == 5 && i + 2 < values.len() {
                        let color = palette_256(values[i + 2]);
                        if is_fg {
                            self.attrs.fg = color;
                        } else {
                            self.attrs.bg = Some(color);
                        }
                        i += 2;
                    } else if i + 1 < values.len() && values[i + 1] == 2 && i + 4 < values.len() {
                        let r = values[i + 2] as f32 / 255.;
                        let g = values[i + 3] as f32 / 255.;
                        let b = values[i + 4] as f32 / 255.;
                        let color = rgb_to_hsla(r, g, b);
                        if is_fg {
                            self.attrs.fg = color;
                        } else {
                            self.attrs.bg = Some(color);
                        }
                        i += 4;
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
            b'\n' => self.line_feed(),
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

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            b'7' => self.saved_cursor = (self.cursor_row, self.cursor_col),
            b'8' => (self.cursor_row, self.cursor_col) = self.saved_cursor,
            b'c' => {
                self.attrs = Attrs::default();
                self.alt_screen = None;
                self.scroll_top = 0;
                self.scroll_bottom = self.rows.saturating_sub(1);
                self.application_cursor_keys = false;
                self.autowrap = true;
                self.cells = self.blank_row_grid();
                self.cursor_row = 0;
                self.cursor_col = 0;
            }
            _ => {}
        }
    }
}

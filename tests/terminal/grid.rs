use super::*;

#[test]
fn reverse_video_swaps_the_default_colours() {
    let palette = TerminalPalette::default();
    let mut grid = Grid::new(2, 10);
    grid.feed(b"\x1b[7mx\x1b[27my");

    let row = grid.line_cells(grid.screen_top_line()).expect("row");
    assert_eq!(row[0].fg.as_fg(&palette), palette.background);
    assert_eq!(row[0].bg.as_bg(&palette), Some(palette.foreground));
    assert_eq!(row[1].fg.as_fg(&palette), palette.foreground);
    assert_eq!(row[1].bg.as_bg(&palette), None);
}

#[test]
fn a_combining_accent_joins_the_letter_before_it() {
    let mut grid = Grid::new(2, 10);
    grid.feed("inte\u{301}gre\u{301}!".as_bytes());

    let row = grid.line_cells(grid.screen_top_line()).expect("row");
    let text: String = row.iter().map(|cell| cell.ch()).collect();
    assert_eq!(text.trim_end(), "intégré!");
    assert_eq!(grid.cursor_col, 8);
}

#[test]
fn a_combining_mark_with_no_letter_before_it_is_dropped() {
    let mut grid = Grid::new(2, 10);
    grid.feed("\u{301}a".as_bytes());

    let row = grid.line_cells(grid.screen_top_line()).expect("row");
    assert_eq!(row[0].ch(), 'a');
    assert_eq!(grid.cursor_col, 1);
}

fn snapshot(cells: &[Cell]) -> Vec<(char, u32, Color, Color)> {
    cells
        .iter()
        .map(|cell| (cell.ch(), cell.ch_flags & !CHAR_MASK, cell.fg, cell.bg))
        .collect()
}

#[test]
fn scrolled_off_lines_keep_their_text_and_style() {
    let mut grid = Grid::new(2, 20);
    grid.feed(
        "plain\r\n\x1b[1;31mré\x1b[0m-\x1b[4;44;38;2;1;2;3m日本\x1b[0m \x1b[7mx\x1b[0m\r\n"
            .as_bytes(),
    );
    let styled = snapshot(&grid.line_cells(grid.screen_top_line()).expect("row"));
    grid.feed(b"\r\n\r\n");

    assert_eq!(grid.scrollback_len(), 3);
    let plain = grid.line_cells(0).expect("plain line");
    assert!(matches!(plain, Cow::Owned(_)));
    let text: String = plain.iter().map(|cell| cell.ch()).collect();
    assert_eq!(text, "plain");

    let kept = grid.line_cells(1).expect("styled line");
    let used = styled
        .iter()
        .rposition(|c| c.0 != ' ' || c.3 != Color::Default);
    assert!(snapshot(&kept) == styled[..used.map_or(0, |i| i + 1)]);
}

#[test]
fn a_default_styled_line_stores_no_runs() {
    let line = ScrollbackLine::new(
        &[
            Cell::BLANK,
            Cell::new('a', Color::Default, Color::Default, 0),
        ],
        &mut Vec::new(),
    );
    assert_eq!(line.runs, 0);
    assert_eq!(&*line.data, b" a");

    let cells = [
        Cell::new('a', Color::Indexed(1), Color::Default, FLAG_BOLD),
        Cell::new('é', Color::Indexed(1), Color::Default, FLAG_BOLD),
        Cell::new('c', Color::DefaultBg, Color::Rgb(1, 2, 3), FLAG_DIM),
        Cell::new('d', Color::Default, Color::DefaultFg, 0),
    ];
    let styled = ScrollbackLine::new(&cells, &mut Vec::new());
    assert_eq!(styled.runs, 3);
    assert!(snapshot(&styled.cells()) == snapshot(&cells));
    assert!(ScrollbackLine::new(&[], &mut Vec::new()).cells().is_empty());
}

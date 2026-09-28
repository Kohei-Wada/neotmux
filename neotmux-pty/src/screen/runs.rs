use neotmux_proto::Color;
use neotmux_proto::screen::Run;

/// One row as the runs it is drawn in: consecutive cells that look the same, joined.
///
/// The text is what [`crate::screen::lines::row`] reads for the same cells — a wide character
/// once, at the column it starts in; a blank cell as a space — so a run's text is a slice of
/// that row's line. The blank rest of the row is left off where it is only the terminal's own
/// blank, default colours and no attribute; a blank the program painted, a coloured
/// background, is a run like any other.
pub(crate) fn row(screen: &vt100::Screen, row: u16, cols: u16) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut col = 0;
    while col < cols {
        let Some(cell) = screen.cell(row, col) else {
            break;
        };
        if cell.is_wide_continuation() {
            col += 1;
            continue;
        }
        let text = match cell.contents() {
            "" => " ",
            text => text,
        };
        let this = Run {
            text: text.to_string(),
            fg: color(cell.fgcolor()),
            bg: color(cell.bgcolor()),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
        };
        match runs.last_mut() {
            Some(last) if last.looks_like(&this) => last.text.push_str(&this.text),
            _ => runs.push(this),
        }
        col += if cell.is_wide() { 2 } else { 1 };
    }
    trim(&mut runs);
    runs
}

/// The terminal's own blank at the end of a row, taken off: what a trimmed line cuts, and no
/// more.
fn trim(runs: &mut Vec<Run>) {
    while let Some(last) = runs.last_mut() {
        if !last.looks_like(&Run::default()) {
            break;
        }
        let kept = last.text.trim_end().len();
        if kept > 0 {
            last.text.truncate(kept);
            break;
        }
        runs.pop();
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Idx(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The visible screen as runs, top to bottom, `last` rows from the bottom as
/// [`crate::screen::lines::visible`] takes them.
pub fn visible(screen: &vt100::Screen, last: Option<usize>) -> Vec<Vec<Run>> {
    let (rows, cols) = screen.size();
    (crate::screen::lines::first_row(rows, last)..rows)
        .map(|r| row(screen, r, cols))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(feed: &[u8]) -> Vec<Vec<Run>> {
        let mut parser = vt100::Parser::new(3, 20, 0);
        parser.process(feed);
        visible(parser.screen(), None)
    }

    fn plain(text: &str) -> Run {
        Run {
            text: text.to_string(),
            ..Run::default()
        }
    }

    #[test]
    fn a_coloured_word_is_a_run_of_its_own_between_plain_ones() {
        let rows = runs(b"say \x1b[1;31mred\x1b[0m now");
        assert_eq!(
            rows[0],
            vec![
                plain("say "),
                Run {
                    text: "red".to_string(),
                    fg: Color::Idx(1),
                    bold: true,
                    ..Run::default()
                },
                plain(" now"),
            ]
        );
        assert!(
            rows[1].is_empty(),
            "an untouched row is no runs: {:?}",
            rows[1]
        );
    }

    /// A background the program painted is part of the picture even where there is no text,
    /// so it survives the trim that takes the terminal's own blank off.
    #[test]
    fn a_painted_blank_is_kept_and_the_terminals_blank_is_not() {
        let rows = runs(b"x\x1b[44m  \x1b[0m");
        assert_eq!(
            rows[0],
            vec![
                plain("x"),
                Run {
                    text: "  ".to_string(),
                    bg: Color::Idx(4),
                    ..Run::default()
                },
            ]
        );
    }

    #[test]
    fn rgb_and_the_other_attributes_are_read() {
        let rows = runs(b"\x1b[38;2;1;2;3;2;3;4;7mz");
        assert_eq!(
            rows[0],
            vec![Run {
                text: "z".to_string(),
                fg: Color::Rgb(1, 2, 3),
                dim: true,
                italic: true,
                underline: true,
                inverse: true,
                ..Run::default()
            }]
        );
    }

    #[test]
    fn a_wide_character_is_one_run_text_the_way_a_line_reads_it() {
        let rows = runs("作業 ok".as_bytes());
        assert_eq!(rows[0], vec![plain("作業 ok")]);
    }
}

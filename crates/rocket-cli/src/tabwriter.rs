//! The subset of Go's `text/tabwriter` that rocket uses: left-aligned
//! columns, `minwidth 0`, `padding 2`, space padding, no flags
//! (`tabwriter.NewWriter(out, 0, 2, 2, ' ', 0)`).
//!
//! A cell is terminated by a tab; the last cell of a line is not and never
//! takes part in column width. A column block is a run of consecutive lines
//! that all have a terminated cell in that column, so a short line ends the
//! block exactly like in Go.

const PADDING: usize = 2;

/// Rows of cells; the last cell of each row is the (unpadded) trailing text.
#[derive(Debug, Default)]
pub struct Table {
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a line. `cells` are the tab-separated fields of the line.
    pub fn row<I, S>(&mut self, cells: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut row: Vec<String> = cells.into_iter().map(Into::into).collect();
        if row.is_empty() {
            row.push(String::new());
        }
        self.rows.push(row);
    }

    /// The aligned text, one `\n`-terminated line per row.
    pub fn render(&self) -> String {
        let mut out = String::new();
        format(&self.rows, &mut Vec::new(), 0, self.rows.len(), &mut out);
        out
    }
}

fn width(cell: &str) -> usize {
    cell.chars().count()
}

fn format(
    lines: &[Vec<String>],
    widths: &mut Vec<usize>,
    line0: usize,
    line1: usize,
    out: &mut String,
) {
    let column = widths.len();
    let mut line0 = line0;
    let mut this = line0;
    while this < line1 {
        // A line starts a block when it has a terminated cell in this column.
        if column >= lines[this].len() - 1 {
            this += 1;
            continue;
        }
        write_lines(lines, widths, line0, this, out);
        line0 = this;
        let mut w = 0;
        while this < line1 && column < lines[this].len() - 1 {
            w = w.max(width(&lines[this][column]) + PADDING);
            this += 1;
        }
        widths.push(w);
        format(lines, widths, line0, this, out);
        widths.pop();
        line0 = this;
        this += 1;
    }
    write_lines(lines, widths, line0, line1, out);
}

fn write_lines(
    lines: &[Vec<String>],
    widths: &[usize],
    line0: usize,
    line1: usize,
    out: &mut String,
) {
    for line in &lines[line0..line1] {
        for (j, cell) in line.iter().enumerate() {
            out.push_str(cell);
            if let Some(w) = widths.get(j) {
                for _ in width(cell)..*w {
                    out.push(' ');
                }
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(rows: &[&[&str]]) -> String {
        let mut t = Table::new();
        for r in rows {
            t.row(r.iter().copied());
        }
        t.render()
    }

    #[test]
    fn aligns_columns_with_two_spaces_of_padding() {
        let got = render(&[
            &["SERVICE", "ACTION", "NOTE"],
            &["api", "started", ""],
            &["worker-long", "already_running", "x"],
        ]);
        assert_eq!(
            got,
            "SERVICE      ACTION           NOTE\n\
             api          started          \n\
             worker-long  already_running  x\n"
        );
    }

    #[test]
    fn short_lines_end_a_column_block() {
        // The middle line has no terminated cell in column 0, so the first
        // and last line are aligned independently.
        let got = render(&[&["a", "1"], &["wide-cell"], &["bb", "2"]]);
        assert_eq!(got, "a  1\nwide-cell\nbb  2\n");
    }

    #[test]
    fn counts_runes_not_bytes() {
        let got = render(&[&["caf\u{e9}", "x"], &["ab", "y"]]);
        assert_eq!(got, "caf\u{e9}  x\nab    y\n");
    }

    #[test]
    fn single_cell_rows_are_unpadded() {
        assert_eq!(render(&[&["one"], &["two"]]), "one\ntwo\n");
        assert_eq!(Table::new().render(), "");
    }

    #[test]
    fn nested_blocks_use_their_own_widths() {
        let got = render(&[
            &["a", "bbbb", "c"],
            &["aaa", "b", "c"],
            &["a", "x"],
            &["a", "bb", "c"],
        ]);
        assert_eq!(
            got,
            "a    bbbb  c\n\
             aaa  b     c\n\
             a    x\n\
             a    bb  c\n"
        );
    }
}

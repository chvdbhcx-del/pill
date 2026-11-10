//! Markdown for AI answers = `glasskit::markdown` (bold, italic, strikethrough, lists, quotes, headings, fenced code with a copy button)
//! plus the two GitHub extensions glasskit has no block for:
//! * task lists: `- [ ] a` / `- [x] b` show a ballot box (the box is part of the item text, so selecting and copying still work),
//! * tables: a header row, a `---|:---:` separator row and body rows. glasskit cannot lay out a grid, so a table that fits is a
//!   monospace block (language label `table`, its copy button copies the aligned text) and a wide one becomes a list with one
//!   `**first cell** - header: value` item per row. Cell markup is flattened to plain text.
//! Pure data in, pure data out. Both the Chat tab and the command-bar answer call [`parse`] instead of `markdown::parse`.
use glasskit::markdown::{self, Block, Inline, ListItem};

/// Widest aligned table (characters) kept as a grid; wider ones read better as a list in the narrow island column.
const GRID_MAX: usize = 46;
const MAX_COLS: usize = 12;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

pub fn parse(text: &str) -> Vec<Block> {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = norm.split('\n').collect();
    let mut out = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    let (mut fenced, mut i) = (false, 0);
    while i < lines.len() {
        let l = lines[i];
        if l.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && l.len() - l.trim_start().len() <= 3 {
            if let Some(aligns) = lines.get(i + 1).filter(|_| l.contains('|')).and_then(|s| separator(s)) {
                let head = cells(l);
                if head.len() == aligns.len() && head.len() <= MAX_COLS {
                    let rows: Vec<Vec<String>> = lines[i + 2..].iter().take_while(|r| !r.trim().is_empty() && r.contains('|')).map(|r| cells(r)).collect();
                    flush(&mut buf, &mut out);
                    out.push(render(&head, &aligns, &rows));
                    i += 2 + rows.len();
                    continue;
                }
            }
        }
        buf.push(l);
        i += 1;
    }
    flush(&mut buf, &mut out);
    for b in &mut out {
        tasks(b);
    }
    out
}

fn flush(buf: &mut Vec<&str>, out: &mut Vec<Block>) {
    if !buf.is_empty() {
        out.extend(markdown::parse(&buf.join("\n")));
        buf.clear();
    }
}

/// `| a | b |` -> ["a", "b"] (outer pipes optional, `\|` is a literal pipe); markup removed.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") { &t[..t.len() - 1] } else { t };
    let (mut out, mut cur, mut esc) = (Vec::new(), String::new(), false);
    for c in t.chars() {
        match (esc, c) {
            (true, '|') => cur.push('|'),
            (true, c) => {
                cur.push('\\');
                cur.push(c);
            }
            (false, '\\') => {
                esc = true;
                continue;
            }
            (false, '|') => out.push(std::mem::take(&mut cur)),
            (false, c) => cur.push(c),
        }
        esc = false;
    }
    out.push(cur);
    out.iter().map(|c| markdown::inline_plain(&markdown::inline(c.trim())).replace('\n', " ")).collect()
}

/// The `---|:--:|--:` row: every cell dashes with optional colons. None = not a separator.
fn separator(line: &str) -> Option<Vec<Align>> {
    if !line.contains('-') || !line.contains('|') {
        return None;
    }
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|')
        .map(|c| {
            let c = c.trim();
            let (l, r) = (c.starts_with(':'), c.ends_with(':'));
            let d = c.trim_matches(':');
            (!d.is_empty() && d.chars().all(|x| x == '-')).then_some(match (l, r) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

fn pad(s: &str, w: usize, a: Align) -> String {
    let gap = w.saturating_sub(s.chars().count()); // ponytail: wide (CJK / emoji) glyphs count as one column, so such a grid is a little ragged
    match a {
        Align::Left => format!("{s}{}", " ".repeat(gap)),
        Align::Right => format!("{}{s}", " ".repeat(gap)),
        Align::Center => format!("{}{s}{}", " ".repeat(gap / 2), " ".repeat(gap - gap / 2)),
    }
}

fn render(head: &[String], aligns: &[Align], rows: &[Vec<String>]) -> Block {
    let n = head.len();
    let row = |r: &Vec<String>| -> Vec<String> { (0..n).map(|i| r.get(i).cloned().unwrap_or_default()).collect() };
    let body: Vec<Vec<String>> = rows.iter().map(row).collect();
    let widths: Vec<usize> = (0..n).map(|i| std::iter::once(head[i].chars().count()).chain(body.iter().map(|r| r[i].chars().count())).max().unwrap_or(0).max(1)).collect();
    if widths.iter().sum::<usize>() + 3 * (n - 1) <= GRID_MAX {
        let line = |r: &[String]| r.iter().enumerate().map(|(i, c)| pad(c, widths[i], aligns[i])).collect::<Vec<_>>().join(" | ").trim_end().to_string();
        let rule = widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("-+-");
        let mut text = vec![line(head), rule];
        text.extend(body.iter().map(|r| line(r)));
        return Block::Code { lang: "table".into(), text: text.join("\n") };
    }
    let items = body
        .iter()
        .map(|r| {
            let rest: Vec<String> = (1..n).filter(|&i| !r[i].is_empty()).map(|i| format!("{}: {}", head[i], r[i])).collect();
            let mut content = vec![Inline::Bold(vec![Inline::Text(r[0].clone())])];
            if !rest.is_empty() {
                content.push(Inline::Text(format!(" - {}", rest.join(" \u{b7} "))));
            }
            ListItem { content, children: Vec::new() }
        })
        .collect();
    Block::List { ordered: false, start: 1, items }
}

/// `[ ] ` / `[x] ` at the start of a list item -> ballot box.
fn tasks(b: &mut Block) {
    match b {
        Block::List { items, .. } => {
            for it in items {
                if let Some(Inline::Text(t)) = it.content.first_mut() {
                    for (mark, box_) in [("[ ] ", "\u{2610} "), ("[x] ", "\u{2611} "), ("[X] ", "\u{2611} ")] {
                        if let Some(rest) = t.strip_prefix(mark) {
                            *t = format!("{box_}{rest}");
                            break;
                        }
                    }
                }
                it.children.iter_mut().for_each(tasks);
            }
        }
        Block::Quote(bs) => bs.iter_mut().for_each(tasks),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(b: &Block) -> &str {
        match b {
            Block::Code { lang, text } if lang == "table" => text,
            o => panic!("not a table: {o:?}"),
        }
    }

    #[test]
    fn a_small_table_is_an_aligned_block() {
        let b = parse("Before\n| Name | Qty |\n|:--|--:|\n| **apple** | 3 |\n| fig | 12 |\n\nAfter");
        assert_eq!(b.len(), 3);
        assert!(matches!(&b[0], Block::Paragraph(_)) && matches!(&b[2], Block::Paragraph(_)));
        assert_eq!(code(&b[1]), "Name  | Qty\n------+----\napple |   3\nfig   |  12");
    }

    #[test]
    fn center_alignment_pipes_and_missing_cells() {
        let b = parse("a|b|c\n:-:|-|-:\n1|2\nx \\| y|z|w");
        assert_eq!(code(&b[0]), "  a   | b | c
------+---+--
  1   | 2 |
x | y | z | w");
    }

    #[test]
    fn a_wide_table_becomes_a_list() {
        let b = parse("| Tool | Purpose | Notes |\n|---|---|---|\n| ripgrep | searches file contents very quickly | respects .gitignore |\n| fd | finds files | |");
        match &b[0] {
            Block::List { ordered: false, items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(markdown::inline_plain(&items[0].content), "ripgrep - Purpose: searches file contents very quickly \u{b7} Notes: respects .gitignore");
                assert_eq!(markdown::inline_plain(&items[1].content), "fd - Purpose: finds files");
                assert!(matches!(items[0].content[0], Inline::Bold(_)));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn not_a_table_stays_text() {
        for s in ["a | b\njust text", "| a |\n| b |", "x\n---", "a | b\n--- | ---- | ---", "```\n| a | b |\n|---|---|\n```"] {
            assert!(!parse(s).iter().any(|b| matches!(b, Block::Code { lang, .. } if lang == "table")), "{s}");
        }
        // inside a fence the table is verbatim code
        assert_eq!(parse("```\n| a | b |\n|---|---|\n```")[0], Block::Code { lang: String::new(), text: "| a | b |\n|---|---|".into() });
    }

    #[test]
    fn a_streaming_table_settles_as_rows_arrive() {
        assert!(matches!(parse("| a | b |")[0], Block::Paragraph(_)));
        assert!(matches!(parse("| a | b |\n|---|")[0], Block::Paragraph(_)), "column counts differ");
        assert_eq!(code(&parse("| a | b |\n|---|---|")[0]), "a | b\n--+--");
        assert_eq!(code(&parse("| a | b |\n|---|---|\n| 1")[0]), "a | b\n--+--\n1 |");
    }

    #[test]
    fn task_lists_get_ballot_boxes_also_nested() {
        let b = parse("- [ ] write\n- [x] test **it**\n  - [X] deep\n- [y] plain");
        let Block::List { items, .. } = &b[0] else { panic!() };
        assert_eq!(markdown::inline_plain(&items[0].content), "\u{2610} write");
        assert_eq!(markdown::inline_plain(&items[1].content), "\u{2611} test it");
        assert_eq!(markdown::inline_plain(&items[2].content), "[y] plain");
        let Block::List { items: sub, .. } = &items[1].children[0] else { panic!() };
        assert_eq!(markdown::inline_plain(&sub[0].content), "\u{2611} deep");
    }

    #[test]
    fn strike_bold_italic_and_hostile_text_pass_through() {
        let b = parse("~~old~~ **new** *it* <script>x</script>");
        let Block::Paragraph(c) = &b[0] else { panic!() };
        assert!(c.iter().any(|i| matches!(i, Inline::Strike(_))) && c.iter().any(|i| matches!(i, Inline::Bold(_))) && c.iter().any(|i| matches!(i, Inline::Italic(_))));
        assert_eq!(markdown::blocks_plain(&parse("")), "");
    }

    #[test]
    fn pathological_tables_stay_fast() {
        let t0 = std::time::Instant::now();
        let s = format!("{}\n{}\n{}", "|a".repeat(20000), "|-".repeat(20000), "|x".repeat(20000));
        let _ = parse(&s);
        let row = "| a ".repeat(500);
        let many = format!("{row}|\n{}|\n{}", "|---".repeat(500), format!("{row}|\n").repeat(500));
        let _ = parse(&many);
        assert!(t0.elapsed().as_secs_f32() < 2.0, "{:?}", t0.elapsed());
    }
}

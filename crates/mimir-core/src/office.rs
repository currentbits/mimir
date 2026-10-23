//! Office files a bot sends: a workspace text file turned into Word (.docx) or PDF from markdown, or Excel (.xlsx) from
//! CSV or a markdown table. Built here (Office files are zipped XML, a PDF is text and offsets): nothing installed or
//! downloaded, no scripts. The source stays the plain-text file in the workspace, so the bot can keep editing it. Only
//! text goes in: no links, macros or formulas (a cell starting with = stays text), so outside content stays inert.
//! ponytail: no slides (.pptx), no images, no fonts beyond Helvetica in PDFs (Latin-1 only; other letters print as ?).

use anyhow::{Result, bail};
use std::io::Write;

/// The file `name` (a workspace path) turned into `format`: its new name and bytes.
pub fn make(format: &str, name: &str, text: &str) -> Result<(String, Vec<u8>)> {
    let file = name.rsplit('/').next().unwrap_or(name);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let bytes = match format {
        "docx" => docx(&blocks(name, text)),
        "pdf" => pdf(&blocks(name, text)),
        "xlsx" => xlsx(&table(name, text).ok_or_else(|| anyhow::anyhow!("{name} has no table; write it as CSV or a markdown table"))?),
        _ => bail!("format is docx, xlsx or pdf"),
    }?;
    Ok((format!("{stem}.{format}"), bytes))
}

#[derive(Debug, PartialEq)]
enum Block {
    Heading(u8, String),
    Para(String),
    /// A list item: its mark (• or 1.) and text.
    Item(String, String),
    Table(Vec<Vec<String>>),
    Rule,
}

fn sep(name: &str) -> Option<char> {
    let n = name.to_lowercase();
    if n.ends_with(".csv") { Some(',') } else if n.ends_with(".tsv") { Some('\t') } else { None }
}

/// Markdown's blocks (a CSV file is one table): headings, paragraphs, list items, tables, rules.
fn blocks(name: &str, text: &str) -> Vec<Block> {
    if let Some(c) = sep(name) {
        return vec![Block::Table(csv(text, c))];
    }
    let mut out = vec![];
    let mut para: Vec<&str> = vec![];
    let mut rows: Vec<Vec<String>> = vec![];
    let flush = |para: &mut Vec<&str>, rows: &mut Vec<Vec<String>>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(inline(&para.join(" "))));
            para.clear();
        }
        if !rows.is_empty() {
            out.push(Block::Table(std::mem::take(rows)));
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if let Some(r) = md_row(t) {
            if !para.is_empty() {
                flush(&mut para, &mut vec![], &mut out);
            }
            rows.extend(r);
            continue;
        }
        if !rows.is_empty() {
            flush(&mut vec![], &mut rows, &mut out);
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        let item = t.strip_prefix("- ").or(t.strip_prefix("* ")).or(t.strip_prefix("+ ")).map(|r| ("•".to_string(), r)).or_else(|| {
            let (n, rest) = t.split_once(". ")?;
            (!n.is_empty() && n.len() < 4 && n.chars().all(|c| c.is_ascii_digit())).then(|| (format!("{n}."), rest))
        });
        if t.is_empty() {
            flush(&mut para, &mut rows, &mut out);
        } else if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut rows, &mut out);
            out.push(Block::Heading(hashes.min(3) as u8, inline(&t[hashes..])));
        } else if t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*')) {
            flush(&mut para, &mut rows, &mut out);
            out.push(Block::Rule);
        } else if let Some((mark, i)) = item {
            flush(&mut para, &mut rows, &mut out);
            out.push(Block::Item(mark, inline(i.trim_start_matches("[ ] ").trim_start_matches("[x] "))));
        } else {
            para.push(t);
        }
    }
    flush(&mut para, &mut rows, &mut out);
    out
}

/// A markdown table row's cells (`| a | b |`); Some(None) for the `|---|---|` line under the header.
fn md_row(t: &str) -> Option<Option<Vec<String>>> {
    let inner = t.strip_prefix('|')?.strip_suffix('|')?;
    let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
    if cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':'))) {
        return Some(None);
    }
    Some(Some(cells.iter().map(|c| inline(c)).collect()))
}

/// Markdown's inline marks dropped (bold, code, links become "text (address)").
fn inline(s: &str) -> String {
    let mut s = s.trim().replace("**", "").replace("__", "").replace('`', "");
    while let Some(open) = s.find('[') {
        let Some(mid) = s[open..].find("](").map(|i| open + i) else { break };
        let Some(end) = s[mid..].find(')').map(|i| mid + i) else { break };
        s = format!("{}{} ({}){}", &s[..open], &s[open + 1..mid], &s[mid + 2..end], &s[end + 1..]);
    }
    s
}

/// CSV (quotes, "" inside quotes, line breaks inside quotes), or TSV with `c` = tab.
fn csv(text: &str, c: char) -> Vec<Vec<String>> {
    let (mut rows, mut row, mut cell, mut quoted) = (vec![], vec![], String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' if quoted || cell.is_empty() => quoted = !quoted,
            _ if ch == c && !quoted => row.push(std::mem::take(&mut cell)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            '\r' if !quoted => {}
            _ => cell.push(ch),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.retain(|r: &Vec<String>| r.iter().any(|c| !c.trim().is_empty()));
    rows
}

/// The rows of a CSV file, or of the first table in a markdown one.
fn table(name: &str, text: &str) -> Option<Vec<Vec<String>>> {
    blocks(name, text).into_iter().find_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).filter(|t| !t.is_empty())
}

/// Text for XML: escaped, without the control characters XML can't hold.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\t' | '\n' => o.push(' '),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => o.push(c),
        }
    }
    o
}

fn zipped(parts: &[(&str, String)]) -> Result<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    for (name, xml) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default())?;
        z.write_all(xml.as_bytes())?;
    }
    Ok(z.finish()?.into_inner())
}

const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const PKG_RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_RELS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn content_types(main: &str, main_type: &str, extra: &str) -> String {
    format!(
        r#"{XML}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/{main}" ContentType="application/vnd.openxmlformats-officedocument.{main_type}.main+xml"/>{extra}</Types>"#
    )
}

fn root_rels(main: &str) -> String {
    format!(r#"{XML}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/officeDocument" Target="{main}"/></Relationships>"#)
}

fn docx(blocks: &[Block]) -> Result<Vec<u8>> {
    let p = |props: &str, run: &str, text: &str| format!(r#"<w:p><w:pPr>{props}</w:pPr><w:r><w:rPr>{run}</w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, esc(text));
    let mut body = String::new();
    for b in blocks {
        body += &match b {
            Block::Heading(l, t) => {
                let size = [40, 32, 26][*l as usize - 1];
                p(&format!(r#"<w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="{}"/>"#, l - 1), &format!(r#"<w:b/><w:sz w:val="{size}"/>"#), t)
            }
            Block::Para(t) => p("", "", t),
            // The mark, a tab, the text: a line that wraps starts under the text.
            Block::Item(mark, t) => format!(
                r#"<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="360"/></w:tabs><w:spacing w:after="60"/><w:ind w:left="360" w:hanging="360"/></w:pPr><w:r><w:t>{}</w:t><w:tab/><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                esc(mark),
                esc(t)
            ),
            Block::Rule => r#"<w:p><w:pPr><w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="999999"/></w:pBdr></w:pPr></w:p>"#.into(),
            Block::Table(rows) => {
                let line = r#"w:val="single" w:sz="4" w:space="0" w:color="BBBBBB""#;
                let mut x = format!(
                    r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="pct"/><w:tblBorders><w:top {line}/><w:left {line}/><w:bottom {line}/><w:right {line}/><w:insideH {line}/><w:insideV {line}/></w:tblBorders></w:tblPr>"#
                );
                for (i, r) in rows.iter().enumerate() {
                    x += "<w:tr>";
                    for c in r {
                        x += &format!("<w:tc>{}</w:tc>", p(r#"<w:spacing w:after="0"/>"#, if i == 0 { "<w:b/>" } else { "" }, c));
                    }
                    x += "</w:tr>";
                }
                // Word wants a paragraph after a table.
                x + "</w:tbl><w:p/>"
            }
        };
    }
    let doc = format!(
        r#"{XML}<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="0" w:footer="0" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let styles = format!(
        r#"{XML}<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults></w:styles>"#
    );
    zipped(&[
        ("[Content_Types].xml", content_types("word/document.xml", "wordprocessingml.document", r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>"#)),
        ("_rels/.rels", root_rels("word/document.xml")),
        ("word/_rels/document.xml.rels", format!(r#"{XML}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/styles" Target="styles.xml"/></Relationships>"#)),
        ("word/document.xml", doc),
        ("word/styles.xml", styles),
    ])
}

/// A cell that is a number to Excel (so it sums): not a code with a leading 0 or +, like a postal code or phone number.
fn number(s: &str) -> Option<f64> {
    let s = s.trim();
    let b = s.as_bytes();
    let code = s.starts_with('+') || (b.len() > 1 && b[0] == b'0' && b[1] != b'.');
    s.parse::<f64>().ok().filter(|n| n.is_finite() && !code && !s.contains(['e', 'E']))
}

fn column(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            return s;
        }
        i = i / 26 - 1;
    }
}

fn xlsx(rows: &[Vec<String>]) -> Result<Vec<u8>> {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: String = (0..cols)
        .map(|c| {
            let w = rows.iter().filter_map(|r| r.get(c)).map(|v| v.chars().count()).max().unwrap_or(0).clamp(6, 60) + 2;
            format!(r#"<col min="{n}" max="{n}" width="{w}" customWidth="1"/>"#, n = c + 1)
        })
        .collect();
    let mut data = String::new();
    for (i, r) in rows.iter().enumerate() {
        data += &format!(r#"<row r="{}">"#, i + 1);
        for (c, v) in r.iter().enumerate() {
            let at = format!("{}{}", column(c), i + 1);
            let style = if i == 0 { r#" s="1""# } else { "" };
            data += &match number(v).filter(|_| i > 0) {
                Some(n) => format!(r#"<c r="{at}"{style}><v>{n}</v></c>"#),
                // Inline text, never a formula: "=..." from a page stays what it says.
                None => format!(r#"<c r="{at}"{style} t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, esc(&v.chars().take(32_767).collect::<String>())),
            };
        }
        data += "</row>";
    }
    let sheet = format!(
        r#"{XML}<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols>{widths}</cols><sheetData>{data}</sheetData></worksheet>"#
    );
    let styles = format!(
        r#"{XML}<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
    );
    let sml = "application/vnd.openxmlformats-officedocument.spreadsheetml";
    zipped(&[
        ("[Content_Types].xml", content_types("xl/workbook.xml", "spreadsheetml.sheet", &format!(r#"<Override PartName="/xl/worksheets/sheet1.xml" ContentType="{sml}.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="{sml}.styles+xml"/>"#))),
        ("_rels/.rels", root_rels("xl/workbook.xml")),
        ("xl/workbook.xml", format!(r#"{XML}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{DOC_RELS}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#)),
        ("xl/_rels/workbook.xml.rels", format!(r#"{XML}<Relationships xmlns="{PKG_RELS}"><Relationship Id="rId1" Type="{DOC_RELS}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{DOC_RELS}/styles" Target="styles.xml"/></Relationships>"#)),
        ("xl/worksheets/sheet1.xml", sheet),
        ("xl/styles.xml", styles),
    ])
}

// Helvetica and Helvetica-Bold advance widths (1/1000 em) for ' '..='~', from Adobe's AFM files.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500,
    667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500,
    722, 500, 500, 500, 334, 260, 334, 584,
];
const HELV_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556,
    722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556,
    778, 556, 556, 500, 389, 280, 389, 584,
];

/// A character in the PDF's WinAnsi encoding (Latin-1 plus typographic quotes and dashes), ? when it has none.
fn win(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{a0}'..='\u{ff}' => c as u32 as u8,
        '€' => 0x80,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '\t' => b' ',
        _ => b'?',
    }
}

fn width(s: &[u8], bold: bool, size: f32) -> f32 {
    let t = if bold { &HELV_BOLD } else { &HELV };
    s.iter()
        .map(|&b| match b {
            32..=126 => t[b as usize - 32],
            0x95 => 350,
            0x97 => 1000,
            0x91 | 0x92 => 222,
            0x85 => 1000,
            _ => 556,
        } as f32)
        .sum::<f32>()
        * size
        / 1000.0
}

/// Lines of at most `max` points wide: whole words, a word too long for a line split.
fn wrap(s: &str, bold: bool, size: f32, max: f32) -> Vec<Vec<u8>> {
    let mut lines = vec![];
    let mut line: Vec<u8> = vec![];
    for word in s.split_whitespace() {
        let mut w: Vec<u8> = word.chars().map(win).collect();
        loop {
            let with = if line.is_empty() { w.clone() } else { [&line[..], b" ", &w[..]].concat() };
            if width(&with, bold, size) <= max {
                line = with;
                break;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                continue;
            }
            let mut n = w.len();
            while n > 1 && width(&w[..n], bold, size) > max {
                n -= 1;
            }
            lines.push(w[..n].to_vec());
            w.drain(..n);
            if w.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// A4 pages of text drawn top to bottom.
struct Pages {
    done: Vec<Vec<u8>>,
    page: Vec<u8>,
    y: f32,
}

const W: f32 = 595.0;
const H: f32 = 842.0;
const M: f32 = 56.0;

impl Pages {
    fn room(&mut self, h: f32) {
        if self.y - h < M && !self.page.is_empty() {
            self.done.push(std::mem::take(&mut self.page));
            self.y = H - M;
        }
    }

    fn text(&mut self, x: f32, s: &[u8], bold: bool, size: f32) {
        let mut esc = vec![];
        for &b in s {
            if matches!(b, b'\\' | b'(' | b')') {
                esc.push(b'\\');
            }
            esc.push(b);
        }
        self.page.extend(format!("BT /F{} {size} Tf {x:.1} {:.1} Td (", if bold { 2 } else { 1 }, self.y - size).as_bytes());
        self.page.extend(esc);
        self.page.extend(b") Tj ET\n");
    }

    fn line(&mut self, x0: f32, x1: f32, y: f32) {
        self.page.extend(format!("0.6 G 0.5 w {x0:.1} {y:.1} m {x1:.1} {y:.1} l S 0 G\n").as_bytes());
    }

    fn lines(&mut self, x: f32, s: &str, bold: bool, size: f32, after: f32) {
        let lead = size * 1.4;
        for l in wrap(s, bold, size, W - M - x) {
            self.room(lead);
            self.text(x, &l, bold, size);
            self.y -= lead;
        }
        self.y -= after;
    }
}

fn pdf(blocks: &[Block]) -> Result<Vec<u8>> {
    let mut p = Pages { done: vec![], page: vec![], y: H - M };
    for b in blocks {
        match b {
            Block::Heading(l, t) => {
                let size = [20.0, 15.0, 12.5][*l as usize - 1];
                p.y -= size * 0.6;
                // A heading isn't left alone at a page's foot.
                p.room(size * 1.4 + 30.0);
                p.lines(M, t, true, size, 4.0);
            }
            Block::Para(t) => p.lines(M, t, false, 11.0, 7.0),
            Block::Item(mark, t) => {
                p.room(15.4);
                p.text(M + 4.0, &mark.chars().map(win).collect::<Vec<_>>(), false, 11.0);
                p.lines(M + 20.0, t, false, 11.0, 3.0);
            }
            Block::Rule => {
                p.room(14.0);
                p.line(M, W - M, p.y - 6.0);
                p.y -= 14.0;
            }
            Block::Table(rows) => {
                let n = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let size = if n > 4 { 9.0 } else { 10.0 };
                let col = (W - 2.0 * M) / n as f32;
                for (i, r) in rows.iter().enumerate() {
                    let cells: Vec<_> = (0..n).map(|c| wrap(r.get(c).map_or("", |s| s.as_str()), i == 0, size, col - 6.0)).collect();
                    let h = cells.iter().map(Vec::len).max().unwrap_or(1) as f32 * size * 1.3 + 5.0;
                    p.room(h);
                    let top = p.y;
                    for (c, ls) in cells.iter().enumerate() {
                        p.y = top - 2.5;
                        for l in ls {
                            p.text(M + c as f32 * col, l, i == 0, size);
                            p.y -= size * 1.3;
                        }
                    }
                    p.y = top - h;
                    p.line(M, W - M, p.y + 1.5);
                }
                p.y -= 8.0;
            }
        }
    }
    p.done.push(p.page);
    // 1 catalog, 2 pages, 3-4 fonts, then a page and its contents for each page.
    let kids: Vec<String> = (0..p.done.len()).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    let font = |name: &str| format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>").into_bytes();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), p.done.len()).into_bytes(),
        font("Helvetica"),
        font("Helvetica-Bold"),
    ];
    for (i, content) in p.done.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>", 6 + 2 * i).into_bytes());
        objs.push([format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(), content, b"\nendstream"].concat());
    }
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut at = vec![];
    for (i, o) in objs.iter().enumerate() {
        at.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(o);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for a in at {
        out.extend(format!("{a:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "# Q3 review\n\nRevenue **up** 12%, see [the deck](https://x.io).\nCosts flat.\n\n## Actions\n- Hire\n1. Ship it\n\n| Item | Cost |\n|---|---|\n| Rent | 9100 |\n| Café & co | 0400 |\n\n---\n";

    #[test]
    fn markdown_blocks() {
        assert_eq!(
            blocks("n.md", NOTES),
            vec![
                Block::Heading(1, "Q3 review".into()),
                Block::Para("Revenue up 12%, see the deck (https://x.io). Costs flat.".into()),
                Block::Heading(2, "Actions".into()),
                Block::Item("•".into(), "Hire".into()),
                Block::Item("1.".into(), "Ship it".into()),
                Block::Table(vec![vec!["Item".into(), "Cost".into()], vec!["Rent".into(), "9100".into()], vec!["Café & co".into(), "0400".into()]]),
                Block::Rule,
            ]
        );
        assert_eq!(csv("a,\"b, \"\"c\"\"\"\r\n1,\"two\nlines\"\n\n", ','), vec![vec!["a", "b, \"c\""], vec!["1", "two\nlines"]]);
    }

    #[test]
    fn files_open_as_what_they_say() {
        // Word and Excel files read back with the same reader people's own files go through.
        let (name, d) = make("docx", "notes/q3.md", NOTES).unwrap();
        assert_eq!(name, "q3.docx");
        let t = crate::document_text(&name, "", &d).unwrap().unwrap();
        assert!(t.contains("Q3 review") && t.contains("•\tHire") && t.contains("Café & co"), "{t}");
        let (name, x) = make("xlsx", "costs.csv", "Item,Cost,Note\nRent,9100,=HYPERLINK(\"x\")\nZip,0400,+27 82\n").unwrap();
        assert_eq!(name, "costs.xlsx");
        assert_eq!(crate::document_text(&name, "", &x).unwrap().unwrap(), "Sheet: Sheet1\nItem\tCost\tNote\nRent\t9100\t=HYPERLINK(\"x\")\nZip\t0400\t+27 82\n\n");
        let sheet = String::from_utf8_lossy(&x);
        assert!(!sheet.contains("<f>"), "no formulas");
        assert!(make("xlsx", "n.md", "# no table\n").is_err());
        assert_eq!(column(0) + &column(25) + &column(26) + &column(701), "AZAAZZ");
        // A PDF the reader opens, long enough for two pages, with every character in it either kept or a ?.
        let long = format!("{NOTES}\n{}", "A paragraph (with brackets) \\ that wraps around and around. ".repeat(200));
        let (name, p) = make("pdf", "q3.md", &long).unwrap();
        assert_eq!(name, "q3.pdf");
        assert!(!String::from_utf8_lossy(&p).contains("/Count 1 "), "more than one page");
        let t = crate::document_text(&name, "", &p).unwrap().unwrap();
        assert!(t.contains("Q3 review") && t.contains("Caf") && t.contains("(with brackets)"), "{t}");
        assert_eq!(wrap("日本 x", false, 11.0, 100.0), vec![b"?? x".to_vec()]);
        assert_eq!(wrap("abcdefghij", false, 11.0, 20.0).concat(), b"abcdefghij", "a long word is split, nothing lost");
    }
}

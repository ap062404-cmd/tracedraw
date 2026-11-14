//! PDF text back into paragraph text (#2323).
//!
//! A PDF keeps no paragraphs: each line (often each piece of a line in
//! one font) is drawn on its own. Read as it is, a page of body text
//! becomes a pile of artistic texts, one per line, and editing a word
//! means retyping the lines after it by hand. Here the lines of a page
//! that stand one under the other in the same font, size and spacing,
//! lined up on the left (or centred, or on the right), become one
//! paragraph text frame, and a frame whose last line is full continues
//! in the next column or at the top of the next page: the two frames
//! are marked to be linked ([`FLOW_KEY`]), so text typed in the first
//! flows on into the second.
//!
//! Every frame is laid out with the text engine, when one is registered,
//! before it replaces its lines: unless the engine breaks the text where
//! the file broke it, with the baselines where the file has them, the
//! lines stay as they came (first with each line ended by a line break,
//! then as artistic texts). The look of the page comes first.

use std::collections::HashMap;
use tracedraw_core::{
    document::{text_outline, LeadingUnit, ParagraphStyle, Shape, ShapeKind, TextSpan},
    geometry::{Affine, Point, Size},
    Fill, ShapeId, TextAlign,
};

use crate::pdf_import::ImportedPage;

const PT_MM: f64 = 25.4 / 72.0;

/// The object data key of a frame to link: `"<chain>:<place>"`, the
/// frames of one chain numbered in their order from 0. The program
/// links them (`text.next` and `text.prev`) once the frames have the ids
/// they keep in the drawing, and takes the key away.
pub const FLOW_KEY: &str = "pdf.flow";

/// How far apart two lines' left edges (or centres, or right edges) may be
/// and still be lined up, mm.
const LINE_UP_MM: f64 = 0.35;

/// One line of text as the file placed it: one or more artistic texts on
/// one baseline, next to one another.
#[derive(Debug, Clone)]
struct Line {
    /// Indexes in the page's objects of the texts it is made of.
    pieces: Vec<usize>,
    /// The texts' turn (and slant) without their place.
    linear: [f64; 4],
    /// Where the baseline starts and how high it is, measured along and
    /// across the text's own direction (`linear` undone), mm.
    u: f64,
    w: f64,
    /// Where it ends along the baseline.
    right: f64,
    spans: Vec<TextSpan>,
    fill: Fill,
    opacity: f64,
}

impl Line {
    fn width(&self) -> f64 {
        self.right - self.u
    }

    fn centre(&self) -> f64 {
        (self.u + self.right) / 2.0
    }

    /// The span with the most characters: the line's font.
    fn main(&self) -> &TextSpan {
        self.spans
            .iter()
            .max_by_key(|s| s.text.chars().count())
            .unwrap_or(&self.spans[0])
    }

    fn em(&self) -> f64 {
        self.main().size_pt * PT_MM
    }

    fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// How wide the first word may be, with the space before it: on the
    /// wide side (letters of 0.65 em), so that a line followed by a word
    /// that did not fit after it counts as full; a paragraph's last line
    /// leaves far more room.
    fn first_word_width(&self) -> f64 {
        let text = self.text();
        let n = text
            .trim_start()
            .split(' ')
            .next()
            .map_or(0, |w| w.chars().count());
        (n as f64 * 0.65 + 0.35) * self.em()
    }
}

/// Same turn, slant and font as `other`.
fn same_kind(a: &Line, b: &Line) -> bool {
    let (ma, mb) = (a.main(), b.main());
    a.linear
        .iter()
        .zip(&b.linear)
        .all(|(x, y)| (x - y).abs() < 1e-4)
        && ma.font_family == mb.font_family
        && (ma.size_pt - mb.size_pt).abs() < 0.05
        && ma.bold == mb.bold
        && ma.italic == mb.italic
        && (a.opacity - b.opacity).abs() < 1e-6
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lined {
    Left,
    Centre,
    Right,
}

/// Lines that make one paragraph frame.
#[derive(Debug, Clone)]
struct Block {
    lines: Vec<Line>,
    lined: Lined,
    /// The first line starts this much further right than the others.
    indent: f64,
    /// Baseline to baseline, mm (0 for one line).
    leading: f64,
    /// The frame is at least this wide: a line alone that a frame flows
    /// on to is as wide as that frame.
    at_least: f64,
}

impl Block {
    fn first(&self) -> &Line {
        &self.lines[0]
    }

    fn last(&self) -> &Line {
        &self.lines[self.lines.len() - 1]
    }

    /// The left edge of the lines other than an indented first one.
    fn left(&self) -> f64 {
        match self.lines.get(1..) {
            Some(rest) if !rest.is_empty() => rest.iter().map(|l| l.u).fold(f64::MAX, f64::min),
            _ => self.first().u,
        }
    }

    fn right(&self) -> f64 {
        self.lines.iter().map(|l| l.right).fold(f64::MIN, f64::max)
    }

    /// The frame's width the lines need.
    fn width(&self) -> f64 {
        let own = match self.lined {
            Lined::Left => self.right() - self.left(),
            _ => self.lines.iter().map(Line::width).fold(0.0, f64::max),
        };
        own.max(self.at_least)
    }

    /// The room left at the end of line `i` in a frame of `width`.
    fn room_after(&self, i: usize, width: f64) -> f64 {
        let l = &self.lines[i];
        match self.lined {
            Lined::Left => self.left() + width - l.right,
            _ => width - l.width(),
        }
    }

    /// Line `i` ends its paragraph: the next line's first word would
    /// have fitted after it.
    fn ends_paragraph(&self, i: usize, width: f64) -> bool {
        match self.lines.get(i + 1) {
            Some(next) => self.room_after(i, width) > next.first_word_width(),
            None => true,
        }
    }

    /// `line` goes on this block, the next one down.
    fn takes(&mut self, line: &Line) -> bool {
        let last = self.last();
        if !same_kind(self.first(), line) {
            return false;
        }
        let em = self.first().em();
        let drop = last.w - line.w;
        if drop < 0.8 * em || drop > 2.5 * em {
            return false;
        }
        if self.lines.len() > 1 && (drop - self.leading).abs() > (0.02 * em).max(0.1) {
            return false;
        }
        let first = self.first().clone();
        if self.lines.len() == 1 {
            let lined = if (line.u - first.u).abs() < LINE_UP_MM {
                Some((Lined::Left, 0.0))
            } else if first.u - line.u > LINE_UP_MM && first.u - line.u < 4.0 * em {
                // An indented first line.
                Some((Lined::Left, first.u - line.u))
            } else if (line.centre() - first.centre()).abs() < LINE_UP_MM {
                Some((Lined::Centre, 0.0))
            } else if (line.right - first.right).abs() < LINE_UP_MM {
                Some((Lined::Right, 0.0))
            } else {
                None
            };
            let Some((lined, indent)) = lined else {
                return false;
            };
            self.lined = lined;
            self.indent = indent;
            self.leading = drop;
            self.lines.push(line.clone());
            return true;
        }
        let ok = match self.lined {
            Lined::Left => {
                let left = self.left();
                (line.u - left).abs() < LINE_UP_MM
                    // The indented first line of the next paragraph, after
                    // a line that ended one.
                    || (self.indent > 0.0
                        && (line.u - (left + self.indent)).abs() < LINE_UP_MM
                        && self.right() - last.right > line.first_word_width())
            }
            Lined::Centre => (line.centre() - first.centre()).abs() < LINE_UP_MM,
            Lined::Right => (line.right - first.right).abs() < LINE_UP_MM,
        };
        if ok {
            self.lines.push(line.clone());
        }
        ok
    }
}

/// Make paragraph frames of the pages' lines of text, linking those that
/// continue one another. `widths` gives each artistic text's length
/// along its baseline, mm, as the file draws it.
pub fn make_paragraphs(pages: &mut [ImportedPage], widths: &HashMap<ShapeId, f64>) {
    // The blocks of every page, in order.
    let mut blocks: Vec<Vec<Block>> = pages
        .iter()
        .map(|p| blocks_of(&lines_of(&p.shapes, widths)))
        .collect();
    let links = links_between(&blocks);
    // A line alone that a frame flows on to takes that frame's width and
    // spacing.
    for &(a, b) in &links {
        if blocks[b.0][b.1].lines.len() == 1 {
            let (width, leading) = {
                let from = &blocks[a.0][a.1];
                (from.width(), from.leading)
            };
            let to = &mut blocks[b.0][b.1];
            to.at_least = width;
            to.leading = leading;
        }
    }
    let mut chain_of: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    let mut chains = 0;
    for &(a, b) in &links {
        let (chain, place) = match chain_of.get(&a) {
            Some(&c) => c,
            None => {
                chains += 1;
                chain_of.insert(a, (chains - 1, 0));
                (chains - 1, 0)
            }
        };
        chain_of.insert(b, (chain, place + 1));
    }
    for (pi, page) in pages.iter_mut().enumerate() {
        let mut gone = vec![false; page.shapes.len()];
        for (bi, block) in blocks[pi].iter().enumerate() {
            let chained = chain_of.get(&(pi, bi)).copied();
            // A line alone stays artistic text, unless it is where a
            // frame flows on to.
            if block.lines.len() < 2 && chained.is_none() {
                continue;
            }
            let continues = links.iter().any(|(a, _)| *a == (pi, bi));
            let at = block.first().pieces[0];
            let Some(frame) = frame_of(block, &page.shapes[at], continues, chained) else {
                continue;
            };
            for line in &block.lines {
                for &i in &line.pieces {
                    gone[i] = true;
                }
            }
            gone[at] = false;
            page.shapes[at] = frame;
        }
        let mut i = 0;
        page.shapes.retain(|_| {
            i += 1;
            !gone[i - 1]
        });
    }
}

/// The page's artistic texts on one baseline joined into lines, in the
/// order they come.
fn lines_of(shapes: &[Shape], widths: &HashMap<ShapeId, f64>) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let ShapeKind::Text {
            spans,
            frame: None,
            on_path: None,
            ..
        } = &s.kind
        else {
            continue;
        };
        let Some(&width) = widths.get(&s.id) else {
            continue;
        };
        if spans.is_empty() || spans.iter().any(|sp| sp.text.contains('\n')) {
            continue;
        }
        let c = s.transform.as_coeffs();
        let det = c[0] * c[3] - c[1] * c[2];
        // Mirrored or squashed text is left alone.
        if det < 0.5 {
            continue;
        }
        let linear = [c[0], c[1], c[2], c[3]];
        // The place in the text's own directions.
        let u = (c[3] * c[4] - c[2] * c[5]) / det;
        let w = (-c[1] * c[4] + c[0] * c[5]) / det;
        let fill = s.fill.clone();
        let mut spans = spans.clone();
        let piece = Line {
            pieces: vec![i],
            linear,
            u,
            w,
            right: u + width,
            spans: spans.clone(),
            fill: fill.clone(),
            opacity: s.opacity,
        };
        // The next piece of the line before: right after it, on its
        // baseline.
        if let Some(prev) = lines.last_mut() {
            let em = prev.em().max(piece.em());
            let follows = i > 0
                && prev.pieces.last() == Some(&(i - 1))
                && prev
                    .linear
                    .iter()
                    .zip(&linear)
                    .all(|(x, y)| (x - y).abs() < 1e-4)
                && (prev.w - w).abs() < 0.05 * em
                && u > prev.right - 0.2 * em
                && u < prev.right + 3.0 * em
                && (prev.opacity - s.opacity).abs() < 1e-6;
            if follows {
                let gap = u - prev.right;
                let ends_space = prev.text().ends_with(' ');
                let starts_space = spans[0].text.starts_with(' ');
                if gap > 0.18 * em && !ends_space && !starts_space {
                    spans[0].text.insert(0, ' ');
                }
                for sp in &mut spans {
                    if sp.fill.is_none() && fill != prev.fill {
                        sp.fill = Some(fill.clone());
                    }
                }
                prev.spans.extend(spans);
                prev.pieces.push(i);
                prev.right = prev.right.max(u + width);
                continue;
            }
        }
        lines.push(piece);
    }
    lines
}

/// Lines that follow one another in the page's objects, one under the
/// other, gathered into blocks.
fn blocks_of(lines: &[Line]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for line in lines {
        if let Some(b) = blocks.last_mut() {
            let next_object = b.last().pieces.last().map(|i| i + 1) == line.pieces.first().copied();
            if next_object && b.takes(line) {
                continue;
            }
        }
        blocks.push(Block {
            lines: vec![line.clone()],
            lined: Lined::Left,
            indent: 0.0,
            leading: 0.0,
            at_least: 0.0,
        });
    }
    blocks
}

/// Which blocks continue in which: (page, block) pairs, the first
/// flowing into the second. A frame of body text whose last line is full
/// goes on in the next block when that one is in the same font, about
/// as wide and as spaced, and starts the next column of the page or the
/// next page.
fn links_between(blocks: &[Vec<Block>]) -> Vec<((usize, usize), (usize, usize))> {
    let mut out = Vec::new();
    for (pi, page) in blocks.iter().enumerate() {
        for (bi, a) in page.iter().enumerate() {
            if a.lines.len() < 2 || a.lined != Lined::Left {
                continue;
            }
            let (b_at, b) = if bi + 1 < page.len() {
                ((pi, bi + 1), &page[bi + 1])
            } else {
                match blocks.get(pi + 1).and_then(|p| p.first()) {
                    Some(b) => ((pi + 1, 0), b),
                    None => continue,
                }
            };
            if !same_kind(a.first(), b.first()) || b.lined != Lined::Left {
                continue;
            }
            let width = a.width();
            // The text goes on: the first word of the next block would
            // not have fitted at the end of this one.
            let room = a.left() + width - a.last().right;
            if room > b.first().first_word_width() {
                continue;
            }
            // As wide (its ragged lines may all end a little short) and
            // as spaced.
            if b.width() > width + 1.0
                || (b.lines.len() > 1
                    && (b.width() < width * 0.8 || (b.leading - a.leading).abs() > 0.1))
            {
                continue;
            }
            if b_at.0 == pi {
                // The next column: higher up than this block's end and to
                // its right.
                if b.first().w <= a.last().w || b.left() < a.left() + width * 0.5 {
                    continue;
                }
            }
            out.push(((pi, bi), b_at));
        }
    }
    out
}

/// The paragraph frame of `block`, in place of the line `like` (the
/// block's first text, whose look it keeps): `None` when the text engine
/// would not lay its text out as the file has it. `continues`: the frame
/// flows on into another, so its text ends with the space between the two.
fn frame_of(
    block: &Block,
    like: &Shape,
    continues: bool,
    chained: Option<(usize, usize)>,
) -> Option<Shape> {
    let first = block.first();
    let em = first.em();
    let leading = if block.leading > 0.0 {
        block.leading
    } else {
        1.2 * em
    };
    let width = block.width();
    let align = match block.lined {
        Lined::Left => {
            // Justified: every line that does not end a paragraph reaches
            // the right edge.
            let full: Vec<f64> = (0..block.lines.len() - 1)
                .filter(|&i| !block.ends_paragraph(i, width))
                .map(|i| block.lines[i].right)
                .collect();
            let spread = full.iter().fold(f64::MIN, |a, &b| a.max(b))
                - full.iter().fold(f64::MAX, |a, &b| a.min(b));
            if full.len() >= 2 && spread < 0.3 {
                TextAlign::Justify
            } else {
                TextAlign::Left
            }
        }
        Lined::Centre => TextAlign::Center,
        Lined::Right => TextAlign::Right,
    };
    // Tries: the lines run on with the file's breaks found again by the
    // engine, the frame a little wider each time; then each line ended by
    // a line break.
    let tries: [(f64, bool); 4] = [
        (0.3, false),
        (width * 0.015, false),
        (width * 0.04, false),
        (width * 0.1, true),
    ];
    for (more, broken) in tries {
        let frame_width = width + more;
        let indent = if broken { 0.0 } else { block.indent };
        let (spans, starts) = block_text(block, width, broken, continues);
        let para = ParagraphStyle {
            leading_pct: leading / PT_MM,
            leading_unit: LeadingUnit::Points,
            first_line_indent: indent,
            ..ParagraphStyle::default()
        };
        let req = text_outline::TextRequest {
            spans: &spans,
            frame: Some(Size::new(
                frame_width,
                leading * (block.lines.len() as f64 + 4.0),
            )),
            align,
            para: &para,
            on_path: None,
        };
        // Where the engine puts the first baseline below the frame's top,
        // and how deep the last line goes.
        let (first_base, last_depth) = match text_outline::lines(&req) {
            Some(laid) => {
                let fits = laid.len() == block.lines.len()
                    && laid.iter().zip(&starts).all(|(l, s)| l.start_char == *s)
                    && laid.iter().zip(&block.lines).all(|(l, line)| {
                        let want = line.w - first.w;
                        (l.baseline - laid[0].baseline - want).abs() < 0.2
                    });
                if !fits {
                    continue;
                }
                (laid[0].baseline, laid[laid.len() - 1].descent)
            }
            None => (-0.9 * em, 0.3 * em),
        };
        let left = match block.lined {
            Lined::Left if broken => block.lines.iter().map(|l| l.u).fold(f64::MAX, f64::min),
            Lined::Left => block.left(),
            Lined::Centre => first.centre() - frame_width / 2.0,
            Lined::Right => first.right - frame_width,
        };
        let top = first.w - first_base;
        let bottom = block.last().w - last_depth - (0.15 * em).min(leading * 0.3);
        let l = first.linear;
        let mut shape = like.clone();
        shape.kind = ShapeKind::Text {
            spans,
            origin: Point::ZERO,
            frame: Some(Size::new(frame_width, top - bottom)),
            align,
            para,
            on_path: None,
        };
        shape.transform =
            Affine::new([l[0], l[1], l[2], l[3], 0.0, 0.0]) * Affine::translate((left, bottom));
        shape.fill = first.fill.clone();
        shape.opacity = first.opacity;
        if let Some((chain, place)) = chained {
            shape.data.retain(|(k, _)| k != FLOW_KEY);
            shape
                .data
                .push((FLOW_KEY.to_string(), format!("{chain}:{place}")));
        }
        return Some(shape);
    }
    None
}

/// The block's text, its lines run on (or each ended by a line break,
/// `broken`), with the character each line starts at. `continues`: a
/// space (or nothing after a hyphen) at the end, for the text of the
/// next frame to follow.
fn block_text(
    block: &Block,
    width: f64,
    broken: bool,
    continues: bool,
) -> (Vec<TextSpan>, Vec<usize>) {
    let first = block.first();
    let mut spans: Vec<TextSpan> = Vec::new();
    let mut starts = Vec::new();
    let mut chars = 0;
    let n = block.lines.len();
    for (i, line) in block.lines.iter().enumerate() {
        starts.push(chars);
        let mut own = line.spans.clone();
        if i == 0 {
            if let Some(s) = own.first_mut() {
                s.text = s.text.trim_start().to_string();
            }
        }
        // The line's own colour where it is not the frame's.
        if line.fill != first.fill {
            for s in &mut own {
                if s.fill.is_none() {
                    s.fill = Some(line.fill.clone());
                }
            }
        }
        let text = line.text();
        let trimmed = text.trim_end();
        // The spaces at the end of the line go: the break stands for them.
        let mut cut = text.len() - trimmed.len();
        while cut > 0 {
            let Some(last) = own.last_mut() else { break };
            let keep = last.text.len().saturating_sub(cut);
            cut -= last.text.len() - keep;
            last.text.truncate(keep);
            if last.text.is_empty() {
                own.pop();
            }
        }
        let hyphen = trimmed.ends_with('-') || trimmed.ends_with('\u{ad}');
        let sep = if i + 1 == n {
            if continues && !hyphen {
                " "
            } else {
                ""
            }
        } else if broken || block.ends_paragraph(i, width) {
            "\n"
        } else if hyphen {
            ""
        } else {
            " "
        };
        if let Some(last) = own.last_mut() {
            last.text.push_str(sep);
        }
        chars += own.iter().map(|s| s.text.chars().count()).sum::<usize>();
        spans.extend(own);
    }
    // Spans next to one another that differ only in text become one.
    let mut joined: Vec<TextSpan> = Vec::with_capacity(spans.len());
    for s in spans {
        if let Some(prev) = joined.last_mut() {
            let mut a = prev.clone();
            a.text.clear();
            let mut b = s.clone();
            b.text.clear();
            if a == b {
                prev.text.push_str(&s.text);
                continue;
            }
        }
        joined.push(s);
    }
    (joined, starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Affine;

    fn line(id: u64, text: &str, x: f64, y: f64, width: f64) -> (Shape, f64) {
        let mut s = Shape::new(
            ShapeId(id),
            ShapeKind::Text {
                spans: vec![TextSpan::new(text, "Arial", 10.0)],
                origin: Point::ZERO,
                frame: None,
                align: TextAlign::Left,
                para: ParagraphStyle::default(),
                on_path: None,
            },
        );
        s.transform = Affine::translate((x, y));
        (s, width)
    }

    fn page(lines: &[(&str, f64, f64, f64)]) -> (ImportedPage, HashMap<ShapeId, f64>) {
        let mut widths = HashMap::new();
        let mut shapes = Vec::new();
        for (i, (t, x, y, w)) in lines.iter().enumerate() {
            let (s, w) = line(i as u64 + 1, t, *x, *y, *w);
            widths.insert(s.id, w);
            shapes.push(s);
        }
        (
            ImportedPage {
                size: Size::new(210.0, 297.0),
                shapes,
            },
            widths,
        )
    }

    fn texts(p: &ImportedPage) -> Vec<String> {
        p.shapes
            .iter()
            .filter_map(|s| match &s.kind {
                ShapeKind::Text { spans, .. } => {
                    Some(spans.iter().map(|s| s.text.as_str()).collect())
                }
                _ => None,
            })
            .collect()
    }

    /// Three lines one under the other, lined up on the left, become one
    /// frame: the full lines run on, the short last one ends it.
    #[test]
    fn lines_one_under_another_become_one_frame() {
        let (p, widths) = page(&[
            ("The quick brown fox jumps", 20.0, 250.0, 80.0),
            ("over the lazy dog and runs", 20.0, 245.0, 79.5),
            ("away.", 20.0, 240.0, 12.0),
        ]);
        let mut pages = vec![p];
        make_paragraphs(&mut pages, &widths);
        assert_eq!(pages[0].shapes.len(), 1);
        assert_eq!(
            texts(&pages[0]),
            ["The quick brown fox jumps over the lazy dog and runs away."]
        );
        let s = &pages[0].shapes[0];
        let ShapeKind::Text {
            frame: Some(f),
            para,
            ..
        } = &s.kind
        else {
            panic!("not a frame: {:?}", s.kind);
        };
        assert!((f.width - 80.3).abs() < 1e-6, "{f:?}");
        assert!((para.leading_pct * PT_MM - 5.0).abs() < 1e-9);
        assert_eq!(para.leading_unit, LeadingUnit::Points);
        // The frame starts at the lines' left edge.
        let origin = s.transform * Point::ZERO;
        assert!((origin.x - 20.0).abs() < 1e-6, "{origin:?}");
        assert!(origin.y < 240.0, "{origin:?}");
    }

    /// A short line in the middle ends its paragraph: a line break, not a
    /// space.
    #[test]
    fn a_short_line_ends_its_paragraph() {
        let (p, widths) = page(&[
            ("First paragraph goes on and", 20.0, 250.0, 80.0),
            ("ends here.", 20.0, 245.0, 20.0),
            ("Second paragraph starts", 20.0, 240.0, 70.0),
        ]);
        let mut pages = vec![p];
        make_paragraphs(&mut pages, &widths);
        assert_eq!(
            texts(&pages[0]),
            ["First paragraph goes on and ends here.\nSecond paragraph starts"]
        );
    }

    /// Lines in other fonts, far apart or out of line stay as they are.
    #[test]
    fn unrelated_lines_stay_artistic() {
        let (mut p, widths) = page(&[
            ("Title", 20.0, 280.0, 30.0),
            ("Body far below", 20.0, 200.0, 50.0),
            ("Off to the right", 90.0, 195.0, 50.0),
        ]);
        if let ShapeKind::Text { spans, .. } = &mut p.shapes[0].kind {
            spans[0].size_pt = 24.0;
        }
        let mut pages = vec![p];
        make_paragraphs(&mut pages, &widths);
        assert_eq!(pages[0].shapes.len(), 3);
        assert!(pages[0]
            .shapes
            .iter()
            .all(|s| matches!(s.kind, ShapeKind::Text { frame: None, .. })));
    }

    /// Body text whose last line on a page is full goes on at the top of
    /// the next page: both frames are marked as one chain, the first one's
    /// text ending in the space between them.
    #[test]
    fn text_running_over_the_page_end_is_chained() {
        let (p1, w1) = page(&[
            ("Lorem ipsum dolor sit amet", 20.0, 30.0, 80.0),
            ("consectetur adipiscing elit", 20.0, 25.0, 79.8),
        ]);
        let (mut p2, w2) = page(&[
            ("sed do eiusmod tempor", 20.0, 270.0, 79.9),
            ("incididunt.", 20.0, 265.0, 20.0),
        ]);
        for (i, s) in p2.shapes.iter_mut().enumerate() {
            s.id = ShapeId(100 + i as u64);
        }
        let mut widths = w1;
        for (k, v) in w2 {
            widths.insert(ShapeId(k.0 + 99), v);
        }
        let mut pages = vec![p1, p2];
        make_paragraphs(&mut pages, &widths);
        let flow = |s: &Shape| {
            s.data
                .iter()
                .find(|(k, _)| k == FLOW_KEY)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(flow(&pages[0].shapes[0]).as_deref(), Some("0:0"));
        assert_eq!(flow(&pages[1].shapes[0]).as_deref(), Some("0:1"));
        assert_eq!(
            texts(&pages[0]),
            ["Lorem ipsum dolor sit amet consectetur adipiscing elit "]
        );
        assert_eq!(texts(&pages[1]), ["sed do eiusmod tempor incididunt."]);
    }

    /// Pieces of one line in different fonts (a bold word) join the line.
    #[test]
    fn pieces_of_a_line_join_it() {
        let (mut p, widths) = page(&[
            ("A line with a", 20.0, 250.0, 30.0),
            ("bold", 51.5, 250.0, 10.0),
            ("word in it and more", 62.5, 250.0, 37.5),
            ("text on the next line.", 20.0, 245.0, 40.0),
        ]);
        if let ShapeKind::Text { spans, .. } = &mut p.shapes[1].kind {
            spans[0].bold = true;
        }
        let mut pages = vec![p];
        make_paragraphs(&mut pages, &widths);
        assert_eq!(pages[0].shapes.len(), 1);
        let ShapeKind::Text { spans, .. } = &pages[0].shapes[0].kind else {
            panic!()
        };
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            text,
            "A line with a bold word in it and more text on the next line."
        );
        assert!(spans.iter().any(|s| s.bold && s.text.trim() == "bold"));
    }
}

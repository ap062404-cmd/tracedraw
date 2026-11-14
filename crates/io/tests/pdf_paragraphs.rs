//! Paragraph text through a PDF and back (#2323): a text that flows from
//! a frame at the end of one page into a frame at the top of the next,
//! published to PDF, comes back as two paragraph frames marked as one
//! chain, each holding the words it held, laid out on the same lines.
//! A test binary of its own: it lays text out with the real text engine.

use tracedraw_core::{
    document::{text_outline::TextRequest, Layer, Page},
    Color, Document, Fill, ParagraphStyle, Point, Shape, ShapeKind, Size, TextAlign, TextSpan,
};
use tracedraw_io::pdf_import::{parse_with_options, ImportOptions};
use tracedraw_io::pdf_paragraphs::FLOW_KEY;

const FONT: &str = "DejaVu Sans";
const TEXT: &str = "TraceDraw reads the lines of a PDF back into paragraph text, so that \
a word typed in the middle of a page pushes the words after it along the lines and on \
into the next frame, as it does in the drawing the file was published from. The lines \
of body text stand one under the other in the same font, size and spacing, lined up on \
the left; a line that ends short ends its paragraph. Every frame is laid out again before \
it replaces its lines, and only when its lines break where the file broke them.";

fn fonts() -> &'static tracedraw_text::FontSystem {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(tracedraw_text::install);
    tracedraw_text::fonts()
}

fn frame(doc: &mut Document, text: &str, origin: Point, size: Size) -> Shape {
    let id = doc.ids_mut().shape();
    let mut s = Shape::new(
        id,
        ShapeKind::Text {
            spans: vec![TextSpan::new(text, FONT, 11.0)],
            origin,
            frame: Some(size),
            align: TextAlign::Left,
            para: ParagraphStyle::default(),
            on_path: None,
        },
    );
    s.fill = Fill::Solid(Color::BLACK);
    s.stroke = None;
    s
}

fn text_of(s: &Shape) -> String {
    match &s.kind {
        ShapeKind::Text { spans, .. } => spans.iter().map(|s| s.text.as_str()).collect(),
        _ => String::new(),
    }
}

fn flow_of(s: &Shape) -> Option<String> {
    s.data
        .iter()
        .find(|(k, _)| k == FLOW_KEY)
        .map(|(_, v)| v.clone())
}

#[test]
fn text_flowing_over_a_page_comes_back_as_linked_frames() {
    let fonts = fonts();
    let page = Size::new(150.0, 120.0);
    let mut doc = Document::new("t", page);
    // The first frame holds what fits in its four or so lines; the rest
    // is in the frame at the top of the second page.
    let size_a = Size::new(100.0, 22.0);
    let spans = [TextSpan::new(TEXT, FONT, 11.0)];
    let para = ParagraphStyle::default();
    let layout = fonts.layout(&TextRequest {
        spans: &spans,
        frame: Some(size_a),
        align: TextAlign::Left,
        para: &para,
        on_path: None,
    });
    let fitted = layout.fitted_chars;
    assert!(fitted > 0 && fitted < TEXT.chars().count(), "{fitted}");
    let head: String = TEXT.chars().take(fitted).collect();
    let tail: String = TEXT.chars().skip(fitted).collect();
    let a = frame(&mut doc, &head, Point::new(20.0, 10.0), size_a);
    let b = frame(
        &mut doc,
        &tail,
        Point::new(20.0, 40.0),
        Size::new(100.0, 70.0),
    );
    let layer = doc.pages[0].layers[0].id;
    doc.layer_mut(layer).expect("layer").shapes.push(a);
    let pid = doc.ids_mut().page();
    let lid = doc.ids_mut().layer();
    let mut l2 = Layer::new(lid, "Layer 1");
    l2.shapes.push(b);
    doc.pages.push(Page {
        id: pid,
        name: "Page 2".into(),
        size: page,
        layers: vec![l2],
        guides: Vec::new(),
        background: None,
    });
    let bytes = tracedraw_io::pdf::document_to_pdf(&doc);

    let options = ImportOptions {
        text_as_paragraphs: true,
        ..ImportOptions::default()
    };
    let mut ids = tracedraw_core::id::IdSource::default();
    let imp = parse_with_options(&bytes, &mut ids, "", options).expect("reads");
    assert_eq!(imp.pages.len(), 2);
    let texts = |p: usize| -> Vec<&Shape> {
        imp.pages[p]
            .shapes
            .iter()
            .filter(|s| matches!(s.kind, ShapeKind::Text { .. }))
            .collect()
    };
    let (p1, p2) = (texts(0), texts(1));
    assert_eq!(
        p1.len(),
        1,
        "{:?}",
        p1.iter().map(|s| text_of(s)).collect::<Vec<_>>()
    );
    assert_eq!(
        p2.len(),
        1,
        "{:?}",
        p2.iter().map(|s| text_of(s)).collect::<Vec<_>>()
    );
    let (fa, fb) = (p1[0], p2[0]);
    assert!(matches!(fa.kind, ShapeKind::Text { frame: Some(_), .. }));
    assert!(matches!(fb.kind, ShapeKind::Text { frame: Some(_), .. }));
    // The words each frame held, with the space between them at the end
    // of the first.
    assert_eq!(text_of(fa).trim_end(), head.trim_end());
    assert_eq!(text_of(fb), tail);
    assert!(text_of(fa).ends_with(' '));
    assert_eq!(flow_of(fa).as_deref(), Some("0:0"));
    assert_eq!(flow_of(fb).as_deref(), Some("0:1"));

    // Flowed as one chain, the first frame keeps the same words.
    let ShapeKind::Text {
        frame: Some(fsize),
        align,
        para,
        ..
    } = &fa.kind
    else {
        unreachable!()
    };
    let mut all = match &fa.kind {
        ShapeKind::Text { spans, .. } => spans.clone(),
        _ => unreachable!(),
    };
    if let ShapeKind::Text { spans, .. } = &fb.kind {
        all.extend(spans.iter().cloned());
    }
    let again = fonts.layout(&TextRequest {
        spans: &all,
        frame: Some(*fsize),
        align: *align,
        para,
        on_path: None,
    });
    assert_eq!(again.fitted_chars, text_of(fa).chars().count());

    // The frame's top left stands where the published one's stood.
    let corner = fa.transform * Point::new(0.0, fsize.height);
    assert!((corner.x - 20.0).abs() < 0.5, "{corner:?}");
    assert!((corner.y - 32.0).abs() < 0.5, "{corner:?}");
}

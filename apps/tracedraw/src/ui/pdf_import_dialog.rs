//! Import PDF: how a PDF's (or AI file's) text is read in, asked when one
//! is opened or imported (#386): as editable text in the fonts installed
//! here, or as curves drawn from the fonts the file embeds, which keeps
//! its look exactly. Each page can also come in as one picture (#1916);
//! opened that way, the pages can be chosen and set to a page size
//! (#2570).

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::ui::dialogs::{ok_cancel, window};
use crate::ui::tips::RichTip;
use egui::Context;
use std::path::PathBuf;
use tracedraw_core::geometry::Size;

/// The dialog's state.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfTextState {
    pub path: PathBuf,
    /// Import into the drawing (File > Import) rather than open.
    pub import: bool,
    pub curves: bool,
    /// Lines joined into paragraph text (#2323).
    pub paragraphs: bool,
    /// Each page as one picture at `dpi` (#1916; opened too, #2570).
    pub picture: bool,
    pub dpi: f64,
    /// Opened as pictures: the pages put on a page of this size, the
    /// paper's name or empty for a custom size; `None` keeps each page's
    /// own size (#2570).
    pub page_size: Option<(String, Size)>,
    /// Opened as pictures: every page, or those of `range` ("1-3, 5").
    pub all_pages: bool,
    pub range: String,
    /// Reduce very large pictures (#1835).
    pub reduce_pictures: bool,
    /// Keep the choice and stop asking (Options > File Formats).
    pub dont_ask: bool,
}

/// A PDF opened with each page as one picture (#2570): which pages, the
/// page size they are put on (`None`: each its own) and the resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfOpenPicture {
    pub all_pages: bool,
    pub range: String,
    pub page_size: Option<Size>,
    pub dpi: f64,
}

/// `size` turned to the orientation of a page of `page`: a landscape page
/// goes on the paper turned on its side.
pub fn oriented(size: Size, page: Size) -> Size {
    let (short, long) = (size.width.min(size.height), size.width.max(size.height));
    if page.width > page.height {
        Size::new(long, short)
    } else {
        Size::new(short, long)
    }
}

/// What moves a page of `from` onto a page of `to`: scaled to fit, never
/// stretched, and centred.
pub fn fit_on_page(from: Size, to: Size) -> tracedraw_core::geometry::Affine {
    use tracedraw_core::geometry::{Affine, Vec2};
    if from.width <= 0.0 || from.height <= 0.0 {
        return Affine::IDENTITY;
    }
    let k = (to.width / from.width).min(to.height / from.height);
    Affine::translate(Vec2::new(
        (to.width - from.width * k) / 2.0,
        (to.height - from.height * k) / 2.0,
    )) * Affine::scale(k)
}

/// The pages of a PDF opened as pictures (#2570): those chosen (all
/// when none of the typed ones is in the file), each put on the page size
/// asked for, scaled to fit and centred, over the white of its paper so
/// that its picture covers the whole page.
pub fn pages_for_pictures(
    pages: &mut Vec<tracedraw_io::pdf_import::ImportedPage>,
    pic: &PdfOpenPicture,
) {
    use tracedraw_core::{Fill, Shape, ShapeKind};
    let keep = crate::ui::pdf_pages::chosen_pages(pic.all_pages, &pic.range, pages.len());
    if !keep.is_empty() {
        let mut i = 0;
        pages.retain(|_| {
            i += 1;
            keep.contains(&(i - 1))
        });
    }
    let mut ids = tracedraw_core::id::IdSource::default();
    for p in pages.iter_mut() {
        if let Some(size) = pic.page_size {
            let to = oriented(size, p.size);
            let t = fit_on_page(p.size, to);
            for s in &mut p.shapes {
                s.transform_by(t);
            }
            p.size = to;
        }
        let mut paper = Shape::new(
            ids.shape(),
            ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(0.0, 0.0, p.size.width, p.size.height),
                radius: 0.0,
                corners: None,
            },
        );
        paper.fill = Fill::Solid(tracedraw_core::Color::WHITE);
        paper.stroke = None;
        p.shapes.insert(0, paper);
    }
}

impl App {
    /// The pages of the PDF just opened as pictures become one picture
    /// each at `dpi`, and the drawing opens with them as it is, with
    /// nothing to undo (#2570).
    pub fn opened_pages_to_pictures(&mut self, dpi: f64) {
        let options = crate::bitmap_modes::ConvertOptions {
            dpi,
            ..Default::default()
        };
        for i in 0..self.doc().pages.len() {
            self.goto_page(i);
            let ids: Vec<tracedraw_core::ShapeId> = self.doc().pages[i]
                .layers
                .iter()
                .flat_map(|l| &l.shapes)
                .map(|s| s.id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            self.selection = ids;
            self.convert_to_bitmap_with(&options);
        }
        self.goto_page(0);
        self.selection.clear();
        let doc = self.doc().clone();
        self.engine.replace(doc);
    }

    /// Before a PDF is read: ask how to read its text, when the
    /// preferences say so and it has not been asked for this file yet.
    /// True when the dialog was opened (the caller stops there).
    pub fn ask_pdf_text(&mut self, path: &std::path::Path, import: bool) -> bool {
        if !self.settings.formats.pdf_ask_text || std::mem::take(&mut self.pdf_text_chosen) {
            return false;
        }
        self.dialog = crate::ui::dialogs::Dialog::PdfText(PdfTextState {
            path: path.to_path_buf(),
            import,
            curves: self.settings.formats.pdf_text_as_curves,
            paragraphs: self.settings.formats.pdf_text_paragraphs,
            picture: self.settings.formats.pdf_as_picture,
            dpi: self.settings.formats.pdf_picture_dpi,
            page_size: None,
            all_pages: true,
            range: "1".into(),
            reduce_pictures: self.settings.formats.pdf_reduce_pictures,
            dont_ask: false,
        });
        true
    }

    /// The import options the preferences give. Pictures come in at
    /// their own size up to the memory guard, unless Reduce very large
    /// pictures is on; the browser and Android always reduce them, having
    /// less memory to give (#1835).
    pub fn pdf_options(&self) -> tracedraw_io::pdf_import::ImportOptions {
        use tracedraw_io::pdf_import::{FULL_PICTURE_PIXELS, REDUCED_PICTURE_PIXELS};
        let reduce = self.settings.formats.pdf_reduce_pictures || always_reduce_pictures();
        tracedraw_io::pdf_import::ImportOptions {
            // A page made a picture keeps the file's look: its text is
            // drawn from the fonts the file embeds (#1916).
            text_as_curves: self.settings.formats.pdf_text_as_curves
                || self.settings.formats.pdf_as_picture,
            max_picture_pixels: if reduce {
                REDUCED_PICTURE_PIXELS
            } else {
                FULL_PICTURE_PIXELS
            },
            text_as_paragraphs: self.settings.formats.pdf_text_paragraphs,
        }
    }
}

/// The browser and Android reduce very large pictures whatever the
/// choice (#1835).
pub fn always_reduce_pictures() -> bool {
    crate::files::WEB || crate::files::ANDROID
}

/// The Reduce very large pictures check box, ticked and greyed where
/// pictures are always reduced.
pub fn reduce_pictures_checkbox(ui: &mut egui::Ui, reduce: &mut bool) {
    if always_reduce_pictures() {
        let mut on = true;
        ui.add_enabled(
            false,
            egui::Checkbox::new(&mut on, tr("pdf_import.reduce_pictures")),
        )
        .rich_tip(crate::ui::tips::pair(
            "pdf_import.reduce_pictures",
            "pdf_import.reduce_pictures_always_tip",
        ));
    } else {
        ui.checkbox(reduce, tr("pdf_import.reduce_pictures"))
            .rich_tip(crate::ui::tips::pair(
                "pdf_import.reduce_pictures",
                "pdf_import.reduce_pictures_tip",
            ));
    }
}

/// The Join lines into paragraph text check box (#2323), for text read
/// as text only (`text`).
pub fn paragraphs_checkbox(ui: &mut egui::Ui, paragraphs: &mut bool, text: bool) {
    ui.add_enabled(
        text,
        egui::Checkbox::new(paragraphs, tr("pdf_import.paragraphs")),
    )
    .rich_tip(crate::ui::tips::pair(
        "pdf_import.paragraphs",
        "pdf_import.paragraphs_tip",
    ));
}

/// The Text, Curves and Picture choices of the dialog and of Options >
/// File Formats; Picture (and its resolution) where `picture` is given.
pub fn read_as_choices(
    ui: &mut egui::Ui,
    curves: &mut bool,
    picture: Option<(&mut bool, &mut f64)>,
) {
    // 0 text, 1 curves, 2 picture.
    let before = match &picture {
        Some((p, _)) if **p => 2,
        _ => u8::from(*curves),
    };
    let mut mode = before;
    ui.radio_value(&mut mode, 0, tr("pdf_import.text"))
        .rich_tip(crate::ui::tips::pair(
            "pdf_import.text",
            "pdf_import.text_tip",
        ));
    ui.radio_value(&mut mode, 1, tr("pdf_import.curves"))
        .rich_tip(crate::ui::tips::pair(
            "pdf_import.curves",
            "pdf_import.curves_tip",
        ));
    let Some((picture, dpi)) = picture else {
        *curves = mode == 1;
        return;
    };
    ui.radio_value(&mut mode, 2, tr("pdf_import.picture"))
        .rich_tip(crate::ui::tips::pair(
            "pdf_import.picture",
            "pdf_import.picture_tip",
        ));
    if mode != before {
        // The text choice stays as it was under a picture.
        if mode < 2 {
            *curves = mode == 1;
        }
        *picture = mode == 2;
    }
    if *picture {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label(tr("dialog.resolution"));
            ui.add(
                crate::ui::field::NumField::new(dpi)
                    .range(36.0..=2400.0)
                    .suffix(" dpi"),
            )
            .rich_tip(crate::ui::tips::pair(
                "dialog.resolution",
                "dialog.convert_dpi_desc",
            ));
        });
    }
}

/// Text whose font the file does not embed, left as text by the import:
/// as curves too, in the font that stands in for it here.
pub fn remaining_text_to_curves(pages: &mut [tracedraw_io::pdf_import::ImportedPage]) {
    let mut ids = tracedraw_core::id::IdSource::default();
    let mut next = || ids.shape();
    for p in pages {
        for s in &mut p.shapes {
            to_curves(s, &mut next);
        }
    }
}

fn to_curves(s: &mut tracedraw_core::Shape, next: &mut dyn FnMut() -> tracedraw_core::ShapeId) {
    use tracedraw_core::ShapeKind;
    match &mut s.kind {
        ShapeKind::Text { .. } => {
            if let Some(kind) = s.curves_kind(&[], next) {
                s.kind = kind;
            }
        }
        ShapeKind::Group { children } => {
            for c in children {
                to_curves(c, next);
            }
        }
        ShapeKind::ClipFrame { contents, .. } => {
            for c in contents {
                to_curves(c, next);
            }
        }
        _ => {}
    }
}

pub fn dialog(app: &mut App, ctx: &Context, st: &mut PdfTextState, close: &mut bool) {
    window(ctx, tr("pdf_import.title")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        let name = st
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        ui.label(trf("pdf_import.prompt", &[("name", &name)]));
        ui.add_space(6.0);
        ui.label(tr("pdf_import.text_as"));
        // Opened as pictures too, with the pages and their size (#2570).
        read_as_choices(ui, &mut st.curves, Some((&mut st.picture, &mut st.dpi)));
        if st.picture && !st.import {
            open_picture_choices(app, ui, st);
        }
        let text = !st.curves && !st.picture;
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            paragraphs_checkbox(ui, &mut st.paragraphs, text);
        });
        ui.add_space(4.0);
        reduce_pictures_checkbox(ui, &mut st.reduce_pictures);
        ui.add_space(4.0);
        ui.checkbox(&mut st.dont_ask, tr("pdf_import.dont_ask"));
        if ok_cancel(ui, close) {
            app.settings.formats.pdf_text_as_curves = st.curves;
            app.settings.formats.pdf_text_paragraphs = st.paragraphs;
            app.settings.formats.pdf_reduce_pictures = st.reduce_pictures;
            app.settings.formats.pdf_picture_dpi = st.dpi;
            if st.import {
                app.settings.formats.pdf_as_picture = st.picture;
            } else {
                // Opened as pictures this once; the import keeps its own
                // choice.
                app.pdf_open_picture = st.picture.then(|| PdfOpenPicture {
                    all_pages: st.all_pages,
                    range: st.range.clone(),
                    page_size: st.page_size.as_ref().map(|(_, s)| *s),
                    dpi: st.dpi,
                });
            }
            if st.dont_ask {
                app.settings.formats.pdf_ask_text = false;
            }
            app.pdf_text_chosen = true;
            let path = st.path.clone();
            if st.import {
                app.import_pdf(&path);
            } else {
                app.open_path(path);
            }
        }
    });
}

/// The page size and the pages of a PDF opened as pictures (#2570).
fn open_picture_choices(app: &App, ui: &mut egui::Ui, st: &mut PdfTextState) {
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.label(tr("pdf_import.page_size"));
        let shown = match &st.page_size {
            None => tr("pdf_import.size_as_file"),
            Some((n, _)) if n.is_empty() => tr("newdoc.preset_custom"),
            Some((n, _)) => n.clone(),
        };
        let ticked = st.page_size.as_ref().map(|(n, _)| n.clone());
        egui::ComboBox::from_id_salt("pdf_open_page_size")
            .selected_text(shown)
            .height(320.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(st.page_size.is_none(), tr("pdf_import.size_as_file"))
                    .clicked()
                {
                    st.page_size = None;
                }
                if ui
                    .selectable_label(ticked.as_deref() == Some(""), tr("newdoc.preset_custom"))
                    .clicked()
                {
                    let s = st
                        .page_size
                        .as_ref()
                        .map_or(tracedraw_core::document::paper::A4, |(_, s)| *s);
                    st.page_size = Some((String::new(), s));
                }
                ui.separator();
                let saved = &app.settings.page_sizes;
                if let Some(pick) = crate::ui::dialogs::paper_preset_menu(
                    ui,
                    ticked.as_deref().unwrap_or("-"),
                    saved,
                ) {
                    st.page_size = Some(pick);
                }
            })
            .response
            .rich_tip(crate::ui::tips::pair(
                "pdf_import.page_size",
                "pdf_import.page_size_tip",
            ));
    });
    if let Some((n, s)) = &mut st.page_size {
        if n.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(48.0);
                ui.add(
                    crate::ui::field::NumField::new(&mut s.width)
                        .range(1.0..=10000.0)
                        .suffix(" mm"),
                );
                ui.label("x");
                ui.add(
                    crate::ui::field::NumField::new(&mut s.height)
                        .range(1.0..=10000.0)
                        .suffix(" mm"),
                );
            });
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(24.0);
        ui.radio_value(&mut st.all_pages, true, tr("pdf_import.all_pages"));
        ui.radio_value(&mut st.all_pages, false, tr("pdf_import.pages"));
        let r = ui.add_enabled(
            !st.all_pages,
            egui::TextEdit::singleline(&mut st.range).desired_width(110.0),
        );
        r.rich_tip(crate::ui::tips::pair(
            "pdf_import.pages",
            "pdf_import.pages_tip",
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opening a PDF asks how to read its text once; the answer is kept,
    /// and Do not ask again stops the question.
    #[test]
    fn opening_a_pdf_asks_how_to_read_its_text() {
        let mut app = App::headless();
        app.settings.formats.pdf_ask_text = true;
        let p = std::path::Path::new("x.pdf");
        assert!(app.ask_pdf_text(p, false));
        assert!(matches!(
            app.dialog,
            crate::ui::dialogs::Dialog::PdfText(PdfTextState { import: false, .. })
        ));
        // Answered: the next read goes through without asking.
        app.pdf_text_chosen = true;
        assert!(!app.ask_pdf_text(p, false));
        assert!(app.ask_pdf_text(p, true));
        app.settings.formats.pdf_ask_text = false;
        assert!(!app.ask_pdf_text(p, true));
        app.settings.formats.pdf_text_as_curves = true;
        assert!(app.pdf_options().text_as_curves);
    }

    /// Text read as text joins its lines into paragraph text unless the
    /// choice is turned off (#2323).
    #[test]
    fn pdf_text_comes_in_as_paragraphs_by_default() {
        let mut app = App::headless();
        assert!(app.pdf_options().text_as_paragraphs);
        app.settings.formats.pdf_text_paragraphs = false;
        assert!(!app.pdf_options().text_as_paragraphs);
    }

    /// Pictures come in at their own size on the desktop unless Reduce
    /// very large pictures is ticked, which is off by default (#1835).
    #[test]
    fn pictures_are_reduced_only_when_asked_on_the_desktop() {
        use tracedraw_io::pdf_import::{FULL_PICTURE_PIXELS, REDUCED_PICTURE_PIXELS};
        let mut app = App::headless();
        assert!(!app.settings.formats.pdf_reduce_pictures);
        assert!(!always_reduce_pictures());
        assert_eq!(app.pdf_options().max_picture_pixels, FULL_PICTURE_PIXELS);
        app.settings.formats.pdf_reduce_pictures = true;
        assert_eq!(app.pdf_options().max_picture_pixels, REDUCED_PICTURE_PIXELS);
    }

    /// A picture that came in smaller is told after opening, in a message
    /// of its own, whatever the warnings say (#1835).
    #[test]
    fn a_shrunk_picture_is_told_after_opening() {
        use crate::ui::dialogs::Dialog;
        let mut app = App::headless();
        app.settings.warnings.unread_objects = false;
        app.settings.warnings.missing_fonts = false;
        app.pdf_shrunk = vec![tracedraw_io::pdf_import::ShrunkPicture {
            page: 0,
            width: 11_160,
            height: 14_794,
            new_width: 5_580,
            new_height: 7_397,
        }];
        app.show_open_notes("test_file.pdf".into(), 0);
        let Dialog::OpenNotes(notes) = &app.dialog else {
            panic!("no message: {:?}", app.dialog);
        };
        let text = notes.text();
        assert!(
            text.contains("test_file.pdf") && text.contains("11160 x 14794 to 5580 x 7397"),
            "{text}"
        );
        assert!(app.pdf_shrunk.is_empty(), "told once");
        // Nothing shrunk: no message.
        app.dialog = Dialog::None;
        app.show_open_notes("other.pdf".into(), 0);
        assert!(matches!(app.dialog, Dialog::None));
    }
}

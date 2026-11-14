//! A picture dragged from the drawing and let go on a palette bar (#2137):
//! a small window asks how many colours to take from it (a slider with
//! its number box), shows them, and OK adds them to the palette it was
//! dropped on, the colours it already has skipped. Cancel adds nothing.
//!
//! The colours are those of Add Colors from Bitmap
//! ([`crate::ops2::picture_colors_from_sample`]): the picture is read
//! once, when it is dropped, and the colours are worked out again only
//! when the number changes, never every frame. A built-in or locked
//! palette takes no colours: OK then makes a new palette of the user's
//! with them. See docs/behavior/color-palettes.md.
//!
//! The same window makes colour styles of objects let go on the Color
//! Styles docker (#2625): a picture's colours as above, a vector's fill
//! and outline colours (all of them, exactly, while they are no more
//! than the number asked for), grouped into a colour harmony unless that
//! is unticked. See docs/behavior/color-palettes.md.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::palette::Row;
use crate::ui::tips::RichTip;
use egui::{Context, Pos2};
use std::sync::Arc;
use tracedraw_core::Color;

/// How many colours the window offers at first, and its range.
pub const DEFAULT_COUNT: u32 = 8;
pub const MIN_COUNT: u32 = 2;
pub const MAX_COUNT: u32 = 64;

/// The palette a picture was dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteTarget {
    /// A palette of [`App::palettes`] (the default palette's bar too).
    Palette(usize),
    /// The document palette (one undo step).
    Document,
    /// The drawing's colour styles (#2625), one undo step.
    ColorStyles,
}

/// The window's state: where the colours go, how many, and the colours
/// for that number, kept until it changes.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureColorsState {
    pub target: PaletteTarget,
    pub count: u32,
    /// The picture's pixels the colours are chosen from, read once.
    sample: Arc<Vec<[u8; 3]>>,
    /// The colours for `shown_for` colours.
    colors: Vec<Color>,
    shown_for: u32,
    /// How many times the colours were worked out (a check that they are
    /// not every frame).
    pub computed: u32,
    /// The vectors' own colours (fills and outlines), kept as they are
    /// (CMYK, spot) while they are no more than `count` (#2625).
    exact: Vec<Color>,
    /// Colour styles only: the new styles form one harmony.
    pub harmony: bool,
}

impl PictureColorsState {
    pub fn new(target: PaletteTarget, sample: Vec<[u8; 3]>) -> Self {
        let mut st = PictureColorsState {
            target,
            count: DEFAULT_COUNT,
            sample: Arc::new(sample),
            colors: Vec::new(),
            shown_for: 0,
            computed: 0,
            exact: Vec::new(),
            harmony: true,
        };
        st.colors();
        st
    }

    /// The window for colour styles from objects (#2625): the pixels of
    /// the pictures among them (`sample`) and the vectors' colours
    /// (`exact`).
    pub fn for_color_styles(sample: Vec<[u8; 3]>, exact: Vec<Color>) -> Self {
        let mut st = PictureColorsState::new(PaletteTarget::ColorStyles, Vec::new());
        st.sample = Arc::new(sample);
        st.exact = exact;
        st.shown_for = 0;
        st.computed = 0;
        st.colors();
        st
    }

    /// The colours for [`Self::count`], worked out again only when the
    /// count changed since the last time.
    pub fn colors(&mut self) -> &[Color] {
        self.count = self.count.clamp(MIN_COUNT, MAX_COUNT);
        if self.shown_for != self.count {
            self.colors = if self.sample.is_empty() && self.exact.len() <= self.count as usize {
                // Vectors only, no more colours than asked for: theirs.
                self.exact.clone()
            } else if self.sample.is_empty() {
                let rgb: Vec<[u8; 3]> = self.exact.iter().map(|c| c.to_rgb8()).collect();
                crate::ops2::picture_colors_from_sample(&rgb, self.count)
            } else {
                let mut sample = self.sample.as_ref().clone();
                sample.extend(self.exact.iter().map(|c| c.to_rgb8()));
                crate::ops2::picture_colors_from_sample(&sample, self.count)
            };
            self.shown_for = self.count;
            self.computed += 1;
        }
        &self.colors
    }
}

impl App {
    /// The palette whose strip lies at `at` (screen), the strip drawn
    /// last (over the others) first.
    pub fn palette_target_at(&self, at: Pos2) -> Option<PaletteTarget> {
        let (which, _) = self
            .palette_strip_rects
            .iter()
            .rev()
            .find(|(_, r)| r.contains(at))?;
        self.palette_target_of(*which)
    }

    /// The palette a strip shows.
    pub fn palette_target_of(&self, which: Row) -> Option<PaletteTarget> {
        match which {
            Row::Main => self
                .visible_palettes
                .first()
                .map(|i| PaletteTarget::Palette(*i)),
            Row::Document => self.has_document().then_some(PaletteTarget::Document),
            Row::Extra(k) => {
                let r = self.extra_palettes.get(k)?;
                if r.document {
                    self.has_document().then_some(PaletteTarget::Document)
                } else {
                    (r.index < self.palettes.len()).then_some(PaletteTarget::Palette(r.index))
                }
            }
        }
    }

    /// The selection is one picture and the pointer is over a palette
    /// strip: a move let go now gives that palette the picture's colours
    /// instead of moving it.
    pub fn picture_drop_target(&self) -> Option<PaletteTarget> {
        if self.selection.len() != 1 || self.selected_bitmap().is_none() {
            return None;
        }
        self.pointer_screen.and_then(|p| self.palette_target_at(p))
    }

    /// Open the window that takes the selected picture's colours to
    /// `target`. False when no picture is selected or it cannot be read.
    pub fn open_picture_colors(&mut self, target: PaletteTarget) -> bool {
        let Some(sample) = self.selected_bitmap_sample() else {
            return false;
        };
        self.dialog = crate::ui::dialogs::Dialog::PictureColors(Box::new(PictureColorsState::new(
            target, sample,
        )));
        true
    }

    /// `target` takes colours itself (else OK makes a new palette).
    pub fn picture_target_takes_colors(&self, target: PaletteTarget) -> bool {
        match target {
            PaletteTarget::Document | PaletteTarget::ColorStyles => self.has_document(),
            PaletteTarget::Palette(i) => self.palette_takes_colors(i),
        }
    }

    /// The name `target` is shown under.
    pub fn picture_target_name(&self, target: PaletteTarget) -> String {
        match target {
            PaletteTarget::Document => self.document_palette_name(),
            PaletteTarget::ColorStyles => tr("docker.color_styles"),
            PaletteTarget::Palette(i) => self.palette_name(i),
        }
    }

    /// The pointer is over the Color Styles docker with objects selected:
    /// a move let go now makes colour styles of their colours instead of
    /// moving them (#2625).
    pub fn color_styles_drop_target(&self) -> bool {
        !self.selection.is_empty()
            && self.has_document()
            && self
                .color_styles_rect
                .zip(self.pointer_screen)
                .is_some_and(|(r, p)| r.contains(p))
    }

    /// The selection's colours for colour styles: the pixels of its
    /// pictures (also in groups and clip frames) and the fill and outline
    /// colours of everything in it.
    pub fn selection_color_sample(&self) -> (Vec<[u8; 3]>, Vec<Color>) {
        fn pictures(shapes: &[tracedraw_core::Shape], out: &mut Vec<[u8; 3]>) {
            use tracedraw_core::ShapeKind;
            for s in shapes {
                match &s.kind {
                    ShapeKind::Bitmap { png, .. } => {
                        if let Some(img) = crate::bitmap_fx::decode(png) {
                            out.extend(crate::bitmap_modes::palette_sample(&img));
                        }
                    }
                    ShapeKind::Group { children } => pictures(children, out),
                    ShapeKind::ClipFrame { frame, contents } => {
                        pictures(std::slice::from_ref(frame), out);
                        pictures(contents, out);
                    }
                    _ => {}
                }
            }
        }
        let shapes = self.selected_shapes();
        let mut sample = Vec::new();
        pictures(&shapes, &mut sample);
        let mut exact = Vec::new();
        App::colors_of(&shapes, &mut exact);
        (sample, exact)
    }

    /// Open the window that makes colour styles of the selection's
    /// colours (#2625). False (and a word in the status line) when the
    /// selection has no colour to take.
    pub fn open_color_styles_from_selection(&mut self) -> bool {
        let (sample, exact) = self.selection_color_sample();
        if sample.is_empty() && exact.is_empty() {
            self.status = tr("palette.extract_styles_nothing");
            return false;
        }
        self.dialog = crate::ui::dialogs::Dialog::PictureColors(Box::new(
            PictureColorsState::for_color_styles(sample, exact),
        ));
        true
    }

    /// OK for colour styles: each of `colors` the drawing has no style of
    /// yet becomes one ("Color N"), in a new harmony ("Harmony N") with
    /// `harmony`; one undo step. Returns how many were added.
    pub fn add_color_styles(&mut self, colors: &[Color], harmony: bool) -> usize {
        let mut st = self.doc().color_styles.clone();
        let group = harmony.then(|| {
            let mut n = 1;
            loop {
                let name = trf("palette.harmony_name", &[("n", &n.to_string())]);
                if !st
                    .iter()
                    .any(|s| s.harmony.as_deref() == Some(name.as_str()))
                {
                    break name;
                }
                n += 1;
            }
        });
        let before = st.len();
        for c in colors {
            if st.iter().any(|s| s.color == *c) {
                continue;
            }
            st.push(tracedraw_core::ColorStyle {
                name: format!("Color {}", st.len() + 1),
                color: *c,
                harmony: group.clone(),
            });
        }
        let added = st.len() - before;
        if added > 0 {
            self.run(tracedraw_core::Command::SetColorStyles { styles: st });
        }
        added
    }

    /// OK in the window: `colors` join `target` (those it has not got
    /// yet), or a new palette of the user's when `target` cannot be
    /// changed. Says in the status line what happened; returns how many
    /// colours were added.
    pub fn add_picture_colors(&mut self, target: PaletteTarget, colors: &[Color]) -> usize {
        if target == PaletteTarget::ColorStyles {
            return self.add_color_styles(colors, true);
        }
        let count_in = |app: &App, t: PaletteTarget| match t {
            PaletteTarget::Document if app.has_document() => app.doc().palette.len(),
            PaletteTarget::Document | PaletteTarget::ColorStyles => 0,
            PaletteTarget::Palette(i) => app.palettes.get(i).map_or(0, |p| p.colors.len()),
        };
        let target = if self.picture_target_takes_colors(target) {
            target
        } else {
            let name = self.next_palette_name();
            match self.new_palette(&name) {
                Some(i) => {
                    self.settings.save();
                    PaletteTarget::Palette(i)
                }
                None => return 0,
            }
        };
        let before = count_in(self, target);
        match target {
            PaletteTarget::Document => {
                self.add_colors_to_document_palette(colors);
            }
            PaletteTarget::ColorStyles => {}
            PaletteTarget::Palette(i) => {
                self.add_colors_to_palette(i, colors);
            }
        }
        let added = count_in(self, target).saturating_sub(before);
        let name = self.picture_target_name(target);
        self.status = if added == 0 {
            trf("palette.extract_none", &[("name", &name)])
        } else {
            trf(
                "palette.extract_added",
                &[("n", &added.to_string()), ("name", &name)],
            )
        };
        added
    }
}

/// The window: the number of colours, the colours, OK and Cancel.
pub fn dialog(app: &mut App, ctx: &Context, st: &mut PictureColorsState, close: &mut bool) {
    let takes = app.picture_target_takes_colors(st.target);
    let name = app.picture_target_name(st.target);
    let styles = st.target == PaletteTarget::ColorStyles;
    let title = if styles {
        tr("palette.extract_styles_title")
    } else {
        tr("palette.extract_title")
    };
    crate::ui::dialogs::window(ctx, title).show(ctx, |ui| {
        ui.set_max_width(360.0);
        let note = if styles {
            tr("palette.extract_into_styles")
        } else if takes {
            trf("palette.extract_into", &[("name", &name)])
        } else {
            trf("palette.extract_new_palette", &[("name", &name)])
        };
        ui.label(note);
        ui.add_space(6.0);
        ui.add(
            crate::ui::slider::Slider::new(&mut st.count, MIN_COUNT..=MAX_COUNT)
                .label(tr("palette.extract_count"))
                .integer()
                .width(300.0),
        )
        .rich_tip(crate::ui::tips::pair(
            "palette.extract_count",
            "palette.extract_count_tip",
        ));
        ui.add_space(6.0);
        // The colours, 16 to a row; worked out only when the number
        // changed.
        let colors = st.colors().to_vec();
        let side = 18.0;
        let gap = 2.0;
        let per_row = 16usize;
        let rows = colors.len().div_ceil(per_row).max(1);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(per_row as f32 * (side + gap), rows as f32 * (side + gap)),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        for (k, c) in colors.iter().enumerate() {
            let at = rect.min
                + egui::vec2(
                    (k % per_row) as f32 * (side + gap),
                    (k / per_row) as f32 * (side + gap),
                );
            let r = egui::Rect::from_min_size(at, egui::vec2(side, side));
            painter.rect_filled(r, 0.0, crate::canvas::to_color32(*c));
            painter.rect_stroke(
                r,
                0.0,
                egui::Stroke::new(1.0, Tokens::border()),
                egui::StrokeKind::Inside,
            );
        }
        ui.label(
            egui::RichText::new(trf(
                "palette.extract_found",
                &[("n", &colors.len().to_string())],
            ))
            .color(Tokens::text_dim())
            .size(11.0),
        );
        if styles {
            ui.add_space(4.0);
            ui.checkbox(&mut st.harmony, tr("palette.extract_harmony"))
                .rich_tip(crate::ui::tips::pair(
                    "palette.extract_harmony",
                    "palette.extract_harmony_tip",
                ));
        }
        ui.add_space(6.0);
        if crate::ui::dialogs::ok_cancel(ui, close) {
            if styles {
                let added = app.add_color_styles(&colors, st.harmony);
                app.status = if added == 0 {
                    trf("palette.extract_none", &[("name", &name)])
                } else {
                    trf(
                        "palette.extract_added",
                        &[("n", &added.to_string()), ("name", &name)],
                    )
                };
            } else {
                app.add_picture_colors(st.target, &colors);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Drag;
    use tracedraw_core::geometry::Point;

    /// A drawing with a picture of four colours, selected, a palette of
    /// the user's in a bar of its own, and where its strip and the
    /// default palette's lie.
    fn app_with_picture() -> (App, usize) {
        let mut app = App::headless();
        app.show_welcome = false;
        let img = image::RgbaImage::from_fn(40, 40, |x, y| match (x < 20, y < 20) {
            (true, true) => image::Rgba([220, 20, 30, 255]),
            (false, true) => image::Rgba([20, 160, 40, 255]),
            (true, false) => image::Rgba([30, 40, 200, 255]),
            (false, false) => image::Rgba([255, 255, 255, 255]),
        });
        let id = app.place_image(img, None).expect("picture");
        app.select(vec![id]);
        let mine = app.new_palette("Mine").expect("palette");
        let k = app
            .extra_palettes
            .iter()
            .position(|r| !r.document && r.index == mine)
            .expect("its bar");
        app.palette_strip_rects = vec![
            (
                Row::Main,
                egui::Rect::from_min_size(egui::pos2(0.0, 900.0), egui::vec2(800.0, 25.0)),
            ),
            (
                Row::Extra(k),
                egui::Rect::from_min_size(egui::pos2(0.0, 870.0), egui::vec2(800.0, 25.0)),
            ),
            (
                Row::Document,
                egui::Rect::from_min_size(egui::pos2(0.0, 840.0), egui::vec2(800.0, 25.0)),
            ),
        ];
        (app, mine)
    }

    fn moving(app: &mut App, at: Pos2) {
        app.drag = Drag::Move {
            start: Point::new(20.0, 20.0),
            last: Point::new(40.0, 60.0),
            total: tracedraw_core::geometry::Vec2::new(20.0, 40.0),
            start_bounds: tracedraw_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
            source: None,
            start_geometry: None,
        };
        app.pointer_screen = Some(at);
    }

    /// The menu path and the drop give the same colours (one function).
    #[test]
    fn the_window_shows_the_colours_add_colors_from_bitmap_takes() {
        let (app, _) = app_with_picture();
        let sample = app.selected_bitmap_sample().expect("sample");
        let mut st = PictureColorsState::new(PaletteTarget::Document, sample);
        for n in [2, 3, 8, 16, 64] {
            st.count = n;
            let menu = app.selected_bitmap_colors(n).expect("colours");
            assert_eq!(st.colors(), menu.as_slice(), "{n}");
        }
        // Four colours in the picture: no more than four, however many
        // are asked for.
        st.count = 64;
        assert_eq!(st.colors().len(), 4);
        // Out of range: kept in it.
        st.count = 500;
        st.colors();
        assert_eq!(st.count, MAX_COUNT);
        st.count = 0;
        st.colors();
        assert_eq!(st.count, MIN_COUNT);
    }

    /// The colours are worked out when the number changes, not on every
    /// frame the window is drawn.
    #[test]
    fn the_preview_is_worked_out_only_when_the_number_changes() {
        let (mut app, mine) = app_with_picture();
        assert!(app.open_picture_colors(PaletteTarget::Palette(mine)));
        let ctx = crate::theme::ui_context();
        let frame = |app: &mut App| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::ui::dialogs::show(app, &ui.ctx().clone())
            });
            out.textures_delta.clear();
        };
        let computed = |app: &App| match &app.dialog {
            crate::ui::dialogs::Dialog::PictureColors(st) => st.computed,
            _ => panic!("the window closed"),
        };
        for _ in 0..5 {
            frame(&mut app);
        }
        assert_eq!(computed(&app), 1, "once, when it opened");
        if let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog {
            st.count = 3;
        }
        for _ in 0..5 {
            frame(&mut app);
        }
        assert_eq!(computed(&app), 2, "once more for the new number");
    }

    /// A picture let go on a palette bar opens the window for that
    /// palette and stays where it was; OK adds its colours (skipping
    /// those the palette has), Cancel adds nothing.
    #[test]
    fn a_picture_dropped_on_a_palette_bar_offers_its_colours() {
        let (mut app, mine) = app_with_picture();
        let id = app.selection[0];
        let before = app.doc().find_shape(id).expect("picture").clone();
        app.palettes[mine].colors = vec![("Red".into(), Color::rgb8(220, 20, 30))];
        moving(&mut app, egui::pos2(100.0, 880.0));
        assert_eq!(
            app.picture_drop_target(),
            Some(PaletteTarget::Palette(mine))
        );
        app.end_drag();
        let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog else {
            panic!("no window: {:?}", app.dialog);
        };
        assert_eq!(st.target, PaletteTarget::Palette(mine));
        assert_eq!(st.count, DEFAULT_COUNT);
        let colors = st.colors().to_vec();
        assert_eq!(colors.len(), 4);
        assert_eq!(
            app.doc().find_shape(id).expect("picture").transform,
            before.transform,
            "the picture did not move"
        );
        // Cancel: the palette as it was.
        app.dialog = crate::ui::dialogs::Dialog::None;
        assert_eq!(app.palettes[mine].colors.len(), 1);
        // OK: the three it had not got.
        assert_eq!(
            app.add_picture_colors(PaletteTarget::Palette(mine), &colors),
            3
        );
        let got: Vec<Color> = app.palettes[mine].colors.iter().map(|(_, c)| *c).collect();
        assert_eq!(got.len(), 4);
        for c in &colors {
            assert!(got.contains(c), "{c:?}");
        }
        // Again: nothing new.
        assert_eq!(
            app.add_picture_colors(PaletteTarget::Palette(mine), &colors),
            0
        );
    }

    /// The document palette takes them as one undo step.
    #[test]
    fn a_picture_dropped_on_the_document_palette_adds_one_undo_step() {
        let (mut app, _) = app_with_picture();
        moving(&mut app, egui::pos2(100.0, 850.0));
        app.end_drag();
        let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog else {
            panic!("no window");
        };
        assert_eq!(st.target, PaletteTarget::Document);
        let colors = st.colors().to_vec();
        let before = app.doc().palette.clone();
        app.add_picture_colors(PaletteTarget::Document, &colors);
        for c in &colors {
            assert!(app.doc().palette.contains(c));
        }
        app.undo();
        assert_eq!(app.doc().palette, before);
    }

    /// A built-in palette is not changed: OK makes a new palette of the
    /// user's with the colours.
    #[test]
    fn a_builtin_palette_gets_a_new_palette_instead() {
        let (mut app, _) = app_with_picture();
        moving(&mut app, egui::pos2(100.0, 910.0));
        let main = app.visible_palettes[0];
        assert!(app.palettes[main].builtin);
        assert_eq!(
            app.picture_drop_target(),
            Some(PaletteTarget::Palette(main))
        );
        app.end_drag();
        let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog else {
            panic!("no window");
        };
        let colors = st.colors().to_vec();
        let target = st.target;
        assert!(!app.picture_target_takes_colors(target));
        let builtin = app.palettes[main].colors.clone();
        let n = app.palettes.len();
        assert_eq!(app.add_picture_colors(target, &colors), 4);
        assert_eq!(app.palettes[main].colors, builtin, "unchanged");
        assert_eq!(app.palettes.len(), n + 1);
        let new = &app.palettes[n];
        assert!(!new.builtin);
        assert_eq!(new.colors.len(), 4);
    }

    /// Anything but one picture moved onto a palette does what it did:
    /// the objects move (or join the document palette there).
    #[test]
    fn other_objects_dropped_on_a_palette_bar_are_not_taken_as_pictures() {
        let (mut app, _) = app_with_picture();
        let rect = app
            .new_shape(tracedraw_core::ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![rect]);
        moving(&mut app, egui::pos2(100.0, 880.0));
        assert_eq!(app.picture_drop_target(), None);
        app.end_drag();
        assert!(matches!(app.dialog, crate::ui::dialogs::Dialog::None));
        // Two objects, the picture among them: not a picture either.
        let pic = app
            .doc()
            .all_layers()
            .flat_map(|l| l.shapes.iter())
            .find(|s| matches!(s.kind, tracedraw_core::ShapeKind::Bitmap { .. }))
            .map(|s| s.id)
            .expect("picture");
        app.select(vec![pic, rect]);
        moving(&mut app, egui::pos2(100.0, 880.0));
        assert_eq!(app.picture_drop_target(), None);
        // Off every strip: nothing.
        app.select(vec![pic]);
        moving(&mut app, egui::pos2(100.0, 100.0));
        assert_eq!(app.picture_drop_target(), None);
    }

    /// Vectors let go on the Color Styles docker open the window for
    /// colour styles and stay where they were; their fill and outline
    /// colours are offered exactly (a CMYK one stays CMYK) while they are
    /// no more than the number asked for, fewer ones are worked out past
    /// it; OK makes them styles in one harmony, Harmony 1, then Harmony 2
    /// for the next ones, skipping colours that are styles already
    /// (#2625).
    #[test]
    fn objects_dropped_on_the_color_styles_docker_become_a_harmony() {
        let (mut app, _) = app_with_picture();
        let kind = |x: f64| tracedraw_core::ShapeKind::Rect {
            rect: tracedraw_core::geometry::Rect::new(x, 0.0, x + 10.0, 10.0),
            radius: 0.0,
            corners: None,
        };
        let a = app.new_shape(kind(0.0)).expect("a");
        let b = app.new_shape(kind(20.0)).expect("b");
        let cmyk = Color::cmyk_pct(0.0, 60.0, 100.0, 0.0);
        let red = Color::rgb8(200, 10, 10);
        let blue = Color::rgb8(10, 10, 200);
        app.select(vec![a]);
        app.apply_fill(tracedraw_core::Fill::Solid(cmyk));
        app.apply_outline_color(Some(blue));
        app.select(vec![b]);
        app.apply_fill(tracedraw_core::Fill::Solid(red));
        app.select(vec![a, b]);
        let before = app.doc().find_shape(a).expect("a").clone();
        app.color_styles_rect = Some(egui::Rect::from_min_size(
            egui::pos2(1000.0, 100.0),
            egui::vec2(250.0, 400.0),
        ));
        moving(&mut app, egui::pos2(100.0, 100.0));
        assert!(!app.color_styles_drop_target());
        moving(&mut app, egui::pos2(1100.0, 200.0));
        assert!(app.color_styles_drop_target());
        app.end_drag();
        let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog else {
            panic!("no window: {:?}", app.dialog);
        };
        assert_eq!(st.target, PaletteTarget::ColorStyles);
        assert!(st.harmony);
        let colors = st.colors().to_vec();
        for c in [cmyk, blue, red] {
            assert!(colors.contains(&c), "{c:?} missing from {colors:?}");
        }
        st.count = 2;
        assert_eq!(st.colors().len(), 2);
        assert_eq!(
            app.doc().find_shape(a).expect("a").transform,
            before.transform,
            "the objects did not move"
        );
        app.dialog = crate::ui::dialogs::Dialog::None;
        assert_eq!(app.add_color_styles(&colors, true), colors.len());
        let styles = app.doc().color_styles.clone();
        assert_eq!(styles.len(), colors.len());
        assert!(styles
            .iter()
            .all(|s| s.harmony.as_deref() == Some("Harmony 1")));
        assert!(styles.iter().any(|s| s.color == cmyk));
        app.undo();
        assert!(app.doc().color_styles.is_empty(), "one undo step");
        app.redo();
        // Again with one new colour: only it is added, in Harmony 2.
        let green = Color::rgb8(10, 200, 10);
        assert_eq!(app.add_color_styles(&[red, green], true), 1);
        let last = app.doc().color_styles.last().cloned().expect("style");
        assert_eq!(
            (last.color, last.harmony.as_deref()),
            (green, Some("Harmony 2"))
        );
        assert_eq!(app.add_color_styles(&[Color::rgb8(1, 2, 3)], false), 1);
        assert_eq!(app.doc().color_styles.last().expect("style").harmony, None);
    }

    /// A picture let go on the Color Styles docker offers its colours as
    /// it does on a palette; nothing selected with colour: no window.
    #[test]
    fn a_picture_dropped_on_the_color_styles_docker_offers_its_colours() {
        let (mut app, _) = app_with_picture();
        app.color_styles_rect = Some(egui::Rect::from_min_size(
            egui::pos2(1000.0, 100.0),
            egui::vec2(250.0, 400.0),
        ));
        moving(&mut app, egui::pos2(1100.0, 200.0));
        app.end_drag();
        let crate::ui::dialogs::Dialog::PictureColors(st) = &mut app.dialog else {
            panic!("no window: {:?}", app.dialog);
        };
        assert_eq!(st.target, PaletteTarget::ColorStyles);
        assert_eq!(st.colors().len(), 4);
        app.dialog = crate::ui::dialogs::Dialog::None;
        app.select(Vec::new());
        assert!(!app.open_color_styles_from_selection());
    }
}

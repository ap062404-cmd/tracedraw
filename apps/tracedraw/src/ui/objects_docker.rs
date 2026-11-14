//! The Objects docker: every page with its layers, topmost first (or
//! only the current page, a choice of the bottom bar), then the Master
//! Page with the master layers, the guides and the document grid. A layer row has a collapse arrow, an eye (show or
//! hide), a printer (print and export or not), a padlock (lock or
//! unlock), the layer icon and the name, in bold on a blue row for the
//! active layer, the one new objects go to; clicking a layer makes it
//! active. Under each open layer come its objects, topmost first, each
//! with a thumbnail (the object itself drawn small, see
//! `object_thumbs.rs`) and its name (or what it is when it has no name).
//! The thumbnail size is set by the control in the bottom bar, the wheel
//! over it, or Ctrl+wheel over the list. Groups open to show their objects. Clicking an object
//! selects it (Shift adds it) and makes its layer active; double-clicking
//! a name renames the layer or object; a layer's context menu moves,
//! renames, moves, shows, locks, empties into the master page or deletes
//! it, and an object's context menu moves it to a layer of any page or of
//! the master page (#1477). The bar at the bottom adds a layer or a master
//! layer (its menu also sets which master layers are drawn in front of
//! the page's layers, #1132), shows all pages or the current one, and
//! deletes the selected objects or, with none, the active layer.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::dockers::kind_name;
use crate::ui::tips::RichTip;
use egui::{Color32, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use tracedraw_core::{Command, Layer, LayerId, MasterScope, PageId, Shape, ShapeId, ShapeKind};

/// Height of a row.
const ROW_H: f32 = 22.0;
/// Indent per nesting level.
const INDENT: f32 = 16.0;
/// The nesting level of a layer's own objects (under the page and the
/// layer); a group's objects come deeper.
const LAYER_OBJECT_DEPTH: usize = 2;
/// Height of the bar with the New Layer, New Master Layer and Delete
/// buttons.
const BAR_H: f32 = 30.0;
/// The top sheet of the layer icon: blue for a page's layers, amber for
/// master layers.
const LAYER_TOP: Color32 = Color32::from_rgb(0x4A, 0x90, 0xD9);
const MASTER_TOP: Color32 = Color32::from_rgb(0xE0, 0x9A, 0x2E);
const GUIDE_BLUE: Color32 = Color32::from_rgb(0x1E, 0x6F, 0xD9);
const PLUS_GREEN: Color32 = Color32::from_rgb(0x1E, 0x9E, 0x3C);

/// What the docker asks for after drawing.
#[derive(Default)]
struct Actions {
    visible: Option<(LayerId, bool)>,
    printable: Option<(LayerId, bool)>,
    locked: Option<(LayerId, bool)>,
    scope: Option<(LayerId, MasterScope)>,
    in_front: Option<(LayerId, bool)>,
    pick: Option<LayerId>,
    select: Option<ShapeId>,
    /// An object's eye or padlock was clicked (#832).
    object_visible: Option<(ShapeId, bool)>,
    object_locked: Option<(ShapeId, bool)>,
    rename_layer: Option<(LayerId, String)>,
    rename_object: Option<(ShapeId, String)>,
    rename_page: Option<String>,
    delete_layer: Option<LayerId>,
    reorder: Option<(LayerId, usize)>,
    /// Each top-level object row drawn and where, and each layer row
    /// that takes objects, for a row dragged along the list (live test
    /// of 0.41).
    object_rows: Vec<(ShapeId, Rect)>,
    /// Every object row of the list, top to bottom (laid out or not), for
    /// Shift+click's range (#2483).
    row_order: Vec<ShapeId>,
    layer_drop_rows: Vec<(LayerId, Rect)>,
    collapse: Option<LayerId>,
    expand_group: Option<ShapeId>,
    toggle_page: Option<PageId>,
    toggle_master: bool,
    /// A row of another page was used: that page becomes the current one
    /// first.
    goto: Option<PageId>,
    select_layer: Option<LayerId>,
    /// Cut, copy or paste a whole layer's objects from its menu (#1992).
    layer_clip: Option<(LayerId, LayerClip)>,
    new_layer: bool,
    new_master: Option<MasterScope>,
    /// An object row was right-clicked there: its context menu (#2601).
    object_menu: Option<(ShapeId, Pos2)>,
    layer_to_master: Option<LayerId>,
    master_to_page: Option<LayerId>,
    /// Not an action: the page whose rows are being drawn (None for the
    /// master page).
    row_page: Option<PageId>,
    toggle_guides: Option<(Option<PageId>, bool)>,
    /// A Guides row was clicked: the master page's guidelines (true) or
    /// the page's own (false) are the ones edited from now (#1928).
    guides_master: Option<bool>,
    toggle_grid: bool,
    /// Lock or unlock all the guidelines of a Guides row: the master
    /// page's (true) or the page's own, and locked or not.
    lock_guides: Option<(bool, bool)>,
    /// Not an action: the object whose row is scrolled into view.
    scroll_to: Option<ShapeId>,
    /// Not an action: the thumbnail cache, taken from the app while the
    /// tree is drawn, the thumbnail size (points) and the drawing's
    /// (instance, revision).
    thumbs: crate::object_thumbs::ObjectThumbs,
    thumb_size: f32,
    drawing: (u64, u64),
}

/// Where a layer sits: its index in its list (0 at the bottom), how many
/// the list holds, whether it is a master layer and whether it shows on
/// the current page.
#[derive(Clone, Copy)]
struct Place {
    index: usize,
    count: usize,
    master: bool,
    shown: bool,
    /// The page the layer belongs to (None for a master layer).
    page: Option<PageId>,
}

/// How a row is painted.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    Object { selected: bool },
    Layer { active: bool },
    Node,
}

/// The label an object shows: its name, else what it is.
pub fn object_label(s: &Shape) -> String {
    match s.name.as_deref().map(str::trim) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => kind_name(&s.kind),
    }
}

/// An eye, open when `on`, else closed (a lid line), centred in `r`.
pub(crate) fn paint_eye(ui: &Ui, r: Rect, on: bool) {
    let p = ui.painter();
    let c = r.center();
    let color = if on {
        Tokens::icon()
    } else {
        Tokens::icon_off()
    };
    let w = 6.0;
    let pts_top: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            let x = -w + 2.0 * w * t;
            Pos2::new(c.x + x, c.y - 3.6 * (1.0 - (x / w).powi(2)))
        })
        .collect();
    let pts_bottom: Vec<Pos2> = pts_top
        .iter()
        .map(|q| Pos2::new(q.x, c.y + (c.y - q.y)))
        .collect();
    let stroke = Stroke::new(1.1, color);
    p.add(egui::Shape::line(pts_top, stroke));
    if on {
        p.add(egui::Shape::line(pts_bottom, stroke));
        p.circle_filled(c, 1.9, color);
    } else {
        // Closed: only the lower lid with lashes.
        p.add(egui::Shape::line(pts_bottom.clone(), stroke));
        for dx in [-3.5f32, 0.0, 3.5] {
            let base = Pos2::new(c.x + dx, c.y + 3.0 * (1.0 - (dx / w).powi(2)));
            p.line_segment([base, base + Vec2::new(dx * 0.15, 2.0)], stroke);
        }
    }
}

/// A printer, dark when `on`, else pale, centred in `r`.
fn paint_printer(ui: &Ui, r: Rect, on: bool) {
    let p = ui.painter();
    let c = r.center();
    let color = if on {
        Tokens::icon()
    } else {
        Tokens::icon_off()
    };
    let stroke = Stroke::new(1.0, color);
    // The sheet going in at the top.
    p.rect_stroke(
        Rect::from_min_max(c + Vec2::new(-3.5, -6.5), c + Vec2::new(3.5, -2.0)),
        0.0,
        stroke,
        egui::epaint::StrokeKind::Inside,
    );
    // The body.
    p.rect_filled(
        Rect::from_min_max(c + Vec2::new(-6.5, -2.5), c + Vec2::new(6.5, 3.5)),
        1.0,
        color,
    );
    // The printed sheet coming out.
    let out = Rect::from_min_max(c + Vec2::new(-3.5, 1.0), c + Vec2::new(3.5, 6.5));
    p.rect_filled(out, 0.0, Color32::WHITE);
    p.rect_stroke(out, 0.0, stroke, egui::epaint::StrokeKind::Inside);
}

/// A padlock, shut when `locked`, else open, centred in `r`: dark when
/// shut, pale when open.
fn paint_lock(ui: &Ui, r: Rect, locked: bool) {
    let color = if locked {
        Tokens::icon()
    } else {
        Tokens::icon_off()
    };
    paint_padlock(ui.painter(), r, locked, color);
}

/// A padlock in `color`, shut when `locked`, else open.
pub(crate) fn paint_padlock(p: &Painter, r: Rect, locked: bool, color: Color32) {
    let c = r.center();
    let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(9.0, 7.0));
    p.rect_filled(body, 1.0, color);
    let stroke = Stroke::new(1.3, color);
    let left = body.left() + 2.2;
    let right = body.right() - 2.2;
    let top = body.top() - 4.0;
    let shackle_right = if locked { right } else { right + 3.0 };
    let arc: Vec<Pos2> = (0..=10)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / 10.0;
            let mid = (left + shackle_right) / 2.0;
            let rx = (shackle_right - left) / 2.0;
            Pos2::new(mid - rx * a.cos(), top - 0.5 - 2.0 * a.sin() + 2.0)
        })
        .collect();
    p.add(egui::Shape::line(arc, stroke));
    p.line_segment(
        [Pos2::new(left, top + 1.5), Pos2::new(left, body.top())],
        stroke,
    );
    let right_end = if locked { body.top() } else { top + 3.0 };
    p.line_segment(
        [
            Pos2::new(shackle_right, top + 1.5),
            Pos2::new(shackle_right, right_end),
        ],
        stroke,
    );
}

/// Three stacked sheets: the layer icon, its top sheet amber for a master
/// layer.
fn paint_layer_icon(p: &Painter, r: Rect, master: bool) {
    let c = r.center();
    for (i, dy) in [3.0f32, 0.0, -3.0].iter().enumerate() {
        let pts = vec![
            Pos2::new(c.x - 6.0, c.y + dy),
            Pos2::new(c.x, c.y + dy - 2.8),
            Pos2::new(c.x + 6.0, c.y + dy),
            Pos2::new(c.x, c.y + dy + 2.8),
        ];
        let fill = match (i, master) {
            (2, false) => LAYER_TOP,
            (2, true) => MASTER_TOP,
            _ => Color32::from_rgb(0xC9, 0xDD, 0xF2),
        };
        p.add(egui::Shape::convex_polygon(
            pts,
            fill,
            Stroke::new(0.8, Color32::from_rgb(0x2F, 0x6D, 0xB5)),
        ));
    }
}

/// A sheet with a folded corner: the page and master page nodes.
fn paint_page_icon(p: &Painter, r: Rect, master: bool) {
    let c = r.center();
    let (x0, x1, y0, y1) = (c.x - 5.0, c.x + 5.0, c.y - 6.5, c.y + 6.5);
    let fold = 3.5;
    let outline = vec![
        Pos2::new(x0, y0),
        Pos2::new(x1 - fold, y0),
        Pos2::new(x1, y0 + fold),
        Pos2::new(x1, y1),
        Pos2::new(x0, y1),
    ];
    let fill = if master {
        Color32::from_rgb(0xFB, 0xE7, 0xC6)
    } else {
        crate::theme::Tokens::field()
    };
    p.add(egui::Shape::convex_polygon(
        outline,
        fill,
        Stroke::new(1.0, Tokens::icon()),
    ));
    p.add(egui::Shape::line(
        vec![
            Pos2::new(x1 - fold, y0),
            Pos2::new(x1 - fold, y0 + fold),
            Pos2::new(x1, y0 + fold),
        ],
        Stroke::new(0.8, Tokens::icon()),
    ));
}

/// Two crossing guidelines.
fn paint_guides_icon(p: &Painter, r: Rect) {
    let c = r.center();
    let stroke = Stroke::new(1.2, GUIDE_BLUE);
    p.line_segment(
        [c + Vec2::new(-7.0, -2.5), c + Vec2::new(7.0, -2.5)],
        stroke,
    );
    p.line_segment([c + Vec2::new(2.5, -7.0), c + Vec2::new(2.5, 7.0)], stroke);
    p.circle_filled(c + Vec2::new(2.5, -2.5), 1.6, GUIDE_BLUE);
}

/// A three by three grid.
fn paint_grid_icon(p: &Painter, r: Rect) {
    let c = r.center();
    let stroke = Stroke::new(1.0, crate::theme::Tokens::text_dim());
    for i in 0..4 {
        let t = -6.0 + 4.0 * i as f32;
        p.line_segment([c + Vec2::new(t, -6.0), c + Vec2::new(t, 6.0)], stroke);
        p.line_segment([c + Vec2::new(-6.0, t), c + Vec2::new(6.0, t)], stroke);
    }
}

/// The open (pointing down) or closed (pointing right) arrow.
fn paint_arrow(ui: &Ui, r: Rect, open: bool) {
    let c = r.center();
    let pts = if open {
        vec![
            c + Vec2::new(-3.5, -1.8),
            c + Vec2::new(3.5, -1.8),
            c + Vec2::new(0.0, 2.2),
        ]
    } else {
        vec![
            c + Vec2::new(-1.8, -3.5),
            c + Vec2::new(2.2, 0.0),
            c + Vec2::new(-1.8, 3.5),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        pts,
        Tokens::text_dim(),
        Stroke::NONE,
    ));
}

/// The swatch of an object: its fill (or a crossed box for none) inside
/// its outline colour.
fn paint_swatch(ui: &Ui, r: Rect, s: &Shape) {
    let p = ui.painter();
    let fill = crate::app::fill_preview_color(&s.fill);
    let edge = s
        .stroke
        .as_ref()
        .map(|st| crate::canvas::to_color32(st.color))
        .unwrap_or(Tokens::text_dim());
    match fill {
        Some(c) => {
            p.rect_filled(r, 1.0, c);
        }
        None => {
            p.rect_filled(r, 1.0, Color32::WHITE);
            p.line_segment(
                [r.left_bottom(), r.right_top()],
                Stroke::new(1.0, Color32::from_rgb(0xE0, 0x40, 0x40)),
            );
        }
    }
    p.rect_stroke(
        r,
        1.0,
        Stroke::new(1.0, edge),
        egui::epaint::StrokeKind::Inside,
    );
}

/// A small green badge with a white plus at the top right of `r`.
fn paint_plus_badge(p: &Painter, r: Rect) {
    let badge = Rect::from_min_size(r.right_top() + Vec2::new(-7.0, -1.0), Vec2::splat(8.0));
    p.rect_filled(badge, 1.0, PLUS_GREEN);
    let c = badge.center();
    let w = Stroke::new(1.3, Color32::WHITE);
    p.line_segment([c - Vec2::new(2.5, 0.0), c + Vec2::new(2.5, 0.0)], w);
    p.line_segment([c - Vec2::new(0.0, 2.5), c + Vec2::new(0.0, 2.5)], w);
}

/// A waste bin, pale when `on` is false.
fn paint_bin(p: &Painter, r: Rect, on: bool) {
    let c = r.center();
    let color = if on {
        Tokens::icon()
    } else {
        Tokens::icon_off()
    };
    let stroke = Stroke::new(1.2, color);
    p.line_segment(
        [c + Vec2::new(-6.0, -4.5), c + Vec2::new(6.0, -4.5)],
        stroke,
    );
    p.line_segment(
        [c + Vec2::new(-2.0, -6.5), c + Vec2::new(2.0, -6.5)],
        stroke,
    );
    p.add(egui::Shape::closed_line(
        vec![
            c + Vec2::new(-4.5, -4.5),
            c + Vec2::new(4.5, -4.5),
            c + Vec2::new(3.5, 6.5),
            c + Vec2::new(-3.5, 6.5),
        ],
        stroke,
    ));
    for dx in [-1.6f32, 1.6] {
        p.line_segment(
            [c + Vec2::new(dx, -2.0), c + Vec2::new(dx * 0.8, 4.5)],
            Stroke::new(0.9, color),
        );
    }
}

/// A full-width row; returns its rectangle and its response.
fn row(ui: &mut Ui, look: Look) -> (Rect, egui::Response) {
    row_of(ui, look, ROW_H)
}

/// A full-width row `height` high.
fn row_of(ui: &mut Ui, look: Look, height: f32) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    match look {
        Look::Object { selected: true } => {
            p.rect_filled(rect, 0.0, crate::theme::Tokens::selected_row());
            p.rect_stroke(
                rect,
                0.0,
                Stroke::new(1.0, crate::theme::Tokens::selected_row_line()),
                egui::epaint::StrokeKind::Inside,
            );
        }
        Look::Layer { active: true } => {
            p.rect_filled(rect, 0.0, crate::theme::Tokens::tool_active());
            p.rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                0.0,
                Tokens::ACCENT,
            );
        }
        _ if resp.hovered() => {
            p.rect_filled(rect, 0.0, crate::theme::Tokens::hovered_row());
        }
        Look::Layer { active: false } => {
            p.rect_filled(rect, 0.0, crate::theme::Tokens::tab_strip());
        }
        _ => {}
    }
    (rect, resp)
}

/// A clickable square of `size` at `x` in the row `rect`.
fn hot(ui: &mut Ui, rect: Rect, x: f32, size: f32, id: egui::Id) -> (Rect, egui::Response) {
    let r = square(rect, x, size);
    (r, ui.interact(r, id, Sense::click()))
}

/// The square of `size` at `x`, centred on the row `rect`.
fn square(rect: Rect, x: f32, size: f32) -> Rect {
    Rect::from_min_size(
        Pos2::new(x, rect.center().y - size / 2.0),
        Vec2::splat(size),
    )
}

/// The docker: the tree above the bar with its buttons.
pub fn objects_docker(app: &mut App, ui: &mut Ui) {
    let content_h = (ui.available_height() - BAR_H - 4.0).max(40.0);
    let out = egui::ScrollArea::vertical()
        .max_height(content_h)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            objects(app, ui);
        });
    // Ctrl+wheel over the list makes the thumbnails larger or smaller, a
    // step a notch on the desktop and in the browser alike (#1406).
    if let Some(steps) =
        crate::ui::bars::ctrl_wheel_steps(ui, out.inner_rect, egui::Id::new("objects_thumb_wheel"))
    {
        zoom_thumbs(app, steps);
    }
    bottom_bar(app, ui);
}

/// The thumbnails `steps` steps larger (smaller when negative).
fn zoom_thumbs(app: &mut App, steps: i32) {
    if steps == 0 {
        return;
    }
    let steps = steps.clamp(-4, 4) as f32;
    app.settings.object_thumb_size = crate::object_thumbs::clamp_size(
        app.settings.object_thumb_size + steps * crate::object_thumbs::STEP,
    );
}

/// The object whose row the list scrolls to this frame: the first
/// selected one, when the selection is not the one the list last showed
/// (a pick on the page, a Select All, an undo), so a selection far down a
/// long list comes into view (#1044).
fn scroll_target(app: &App, ui: &Ui) -> Option<ShapeId> {
    let seen = egui::Id::new("objects_seen_selection");
    let changed = ui.data_mut(|d| {
        let last = d.get_temp::<Vec<ShapeId>>(seen);
        d.insert_temp(seen, app.selection.clone());
        last.as_ref() != Some(&app.selection)
    });
    changed.then(|| app.selection.first().copied()).flatten()
}

/// The tree: the pages and their layers (all of them, or the current
/// one), then the Master Page.
pub fn objects(app: &mut App, ui: &mut Ui) {
    let mut act = Actions {
        scroll_to: scroll_target(app, ui),
        thumbs: std::mem::take(&mut app.object_thumbs),
        thumb_size: crate::object_thumbs::clamp_size(app.settings.object_thumb_size),
        drawing: (app.engine.instance(), app.engine.revision()),
        ..Default::default()
    };
    act.thumbs.begin_frame();
    {
        let doc = app.doc();
        if doc.page(app.page).is_err() {
            return;
        }
        let index = doc.pages.iter().position(|p| p.id == app.page).unwrap_or(0);
        let active = app.active_layer();
        ui.spacing_mut().item_spacing.y = 0.0;
        let all = app.settings.objects_all_pages;
        for page in doc.pages.iter().filter(|p| all || p.id == app.page) {
            let current = page.id == app.page;
            let page_open = !app.objects_closed_pages.contains(&page.id);
            let (toggle, resp) = node_row(
                ui,
                ("objects_page", page.id.raw()),
                &page.name,
                page_open,
                false,
                current || !all,
            );
            if toggle {
                act.toggle_page = Some(page.id);
            }
            // Double-click (or the context menu) renames the page.
            if resp.double_clicked() {
                act.rename_page = Some(page.name.clone());
                if !current {
                    act.goto = Some(page.id);
                }
            }
            resp.context_menu(|ui| {
                if !current && ui.button(tr("docker.go_to_page")).clicked() {
                    act.goto = Some(page.id);
                    ui.close();
                }
                if ui.button(tr("docker.rename")).clicked() {
                    act.rename_page = Some(page.name.clone());
                    if !current {
                        act.goto = Some(page.id);
                    }
                    ui.close();
                }
            });
            if page_open {
                act.row_page = Some(page.id);
                let count = page.layers.len();
                for (i, layer) in page.layers.iter().enumerate().rev() {
                    let place = Place {
                        index: i,
                        count,
                        master: false,
                        shown: true,
                        page: Some(page.id),
                    };
                    layer_rows(app, ui, layer, place, active, &mut act);
                }
                // The page's own guidelines (#1928).
                guides_row(app, ui, &page.guides, Some(page.id), &mut act);
            }
        }
        act.row_page = None;
        let master_open = !app.objects_master_closed;
        act.toggle_master = node_row(
            ui,
            "objects_master",
            &tr("docker.master_page"),
            master_open,
            true,
            true,
        )
        .0;
        if master_open {
            let count = doc.master.len();
            for (i, layer) in doc.master.iter().enumerate().rev() {
                let place = Place {
                    index: i,
                    count,
                    master: true,
                    shown: layer.scope.applies(index),
                    page: None,
                };
                layer_rows(app, ui, layer, place, active, &mut act);
            }
            guides_row(app, ui, &doc.master_guides, None, &mut act);
            grid_row(app, ui, &mut act);
        }
    }
    let mut thumbs = std::mem::take(&mut act.thumbs);
    thumbs.trim(crate::object_thumbs::MAX_BYTES);
    if thumbs.waiting {
        // New pictures wait for the next frames.
        ui.ctx().request_repaint();
    }
    app.object_thumbs = thumbs;
    apply(app, ui, act);
}

/// The page or Master Page node: an arrow, the page icon and the title in
/// bold. True when clicked (to open or close it), and the row's response.
fn node_row(
    ui: &mut Ui,
    id: impl std::hash::Hash + Copy + std::fmt::Debug,
    title: &str,
    open: bool,
    master: bool,
    bold: bool,
) -> (bool, egui::Response) {
    let (rect, resp) = row(ui, Look::Node);
    let resp = resp.rich_tip_with(|| format!("{title}\n{}", tr("docker.page_row_desc")));
    let x = rect.left() + 3.0;
    let (ar, a) = hot(ui, rect, x, 14.0, egui::Id::new((id, "arrow")));
    let a = a.rich_tip_with(|| crate::ui::tips::keyed("docker.expand_row"));
    paint_arrow(ui, ar, open);
    paint_page_icon(ui.painter(), square(rect, x + 16.0, 16.0), master);
    // The current page in bold among the others.
    ui.painter().text(
        Pos2::new(x + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        if bold {
            crate::theme::bold(13.0)
        } else {
            egui::FontId::proportional(13.0)
        },
        Tokens::text(),
    );
    // A double-click is two clicks, so renaming closes and reopens the
    // node and leaves it as it was.
    (a.clicked() || resp.clicked(), resp)
}

/// A layer's row, then its objects when it is open.
fn layer_rows(
    app: &App,
    ui: &mut Ui,
    layer: &Layer,
    at: Place,
    active: Option<LayerId>,
    act: &mut Actions,
) {
    let open = !app.objects_collapsed.contains(&layer.id);
    let is_active = active == Some(layer.id);
    let (rect, resp) = row(ui, Look::Layer { active: is_active });
    let resp = resp.rich_tip_with(|| format!("{}\n{}", layer.name, tr("docker.layer_row_desc")));
    let base = egui::Id::new(("objects_layer", layer.id.raw()));
    let mut x = rect.left() + 3.0 + INDENT;
    let (ar, a) = hot(ui, rect, x, 14.0, base.with("arrow"));
    let a = a.rich_tip_with(|| crate::ui::tips::keyed("docker.expand_row"));
    paint_arrow(ui, ar, open);
    if a.clicked() {
        act.collapse = Some(layer.id);
    }
    x += 16.0;
    let (er, e) = hot(ui, rect, x, 18.0, base.with("eye"));
    paint_eye(ui, er, layer.visible);
    if e.rich_tip(tr("docker.show_hide")).clicked() {
        act.visible = Some((layer.id, !layer.visible));
    }
    x += 20.0;
    let (pr, p) = hot(ui, rect, x, 18.0, base.with("print"));
    paint_printer(ui, pr, layer.printable);
    if p.rich_tip(tr("docker.print_layer")).clicked() {
        act.printable = Some((layer.id, !layer.printable));
    }
    x += 20.0;
    let (lr, l) = hot(ui, rect, x, 18.0, base.with("lock"));
    paint_lock(ui, lr, layer.locked);
    if l.rich_tip(tr("docker.lock_layer")).clicked() {
        act.locked = Some((layer.id, !layer.locked));
    }
    x += 22.0;
    paint_layer_icon(ui.painter(), square(rect, x, 16.0), at.master);
    x += 22.0;
    let font = if is_active {
        crate::theme::bold(13.0)
    } else {
        egui::FontId::proportional(13.0)
    };
    let color = if at.shown {
        Tokens::text()
    } else {
        Tokens::text_dim()
    };
    let name = ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &layer.name,
        font,
        color,
    );
    // A master layer's pages and, when it is drawn over the page's own
    // layers, "In front" (it is listed under them all the same, #1132).
    let mut notes = Vec::new();
    if at.master && layer.scope != MasterScope::All {
        notes.push(tr(scope_key(layer.scope)));
    }
    if at.master && layer.in_front {
        notes.push(tr("docker.in_front"));
    }
    if !notes.is_empty() {
        ui.painter().text(
            Pos2::new(name.right() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            notes.join(", "),
            egui::FontId::proportional(11.0),
            Tokens::text_dim(),
        );
    }
    // An object row let go here goes to the top of this layer.
    if !layer.locked {
        act.layer_drop_rows.push((layer.id, rect));
    }
    // A layer of another page: that page comes first.
    let elsewhere = at.page.filter(|p| *p != app.page);
    if resp.double_clicked() {
        act.rename_layer = Some((layer.id, layer.name.clone()));
    } else if resp.clicked() {
        act.goto = elsewhere;
        act.pick = Some(layer.id);
    }
    resp.context_menu(|ui| {
        if ui.button(tr("docker.rename")).clicked() {
            act.rename_layer = Some((layer.id, layer.name.clone()));
            ui.close();
        }
        // A new layer beside this one: on its page, or a master layer
        // showing on the same pages.
        let new_key = if at.master {
            "docker.new_master_layer_short"
        } else {
            "docker.new_layer"
        };
        if ui.button(tr(new_key)).clicked() {
            if at.master {
                act.new_master = Some(layer.scope);
            } else {
                act.goto = elsewhere;
                act.new_layer = true;
            }
            ui.close();
        }
        if ui
            .add_enabled(
                !layer.shapes.is_empty(),
                egui::Button::new(tr("docker.select_layer_objects")),
            )
            .clicked()
        {
            act.goto = elsewhere;
            act.select_layer = Some(layer.id);
            ui.close();
        }
        // The layer's objects to the clipboard, or the clipboard's onto
        // the layer (#1992).
        ui.separator();
        let has = !layer.shapes.is_empty();
        let can_paste = app.clipboard.is_some() && !layer.locked;
        for (what, key, on) in [
            (LayerClip::Cut, "menu.edit.cut", has && !layer.locked),
            (LayerClip::Copy, "menu.edit.copy", has),
            (LayerClip::Paste, "menu.edit.paste", can_paste),
        ] {
            if ui.add_enabled(on, egui::Button::new(tr(key))).clicked() {
                act.goto = elsewhere;
                act.layer_clip = Some((layer.id, what));
                ui.close();
            }
        }
        ui.separator();
        let mut visible = layer.visible;
        if ui.checkbox(&mut visible, tr("docker.show_hide")).clicked() {
            act.visible = Some((layer.id, visible));
            ui.close();
        }
        let mut printable = layer.printable;
        if ui
            .checkbox(&mut printable, tr("docker.print_layer"))
            .clicked()
        {
            act.printable = Some((layer.id, printable));
            ui.close();
        }
        let mut locked = layer.locked;
        if ui.checkbox(&mut locked, tr("docker.lock_layer")).clicked() {
            act.locked = Some((layer.id, locked));
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                at.index + 1 < at.count,
                egui::Button::new(tr("docker.move_up")),
            )
            .clicked()
        {
            act.reorder = Some((layer.id, at.index + 1));
            ui.close();
        }
        if ui
            .add_enabled(at.index > 0, egui::Button::new(tr("docker.move_down")))
            .clicked()
        {
            act.reorder = Some((layer.id, at.index.saturating_sub(1)));
            ui.close();
        }
        if at.master {
            ui.menu_button(tr("docker.show_on"), |ui| {
                for scope in [MasterScope::All, MasterScope::Odd, MasterScope::Even] {
                    if ui
                        .selectable_label(layer.scope == scope, tr(scope_key(scope)))
                        .clicked()
                    {
                        act.scope = Some((layer.id, scope));
                        ui.close();
                    }
                }
            });
            let mut front = layer.in_front;
            if ui
                .checkbox(&mut front, tr("docker.master_in_front"))
                .clicked()
            {
                act.in_front = Some((layer.id, front));
                ui.close();
            }
            // Its objects become a layer of the page shown (#1477).
            if ui.button(tr("docker.master_to_page")).clicked() {
                act.master_to_page = Some(layer.id);
                ui.close();
            }
        } else if ui.button(tr("docker.layer_to_master")).clicked() {
            act.layer_to_master = Some(layer.id);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(
                at.master || at.count > 1,
                egui::Button::new(tr("docker.delete_layer")),
            )
            .clicked()
        {
            act.delete_layer = Some(layer.id);
            ui.close();
        }
    });
    if open {
        for s in layer.shapes.iter().rev() {
            object_rows(app, ui, s, LAYER_OBJECT_DEPTH, act);
        }
    }
}

/// Opens the context menu of the object `id` from its Objects docker
/// row at `pos`: the menu the drawing shows for it (#2601). A locked
/// object gets the drawing's locked-object menu; another object is
/// selected first, unless it is part of the selection already (then the
/// menu is for the whole selection, as on the drawing).
pub(crate) fn open_row_menu(app: &mut App, id: ShapeId, pos: Pos2) {
    let Some(s) = app.doc().find_shape(id) else {
        return;
    };
    let (locked, centre) = (s.locked, s.bounds().center());
    app.context_locked = None;
    app.tab_ruler_menu = None;
    if locked {
        app.context_locked = Some(id);
    } else if !app.selection.contains(&id) {
        app.select(vec![id]);
    }
    app.context_docker = Some(id);
    app.context_menu = Some((pos, centre));
}

/// Every layer an object can go to: each page's layers, then the master
/// layers (page name or none for the master page, layer id and name).
fn layer_targets(app: &App) -> Vec<(Option<String>, LayerId, String)> {
    let doc = app.doc();
    let mut out = Vec::new();
    for page in &doc.pages {
        for l in page.layers.iter().rev() {
            out.push((Some(page.name.clone()), l.id, l.name.clone()));
        }
    }
    for l in doc.master.iter().rev() {
        out.push((None, l.id, l.name.clone()));
    }
    out
}

/// The Objects docker's own commands at the top of an object row's
/// context menu (#2601): Rename, and Move to Layer for the selection
/// (or the object when it is not selected).
pub(crate) fn row_menu_items(app: &mut App, ui: &mut Ui, id: ShapeId, close: &mut bool) {
    use crate::ui::menus::{item, item_label, sub};
    let Some(name) = app.doc().find_shape(id).map(object_label) else {
        return;
    };
    let ids: Vec<ShapeId> = if app.selection.contains(&id) {
        app.selection.clone()
    } else {
        vec![id]
    };
    if item(ui, "docker.rename", "", true) {
        *close = true;
        app.dialog = crate::ui::dialogs::Dialog::RenameObject { shape: id, name };
    }
    let own = layer_holding(app, id);
    let targets = layer_targets(app);
    let mut to = None;
    sub(ui, "docker.move_to_layer", |ui| {
        let mut heading: Option<Option<String>> = None;
        for (page, layer, name) in &targets {
            if heading.as_ref() != Some(page) {
                if heading.is_some() {
                    crate::ui::menus::sep(ui);
                }
                ui.label(
                    egui::RichText::new(page.clone().unwrap_or_else(|| tr("docker.master_page")))
                        .strong(),
                );
                heading = Some(page.clone());
            }
            if item_label(ui, name, "", own != Some(*layer)) {
                to = Some(*layer);
            }
        }
    });
    if let Some(layer) = to {
        *close = true;
        move_to_layer(app, &ids, layer);
    }
}

/// The text naming the pages a master layer shows on.
fn scope_key(scope: MasterScope) -> &'static str {
    match scope {
        MasterScope::All => "options.scope_all",
        MasterScope::Odd => "options.scope_odd",
        MasterScope::Even => "options.scope_even",
    }
}

/// A Guides row: under the Master Page the guidelines every page shows
/// ("Guides (all pages)"), under a page (`page`) that page's own
/// (#1928). A click on the row makes them the guidelines edited (new
/// ones go there too); the eye shows or hides the row's guidelines
/// (#1775; with guidelines off, showing them turns them on), the
/// padlock locks or unlocks all of the row's; guidelines never print.
fn guides_row(
    app: &App,
    ui: &mut Ui,
    guides: &[tracedraw_core::document::Guide],
    page: Option<PageId>,
    act: &mut Actions,
) {
    let master = page.is_none();
    let chosen = app.edit_master_guides == master && page.is_none_or(|p| p == app.page);
    let (rect, resp) = row(ui, Look::Layer { active: chosen });
    let desc = if master {
        "docker.guides_master_row_desc"
    } else {
        "docker.guides_row_desc"
    };
    let label = if master {
        tr("docker.guides_all_pages")
    } else {
        tr("docker.guides")
    };
    let resp = resp.rich_tip_with(|| format!("{label}\n{}", tr(desc)));
    if resp.clicked() {
        act.guides_master = Some(master);
        if let Some(p) = page.filter(|p| *p != app.page) {
            act.goto = Some(p);
        }
    }
    let base = match page {
        Some(p) => egui::Id::new(("objects_guides", p.raw())),
        None => egui::Id::new("objects_guides"),
    };
    let mut x = rect.left() + 3.0 + INDENT + 16.0;
    // Each guidelines row has its own eye (#1775).
    let shown = app.show_guides && app.doc().guides_shown(page);
    let (er, e) = hot(ui, rect, x, 18.0, base.with("eye"));
    paint_eye(ui, er, shown);
    if e.rich_tip(tr("docker.show_hide")).clicked() {
        act.toggle_guides = Some((page, !shown));
    }
    x += 20.0;
    paint_printer(ui, square(rect, x, 18.0), false);
    x += 20.0;
    let any = !guides.is_empty();
    let locked = any && guides.iter().all(|g| g.locked);
    let (lr, l) = hot(ui, rect, x, 18.0, base.with("lock"));
    paint_lock(ui, lr, locked);
    if l.rich_tip(tr("docker.lock_guides")).clicked() && any {
        act.lock_guides = Some((master, !locked));
        if let Some(p) = page.filter(|p| *p != app.page) {
            act.goto = Some(p);
        }
    }
    x += 22.0;
    paint_guides_icon(ui.painter(), square(rect, x, 16.0));
    x += 22.0;
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        Tokens::text(),
    );
}

/// The Document Grid row of the Master Page: the eye shows or hides the
/// grid, which never prints and cannot be edited.
fn grid_row(app: &App, ui: &mut Ui, act: &mut Actions) {
    let (rect, resp) = row(ui, Look::Layer { active: false });
    resp.rich_tip_with(|| crate::ui::tips::pair("menu.view.document_grid", "docker.grid_row_desc"));
    let mut x = rect.left() + 3.0 + INDENT + 16.0;
    let (er, e) = hot(ui, rect, x, 18.0, egui::Id::new("objects_grid_eye"));
    paint_eye(ui, er, app.show_grid);
    if e.rich_tip(tr("docker.show_hide")).clicked() {
        act.toggle_grid = true;
    }
    x += 20.0;
    paint_printer(ui, square(rect, x, 18.0), false);
    x += 20.0;
    paint_padlock(
        ui.painter(),
        square(rect, x, 18.0),
        true,
        Tokens::icon_off(),
    );
    x += 22.0;
    paint_grid_icon(ui.painter(), square(rect, x, 16.0));
    x += 22.0;
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        tr("menu.view.document_grid"),
        egui::FontId::proportional(13.0),
        Tokens::text(),
    );
}

/// The row of `s` at nesting `depth`, then its children when it is an
/// open group.
fn object_rows(app: &App, ui: &mut Ui, s: &Shape, depth: usize, act: &mut Actions) {
    let selected = app.selection.contains(&s.id);
    act.row_order.push(s.id);
    let height = (act.thumb_size + 6.0).max(ROW_H);
    let (rect, resp) = row_of(ui, Look::Object { selected }, height);
    if act.scroll_to == Some(s.id) {
        ui.scroll_to_rect(rect, None);
    }
    let children: Option<&Vec<Shape>> = match &s.kind {
        ShapeKind::Group { children } => Some(children),
        _ => None,
    };
    let expanded = app.objects_open_groups.contains(&s.id);
    // Rows scrolled out of sight are only laid out: nothing is drawn or
    // asked of them, so a long list costs little.
    if ui.is_rect_visible(rect) {
        object_row(app, ui, s, (rect, resp), depth, expanded, act);
    }
    if let (Some(children), true) = (children, expanded) {
        for c in children.iter().rev() {
            object_rows(app, ui, c, depth + 1, act);
        }
    }
}

/// The contents of the row of `s`: the group arrow, the thumbnail, the
/// name, the eye and the padlock.
fn object_row(
    app: &App,
    ui: &mut Ui,
    s: &Shape,
    (rect, resp): (Rect, egui::Response),
    depth: usize,
    expanded: bool,
    act: &mut Actions,
) {
    let mut x = rect.left() + 3.0 + depth as f32 * INDENT;
    if matches!(s.kind, ShapeKind::Group { .. }) {
        let (ar, a) = hot(
            ui,
            rect,
            x,
            14.0,
            egui::Id::new(("objects_group", s.id.raw())),
        );
        let a = a.rich_tip_with(|| crate::ui::tips::keyed("docker.expand_row"));
        paint_arrow(ui, ar, expanded);
        if a.clicked() {
            act.expand_group = Some(s.id);
        }
    }
    x += 16.0;
    let size = act.thumb_size;
    paint_thumb(
        app,
        ui,
        Rect::from_min_size(
            Pos2::new(x, rect.center().y - size / 2.0),
            Vec2::splat(size),
        ),
        s,
        act,
    );
    x += size + 7.0;
    let label = object_label(s);
    let resp = resp.rich_tip_with(|| format!("{label}\n{}", tr("docker.object_row_desc")));
    let color = if s.visible {
        Tokens::text()
    } else {
        Tokens::text_dim()
    };
    // The name stops short of the eye and padlock.
    let name_room = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 48.0, rect.max.y));
    ui.painter().with_clip_rect(name_room).text(
        Pos2::new(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &label,
        egui::FontId::proportional(13.0),
        color,
    );
    // Its own eye and padlock at the right end of the row, so an object
    // is hidden or locked from the list, and a locked one unlocked (#832).
    let base = egui::Id::new(("objects_object", s.id.raw()));
    let lock_x = rect.right() - 22.0;
    let eye_x = lock_x - 20.0;
    let (er, e) = hot(ui, rect, eye_x, 18.0, base.with("eye"));
    paint_eye(ui, er, s.visible);
    if e.rich_tip(tr("docker.show_hide")).clicked() {
        act.object_visible = Some((s.id, !s.visible));
    }
    let (lr, l) = hot(ui, rect, lock_x, 18.0, base.with("lock"));
    paint_lock(ui, lr, s.locked);
    let lock_tip = if s.locked {
        "menu.object.unlock_object"
    } else {
        "menu.object.lock_object"
    };
    if l.rich_tip(tr(lock_tip)).clicked() {
        act.object_locked = Some((s.id, !s.locked));
    }
    // A row dragged along the list moves its object (with the rest of
    // the selection when it is selected) in the stacking order where it
    // is let go (see `drag_rows`). Only a layer's own objects move so: a
    // group's objects keep their place in it.
    if depth == LAYER_OBJECT_DEPTH {
        act.object_rows.push((s.id, rect));
        if resp.drag_started() && !s.locked {
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new(ROW_DRAG), s.id));
        }
    }
    if resp.double_clicked() {
        act.rename_object = Some((s.id, s.name.clone().unwrap_or_else(|| label.clone())));
    } else if resp.clicked() {
        act.goto = act.row_page.filter(|p| *p != app.page);
        act.select = Some(s.id);
    }
    // A right click opens the drawing's own object menu for it (#2601).
    if resp.secondary_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            act.goto = act.goto.or(act.row_page.filter(|p| *p != app.page));
            act.object_menu = Some((s.id, pos));
        }
    }
}

/// The thumbnail of `s` in the square `r`: the object drawn small on a
/// white tile, centred; dimmed when the object is hidden. Until its
/// picture is drawn (a few per frame), the swatch of its fill and outline
/// stands in.
fn paint_thumb(app: &App, ui: &Ui, r: Rect, s: &Shape, act: &mut Actions) {
    let p = ui.painter();
    p.rect_filled(r, 2.0, Color32::WHITE);
    p.rect_stroke(
        r,
        2.0,
        Stroke::new(1.0, Tokens::border()),
        egui::epaint::StrokeKind::Inside,
    );
    let ppp = ui.ctx().pixels_per_point().max(0.5);
    let inner = r.shrink(2.0);
    let px = (inner.width() * ppp).round().max(4.0) as u32;
    match act.thumbs.get(ui.ctx(), app.doc(), act.drawing, s, px) {
        crate::object_thumbs::Thumb::Picture(id, [w, h]) => {
            let size = Vec2::new(w as f32, h as f32) / ppp;
            // An older picture of another size still fits the tile.
            let fit = (inner.width() / size.x.max(size.y).max(1.0)).min(1.0);
            let at = Rect::from_center_size(inner.center(), size * fit);
            let tint = if s.visible {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(90)
            };
            p.image(
                id,
                at,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        crate::object_thumbs::Thumb::Empty => {}
        crate::object_thumbs::Thumb::Pending => {
            let side = (inner.width() * 0.6).max(8.0);
            paint_swatch(
                ui,
                Rect::from_center_size(inner.center(), Vec2::splat(side)),
                s,
            );
        }
    }
}

/// The bar under the tree: New Layer and New Master Layer (all pages; its
/// context menu offers odd or even pages) at the left, Delete at the right.
fn bottom_bar(app: &mut App, ui: &mut Ui) {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BAR_H), Sense::hover());
    ui.painter()
        .hline(bar.x_range(), bar.min.y, Stroke::new(1.0, Tokens::border()));
    let slot =
        |x: f32| Rect::from_center_size(Pos2::new(x, bar.center().y + 1.0), Vec2::splat(24.0));
    let new_r = slot(bar.min.x + 14.0);
    let master_r = slot(bar.min.x + 42.0);
    let pages_r = slot(bar.min.x + 70.0);
    let delete_r = slot(bar.max.x - 14.0);
    let can_delete = !app.selection.is_empty() || deletable_layer(app).is_some();
    let new = ui
        .interact(new_r, egui::Id::new("objects_new_layer"), Sense::click())
        .rich_tip(tr("docker.new_layer"));
    let master = ui
        .interact(
            master_r,
            egui::Id::new("objects_new_master"),
            Sense::click(),
        )
        .rich_tip(tr("docker.new_master_layer"));
    let all_pages = app.settings.objects_all_pages;
    let pages = ui
        .interact(pages_r, egui::Id::new("objects_all_pages"), Sense::click())
        .rich_tip(tr("docker.all_pages"));
    let delete = ui
        .interact(
            delete_r,
            egui::Id::new("objects_delete"),
            if can_delete {
                Sense::click()
            } else {
                Sense::hover()
            },
        )
        .rich_tip(tr("docker.delete_hint"));
    let p = ui.painter();
    for (r, resp, on) in [
        (new_r, &new, true),
        (master_r, &master, true),
        (pages_r, &pages, true),
        (delete_r, &delete, can_delete),
    ] {
        if on && resp.hovered() {
            p.rect_filled(r, 2.0, Tokens::tool_hover());
        }
    }
    // Show all pages: on like a pressed tool button.
    if all_pages {
        p.rect_filled(pages_r, 2.0, Tokens::tool_active());
    }
    let sheet =
        |dx: f32| Rect::from_center_size(pages_r.center() + Vec2::new(dx, dx), Vec2::splat(13.0));
    paint_page_icon(p, sheet(-2.5), false);
    paint_page_icon(p, sheet(2.5), false);
    let icon = |r: Rect| Rect::from_center_size(r.center(), Vec2::splat(16.0));
    paint_layer_icon(p, icon(new_r), false);
    paint_plus_badge(p, icon(new_r));
    paint_layer_icon(p, icon(master_r), true);
    paint_plus_badge(p, icon(master_r));
    paint_bin(p, icon(delete_r), can_delete);
    let mut scoped = None;
    master.context_menu(|ui| {
        for (scope, key) in [
            (MasterScope::All, "options.new_master_all"),
            (MasterScope::Odd, "options.new_master_odd"),
            (MasterScope::Even, "options.new_master_even"),
        ] {
            if ui.button(tr(key)).clicked() {
                scoped = Some(scope);
                ui.close();
            }
        }
        // Which kinds of master layer are drawn over the page's own
        // layers: the ones the drawing has and the new ones (#1132).
        ui.separator();
        master_front_choices(app, ui);
    });
    if new.clicked() {
        new_layer(app);
    }
    if master.clicked() {
        new_master_layer(app, MasterScope::All);
    }
    if pages.clicked() {
        app.settings.objects_all_pages = !all_pages;
    }
    if let Some(scope) = scoped {
        new_master_layer(app, scope);
    }
    if can_delete && delete.clicked() {
        delete_current(app);
    }
    // The thumbnail size between the buttons, when there is room.
    let zoom = Rect::from_min_max(
        Pos2::new(pages_r.right() + 10.0, bar.min.y + 2.0),
        Pos2::new(delete_r.left() - 10.0, bar.max.y),
    );
    if zoom.width() >= 70.0 {
        thumb_size_control(app, ui, zoom);
    }
}

/// The thumbnail size control in `r`: a small tile, a slider and a large
/// tile; the wheel over it changes the size too.
fn thumb_size_control(app: &mut App, ui: &mut Ui, r: Rect) {
    use crate::object_thumbs::{clamp_size, MAX_SIZE, MIN_SIZE, STEP};
    let small = Rect::from_center_size(Pos2::new(r.min.x + 4.0, r.center().y), Vec2::splat(7.0));
    let large = Rect::from_center_size(Pos2::new(r.max.x - 7.0, r.center().y), Vec2::splat(13.0));
    for t in [small, large] {
        ui.painter().rect_stroke(
            t,
            1.0,
            Stroke::new(1.0, Tokens::icon()),
            egui::epaint::StrokeKind::Inside,
        );
    }
    let groove = Rect::from_min_max(
        Pos2::new(small.right() + 6.0, r.min.y),
        Pos2::new(large.left() - 6.0, r.max.y),
    );
    let mut size = clamp_size(app.settings.object_thumb_size);
    let slider = ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(groove)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().slider_width = groove.width();
            ui.add(
                crate::ui::Slider::new(&mut size, MIN_SIZE..=MAX_SIZE)
                    .step_by(f64::from(STEP))
                    .show_value(false),
            )
            .rich_tip(tr("docker.thumb_size"))
            .rect
        },
    );
    app.settings.object_thumb_size = clamp_size(size);
    // The slider steps with the wheel over itself (#1335); the rest of
    // the control (the tiles, around the groove) steps here, once a notch.
    if ui.rect_contains_pointer(r) && !ui.rect_contains_pointer(slider.inner) {
        if let Some(steps) = crate::ui::field::wheel_steps(ui, egui::Id::new("objects_bar_wheel")) {
            zoom_thumbs(app, steps);
        }
    }
}

/// `key`'s name with the first number from `from` up that `names` does
/// not use yet.
fn free_name(names: &[String], key: &str, from: usize) -> String {
    (from..)
        .map(|n| trf(key, &[("n", &n.to_string())]))
        .find(|name| !names.contains(name))
        .unwrap_or_default()
}

/// Add a layer on top of the current page's and make it active.
pub fn new_layer(app: &mut App) {
    let page = app.page;
    let names: Vec<String> = app
        .doc()
        .page(page)
        .map(|p| p.layers.iter().map(|l| l.name.clone()).collect())
        .unwrap_or_default();
    let name = free_name(&names, "docker.layer_n", names.len() + 1);
    app.run(Command::AddLayer { page, name });
    app.picked_layer = app
        .doc()
        .page(page)
        .ok()
        .and_then(|p| p.layers.last())
        .map(|l| l.id);
    app.objects_closed_pages.remove(&page);
}

/// Add a master layer showing on the pages `scope` names and make it
/// active.
pub fn new_master_layer(app: &mut App, scope: MasterScope) {
    // The desktop layer is not one of the numbered master layers.
    let names: Vec<String> = app
        .doc()
        .master
        .iter()
        .filter(|l| !l.desktop)
        .map(|l| l.name.clone())
        .collect();
    let name = free_name(&names, "docker.master_layer_n", names.len() + 1);
    app.run(Command::AddMasterLayer { name, scope });
    app.picked_layer = app.doc().master.last().map(|l| l.id);
    app.objects_master_closed = false;
    // In front of the page's layers when the preferences say so for
    // master layers of these pages (#1132, #1477): one undo step.
    if master_front_default(app, scope) {
        if let Some(layer) = app.picked_layer {
            if let Err(e) = app.engine.amend(&Command::SetLayerInFront {
                layer,
                in_front: true,
            }) {
                app.status = e.to_string();
            }
        }
    }
}

/// Which preference holds the in-front choice of master layers showing
/// on `scope`'s pages.
fn scope_slot(scope: MasterScope) -> usize {
    match scope {
        MasterScope::All => 0,
        MasterScope::Odd => 1,
        MasterScope::Even => 2,
    }
}

/// New master layers of `scope` are drawn in front of the page's layers.
pub fn master_front_default(app: &App, scope: MasterScope) -> bool {
    app.settings.master_in_front[scope_slot(scope)]
}

/// Master layers showing on `scope`'s pages in front of the page's own
/// layers (`front`) or behind them: the master layers of that kind the
/// drawing has, in one undo step, and the ones made from now on (the
/// choice is kept in the preferences, #1132, #1477).
pub fn set_master_front(app: &mut App, scope: MasterScope, front: bool) {
    app.settings.master_in_front[scope_slot(scope)] = front;
    let cmds: Vec<Command> = app
        .doc()
        .master
        .iter()
        .filter(|l| l.scope == scope && l.in_front != front && !l.desktop)
        .map(|l| Command::SetLayerInFront {
            layer: l.id,
            in_front: front,
        })
        .collect();
    if !cmds.is_empty() {
        if let Err(e) = app.engine.run_batch("Master Layer Order", &cmds) {
            app.status = e.to_string();
        }
    }
}

/// The three in-front choices, one per kind of master layer, as
/// checkboxes (the New Master Layer menu and Options > Page).
pub fn master_front_choices(app: &mut App, ui: &mut Ui) {
    ui.label(egui::RichText::new(tr("docker.master_in_front")).strong());
    for (scope, key) in [
        (MasterScope::All, "options.scope_all"),
        (MasterScope::Odd, "options.scope_odd"),
        (MasterScope::Even, "options.scope_even"),
    ] {
        let mut on = master_front_default(app, scope);
        if ui
            .checkbox(&mut on, tr(key))
            .rich_tip_with(|| {
                format!(
                    "{}\n{}",
                    tr("docker.master_in_front"),
                    tr(&format!("{key}_tip"))
                )
            })
            .clicked()
        {
            set_master_front(app, scope, on);
        }
    }
}

/// What a layer's menu does with the clipboard (#1992).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LayerClip {
    Cut,
    Copy,
    Paste,
}

/// Select the objects of `layer` that can be (shown and not locked).
fn select_layer_objects(app: &mut App, layer: LayerId) {
    let ids: Vec<ShapeId> = app
        .doc()
        .all_layers()
        .find(|l| l.id == layer)
        .map(|l| {
            l.shapes
                .iter()
                .filter(|s| s.visible && !s.locked)
                .map(|s| s.id)
                .collect()
        })
        .unwrap_or_default();
    app.picked_layer = Some(layer);
    app.select(ids);
}

/// The commands that move the top-level objects `ids` to the top of
/// `layer`, keeping their order (bottom first) and their place on the
/// page; objects already there stay.
fn move_commands(app: &App, ids: &[ShapeId], layer: LayerId) -> Vec<Command> {
    let doc = app.doc();
    let mut found: Vec<(usize, usize, Shape)> = Vec::new();
    for (rank, l) in doc.all_layers().enumerate() {
        if l.id == layer {
            continue;
        }
        for (i, s) in l.shapes.iter().enumerate() {
            if ids.contains(&s.id) {
                found.push((rank, i, s.clone()));
            }
        }
    }
    if found.is_empty() {
        return Vec::new();
    }
    found.sort_by_key(|(rank, i, _)| (*rank, *i));
    let mut cmds = vec![Command::DeleteShapes {
        shapes: found.iter().map(|(_, _, s)| s.id).collect(),
    }];
    cmds.extend(
        found
            .into_iter()
            .map(|(_, _, shape)| Command::AddShape { layer, shape }),
    );
    cmds
}

/// Move the objects `ids` to the top of `layer` (a layer of any page or
/// of the master page), one undo step; they stay selected, on the page
/// they now are on (#1477).
pub fn move_to_layer(app: &mut App, ids: &[ShapeId], layer: LayerId) {
    let cmds = move_commands(app, ids, layer);
    if cmds.is_empty() {
        return;
    }
    if let Err(e) = app.engine.run_batch("Move to Layer", &cmds) {
        app.status = e.to_string();
        return;
    }
    let page = app
        .doc()
        .pages
        .iter()
        .position(|p| p.layers.iter().any(|l| l.id == layer));
    if let Some(i) = page {
        if app.doc().pages.get(i).map(|p| p.id) != Some(app.page) {
            app.goto_page(i);
        }
    }
    app.picked_layer = Some(layer);
    let moved: Vec<ShapeId> = ids
        .iter()
        .copied()
        .filter(|id| layer_holding(app, *id) == Some(layer))
        .collect();
    app.select(moved);
}

/// The look of `from` given to the new layer `to`, its objects moved
/// over, and `from` deleted (kept, empty, when it is its page's last).
fn hand_over(app: &mut App, from: &Layer, to: LayerId, keep: bool) {
    let mut cmds = vec![
        Command::SetLayerVisible {
            layer: to,
            visible: from.visible,
        },
        Command::SetLayerLocked {
            layer: to,
            locked: from.locked,
        },
        Command::SetLayerPrintable {
            layer: to,
            printable: from.printable,
        },
    ];
    let ids: Vec<ShapeId> = from.shapes.iter().map(|s| s.id).collect();
    cmds.extend(move_commands(app, &ids, to));
    if !keep {
        cmds.push(Command::DeleteLayer { layer: from.id });
    }
    if let Err(e) = app.engine.amend_batch("Move Layer", &cmds) {
        app.status = e.to_string();
    }
    if app.picked_layer == Some(from.id) || !keep {
        app.picked_layer = Some(to);
    }
    app.selection.clear();
}

/// A page's layer becomes a master layer showing on every page (with its
/// objects, name and look), one undo step (#1477).
fn layer_to_master(app: &mut App, layer: LayerId) {
    let doc = app.doc();
    let Some(page) = doc
        .pages
        .iter()
        .find(|p| p.layers.iter().any(|l| l.id == layer))
    else {
        return;
    };
    let keep = page.layers.len() <= 1;
    let Some(from) = page.layers.iter().find(|l| l.id == layer).cloned() else {
        return;
    };
    let scope = MasterScope::All;
    if let Err(e) = app.engine.run_with_label(
        &Command::AddMasterLayer {
            name: from.name.clone(),
            scope,
        },
        "Move Layer",
    ) {
        app.status = e.to_string();
        return;
    }
    let Some(to) = app.doc().master.last().map(|l| l.id) else {
        return;
    };
    hand_over(app, &from, to, keep);
    if master_front_default(app, scope) {
        let _ = app.engine.amend(&Command::SetLayerInFront {
            layer: to,
            in_front: true,
        });
    }
    app.objects_master_closed = false;
}

/// A master layer becomes a layer on top of the current page (with its
/// objects, name and look), one undo step: what it held shows on that
/// page only (#1477).
fn master_to_page(app: &mut App, layer: LayerId) {
    let Some(from) = app.doc().master.iter().find(|l| l.id == layer).cloned() else {
        return;
    };
    let page = app.page;
    if let Err(e) = app.engine.run_with_label(
        &Command::AddLayer {
            page,
            name: from.name.clone(),
        },
        "Move Layer",
    ) {
        app.status = e.to_string();
        return;
    }
    let Some(to) = app
        .doc()
        .page(page)
        .ok()
        .and_then(|p| p.layers.last())
        .map(|l| l.id)
    else {
        return;
    };
    hand_over(app, &from, to, false);
    app.objects_closed_pages.remove(&page);
}

/// The active layer when Delete can take it: a master layer, or one of
/// several layers of the page (a page keeps its last layer).
fn deletable_layer(app: &App) -> Option<LayerId> {
    let id = app.active_layer()?;
    let doc = app.doc();
    if doc.master.iter().any(|l| l.id == id) {
        return Some(id);
    }
    let page = doc.page(app.page).ok()?;
    (page.layers.len() > 1 && page.layers.iter().any(|l| l.id == id)).then_some(id)
}

/// The bar's Delete: the selected objects, else the active layer.
pub fn delete_current(app: &mut App) {
    if !app.selection.is_empty() {
        app.delete_selection();
    } else if let Some(layer) = deletable_layer(app) {
        delete_layer(app, layer);
    }
}

fn delete_layer(app: &mut App, layer: LayerId) {
    app.selection.clear();
    app.run(Command::DeleteLayer { layer });
    if app.picked_layer == Some(layer) {
        app.picked_layer = None;
    }
}

/// The layer holding `id` (inside groups too).
fn layer_holding(app: &App, id: ShapeId) -> Option<LayerId> {
    let doc = app.doc();
    let top = doc.top_level_of(id)?;
    doc.all_layers()
        .find(|l| l.shapes.iter().any(|s| s.id == top))
        .map(|l| l.id)
}

/// The object row being dragged, kept in the context's memory.
const ROW_DRAG: &str = "objects_row_drag";

/// Where an object row let go goes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RowDrop {
    /// In front of `reference` (`in_front`) or behind it, in its layer.
    Object { reference: ShapeId, in_front: bool },
    /// On the top of a layer.
    Layer(LayerId),
}

/// The objects a dragged row carries, bottom first as they are drawn: the
/// selected objects of the layers when the row's object is selected (a
/// group's objects stay in their group), else that object; locked ones
/// stay where they are.
fn dragged_objects(app: &App, dragged: ShapeId) -> Vec<ShapeId> {
    let with_selection = app.selection.contains(&dragged);
    let mut out = Vec::new();
    // Layer after layer as their objects are kept, each bottom first.
    for l in app.doc().all_layers() {
        for s in &l.shapes {
            let carried = s.id == dragged || (with_selection && app.selection.contains(&s.id));
            if carried && !s.locked {
                out.push(s.id);
            }
        }
    }
    out
}

/// The commands that put `ids` (bottom first) where `drop` says: objects
/// of other layers (of any page or of the master page) go over to the
/// target's layer first, keeping their place on the page. Empty when
/// the target cannot take them (a locked layer, an object among those
/// dragged).
fn drop_commands(app: &App, ids: &[ShapeId], drop: RowDrop) -> Vec<Command> {
    let doc = app.doc();
    let layer = match drop {
        RowDrop::Object { reference, .. } => {
            if ids.is_empty() || ids.contains(&reference) {
                return Vec::new();
            }
            match doc.locate(reference) {
                Ok((l, _)) => l,
                Err(_) => return Vec::new(),
            }
        }
        RowDrop::Layer(l) => l,
    };
    if doc.all_layers().any(|l| l.id == layer && l.locked) {
        return Vec::new();
    }
    let mut cmds = move_commands(app, ids, layer);
    if let RowDrop::Object {
        reference,
        in_front,
    } = drop
    {
        cmds.push(Command::OrderRelative {
            shapes: ids.to_vec(),
            reference,
            in_front,
        });
    }
    cmds
}

/// Put the objects `ids` (bottom first) where `drop` says, one undo
/// step; they stay selected, and their layer becomes the active one, on
/// its page.
fn drop_rows(app: &mut App, ids: &[ShapeId], drop: RowDrop) {
    let layer = match drop {
        RowDrop::Object { reference, .. } => layer_holding(app, reference),
        RowDrop::Layer(l) => Some(l),
    };
    let cmds = drop_commands(app, ids, drop);
    let Some(layer) = layer.filter(|_| !cmds.is_empty()) else {
        return;
    };
    if let [one] = cmds.as_slice() {
        app.run(one.clone());
    } else if let Err(e) = app.engine.run_batch("Move to Layer", &cmds) {
        app.status = e.to_string();
        return;
    }
    let page = app
        .doc()
        .pages
        .iter()
        .position(|p| p.layers.iter().any(|l| l.id == layer));
    if let Some(i) = page {
        if app.doc().pages.get(i).map(|p| p.id) != Some(app.page) {
            app.goto_page(i);
        }
    }
    app.picked_layer = Some(layer);
    app.select(ids.to_vec());
}

/// An object row dragged along the list: a line shows where it goes. Let
/// go over another object's row, the object goes in front of that object
/// when dropped on the row's upper half, behind it on the lower half (the
/// list shows the front object first), in the layer and page of that
/// object; let go over a layer's row, it goes on the top of that layer. A
/// selected row carries the rest of the selection along. One undo step.
fn drag_rows(app: &mut App, ui: &Ui, rows: &[(ShapeId, Rect)], layers: &[(LayerId, Rect)]) {
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        d.insert_temp(egui::Id::new("objects_rows_drawn"), rows.to_vec());
        d.insert_temp(egui::Id::new("objects_layer_rows_drawn"), layers.to_vec());
    });
    let id = egui::Id::new(ROW_DRAG);
    let Some(dragged) = ui.ctx().data(|d| d.get_temp::<ShapeId>(id)) else {
        return;
    };
    let (pos, down, released) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
        )
    });
    let target = pos.and_then(|p| {
        if let Some((sid, r)) = rows.iter().find(|(_, r)| r.contains(p)) {
            if *sid == dragged {
                return None;
            }
            let in_front = p.y < r.center().y;
            let y = if in_front { r.top() } else { r.bottom() };
            let drop = RowDrop::Object {
                reference: *sid,
                in_front,
            };
            return Some((drop, y, *r));
        }
        layers
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|(l, r)| (RowDrop::Layer(*l), r.bottom(), *r))
    });
    if down {
        if let Some((_, y, r)) = target {
            ui.painter().line_segment(
                [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                Stroke::new(2.0, Tokens::ACCENT),
            );
        }
        return;
    }
    // The button is up: the drag ends, and moves the objects only when it
    // was let go over a row this frame (not when the list was away).
    ui.ctx().data_mut(|d| d.remove::<ShapeId>(id));
    if let (true, Some((drop, _, _))) = (released, target) {
        let ids = dragged_objects(app, dragged);
        drop_rows(app, &ids, drop);
    }
}

fn apply(app: &mut App, ui: &Ui, act: Actions) {
    drag_rows(app, ui, &act.object_rows, &act.layer_drop_rows);
    if let Some((layer, visible)) = act.visible {
        app.run(Command::SetLayerVisible { layer, visible });
    }
    if let Some((layer, printable)) = act.printable {
        app.run(Command::SetLayerPrintable { layer, printable });
    }
    if let Some((layer, locked)) = act.locked {
        app.run(Command::SetLayerLocked { layer, locked });
    }
    if let Some((id, visible)) = act.object_visible {
        // The eye of one of several selected objects hides or shows them
        // all, as the clicked one goes, in one undo step; an object out
        // of the selection goes alone (#2571).
        let shapes = if app.selection.len() > 1 && app.selection.contains(&id) {
            app.selection.clone()
        } else {
            vec![id]
        };
        app.run(Command::SetVisible { shapes, visible });
    }
    if let Some((id, locked)) = act.object_locked {
        app.run(Command::SetLocked {
            shapes: vec![id],
            locked,
        });
    }
    if let Some((layer, scope)) = act.scope {
        app.run(Command::SetLayerScope { layer, scope });
    }
    if let Some((layer, in_front)) = act.in_front {
        app.run(Command::SetLayerInFront { layer, in_front });
    }
    if let Some(layer) = act.collapse {
        if !app.objects_collapsed.remove(&layer) {
            app.objects_collapsed.insert(layer);
        }
    }
    if let Some(group) = act.expand_group {
        if !app.objects_open_groups.remove(&group) {
            app.objects_open_groups.insert(group);
        }
    }
    if let Some(page) = act.toggle_page {
        if !app.objects_closed_pages.remove(&page) {
            app.objects_closed_pages.insert(page);
        }
    }
    // A row of another page: its page first (the selection there is new).
    if let Some(page) = act.goto {
        if let Some(i) = app.doc().pages.iter().position(|p| p.id == page) {
            app.goto_page(i);
        }
    }
    if act.toggle_master {
        app.objects_master_closed = !app.objects_master_closed;
    }
    if let Some((page, shown)) = act.toggle_guides {
        app.set_guides_shown(page, shown);
    }
    if act.toggle_grid {
        app.show_grid = !app.show_grid;
    }
    if let Some(master) = act.guides_master {
        app.choose_master_guides(master);
    }
    if let Some((master, locked)) = act.lock_guides {
        app.choose_master_guides(master);
        let all: Vec<usize> = (0..app.page_guides().len()).collect();
        app.set_guides_locked(&all, locked);
    }
    if let Some((layer, name)) = act.rename_layer {
        app.dialog = crate::ui::dialogs::Dialog::RenameLayer { layer, name };
    }
    if let Some(name) = act.rename_page {
        app.dialog = crate::ui::dialogs::Dialog::RenamePage { name };
    }
    if let Some((shape, name)) = act.rename_object {
        app.dialog = crate::ui::dialogs::Dialog::RenameObject { shape, name };
    }
    if let Some(layer) = act.delete_layer {
        delete_layer(app, layer);
    }
    if let Some((layer, index)) = act.reorder {
        app.run(Command::ReorderLayer { layer, index });
    }
    if let Some(layer) = act.pick {
        app.picked_layer = Some(layer);
        app.select(Vec::new());
    }
    if act.new_layer {
        new_layer(app);
    }
    if let Some(scope) = act.new_master {
        new_master_layer(app, scope);
    }
    if let Some(layer) = act.select_layer {
        select_layer_objects(app, layer);
    }
    if let Some((layer, what)) = act.layer_clip {
        match what {
            LayerClip::Cut => {
                select_layer_objects(app, layer);
                app.cut();
            }
            LayerClip::Copy => {
                select_layer_objects(app, layer);
                app.copy();
            }
            LayerClip::Paste => {
                app.picked_layer = Some(layer);
                app.paste();
            }
        }
    }
    if let Some((id, pos)) = act.object_menu {
        open_row_menu(app, id, pos);
    }
    if let Some(layer) = act.layer_to_master {
        layer_to_master(app, layer);
    }
    if let Some(layer) = act.master_to_page {
        master_to_page(app, layer);
    }
    if let Some(id) = act.select {
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        let key = egui::Id::new("objects_docker_anchor");
        let anchor: Option<ShapeId> = ui.data(|d| d.get_temp(key));
        let anchor = select_row(app, id, &act.row_order, anchor, shift, ctrl);
        ui.data_mut(|d| d.insert_temp(key, anchor));
        if let Some(layer) = layer_holding(app, id) {
            app.picked_layer = Some(layer);
        }
    }
}

/// A click on the row of `id` in the list whose rows are `rows` (top to
/// bottom), as in a file list (#2483): alone it selects that object;
/// Shift selects every row from the row last clicked (`anchor`) to this
/// one, Ctrl+Shift adds that range; Ctrl adds the object or takes it out.
/// A locked object stays out of the selection (#2347). Gives the row a
/// next Shift+click counts from.
fn select_row(
    app: &mut App,
    id: ShapeId,
    rows: &[ShapeId],
    anchor: Option<ShapeId>,
    shift: bool,
    ctrl: bool,
) -> ShapeId {
    let locked = app.locked_ids();
    let free = |i: &ShapeId| !locked.contains(i);
    if shift {
        let from = anchor.and_then(|a| rows.iter().position(|r| *r == a));
        let to = rows.iter().position(|r| *r == id);
        let range: Vec<ShapeId> = match (from, to) {
            (Some(a), Some(b)) => rows[a.min(b)..=a.max(b)].to_vec(),
            _ => vec![id],
        };
        let mut picked = if ctrl {
            app.selection.clone()
        } else {
            Vec::new()
        };
        for r in range.into_iter().filter(free) {
            if !picked.contains(&r) {
                picked.push(r);
            }
        }
        app.select(picked);
        // The range keeps counting from where it started.
        return anchor.filter(|a| rows.contains(a)).unwrap_or(id);
    }
    if ctrl {
        let mut picked = app.selection.clone();
        if let Some(i) = picked.iter().position(|r| *r == id) {
            picked.remove(i);
        } else if free(&id) {
            picked.push(id);
        }
        app.select(picked);
    } else {
        app.select(vec![id]);
    }
    id
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// #2483: in the Objects docker Shift+click selects every row from the
    /// row clicked before to this one, as in a file list, and Ctrl+click
    /// adds or takes out one object. Shift+click added the one object
    /// only, leaving the rows between unselected.
    #[test]
    fn shift_click_selects_a_range_of_rows_and_ctrl_click_one_more() {
        let mut app = App::headless();
        let ids: Vec<ShapeId> = (0..5)
            .map(|_| app.new_shape(rect_kind()).expect("shape"))
            .collect();
        // The list shows the top object first.
        let rows: Vec<ShapeId> = ids.iter().rev().copied().collect();
        let a = select_row(&mut app, rows[1], &rows, None, false, false);
        assert_eq!(app.selection, vec![rows[1]]);
        // Shift on the fourth row: the second to the fourth.
        let a = select_row(&mut app, rows[3], &rows, Some(a), true, false);
        assert_eq!(app.selection, rows[1..=3].to_vec());
        assert_eq!(a, rows[1], "the range counts from its first row");
        // Shift upwards from the same start: the first two.
        select_row(&mut app, rows[0], &rows, Some(a), true, false);
        assert_eq!(app.selection, rows[0..=1].to_vec());
        // Ctrl adds the last row alone, and takes it out again.
        let a = select_row(&mut app, rows[4], &rows, Some(a), false, true);
        assert_eq!(app.selection, vec![rows[0], rows[1], rows[4]]);
        select_row(&mut app, rows[4], &rows, Some(a), false, true);
        assert_eq!(app.selection, vec![rows[0], rows[1]]);
        // A locked object in the range stays out of it (#2347).
        app.run(Command::SetLocked {
            shapes: vec![rows[2]],
            locked: true,
        });
        let a = select_row(&mut app, rows[1], &rows, None, false, false);
        select_row(&mut app, rows[3], &rows, Some(a), true, false);
        assert_eq!(app.selection, vec![rows[1], rows[3]]);
        // No row clicked before: Shift selects the one row.
        select_row(&mut app, rows[4], &rows, None, true, false);
        assert_eq!(app.selection, vec![rows[4]]);
    }

    /// A new drawing without its desktop layer, for the tests of the
    /// numbered master layers.
    pub(crate) fn without_desktop() -> App {
        let mut app = App::headless();
        let desktop: Vec<LayerId> = app
            .doc()
            .master
            .iter()
            .filter(|l| l.desktop)
            .map(|l| l.id)
            .collect();
        for layer in desktop {
            app.run(Command::DeleteLayer { layer });
        }
        app
    }

    fn rect_kind() -> ShapeKind {
        ShapeKind::Rect {
            rect: tracedraw_core::Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 0.0,
            corners: None,
        }
    }

    fn with_ui(mut f: impl FnMut(&mut Ui)) {
        let ctx = crate::theme::ui_context();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 600.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| f(ui));
        out.textures_delta.clear();
    }

    fn draw(app: &mut App) {
        with_ui(|ui| objects_docker(app, ui));
    }

    #[test]
    fn objects_show_their_names_or_what_they_are() {
        crate::i18n::set_language("en");
        let mut s = Shape::new(ShapeId(1), rect_kind());
        assert_eq!(object_label(&s), "Rectangle");
        s.name = Some("Water".into());
        assert_eq!(object_label(&s), "Water");
        s.name = Some("   ".into());
        assert_eq!(object_label(&s), "Rectangle");
    }

    /// Each object's row has its own eye and padlock: a click hides or
    /// shows it, locks or unlocks it, one undo step each (#832).
    #[test]
    fn an_objects_eye_and_padlock_hide_and_lock_it() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let id = app.new_shape(rect_kind()).expect("shape");
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| objects_docker(app, ui));
            out.textures_delta.clear();
        };
        let click = |app: &mut App, what: &str| {
            run(app, Vec::new());
            let wid = egui::Id::new(("objects_object", id.raw())).with(what);
            let at = ctx.read_response(wid).expect("drawn").rect.center();
            run(app, vec![egui::Event::PointerMoved(at)]);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            run(app, vec![button(true)]);
            run(app, vec![button(false)]);
        };
        click(&mut app, "eye");
        assert!(!app.doc().find_shape(id).expect("shape").visible);
        click(&mut app, "eye");
        assert!(app.doc().find_shape(id).expect("shape").visible);
        click(&mut app, "lock");
        assert!(app.doc().find_shape(id).expect("shape").locked);
        app.undo();
        assert!(!app.doc().find_shape(id).expect("shape").locked);
    }

    /// With several objects selected, the eye of one of them hides them
    /// all, or shows them all when it is hidden, whatever each was; they
    /// stay selected and one undo brings them back. An object outside
    /// the selection goes alone (#2571).
    #[test]
    fn the_eye_of_a_selected_object_hides_and_shows_the_whole_selection() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let a = app.new_shape(rect_kind()).expect("shape");
        let b = app.new_shape(rect_kind()).expect("shape");
        let c = app.new_shape(rect_kind()).expect("shape");
        let other = app.new_shape(rect_kind()).expect("shape");
        // One of the selection already hidden.
        app.run(Command::SetVisible {
            shapes: vec![c],
            visible: false,
        });
        app.select(vec![a, b, c]);
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| objects_docker(app, ui));
            out.textures_delta.clear();
        };
        let eye = |app: &mut App, id: ShapeId| {
            run(app, Vec::new());
            let wid = egui::Id::new(("objects_object", id.raw())).with("eye");
            let at = ctx.read_response(wid).expect("drawn").rect.center();
            run(app, vec![egui::Event::PointerMoved(at)]);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            run(app, vec![button(true)]);
            run(app, vec![button(false)]);
        };
        let visible = |app: &App, id: ShapeId| app.doc().find_shape(id).expect("shape").visible;
        // A visible one's eye hides the three.
        eye(&mut app, a);
        assert!(!visible(&app, a) && !visible(&app, b) && !visible(&app, c));
        assert!(visible(&app, other));
        let mut sel = app.selection.clone();
        sel.sort();
        let mut want = vec![a, b, c];
        want.sort();
        assert_eq!(sel, want, "the selection stays");
        // One undo step brings back each as it was.
        app.undo();
        assert!(visible(&app, a) && visible(&app, b) && !visible(&app, c));
        // A hidden one's eye shows them all.
        eye(&mut app, c);
        assert!(visible(&app, a) && visible(&app, b) && visible(&app, c));
        // The eye of an object outside the selection hides it alone.
        eye(&mut app, other);
        assert!(!visible(&app, other));
        assert!(visible(&app, a) && visible(&app, b) && visible(&app, c));
    }

    /// Selecting an object on the page scrolls the list to its row when
    /// it is out of view, as when it lies far down a long list (#1044).
    /// A selection that stays the same leaves the scroll alone.
    /// One frame of the docker, `size` wide and high, with `events`.
    fn frame(ctx: &egui::Context, app: &mut App, size: f32, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size, size),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| objects_docker(app, ui));
        out.textures_delta.clear();
    }

    fn wheel(at: Pos2, notches: f32, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: Vec2::new(0.0, notches),
                phase: egui::TouchPhase::Move,
                modifiers,
            },
        ]
    }

    /// Each object row shows a thumbnail of the object, drawn once and
    /// drawn again only when that object changes (#896).
    #[test]
    fn rows_show_thumbnails_drawn_once_per_change() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let ids: Vec<ShapeId> = (0..3)
            .map(|_| app.new_shape(rect_kind()).expect("shape"))
            .collect();
        app.selection.clear();
        for _ in 0..3 {
            frame(&ctx, &mut app, 400.0, Vec::new());
        }
        assert_eq!(app.object_thumbs.renders, 3);
        assert_eq!(app.object_thumbs.len(), 3);
        assert!(app.object_thumbs.bytes() > 0);
        // A change to one object draws that one again, the others stay.
        app.run(Command::SetFill {
            shapes: vec![ids[1]],
            fill: tracedraw_core::Fill::Solid(tracedraw_core::Color::rgb8(0, 200, 0)),
        });
        for _ in 0..3 {
            frame(&ctx, &mut app, 400.0, Vec::new());
        }
        assert_eq!(app.object_thumbs.renders, 4);
        // Selecting (no change to the drawing) draws nothing.
        app.select(vec![ids[0]]);
        frame(&ctx, &mut app, 400.0, Vec::new());
        assert_eq!(app.object_thumbs.renders, 4);
    }

    /// Ctrl+wheel over the list and the wheel over the bar's slider
    /// change the thumbnail size by one step a notch; the rows grow.
    #[test]
    fn ctrl_wheel_and_the_bar_change_the_thumbnail_size() {
        use crate::object_thumbs::{DEFAULT_SIZE, MAX_SIZE, STEP};
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let id = app.new_shape(rect_kind()).expect("shape");
        app.selection.clear();
        assert_eq!(app.settings.object_thumb_size, DEFAULT_SIZE);
        let eye = |ctx: &egui::Context| {
            ctx.read_response(egui::Id::new(("objects_object", id.raw())).with("eye"))
                .map(|r| r.rect)
        };
        frame(&ctx, &mut app, 400.0, Vec::new());
        let before = eye(&ctx).expect("row drawn");
        // A plain wheel over the list scrolls; it leaves the size.
        frame(
            &ctx,
            &mut app,
            400.0,
            wheel(Pos2::new(200.0, 60.0), 1.0, egui::Modifiers::NONE),
        );
        assert_eq!(app.settings.object_thumb_size, DEFAULT_SIZE);
        for _ in 0..2 {
            frame(
                &ctx,
                &mut app,
                400.0,
                wheel(Pos2::new(200.0, 60.0), 1.0, egui::Modifiers::CTRL),
            );
        }
        assert_eq!(app.settings.object_thumb_size, DEFAULT_SIZE + 2.0 * STEP);
        // (Responses are read a frame late.)
        frame(&ctx, &mut app, 400.0, Vec::new());
        frame(&ctx, &mut app, 400.0, Vec::new());
        // The row is taller; the thumbnail is drawn again at the new size.
        let after = eye(&ctx).expect("row drawn");
        assert!(after.center().y > before.center().y, "{before:?} {after:?}");
        // The wheel over the slider in the bottom bar, down a notch.
        frame(
            &ctx,
            &mut app,
            400.0,
            wheel(Pos2::new(200.0, 385.0), -1.0, egui::Modifiers::NONE),
        );
        assert_eq!(app.settings.object_thumb_size, DEFAULT_SIZE + STEP);
        // Never past the largest size.
        for _ in 0..30 {
            frame(
                &ctx,
                &mut app,
                400.0,
                wheel(Pos2::new(200.0, 60.0), 1.0, egui::Modifiers::CTRL),
            );
        }
        assert_eq!(app.settings.object_thumb_size, MAX_SIZE);
    }

    /// In the browser a wheel notch comes as three lines (Firefox) or as
    /// 100 pixels (Chromium): Ctrl+wheel over the list and the wheel over
    /// the bar's slider still change the thumbnail size by one step a
    /// notch, as on the desktop; they took three or two (#1406). Half a
    /// browser notch from a touchpad adds up to a step with the next half.
    #[test]
    fn a_browser_wheel_notch_steps_the_thumbnail_size_once() {
        use crate::object_thumbs::{DEFAULT_SIZE, STEP};
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.new_shape(rect_kind()).expect("shape");
        app.selection.clear();
        frame(&ctx, &mut app, 400.0, Vec::new());
        let turn = |at: Pos2, unit, dy: f32, modifiers| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::MouseWheel {
                    unit,
                    delta: Vec2::new(0.0, dy),
                    phase: egui::TouchPhase::Move,
                    modifiers,
                },
            ]
        };
        let list = Pos2::new(200.0, 60.0);
        let bar = Pos2::new(200.0, 385.0);
        let ctrl = egui::Modifiers::CTRL;
        let none = egui::Modifiers::NONE;
        use egui::MouseWheelUnit::{Line, Point};
        crate::ui::field::set_web_wheel(true);
        let mut size = DEFAULT_SIZE;
        for (at, unit, dy, mods, step) in [
            (list, Line, 3.0, ctrl, 1.0),
            (list, Point, 100.0, ctrl, 1.0),
            (list, Line, -3.0, ctrl, -1.0),
            (bar, Point, -100.0, none, -1.0),
            (bar, Line, 3.0, none, 1.0),
            (list, Point, 50.0, ctrl, 0.0),
            (list, Point, 50.0, ctrl, 1.0),
        ] {
            frame(&ctx, &mut app, 400.0, turn(at, unit, dy, mods));
            size += step * STEP;
            assert_eq!(app.settings.object_thumb_size, size, "{unit:?} {dy}");
        }
        // The desktop's notch, one line or 50 points, is one step as before.
        crate::ui::field::set_web_wheel(false);
        for (unit, dy) in [(Line, -1.0), (Point, -50.0)] {
            frame(&ctx, &mut app, 400.0, turn(list, unit, dy, ctrl));
            size -= STEP;
            assert_eq!(app.settings.object_thumb_size, size, "{unit:?} {dy}");
        }
    }

    /// A drawing of thousands of objects: only the rows in sight get
    /// thumbnails, a frame with nothing changed draws none, and the cache
    /// stays small.
    #[test]
    fn a_long_list_draws_only_the_rows_in_sight() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        for i in 0..3000 {
            let x = f64::from(i % 50) * 4.0;
            let y = f64::from(i / 50) * 4.0;
            app.new_shape(ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(x, y, x + 3.0, y + 3.0),
                radius: 0.0,
                corners: None,
            });
        }
        app.selection.clear();
        // A few thumbnails are drawn per frame: frames until the rows in
        // sight all have theirs (how many fit depends on the fonts).
        let mut drawn = u64::MAX;
        for _ in 0..30 {
            frame(&ctx, &mut app, 400.0, Vec::new());
            if app.object_thumbs.renders == drawn {
                break;
            }
            drawn = app.object_thumbs.renders;
        }
        assert!((5..40).contains(&drawn), "{drawn}");
        frame(&ctx, &mut app, 400.0, Vec::new());
        assert_eq!(app.object_thumbs.renders, drawn);
        assert!(app.object_thumbs.bytes() < 2 * 1024 * 1024);
    }

    /// Frame times of the docker over 3000 objects (`cargo test --release
    /// -p tracedraw -- --ignored objects_docker_frame_times --nocapture`):
    /// the first frames (thumbnails drawn), a steady frame, and frames
    /// scrolling the list.
    #[test]
    #[ignore]
    fn objects_docker_frame_times() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        for i in 0..3000 {
            let x = f64::from(i % 50) * 4.0;
            let y = f64::from(i / 50) * 4.0;
            app.new_shape(ShapeKind::Ellipse {
                rect: tracedraw_core::Rect::new(x, y, x + 3.0, y + 3.0),
                arc: None,
            });
        }
        app.selection.clear();
        let time = |app: &mut App, events: Vec<egui::Event>| {
            let t = web_time::Instant::now();
            frame(&ctx, app, 600.0, events);
            t.elapsed().as_secs_f64() * 1000.0
        };
        let first = time(&mut app, Vec::new());
        let mut fill = 0.0;
        for _ in 0..10 {
            fill += time(&mut app, Vec::new());
        }
        let steady: f64 = (0..20).map(|_| time(&mut app, Vec::new())).sum::<f64>() / 20.0;
        let scroll: f64 = (0..40)
            .map(|_| {
                time(
                    &mut app,
                    wheel(Pos2::new(300.0, 200.0), -3.0, egui::Modifiers::NONE),
                )
            })
            .sum::<f64>()
            / 40.0;
        println!(
            "objects docker, 3000 objects: first {first:.1} ms, next 10 {fill:.1} ms, \
             steady {steady:.2} ms, scrolling {scroll:.2} ms; {} thumbnails drawn, \
             {} kept, {} KB",
            app.object_thumbs.renders,
            app.object_thumbs.len(),
            app.object_thumbs.bytes() / 1024
        );
    }

    #[test]
    fn the_list_scrolls_to_an_object_selected_on_the_page() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let ids: Vec<ShapeId> = (0..80)
            .map(|_| app.new_shape(rect_kind()).expect("shape"))
            .collect();
        app.selection.clear();
        // The scroll lands in one frame.
        ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
        let run = |app: &mut App, scroll: f32| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 300.0),
                )),
                events: if scroll != 0.0 {
                    vec![
                        egui::Event::PointerMoved(Pos2::new(150.0, 100.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: Vec2::new(0.0, scroll),
                            phase: egui::TouchPhase::Move,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| objects_docker(app, ui));
            out.textures_delta.clear();
        };
        let row_of = |id: ShapeId| {
            ctx.read_response(egui::Id::new(("objects_object", id.raw())).with("eye"))
                .map(|r| r.rect)
        };
        let shown =
            |r: Option<egui::Rect>| r.is_some_and(|r| r.top() >= 0.0 && r.bottom() <= 300.0);
        for _ in 0..4 {
            run(&mut app, 0.0);
        }
        // The first object drawn is the bottom of the list.
        let far = ids[0];
        assert!(!shown(row_of(far)), "{:?}", row_of(far));
        app.select(vec![far]);
        for _ in 0..4 {
            run(&mut app, 0.0);
        }
        assert!(shown(row_of(far)), "{:?}", row_of(far));
        // Scrolled away by hand, the unchanged selection stays out of view.
        for _ in 0..4 {
            run(&mut app, 2000.0);
        }
        assert!(!shown(row_of(far)), "{:?}", row_of(far));
    }

    #[test]
    fn the_docker_draws_layers_groups_and_selection() {
        let mut app = App::headless();
        let id = app.new_shape(rect_kind()).expect("shape");
        app.select(vec![id]);
        new_master_layer(&mut app, MasterScope::Odd);
        let layer = app.doc().pages[0].layers[0].id;
        for collapsed in [false, true] {
            if collapsed {
                app.objects_collapsed.insert(layer);
                app.objects_master_closed = true;
            }
            draw(&mut app);
        }
        assert_eq!(app.selection, vec![id]);
    }

    #[test]
    fn new_layers_get_free_names_and_become_active() {
        crate::i18n::set_language("en");
        let mut app = without_desktop();
        let first = app.doc().pages[0].layers[0].id;
        new_layer(&mut app);
        new_layer(&mut app);
        let names: Vec<String> = app.doc().pages[0]
            .layers
            .iter()
            .map(|l| l.name.clone())
            .collect();
        assert_eq!(names, ["Layer 1", "Layer 2", "Layer 3"]);
        let top = app.doc().pages[0].layers[2].id;
        assert_eq!(app.active_layer(), Some(top));
        // Names in use are skipped.
        app.run(Command::DeleteLayer {
            layer: app.doc().pages[0].layers[1].id,
        });
        new_layer(&mut app);
        assert_eq!(app.doc().pages[0].layers[2].name, "Layer 4");
        // Picking a lower layer sends new objects there.
        app.picked_layer = Some(first);
        let id = app.new_shape(rect_kind()).expect("shape");
        assert!(app.doc().pages[0].layers[0]
            .shapes
            .iter()
            .any(|s| s.id == id));
        // A hidden or locked pick does not take them.
        app.run(Command::SetLayerVisible {
            layer: first,
            visible: false,
        });
        assert_ne!(app.active_layer(), Some(first));
        new_master_layer(&mut app, MasterScope::All);
        new_master_layer(&mut app, MasterScope::Even);
        let master: Vec<String> = app.doc().master.iter().map(|l| l.name.clone()).collect();
        assert_eq!(master, ["Master Layer 1", "Master Layer 2"]);
        // An even-pages master layer does not show on page 1, so it is
        // not where page 1's objects go.
        assert_ne!(app.active_layer(), app.doc().master.last().map(|l| l.id));
        app.picked_layer = app.doc().master.first().map(|l| l.id);
        assert_eq!(app.active_layer(), app.picked_layer);
    }

    /// #1132, #1477: the in-front choice per kind of master layer moves
    /// the master layers of that kind the drawing has in one step and
    /// is taken by the new ones; a master object can be moved onto a
    /// page's layer, a page layer into the master page and back.
    #[test]
    fn master_layers_in_front_by_choice_and_objects_move_between_layers() {
        let mut app = without_desktop();
        app.settings.master_in_front = [false; 3];
        new_master_layer(&mut app, MasterScope::All);
        new_master_layer(&mut app, MasterScope::Odd);
        assert!(app.doc().master.iter().all(|l| !l.in_front));
        set_master_front(&mut app, MasterScope::All, true);
        assert!(app.doc().master[0].in_front);
        assert!(!app.doc().master[1].in_front, "odd pages keep theirs");
        // One undo step for the drawing; the choice stays.
        app.undo();
        assert!(!app.doc().master[0].in_front);
        assert!(master_front_default(&app, MasterScope::All));
        // A new all-pages master layer comes in front, in one step.
        new_master_layer(&mut app, MasterScope::All);
        let front = app.doc().master[2].id;
        assert!(app.doc().master[2].in_front);
        app.undo();
        assert!(app.doc().master.iter().all(|l| l.id != front));
        // It is drawn over the page's own layers.
        new_master_layer(&mut app, MasterScope::All);
        let master = app.doc().master.last().expect("master").id;
        let id = app.new_shape(rect_kind()).expect("shape");
        assert_eq!(layer_holding(&app, id), Some(master));
        let layers: Vec<LayerId> = app.page_layers().iter().map(|l| l.id).collect();
        assert_eq!(layers.last(), Some(&master));
        // Move to a layer of the page: it shows on this page only.
        let page_layer = app.doc().pages[0].layers[0].id;
        move_to_layer(&mut app, &[id], page_layer);
        assert_eq!(layer_holding(&app, id), Some(page_layer));
        assert_eq!(app.selection, vec![id]);
        assert!(app.doc().master.last().expect("master").shapes.is_empty());
        app.undo();
        assert_eq!(layer_holding(&app, id), Some(master), "one undo step");
        app.redo();
        // To a layer of another page: that page becomes the current one.
        let first = app.page;
        app.run(Command::duplicate_page(first));
        let second = app.doc().pages[1].id;
        let there = app.doc().pages[1].layers[0].id;
        move_to_layer(&mut app, &[id], there);
        assert_eq!(app.page, second);
        assert_eq!(layer_holding(&app, id), Some(there));
        // A page layer into the master page, with its objects, and back.
        layer_to_master(&mut app, there);
        let moved = app.doc().master.last().expect("master").id;
        assert_eq!(layer_holding(&app, id), Some(moved));
        assert_eq!(
            app.doc().pages[1].layers.len(),
            1,
            "a page keeps its last layer"
        );
        master_to_page(&mut app, moved);
        assert!(app.doc().master.iter().all(|l| l.id != moved));
        let back = layer_holding(&app, id).expect("on a layer");
        assert!(app.doc().pages[1].layers.iter().any(|l| l.id == back));
        // The docker lists every page, or the current one.
        draw(&mut app);
        app.settings.objects_all_pages = false;
        draw(&mut app);
    }

    /// #1161, #1775: a new drawing has the master page's desktop layer,
    /// behind the pages; new objects still go to the page's own layer,
    /// and an object moved onto the desktop shows and is picked from
    /// every page, beside the page or on it. It is not one of the
    /// numbered master layers, and the in-front choice leaves it behind.
    #[test]
    fn a_new_drawing_has_a_desktop_shared_by_every_page() {
        let mut app = App::headless();
        let desktop: Vec<&tracedraw_core::Layer> =
            app.doc().master.iter().filter(|l| l.desktop).collect();
        assert_eq!(desktop.len(), 1);
        assert_eq!(desktop[0].name, "Desktop");
        assert_eq!(desktop[0].scope, MasterScope::All);
        assert!(!desktop[0].in_front);
        let desktop = desktop[0].id;
        let page_layer = app.doc().pages[0].layers[0].id;
        assert_eq!(app.active_layer(), Some(page_layer));
        // An object off the page, moved onto the desktop.
        let id = app
            .new_shape(tracedraw_core::document::ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(-60.0, 10.0, -20.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        assert_eq!(layer_holding(&app, id), Some(page_layer));
        move_to_layer(&mut app, &[id], desktop);
        assert_eq!(layer_holding(&app, id), Some(desktop));
        // From a new page it is there and picked.
        app.add_page();
        assert_ne!(app.page, app.doc().pages[0].id);
        let edge = tracedraw_core::Point::new(-60.0, 25.0);
        assert_eq!(app.hit_test(edge), Some(id));
        assert!(app
            .page_layers()
            .iter()
            .any(|l| l.id == desktop && l.shapes.iter().any(|s| s.id == id)));
        // Numbered master layers start at 1; the in-front choice for
        // all pages leaves the desktop behind.
        app.settings.master_in_front = [false; 3];
        new_master_layer(&mut app, MasterScope::All);
        assert_eq!(
            app.doc().master.last().expect("master").name,
            "Master Layer 1"
        );
        set_master_front(&mut app, MasterScope::All, true);
        let d = app
            .doc()
            .master
            .iter()
            .find(|l| l.id == desktop)
            .expect("desktop");
        assert!(!d.in_front);
        assert!(app.doc().master.last().expect("master").in_front);
    }

    #[test]
    fn master_layer_objects_are_picked_where_their_layer_shows() {
        let mut app = without_desktop();
        new_master_layer(&mut app, MasterScope::Odd);
        let master = app.doc().master[0].id;
        assert_eq!(app.active_layer(), Some(master));
        let id = app.new_shape(rect_kind()).expect("shape");
        assert_eq!(app.doc().master[0].shapes.len(), 1);
        app.select(Vec::new());
        // On its outline (new rectangles have no fill).
        let inside = tracedraw_core::Point::new(0.0, 5.0);
        assert_eq!(app.hit_test(inside), Some(id));
        app.select_all();
        assert_eq!(app.selection, vec![id]);
        // Page 2 is even: the layer does not show there.
        let page = app.page;
        app.run(Command::duplicate_page(page));
        app.page = app.doc().pages[1].id;
        assert_eq!(app.hit_test(inside), None);
        app.page = page;
        app.run(Command::SetLayerLocked {
            layer: master,
            locked: true,
        });
        assert_eq!(app.hit_test(inside), None);
    }

    #[test]
    fn delete_takes_the_selection_else_the_active_layer() {
        let mut app = without_desktop();
        let only = app.doc().pages[0].layers[0].id;
        let id = app.new_shape(rect_kind()).expect("shape");
        app.select(vec![id]);
        delete_current(&mut app);
        assert!(app.doc().find_shape(id).is_none());
        // A page keeps its last layer.
        assert_eq!(deletable_layer(&app), None);
        delete_current(&mut app);
        assert_eq!(app.doc().pages[0].layers.len(), 1);
        new_layer(&mut app);
        delete_current(&mut app);
        assert_eq!(app.doc().pages[0].layers.len(), 1);
        assert_eq!(app.doc().pages[0].layers[0].id, only);
        new_master_layer(&mut app, MasterScope::All);
        delete_current(&mut app);
        assert!(app.doc().master.is_empty());
        assert_eq!(app.picked_layer, None);
        assert_eq!(app.active_layer(), Some(only));
    }

    #[test]
    fn clicking_an_object_in_the_docker_makes_its_layer_active() {
        let mut app = App::headless();
        let low = app.doc().pages[0].layers[0].id;
        let id = app.new_shape(rect_kind()).expect("shape");
        new_layer(&mut app);
        assert_ne!(app.active_layer(), Some(low));
        with_ui(|ui| {
            apply(
                &mut app,
                ui,
                Actions {
                    select: Some(id),
                    ..Default::default()
                },
            );
        });
        assert_eq!(app.selection, vec![id]);
        assert_eq!(app.active_layer(), Some(low));
    }

    /// A layer's menu copies or cuts its objects and pastes the
    /// clipboard onto it (#1992).
    #[test]
    fn a_layers_menu_cuts_copies_and_pastes_its_objects() {
        let mut app = App::headless();
        let low = app.doc().pages[0].layers[0].id;
        app.new_shape(rect_kind()).expect("shape");
        app.new_shape(rect_kind()).expect("shape");
        new_layer(&mut app);
        let high = app.active_layer().expect("new layer");
        assert_ne!(high, low);
        let count = |app: &App, id: LayerId| {
            app.doc()
                .all_layers()
                .find(|l| l.id == id)
                .map_or(0, |l| l.shapes.len())
        };
        let run = |app: &mut App, clip: (LayerId, LayerClip)| {
            with_ui(|ui| {
                apply(
                    app,
                    ui,
                    Actions {
                        layer_clip: Some(clip),
                        ..Default::default()
                    },
                );
            });
        };
        run(&mut app, (low, LayerClip::Copy));
        run(&mut app, (high, LayerClip::Paste));
        assert_eq!((count(&app, low), count(&app, high)), (2, 2));
        run(&mut app, (low, LayerClip::Cut));
        assert_eq!(count(&app, low), 0);
        run(&mut app, (low, LayerClip::Paste));
        assert_eq!(count(&app, low), 2);
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;

    fn rect_kind() -> ShapeKind {
        ShapeKind::Rect {
            rect: tracedraw_core::Rect::new(0.0, 0.0, 10.0, 10.0),
            radius: 0.0,
            corners: None,
        }
    }

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| objects_docker(app, ui));
        out.textures_delta.clear();
    }

    fn object_row(ctx: &egui::Context, id: ShapeId) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(ShapeId, Rect)>>(egui::Id::new("objects_rows_drawn")))
            .and_then(|rows| rows.into_iter().find(|(s, _)| *s == id))
            .map(|(_, r)| r)
            .expect("the object's row")
    }

    fn layer_row(ctx: &egui::Context, id: LayerId) -> Rect {
        ctx.data(|d| d.get_temp::<Vec<(LayerId, Rect)>>(egui::Id::new("objects_layer_rows_drawn")))
            .and_then(|rows| rows.into_iter().find(|(l, _)| *l == id))
            .map(|(_, r)| r)
            .expect("the layer's row")
    }

    /// Press at `from`, drag to `to` over a few frames and let go there.
    fn drag(ctx: &egui::Context, app: &mut App, from: Pos2, to: Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(ctx, app, vec![egui::Event::PointerMoved(from)]);
        frame(ctx, app, vec![button(from, true)]);
        let mid = from + (to - from) * 0.5;
        frame(ctx, app, vec![egui::Event::PointerMoved(mid)]);
        frame(ctx, app, vec![egui::Event::PointerMoved(to)]);
        frame(ctx, app, vec![button(to, false)]);
        frame(ctx, app, Vec::new());
    }

    fn order(app: &App, layer: usize) -> Vec<ShapeId> {
        app.doc().pages[0].layers[layer]
            .shapes
            .iter()
            .map(|s| s.id)
            .collect()
    }

    /// Live test of 0.41: an object's row dragged along the Objects
    /// docker and let go on another object's row puts it in front of
    /// that object (the row's upper half) or behind it (its lower half),
    /// one undo step; a selected row carries the selection along.
    #[test]
    fn a_row_dragged_along_the_list_changes_the_stacking_order() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let a = app.new_shape(rect_kind()).expect("a");
        let b = app.new_shape(rect_kind()).expect("b");
        let c = app.new_shape(rect_kind()).expect("c");
        app.select(Vec::new());
        assert_eq!(order(&app, 0), [a, b, c]);
        frame(&ctx, &mut app, Vec::new());
        // The list shows the front object first: c, b, a.
        let (ra, rc) = (object_row(&ctx, a), object_row(&ctx, c));
        assert!(rc.top() < ra.top());
        // a onto the upper half of c's row: in front of c.
        let to = Pos2::new(rc.center().x, rc.top() + 2.0);
        drag(&ctx, &mut app, ra.center(), to);
        assert_eq!(order(&app, 0), [b, c, a]);
        assert_eq!(app.selection, vec![a]);
        app.undo();
        assert_eq!(order(&app, 0), [a, b, c], "one undo step");
        // c onto the lower half of a's row: behind a.
        frame(&ctx, &mut app, Vec::new());
        let (ra, rc) = (object_row(&ctx, a), object_row(&ctx, c));
        let to = Pos2::new(ra.center().x, ra.bottom() - 2.0);
        drag(&ctx, &mut app, rc.center(), to);
        assert_eq!(order(&app, 0), [c, a, b]);
        app.undo();
        // a and c selected, a's row dragged onto b's upper half: both go
        // in front of b, keeping their order.
        app.select(vec![a, c]);
        frame(&ctx, &mut app, Vec::new());
        let (ra, rb) = (object_row(&ctx, a), object_row(&ctx, b));
        let to = Pos2::new(rb.center().x, rb.top() + 2.0);
        drag(&ctx, &mut app, ra.center(), to);
        assert_eq!(order(&app, 0), [b, a, c]);
        app.undo();
        assert_eq!(order(&app, 0), [a, b, c]);
        // Let go on its own row, or below every row: nothing changes.
        app.select(Vec::new());
        frame(&ctx, &mut app, Vec::new());
        let ra = object_row(&ctx, a);
        drag(
            &ctx,
            &mut app,
            ra.center(),
            ra.center() + Vec2::new(30.0, 0.0),
        );
        drag(&ctx, &mut app, ra.center(), Pos2::new(200.0, 700.0));
        assert_eq!(order(&app, 0), [a, b, c]);
        // A locked object's row does not move.
        app.run(Command::SetLocked {
            shapes: vec![a],
            locked: true,
        });
        frame(&ctx, &mut app, Vec::new());
        let (ra, rc) = (object_row(&ctx, a), object_row(&ctx, c));
        let to = Pos2::new(rc.center().x, rc.top() + 2.0);
        drag(&ctx, &mut app, ra.center(), to);
        assert_eq!(order(&app, 0), [a, b, c]);
    }

    /// A row let go on an object of another layer goes over to that layer
    /// beside the object, and let go on a layer's row on the top of that
    /// layer: one undo step each. A locked layer takes nothing.
    #[test]
    fn a_row_dragged_onto_another_layer_moves_the_object_there() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let low = app.doc().pages[0].layers[0].id;
        let a = app.new_shape(rect_kind()).expect("a");
        let b = app.new_shape(rect_kind()).expect("b");
        let page = app.page;
        app.run(Command::AddLayer {
            page,
            name: "Top".into(),
        });
        let high = app.doc().pages[0].layers[1].id;
        app.picked_layer = Some(high);
        let c = app.new_shape(rect_kind()).expect("c");
        let d = app.new_shape(rect_kind()).expect("d");
        app.select(Vec::new());
        assert_eq!(order(&app, 0), [a, b]);
        assert_eq!(order(&app, 1), [c, d]);
        frame(&ctx, &mut app, Vec::new());
        // b onto the lower half of d's row: behind d, in the top layer.
        let (rb, rd) = (object_row(&ctx, b), object_row(&ctx, d));
        let to = Pos2::new(rd.center().x, rd.bottom() - 2.0);
        drag(&ctx, &mut app, rb.center(), to);
        assert_eq!(order(&app, 0), [a]);
        assert_eq!(order(&app, 1), [c, b, d]);
        assert_eq!(app.active_layer(), Some(high));
        app.undo();
        assert_eq!(order(&app, 0), [a, b], "one undo step");
        assert_eq!(order(&app, 1), [c, d]);
        // c onto the lower layer's row: on the top of that layer.
        frame(&ctx, &mut app, Vec::new());
        let (rc, rl) = (object_row(&ctx, c), layer_row(&ctx, low));
        drag(&ctx, &mut app, rc.center(), rl.center());
        assert_eq!(order(&app, 0), [a, b, c]);
        assert_eq!(order(&app, 1), [d]);
        app.undo();
        assert_eq!(order(&app, 1), [c, d]);
        // A locked layer takes nothing, nor does an object dropped on
        // itself.
        app.run(Command::SetLayerLocked {
            layer: low,
            locked: true,
        });
        let ids = [d];
        let on = |reference| RowDrop::Object {
            reference,
            in_front: true,
        };
        assert!(drop_commands(&app, &ids, RowDrop::Layer(low)).is_empty());
        assert!(drop_commands(&app, &ids, on(a)).is_empty());
        assert!(drop_commands(&app, &ids, on(d)).is_empty());
        assert_eq!(drop_commands(&app, &ids, on(c)).len(), 1);
    }

    /// A right click on an object's row opens the drawing's own object
    /// menu for it: the object is selected first (the whole selection
    /// stays when it is part of it), a locked object gets the locked-object
    /// menu, and the menu is the object menu whatever the tool (#2601).
    #[test]
    fn a_right_click_on_an_object_row_opens_the_drawings_object_menu() {
        let mut app = App::headless();
        let a = app.new_shape(rect_kind()).expect("a");
        let b = app.new_shape(rect_kind()).expect("b");
        let pos = Pos2::new(40.0, 50.0);
        app.select(vec![a]);
        open_row_menu(&mut app, b, pos);
        assert_eq!(app.selection, vec![b]);
        assert_eq!(app.context_docker, Some(b));
        assert_eq!(app.context_menu.map(|(p, _)| p), Some(pos));
        assert_eq!(app.context_locked, None);
        app.select(vec![a, b]);
        open_row_menu(&mut app, a, pos);
        assert_eq!(app.selection, vec![a, b]);
        app.set_locked(true);
        app.select(Vec::new());
        open_row_menu(&mut app, a, pos);
        assert_eq!(app.context_locked, Some(a));
        assert!(app.selection.is_empty());
    }

    /// The row's menu holds the docker's Rename and Move to Layer and the
    /// drawing's object commands, also with the Shape tool on; Move to
    /// Layer moves the object (#2601).
    #[test]
    fn an_object_rows_menu_is_the_full_object_menu_with_rename_and_move() {
        let ctx = egui::Context::default();
        let fresh = || {
            let mut app = App::headless();
            let a = app.new_shape(rect_kind()).expect("a");
            new_layer(&mut app);
            app.tool = crate::tools::Tool::Shape;
            open_row_menu(&mut app, a, Pos2::new(40.0, 50.0));
            (app, a)
        };
        let frame = |app: &mut App, target: usize| {
            crate::ui::menus::replay::start(target);
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let mut close = false;
                crate::ui::context::menu_items(app, ui, &mut close);
            });
            out.textures_delta.clear();
            crate::ui::menus::replay::finish()
        };
        let (mut app, _) = fresh();
        let (_, rows) = frame(&mut app, usize::MAX);
        let mut keys = Vec::new();
        let mut moved = false;
        for t in 0..rows {
            let (mut app, a) = fresh();
            let before = layer_holding(&app, a);
            let (hit, _) = frame(&mut app, t);
            let Some(key) = hit else { continue };
            if layer_holding(&app, a) != before {
                moved = true;
            }
            if key == "docker.rename" {
                assert!(matches!(
                    app.dialog,
                    crate::ui::dialogs::Dialog::RenameObject { shape, .. } if shape == a
                ));
            }
            keys.push(key);
        }
        for want in [
            "docker.rename",
            "menu.edit.duplicate",
            "menu.object.to_front_of_page",
            "menu.object.hide_object",
            "menu.object.properties",
        ] {
            assert!(
                keys.iter().any(|k| k == want),
                "{want} missing from {keys:?}"
            );
        }
        assert!(moved, "Move to Layer moved nothing: {keys:?}");
    }
}

//! Full-screen preview (F9) and the Page Sorter view.

use crate::app::App;
use crate::i18n::tr;
use crate::theme::Tokens;
use egui::Ui;
use tracedraw_core::Command;

/// What the full-screen preview shows: the selection's box with Preview
/// Selected Only, else the page and every object of it, those lying off
/// the page too, as the drawing window shows them. Only the page showed,
/// so a drawing made beside or larger than its page was previewed as an
/// empty sheet (#2317).
pub fn preview_bounds(app: &App) -> tracedraw_core::geometry::Rect {
    let page = app.page_rect();
    if app.preview_selected_only {
        return app.selection_bounds().unwrap_or(page);
    }
    match app.doc().content_bounds(app.page).ok().flatten() {
        Some(b) if b.width().is_finite() && b.height().is_finite() => page.union(b),
        _ => page,
    }
}

pub fn fullscreen(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(egui::Color32::from_gray(40)))
        .show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            let page = app.page_rect();
            let bounds = preview_bounds(app);
            let painter = ui.painter_at(rect);
            // Fit the page into the screen.
            let margin = 20.0;
            let sx = (rect.width() - 2.0 * margin) as f64 / bounds.width().max(1e-6);
            let sy = (rect.height() - 2.0 * margin) as f64 / bounds.height().max(1e-6);
            let zoom = sx.min(sy);
            let w = (bounds.width() * zoom).ceil() as u32;
            let h = (bounds.height() * zoom).ceil() as u32;
            let doc = app.engine.document().clone();
            let view = tracedraw_render::ViewTransform {
                zoom,
                origin_x: -bounds.x0 * zoom,
                origin_y: bounds.y1 * zoom,
            };
            let opts = tracedraw_render::RenderOptions {
                width: w.max(1),
                height: h.max(1),
                view,
                preview: None,
                wireframe: false,
                ..tracedraw_render::RenderOptions::default()
            };
            let page_id = app.page;
            if let Some(mut pm) = tracedraw_render::render_page(&doc, page_id, &opts) {
                if !app.preview_selected_only {
                    let mut white = tiny_skia::Pixmap::new(pm.width(), pm.height())
                        .unwrap_or_else(|| tiny_skia::Pixmap::new(1, 1).expect("pixmap"));
                    white.fill(tiny_skia::Color::WHITE);
                    white.draw_pixmap(
                        0,
                        0,
                        pm.as_ref(),
                        &Default::default(),
                        tiny_skia::Transform::identity(),
                        None,
                    );
                    pm = white;
                }
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [pm.width() as usize, pm.height() as usize],
                    pm.data(),
                );
                let tex =
                    ctx.load_texture("fullscreen_preview", image, egui::TextureOptions::LINEAR);
                let size = egui::vec2(pm.width() as f32, pm.height() as f32);
                let pos = rect.center() - size / 2.0;
                let page_rect = egui::Rect::from_min_size(pos, size);
                painter.image(
                    tex.id(),
                    page_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                // Options > Display > Full-screen preview > Show page border:
                // round the page, inside what is shown.
                if !app.preview_selected_only && app.settings.preview_page_border {
                    let at = |x: f64, y: f64| {
                        page_rect.min
                            + egui::vec2(
                                ((x - bounds.x0) * zoom) as f32,
                                ((bounds.y1 - y) * zoom) as f32,
                            )
                    };
                    let sheet =
                        egui::Rect::from_two_pos(at(page.x0, page.y0), at(page.x1, page.y1));
                    painter.rect_stroke(
                        sheet,
                        0.0,
                        egui::Stroke::new(1.0, Tokens::page_border()),
                        egui::StrokeKind::Outside,
                    );
                }
            }
            painter.text(
                egui::pos2(rect.center().x, rect.max.y - 10.0),
                egui::Align2::CENTER_BOTTOM,
                tr("preview.exit_hint"),
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(200),
            );
        });
    let exit = ctx.input(|i| {
        i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::F9) || i.pointer.any_click()
    });
    if exit {
        app.fullscreen_preview = false;
        app.preview_selected_only = false;
    }
}

pub fn page_sorter(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.heading(tr("menu.view.page_sorter"));
        if ui.button(tr("preview.back_to_page")).clicked() {
            app.page_sorter = false;
        }
        if ui.button(tr("menu.layout.insert_page")).clicked() {
            app.add_page();
        }
    });
    ui.separator();
    let pages: Vec<(tracedraw_core::PageId, String, tracedraw_core::Size)> = app
        .doc()
        .pages
        .iter()
        .map(|p| (p.id, p.name.clone(), p.size))
        .collect();
    let n = pages.len();
    let doc = app.engine.document().clone();
    let mut goto: Option<usize> = None;
    let mut mv: Option<(tracedraw_core::PageId, usize)> = None;
    let mut del: Option<tracedraw_core::PageId> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, (id, name, size)) in pages.iter().enumerate() {
                let thumb_h = 150.0f32;
                let thumb_w = (thumb_h as f64 * size.width / size.height.max(1e-6)) as f32;
                ui.vertical(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(thumb_w.max(40.0), thumb_h),
                        egui::Sense::click(),
                    );
                    let active = *id == app.page;
                    ui.painter().rect_filled(
                        rect.expand(3.0),
                        3.0,
                        if active {
                            Tokens::tool_active()
                        } else {
                            Tokens::panel_dark()
                        },
                    );
                    let dpi = thumb_h as f64 / (size.height / 25.4);
                    if let Some(pm) = tracedraw_render::render_page_image(&doc, *id, dpi.max(4.0)) {
                        let image = egui::ColorImage::from_rgba_premultiplied(
                            [pm.width() as usize, pm.height() as usize],
                            pm.data(),
                        );
                        let tex = ui.ctx().load_texture(
                            format!("sorter_{}", id.raw()),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    if resp.clicked() {
                        goto = Some(i);
                    }
                    if resp.double_clicked() {
                        goto = Some(i);
                        app.page_sorter = false;
                    }
                    ui.horizontal(|ui| {
                        ui.label(format!("{}. {name}", i + 1));
                        if i > 0 && ui.small_button("\u{25C0}").clicked() {
                            mv = Some((*id, i - 1));
                        }
                        if i + 1 < n && ui.small_button("\u{25B6}").clicked() {
                            mv = Some((*id, i + 1));
                        }
                        if n > 1 && ui.small_button("x").clicked() {
                            del = Some(*id);
                        }
                    });
                });
                ui.add_space(12.0);
            }
        });
    });
    if let Some(i) = goto {
        app.goto_page(i);
    }
    if let Some((id, to)) = mv {
        app.run(Command::MovePage { page: id, to });
    }
    if let Some(id) = del {
        app.run(Command::DeletePage { page: id });
        if app.page == id {
            app.page = app.doc().pages[0].id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::geometry::Rect;

    /// A drawing lying off its page is previewed with the page, not cut
    /// away to an empty sheet; Preview Selected Only shows the selection
    /// alone (#2317).
    #[test]
    fn the_preview_shows_objects_off_the_page() {
        let mut app = App::headless();
        app.show_welcome = false;
        let page = app.page_rect();
        assert_eq!(preview_bounds(&app), page, "an empty drawing: its page");
        // A wide bar above the page, as a title drawn off the sheet.
        let bar = Rect::new(
            page.x0 - 200.0,
            page.y1 + 50.0,
            page.x1 + 200.0,
            page.y1 + 90.0,
        );
        let id = app
            .new_shape(tracedraw_core::ShapeKind::Rect {
                rect: bar,
                radius: 0.0,
                corners: None,
            })
            .expect("a rectangle");
        let b = preview_bounds(&app);
        assert!(
            b.x0 <= bar.x0 + 1e-6 && b.x1 >= bar.x1 - 1e-6 && b.y1 >= bar.y1 - 1e-6,
            "{b:?} {bar:?}"
        );
        assert!(b.y0 <= page.y0 + 1e-6, "the page stays in: {b:?}");
        // Rendered as the preview renders it, the bar shows.
        let zoom = 0.5;
        let opts = tracedraw_render::RenderOptions {
            width: (b.width() * zoom).ceil() as u32,
            height: (b.height() * zoom).ceil() as u32,
            view: tracedraw_render::ViewTransform {
                zoom,
                origin_x: -b.x0 * zoom,
                origin_y: b.y1 * zoom,
            },
            preview: None,
            wireframe: false,
            ..tracedraw_render::RenderOptions::default()
        };
        let doc = app.engine.document().clone();
        let pm = tracedraw_render::render_page(&doc, app.page, &opts).expect("render");
        // Some of the bar's outline is drawn in the strip above the page.
        let top = ((b.y1 - bar.y1) * zoom).floor() as u32;
        let bottom = ((b.y1 - bar.y0) * zoom).ceil() as u32;
        let drawn = (top..bottom.min(pm.height()))
            .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
            .filter(|(x, y)| pm.pixel(*x, *y).is_some_and(|p| p.alpha() > 0))
            .count();
        assert!(drawn > 0, "the bar is drawn");
        // Preview Selected Only: the selection's box.
        app.select(vec![id]);
        app.preview_selected_only = true;
        let sel = app.selection_bounds().expect("selected");
        assert_eq!(preview_bounds(&app), sel);
    }

    /// One frame of the whole window with `events`.
    fn window(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
        out.textures_delta.clear();
    }

    fn f9() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::F9,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn f9_opens_the_full_screen_preview_and_closes_it() {
        // #2195: F9 worked only from the View menu.
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        window(&ctx, &mut app, Vec::new());
        assert!(!app.fullscreen_preview);
        window(&ctx, &mut app, vec![f9()]);
        assert!(app.fullscreen_preview, "F9 opens the preview");
        window(&ctx, &mut app, Vec::new());
        assert!(app.fullscreen_preview, "it stays open");
        window(&ctx, &mut app, vec![f9()]);
        assert!(!app.fullscreen_preview, "F9 again closes it");
        window(&ctx, &mut app, Vec::new());
        assert!(!app.fullscreen_preview);
    }
}

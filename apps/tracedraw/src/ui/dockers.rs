//! Dockers: Properties, Objects (layer manager) and Hints.

use crate::app::{App, DockerDrag, DockerPlace, DockerTab};
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::pattern_pick::{PatternAction, PatternPickers};
use crate::ui::slider::Settled;
use crate::ui::tips::RichTip;
use egui::Ui;
use tracedraw_core::patch::{FillField, FillMix, StrokeField, StrokeMix};
use tracedraw_core::{
    document::ShapeKind, Arrowhead, Color, ColorBlend, Command, Fill, Fountain, FountainKind,
    FountainRepeat, Pattern, PatternTile, Procedural, ProceduralParam, ProceduralParams,
    ProceduralPattern, Stop, Stroke, Texture, TextureKind,
};

/// The vertical strip of docker tabs on the right edge of the window.
/// Height of a docker's title bar (it runs across the tab column too).
pub const TITLE_H: f32 = 27.0;
/// The slot at the top of each tab where its close cross shows while
/// the pointer is over the tab (#docker tabs).
const CLOSE_SLOT: f32 = 16.0;

/// Where a tab's close cross sits: the slot at the top of the tab `bg`.
pub fn tab_close_rect(bg: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(bg.min.x, bg.min.y + 2.0),
        egui::vec2(bg.width(), CLOSE_SLOT - 2.0),
    )
}

/// The width of a docker tab column.
pub const STRIP_W: f32 = 27.0;

/// The fill of a docker tab: the docker in front takes the theme's
/// colour for the tab in front, the light blue of the document tab in
/// front on the light themes (the High Contrast theme's highlight
/// there). It was the frame colour of windows, a grey hardly apart from
/// the other tabs (#2316).
pub(crate) fn tab_fill(active: bool, hovered: bool) -> egui::Color32 {
    use crate::theme::{Theme, Tokens};
    if active {
        if crate::theme::theme() == Theme::HighContrast {
            Tokens::window_border()
        } else {
            Tokens::tab_active()
        }
    } else if hovered {
        Tokens::icon_off()
    } else {
        Tokens::tab()
    }
}

/// A docker tab column: the open dockers of the column at `place` as
/// vertical tabs (an icon on top, the name running downwards), grey, the
/// open one in the colour of the tab in front ([`tab_fill`]), then the
/// button that adds a docker. A tab dragged moves its docker (see
/// [`drag_overlay`]).
pub fn tab_strip(app: &mut App, ui: &mut Ui, place: DockerPlace) {
    let full = ui.max_rect();
    // The title bar's part and the line along the docker.
    ui.painter()
        .rect_filled(full, 0.0, crate::theme::Tokens::tab_strip());
    let line_x = if strip_at_left(place) {
        full.right() - 0.5
    } else {
        full.left() + 0.5
    };
    ui.painter().vline(
        line_x,
        (full.top() + TITLE_H)..=full.bottom(),
        egui::Stroke::new(1.0, crate::theme::Tokens::window_border()),
    );
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
    ui.add_space(TITLE_H);
    // More tabs than the column's height: the column slides, with
    // arrows at its ends and the mouse wheel over it, so the last tabs
    // and the add button stay in reach (#950).
    let salt = format!("docker_tabs_{}", place as u8);
    crate::ui::overflow::column(ui, &salt, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        let (tabs, front, shown) = app.docker_column(place);
        let tabs = tabs.to_vec();
        let mut laid = Vec::with_capacity(tabs.len());
        for tab in tabs {
            let name = tr(tab.key());
            let active = shown && front == tab;
            let galley = ui.painter().layout_no_wrap(
                name.clone(),
                egui::FontId::proportional(12.0),
                Tokens::text(),
            );
            let w = galley.size().x;
            let gh = galley.size().y;
            let icon_h = if tab_icon(tab).is_some() { 24.0 } else { 0.0 };
            // A slot at the top of the tab for the close cross, shown while
            // the pointer is over the tab.
            let h = CLOSE_SLOT + 14.0 + icon_h + w;
            let (r, resp) =
                ui.allocate_exact_size(egui::vec2(full.width(), h), egui::Sense::click_and_drag());
            laid.push((tab, r));
            let bg = if strip_at_left(place) {
                egui::Rect::from_min_max(r.min, egui::pos2(r.max.x - 1.0, r.max.y))
            } else {
                egui::Rect::from_min_max(egui::pos2(r.min.x + 1.0, r.min.y), r.max)
            };
            let fill = tab_fill(active, resp.hovered());
            ui.painter().rect_filled(bg, 0.0, fill);
            let close = tab_close_rect(bg);
            let over_close = resp.hover_pos().is_some_and(|p| close.contains(p));
            if resp.hovered() {
                let col = if over_close {
                    Tokens::text()
                } else {
                    Tokens::text_dim()
                };
                let c = close.center();
                let s = egui::Stroke::new(1.3, col);
                ui.painter()
                    .line_segment([c + egui::vec2(-3.5, -3.5), c + egui::vec2(3.5, 3.5)], s);
                ui.painter()
                    .line_segment([c + egui::vec2(3.5, -3.5), c + egui::vec2(-3.5, 3.5)], s);
            }
            let resp = if over_close {
                resp.rich_tip(tr("docker.close_docker"))
            } else {
                resp.rich_tip(format!("{name}\n{}", tr("docker.tab_desc")))
            };
            if let Some(icon) = tab_icon(tab) {
                let ir = egui::Rect::from_center_size(
                    egui::pos2(bg.center().x, r.min.y + CLOSE_SLOT + 7.0 + icon_h / 2.0),
                    egui::vec2(16.0, 16.0),
                );
                icon(ui.painter(), ir);
            }
            let text_top = r.min.y + CLOSE_SLOT + 7.0 + icon_h;
            let mut ts = egui::epaint::TextShape::new(
                egui::pos2(bg.center().x + gh / 2.0, text_top),
                galley,
                Tokens::text(),
            );
            if active && crate::theme::theme() == crate::theme::Theme::HighContrast {
                ts.override_text_color = Some(crate::theme::on_highlight());
            }
            ts.angle = std::f32::consts::FRAC_PI_2;
            ui.painter().add(ts);
            // A tab dragged moves its docker; not when a finger slides a
            // long column along its length to reach the tabs below (#1997).
            if resp.drag_started_by(egui::PointerButton::Primary)
                && !crate::ui::overflow::finger_sliding(ui.ctx())
            {
                let grab = resp
                    .interact_pointer_pos()
                    .map(|p| p - r.min)
                    .unwrap_or(egui::vec2(12.0, 12.0));
                app.docker_drag = Some(DockerDrag {
                    tab,
                    grab: egui::vec2(grab.x.min(60.0), grab.y.min(TITLE_H)),
                    last: None,
                });
            }
            if resp.clicked() {
                let on_close = resp
                    .interact_pointer_pos()
                    .is_some_and(|p| close.contains(p));
                if on_close {
                    app.close_docker(tab);
                } else if active {
                    app.set_docker_column(place, false, None);
                } else {
                    app.set_docker_column(place, true, Some(tab));
                }
            }
            egui::Popup::context_menu(&resp)
                .id(egui::Id::new(("docker_tab_menu", tab.key())))
                .style(crate::ui::menus::menu_popup_style)
                .show(|ui| {
                    crate::ui::menus::body(ui, |ui| {
                        place_menu(app, ui, tab);
                        crate::ui::menus::sep(ui);
                        if crate::ui::menus::item(ui, "docker.close_docker", "", true) {
                            app.close_docker(tab);
                        }
                    })
                });
        }
        // Where the tabs lie this frame, within the part of the column on
        // screen: a tab dragged onto the column goes in between the two
        // it is dropped between (#1051).
        let shown_part = ui.clip_rect();
        let strip = egui::Rect::from_x_y_ranges(
            full.x_range(),
            shown_part.top().max(full.top() + TITLE_H)..=shown_part.bottom().min(full.bottom()),
        );
        let nr = ui.ctx().cumulative_frame_nr();
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                strip_id(place),
                StripLayout {
                    frame: nr,
                    strip,
                    tabs: laid,
                },
            )
        });
        // The panel list: three dots.
        ui.add_space(8.0);
        let (r, _) = ui.allocate_exact_size(egui::vec2(full.width(), 22.0), egui::Sense::hover());
        let resp = ui.interact(r, more_button_id(place), egui::Sense::click());
        let c = r.center() + egui::vec2(0.5, 0.0);
        if resp.hovered() {
            ui.painter().rect_filled(
                egui::Rect::from_center_size(c, egui::vec2(18.0, 18.0)),
                0.0,
                crate::theme::Tokens::icon_off(),
            );
        }
        for dx in [-4.5, 0.0, 4.5] {
            ui.painter().circle_filled(
                c + egui::vec2(dx, 0.0),
                1.4,
                crate::theme::Tokens::text_dim(),
            );
        }
        let resp = resp.rich_tip(tr("docker.add_docker"));
        // The panels in the groups of the Window menu, each row a box
        // ticked at a panel open anywhere, the panel's picture (the one
        // its tab shows, or a blank where it has none), its name: ticking
        // opens it in front of this column, unticking closes it.
        egui::Popup::menu(&resp)
            .id(more_menu_id(place))
            .style(crate::ui::menus::menu_popup_style)
            .show(|ui| {
                crate::ui::menus::body(ui, |ui| {
                    let row = |ui: &mut Ui, app: &mut App, tab: DockerTab| {
                        let open = app.docker_open(tab);
                        let pic = tab_icon(tab);
                        let pic = pic
                            .as_ref()
                            .map(|f| f as &dyn Fn(&egui::Painter, egui::Rect));
                        if crate::ui::menus::check_pic(ui, tab.key(), open, pic) {
                            if open {
                                app.close_docker(tab);
                            } else {
                                app.move_docker(tab, place, None);
                            }
                        }
                    };
                    for (i, group) in DockerTab::MENU_GROUPS.iter().enumerate() {
                        if i > 0 {
                            crate::ui::menus::sep(ui);
                        }
                        for tab in group.iter() {
                            row(ui, app, *tab);
                        }
                        if i == 5 {
                            crate::ui::menus::sep(ui);
                            for tab in DockerTab::EFFECTS_MENU {
                                row(ui, app, tab);
                            }
                        }
                    }
                })
            });
    });
}

/// The three-dot button at the end of the tab column at `place`.
pub fn more_button_id(place: DockerPlace) -> egui::Id {
    egui::Id::new(("docker_more", place as u8))
}

/// The panel list the three-dot button of the column at `place` opens.
pub fn more_menu_id(place: DockerPlace) -> egui::Id {
    egui::Id::new(("docker_add_menu", place as u8))
}

/// A column's tab strip as last drawn: the part of it on screen and its
/// tabs' rectangles.
#[derive(Clone)]
struct StripLayout {
    frame: u64,
    strip: egui::Rect,
    tabs: Vec<(DockerTab, egui::Rect)>,
}

fn strip_id(place: DockerPlace) -> egui::Id {
    egui::Id::new(("docker_tab_strip", place as u8))
}

/// A docker dragged with the pointer at `p` over a column's tab strip
/// (drawn this frame or the last): the column, the place among its
/// other tabs it would take, and the line marking that place.
fn strip_drop(
    ctx: &egui::Context,
    p: egui::Pos2,
    dragged: DockerTab,
) -> Option<(DockerPlace, usize, egui::Rect)> {
    let nr = ctx.cumulative_frame_nr();
    [
        DockerPlace::Right,
        DockerPlace::RightInner,
        DockerPlace::Left,
        DockerPlace::LeftInner,
    ]
    .into_iter()
    .find_map(|place| {
        let l = ctx.data(|d| d.get_temp::<StripLayout>(strip_id(place)))?;
        if l.frame + 1 < nr || !l.strip.contains(p) {
            return None;
        }
        let others: Vec<egui::Rect> = l
            .tabs
            .iter()
            .filter(|(t, _)| *t != dragged)
            .map(|(_, r)| *r)
            .collect();
        let index = others.iter().filter(|r| r.center().y < p.y).count();
        let y = match (
            index.checked_sub(1).and_then(|i| others.get(i)),
            others.get(index),
        ) {
            (_, Some(next)) => next.top(),
            (Some(last), None) => last.bottom(),
            (None, None) => l.strip.top(),
        };
        // A tab slid out of view: the line stays at the strip's end.
        let y = y.clamp(l.strip.top() + 1.5, l.strip.bottom() - 1.5);
        let line = egui::Rect::from_x_y_ranges(l.strip.x_range(), (y - 1.5)..=(y + 1.5));
        Some((place, index, line))
    })
}

/// The places a docker can go, a dot at the one it has (a tab's menu and
/// a docker's title menu). A side's second column is offered while the
/// column at that side's edge holds another docker (a docker alone in
/// it would only come back there).
fn place_menu(app: &mut App, ui: &mut Ui, tab: DockerTab) {
    let current = app.docker_place(tab);
    for (key, place) in [
        ("bars.right", DockerPlace::Right),
        ("docker.right_inner", DockerPlace::RightInner),
        ("bars.left", DockerPlace::Left),
        ("docker.left_inner", DockerPlace::LeftInner),
        ("bars.floating", DockerPlace::Floating),
    ] {
        let enabled = !place.is_inner() || can_take_inner(app, place, tab);
        if crate::ui::menus::radio_if(ui, key, "", current == Some(place), enabled)
            && current != Some(place)
        {
            app.move_docker(tab, place, None);
        }
    }
}

/// `tab` can go into the second column at `inner`: the column is there
/// already, or the column at that side's edge holds a docker other than
/// `tab` for it to stand beside.
fn can_take_inner(app: &App, inner: DockerPlace, tab: DockerTab) -> bool {
    !app.docker_column(inner).0.is_empty()
        || app.docker_column(inner.edge()).0.iter().any(|t| *t != tab)
}

/// The collapse button's two triangles in `r`: pointing right for a
/// column at the window's right edge, left for one at the left edge.
pub(crate) fn fold_arrows(r: egui::Rect, at_left: bool) -> [[egui::Pos2; 3]; 2] {
    let dir = if at_left { -1.0 } else { 1.0 };
    [-3.5f32, 3.0].map(|dx| {
        let c = r.center() + egui::vec2(dx * dir, 0.0);
        [
            c + egui::vec2(-2.5 * dir, -4.5),
            c + egui::vec2(2.5 * dir, 0.0),
            c + egui::vec2(-2.5 * dir, 4.5),
        ]
    })
}

/// A column's tab strip lies at the window's left edge: the other
/// column's (the default column's in a right-to-left language).
/// A side's second column has its strip at the same side as the column at
/// that side's edge.
pub(crate) fn strip_at_left(place: DockerPlace) -> bool {
    (place.edge() == DockerPlace::Left) != crate::i18n::is_rtl()
}

/// Where the window drawing keeps, each frame, the docker columns on
/// screen (their places and rects: the tab strip and the docker), for the
/// drop targets of a dragged docker and of the palette.
pub const DOCKER_COLUMNS: &str = "docker_column_rects";

/// The docker columns the last layout put on screen (see
/// [`DOCKER_COLUMNS`]).
pub fn column_rects(ctx: &egui::Context) -> Vec<(DockerPlace, egui::Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(DockerPlace, egui::Rect)>>(egui::Id::new(DOCKER_COLUMNS)))
        .unwrap_or_default()
}

/// Where a docker dropped with the pointer at `p` goes, in a window
/// `screen` wide with the docker columns `columns` on screen: over a
/// column into it; within [`crate::ui::bars::EDGE_ZONE`] of a side the
/// column there; within that distance of the drawing's side of a side's
/// only column a second column there, beside it; elsewhere floating.
pub fn drop_target(
    screen: egui::Rect,
    columns: &[(DockerPlace, egui::Rect)],
    p: egui::Pos2,
) -> DockerPlace {
    let zone = crate::ui::bars::EDGE_ZONE;
    if let Some((place, _)) = columns.iter().find(|(_, r)| r.contains(p)) {
        return *place;
    }
    let (left, right) = if crate::i18n::is_rtl() {
        (DockerPlace::Right, DockerPlace::Left)
    } else {
        (DockerPlace::Left, DockerPlace::Right)
    };
    if p.x <= screen.left() + zone {
        return left;
    }
    if p.x >= screen.right() - zone {
        return right;
    }
    for (place, r) in columns {
        if place.is_inner() || columns.iter().any(|(q, _)| *q == place.inner()) {
            continue;
        }
        let gap = if strip_at_left(*place) {
            p.x - r.right()
        } else {
            r.left() - p.x
        };
        if (0.0..=zone).contains(&gap) && p.y >= r.top() && p.y <= r.bottom() {
            return place.inner();
        }
    }
    DockerPlace::Floating
}

/// The band (or, floating, the window) a dragged docker would take: a
/// column on screen, a new second column beside the side's column, or a
/// column at the window's edge.
fn drop_zone(
    screen: egui::Rect,
    menu_h: f32,
    columns: &[(DockerPlace, egui::Rect)],
    place: DockerPlace,
    p: egui::Pos2,
    grab: egui::Vec2,
) -> egui::Rect {
    let top = screen.top() + menu_h;
    let w = STRIP_W + 340.0;
    if let Some((_, r)) = columns.iter().find(|(q, _)| *q == place) {
        return *r;
    }
    if place.is_inner() {
        if let Some((_, r)) = columns.iter().find(|(q, _)| *q == place.edge()) {
            return if strip_at_left(place) {
                egui::Rect::from_min_max(
                    egui::pos2(r.right(), r.top()),
                    egui::pos2(r.right() + w, r.bottom()),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(r.left() - w, r.top()),
                    egui::pos2(r.left(), r.bottom()),
                )
            };
        }
    }
    match place {
        DockerPlace::Floating => {
            egui::Rect::from_min_size(p - grab, crate::app::FloatingDocker::DEFAULT_SIZE)
        }
        place if strip_at_left(place) => egui::Rect::from_min_max(
            egui::pos2(screen.left(), top),
            egui::pos2(screen.left() + w, screen.bottom()),
        ),
        _ => egui::Rect::from_min_max(
            egui::pos2(screen.right() - w, top),
            egui::pos2(screen.right(), screen.bottom()),
        ),
    }
}

/// A docker being moved: the band or window it would take shows under the
/// pointer, and the release puts it there (also when the pointer has left
/// the window: the last place it was seen counts).
pub fn drag_overlay(app: &mut App, ctx: &egui::Context, menu_h: f32) {
    let Some(mut drag) = app.docker_drag else {
        return;
    };
    let screen = ctx.content_rect();
    let (pointer, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
    let Some(p) = pointer.or(drag.last) else {
        if !down {
            app.docker_drag = None;
        }
        return;
    };
    drag.last = Some(p);
    app.docker_drag = Some(drag);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("docker_drag_overlay"),
    ));
    let fill = egui::Color32::from_rgba_unmultiplied(0x1E, 0x78, 0xD7, 46);
    let edge = egui::Stroke::new(1.5, egui::Color32::from_rgb(0x1E, 0x78, 0xD7));
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    // Over a column's tabs: a line between the two tabs it would go
    // between, and the release puts it there, so the tabs of a column
    // can be put in any order (#1051).
    if let Some((place, index, line)) = strip_drop(ctx, p, drag.tab) {
        painter.rect_filled(line, 0.0, edge.color);
        if !down {
            app.docker_drag = None;
            app.insert_docker(drag.tab, place, index);
        }
        return;
    }
    let columns = column_rects(ctx);
    let mut place = drop_target(screen, &columns, p);
    // A docker alone in its column, dropped beside it: it stays.
    if place.is_inner() && !can_take_inner(app, place, drag.tab) {
        place = place.edge();
    }
    let zone = drop_zone(screen, menu_h, &columns, place, p, drag.grab);
    painter.rect(zone, 2.0, fill, edge, egui::StrokeKind::Inside);
    if !down {
        app.docker_drag = None;
        let at = (place == DockerPlace::Floating).then_some(zone.min);
        // Dropped where it already is: nothing moves.
        let same = app.docker_place(drag.tab) == Some(place) && place != DockerPlace::Floating;
        if !same {
            app.move_docker(drag.tab, place, at);
        }
    }
}

/// The floating dockers, each in a window of its own with a title bar
/// (its name; a menu with its places; close), resizable at its edges.
/// Dragging the title moves it (near a side it docks there).
pub fn floating_dockers(app: &mut App, ctx: &egui::Context) {
    let tabs: Vec<DockerTab> = app.floating_dockers.iter().map(|f| f.tab).collect();
    for tab in tabs {
        let Some(fd) = app.floating_dockers.iter_mut().find(|f| f.tab == tab) else {
            continue;
        };
        let screen = ctx.content_rect();
        let mut pos = fd.pos;
        pos.x = pos
            .x
            .clamp(screen.left(), (screen.right() - 60.0).max(screen.left()));
        pos.y = pos
            .y
            .clamp(screen.top(), (screen.bottom() - TITLE_H).max(screen.top()));
        let size = fd.size;
        let place = std::mem::take(&mut fd.place);
        let mut window = egui::Window::new(tr(tab.key()))
            .id(egui::Id::new(("floating_docker", tab.key())))
            .title_bar(false)
            .collapsible(false)
            .movable(false)
            .resizable(true)
            .default_size(size)
            .default_pos(pos)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::Tokens::field())
                    .stroke(egui::Stroke::new(
                        1.0,
                        crate::theme::Tokens::window_border(),
                    ))
                    .shadow(crate::theme::shadow(egui::epaint::Shadow {
                        offset: [0, 3],
                        blur: 10,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(50),
                    })),
            );
        if place {
            window = window.current_pos(pos).fixed_size(size);
        }
        let shown = window.show(ctx, |ui| {
            ui.set_min_size(ui.available_size());
            docker_title(app, ui, tab, DockerPlace::Floating);
            let content = ui.available_rect_before_wrap();
            let inner = content.shrink2(egui::vec2(8.0, 6.0));
            let mut page = ui.new_child(egui::UiBuilder::new().max_rect(inner));
            page.set_clip_rect(content.intersect(ui.clip_rect()));
            ui.advance_cursor_after_rect(content);
            docker_page(app, &mut page, tab);
        });
        if let Some(shown) = shown {
            if let Some(fd) = app.floating_dockers.iter_mut().find(|f| f.tab == tab) {
                let r = shown.response.rect;
                if r.is_finite() && r.width() >= 120.0 && r.height() >= 120.0 {
                    fd.pos = r.min;
                    fd.size = r.size();
                }
            }
        }
    }
}

type TabIcon = fn(&egui::Painter, egui::Rect);

/// The small picture above a docker tab's name.
fn tab_icon(tab: DockerTab) -> Option<TabIcon> {
    match tab {
        DockerTab::Hints => Some(|p, r| {
            // A pointer with a question mark.
            let s = egui::Stroke::new(1.2, Tokens::icon());
            let o = r.min + egui::vec2(2.0, 1.0);
            let pts = vec![
                o,
                o + egui::vec2(0.0, 11.0),
                o + egui::vec2(3.0, 8.0),
                o + egui::vec2(5.5, 13.0),
                o + egui::vec2(7.0, 12.2),
                o + egui::vec2(4.6, 7.4),
                o + egui::vec2(8.5, 7.4),
            ];
            p.add(egui::epaint::PathShape::convex_polygon(
                pts,
                Tokens::icon(),
                s,
            ));
            p.text(
                r.min + egui::vec2(12.5, 3.5),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(10.0),
                Tokens::icon(),
            );
        }),
        DockerTab::Properties => Some(|p, r| {
            // A pen over a ruled sheet.
            let s = egui::Stroke::new(1.2, Tokens::icon());
            let c = r.center();
            p.line_segment([c + egui::vec2(-6.0, 6.0), c + egui::vec2(5.0, -5.0)], s);
            p.line_segment([c + egui::vec2(-6.0, 6.0), c + egui::vec2(-3.0, 5.0)], s);
            p.line_segment([c + egui::vec2(3.0, -7.0), c + egui::vec2(7.0, -3.0)], s);
            for y in [2.0, 5.0] {
                p.line_segment([c + egui::vec2(1.0, y), c + egui::vec2(7.0, y)], s);
            }
        }),
        DockerTab::Objects => Some(|p, r| {
            // Three stacked sheets.
            let c = r.center();
            for (k, dy) in [(0u8, 4.0f32), (1, 0.0), (2, -4.0)] {
                let pts = vec![
                    c + egui::vec2(-7.0, dy),
                    c + egui::vec2(0.0, dy - 3.5),
                    c + egui::vec2(7.0, dy),
                    c + egui::vec2(0.0, dy + 3.5),
                ];
                let fill = if k == 2 {
                    Tokens::icon()
                } else {
                    crate::theme::Tokens::field()
                };
                p.add(egui::epaint::PathShape::convex_polygon(
                    pts,
                    fill,
                    egui::Stroke::new(1.0, Tokens::icon()),
                ));
            }
        }),
        _ => None,
    }
}

/// The docker column at `place`: the title bar of the docker in front,
/// then its page in the width the column has.
pub fn dockers(app: &mut App, ui: &mut Ui, place: DockerPlace) {
    let full = ui.max_rect();
    let (_, front, _) = app.docker_column(place);
    docker_title(app, ui, front, place);
    let content = egui::Rect::from_min_max(egui::pos2(full.left(), full.top() + TITLE_H), full.max);
    ui.painter()
        .rect_filled(content, 0.0, crate::theme::Tokens::field());
    // The docker keeps the width it was given (its default or the one the
    // user dragged it to): the page lies in a child that never widens the
    // panel, clipped to the docker, so no section can push it wider.
    let inner = content.shrink2(egui::vec2(8.0, 6.0));
    let mut page = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    page.set_clip_rect(content.intersect(ui.clip_rect()));
    ui.advance_cursor_after_rect(content);
    // The Welcome Screen is no drawing: the dockers that work on one say
    // so instead of showing the hidden drawing's selection (#482); Hints
    // still helps.
    if !app.drawing_shown() && front != DockerTab::Hints {
        page.label(
            egui::RichText::new(crate::i18n::tr("docker.no_drawing"))
                .color(crate::theme::Tokens::text_dim()),
        );
    } else {
        docker_page(app, &mut page, front);
    }
    // How wide the page's widgets came out, for the tests that check
    // that every section keeps to the docker.
    #[cfg(test)]
    {
        let need = page.min_rect().width() - inner.width();
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("docker_overflow"), need));
    }
}

/// A docker's title bar: its name, then collapse (a column's) and close in
/// grey at the right. Dragging it moves the docker; a right click lists
/// its places.
fn docker_title(app: &mut App, ui: &mut Ui, tab: DockerTab, place: DockerPlace) {
    let full = ui.max_rect();
    let (bar, resp) = ui.allocate_exact_size(
        egui::vec2(full.width(), TITLE_H),
        egui::Sense::click_and_drag(),
    );
    let resp = resp.rich_tip(format!("{}\n{}", tr(tab.key()), tr("docker.title_desc")));
    ui.painter()
        .rect_filled(bar, 0.0, crate::theme::Tokens::tab_strip());
    ui.painter().text(
        egui::pos2(bar.left() + 7.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        tr(tab.key()),
        egui::FontId::proportional(12.0),
        Tokens::text(),
    );
    if resp.drag_started_by(egui::PointerButton::Primary) {
        let grab = resp
            .interact_pointer_pos()
            .map(|p| p - bar.min)
            .unwrap_or(egui::vec2(12.0, 12.0));
        app.docker_drag = Some(DockerDrag {
            tab,
            grab: egui::vec2(grab.x.clamp(0.0, 300.0), grab.y.clamp(0.0, TITLE_H)),
            last: None,
        });
    }
    egui::Popup::context_menu(&resp)
        .id(egui::Id::new(("docker_title_menu", tab.key())))
        .style(crate::ui::menus::menu_popup_style)
        .show(|ui| {
            crate::ui::menus::body(ui, |ui| {
                place_menu(app, ui, tab);
                crate::ui::menus::sep(ui);
                if crate::ui::menus::item(ui, "docker.close_docker", "", true) {
                    app.close_docker(tab);
                }
            })
        });
    let grey = crate::theme::Tokens::text_dim();
    // Close, at the right end.
    let close = egui::Rect::from_center_size(
        egui::pos2(bar.right() - 12.0, bar.center().y),
        egui::vec2(20.0, 20.0),
    );
    let resp = ui
        .interact(
            close,
            egui::Id::new(("docker_close", tab.key())),
            egui::Sense::click(),
        )
        .rich_tip(tr("docker.close_docker"));
    let col = if resp.hovered() { Tokens::text() } else { grey };
    let c = close.center();
    let s = egui::Stroke::new(1.5, col);
    ui.painter()
        .line_segment([c + egui::vec2(-4.5, -4.5), c + egui::vec2(4.5, 4.5)], s);
    ui.painter()
        .line_segment([c + egui::vec2(4.5, -4.5), c + egui::vec2(-4.5, 4.5)], s);
    if resp.clicked() {
        app.close_docker(tab);
    }
    if place == DockerPlace::Floating {
        return;
    }
    // Collapse: two small triangles pointing at the window edge the
    // column folds toward (left for a column at the left).
    let fold = egui::Rect::from_center_size(
        egui::pos2(bar.right() - 36.0, bar.center().y),
        egui::vec2(20.0, 20.0),
    );
    let resp = ui
        .interact(
            fold,
            egui::Id::new(("docker_fold", place as u8)),
            egui::Sense::click(),
        )
        .rich_tip(tr("docker.collapse"));
    let col = if resp.hovered() { Tokens::text() } else { grey };
    for points in fold_arrows(fold, strip_at_left(place)) {
        ui.painter().add(egui::epaint::PathShape::convex_polygon(
            points.to_vec(),
            col,
            egui::Stroke::NONE,
        ));
    }
    if resp.clicked() {
        app.set_docker_column(place, false, None);
    }
}

/// The page of the docker `tab`: its own layout under the title bar.
fn docker_page(app: &mut App, ui: &mut Ui, tab: DockerTab) {
    // Hints scrolls its page above its own navigation bar.
    if tab == DockerTab::Hints {
        crate::ui::hints::hints_docker(app, ui);
        return;
    }
    // AI Generate keeps Generate at the bottom, under its own scroll.
    if tab == DockerTab::AiGenerate {
        crate::ui::ai_generate_docker::ai_generate(app, ui);
        return;
    }
    // Objects keeps its New Layer and Delete bar under the tree.
    if tab == DockerTab::Objects {
        crate::ui::objects_docker::objects_docker(app, ui);
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt(("docker_scroll", tab.key()))
        .show(ui, |ui| match tab {
            DockerTab::Properties => properties(app, ui),
            DockerTab::Hints | DockerTab::Objects => {}
            DockerTab::Transformations => crate::ui::transform_docker::transformations(app, ui),
            DockerTab::Undo => undo_docker(app, ui),
            other => crate::ui::dockers2::show(app, ui, other),
        });
}

fn color_row(ui: &mut Ui, label: &str, c: &mut Color) -> bool {
    color_row_mixed(ui, label, c, false)
}

/// [`color_row`] for objects whose colours differ when `mixed`: no colour
/// is shown, and the one picked goes to all of them.
fn color_row_mixed(ui: &mut Ui, label: &str, c: &mut Color, mixed: bool) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        // The picker opens beside the swatch and keeps the colour's
        // model.
        let before = ui.cursor().min;
        let ch = crate::ui::color_picker::color_button(ui, c);
        let swatch = egui::Rect::from_min_size(before, egui::vec2(40.0, 18.0));
        if mixed {
            crate::ui::field::mixed_swatch(ui, swatch);
            crate::ui::field::mixed_note(ui);
        } else {
            ui.label(
                egui::RichText::new(crate::app::color_description(*c))
                    .color(Tokens::text_dim())
                    .size(11.0),
            );
        }
        ch
    })
    .inner
}

/// Give the selection these further fills and outlines (one undo step).
fn set_extra_paint(app: &mut App, fills: Vec<Fill>, strokes: Vec<tracedraw_core::Stroke>) {
    let shapes = app.selection.clone();
    app.run(Command::SetExtraPaint {
        shapes,
        fills,
        strokes,
    });
}

/// The further fills of the selection (the first object's): each with
/// its editor and a cross; Add Fill puts a copy of the fill on top. It
/// shows only while the object has a fill (`has_fill`): the first fill
/// comes from the kinds above it, not from this button (#487).
/// The object's opacity beside its fill and its outline colour (#1818):
/// one slider, 100 % solid, for the whole object, fill and outline
/// together (an object has one transparency, so the two cannot differ).
/// It is the Transparency section's uniform transparency the other way
/// round, and a change makes it uniform for the whole object. With
/// another kind of transparency (a fountain, a pattern, a texture), or
/// kinds that differ across the selection, it is shown off: the
/// Transparency section changes those. `salt` keeps the two apart.
fn object_opacity_row(app: &mut App, ui: &mut Ui, salt: &str) {
    use crate::mixed::TransparencyField as F;
    let (mut set, current, mixed, has_selection) = crate::ui::toolbar::transparency_state(app);
    let plain = matches!(current, Some(0) | Some(1));
    let mut opacity = if current == Some(1) {
        100.0 - set.amount
    } else {
        100.0
    };
    let amount_mixed = current == Some(1) && mixed.contains(&F::Amount);
    ui.add_space(4.0);
    // One tip over the slider, its box and the box's arrows.
    let tip = crate::ui::tips::keyed("docker.object_opacity");
    let resp = ui
        .push_id(("object_opacity", salt), |ui| {
            crate::ui::tips::tip_over(ui, tip, |ui| {
                ui.add_enabled(
                    has_selection && plain,
                    crate::ui::Slider::new(&mut opacity, 0.0..=100.0)
                        .text(tr("docker.object_opacity"))
                        .suffix(" %")
                        .mixed(amount_mixed),
                )
            })
        })
        .inner;
    if resp.changed() {
        set.amount = (100.0 - opacity).clamp(0.0, 100.0);
        // The whole object: fill and outline.
        set.target = 2;
        app.patch_transparency(&set, &[F::Amount, F::Target]);
    }
}

/// Fill winding (#1211, #1919): on, the selected curves fill with the
/// nonzero rule, so where their contours overlap turning the same way the
/// fill stays (traced lettering made of overlapping parts); off, with the
/// even-odd rule, where an overlap is a hole. Shown for curves and
/// polygons (other shapes have no overlapping contours; text always
/// fills nonzero); a selection that differs shows the box mixed.
fn fill_winding_row(app: &mut App, ui: &mut Ui) {
    use tracedraw_core::shaping::FillRule;
    let ids: Vec<(tracedraw_core::ShapeId, FillRule)> = app
        .selected_shapes()
        .iter()
        // Curves and polygons: the outlines whose contours can overlap.
        .filter(|s| matches!(s.kind, ShapeKind::Path { .. } | ShapeKind::Polygon { .. }))
        .map(|s| (s.id, s.fill_rule))
        .collect();
    if ids.is_empty() {
        return;
    }
    let nonzero = ids.iter().filter(|(_, r)| *r == FillRule::NonZero).count();
    let mut on = nonzero == ids.len();
    let mixed = nonzero > 0 && nonzero < ids.len();
    let resp = ui
        .add(egui::Checkbox::new(&mut on, tr("docker.fill_winding")).indeterminate(mixed))
        .rich_tip(crate::ui::tips::keyed("docker.fill_winding"));
    if resp.changed() {
        app.run(Command::SetFillRule {
            shapes: ids.into_iter().map(|(id, _)| id).collect(),
            rule: if on {
                FillRule::NonZero
            } else {
                FillRule::EvenOdd
            },
        });
    }
}

/// Whether the painted objects overprint their fill (`fill`) or their
/// outline: `Some(true)` all of them, `Some(false)` none, `None` mixed or
/// nothing selected.
pub fn overprint_state(app: &App, fill: bool) -> Option<bool> {
    app.selection_overprint(fill)
}

/// The Overprint fill or Overprint outline checkbox of the docker (#1200),
/// as the Edit Fill and Outline Pen dialogs have it.
fn overprint_row(app: &mut App, ui: &mut Ui, fill: bool) {
    if app.selection.is_empty() {
        return;
    }
    let state = overprint_state(app, fill);
    let mut on = state.unwrap_or(false);
    let key = if fill {
        "edit_fill.overprint"
    } else {
        "outline_pen.overprint"
    };
    if ui
        .add(egui::Checkbox::new(&mut on, tr(key)).indeterminate(state.is_none()))
        .rich_tip(crate::ui::tips::pair(key, "docker.overprint_tip"))
        .changed()
    {
        app.set_overprint(fill, on);
    }
}

fn extra_fills(app: &mut App, ui: &mut Ui, first: &tracedraw_core::Shape, has_fill: bool) {
    let mut fills = first.extra_fills.clone();
    let mut changed = false;
    let mut remove = None;
    for (i, f) in fills.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(trf("docker.fill_n", &[("n", &(i + 2).to_string())]));
            if ui
                .small_button("\u{00D7}")
                .rich_tip(tr("docker.remove_fill"))
                .clicked()
            {
                remove = Some(i);
            }
        });
        ui.push_id(("extra_fill", i), |ui| {
            changed |= fill_editor(ui, f);
        });
    }
    if let Some(i) = remove {
        fills.remove(i);
        changed = true;
    }
    if has_fill {
        ui.add_space(4.0);
    }
    if has_fill
        && ui
            .button(tr("docker.add_fill"))
            .rich_tip(tr("docker.add_fill_tip"))
            .clicked()
    {
        fills.push(first.fill.clone());
        changed = true;
    }
    if changed {
        set_extra_paint(app, fills, first.extra_strokes.clone());
    }
}

/// The further outlines of the selection, as [`extra_fills`]: Add Outline
/// shows only while the object has an outline (`has_outline`); the
/// Outline box gives it its first one (#487).
fn extra_strokes(app: &mut App, ui: &mut Ui, first: &tracedraw_core::Shape, has_outline: bool) {
    let mut strokes = first.extra_strokes.clone();
    let custom = crate::ui::line_style::OutlineLib::of(app);
    let mut changed = false;
    let mut remove = None;
    for (i, st) in strokes.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(trf("docker.outline_n", &[("n", &(i + 2).to_string())]));
            if ui
                .small_button("\u{00D7}")
                .rich_tip(tr("docker.remove_outline"))
                .clicked()
            {
                remove = Some(i);
            }
        });
        ui.push_id(("extra_stroke", i), |ui| {
            let mut s = Some(st.clone());
            if outline_editor(ui, &mut s, &custom) {
                if let Some(s) = s {
                    *st = s;
                }
                changed = true;
            }
        });
    }
    if let Some(i) = remove {
        strokes.remove(i);
        changed = true;
    }
    if has_outline {
        ui.add_space(4.0);
    }
    if has_outline
        && ui
            .button(tr("docker.add_outline"))
            .rich_tip(tr("docker.add_outline_tip"))
            .clicked()
    {
        let base = first
            .stroke
            .clone()
            .unwrap_or_else(|| tracedraw_core::Stroke::hairline(tracedraw_core::Color::BLACK));
        strokes.push(base);
        changed = true;
    }
    if changed {
        set_extra_paint(app, first.extra_fills.clone(), strokes);
    }
}

/// Where the fill editors find the current colours (see
/// `App::current_colors`).
fn current_colors_id() -> egui::Id {
    egui::Id::new("fill_editor_current_colors")
}

/// A drop shadow in a line of the Properties docker: flat or the edge
/// it stands on, and its opacity.
pub(crate) fn shadow_summary(sh: &tracedraw_core::Shadow) -> String {
    use tracedraw_core::shadow::ShadowEdge;
    let kind = match sh.perspective.map(|p| p.edge) {
        None => "shadow.flat",
        Some(ShadowEdge::Bottom) => "shadow.edge_bottom",
        Some(ShadowEdge::Top) => "shadow.edge_top",
        Some(ShadowEdge::Left) => "shadow.edge_left",
        Some(ShadowEdge::Right) => "shadow.edge_right",
    };
    let opacity = format!("{:.0}", sh.opacity.clamp(0.0, 1.0) * 100.0);
    trf(
        "docker.shadow_summary",
        &[("kind", &tr(kind)), ("opacity", &opacity)],
    )
}

/// The area a closed object covers on the page, mm²: rectangles,
/// ellipses and pies, polygons and stars, and closed curves, holes taken
/// out as its fill rule has them; `None` for anything else (open curves,
/// text, pictures, tables, groups) (#2599).
pub(crate) fn selection_area_mm2(s: &tracedraw_core::Shape) -> Option<f64> {
    use tracedraw_core::ShapeKind as K;
    let closed = match &s.kind {
        K::Rect { .. } | K::Polygon { .. } => true,
        K::Ellipse { .. } | K::Path { .. } => !s.kind.is_open_curve(),
        _ => false,
    };
    if !closed {
        return None;
    }
    // A whole ellipse is drawn with four curves a little off the true
    // ellipse; its area is the true one, as a measure should be.
    if let K::Ellipse { rect, arc: None } = &s.kind {
        let a = std::f64::consts::PI * rect.width() * rect.height() / 4.0
            * s.transform.determinant().abs();
        return a.is_finite().then_some(a);
    }
    let a = tracedraw_core::shaping::filled_area(&s.page_path(), s.fill_rule);
    a.is_finite().then_some(a)
}

fn properties(app: &mut App, ui: &mut Ui) {
    let cc = app.current_colors;
    ui.data_mut(|d| d.insert_temp(current_colors_id(), cc));
    let shapes = app.selected_rc();
    if shapes.is_empty() {
        ui.label(egui::RichText::new(tr("docker.no_objects_selected")).color(Tokens::text_dim()));
        ui.add_space(6.0);
        ui.strong(tr("docker.default_properties"));
        ui.add_space(4.0);
        // The same sections, in the same order, as with an object
        // selected, so each row of paint kinds sits under its own title
        // (#487).
        if section_header(app, ui, "outline", &tr("docker.outline")) {
            ui.add_space(2.0);
            let mut stroke = app.default_stroke();
            let custom = crate::ui::line_style::OutlineLib::of(app);
            if outline_editor(ui, &mut stroke, &custom) {
                app.set_default_properties(tracedraw_core::DefaultKind::Graphic, |p| {
                    p.stroke = stroke
                });
            }
            ui.add_space(6.0);
        }
        if section_header(app, ui, "fill", &tr("docker.fill")) {
            ui.add_space(2.0);
            let mut fill = app.default_fill();
            if fill_editor(ui, &mut fill) {
                app.set_default_properties(tracedraw_core::DefaultKind::Graphic, |p| p.fill = fill);
            }
        }
        return;
    }
    let first = &shapes[0];
    ui.strong(format!(
        "{}{}",
        kind_name(&first.kind),
        if shapes.len() > 1 {
            trf(
                "docker.and_n_more",
                &[("n", &(shapes.len() - 1).to_string())],
            )
        } else {
            String::new()
        }
    ));
    let b = app.selection_bounds().unwrap_or_default();
    let o = app.to_ruler(tracedraw_core::geometry::Point::new(b.x0, b.y0));
    ui.label(
        egui::RichText::new(format!(
            "x {:.2}  y {:.2}  w {:.2}  h {:.2} {}",
            app.units.world_from_mm(o.x),
            app.units.world_from_mm(o.y),
            app.units.world_from_mm(b.width()),
            app.units.world_from_mm(b.height()),
            app.units.short()
        ))
        .monospace()
        .size(11.0),
    );
    // One closed object: the area it covers, in the document's unit
    // squared (#2599).
    if let Some(area) = shapes
        .first()
        .filter(|_| shapes.len() == 1)
        .and_then(selection_area_mm2)
    {
        let k = app.units.world_from_mm(1.0);
        ui.label(
            egui::RichText::new(format!(
                "{} {:.2} {}\u{b2}",
                tr("docker.area"),
                area * k * k,
                app.units.short()
            ))
            .monospace()
            .size(11.0),
        )
        .rich_tip(crate::ui::tips::keyed("docker.area"));
    }

    ui.separator();
    // A jump from elsewhere (F11 to the fill, the outline from the status
    // bar) opens that section.
    if let Some(i) = app.properties_open.take() {
        let id = if i == 0 { "fill" } else { "outline" };
        app.settings.properties_closed.retain(|c| c != id);
    }
    // The sections this object has, in the docker's order, with the
    // buttons that open and close them.
    let mut sections: Vec<(&'static str, &'static str, SectionIcon)> = Vec::new();
    // A QR code object's own settings come first.
    if first.qr.is_some() {
        sections.push(("qr", "qr.section", SectionIcon::Qr));
    }
    sections.extend([
        (
            "outline",
            "docker.outline",
            SectionIcon::Tool(crate::tools::Tool::OutlinePen),
        ),
        (
            "fill",
            "docker.fill",
            SectionIcon::Tool(crate::tools::Tool::InteractiveFill),
        ),
        (
            "transparency",
            "docker.transparency",
            SectionIcon::Tool(crate::tools::Tool::Transparency),
        ),
    ]);
    let corners = app.corners_as_seen(first);
    if corners.is_some() {
        sections.push((
            "corners",
            "kind.rectangle",
            SectionIcon::Tool(crate::tools::Tool::Rectangle),
        ));
    }
    // Bitmap effects: a bitmap's, or a vector object's.
    sections.push(("effects", "docker.fx", SectionIcon::Effects));
    // A drop shadow: what it is, and Edit Shadow to the Drop Shadow tool.
    let shadow = app.selected_shadow().map(|(_, sh, _)| sh);
    if shadow.is_some() {
        sections.push((
            "shadow",
            "tool.drop_shadow",
            SectionIcon::Tool(crate::tools::Tool::DropShadow),
        ));
    }
    if matches!(first.kind, ShapeKind::Text { .. }) {
        sections.push((
            "character",
            "docker.character",
            SectionIcon::Tool(crate::tools::Tool::Text),
        ));
    }
    sections.push(("summary", "docker.summary", SectionIcon::Summary));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (id, key, icon) in &sections {
            let open = !app.settings.properties_closed.iter().any(|c| c == id);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
            let fill = if open {
                Some(Tokens::tool_active())
            } else if resp.hovered() {
                Some(Tokens::tool_hover())
            } else {
                None
            };
            if let Some(c) = fill {
                ui.painter().rect_filled(rect, 2.0, c);
            }
            icon.paint(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
            );
            // A click shows the section alone, so the docker's content
            // is what was picked at the top; with Ctrl or Shift it opens
            // or closes beside the others, as the headers do (#2200).
            let tip = crate::ui::tips::pair(key, "docker.section_pick_hint");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(section_button_id(id), rect));
            if resp.rich_tip(tip).clicked() {
                let beside = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                let shown: Vec<&str> = sections.iter().map(|(id, _, _)| *id).collect();
                pick_section(app, id, &shown, beside);
            }
        }
    });
    ui.add_space(2.0);
    for (id, key, _) in sections {
        if !section_header(app, ui, id, &tr(key)) {
            continue;
        }
        ui.add_space(2.0);
        match id {
            "qr" => {
                if let Some(qr) = &first.qr {
                    qr_section(app, ui, first.id, qr);
                }
            }
            "outline" if app.text_selection_active() => {
                // Characters selected in the text being edited: their own
                // outline, and an edit goes to them alone.
                let old = app.text_selection_stroke().unwrap_or(None);
                let mut stroke = old.clone();
                let custom = crate::ui::line_style::OutlineLib::of(app);
                if outline_editor(ui, &mut stroke, &custom) {
                    app.text_apply_outline(stroke);
                }
            }
            "outline" => {
                // A group shows what is inside it; values that differ
                // across the selection show as mixed, and an edit changes
                // only what was edited on each object.
                let base = app.first_paint_target();
                let old = base
                    .as_ref()
                    .map_or(first.stroke.clone(), |b| b.stroke.clone());
                let mix = app.selection_stroke_mix();
                let mut stroke = old.clone();
                let custom = crate::ui::line_style::OutlineLib::of(app);
                let mut touched = Vec::new();
                if outline_editor_mixed(ui, &mut stroke, &custom, &mix, &mut touched) {
                    // Dragging in the picker (or a slider): one step, and
                    // a new colour joins the document palette on release.
                    let live = app.live_stroke.is_some() || ui.input(|i| i.pointer.any_down());
                    if live {
                        app.patch_selection_stroke_live(&old, &stroke, &touched);
                    } else {
                        app.patch_selection_stroke(&old, &stroke, &touched);
                    }
                }
                let has = stroke.is_some() || mix.presence;
                object_opacity_row(app, ui, "outline");
                extra_strokes(app, ui, first, has);
                // Overprint and the dialog's button on one row (#1200).
                ui.horizontal_wrapped(|ui| {
                    overprint_row(app, ui, false);
                    if ui
                        .button(tr("docker.outline_pen_dialog"))
                        .rich_tip(tr("docker.outline_pen_dialog_tip"))
                        .clicked()
                    {
                        app.open_outline_pen();
                    }
                });
            }
            "fill" if app.text_selection_active() => {
                // Characters selected in the text being edited: their own
                // fill, and an edit goes to them alone.
                let old = app.text_selection_fill().unwrap_or(Fill::None);
                let mut fill = old.clone();
                if fill_editor(ui, &mut fill) {
                    app.text_apply_fill(fill);
                }
            }
            "fill" => {
                let base = app.first_paint_target();
                let old = base.as_ref().map_or(first.fill.clone(), |b| b.fill.clone());
                let mix = app.selection_fill_mix();
                let mut fill = old.clone();
                let mut touched = Vec::new();
                // The drawing's patterns, fetched only when a picker needs
                // them (not every frame).
                let fetch = || app.pattern_library();
                let mut pickers = PatternPickers::new(&fetch);
                let mut edited =
                    fill_editor_mixed(ui, &mut fill, &mix, &mut touched, Some(&mut pickers));
                let action = pickers.action;
                // Where a fountain's nodes sit, in numbers (#1200).
                if let (Fill::Fountain(f), Some(b)) = (&mut fill, base.as_ref()) {
                    if let Some(bounds) = app.doc().shape_on_page(b.id).map(|s| s.bounds()) {
                        if crate::ui::fountain_place::placement_editor(ui, f, bounds) {
                            touched.push(FillField::Handles);
                            edited = true;
                        }
                    }
                }
                let has = !matches!(fill, Fill::None) || mix.kind;
                if let Some(action) = action {
                    app.run_pattern_action(action);
                }
                if edited {
                    // Dragging in the picker: one step, and the colour
                    // joins the document palette on release. A pattern's
                    // placement goes into each object's own pattern.
                    let live = app.live_fill.is_some() || ui.input(|i| i.pointer.any_down());
                    let placement_only = matches!(fill, Fill::Pattern(_))
                        && !touched.contains(&FillField::All)
                        && tracedraw_core::patch::fill_changes(&old, &fill)
                            .iter()
                            .all(|f| f.is_placement());
                    if live && !matches!(fill, Fill::Fountain(_)) && !placement_only {
                        app.apply_fill_live(fill);
                    } else {
                        app.patch_selection_fill(&old, &fill, &touched);
                    }
                }
                object_opacity_row(app, ui, "fill");
                fill_winding_row(app, ui);
                extra_fills(app, ui, first, has);
                // Overprint and the dialog's button on one row (#1200).
                ui.horizontal_wrapped(|ui| {
                    overprint_row(app, ui, true);
                    if ui
                        .button(tr("docker.edit_fill_dialog"))
                        .rich_tip(tr("docker.edit_fill_dialog_tip"))
                        .clicked()
                    {
                        app.open_edit_fill();
                    }
                });
            }
            "transparency" => {
                crate::ui::toolbar::transparency_section(app, ui);
            }
            "corners" => {
                if let Some(c) = corners {
                    rectangle_section(app, ui, c);
                }
            }
            "effects" => {
                let id = first.id;
                crate::ui::effect_dialog::fx_section(app, ui, id);
            }
            "shadow" => {
                if let Some(sh) = shadow {
                    ui.label(shadow_summary(&sh));
                    if ui
                        .button(tr("context.edit_shadow"))
                        .rich_tip(tr("docker.edit_shadow_tip"))
                        .clicked()
                    {
                        app.edit_shadow();
                    }
                }
            }
            "character" => {
                if let ShapeKind::Text { spans, .. } = &first.kind {
                    // The selected characters' span while text is edited,
                    // else the first span.
                    let chosen = app.text_selection_span();
                    if let Some(sp) = chosen.as_ref().or(spans.first()) {
                        let u = app.text_units();
                        // The styles set on the characters, after the
                        // weight and slant (#1209).
                        let mut styles: Vec<String> = Vec::new();
                        if sp.bold {
                            styles.push(tr("docker.bold"));
                        }
                        if sp.italic {
                            styles.push(tr("docker.italic"));
                        }
                        for (style, key) in [
                            (sp.underline, "docker.underline"),
                            (sp.strikethrough, "docker.strikethrough"),
                        ] {
                            if style.is_on() {
                                styles.push(format!(
                                    "{} ({})",
                                    tr(key),
                                    crate::ui::text_docker::line_style_name(style)
                                ));
                            }
                        }
                        if sp.all_caps {
                            styles.push(tr("docker.all_caps"));
                        }
                        if sp.enclosed {
                            styles.push(tr("docker.enclosed"));
                        }
                        if sp.overprint_fill.unwrap_or(first.overprint_fill) {
                            styles.push(tr("docker.overprint_fill"));
                        }
                        if sp.overprint_outline.unwrap_or(first.overprint_outline) {
                            styles.push(tr("docker.overprint_outline"));
                        }
                        let styles = styles.iter().map(|s| format!(" {s}")).collect::<String>();
                        ui.label(format!(
                            "{} {} {}{}",
                            sp.font_family,
                            crate::ui::field::format_number(
                                u.from_pt(sp.size_pt),
                                false,
                                0,
                                Some(2)
                            ),
                            u.short(),
                            styles
                        ));
                    }
                    ui.label(
                        egui::RichText::new(tr("docker.edit_with_text_tool"))
                            .color(Tokens::text_dim())
                            .size(11.0),
                    );
                }
            }
            "summary" => summary_section(app, ui, &shapes),
            _ => {}
        }
        ui.add_space(6.0);
    }
}

/// What the Summary section of the Properties docker says about the
/// selection (#1202).
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryInfo {
    /// How many objects are selected.
    pub objects: usize,
    /// The names of the layers holding them, each once, in layer order.
    pub layers: Vec<String>,
    /// The kinds of the objects, each once.
    pub kinds: Vec<String>,
    /// Width and height of the selection, mm.
    pub size: (f64, f64),
    /// The middle of the selection, in ruler coordinates (mm).
    pub center: tracedraw_core::geometry::Point,
    /// The centre of rotation, in ruler coordinates (mm).
    pub rotation_center: tracedraw_core::geometry::Point,
}

/// The Summary section's figures for the selection; `None` with nothing
/// selected.
pub fn summary_info(app: &App) -> Option<SummaryInfo> {
    let shapes = app.selected_shapes();
    if shapes.is_empty() {
        return None;
    }
    let b = app.selection_bounds()?;
    let doc = app.doc();
    let mut layers = Vec::new();
    for layer in doc.all_layers() {
        if shapes.iter().any(|s| layer_holds(layer, s.id)) && !layers.contains(&layer.name) {
            layers.push(layer.name.clone());
        }
    }
    let mut kinds: Vec<String> = Vec::new();
    for s in &shapes {
        let k = kind_name(&s.kind);
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let rotation = app.rotation_center().unwrap_or(b.center());
    Some(SummaryInfo {
        objects: shapes.len(),
        layers,
        kinds,
        size: (b.width(), b.height()),
        center: app.to_ruler(b.center()),
        rotation_center: app.to_ruler(rotation),
    })
}

/// Whether `layer` holds the shape `id`, inside groups and clip frames too.
fn layer_holds(layer: &tracedraw_core::document::Layer, id: tracedraw_core::ShapeId) -> bool {
    fn walk(s: &tracedraw_core::Shape, id: tracedraw_core::ShapeId) -> bool {
        if s.id == id {
            return true;
        }
        match &s.kind {
            ShapeKind::Group { children } => children.iter().any(|c| walk(c, id)),
            ShapeKind::ClipFrame { frame, contents } => {
                walk(frame, id) || contents.iter().any(|c| walk(c, id))
            }
            _ => false,
        }
    }
    layer.shapes.iter().any(|s| walk(s, id))
}

/// The Summary section: the selection in figures (count, layer, kind,
/// size, centre, centre of rotation), each object, Wrap paragraph text
/// with its style and offset, and Lock Object (#1202).
fn summary_section(app: &mut App, ui: &mut Ui, shapes: &[tracedraw_core::Shape]) {
    let Some(info) = summary_info(app) else {
        return;
    };
    let value = |ui: &mut Ui, key: &str, text: String| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr(key)).color(Tokens::text_dim()));
            ui.label(text);
        });
    };
    let u = &app.units;
    value(
        ui,
        "docker.summary_selection",
        trf(
            "docker.summary_objects",
            &[("n", &info.objects.to_string())],
        ),
    );
    value(ui, "docker.summary_layer", info.layers.join(", "));
    value(ui, "docker.summary_type", info.kinds.join(", "));
    value(
        ui,
        "docker.summary_size",
        format!(
            "{:.2} x {:.2} {}",
            u.world_from_mm(info.size.0),
            u.world_from_mm(info.size.1),
            u.short()
        ),
    );
    value(
        ui,
        "docker.summary_center",
        format!(
            "({:.2}, {:.2}) {}",
            u.world_from_mm(info.center.x),
            u.world_from_mm(info.center.y),
            u.short()
        ),
    );
    value(
        ui,
        "docker.summary_rotation_center",
        format!(
            "({:.2}, {:.2}) {}",
            u.world_from_mm(info.rotation_center.x),
            u.world_from_mm(info.rotation_center.y),
            u.short()
        ),
    );
    for s in shapes.iter() {
        ui.label(
            egui::RichText::new(format!(
                "{}  id {}  {}",
                kind_name(&s.kind),
                s.id.raw(),
                s.name.clone().unwrap_or_default()
            ))
            .size(11.0)
            .color(Tokens::text_dim()),
        );
    }
    ui.add_space(4.0);
    // Wrap paragraph text: None or a style; the offset while a style is on.
    let (on, current) = app.wrap_state();
    ui.label(tr("docker.summary_wrap"));
    let chosen = if on {
        tr(crate::ui::propbar::wrap_style_key(
            current.mode,
            current.side,
        ))
    } else {
        tr("context.wrap_none")
    };
    egui::ComboBox::from_id_salt("summary_wrap_style")
        .selected_text(chosen)
        .width(200.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(!on, tr("context.wrap_none")).clicked() && on {
                let shapes = app.selection.clone();
                app.run(Command::SetWrapText {
                    shapes,
                    wrap: false,
                });
            }
            for (mode, side) in tracedraw_core::TextWrap::STYLES {
                let here = on && current.mode == mode && current.side == side;
                if ui
                    .selectable_label(here, tr(crate::ui::propbar::wrap_style_key(mode, side)))
                    .clicked()
                {
                    app.set_wrap_style(mode, side);
                }
            }
        })
        .response
        .rich_tip(crate::ui::tips::pair(
            "docker.summary_wrap",
            "context.wrap_style",
        ));
    ui.add_enabled_ui(on, |ui| {
        crate::ui::propbar::wrap_offset_field(app, ui, current.offset)
    });
    ui.add_space(4.0);
    if ui
        .button(tr("menu.object.lock_object"))
        .rich_tip(crate::ui::tips::pair(
            "menu.object.lock_object",
            "docker.lock_object_tip",
        ))
        .clicked()
    {
        app.set_locked(true);
    }
}

/// The QR Code section of Properties: the kind of content and its
/// fields, the fills of the dark pixels and the background, the pixels'
/// outline, the pixel size, and Validate. Each change makes the code again
/// (one undo step); content that cannot be encoded is shown in red and
/// changes nothing.
fn qr_section(
    app: &mut App,
    ui: &mut Ui,
    id: tracedraw_core::ShapeId,
    qr: &tracedraw_core::QrCode,
) {
    use tracedraw_core::QrKind;
    let mut next = qr.clone();
    let mut changed = false;
    let msg_id = egui::Id::new(("qr_message", id.raw()));
    // A kind picked whose fields make no content yet is shown and edited
    // here while the object keeps its code (#1845).
    let draft_id = egui::Id::new(("qr_draft", id.raw()));
    let draft: Option<crate::qr::QrDraft> = ui.data(|d| d.get_temp(draft_id));
    let (kind, fields) = match &draft {
        Some(d) => (d.kind, d.fields.clone()),
        None => (qr.kind, qr.fields.clone()),
    };
    let mut content: Option<(QrKind, Vec<String>)> = None;
    crate::ui::keynav::ComboBox::from_id_salt(("qr_kind", id.raw()))
        .width(ui.available_width() - 8.0)
        .selected_text(tr(qr_kind_key(kind)))
        .show_ui(ui, |ui| {
            for k in QrKind::ALL {
                if ui.selectable_label(kind == k, tr(qr_kind_key(k))).clicked() && k != kind {
                    content = Some((k, k.carry(kind, &fields)));
                }
            }
        });
    // The fields are edited as text and committed when they lose focus or
    // Enter is pressed, so typing does not add an undo step per letter.
    for (i, key) in kind.fields().iter().enumerate() {
        let edit_id = egui::Id::new(("qr_field", id.raw(), i));
        let mut text: String = ui
            .data(|d| d.get_temp::<String>(edit_id))
            .unwrap_or_else(|| fields.get(i).cloned().unwrap_or_default());
        ui.label(egui::RichText::new(tr(key)).size(11.0));
        let multi = matches!(kind, QrKind::Text) || key.ends_with("message");
        let resp = if multi {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            )
        } else {
            ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY))
        };
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(edit_id, text.clone()));
        }
        let commit = resp.lost_focus() || (resp.changed() && !resp.has_focus());
        if commit {
            ui.data_mut(|d| d.remove::<String>(edit_id));
            let mut f = fields.clone();
            if f.len() <= i {
                f.resize(i + 1, String::new());
            }
            if f[i] != text {
                f[i] = text;
                content = Some((kind, f));
            }
        }
    }
    if let Some((k, f)) = content {
        match app.edit_qr_content(id, k, f) {
            Ok(Some(d)) => ui.data_mut(|data| {
                data.insert_temp(msg_id, (false, tr("dialog.qr_empty")));
                data.insert_temp(draft_id, d);
            }),
            Ok(None) => ui.data_mut(|data| {
                data.remove::<(bool, String)>(msg_id);
                data.remove::<crate::qr::QrDraft>(draft_id);
            }),
            Err(e) => ui.data_mut(|data| {
                data.insert_temp(msg_id, (false, e));
            }),
        }
        // The code just made is what the other settings below change.
        if let Some(q) = app.doc().find_shape(id).and_then(|s| s.qr.as_deref()) {
            next = q.clone();
        }
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tr("qr.dark_fill")).size(11.0));
    if fill_editor(ui, &mut next.dark) {
        changed = true;
    }
    ui.label(egui::RichText::new(tr("qr.light_fill")).size(11.0));
    if fill_editor(ui, &mut next.light) {
        changed = true;
    }
    ui.label(egui::RichText::new(tr("qr.outline")).size(11.0));
    let custom = crate::ui::line_style::OutlineLib::of(app);
    if outline_editor(ui, &mut next.outline, &custom) {
        changed = true;
    }
    ui.horizontal(|ui| {
        ui.label(tr("qr.pixel_size"));
        let units = app.units;
        let mut v = units.from_mm(next.pixel_mm);
        if ui
            .add(
                crate::ui::field::NumField::new(&mut v)
                    .speed(0.05)
                    .max_decimals(3)
                    .range(units.from_mm(0.1)..=units.from_mm(50.0))
                    .suffix(format!(" {}", units.short())),
            )
            .changed()
        {
            next.pixel_mm = units.to_mm(v).clamp(0.1, 50.0);
            changed = true;
        }
    });
    if ui
        .add_sized(
            [ui.available_width() - 8.0, 24.0],
            egui::Button::new(tr("qr.validate")),
        )
        .clicked()
    {
        let m = match crate::qr::validate(qr) {
            Ok(_) => (true, tr("qr.validate_ok")),
            Err(e) => (false, e),
        };
        ui.data_mut(|d| d.insert_temp(msg_id, m));
    }
    if changed {
        let result = app.set_qr_code(id, next);
        ui.data_mut(|d| match result {
            Ok(()) => {
                d.remove::<(bool, String)>(msg_id);
            }
            Err(e) => {
                d.insert_temp(msg_id, (false, e));
            }
        });
    }
    if let Some((ok, m)) = ui.data(|d| d.get_temp::<(bool, String)>(msg_id)) {
        let colour = if ok {
            egui::Color32::from_rgb(0x10, 0x7C, 0x10)
        } else {
            egui::Color32::from_rgb(0xC4, 0x2B, 0x1C)
        };
        ui.label(egui::RichText::new(m).color(colour).size(11.0));
    }
}

/// The name of a kind of QR content.
fn qr_kind_key(k: tracedraw_core::QrKind) -> &'static str {
    use tracedraw_core::QrKind;
    match k {
        QrKind::Url => "qr.kind_url",
        QrKind::Email => "qr.kind_email",
        QrKind::Phone => "qr.kind_phone",
        QrKind::Sms => "qr.kind_sms",
        QrKind::Contact => "qr.kind_contact",
        QrKind::Text => "qr.kind_text",
        QrKind::Location => "qr.kind_location",
    }
}

/// The picture on a Properties section's button.
#[derive(Debug, Clone, Copy)]
enum SectionIcon {
    Tool(crate::tools::Tool),
    Effects,
    Summary,
    Qr,
}

impl SectionIcon {
    fn paint(self, painter: &egui::Painter, r: egui::Rect) {
        match self {
            SectionIcon::Tool(t) => crate::ui::icons::draw(painter, r, t, Tokens::icon()),
            SectionIcon::Effects => {
                painter.text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    "fx",
                    egui::FontId::proportional(13.0),
                    Tokens::icon(),
                );
            }
            SectionIcon::Qr => {
                // Three finder squares and a few pixels.
                let px = r.width() / 7.0;
                let stroke = egui::Stroke::new(1.2, Tokens::icon());
                for (cx, cy) in [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)] {
                    let sq = egui::Rect::from_min_size(
                        r.min + egui::vec2(cx * px, cy * px),
                        egui::vec2(3.0 * px, 3.0 * px),
                    );
                    painter.rect_stroke(sq.shrink(0.6), 0.0, stroke, egui::StrokeKind::Inside);
                    painter.rect_filled(sq.shrink(px), 0.0, Tokens::icon());
                }
                for (cx, cy) in [(4.0, 4.0), (6.0, 5.0), (5.0, 6.0), (4.0, 6.0)] {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            r.min + egui::vec2(cx * px, cy * px),
                            egui::vec2(px, px),
                        ),
                        0.0,
                        Tokens::icon(),
                    );
                }
            }
            SectionIcon::Summary => {
                painter.circle_stroke(r.center(), 7.0, egui::Stroke::new(1.3, Tokens::icon()));
                painter.text(
                    r.center() + egui::vec2(0.0, 0.5),
                    egui::Align2::CENTER_CENTER,
                    "i",
                    egui::FontId::proportional(11.0),
                    Tokens::icon(),
                );
            }
        }
    }
}

/// Open or close a Properties section (kept in the preferences).
fn toggle_section(app: &mut App, id: &str) {
    let closed = &mut app.settings.properties_closed;
    if closed.iter().any(|c| c == id) {
        closed.retain(|c| c != id);
    } else {
        closed.push(id.to_string());
    }
}

/// Where the button of section `id` was drawn last (for tests).
#[cfg(test)]
fn section_button_id(id: &str) -> egui::Id {
    egui::Id::new(("properties_section_button", id.to_string()))
}

/// Every section the Properties docker can show, whatever is selected.
const PROPERTIES_SECTIONS: [&str; 9] = [
    "qr",
    "outline",
    "fill",
    "transparency",
    "corners",
    "effects",
    "shadow",
    "character",
    "summary",
];

/// A click on a section's button at the top of the Properties docker
/// (#2200). The docker then shows that section alone: every other one
/// is closed (those of other kinds of object too, so the docker stays
/// on the section picked when the selection changes) and only their
/// header bars remain. A click on the button of the only open section
/// of `shown` (the sections the selection has) opens them all again.
/// With `beside` (Ctrl or Shift held) the section opens or closes on its
/// own, as a click on its header does.
fn pick_section(app: &mut App, id: &str, shown: &[&str], beside: bool) {
    if beside {
        toggle_section(app, id);
        return;
    }
    let closed = &mut app.settings.properties_closed;
    let is_open = |closed: &[String], s: &str| !closed.iter().any(|c| c == s);
    let alone = is_open(closed, id) && shown.iter().all(|s| *s == id || !is_open(closed, s));
    closed.retain(|c| !PROPERTIES_SECTIONS.contains(&c.as_str()));
    if !alone {
        closed.extend(
            PROPERTIES_SECTIONS
                .iter()
                .filter(|s| **s != id)
                .map(|s| s.to_string()),
        );
    }
}

/// A Properties section's header: a bar with the open or closed triangle
/// and the title; a click opens or closes the section. True when open.
fn section_header(app: &mut App, ui: &mut Ui, id: &str, title: &str) -> bool {
    let open = !app.settings.properties_closed.iter().any(|c| c == id);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    let fill = if resp.hovered() {
        Tokens::tool_hover()
    } else {
        crate::theme::Tokens::control_fill()
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    let c = egui::pos2(rect.left() + 10.0, rect.center().y);
    let points = if open {
        vec![
            c + egui::vec2(-4.0, -2.0),
            c + egui::vec2(4.0, -2.0),
            c + egui::vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + egui::vec2(-2.0, -4.0),
            c + egui::vec2(3.0, 0.0),
            c + egui::vec2(-2.0, 4.0),
        ]
    };
    ui.painter().add(egui::epaint::PathShape::convex_polygon(
        points,
        Tokens::icon(),
        egui::Stroke::NONE,
    ));
    ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        crate::theme::bold(12.5),
        Tokens::text(),
    );
    let resp = resp.rich_tip_with(|| format!("{title}\n{}", tr("docker.section_desc")));
    if resp.clicked() {
        toggle_section(app, id);
        return !open;
    }
    open
}

/// The Properties docker's rectangle part: corner style, the four corner
/// sizes, editing them together and relative corner scaling, as on the
/// property bar.
fn rectangle_section(app: &mut App, ui: &mut Ui, c: tracedraw_core::Corners) {
    use crate::ui::propbar::{edit_corners, CornerEdit};
    use tracedraw_core::{CornerKind, Corners};
    let mut edit = None;
    // Values that differ across the selected rectangles show as mixed.
    let mix = app.selection_corner_mix().map_or([false; 6], |(_, m)| m);
    // The corner kinds one under the other, so that long names keep to
    // the docker's width.
    for (kind, key) in [
        (CornerKind::Round, "toolbar.corner_round"),
        (CornerKind::Scallop, "toolbar.corner_scallop"),
        (CornerKind::Chamfer, "toolbar.corner_chamfer"),
    ] {
        if ui.radio(!mix[0] && c.kind == kind, tr(key)).clicked() {
            edit = Some(CornerEdit::Kind(kind));
        }
    }
    let u = app.units;
    // The four radii as the corners lie, two by two, each named above
    // its field. Both columns take half the docker, and a name that does
    // not fit goes on to a second line rather than being cut (#487).
    let gap = 10.0;
    let column = ((ui.available_width() - gap) / 2.0).max(60.0);
    let field_w = column.min(96.0);
    egui::Grid::new("docker_corners")
        .num_columns(2)
        .spacing([gap, 4.0])
        .min_col_width(column)
        .max_col_width(column)
        .show(ui, |ui| {
            for pair in [
                [
                    (Corners::TOP_LEFT, "toolbar.corner_top_left"),
                    (Corners::TOP_RIGHT, "toolbar.corner_top_right"),
                ],
                [
                    (Corners::BOTTOM_LEFT, "toolbar.corner_bottom_left"),
                    (Corners::BOTTOM_RIGHT, "toolbar.corner_bottom_right"),
                ],
            ] {
                for (i, key) in pair {
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tr(key))
                                    .color(Tokens::text_dim())
                                    .size(11.0),
                            )
                            .wrap(),
                        );
                        let mut v = u.world_from_mm(c.radii[i]);
                        if mix.get(i + 1).copied().unwrap_or(false) {
                            // Empty until a size is typed, which goes to all.
                            if ui
                                .add(
                                    crate::ui::field::NumField::new(&mut v)
                                        .range(0.0..=10_000.0)
                                        .suffix(format!(" {}", u.short()))
                                        .width(field_w)
                                        .mixed(true),
                                )
                                .changed()
                            {
                                edit = Some(CornerEdit::Size(i, u.world_to_mm(v)));
                            }
                        } else {
                            let r = ui.add(
                                crate::ui::field::NumField::new(&mut v)
                                    .range(0.0..=10_000.0)
                                    .speed(0.1)
                                    .suffix(format!(" {}", u.short()))
                                    .width(field_w),
                            );
                            if r.changed() && (u.world_to_mm(v) - c.radii[i]).abs() > 1e-12 {
                                edit = Some(CornerEdit::Size(i, u.world_to_mm(v)));
                            }
                        }
                    });
                }
                ui.end_row();
            }
        });
    ui.checkbox(&mut app.corners_together, tr("toolbar.corners_together"));
    let mut relative = !c.fixed;
    if ui
        .add(
            egui::Checkbox::new(&mut relative, tr("toolbar.relative_corners"))
                .indeterminate(mix[5]),
        )
        .changed()
    {
        edit = Some(CornerEdit::Scaling(relative));
    }
    if let Some(e) = edit {
        edit_corners(app, e);
    }
}

/// The picture of a fill kind (0 none, 1 uniform, 2 gradient, 3 pattern,
/// 4 texture, 5 mesh, 6 procedural, 7 vector pattern, 8 bitmap pattern)
/// in a 16 x 16 square.
pub(crate) fn fill_kind_picture(painter: &egui::Painter, r: egui::Rect, kind: usize) {
    let edge = egui::Stroke::new(1.0, Tokens::icon());
    let sq = r.shrink(1.0);
    match kind {
        6 => {
            // An octagon of the procedural tiling, its flats on the edges.
            let s = sq.width();
            let k = s / (1.0 + std::f32::consts::SQRT_2) / 2.0;
            let (h, m) = (s / 2.0, sq.min);
            let at = |x: f32, y: f32| m + egui::vec2(x, y);
            let pts = vec![
                at(h - k, 0.0),
                at(h + k, 0.0),
                at(s, h - k),
                at(s, h + k),
                at(h + k, s),
                at(h - k, s),
                at(0.0, h + k),
                at(0.0, h - k),
            ];
            painter.add(egui::Shape::closed_line(
                pts,
                egui::Stroke::new(1.2, Tokens::icon()),
            ));
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
        }
        0 => {
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
            painter.line_segment(
                [sq.left_bottom(), sq.right_top()],
                egui::Stroke::new(1.3, egui::Color32::from_rgb(0xD0, 0x30, 0x30)),
            );
        }
        1 => {
            painter.rect_filled(sq, 0.0, Tokens::icon());
        }
        2 => {
            let mut mesh = egui::Mesh::default();
            let dark = Tokens::icon();
            let light = egui::Color32::WHITE;
            mesh.colored_vertex(sq.left_top(), dark);
            mesh.colored_vertex(sq.right_top(), light);
            mesh.colored_vertex(sq.right_bottom(), light);
            mesh.colored_vertex(sq.left_bottom(), dark);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
        }
        3 => {
            let n = 4;
            let w = sq.width() / n as f32;
            for y in 0..n {
                for x in 0..n {
                    if (x + y) % 2 == 0 {
                        let cell = egui::Rect::from_min_size(
                            sq.min + egui::vec2(x as f32 * w, y as f32 * w),
                            egui::vec2(w, w),
                        );
                        painter.rect_filled(cell, 0.0, Tokens::icon());
                    }
                }
            }
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
        }
        4 => {
            // Speckles of a texture.
            for (x, y, rad) in [
                (3.0, 4.0, 1.6),
                (9.0, 3.0, 1.2),
                (12.0, 9.0, 1.8),
                (5.0, 11.0, 1.4),
                (8.5, 7.5, 1.0),
                (12.5, 13.0, 1.0),
            ] {
                painter.circle_filled(sq.min + egui::vec2(x, y), rad, Tokens::icon());
            }
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
        }
        7 | 8 => crate::ui::pattern_pick::draw_pattern_kind(painter, r, kind),
        _ => {
            painter.rect_stroke(sq, 0.0, edge, egui::StrokeKind::Inside);
            for t in [1.0 / 3.0, 2.0 / 3.0] {
                let x = sq.left() + sq.width() * t;
                let y = sq.top() + sq.height() * t;
                painter.line_segment(
                    [egui::pos2(x, sq.top()), egui::pos2(x + 1.5, sq.bottom())],
                    egui::Stroke::new(0.9, Tokens::icon()),
                );
                painter.line_segment(
                    [egui::pos2(sq.left(), y), egui::pos2(sq.right(), y - 1.5)],
                    egui::Stroke::new(0.9, Tokens::icon()),
                );
            }
        }
    }
}

/// The fill type shown in the type row: `fill_kind`, with a pattern's
/// kind told apart (3 two-colour, 7 vector, 8 bitmap; 6 is the
/// procedural fill).
pub(crate) fn fill_kind_shown(fill: &Fill) -> usize {
    match fill {
        Fill::Pattern(Pattern::Vector { .. }) => 7,
        Fill::Pattern(Pattern::Bitmap { .. }) => 8,
        _ => tracedraw_core::patch::fill_kind(fill),
    }
}

/// The fill types of the type row, in order: none, uniform, gradient,
/// two-colour pattern, vector pattern, bitmap pattern, texture, mesh,
/// procedural.
const FILL_KINDS: [(usize, &str); 9] = [
    (0, "docker.fill_none"),
    (1, "docker.fill_uniform"),
    (2, "docker.fill_fountain"),
    (3, "docker.fill_pattern"),
    (7, "docker.fill_vector_pattern"),
    (8, "docker.fill_bitmap_pattern"),
    (4, "docker.fill_texture"),
    (5, "docker.fill_mesh"),
    (6, "docker.fill_procedural"),
];

fn fill_editor(ui: &mut Ui, fill: &mut Fill) -> bool {
    fill_editor_mixed(ui, fill, &FillMix::default(), &mut Vec::new(), None)
}

/// [`fill_editor`] for a selection whose fills differ as `mix` says: no
/// type is highlighted when the types differ (and no settings are shown
/// until one is picked), a uniform colour that differs shows no colour.
/// A choice that equals the value shown still applies to every object:
/// it is added to `touched`.
fn fill_editor_mixed(
    ui: &mut Ui,
    fill: &mut Fill,
    mix: &FillMix,
    touched: &mut Vec<FillField>,
    mut pickers: Option<&mut PatternPickers>,
) -> bool {
    let mut changed = false;
    let mut kind = fill_kind_shown(fill);
    let shown_kind = (!mix.kind).then_some(kind);
    // The kinds as pictures: none, uniform, gradient, two-colour, vector
    // and bitmap pattern, texture, mesh, procedural.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, n) in FILL_KINDS {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 26.0), egui::Sense::click());
            if shown_kind == Some(i) {
                ui.painter().rect_filled(rect, 2.0, Tokens::tool_active());
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 2.0, Tokens::tool_hover());
            }
            fill_kind_picture(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)),
                i,
            );
            if resp.rich_tip(tr(n)).clicked() && shown_kind != Some(i) {
                if kind == i {
                    // The type the first object has, picked for objects
                    // of other types: they all take its fill.
                    touched.push(FillField::All);
                }
                kind = i;
                changed = true;
            }
        }
    });
    if mix.kind && !changed {
        crate::ui::field::mixed_note(ui);
        return false;
    }
    if changed && fill_kind_shown(fill) != kind {
        let base = fill
            .preview_color()
            .unwrap_or(Color::cmyk_pct(0.0, 0.0, 0.0, 20.0));
        // A pattern keeps its placement when its kind changes.
        let placement = match fill {
            Fill::Pattern(p) => *p.placement(),
            _ => Default::default(),
        };
        *fill = match kind {
            0 => Fill::None,
            1 => Fill::Solid(base),
            2 => Fill::linear(base, Color::WHITE, 0.0),
            3 => Fill::Pattern(Pattern::TwoColor {
                tile: PatternTile::Checker,
                front: base,
                back: Color::WHITE,
                size_mm: 10.0,
                placement,
            }),
            // A vector or bitmap pattern the drawing holds (the first), or
            // a bitmap pattern from a picture on the page; with none, the
            // way to make one (Object > Create > Pattern Fill, or a picture
            // file), the fill unchanged until then.
            7 | 8 => {
                let Some(pk) = pickers.as_deref_mut() else {
                    return false;
                };
                let lib = pk.library();
                let found = if kind == 7 {
                    lib.vector.first().cloned()
                } else {
                    lib.bitmap
                        .first()
                        .cloned()
                        .or_else(|| lib.picture_pattern(0))
                };
                match found {
                    Some(p) => Fill::Pattern(p.with_placement(placement)),
                    None => {
                        pk.action = Some(if kind == 7 {
                            PatternAction::CreateVector
                        } else if crate::files::WEB {
                            PatternAction::ImportBitmap
                        } else {
                            PatternAction::CreateBitmap
                        });
                        return false;
                    }
                }
            }
            4 => Fill::Texture(Texture {
                kind: TextureKind::Clouds,
                color_a: base,
                color_b: Color::WHITE,
                scale: 20.0,
                seed: 1,
            }),
            5 => Fill::Mesh(tracedraw_core::Mesh::new(
                tracedraw_core::geometry::Rect::new(0.0, 0.0, 100.0, 100.0),
                2,
                2,
                base,
            )),
            _ => Fill::Procedural(Procedural::new(ProceduralPattern::Octagons, base)),
        };
    }
    if !mix.kind && !mix.fields.is_empty() && !matches!(fill, Fill::Solid(_)) {
        crate::ui::field::mixed_note(ui);
    }
    match fill {
        Fill::None => {}
        Fill::Solid(c) => {
            let mixed = !mix.kind && mix.has(FillField::All);
            if color_row_mixed(ui, &tr("docker.colour"), c, mixed) {
                changed = true;
                if mixed {
                    touched.push(FillField::All);
                }
            }
        }
        Fill::Fountain(f) => changed |= fountain_editor(ui, f),
        Fill::Pattern(p) => changed |= pattern_editor(ui, p, mix, touched, pickers),
        Fill::Texture(t) => changed |= texture_editor(ui, t),
        Fill::Procedural(p) => changed |= procedural_editor(ui, p),
        Fill::Mesh(m) => {
            ui.label(trf(
                "docker.mesh_n",
                &[("r", &m.rows.to_string()), ("c", &m.cols.to_string())],
            ));
            ui.horizontal(|ui| {
                ui.label(tr("docker.grid"));
                let (mut r, mut c) = (m.rows, m.cols);
                if ui
                    .add(crate::ui::field::NumField::new(&mut r).range(1..=50))
                    .changed()
                    || ui
                        .add(crate::ui::field::NumField::new(&mut c).range(1..=50))
                        .changed()
                {
                    // The colours stay where the old grid had them.
                    *m = m.regrid(r, c);
                    changed = true;
                }
            });
            changed |= ui
                .checkbox(&mut m.smooth, tr("docker.smooth_colour"))
                .changed();
            ui.label(
                egui::RichText::new(tr("docker.mesh_hint"))
                    .color(Tokens::text_dim())
                    .size(11.0),
            );
        }
    }
    changed
}

/// Stops drawn left to right as one mesh.
pub(crate) fn gradient_swatch(p: &egui::Painter, r: egui::Rect, stops: &[tracedraw_core::Stop]) {
    let mut mesh = egui::Mesh::default();
    for (i, st) in stops.iter().enumerate() {
        let [cr, cg, cb] = st.color.to_rgb8();
        let c = egui::Color32::from_rgb(cr, cg, cb);
        let x = r.left() + r.width() * st.pos.clamp(0.0, 1.0) as f32;
        mesh.colored_vertex(egui::pos2(x, r.top()), c);
        mesh.colored_vertex(egui::pos2(x, r.bottom()), c);
        if i > 0 {
            let a = (i as u32 - 1) * 2;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Stops drawn bottom to top as one mesh.
fn gradient_swatch_up(p: &egui::Painter, r: egui::Rect, stops: &[tracedraw_core::Stop]) {
    let mut mesh = egui::Mesh::default();
    for (i, st) in stops.iter().enumerate() {
        let [cr, cg, cb] = st.color.to_rgb8();
        let c = egui::Color32::from_rgb(cr, cg, cb);
        let y = r.bottom() - r.height() * st.pos.clamp(0.0, 1.0) as f32;
        mesh.colored_vertex(egui::pos2(r.left(), y), c);
        mesh.colored_vertex(egui::pos2(r.right(), y), c);
        if i > 0 {
            let a = (i as u32 - 1) * 2;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
    }
    p.add(egui::Shape::mesh(mesh));
}

/// The i18n key of a fountain kind's short name.
pub fn fountain_kind_key(k: FountainKind) -> &'static str {
    match k {
        FountainKind::Linear => "fill.linear",
        FountainKind::Radial => "fill.radial",
        FountainKind::Conical => "fill.conical",
        FountainKind::Square => "fill.square",
        FountainKind::Shaped => "fill.shaped",
    }
}

fn fountain_editor(ui: &mut Ui, f: &mut Fountain) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("docker.type"));
        for k in FountainKind::ALL {
            if ui
                .selectable_label(f.kind == k, tr(fountain_kind_key(k)))
                .clicked()
                && f.kind != k
            {
                f.kind = k;
                changed = true;
            }
        }
    });
    // The whole run as it is drawn: repeats, mirror and inversion in.
    let (r, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(240.0), 14.0),
        egui::Sense::hover(),
    );
    gradient_swatch(ui.painter(), r, &f.resolved_stops());
    ui.painter().rect_stroke(
        r,
        2.0,
        egui::Stroke::new(1.0, crate::theme::Tokens::border()),
        egui::StrokeKind::Inside,
    );
    // Built-in gradients: a click replaces the stops.
    egui::CollapsingHeader::new(tr("docker.gradient_presets"))
        .id_salt("fountain_presets")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                // First, the current colours: the last fill colour picked
                // to the last outline colour picked.
                let (a, b) = ui
                    .data(|d| d.get_temp::<(Color, Color)>(current_colors_id()))
                    .unwrap_or((Color::BLACK, Color::WHITE));
                let current = vec![Stop::new(0.0, a), Stop::new(1.0, b)];
                let (r, resp) =
                    ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::click());
                gradient_swatch(ui.painter(), r, &current);
                let line = if resp.hovered() {
                    crate::theme::Tokens::ACCENT
                } else {
                    crate::theme::Tokens::border()
                };
                ui.painter().rect_stroke(
                    r,
                    2.0,
                    egui::Stroke::new(1.0, line),
                    egui::StrokeKind::Inside,
                );
                if resp.rich_tip(tr("grad.current_colors")).clicked() {
                    f.stops = current;
                    changed = true;
                }
                for (i, (key, _)) in crate::gradient_presets::PRESETS.iter().enumerate() {
                    let (r, resp) =
                        ui.allocate_exact_size(egui::vec2(34.0, 20.0), egui::Sense::click());
                    if let Some(stops) = crate::gradient_presets::stops(i) {
                        let angle = crate::gradient_presets::angle(i);
                        if angle.is_some() {
                            // Landscapes show ground at the bottom, sky on top.
                            gradient_swatch_up(ui.painter(), r, &stops);
                        } else {
                            gradient_swatch(ui.painter(), r, &stops);
                        }
                        let line = if resp.hovered() {
                            crate::theme::Tokens::ACCENT
                        } else {
                            crate::theme::Tokens::border()
                        };
                        ui.painter().rect_stroke(
                            r,
                            2.0,
                            egui::Stroke::new(1.0, line),
                            egui::StrokeKind::Inside,
                        );
                        if resp.rich_tip(tr(key)).clicked() {
                            f.stops = stops;
                            if let Some(a) = angle.filter(|_| f.kind == FountainKind::Linear) {
                                f.angle = a;
                            }
                            changed = true;
                        }
                    }
                }
            });
        });
    // Stops: position and colour, add after, remove (keep at least two).
    let mut remove = None;
    let mut insert = None;
    let n = f.stops.len();
    for i in 0..n {
        let stop = &mut f.stops[i];
        ui.horizontal(|ui| {
            if crate::ui::color_picker::color_button(ui, &mut stop.color) {
                changed = true;
            }
            let mut pct = stop.pos * 100.0;
            if ui
                .add(
                    crate::ui::field::NumField::new(&mut pct)
                        .range(0.0..=100.0)
                        .suffix("%")
                        .speed(1.0),
                )
                .changed()
            {
                stop.pos = pct / 100.0;
                changed = true;
            }
            // The stop's opacity (#1200).
            let mut opacity = f64::from(stop.opacity) * 100.0;
            if ui
                .add(
                    crate::ui::field::NumField::new(&mut opacity)
                        .range(0.0..=100.0)
                        .suffix("%")
                        .speed(1.0)
                        .width(52.0),
                )
                .rich_tip(tr("docker.stop_opacity"))
                .changed()
            {
                stop.opacity = (opacity / 100.0).clamp(0.0, 1.0) as f32;
                changed = true;
            }
            if ui
                .small_button("+")
                .rich_tip(tr("docker.add_stop"))
                .clicked()
            {
                insert = Some(i);
            }
            if n > 2
                && ui
                    .small_button("x")
                    .rich_tip(tr("docker.remove_stop"))
                    .clicked()
            {
                remove = Some(i);
            }
        });
    }
    // A `.ggr` gradient file replaces the stops (of this editor: the fill
    // and the outline each have one).
    let ggr_key = format!("fountain_ggr{:?}", ui.id());
    if ui
        .button(tr("docker.import_gradient"))
        .rich_tip(tr("docker.import_gradient_tip"))
        .clicked()
    {
        crate::files::Dialog::new()
            .add_filter("GIMP gradient", &["ggr"])
            .pick_into(&ggr_key);
    }
    if let Some(path) = crate::files::take_picked(&ggr_key) {
        match crate::files::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| tracedraw_io::ggr::read(&text))
        {
            Ok(g) => {
                f.stops = g.stops;
                changed = true;
            }
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    }
    if let Some(i) = insert {
        let next = f.stops.get(i + 1).map(|s| s.pos).unwrap_or(1.0);
        let pos = (f.stops[i].pos + next) / 2.0;
        // The stops' own colour there (not the repeats or the inversion).
        let color = tracedraw_core::style::sample_stops(&f.sorted_stops(), pos);
        f.stops.insert(i + 1, Stop::new(pos, color));
        changed = true;
    }
    if let Some(i) = remove {
        f.stops.remove(i);
        changed = true;
    }
    if matches!(
        f.kind,
        FountainKind::Linear | FountainKind::Conical | FountainKind::Square
    ) {
        changed |= ui
            .add(
                crate::ui::Slider::new(&mut f.angle, -180.0..=180.0)
                    .text(tr("docker.angle_lc"))
                    .suffix("°"),
            )
            .settled();
    }
    if !matches!(f.kind, FountainKind::Linear | FountainKind::Shaped) {
        ui.horizontal(|ui| {
            ui.label(tr("docker.centre_offset"));
            changed |= ui
                .add(
                    crate::ui::field::NumField::new(&mut f.offset.x)
                        .range(-1.0..=1.0)
                        .speed(0.01),
                )
                .changed();
            changed |= ui
                .add(
                    crate::ui::field::NumField::new(&mut f.offset.y)
                        .range(-1.0..=1.0)
                        .speed(0.01),
                )
                .changed();
        });
    }
    let mut pad = f.edge_pad * 100.0;
    if ui
        .add(
            crate::ui::Slider::new(&mut pad, 0.0..=49.0)
                .text(tr("docker.edge_pad"))
                .suffix("%"),
        )
        .settled()
    {
        f.edge_pad = pad / 100.0;
        changed = true;
    }
    changed |= fountain_spread_editor(ui, f);
    changed
}

/// The repeat mode and count, the bilinear mirror and the inversion.
fn fountain_spread_editor(ui: &mut Ui, f: &mut Fountain) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("docker.repeat"));
        for (m, key) in [
            (FountainRepeat::None, "docker.repeat_none"),
            (FountainRepeat::Repeat, "docker.repeat_sawtooth"),
            (FountainRepeat::Mirror, "docker.repeat_mirror"),
        ] {
            if ui.selectable_label(f.repeat == m, tr(key)).clicked() && f.repeat != m {
                f.repeat = m;
                if m != FountainRepeat::None && f.repeats < 2 {
                    f.repeats = 2;
                }
                changed = true;
            }
        }
    });
    if f.repeat != FountainRepeat::None {
        ui.horizontal(|ui| {
            ui.label(tr("docker.repeat_count"));
            changed |= ui
                .add(
                    crate::ui::field::NumField::new(&mut f.repeats)
                        .range(1..=Fountain::MAX_REPEATS),
                )
                .changed();
        });
    }
    ui.horizontal_wrapped(|ui| {
        changed |= ui
            .checkbox(&mut f.bilinear, tr("docker.bilinear"))
            .rich_tip(tr("docker.bilinear_tip"))
            .changed();
        changed |= ui
            .checkbox(&mut f.invert, tr("docker.invert_colours"))
            .rich_tip(tr("docker.invert_colours_tip"))
            .changed();
    });
    changed |= fountain_blend_editor(ui, f);
    changed
}

/// The step count a fountain gets when Steps is first ticked.
const FIRST_STEPS: u32 = 16;

/// The i18n key of a blend direction's name (the Blend effect's words).
pub fn blend_direction_key(how: ColorBlend) -> &'static str {
    match how {
        ColorBlend::Direct => "docker.blend_direct",
        ColorBlend::Clockwise => "docker.blend_clockwise",
        ColorBlend::CounterClockwise => "docker.blend_counterclockwise",
    }
}

/// The blend direction, the steps, the smooth transition and the
/// acceleration (#1208).
fn fountain_blend_editor(ui: &mut Ui, f: &mut Fountain) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(tr("docker.blend_direction"))
            .rich_tip(tr("docker.blend_direction_tip"));
        for how in ColorBlend::ALL {
            if ui
                .selectable_label(f.blend_direction == how, tr(blend_direction_key(how)))
                .clicked()
                && f.blend_direction != how
            {
                f.blend_direction = how;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        // Unticked, the fill is smooth; the count last used comes back
        // when it is ticked again.
        let mut stepped = f.stepped();
        let last_id = ui.id().with("fountain_steps");
        if ui
            .checkbox(&mut stepped, tr("docker.steps"))
            .rich_tip(tr("docker.fountain_steps_tip"))
            .changed()
        {
            if stepped {
                f.steps = ui
                    .data(|d| d.get_temp::<u32>(last_id))
                    .unwrap_or(FIRST_STEPS);
            } else {
                ui.data_mut(|d| d.insert_temp(last_id, f.band_count()));
                f.steps = Fountain::SMOOTH_STEPS;
            }
            changed = true;
        }
        if f.stepped() {
            let mut n = f.band_count();
            if ui
                .add(
                    crate::ui::field::NumField::new(&mut n)
                        .range(Fountain::MIN_STEPS..=Fountain::SMOOTH_STEPS - 1),
                )
                .changed()
            {
                f.steps = n;
                changed = true;
            }
        }
    });
    changed |= ui
        .checkbox(&mut f.smooth, tr("docker.smooth_transition"))
        .rich_tip(tr("docker.smooth_transition_tip"))
        .changed();
    ui.horizontal(|ui| {
        ui.label(tr("docker.fountain_acceleration"))
            .rich_tip(tr("docker.fountain_acceleration_tip"));
        changed |= ui
            .add(
                crate::ui::field::NumField::new(&mut f.acceleration)
                    .range(-Fountain::MAX_ACCELERATION..=Fountain::MAX_ACCELERATION),
            )
            .changed();
    });
    changed
}

/// The settings of a pattern fill: the tile (a two-colour tile and its
/// colours; a vector or bitmap pattern picked from the drawing's, with
/// `pickers`), then the placement every kind shares: tile size (with a
/// lock keeping the proportions), offset, rotation, skew, mirrored
/// tiles, row or column offset, Transform with object. Values that
/// differ across the selection (`mix`) show as mixed; a value set to
/// what a mixed field shows goes into `touched`.
pub(crate) fn pattern_editor(
    ui: &mut Ui,
    p: &mut Pattern,
    mix: &FillMix,
    touched: &mut Vec<FillField>,
    pickers: Option<&mut PatternPickers>,
) -> bool {
    use crate::ui::field::NumField;
    use crate::ui::pattern_pick::{draw_lock, draw_mirror, draw_shift, picture_button};
    use tracedraw_core::style::TileShift;
    let mut changed = false;
    match p {
        Pattern::TwoColor {
            tile, front, back, ..
        } => {
            crate::ui::keynav::ComboBox::from_label(tr("docker.tile"))
                .selected_text(pattern_tile_name(*tile))
                .show_ui(ui, |ui| {
                    for t in PatternTile::ALL {
                        if ui.selectable_label(*tile == t, pattern_tile_name(t)).clicked() {
                            *tile = t;
                            changed = true;
                        }
                    }
                });
            changed |= color_row(ui, &tr("docker.front"), front);
            changed |= color_row(ui, &tr("docker.back"), back);
        }
        Pattern::Bitmap {
            width_px,
            height_px,
            ..
        } => {
            ui.label(trf(
                "docker.bitmap_tile_n",
                &[("w", &width_px.to_string()), ("h", &height_px.to_string())],
            ));
        }
        Pattern::Vector { shapes, .. } => {
            ui.label(trf(
                "docker.vector_tile_n",
                &[("n", &shapes.len().to_string())],
            ));
        }
    }
    // The drawing's patterns of this kind, and the ways to a new one.
    if let (Some(pk), false) = (pickers, matches!(p, Pattern::TwoColor { .. })) {
        let vector = matches!(p, Pattern::Vector { .. });
        let placement = *p.placement();
        let mut pick: Option<Pattern> = None;
        let mut action = None;
        {
            // The drawing is walked only while the list is open, not on
            // every frame the panel shows a pattern.
            crate::ui::keynav::ComboBox::from_id_salt("pattern_pick")
                .selected_text(tr("docker.pattern_pick"))
                .width(ui.available_width().min(220.0))
                .show_ui(ui, |ui| {
                    let lib = pk.library();
                    let list = if vector { &lib.vector } else { &lib.bitmap };
                    let current = lib.position(p);
                    for (i, lp) in list.iter().enumerate() {
                        let name = crate::ui::pattern_pick::pattern_name(lp, i);
                        if ui.selectable_label(current == Some(i), name).clicked()
                            && current != Some(i)
                        {
                            pick = Some(lp.clone());
                        }
                    }
                    if !vector {
                        for (i, (w, h, _, _)) in lib.pictures.iter().enumerate() {
                            let name = trf(
                                "docker.pattern_from_picture",
                                &[("w", &w.to_string()), ("h", &h.to_string())],
                            );
                            if ui.selectable_label(false, name).clicked() {
                                pick = lib.picture_pattern(i);
                            }
                        }
                    }
                    ui.separator();
                    if ui
                        .selectable_label(false, tr("docker.pattern_new"))
                        .rich_tip(tr("docker.pattern_new_tip"))
                        .clicked()
                    {
                        action = Some(if vector {
                            PatternAction::CreateVector
                        } else {
                            PatternAction::CreateBitmap
                        });
                    }
                    if !vector
                        && ui
                            .selectable_label(false, tr("docker.pattern_import"))
                            .rich_tip(tr("docker.pattern_import_tip"))
                            .clicked()
                    {
                        action = Some(PatternAction::ImportBitmap);
                    }
                });
        }
        if let Some(np) = pick {
            *p = np.with_placement(placement);
            changed = true;
            touched.push(FillField::All);
        }
        if action.is_some() {
            pk.action = action;
        }
    }

    // The placement, shared by every kind.
    let differs = |f: FillField| mix.fields.contains(&f);
    let mut pl = *p.placement();
    fn field<'v>(
        v: &'v mut f64,
        mixed: bool,
        range: std::ops::RangeInclusive<f64>,
        suffix: &str,
    ) -> NumField<'v, f64> {
        NumField::new(v)
            .range(range)
            .speed(0.1)
            .max_decimals(2)
            .suffix(suffix)
            .width(58.0)
            .mixed(mixed)
    }
    fn mm(v: &mut f64, mixed: bool) -> NumField<'_, f64> {
        field(v, mixed, -10000.0..=10000.0, " mm").unit_mm(1.0)
    }
    let lock_id = ui.id().with("pattern_wh_lock");
    let mut locked = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(lock_id))
        .unwrap_or(true);
    let size = p.tile_size();
    let (mut w, mut h) = (size.width, size.height);
    let size_mixed = differs(FillField::TileSize);
    // The long labels (tile size, row or column offset) take a line of
    // their own so the fields fit the panel in every language.
    ui.label(tr("docker.tile_size"));
    ui.horizontal(|ui| {
        let wr = ui
            .add(mm(&mut w, size_mixed).prefix(tr("docker.w")))
            .rich_tip(tr("docker.tile_width"));
        let hr = ui
            .add(mm(&mut h, size_mixed).prefix(tr("docker.h")))
            .rich_tip(tr("docker.tile_height"));
        if picture_button(ui, locked, &tr("docker.tile_lock_tip"), |p, r| {
            draw_lock(p, r, locked)
        }) {
            locked = !locked;
            ui.ctx().data_mut(|d| d.insert_temp(lock_id, locked));
        }
        if wr.changed() || hr.changed() {
            let (ow, oh) = (size.width.max(1e-9), size.height.max(1e-9));
            if locked && wr.changed() {
                h = (w * oh / ow).max(Pattern::MIN_TILE_MM);
            } else if locked && hr.changed() {
                w = (h * ow / oh).max(Pattern::MIN_TILE_MM);
            }
            let w = w.max(Pattern::MIN_TILE_MM);
            let h = h.max(Pattern::MIN_TILE_MM);
            p.set_tile_size(tracedraw_core::geometry::Size::new(w, h));
            pl = *p.placement();
            changed = true;
            if size_mixed {
                touched.push(FillField::TileSize);
            }
        }
    });
    let offset_mixed = differs(FillField::TileOffset);
    ui.horizontal(|ui| {
        ui.label(tr("docker.offset"));
        let xr = ui
            .add(mm(&mut pl.offset_x, offset_mixed).prefix(tr("docker.x")))
            .rich_tip(tr("docker.tile_offset_x_tip"));
        let yr = ui
            .add(mm(&mut pl.offset_y, offset_mixed).prefix(tr("docker.y")))
            .rich_tip(tr("docker.tile_offset_y_tip"));
        if xr.changed() || yr.changed() {
            changed = true;
            if offset_mixed {
                touched.push(FillField::TileOffset);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("docker.rotation"));
        let rot_mixed = differs(FillField::TileRotation);
        if ui
            .add(field(&mut pl.rotation, rot_mixed, -360.0..=360.0, "\u{b0}"))
            .rich_tip(tr("docker.tile_rotation_tip"))
            .changed()
        {
            changed = true;
            if rot_mixed {
                touched.push(FillField::TileRotation);
            }
        }
        ui.label(tr("docker.skew"));
        let skew_mixed = differs(FillField::TileSkew);
        if ui
            .add(field(&mut pl.skew, skew_mixed, -89.0..=89.0, "\u{b0}"))
            .rich_tip(tr("docker.tile_skew_tip"))
            .changed()
        {
            changed = true;
            if skew_mixed {
                touched.push(FillField::TileSkew);
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("docker.mirror_tiles"));
        let mirror_mixed = differs(FillField::TileMirror);
        if picture_button(
            ui,
            pl.mirror_horizontal && !mirror_mixed,
            &tr("docker.mirror_horizontal_tip"),
            |p, r| draw_mirror(p, r, false),
        ) {
            pl.mirror_horizontal = !pl.mirror_horizontal;
            changed = true;
            if mirror_mixed {
                touched.push(FillField::TileMirror);
            }
        }
        if picture_button(
            ui,
            pl.mirror_vertical && !mirror_mixed,
            &tr("docker.mirror_vertical_tip"),
            |p, r| draw_mirror(p, r, true),
        ) {
            pl.mirror_vertical = !pl.mirror_vertical;
            changed = true;
            if mirror_mixed {
                touched.push(FillField::TileMirror);
            }
        }
    });
    ui.label(tr("docker.row_column_offset"));
    ui.horizontal(|ui| {
        let shift_mixed = differs(FillField::TileShift);
        for (kind, tip) in [
            (TileShift::Row, "docker.row_offset_tip"),
            (TileShift::Column, "docker.column_offset_tip"),
        ] {
            if picture_button(ui, pl.shift == kind && !shift_mixed, &tr(tip), |p, r| {
                draw_shift(p, r, kind == TileShift::Column)
            }) && (pl.shift != kind || shift_mixed)
            {
                pl.shift = kind;
                changed = true;
                if shift_mixed {
                    touched.push(FillField::TileShift);
                }
            }
        }
        if ui
            .add(field(&mut pl.shift_percent, shift_mixed, 0.0..=100.0, " %"))
            .rich_tip(tr("docker.row_column_offset_tip"))
            .changed()
        {
            changed = true;
            if shift_mixed {
                touched.push(FillField::TileShift);
            }
        }
    });
    let two_mixed = differs(FillField::TransformWithObject);
    let mut with_object = pl.transform_with_object && !two_mixed;
    if ui
        .checkbox(&mut with_object, tr("docker.transform_with_object"))
        .rich_tip(tr("docker.transform_with_object_tip"))
        .changed()
    {
        pl.transform_with_object = with_object;
        changed = true;
        if two_mixed {
            touched.push(FillField::TransformWithObject);
        }
    }
    if pl != *p.placement() {
        *p.placement_mut() = pl.sanitized();
    }
    changed
}

pub(crate) fn texture_editor(ui: &mut Ui, t: &mut Texture) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (k, n) in [
            (TextureKind::Clouds, "texture.clouds"),
            (TextureKind::Marble, "texture.marble"),
            (TextureKind::Noise, "texture.noise"),
            (TextureKind::Wood, "texture.wood"),
        ] {
            if ui.selectable_label(t.kind == k, tr(n)).clicked() && t.kind != k {
                t.kind = k;
                changed = true;
            }
        }
    });
    changed |= color_row(ui, &tr("docker.colour_a"), &mut t.color_a);
    changed |= color_row(ui, &tr("docker.colour_b"), &mut t.color_b);
    changed |= ui
        .add(
            crate::ui::Slider::new(&mut t.scale, 1.0..=200.0)
                .text(tr("docker.scale_mm"))
                .logarithmic(true),
        )
        .settled();
    ui.horizontal(|ui| {
        ui.label(tr("docker.seed"));
        changed |= ui
            .add(crate::ui::field::NumField::new(&mut t.seed))
            .changed();
        if ui.small_button(tr("docker.regenerate")).clicked() {
            t.seed = t.seed.wrapping_add(1);
            changed = true;
        }
    });
    changed
}

/// The i18n key of a built-in two-colour pattern tile's name.
pub fn pattern_tile_key(t: PatternTile) -> &'static str {
    match t {
        PatternTile::Checker => "pattern_tile.checker",
        PatternTile::Dots => "pattern_tile.dots",
        PatternTile::Stripes => "pattern_tile.stripes",
        PatternTile::DiagonalStripes => "pattern_tile.diagonal_stripes",
        PatternTile::Grid => "pattern_tile.grid",
        PatternTile::Bricks => "pattern_tile.bricks",
        PatternTile::Crosshatch => "pattern_tile.crosshatch",
        PatternTile::Waves => "pattern_tile.waves",
    }
}

/// A built-in two-colour pattern tile's name in the interface language.
pub fn pattern_tile_name(t: PatternTile) -> String {
    tr(pattern_tile_key(t))
}

/// The i18n key of a procedural pattern's name.
pub fn procedural_pattern_key(p: ProceduralPattern) -> &'static str {
    match p {
        ProceduralPattern::Octagons => "procedural.octagons",
        ProceduralPattern::Triangles => "procedural.triangles",
        ProceduralPattern::Bricks => "procedural.bricks",
        ProceduralPattern::Honeycomb => "procedural.honeycomb",
        ProceduralPattern::Crosshatch => "procedural.crosshatch",
        ProceduralPattern::Dots => "procedural.dots",
        ProceduralPattern::Waves => "procedural.waves",
        ProceduralPattern::Spirals => "procedural.spirals",
        ProceduralPattern::Speckle => "procedural.speckle",
    }
}

/// The procedural fill's editor: the pattern list by name, a preview of
/// four tiles (the renderer's picture of the tile paths, kept until the
/// fill changes), the numbers the pattern uses, then the foreground and
/// the background colour. A slider reports its change when the drag ends,
/// so a drag is one undo step.
pub(crate) fn procedural_editor(ui: &mut Ui, p: &mut Procedural) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt("procedural_pattern")
        .selected_text(tr(procedural_pattern_key(p.pattern)))
        .show_ui(ui, |ui| {
            for k in ProceduralPattern::ALL {
                if ui
                    .selectable_label(p.pattern == k, tr(procedural_pattern_key(k)))
                    .clicked()
                    && p.pattern != k
                {
                    p.pattern = k;
                    changed = true;
                }
            }
        })
        .response
        .rich_tip(tr("docker.proc_pattern"));
    // Two tiles each way, the box in the tile's proportions.
    let tile = tracedraw_core::procedural::procedural_tile(p);
    let (tw, th) = (tile.size.width.max(1e-6), tile.size.height.max(1e-6));
    let ratio = (th / tw).clamp(0.5, 2.0) as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(72.0, 72.0 * ratio), egui::Sense::hover());
    crate::ui::edit_fill::draw_fill_in(
        ui,
        rect,
        &Fill::Procedural(p.clone()),
        tracedraw_core::geometry::Rect::new(0.0, 0.0, 2.0 * tw, 2.0 * th),
        egui::Id::new("procedural_editor_preview_tex"),
    );
    let q = &mut p.params;
    for param in p.pattern.params() {
        match param {
            ProceduralParam::Size => {
                changed |= ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(
                            &mut q.size_mm,
                            ProceduralParams::MIN_SIZE_MM..=ProceduralParams::MAX_SIZE_MM,
                        )
                        .text(tr("docker.tile_size_mm"))
                        .logarithmic(true),
                    ))
                    .drag_stopped();
            }
            ProceduralParam::LineWidth => {
                changed |= ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(
                            &mut q.line_width_mm,
                            ProceduralParams::MIN_LINE_WIDTH_MM..=10.0,
                        )
                        .text(tr("docker.line_width_mm"))
                        .logarithmic(true),
                    ))
                    .drag_stopped();
            }
            ProceduralParam::Share => {
                let mut pct = q.share * 100.0;
                if ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(&mut pct, 0.0..=ProceduralParams::MAX_SHARE * 100.0)
                            .text(tr("docker.cover_pct")),
                    ))
                    .drag_stopped()
                {
                    q.share = pct / 100.0;
                    changed = true;
                }
            }
            ProceduralParam::Amplitude => {
                let mut pct = q.amplitude * 100.0;
                if ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(&mut pct, 0.0..=ProceduralParams::MAX_AMPLITUDE * 100.0)
                            .text(tr("docker.amplitude_pct")),
                    ))
                    .drag_stopped()
                {
                    q.amplitude = pct / 100.0;
                    changed = true;
                }
            }
            ProceduralParam::Turns => {
                changed |= ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(&mut q.turns, 0.5..=ProceduralParams::MAX_TURNS)
                            .text(tr("docker.turns")),
                    ))
                    .drag_stopped();
            }
            ProceduralParam::Density => {
                changed |= ui
                    .add(crate::ui::Rail(
                        egui::Slider::new(&mut q.density, 1.0..=ProceduralParams::MAX_DENSITY)
                            .text(tr("docker.specks"))
                            .logarithmic(true),
                    ))
                    .drag_stopped();
            }
            ProceduralParam::Seed => {
                ui.horizontal(|ui| {
                    ui.label(tr("docker.seed"));
                    changed |= ui
                        .add(crate::ui::field::NumField::new(&mut p.seed))
                        .changed();
                    if ui.small_button(tr("docker.regenerate")).clicked() {
                        p.seed = p.seed.wrapping_add(1);
                        changed = true;
                    }
                });
            }
        }
    }
    changed |= color_row(ui, &tr("docker.foreground"), &mut p.foreground);
    ui.horizontal(|ui| {
        let mut has = p.background.is_some();
        if ui.checkbox(&mut has, tr("docker.background")).changed() {
            p.background = has.then_some(Color::WHITE);
            changed = true;
        }
        if let Some(bg) = p.background.as_mut() {
            changed |= crate::ui::color_picker::color_button(ui, bg);
        }
    });
    changed
}

pub(crate) fn arrow_combo(
    ui: &mut Ui,
    id: &str,
    label: &str,
    a: &mut Arrowhead,
    custom: &[Arrowhead],
    mixed: bool,
) -> bool {
    let mut changed = false;
    let shown = if mixed {
        format!("{label}:")
    } else {
        format!("{label}: {}", a.name())
    };
    let tip = if id.to_lowercase().ends_with("end") {
        "toolbar.end_arrow"
    } else {
        "toolbar.start_arrow"
    };
    crate::ui::keynav::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for k in Arrowhead::ALL {
                if ui.selectable_label(!mixed && *a == k, k.name()).clicked() {
                    *a = k;
                    changed = true;
                }
            }
            for (i, k) in custom.iter().enumerate() {
                let name = if k.name().is_empty() {
                    trf("docker.custom_arrowhead_n", &[("n", &(i + 1).to_string())])
                } else {
                    k.name().to_string()
                };
                if ui.selectable_label(!mixed && a == k, name).clicked() {
                    *a = k.clone();
                    changed = true;
                }
            }
        })
        .response
        .rich_tip(crate::ui::tips::keyed(tip));
    changed
}

/// An outline's paint as a fill: its own fill, or its colour as a
/// uniform fill.
fn outline_paint_of(s: &Stroke) -> Fill {
    match s.fill.as_deref() {
        Some(f) if !matches!(f, Fill::None | Fill::Solid(_)) => f.clone(),
        _ => Fill::Solid(s.color),
    }
}

/// Give an outline the paint chosen in its fill editor: a uniform paint
/// becomes its colour, the other kinds its own fill. True when the
/// paint is none (the outline goes).
fn set_outline_paint(s: &mut Stroke, paint: Fill) -> bool {
    match paint {
        Fill::None => true,
        Fill::Solid(c) => {
            s.color = c;
            s.fill = None;
            false
        }
        other => {
            s.fill = Some(Box::new(other));
            false
        }
    }
}

fn outline_editor(
    ui: &mut Ui,
    stroke: &mut Option<Stroke>,
    lib: &crate::ui::line_style::OutlineLib,
) -> bool {
    outline_editor_mixed(ui, stroke, lib, &StrokeMix::default(), &mut Vec::new())
}

/// [`outline_editor`] for a selection whose outlines differ as `mix`
/// says: a value that differs shows empty (no colour, an empty width box,
/// no button highlighted, a check box neither on nor off). Every control
/// the user sets adds its field to `touched`, so a value chosen in a
/// mixed control applies to every object even when it equals the one
/// shown.
fn outline_editor_mixed(
    ui: &mut Ui,
    stroke: &mut Option<Stroke>,
    lib: &crate::ui::line_style::OutlineLib,
    mix: &StrokeMix,
    touched: &mut Vec<StrokeField>,
) -> bool {
    let custom = lib.arrows.as_slice();
    let mut changed = false;
    let mut has = stroke.is_some();
    if ui
        .add(egui::Checkbox::new(&mut has, tr("docker.outline")).indeterminate(mix.presence))
        .rich_tip(crate::ui::tips::keyed("docker.outline"))
        .changed()
    {
        *stroke = if has {
            // Black in the drawing's primary colour mode.
            Some(Stroke {
                color: lib.black,
                ..Stroke::default()
            })
        } else {
            None
        };
        changed = true;
    }
    let mut remove = false;
    if let Some(s) = stroke.as_mut() {
        // The outline's paint offers every kind a fill does, with the
        // same editors: none takes the outline away, uniform is its
        // colour, the others its own fill.
        ui.label(tr("docker.outline_fill"));
        let mut paint = outline_paint_of(s);
        let paint_mix = if mix.has(StrokeField::Paint) {
            FillMix {
                kind: false,
                fields: vec![FillField::All],
            }
        } else {
            FillMix::default()
        };
        let mut paint_touched = Vec::new();
        let edited = ui
            .push_id("outline_paint", |ui| {
                fill_editor_mixed(ui, &mut paint, &paint_mix, &mut paint_touched, None)
            })
            .inner;
        if edited {
            changed = true;
            touched.push(StrokeField::Paint);
            remove = set_outline_paint(s, paint);
        }
    }
    if remove {
        *stroke = None;
    }
    if let Some(s) = stroke {
        let width_mixed = mix.has(StrokeField::Width);
        let mut hair = s.width <= Stroke::HAIRLINE + 1e-9;
        if ui
            .add(egui::Checkbox::new(&mut hair, tr("docker.hairline")).indeterminate(width_mixed))
            .rich_tip(crate::ui::tips::keyed("docker.hairline"))
            .changed()
        {
            s.width = if hair { Stroke::HAIRLINE } else { 0.5 };
            touched.push(StrokeField::Width);
            changed = true;
        }
        if !hair || width_mixed {
            // The slider applies when let go; the box takes any width
            // typed (Enter or leaving it applies).
            ui.horizontal_wrapped(|ui| {
                let width_tip =
                    crate::ui::tips::pair("toolbar.outline_width", "docker.outline_width_desc");
                let r = ui
                    .add(
                        crate::ui::Slider::new(&mut s.width, 0.1..=25.0)
                            .logarithmic(true)
                            .show_value(false)
                            // The width box and the unit list after it.
                            .reserve(150.0),
                    )
                    .rich_tip(width_tip.clone());
                let mut w = r.settled();
                // The width in the outline unit; the model keeps mm.
                let u = lib.units;
                let mut v = u.from_mm(s.width);
                if ui
                    .add(
                        crate::ui::field::NumField::new(&mut v)
                            .range(u.from_mm(0.001)..=u.from_mm(1000.0))
                            .unit_mm(u.mm())
                            .max_decimals(4)
                            .width(56.0)
                            .mixed(width_mixed),
                    )
                    .rich_tip(width_tip)
                    .changed()
                {
                    s.width = u.to_mm(v).clamp(0.001, 1000.0);
                    w = true;
                }
                if let Some(nu) =
                    crate::ui::propbar::outline_unit_combo(ui, "outline_units_docker", u)
                {
                    crate::ui::line_style::request_units(ui, nu);
                }
                if w {
                    touched.push(StrokeField::Width);
                    changed = true;
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("docker.caps"));
            let mixed = mix.has(StrokeField::Cap);
            for (c, n) in [
                (tracedraw_core::LineCap::Butt, "docker.cap_butt"),
                (tracedraw_core::LineCap::Round, "docker.cap_round"),
                (tracedraw_core::LineCap::Square, "docker.cap_square"),
            ] {
                if ui
                    .selectable_label(!mixed && s.cap == c, tr(n))
                    .rich_tip(crate::ui::tips::pair(
                        &format!("{n}_tip"),
                        &format!("{n}_desc"),
                    ))
                    .clicked()
                {
                    s.cap = c;
                    touched.push(StrokeField::Cap);
                    changed = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("docker.corners"));
            let mixed = mix.has(StrokeField::Join);
            for (j, n) in [
                (tracedraw_core::LineJoin::Miter, "docker.join_miter"),
                (tracedraw_core::LineJoin::Round, "docker.join_round"),
                (tracedraw_core::LineJoin::Bevel, "docker.join_bevel"),
            ] {
                if ui
                    .selectable_label(!mixed && s.join == j, tr(n))
                    .rich_tip(crate::ui::tips::pair(
                        &format!("{n}_tip"),
                        &format!("{n}_desc"),
                    ))
                    .clicked()
                {
                    s.join = j;
                    touched.push(StrokeField::Join);
                    changed = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (field, id, key) in [
                (StrokeField::StartArrow, "Start", "docker.arrow_start"),
                (StrokeField::EndArrow, "End", "docker.arrow_end"),
            ] {
                let end = field == StrokeField::EndArrow;
                // With Share attributes on the end follows the start.
                let enabled = !end || !s.share_arrowheads;
                ui.add_enabled_ui(enabled, |ui| {
                    let a = if end {
                        &mut s.end_arrow
                    } else {
                        &mut s.start_arrow
                    };
                    if arrow_combo(ui, id, &tr(key), a, custom, mix.has(field)) {
                        if !end && s.share_arrowheads {
                            s.share_start_head();
                        }
                        touched.push(field);
                        changed = true;
                    }
                    // The Arrowhead editor (#1210): size, offset, mirror,
                    // rotation of this end's head.
                    if ui
                        .small_button("...")
                        .rich_tip(crate::ui::tips::keyed("arrowhead_editor.edit"))
                        .clicked()
                    {
                        crate::ui::arrowhead_editor::request(ui, end);
                    }
                });
            }
            if ui
                .add(
                    egui::Checkbox::new(&mut s.share_arrowheads, tr("arrowhead_editor.share"))
                        .indeterminate(mix.has(StrokeField::ShareArrowheads)),
                )
                .rich_tip(crate::ui::tips::keyed("arrowhead_editor.share"))
                .changed()
            {
                if s.share_arrowheads {
                    s.share_start_head();
                }
                touched.push(StrokeField::ShareArrowheads);
                changed = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("docker.dash"));
            let mixed = mix.has(StrokeField::Dash);
            for (d, n) in [
                (vec![], "docker.dash_solid"),
                (vec![4.0, 2.0], "docker.dash_dashed"),
                (vec![1.0, 1.0], "docker.dash_dotted"),
                (vec![6.0, 2.0, 1.0, 2.0], "docker.dash_dash_dot"),
            ] {
                if ui
                    .selectable_label(!mixed && s.dash == d, tr(n))
                    .rich_tip(crate::ui::tips::pair(
                        &format!("{n}_tip"),
                        &format!("{n}_desc"),
                    ))
                    .clicked()
                {
                    s.dash = d;
                    touched.push(StrokeField::Dash);
                    changed = true;
                }
            }
        });
        if !lib.dashes.is_empty() {
            // Custom dash patterns from Edit Line Style.
            ui.horizontal_wrapped(|ui| {
                ui.label(tr("docker.dash_custom"));
                for d in &lib.dashes {
                    let r = ui
                        .push_id(
                            ("custom_dash", d.len(), d.iter().sum::<f64>() as i64),
                            |ui| {
                                let r = crate::ui::line_style::preview(ui, d, 48.0);
                                if s.dash == *d {
                                    ui.painter().rect_stroke(
                                        r.rect.expand(2.0),
                                        0.0,
                                        egui::Stroke::new(1.0, Tokens::ACCENT),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                r
                            },
                        )
                        .inner;
                    if r.clicked() {
                        s.dash = d.clone();
                        touched.push(StrokeField::Dash);
                        changed = true;
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !s.dash.is_empty(),
                    egui::Checkbox::new(&mut s.align_dashes, tr("docker.align_dashes")),
                )
                .rich_tip(tr("docker.align_dashes_tip"))
                .changed()
            {
                touched.push(StrokeField::AlignDashes);
                changed = true;
            }
            if ui
                .button(tr("docker.edit_line_style"))
                .rich_tip(crate::ui::tips::keyed("docker.edit_line_style"))
                .clicked()
            {
                crate::ui::line_style::request(ui);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(tr("docker.outline_position"));
            for (pos, n) in [
                (
                    tracedraw_core::OutlinePosition::Outside,
                    "docker.position_outside",
                ),
                (
                    tracedraw_core::OutlinePosition::Center,
                    "docker.position_center",
                ),
                (
                    tracedraw_core::OutlinePosition::Inside,
                    "docker.position_inside",
                ),
            ] {
                let shown = !mix.has(StrokeField::Position) && s.position == pos;
                if ui
                    .selectable_label(shown, tr(n))
                    .rich_tip(crate::ui::tips::pair(
                        &format!("{n}_tip"),
                        &format!("{n}_desc"),
                    ))
                    .clicked()
                {
                    s.position = pos;
                    touched.push(StrokeField::Position);
                    changed = true;
                }
            }
        });
        for (field, key) in [
            (StrokeField::BehindFill, "docker.behind_fill"),
            (StrokeField::ScaleWithObject, "docker.scale_with_object"),
        ] {
            let v = if field == StrokeField::BehindFill {
                &mut s.behind_fill
            } else {
                &mut s.scale_with_object
            };
            if ui
                .add(egui::Checkbox::new(v, tr(key)).indeterminate(mix.has(field)))
                .rich_tip(crate::ui::tips::keyed(key))
                .changed()
            {
                touched.push(field);
                changed = true;
            }
        }
        // The settings few outlines need, under their own header so the
        // common ones are found at a glance (#487): open at first when
        // one of them is set away from its default or differs between
        // the objects, so nothing set is hidden.
        let advanced_set = s.miter_limit != Stroke::DEFAULT_MITER_LIMIT
            || s.stretch != 1.0
            || s.nib_angle != 0.0
            || [
                StrokeField::MiterLimit,
                StrokeField::Stretch,
                StrokeField::NibAngle,
            ]
            .iter()
            .any(|f| mix.has(*f));
        let advanced = egui::CollapsingHeader::new(tr("docker.outline_advanced"))
            .id_salt("docker_outline_advanced")
            .default_open(advanced_set)
            .show(ui, |ui| {
                if s.join == tracedraw_core::LineJoin::Miter {
                    ui.horizontal(|ui| {
                        ui.label(tr("docker.miter_limit"));
                        if ui
                            .add(
                                crate::ui::field::NumField::new(&mut s.miter_limit)
                                    .range(Stroke::MITER_LIMIT_RANGE)
                                    .speed(0.5)
                                    .max_decimals(1)
                                    .suffix("°")
                                    .width(56.0)
                                    .mixed(mix.has(StrokeField::MiterLimit)),
                            )
                            .rich_tip(tr("docker.miter_limit_tip"))
                            .changed()
                        {
                            touched.push(StrokeField::MiterLimit);
                            changed = true;
                        }
                    });
                }
                // The calligraphic nib (#1200): stretch as a percentage,
                // the angle, and the nib itself, dragged to turn and
                // flatten it, as in the Outline Pen dialog.
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr("docker.nib_stretch"));
                            let mut pct = s.stretch * 100.0;
                            if ui
                                .add(
                                    crate::ui::field::NumField::new(&mut pct)
                                        .range(10.0..=100.0)
                                        .speed(1.0)
                                        .max_decimals(0)
                                        .suffix(" %")
                                        .width(60.0)
                                        .mixed(mix.has(StrokeField::Stretch)),
                                )
                                .rich_tip(crate::ui::tips::pair(
                                    "docker.nib_stretch_tip",
                                    "docker.nib_stretch_desc",
                                ))
                                .changed()
                            {
                                s.stretch = (pct / 100.0).clamp(0.1, 1.0);
                                touched.push(StrokeField::Stretch);
                                changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(tr("docker.nib_angle_lc"));
                            if ui
                                .add(
                                    crate::ui::field::NumField::new(&mut s.nib_angle)
                                        .range(-90.0..=90.0)
                                        .speed(1.0)
                                        .max_decimals(1)
                                        .suffix("°")
                                        .width(60.0)
                                        .mixed(mix.has(StrokeField::NibAngle)),
                                )
                                .rich_tip(crate::ui::tips::pair(
                                    "docker.nib_angle",
                                    "docker.nib_angle_desc",
                                ))
                                .changed()
                            {
                                touched.push(StrokeField::NibAngle);
                                changed = true;
                            }
                        });
                    });
                    if let Some((stretch, angle)) =
                        crate::ui::outline_pen::nib_picture(ui, s.stretch, s.nib_angle)
                    {
                        s.stretch = stretch;
                        s.nib_angle = angle;
                        touched.push(StrokeField::Stretch);
                        touched.push(StrokeField::NibAngle);
                        changed = true;
                    }
                });
            });
        advanced.header_response.rich_tip(crate::ui::tips::pair(
            "docker.outline_advanced",
            "docker.outline_advanced_desc",
        ));
    }
    changed
}

/// Stable, language-independent object type for scripts (`Shape.Type`).
pub fn kind_id(k: &ShapeKind) -> &'static str {
    match k {
        ShapeKind::Rect { .. } => "Rectangle",
        ShapeKind::Ellipse { .. } => "Ellipse",
        ShapeKind::Polygon {
            complex: Some(_), ..
        } => "ComplexStar",
        ShapeKind::Polygon { sharpness, .. }
            if tracedraw_core::geometry::is_star_sharpness(*sharpness) =>
        {
            "Star"
        }
        ShapeKind::Polygon { .. } => "Polygon",
        ShapeKind::Path { .. } => "Curve",
        ShapeKind::Text { .. } => "Text",
        ShapeKind::Group { .. } => "Group",
        ShapeKind::Bitmap { .. } => "Bitmap",
        ShapeKind::ClipFrame { .. } => "ClipFrame",
        ShapeKind::Table(_) => "Table",
        ShapeKind::SymbolInstance { .. } => "Symbol",
    }
}

pub fn kind_name(k: &ShapeKind) -> String {
    match k {
        ShapeKind::Rect { .. } => tr("kind.rectangle"),
        ShapeKind::Ellipse { .. } => tr("kind.ellipse"),
        ShapeKind::Polygon {
            sharpness,
            points,
            complex,
            ..
        } => {
            let n = points.to_string();
            if complex.is_some() {
                trf("kind.complex_star_n", &[("n", &n)])
            } else if tracedraw_core::geometry::is_star_sharpness(*sharpness) {
                trf("kind.star_n", &[("n", &n)])
            } else {
                trf("kind.polygon_n", &[("n", &n)])
            }
        }
        ShapeKind::Path { path, .. } => {
            let n = tracedraw_core::nodes::nodes(path).len();
            trf("kind.curve_n", &[("n", &n.to_string())])
        }
        ShapeKind::Text { spans, frame, .. } => {
            let t: String = spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>()
                .chars()
                .take(20)
                .collect();
            let key = if frame.is_some() {
                "kind.paragraph_text"
            } else {
                "kind.artistic_text"
            };
            trf(key, &[("t", &t)])
        }
        ShapeKind::Group { children } => trf("kind.group_n", &[("n", &children.len().to_string())]),
        ShapeKind::Bitmap {
            width_px,
            height_px,
            ..
        } => trf(
            "kind.bitmap_n",
            &[("w", &width_px.to_string()), ("h", &height_px.to_string())],
        ),
        ShapeKind::ClipFrame { contents, .. } => {
            trf("kind.clip_frame_n", &[("n", &contents.len().to_string())])
        }
        ShapeKind::Table(t) => trf(
            "kind.table_n",
            &[("r", &t.rows().to_string()), ("c", &t.cols().to_string())],
        ),
        ShapeKind::SymbolInstance { index } => {
            trf("kind.symbol_n", &[("n", &(index + 1).to_string())])
        }
    }
}

fn undo_docker(app: &mut App, ui: &mut Ui) {
    let (undo, redo) = app.engine.history_labels();
    ui.label(
        egui::RichText::new(tr("docker.undo_hint"))
            .color(Tokens::text_dim())
            .size(11.0),
    );
    ui.add_space(4.0);
    let n = undo.len();
    let mut goto: Option<usize> = None;
    for (i, l) in undo.iter().enumerate() {
        let is_last = i + 1 == n;
        let desc = if is_last {
            "docker.undo_row_last_desc"
        } else {
            "docker.undo_row_desc"
        };
        if ui
            .selectable_label(is_last, l.to_string())
            .rich_tip_with(|| format!("{l}\n{}", tr(desc)))
            .clicked()
            && !is_last
        {
            goto = Some(n - 1 - i);
        }
    }
    if let Some(steps) = goto {
        for _ in 0..steps {
            app.undo();
        }
    }
    for l in redo.iter().rev() {
        let _ = ui
            .selectable_label(
                false,
                egui::RichText::new(l.to_string()).color(Tokens::text_dim()),
            )
            .rich_tip_with(|| format!("{l}\n{}", tr("docker.redo_row_desc")));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(n > 0, egui::Button::new(tr("docker.undo")))
            .rich_tip(crate::ui::tips::keyed("docker.undo"))
            .clicked()
        {
            app.undo();
        }
        if ui
            .add_enabled(!redo.is_empty(), egui::Button::new(tr("docker.redo")))
            .rich_tip(crate::ui::tips::keyed("docker.redo"))
            .clicked()
        {
            app.redo();
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The built-in two-colour pattern names follow the interface
    /// language (#2509): every language has each of them, and in
    /// Portuguese none stays in English.
    #[test]
    fn the_two_colour_pattern_names_are_translated() {
        for code in ["en", "pt-BR", "de", "ja", "ar"] {
            crate::i18n::with_thread_language(code, || {
                for t in PatternTile::ALL {
                    let name = pattern_tile_name(t);
                    assert_ne!(name, pattern_tile_key(t), "{code}: {t:?}");
                    assert!(!name.is_empty(), "{code}: {t:?}");
                }
            });
        }
        crate::i18n::with_thread_language("pt-BR", || {
            for t in PatternTile::ALL {
                assert_ne!(pattern_tile_name(t), t.name(), "{t:?}");
            }
            assert_eq!(pattern_tile_name(PatternTile::Bricks), "Tijolos");
        });
    }

    /// A docker dropped over a column joins it; just inside a side's only
    /// column (on the drawing's side) it makes a second column there;
    /// over the second column it joins that; near a window side with no
    /// column, the column there; elsewhere it floats.
    #[test]
    fn a_docker_dropped_beside_a_column_makes_a_second_column() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        let edge = egui::Rect::from_min_max(egui::pos2(1233.0, 100.0), egui::pos2(1600.0, 960.0));
        let one = [(DockerPlace::Right, edge)];
        let at = |cols: &[(DockerPlace, egui::Rect)], x: f32, y: f32| {
            drop_target(screen, cols, egui::pos2(x, y))
        };
        assert_eq!(at(&one, 1400.0, 500.0), DockerPlace::Right);
        assert_eq!(at(&one, 1200.0, 500.0), DockerPlace::RightInner);
        assert_eq!(at(&one, 1233.0 - 56.0, 500.0), DockerPlace::RightInner);
        assert_eq!(at(&one, 1100.0, 500.0), DockerPlace::Floating);
        assert_eq!(at(&one, 20.0, 500.0), DockerPlace::Left);
        // The band: a column's width on the drawing's side of the column.
        let band = drop_zone(
            screen,
            24.0,
            &one,
            DockerPlace::RightInner,
            egui::pos2(1200.0, 500.0),
            egui::Vec2::ZERO,
        );
        assert_eq!(band.right(), edge.left());
        assert_eq!(band.width(), STRIP_W + 340.0);
        assert_eq!(band.y_range(), edge.y_range());
        // With a second column: over it joins it, and no third is made.
        let inner = egui::Rect::from_min_max(egui::pos2(866.0, 100.0), egui::pos2(1233.0, 960.0));
        let two = [(DockerPlace::Right, edge), (DockerPlace::RightInner, inner)];
        assert_eq!(at(&two, 1000.0, 500.0), DockerPlace::RightInner);
        assert_eq!(at(&two, 1400.0, 500.0), DockerPlace::Right);
        assert_eq!(at(&two, 840.0, 500.0), DockerPlace::Floating);
        let band = drop_zone(
            screen,
            24.0,
            &two,
            DockerPlace::RightInner,
            egui::pos2(1000.0, 500.0),
            egui::Vec2::ZERO,
        );
        assert_eq!(band, inner);
        // The left side, mirrored.
        let left = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(367.0, 960.0));
        let cols = [(DockerPlace::Right, edge), (DockerPlace::Left, left)];
        assert_eq!(at(&cols, 380.0, 500.0), DockerPlace::LeftInner);
        // A docker alone in its column has nothing to stand beside.
        let mut app = App::headless();
        assert!(can_take_inner(
            &app,
            DockerPlace::RightInner,
            DockerTab::Hints
        ));
        assert!(!can_take_inner(
            &app,
            DockerPlace::LeftInner,
            DockerTab::Hints
        ));
        app.move_docker(DockerTab::Hints, DockerPlace::Left, None);
        assert!(!can_take_inner(
            &app,
            DockerPlace::LeftInner,
            DockerTab::Hints
        ));
        assert!(can_take_inner(
            &app,
            DockerPlace::LeftInner,
            DockerTab::Objects
        ));
    }

    pub(crate) fn draw(ctx: &egui::Context, app: &mut App) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
        out.textures_delta.clear();
    }

    /// The texture editor applies its Scale slider however the value
    /// changes: a value typed in the slider's box and an arrow key with
    /// the groove focused count, not only the end of a drag (they snapped
    /// back before).
    #[test]
    fn the_fill_editor_sliders_apply_typed_and_stepped_values() {
        let ctx = crate::theme::ui_context();
        let mut t = Texture {
            kind: Default::default(),
            color_a: tracedraw_core::Color::BLACK,
            color_b: tracedraw_core::Color::WHITE,
            scale: 10.0,
            seed: 1,
        };
        let run = |t: &mut Texture, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 900.0),
                )),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let mut out = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| changed = texture_editor(ui, t));
            });
            out.textures_delta.clear();
            changed
        };
        let key = |key: egui::Key, pressed: bool| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut t, Vec::new());
        run(&mut t, Vec::new());
        let rail = crate::ui::slider::rail_rect(&ctx, &tr("docker.scale_mm")).expect("drawn");
        // Type 25 in the box.
        let in_box = egui::pos2(rail.right() + 26.0, rail.center().y);
        run(&mut t, vec![egui::Event::PointerMoved(in_box)]);
        run(&mut t, vec![press(in_box, true)]);
        run(&mut t, vec![press(in_box, false)]);
        run(&mut t, Vec::new());
        run(&mut t, vec![egui::Event::Text("25".into())]);
        let applied = run(
            &mut t,
            vec![key(egui::Key::Enter, true), key(egui::Key::Enter, false)],
        );
        assert!(applied, "a typed value applies");
        assert!((t.scale - 25.0).abs() < 1e-9, "{}", t.scale);
        // Focus the groove (as Tab does) and step it with an arrow key.
        let groove = crate::ui::slider::rail_id(&ctx, &tr("docker.scale_mm")).expect("drawn");
        ctx.memory_mut(|m| m.request_focus(groove));
        run(&mut t, Vec::new());
        let applied = run(
            &mut t,
            vec![
                key(egui::Key::ArrowRight, true),
                key(egui::Key::ArrowRight, false),
            ],
        );
        assert!(applied, "an arrow key step applies");
        assert!(t.scale > 25.0, "{}", t.scale);
    }

    /// The pointer over a docker tab shows a close cross at its top; a
    /// click on the cross closes that docker, a click lower on the tab
    /// brings it to the front as before.
    #[test]
    fn a_docker_tab_closes_with_the_cross_at_its_top() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        let tabs = app.open_dockers.clone();
        assert!(tabs.len() >= 2, "{tabs:?}");
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(27.0, 900.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| tab_strip(app, ui, DockerPlace::Right));
            out.textures_delta.clear();
        };
        run(&mut app, Vec::new());
        // The first tab starts under the title band; its cross sits in
        // the slot at its top.
        let first = tabs[0];
        let cross = egui::pos2(13.5, TITLE_H + 2.0 + (CLOSE_SLOT - 2.0) / 2.0);
        let click = |app: &mut App, at: egui::Pos2| {
            run(app, vec![egui::Event::PointerMoved(at)]);
            run(app, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                run(
                    app,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            run(app, Vec::new());
        };
        // Lower on the tab: brought to the front, still open.
        app.docker_tab = tabs[1];
        click(&mut app, egui::pos2(13.5, TITLE_H + CLOSE_SLOT + 30.0));
        assert_eq!(app.docker_tab, first);
        assert!(app.open_dockers.contains(&first));
        // On the cross: closed.
        click(&mut app, cross);
        assert!(!app.open_dockers.contains(&first), "{:?}", app.open_dockers);
    }

    /// The docker in front has its tab marked in the colour of the tab in
    /// front, as the document tabs are, in every theme: light blue on the
    /// light themes. It was a grey close to the other tabs' (#2316).
    #[test]
    fn the_tab_of_the_docker_in_front_is_marked() {
        let ctx = crate::theme::ui_context();
        for theme in crate::theme::Theme::ALL {
            crate::theme::set_theme(theme);
            let on = tab_fill(true, false);
            assert_ne!(on, tab_fill(false, false), "{theme:?}");
            assert_ne!(on, tab_fill(false, true), "{theme:?}");
            if theme != crate::theme::Theme::HighContrast {
                assert_eq!(on, Tokens::tab_active(), "{theme:?}");
            }
            let mut app = App::headless();
            let tab = app.open_dockers[0];
            let place = app.docker_place(tab).unwrap_or(DockerPlace::Right);
            app.set_docker_column(place, true, Some(tab));
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(STRIP_W, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| tab_strip(&mut app, ui, place),
            );
            out.textures_delta.clear();
            let filled = out
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, egui::epaint::Shape::Rect(r) if r.fill == on));
            assert!(filled, "{theme:?}: no tab in {on:?}");
        }
        crate::theme::set_theme(crate::theme::Theme::Light);
    }

    /// More docker tabs than the column's height: the mouse wheel over
    /// the column slides it, so the last tab can be clicked to the
    /// front. It lay past the window's bottom edge, out of reach (#950).
    #[test]
    fn docker_tabs_past_the_column_bottom_come_into_reach() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        for tab in DockerTab::MENU_GROUPS[0].iter() {
            if !app.open_dockers.contains(tab) {
                app.move_docker(*tab, DockerPlace::Right, None);
            }
        }
        let first = app.open_dockers[0];
        app.docker_tab = first;
        let last = *app.open_dockers.last().unwrap();
        let height = 400.0;
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(27.0, height),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| tab_strip(app, ui, DockerPlace::Right));
            out.textures_delta.clear();
        };
        let click = |app: &mut App, at: egui::Pos2| {
            run(app, vec![egui::Event::PointerMoved(at)]);
            for pressed in [true, false] {
                run(
                    app,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            run(app, Vec::new());
        };
        run(&mut app, Vec::new());
        run(&mut app, Vec::new());
        // Wheel down over the column until it slides no further.
        let over = egui::pos2(13.5, height / 2.0);
        for _ in 0..200 {
            run(
                &mut app,
                vec![
                    egui::Event::PointerMoved(over),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -200.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                    },
                ],
            );
        }
        for _ in 0..60 {
            run(&mut app, vec![egui::Event::PointerMoved(over)]);
        }
        // The last tab lies just above the add button and the down arrow
        // at the column's foot: a click there brings it to the front.
        click(
            &mut app,
            egui::pos2(
                13.5,
                height - crate::ui::overflow::BUTTON - 22.0 - 8.0 - 30.0,
            ),
        );
        assert_eq!(app.docker_tab, last, "{:?}", app.open_dockers);
    }

    /// A tab dragged along its column and dropped between two other
    /// tabs goes there; the column keeps the tab it had in front (#1051).
    #[test]
    fn a_docker_tab_dragged_along_its_column_changes_the_order() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        let run = |app: &mut App, events: Vec<egui::Event>| {
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
        };
        let press = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let rect_of = |t: DockerTab| {
            let l = ctx
                .data(|d| d.get_temp::<StripLayout>(strip_id(DockerPlace::Right)))
                .expect("the right column's tabs are laid out");
            l.tabs.iter().find(|(x, _)| *x == t).expect("tab drawn").1
        };
        // A tab pressed at its middle, dragged to `to` and let go there.
        let drag = |app: &mut App, tab: DockerTab, to: egui::Pos2| {
            let from = rect_of(tab).center();
            run(
                app,
                vec![egui::Event::PointerMoved(from), press(from, true)],
            );
            for k in 1..=4 {
                let p = from + (to - from) * (k as f32 / 4.0);
                run(app, vec![egui::Event::PointerMoved(p)]);
            }
            assert!(app.docker_drag.is_some(), "the tab is being dragged");
            run(app, vec![press(to, false)]);
            assert!(app.docker_drag.is_none());
            run(app, Vec::new());
        };
        run(&mut app, Vec::new());
        run(&mut app, Vec::new());
        let before = app.open_dockers.clone();
        assert!(before.len() >= 3, "{before:?}");
        let (front, shown) = (app.docker_tab, app.show_dockers);
        // The last tab dragged onto the top half of the first one: it
        // goes first.
        let last = *before.last().unwrap();
        let first = rect_of(before[0]);
        drag(
            &mut app,
            last,
            egui::pos2(first.center().x, first.top() + CLOSE_SLOT + 4.0),
        );
        let mut want = before.clone();
        want.pop();
        want.insert(0, last);
        assert_eq!(app.open_dockers, want);
        assert_eq!((app.docker_tab, app.show_dockers), (front, shown));
        // And dragged onto the bottom of the last tab it goes last again.
        let end = rect_of(*want.last().unwrap());
        drag(
            &mut app,
            last,
            egui::pos2(end.center().x, end.bottom() - 4.0),
        );
        assert_eq!(app.open_dockers, before);
        assert_eq!((app.docker_tab, app.show_dockers), (front, shown));
    }

    /// The collapse button of a docker column points at the window edge
    /// the column folds toward: right for the right column, left for a
    /// column docked at the left (it pointed right on both sides). Read
    /// from the shapes the window paints.
    #[test]
    fn the_collapse_arrows_point_to_the_side_of_their_column() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        app.move_docker(DockerTab::Objects, DockerPlace::Left, None);
        app.set_docker_column(DockerPlace::Left, true, Some(DockerTab::Objects));
        app.set_docker_column(DockerPlace::Right, true, Some(DockerTab::Hints));
        let mut shapes = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| crate::ui::root(&mut app, ui));
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        assert_eq!(
            app.docker_place(DockerTab::Objects),
            Some(DockerPlace::Left)
        );
        // Which way the triangles drawn in a button's rectangle point:
        // the tip is the corner off the line of the other two.
        let pointing = |place: DockerPlace| -> Vec<f32> {
            let r = ctx
                .read_response(egui::Id::new(("docker_fold", place as u8)))
                .map(|r| r.rect)
                .expect("the collapse button");
            let mut dirs = Vec::new();
            for clipped in &shapes {
                if let egui::Shape::Path(p) = &clipped.shape {
                    if p.points.len() == 3 && p.points.iter().all(|q| r.contains(*q)) {
                        let [a, b, c] = [p.points[0], p.points[1], p.points[2]];
                        let base = (a.x + c.x) / 2.0;
                        dirs.push((b.x - base).signum());
                    }
                }
            }
            dirs
        };
        assert_eq!(pointing(DockerPlace::Right), vec![1.0, 1.0]);
        assert_eq!(pointing(DockerPlace::Left), vec![-1.0, -1.0]);
        // In pictures: the left column's button is mirrored.
        let r = egui::Rect::from_center_size(egui::pos2(50.0, 50.0), egui::vec2(20.0, 20.0));
        let right = fold_arrows(r, false);
        let left = fold_arrows(r, true);
        for (a, b) in right.iter().flatten().zip(left.iter().flatten()) {
            assert!((a.x - 50.0 + (b.x - 50.0)).abs() < 1e-4 && a.y == b.y);
        }
    }

    /// A docker dragged by its tab (or its title) and dropped near the
    /// left edge goes to the column there, and the window draws with both
    /// columns and a floating docker.
    #[test]
    fn a_docker_dragged_to_a_side_docks_there_and_the_window_draws_them_all() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        let run = |app: &mut App, events: Vec<egui::Event>| {
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
        };
        run(&mut app, Vec::new());
        // A drag under way (as a tab starts one), the pointer held down
        // at the left edge: the band shows; the release docks it there.
        app.docker_drag = Some(DockerDrag {
            tab: DockerTab::Hints,
            grab: egui::vec2(10.0, 10.0),
            last: None,
        });
        let at = egui::pos2(20.0, 450.0);
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(app.docker_drag.is_some());
        assert_eq!(app.docker_place(DockerTab::Hints), Some(DockerPlace::Right));
        run(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.docker_drag.is_none());
        assert_eq!(app.docker_place(DockerTab::Hints), Some(DockerPlace::Left));
        assert!(app.show_left_dockers && app.left_tab == DockerTab::Hints);
        // The middle of the window: floating, with its top left corner
        // where the grab point puts it.
        app.docker_drag = Some(DockerDrag {
            tab: DockerTab::Objects,
            grab: egui::vec2(10.0, 10.0),
            last: None,
        });
        let at = egui::pos2(600.0, 300.0);
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        run(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            app.docker_place(DockerTab::Objects),
            Some(DockerPlace::Floating)
        );
        let fd = app.floating_dockers[0];
        assert_eq!(fd.pos, egui::pos2(590.0, 290.0));
        // Both columns and the floating window draw, and the floating
        // window keeps near where it was put.
        for _ in 0..3 {
            run(&mut app, Vec::new());
        }
        let fd = app.floating_dockers[0];
        assert!(
            (fd.pos.x - 590.0).abs() < 2.0 && (fd.pos.y - 290.0).abs() < 2.0,
            "{fd:?}"
        );
        assert!(fd.size.x > 200.0 && fd.size.y > 200.0, "{fd:?}");
        assert_eq!(app.open_dockers, [DockerTab::Properties]);
        // Dropped at the right edge: back in the default column, in front.
        assert_eq!(
            drop_target(ctx.content_rect(), &[], egui::pos2(1390.0, 400.0)),
            DockerPlace::Right
        );
        app.move_docker(DockerTab::Objects, DockerPlace::Right, None);
        assert!(app.floating_dockers.is_empty());
        assert_eq!(
            app.open_dockers,
            [DockerTab::Properties, DockerTab::Objects]
        );
        assert_eq!(app.docker_tab, DockerTab::Objects);
        run(&mut app, Vec::new());
    }

    /// Dragging in the outline colour picker, as with the fill: every
    /// colour passed over shows on the outline, but the drag is one undo
    /// step and only the colour it ends on joins the document palette.
    #[test]
    fn an_outline_dragged_in_the_picker_is_one_step_and_one_palette_colour() {
        use tracedraw_core::{Color, Stroke};
        let mut app = App::headless();
        app.settings.palette.auto_update_document = true;
        let id = app
            .new_shape(tracedraw_core::ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        let stroke = |app: &App| app.doc().find_shape(id).expect("shape").stroke.clone();
        let before = stroke(&app);
        let old = Some(before.clone().unwrap_or_default());
        let steps = |app: &App| app.engine.history_labels().0.len();
        let n = steps(&app);
        let palette = app.doc().palette.clone();
        let colours = [
            Color::rgb8(250, 10, 10),
            Color::rgb8(10, 250, 10),
            Color::rgb8(10, 10, 250),
        ];
        for c in colours {
            let new = Some(Stroke {
                color: c,
                ..old.clone().unwrap_or_default()
            });
            app.patch_selection_stroke_live(&old, &new, &[]);
            assert_eq!(stroke(&app).map(|s| s.color), Some(c));
            assert_eq!(steps(&app), n + 1, "one step for the drag");
            assert_eq!(app.doc().palette, palette, "nothing added while dragging");
        }
        // A frame with the button up ends the drag: the last colour only.
        let ctx = crate::theme::ui_context();
        draw(&ctx, &mut app);
        assert!(app.live_stroke.is_none());
        let added: Vec<Color> = app
            .doc()
            .palette
            .iter()
            .filter(|c| !palette.contains(c))
            .copied()
            .collect();
        assert_eq!(added, vec![colours[2]]);
        assert_eq!(steps(&app), n + 1);
        // One undo takes the whole drag back, palette colour included.
        app.undo();
        assert_eq!(stroke(&app), before);
        assert_eq!(app.doc().palette, palette);
    }

    /// Dragging in the fill colour picker: every colour passed over shows
    /// on the object, but the drag is one undo step and only the colour
    /// it ends on joins the document palette, when the button is let go.
    #[test]
    fn a_fill_dragged_in_the_picker_is_one_step_and_one_palette_colour() {
        use tracedraw_core::Color;
        let mut app = App::headless();
        app.settings.palette.auto_update_document = true;
        let id = app
            .new_shape(tracedraw_core::ShapeKind::Rect {
                rect: tracedraw_core::Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        let before_fill = app.doc().find_shape(id).expect("shape").fill.clone();
        let steps = |app: &App| app.engine.history_labels().0.len();
        let n = steps(&app);
        let palette = app.doc().palette.clone();
        let colours = [
            Color::rgb8(250, 10, 10),
            Color::rgb8(10, 250, 10),
            Color::rgb8(10, 10, 250),
        ];
        for c in colours {
            app.apply_fill_live(Fill::Solid(c));
            assert_eq!(
                app.doc().find_shape(id).expect("shape").fill,
                Fill::Solid(c)
            );
            assert_eq!(steps(&app), n + 1, "one step for the drag");
            assert_eq!(app.doc().palette, palette, "nothing added while dragging");
        }
        // A frame with the button up ends the drag: the last colour only.
        let ctx = crate::theme::ui_context();
        draw(&ctx, &mut app);
        assert!(app.live_fill.is_none());
        let added: Vec<Color> = app
            .doc()
            .palette
            .iter()
            .filter(|c| !palette.contains(c))
            .copied()
            .collect();
        assert_eq!(added, vec![colours[2]]);
        assert_eq!(steps(&app), n + 1);
        // One undo takes the whole drag back, palette colour included.
        app.undo();
        assert_eq!(app.doc().find_shape(id).expect("shape").fill, before_fill);
        assert_eq!(app.doc().palette, palette);
        // A new drag after other work starts its own step.
        app.apply_fill_live(Fill::Solid(colours[0]));
        app.end_live_fill();
        app.apply_fill_live(Fill::Solid(colours[1]));
        app.end_live_fill();
        assert_eq!(steps(&app), n + 2);
    }

    /// A procedural fill shows its section in the Properties panel: the
    /// pattern's name, the numbers the pattern uses (and not the others)
    /// and the colour rows; a change of one number through the panel's
    /// path is one undo step that goes into each object's own fill, which
    /// keeps its other numbers (#1212).
    #[test]
    fn procedural_fill_section_shows_its_fields_and_patches_one_at_a_time() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let mut ids = Vec::new();
        for x in [10.0, 70.0] {
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: tracedraw_core::geometry::Rect::new(x, 10.0, x + 40.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            ids.push(id);
        }
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        // The Outline section closed, so the fill's rows are in view (the
        // docker paints only what its scroll area shows).
        app.settings.properties_closed = vec!["outline".into()];
        let mut spirals = Procedural::new(ProceduralPattern::Spirals, Color::rgb8(200, 0, 0));
        spirals.params.size_mm = 8.0;
        app.select(vec![ids[0]]);
        app.apply_fill(Fill::Procedural(spirals.clone()));
        let texts = painted_texts(&ctx, &mut app);
        let count = |t: &str| texts.iter().filter(|x| x.as_str() == t).count();
        assert_eq!(count("Spirals"), 1, "{texts:?}");
        assert_eq!(count("line width mm"), 1, "{texts:?}");
        assert_eq!(count("turns"), 1, "{texts:?}");
        assert_eq!(count("Foreground"), 1, "{texts:?}");
        assert_eq!(count("Background"), 1, "{texts:?}");
        // Spirals have no cover, amplitude or specks; the seed is the
        // speckle's.
        assert_eq!(
            count("cover %") + count("amplitude %") + count("specks per tile"),
            0
        );
        let seeds = count("Seed");
        let mut speckle = spirals.clone();
        speckle.pattern = ProceduralPattern::Speckle;
        app.apply_fill(Fill::Procedural(speckle));
        let texts = painted_texts(&ctx, &mut app);
        assert_eq!(texts.iter().filter(|x| x.as_str() == "Speckle").count(), 1);
        assert_eq!(
            texts
                .iter()
                .filter(|x| x.as_str() == "specks per tile")
                .count(),
            1
        );
        assert_eq!(
            texts.iter().filter(|x| x.as_str() == "Seed").count(),
            seeds + 1
        );

        // Two objects, tiles of 8 and 20 mm: a new line width goes into
        // both as one step, their sizes kept.
        let mut big = spirals.clone();
        big.params.size_mm = 20.0;
        app.select(vec![ids[1]]);
        app.apply_fill(Fill::Procedural(big.clone()));
        app.select(ids.clone());
        let steps = |app: &App| app.engine.history_labels().0.len();
        let n = steps(&app);
        let old = Fill::Procedural(spirals.clone());
        let mut wide = spirals.clone();
        wide.params.line_width_mm = 1.5;
        app.patch_selection_fill(&old, &Fill::Procedural(wide), &[]);
        assert_eq!(steps(&app), n + 1);
        let fill_of = |app: &App, id| app.doc().find_shape(id).expect("shape").fill.clone();
        let (Fill::Procedural(a), Fill::Procedural(b)) =
            (fill_of(&app, ids[0]), fill_of(&app, ids[1]))
        else {
            panic!("procedural fills");
        };
        assert_eq!((a.params.line_width_mm, b.params.line_width_mm), (1.5, 1.5));
        assert_eq!((a.params.size_mm, b.params.size_mm), (8.0, 20.0));
        // The panel shows the width once, the size as mixed.
        let mix = app.selection_fill_mix();
        assert!(mix.has(FillField::ProceduralSize));
        assert!(!mix.has(FillField::ProceduralLineWidth));
        app.undo();
        let Fill::Procedural(a) = fill_of(&app, ids[0]) else {
            panic!("a procedural fill");
        };
        assert_eq!(a.params.line_width_mm, 0.5);
    }

    /// Every text the frame painted.
    pub(crate) fn painted_texts(ctx: &egui::Context, app: &mut App) -> Vec<String> {
        fn walk(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut texts);
        }
        texts
    }

    /// The Properties docker shows the gradient's blend direction, steps,
    /// smooth transition and acceleration rows, with the values the fill
    /// has; a change through the fill patching goes into each object's
    /// own gradient as one undo step (#1208).
    #[test]
    fn gradient_blend_rows_show_and_patch() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let mut ids = Vec::new();
        for x in [10.0, 70.0] {
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: tracedraw_core::geometry::Rect::new(x, 10.0, x + 40.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            ids.push(id);
        }
        app.select(ids.clone());
        let mut f = Fountain::two(
            FountainKind::Linear,
            tracedraw_core::Color::BLACK,
            tracedraw_core::Color::WHITE,
            0.0,
        );
        f.steps = 24;
        f.acceleration = -37;
        f.blend_direction = ColorBlend::CounterClockwise;
        app.apply_fill(Fill::Fountain(f.clone()));
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        // The Outline section closed, so the fill's rows are in view (the
        // docker paints only what its scroll area shows).
        toggle_section(&mut app, "outline");
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        let count = |s: &str| texts.iter().filter(|t| t.as_str() == s).count();
        for key in [
            "docker.blend_direction",
            "docker.blend_direct",
            "docker.blend_clockwise",
            "docker.blend_counterclockwise",
            "docker.steps",
            "docker.smooth_transition",
            "docker.fountain_acceleration",
        ] {
            assert!(count(&tr(key)) >= 1, "{key} shows: {texts:?}");
        }
        // The step count and the acceleration as the fill has them.
        assert!(count("24") >= 1, "{texts:?}");
        assert!(count("-37") >= 1, "{texts:?}");
        // Unticked steps hide the count field.
        let mut smooth = f.clone();
        smooth.steps = Fountain::SMOOTH_STEPS;
        app.apply_fill(Fill::Fountain(smooth.clone()));
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert!(!texts.iter().any(|t| t == "24"), "{texts:?}");
        // The second object gets its own angle; an edit of the smooth
        // transition then changes only that on each, in one undo step.
        let mut turned = smooth.clone();
        turned.angle = 60.0;
        app.run(Command::PatchFill {
            shapes: vec![ids[1]],
            fill: Fill::Fountain(turned),
            fields: vec![FillField::Angle],
        });
        let steps = app.engine.history_labels().0.len();
        let mut edited = smooth.clone();
        edited.smooth = true;
        edited.steps = 5;
        app.patch_selection_fill(&Fill::Fountain(smooth), &Fill::Fountain(edited), &[]);
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        for (i, angle) in [(0, 0.0), (1, 60.0)] {
            let Fill::Fountain(g) = &app.doc().find_shape(ids[i]).expect("shape").fill else {
                panic!("a gradient");
            };
            assert_eq!((g.smooth, g.steps, g.angle), (true, 5, angle));
            assert_eq!(g.blend_direction, ColorBlend::CounterClockwise);
            assert_eq!(g.acceleration, -37);
        }
        app.undo();
        let Fill::Fountain(g) = &app.doc().find_shape(ids[1]).expect("shape").fill else {
            panic!("a gradient");
        };
        assert_eq!((g.smooth, g.steps, g.angle), (false, 256, 60.0));
    }

    /// The Fill part of a pattern fill shows the placement fields, with
    /// one object's values, and the values that differ across two objects
    /// as mixed; a placement change goes into each object's own pattern
    /// (each keeps its tile) as one undo step.
    #[test]
    fn pattern_placement_fields_show_and_patch_each_pattern() {
        use tracedraw_core::style::PatternPlacement;
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let pattern = |tile: PatternTile, rotation: f64| {
            Fill::Pattern(Pattern::TwoColor {
                tile,
                front: tracedraw_core::Color::BLACK,
                back: tracedraw_core::Color::WHITE,
                size_mm: 10.0,
                placement: PatternPlacement {
                    rotation,
                    ..PatternPlacement::default()
                },
            })
        };
        let mut ids = Vec::new();
        for (x, tile, rotation) in [
            (10.0, PatternTile::Checker, 0.0),
            (70.0, PatternTile::Dots, 33.0),
        ] {
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: tracedraw_core::geometry::Rect::new(x, 10.0, x + 40.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            app.run(Command::SetFill {
                shapes: vec![id],
                fill: pattern(tile, rotation),
            });
            ids.push(id);
        }
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        // The Outline part closed, so the whole Fill part is in view.
        app.settings.properties_closed = vec!["outline".into()];
        let mixed = tr("docker.mixed");
        app.select(ids[1..].to_vec());
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        for key in [
            "docker.tile_size",
            "docker.mirror_tiles",
            "docker.row_column_offset",
            "docker.transform_with_object",
        ] {
            assert!(texts.iter().any(|t| *t == tr(key)), "{key}: {texts:?}");
        }
        assert!(!texts.contains(&mixed));
        assert!(
            texts.iter().any(|t| t.starts_with("33")),
            "the rotation shows: {texts:?}"
        );
        // Both: the tiles differ, so do the rotations; the panel says so.
        app.select(ids.clone());
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert!(texts.contains(&mixed), "{texts:?}");
        assert!(!texts.iter().any(|t| t.starts_with("33")), "{texts:?}");
        let mix = app.selection_fill_mix();
        assert!(mix.fields.contains(&FillField::TileRotation) && mix.has(FillField::All));

        // A rotation set on the selection: each object's own tile turned,
        // one step.
        let steps = |app: &App| app.engine.history_labels().0.len();
        let n = steps(&app);
        let old = pattern(PatternTile::Checker, 0.0);
        let new = pattern(PatternTile::Checker, 45.0);
        app.patch_selection_fill(&old, &new, &[]);
        assert_eq!(steps(&app), n + 1);
        for (id, tile) in ids.iter().zip([PatternTile::Checker, PatternTile::Dots]) {
            let s = app.doc().find_shape(*id).expect("shape");
            let Fill::Pattern(p) = &s.fill else {
                panic!("a pattern");
            };
            assert!(matches!(p, Pattern::TwoColor { tile: t, .. } if *t == tile));
            assert_eq!(p.placement().rotation, 45.0);
        }
    }

    /// Two objects with different outline colours and widths: the panel
    /// shows the colour and the width as mixed (no colour, an empty width
    /// box); one object alone shows its values.
    #[test]
    fn mixed_values_show_empty() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        // Widths in millimetres, as the numbers below are.
        app.settings.outline_units = crate::app::Units::Millimeters.id().to_string();
        let mut ids = Vec::new();
        for (x, c, w) in [
            (10.0, tracedraw_core::Color::rgb8(0, 0, 255), 1.234),
            (70.0, tracedraw_core::Color::rgb8(255, 255, 0), 3.456),
        ] {
            let id = app
                .new_shape(ShapeKind::Rect {
                    rect: tracedraw_core::geometry::Rect::new(x, 10.0, x + 40.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            app.run(Command::SetStroke {
                shapes: vec![id],
                stroke: Some(Stroke::new(c, w)),
            });
            ids.push(id);
        }
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        let mixed = tr("docker.mixed");
        app.select(ids[..1].to_vec());
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert!(!texts.iter().any(|t| *t == mixed));
        assert!(texts.iter().any(|t| t == "1.234"), "the width shows");
        app.select(ids.clone());
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert!(texts.iter().any(|t| *t == mixed), "the colour is mixed");
        assert!(
            !texts
                .iter()
                .any(|t| t.contains("1.234") || t.contains("3.456")),
            "no width shows: {:?}",
            texts
                .iter()
                .filter(|t| t.contains("1.234") || t.contains("3.456"))
                .collect::<Vec<_>>()
        );
        let mix = app.selection_stroke_mix();
        assert!(mix.has(StrokeField::Width) && mix.has(StrokeField::Paint));
    }

    /// #1210: the outline section has the Arrowhead editor's "..." after
    /// each arrowhead list and Share attributes after the end's; a click
    /// on Share attributes gives the end the start's head as one step.
    #[test]
    fn the_arrowhead_editor_and_share_attributes_sit_beside_the_lists() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        let mut p = tracedraw_core::geometry::BezPath::new();
        p.move_to((10.0, 50.0));
        p.line_to((60.0, 50.0));
        let id = app
            .new_shape(ShapeKind::Path {
                path: p,
                closed: false,
            })
            .expect("line");
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(Stroke {
                start_arrow: tracedraw_core::Arrowhead::Diamond,
                ..Stroke::new(tracedraw_core::Color::BLACK, 1.0)
            }),
        });
        app.select(vec![id]);
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        let frame = |app: &mut App, events: Vec<egui::Event>| -> Vec<(String, egui::Rect)> {
            fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
                match s {
                    egui::Shape::Text(t) => out.push((
                        t.galley.text().to_string(),
                        egui::Rect::from_min_size(t.pos, t.galley.size()),
                    )),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1900.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
            out.textures_delta.clear();
            let mut texts = Vec::new();
            out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
            texts
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        let texts = frame(&mut app, Vec::new());
        let share = tr("arrowhead_editor.share");
        assert_eq!(
            texts.iter().filter(|(t, _)| *t == share).count(),
            1,
            "{texts:?}"
        );
        assert!(
            texts.iter().filter(|(t, _)| t == "...").count() >= 2,
            "the editor buttons: {texts:?}"
        );
        // Share attributes: the end takes the start's diamond, one step.
        let at = texts
            .iter()
            .find(|(t, _)| *t == share)
            .map(|(_, r)| r.center())
            .expect("Share attributes");
        let steps = app.engine.history_labels().0.len();
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, vec![press(true)]);
        frame(&mut app, vec![press(false)]);
        frame(&mut app, Vec::new());
        let s = app
            .doc()
            .find_shape(id)
            .and_then(|s| s.stroke.clone())
            .expect("outline");
        assert!(s.share_arrowheads);
        assert_eq!(s.end_arrow, tracedraw_core::Arrowhead::Diamond);
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        // The "..." asks for the editor on that end.
        let dots: Vec<egui::Pos2> = texts
            .iter()
            .filter(|(t, _)| t == "...")
            .map(|(_, r)| r.center())
            .collect();
        let first = dots[0];
        let press = |pressed| egui::Event::PointerButton {
            pos: first,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(first)]);
        frame(&mut app, vec![press(true)]);
        frame(&mut app, vec![press(false)]);
        frame(&mut app, Vec::new());
        assert!(
            matches!(app.dialog, crate::ui::dialogs::Dialog::ArrowheadEditor(_)),
            "{:?}",
            app.dialog
        );
    }

    /// The outline's paint editor works on the outline: uniform is its
    /// colour, the other kinds its own fill, none takes it away.
    #[test]
    fn outline_paint_offers_every_fill_kind() {
        let red = tracedraw_core::Color::rgb8(255, 0, 0);
        let mut s = tracedraw_core::Stroke::new(red, 1.0);
        assert_eq!(outline_paint_of(&s), Fill::Solid(red));
        let texture = Fill::Texture(Texture {
            kind: TextureKind::Wood,
            color_a: red,
            color_b: tracedraw_core::Color::WHITE,
            scale: 5.0,
            seed: 1,
        });
        for paint in [
            Fill::linear(red, tracedraw_core::Color::WHITE, 0.0),
            Fill::Pattern(Pattern::TwoColor {
                tile: PatternTile::Dots,
                front: red,
                back: tracedraw_core::Color::WHITE,
                size_mm: 4.0,
                placement: Default::default(),
            }),
            texture,
            Fill::Mesh(tracedraw_core::Mesh::new(
                tracedraw_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
                1,
                1,
                red,
            )),
        ] {
            assert!(!set_outline_paint(&mut s, paint.clone()));
            assert_eq!(s.fill.as_deref(), Some(&paint));
            assert_eq!(outline_paint_of(&s), paint);
        }
        let blue = tracedraw_core::Color::rgb8(0, 0, 255);
        assert!(!set_outline_paint(&mut s, Fill::Solid(blue)));
        assert_eq!((s.color, s.fill.is_none()), (blue, true));
        assert!(set_outline_paint(&mut s, Fill::None));
        // A plain colour kept as the outline's fill reads as uniform.
        s.fill = Some(Box::new(Fill::Solid(red)));
        assert_eq!(outline_paint_of(&s), Fill::Solid(blue));
        assert_eq!(fountain_kind_key(FountainKind::Shaped), "fill.shaped");
        assert_ne!(tr("fill.shaped"), "fill.shaped");
    }

    /// The Properties docker has the same titled sections with and without
    /// a selection, so each row of paint kinds is named; Add Outline and
    /// Add Fill show only once there is an outline or a fill to add to;
    /// the corner names are never cut short (#487).
    #[test]
    fn properties_sections_are_titled_and_add_buttons_need_a_paint() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        app.settings.autocorrect.enabled = false;
        app.settings.properties_closed.clear();
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        let count = |texts: &[String], key: &str| texts.iter().filter(|t| **t == tr(key)).count();
        // Nothing selected: an Outline title over the outline's kinds and
        // a Fill title over the fill's.
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert_eq!(count(&texts, "docker.fill"), 1, "{texts:?}");
        assert!(
            count(&texts, "docker.outline") >= 2,
            "title and box: {texts:?}"
        );
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 2.0,
                corners: None,
            })
            .expect("rect");
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: None,
        });
        app.run(Command::SetFill {
            shapes: vec![id],
            fill: Fill::None,
        });
        app.select(vec![id]);
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert_eq!(count(&texts, "docker.add_outline"), 0, "{texts:?}");
        assert_eq!(count(&texts, "docker.add_fill"), 0, "{texts:?}");
        // Every corner named in full.
        for key in [
            "toolbar.corner_top_left",
            "toolbar.corner_top_right",
            "toolbar.corner_bottom_left",
            "toolbar.corner_bottom_right",
        ] {
            let name = tr(key);
            assert!(
                texts.iter().any(|t| t.replace('\n', " ") == name),
                "{name} cut short: {texts:?}"
            );
        }
        assert!(!texts.iter().any(|t| t.ends_with('\u{2026}')), "{texts:?}");
        // With an outline and a fill, a further one can be added.
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(Stroke::new(tracedraw_core::Color::BLACK, 1.0)),
        });
        app.run(Command::SetFill {
            shapes: vec![id],
            fill: Fill::Solid(tracedraw_core::Color::BLACK),
        });
        draw(&ctx, &mut app);
        let texts = painted_texts(&ctx, &mut app);
        assert_eq!(count(&texts, "docker.add_outline"), 1, "{texts:?}");
        assert_eq!(count(&texts, "docker.add_fill"), 1, "{texts:?}");
    }

    /// Every outline option stays in the Properties docker, with and
    /// without a selection: the common ones in view, the mitre limit and
    /// the nib under an Advanced header that a click opens (open at once
    /// when one of them is set), and no label cut short (#487).
    #[test]
    fn every_outline_option_is_in_the_properties_docker() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        app.settings.autocorrect.enabled = false;
        app.settings.properties_closed.clear();
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        // The texts painted, with where they are.
        let frame = |app: &mut App, events: Vec<egui::Event>| -> Vec<(String, egui::Rect)> {
            fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
                match s {
                    egui::Shape::Text(t) => out.push((
                        t.galley.text().to_string(),
                        t.galley.rect.translate(t.pos.to_vec2()),
                    )),
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 1900.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
            out.textures_delta.clear();
            let mut texts = Vec::new();
            out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
            texts
        };
        let common = [
            "docker.hairline",
            "docker.caps",
            "docker.cap_butt",
            "docker.cap_round",
            "docker.cap_square",
            "docker.corners",
            "docker.join_miter",
            "docker.join_round",
            "docker.join_bevel",
            "docker.dash",
            "docker.dash_solid",
            "docker.dash_dashed",
            "docker.dash_dotted",
            "docker.dash_dash_dot",
            "docker.align_dashes",
            "docker.edit_line_style",
            "docker.outline_position",
            "docker.position_outside",
            "docker.position_center",
            "docker.position_inside",
            "docker.behind_fill",
            "docker.scale_with_object",
            "docker.outline_advanced",
        ];
        let advanced = [
            "docker.miter_limit",
            "docker.nib_stretch",
            "docker.nib_angle_lc",
        ];
        let has = |texts: &[(String, egui::Rect)], key: &str| {
            let want = tr(key);
            texts
                .iter()
                .any(|(t, _)| t.replace('\n', " ") == want || t.starts_with(&want))
        };
        let check = |app: &mut App, what: &str| {
            for _ in 0..3 {
                frame(app, Vec::new());
            }
            let texts = frame(app, Vec::new());
            for key in common
                .iter()
                .chain(["docker.arrow_start", "docker.arrow_end"].iter())
            {
                assert!(has(&texts, key), "{what}: {key} missing in {texts:?}");
            }
            for key in advanced {
                assert!(!has(&texts, key), "{what}: {key} not under Advanced");
            }
            assert!(
                !texts.iter().any(|(t, _)| t.ends_with('\u{2026}')),
                "{what}: a label cut short in {texts:?}"
            );
            // A click on the header shows the rest.
            let at = texts
                .iter()
                .find(|(t, _)| *t == tr("docker.outline_advanced"))
                .map(|(_, r)| r.center())
                .expect("the Advanced header");
            let press = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(app, vec![egui::Event::PointerMoved(at)]);
            frame(app, vec![press(true)]);
            frame(app, vec![press(false)]);
            for _ in 0..3 {
                frame(app, Vec::new());
            }
            let texts = frame(app, Vec::new());
            for key in advanced {
                assert!(
                    has(&texts, key),
                    "{what}: {key} missing when open: {texts:?}"
                );
            }
            assert!(!texts.iter().any(|(t, _)| t.ends_with('\u{2026}')));
            // Closed again for the next check.
            frame(app, vec![egui::Event::PointerMoved(at)]);
            frame(app, vec![press(true)]);
            frame(app, vec![press(false)]);
        };
        // Nothing selected: the outline of new objects (mitred corners, so
        // the mitre limit shows).
        let mut pen = app.default_stroke().unwrap_or_default();
        pen.join = tracedraw_core::LineJoin::Miter;
        app.change_defaults(
            &crate::defaults::DefaultChange::Outline(Some(pen.clone())),
            [true, true, true],
        );
        check(&mut app, "nothing selected");
        // An object with that outline.
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(pen.clone()),
        });
        app.select(vec![id]);
        check(&mut app, "a rectangle");
        // A nib set away from its default: the header comes open.
        let ctx2 = crate::theme::ui_context();
        let mut app2 = App::headless();
        app2.show_welcome = false;
        app2.settings.autocorrect.enabled = false;
        app2.settings.properties_closed.clear();
        app2.show_dockers = true;
        app2.docker_tab = DockerTab::Properties;
        let id = app2
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app2.run(Command::SetStroke {
            shapes: vec![id],
            stroke: Some(Stroke {
                nib_angle: 30.0,
                ..pen
            }),
        });
        app2.select(vec![id]);
        for _ in 0..3 {
            draw(&ctx2, &mut app2);
        }
        let texts = painted_texts(&ctx2, &mut app2);
        // The slider's label, with the colon the slider adds.
        assert!(
            texts
                .iter()
                .any(|t| t.trim_end_matches(':') == tr("docker.nib_angle_lc")),
            "{texts:?}"
        );
    }

    /// The Properties docker keeps its default width with every section
    /// open, whatever the object, its fill, outline and transparency, in
    /// the languages with the longest words: no section needs more room
    /// than the docker has, and selecting an object never widens it.
    #[test]
    fn properties_sections_fit_the_default_docker_width() {
        use tracedraw_core::geometry::{Point, Rect};
        use tracedraw_core::Color;
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        app.settings.autocorrect.enabled = false;
        app.settings.properties_closed.clear();
        let rect = app
            .new_shape(ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 2.0,
                corners: None,
            })
            .expect("rect");
        let ellipse = app
            .new_shape(ShapeKind::Ellipse {
                rect: Rect::new(70.0, 10.0, 120.0, 40.0),
                arc: None,
            })
            .expect("ellipse");
        let polygon = app
            .new_shape(ShapeKind::Polygon {
                rect: Rect::new(130.0, 10.0, 170.0, 40.0),
                points: 5,
                sharpness: 50.0,
                complex: None,
                twist: 0.0,
            })
            .expect("polygon");
        app.start_text(Point::new(20.0, 80.0), None);
        app.text_insert("Docker test");
        app.finish_text();
        let text = app.selection[0];
        app.start_text(
            Point::new(20.0, 100.0),
            Some(tracedraw_core::geometry::Size::new(60.0, 30.0)),
        );
        app.text_insert("Paragraph");
        app.finish_text();
        let paragraph = app.selection[0];
        app.select(vec![ellipse]);
        app.duplicate();
        app.convert_to_bitmap(50.0, true);
        let bitmap = app.selection[0];
        app.select(Vec::new());
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        let overflow = |ctx: &egui::Context, app: &mut App| {
            for _ in 0..3 {
                draw(ctx, app);
            }
            let need: f32 = ctx
                .data(|d| d.get_temp(egui::Id::new("docker_overflow")))
                .expect("the docker is drawn");
            let width = egui::containers::panel::PanelState::load(ctx, egui::Id::new("dockers"))
                .map(|s| s.size().x)
                .expect("the docker is shown");
            (need, width)
        };
        let red = Color::rgb8(255, 0, 0);
        let fills = [
            Fill::Solid(red),
            Fill::linear(red, Color::WHITE, 0.0),
            Fill::Pattern(Pattern::TwoColor {
                tile: PatternTile::Dots,
                front: red,
                back: Color::WHITE,
                size_mm: 5.0,
                placement: Default::default(),
            }),
            Fill::Texture(tracedraw_core::style::Texture {
                kind: tracedraw_core::style::TextureKind::Wood,
                color_a: red,
                color_b: Color::WHITE,
                scale: 5.0,
                seed: 1,
            }),
        ];
        for lang in ["en", "de", "ru", "pt-BR"] {
            crate::i18n::with_thread_language(lang, || {
                app.select(Vec::new());
                let (need, empty) = overflow(&ctx, &mut app);
                assert!(need <= 0.5, "{lang}, nothing selected: {need} too wide");
                assert!(empty <= 341.0, "{lang}: {empty}");
                for (name, id) in [
                    ("rectangle", rect),
                    ("ellipse", ellipse),
                    ("polygon", polygon),
                    ("text", text),
                    ("paragraph", paragraph),
                    ("bitmap", bitmap),
                ] {
                    app.select(vec![id]);
                    let (need, w) = overflow(&ctx, &mut app);
                    assert!(need <= 0.5, "{lang}, {name}: {need} too wide");
                    assert!((w - empty).abs() < 1.0, "{lang}, {name}: {w} after {empty}");
                }
                app.select(vec![rect]);
                for (k, fill) in fills.iter().enumerate() {
                    app.apply_fill(fill.clone());
                    for kind in 0..4u8 {
                        app.transparency_default.set_kind(kind);
                        let t = app.transparency_default.clone();
                        app.patch_transparency(&t, &[crate::mixed::TransparencyField::Kind]);
                        let (need, w) = overflow(&ctx, &mut app);
                        assert!(need <= 0.5, "{lang}, fill {k}, transparency {kind}: {need}");
                        assert!((w - empty).abs() < 1.0, "{lang}, fill {k}: {w}");
                    }
                    // Every type of the type buttons, vector and bitmap
                    // patterns too (#1439).
                    for ty in crate::ui::transparency_docker::TransparencyType::ALL {
                        let Some(mask) = ty.default_mask() else {
                            continue;
                        };
                        app.transparency_default.mask = mask;
                        let t = app.transparency_default.clone();
                        app.patch_transparency(&t, &[crate::mixed::TransparencyField::Mask]);
                        let (need, w) = overflow(&ctx, &mut app);
                        assert!(need <= 0.5, "{lang}, fill {k}, {ty:?}: {need}");
                        assert!((w - empty).abs() < 1.0, "{lang}, fill {k}, {ty:?}: {w}");
                    }
                }
            });
        }
    }

    /// #2200: a click on a section's button at the top shows that
    /// section alone, a second click on it all of them again; with Ctrl
    /// or Shift the section opens or closes beside the others. The
    /// sections of other kinds of object are closed too, so text selected
    /// next still shows the section picked.
    #[test]
    fn a_section_button_shows_its_section_alone() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        // What a rectangle shows.
        let shown = [
            "outline",
            "fill",
            "transparency",
            "corners",
            "effects",
            "summary",
        ];
        let open = |app: &App| -> Vec<&'static str> {
            PROPERTIES_SECTIONS
                .into_iter()
                .filter(|s| !app.settings.properties_closed.iter().any(|c| c == s))
                .collect()
        };
        assert_eq!(open(&app).len(), PROPERTIES_SECTIONS.len());
        // Fill alone; the button's tip says so.
        pick_section(&mut app, "fill", &shown, false);
        assert_eq!(open(&app), ["fill"]);
        crate::ui::tips::start_recording();
        draw(&ctx, &mut app);
        let tips = crate::ui::tips::recorded();
        assert!(
            tips.contains(&crate::ui::tips::pair(
                "docker.fill",
                "docker.section_pick_hint"
            )),
            "{tips:?}"
        );
        // Another button: that section alone.
        pick_section(&mut app, "summary", &shown, false);
        assert_eq!(open(&app), ["summary"]);
        // With Ctrl or Shift: beside the others, and closed again.
        pick_section(&mut app, "outline", &shown, true);
        assert_eq!(open(&app), ["outline", "summary"]);
        pick_section(&mut app, "outline", &shown, true);
        assert_eq!(open(&app), ["summary"]);
        // The button of the only open section: all of them again.
        pick_section(&mut app, "summary", &shown, false);
        assert_eq!(open(&app).len(), PROPERTIES_SECTIONS.len());
        assert!(app.settings.properties_closed.is_empty());
        // A section the selection does not have, open from before, does
        // not count: Fill is the only one open of those shown.
        app.settings.properties_closed = shown
            .iter()
            .filter(|s| **s != "fill")
            .map(|s| s.to_string())
            .collect();
        assert!(open(&app).contains(&"character"));
        pick_section(&mut app, "fill", &shown, false);
        assert!(app.settings.properties_closed.is_empty());
        // The headers still open and close one section.
        toggle_section(&mut app, "fill");
        assert_eq!(app.settings.properties_closed, ["fill"]);
        draw(&ctx, &mut app);

        // The buttons themselves: a click, then a click with Ctrl held.
        app.settings.properties_closed.clear();
        draw(&ctx, &mut app);
        let click = |app: &mut App, id: &str, modifiers: egui::Modifiers| {
            let rect: egui::Rect = ctx
                .data(|d| d.get_temp(section_button_id(id)))
                .expect("the button is drawn");
            let at = rect.center();
            for pressed in [true, false] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
                out.textures_delta.clear();
            }
            draw(&ctx, app);
        };
        click(&mut app, "transparency", egui::Modifiers::NONE);
        assert_eq!(open(&app), ["transparency"], "a click");
        click(&mut app, "fill", egui::Modifiers::COMMAND);
        assert_eq!(open(&app), ["fill", "transparency"], "a click with Ctrl");
        click(&mut app, "fill", egui::Modifiers::NONE);
        assert_eq!(open(&app), ["fill"]);
        click(&mut app, "fill", egui::Modifiers::NONE);
        assert!(app.settings.properties_closed.is_empty(), "all again");
    }

    /// The Properties sections come open; a button or a header closes and
    /// opens one (kept in the preferences), and a jump to the fill or the
    /// outline opens it again.
    #[test]
    fn properties_sections_open_close_and_jump() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.settings.autocorrect.enabled = false;
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        app.show_dockers = true;
        app.docker_tab = DockerTab::Properties;
        assert!(app.settings.properties_closed.is_empty());
        draw(&ctx, &mut app);
        toggle_section(&mut app, "fill");
        toggle_section(&mut app, "transparency");
        assert_eq!(app.settings.properties_closed, ["fill", "transparency"]);
        draw(&ctx, &mut app);
        toggle_section(&mut app, "transparency");
        assert_eq!(app.settings.properties_closed, ["fill"]);
        // F11 (the fill) opens the fill section again.
        app.properties_open = Some(0);
        draw(&ctx, &mut app);
        assert!(app.settings.properties_closed.is_empty());
        assert!(app.properties_open.is_none());
        // Further fills and outlines show with their editors.
        set_extra_paint(
            &mut app,
            vec![Fill::Solid(tracedraw_core::Color::BLACK)],
            vec![tracedraw_core::Stroke::hairline(
                tracedraw_core::Color::WHITE,
            )],
        );
        let s = app.doc().find_shape(id).expect("shape").clone();
        assert_eq!((s.extra_fills.len(), s.extra_strokes.len()), (1, 1));
        draw(&ctx, &mut app);
        app.undo();
        assert!(app
            .doc()
            .find_shape(id)
            .expect("shape")
            .extra_fills
            .is_empty());
        // Every outline paint and the gradient options draw in the docker.
        let mut fountain = Fountain::two(
            FountainKind::Shaped,
            tracedraw_core::Color::BLACK,
            tracedraw_core::Color::WHITE,
            0.0,
        );
        fountain.repeat = FountainRepeat::Mirror;
        fountain.repeats = 3;
        fountain.bilinear = true;
        fountain.invert = true;
        app.apply_fill(Fill::Fountain(fountain.clone()));
        for paint in [
            Fill::Fountain(fountain),
            Fill::Texture(Texture {
                kind: TextureKind::Marble,
                color_a: tracedraw_core::Color::BLACK,
                color_b: tracedraw_core::Color::WHITE,
                scale: 5.0,
                seed: 1,
            }),
            Fill::Mesh(tracedraw_core::Mesh::new(
                tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                2,
                2,
                tracedraw_core::Color::BLACK,
            )),
        ] {
            let mut stroke = tracedraw_core::Stroke::new(tracedraw_core::Color::BLACK, 2.0);
            stroke.fill = Some(Box::new(paint));
            app.run(Command::SetStroke {
                shapes: vec![id],
                stroke: Some(stroke),
            });
            draw(&ctx, &mut app);
            let s = app.doc().find_shape(id).expect("shape");
            assert!(s.stroke.as_ref().and_then(|st| st.fill.as_ref()).is_some());
        }
    }

    /// A docker whose contents are longer than the docker: a finger
    /// dragged up them scrolls them, as egui's scroll areas do on a touch
    /// screen (#1997).
    #[test]
    fn a_finger_dragged_up_a_docker_scrolls_its_contents() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: 0.0,
                corners: None,
            })
            .expect("shape");
        app.select(vec![id]);
        let scroll_id = std::cell::Cell::new(egui::Id::NULL);
        let clock = std::cell::Cell::new(0.0);
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            clock.set(clock.get() + 0.02);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240.0, 160.0),
                )),
                events,
                time: Some(clock.get()),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                // The id the scroll area gives itself from its salt.
                scroll_id.set(ui.make_persistent_id(egui::IdSalt::new((
                    "docker_scroll",
                    DockerTab::Properties.key(),
                ))));
                docker_page(app, ui, DockerTab::Properties);
            });
            out.textures_delta.clear();
        };

        let touch = |phase, pos| egui::Event::Touch {
            device_id: egui::TouchDeviceId(1),
            id: egui::TouchId(2),
            phase,
            pos,
            force: None,
        };
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        // A finger (or, without `finger`, the mouse) down at `from`, moved
        // to `to` in `steps`; `lift` lets go there, else it stays down.
        let drag = |app: &mut App,
                    from: egui::Pos2,
                    to: egui::Pos2,
                    steps: usize,
                    finger: bool,
                    lift: bool| {
            let mut events = vec![egui::Event::PointerMoved(from), press(from, true)];
            if finger {
                events.insert(0, touch(egui::TouchPhase::Start, from));
            }
            frame(app, events);
            for k in 1..=steps {
                let p = from + (to - from) * (k as f32 / steps as f32);
                let mut events = vec![egui::Event::PointerMoved(p)];
                if finger {
                    events.insert(0, touch(egui::TouchPhase::Move, p));
                }
                frame(app, events);
            }
            if lift {
                let mut events = vec![press(to, false), egui::Event::PointerGone];
                if finger {
                    events.insert(0, touch(egui::TouchPhase::End, to));
                }
                frame(app, events);
                frame(app, Vec::new());
            }
        };

        let offset = || {
            egui::scroll_area::State::load(&ctx, scroll_id.get())
                .map(|s| s.offset.y)
                .unwrap_or(0.0)
        };
        for _ in 0..3 {
            frame(&mut app, Vec::new());
        }
        assert_eq!(offset(), 0.0);
        // The screen touched once (beside the docker), so egui knows it
        // is a touch screen, then a finger dragged up the contents.
        let off = egui::pos2(-10.0, -10.0);
        frame(
            &mut app,
            vec![
                touch(egui::TouchPhase::Start, off),
                touch(egui::TouchPhase::End, off),
            ],
        );
        frame(&mut app, Vec::new());
        drag(
            &mut app,
            egui::pos2(200.0, 140.0),
            egui::pos2(200.0, 40.0),
            10,
            true,
            true,
        );
        // 100 px of finger: egui's drag-to-scroll has no tap distance.
        assert!(offset() > 50.0, "the contents scrolled up: {}", offset());
    }

    /// #2599: the area of one closed object, holes out; none for an open
    /// curve or a text.
    #[test]
    fn the_area_of_one_closed_object() {
        use tracedraw_core::geometry::Rect;
        let rect = tracedraw_core::Shape::new(
            tracedraw_core::ShapeId(1),
            ShapeKind::Rect {
                rect: Rect::new(0.0, 0.0, 20.0, 10.0),
                radius: 0.0,
                corners: None,
            },
        );
        let a = selection_area_mm2(&rect).expect("closed");
        assert!((a - 200.0).abs() < 1e-6, "{a}");
        let circle = tracedraw_core::Shape::new(
            tracedraw_core::ShapeId(2),
            ShapeKind::Ellipse {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                arc: None,
            },
        );
        let a = selection_area_mm2(&circle).expect("closed");
        assert!((a - std::f64::consts::PI * 100.0).abs() < 1e-9, "{a}");
        let mut line = tracedraw_core::geometry::BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((10.0, 10.0));
        line.line_to((20.0, 0.0));
        let open = tracedraw_core::Shape::new(
            tracedraw_core::ShapeId(3),
            ShapeKind::Path {
                path: line,
                closed: false,
            },
        );
        assert_eq!(selection_area_mm2(&open), None);
    }
}

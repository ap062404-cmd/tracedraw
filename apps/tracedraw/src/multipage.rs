//! Multipage View (View > Multipage View): every page of the drawing laid
//! out in a grid on the desktop. The current page is edited as always;
//! the others are drawn beside it, and a click on one of them makes it
//! the current page without moving anything on screen.

use crate::app::App;
use crate::view::View;
use egui::{Color32, Painter, Pos2, Rect as ERect};
use tracedraw_core::document::{Guide, GuideLine};
use tracedraw_core::geometry::{Rect, Size};
use tracedraw_core::spreads::{self, PageSide};
use tracedraw_core::PageId;

/// The spacings Page > Page Spacing offers, in millimetres; Custom...
/// takes any other.
pub const SPACING_PRESETS: [(f64, &str); 6] = [
    (0.0, "menu.layout.page_spacing_0"),
    (5.0, "menu.layout.page_spacing_5"),
    (10.0, "menu.layout.page_spacing_10"),
    (20.0, "menu.layout.page_spacing_20"),
    (30.0, "menu.layout.page_spacing_30"),
    (50.0, "menu.layout.page_spacing_50"),
];

/// Columns of the grid: always this many (fewer pages fill one row), so
/// a page added goes at the next place and the pages before it stay
/// where they were (#1018).
pub const COLUMNS: usize = 4;

/// The pages' places in the current page's coordinates (mm, Y up): a grid
/// of [`COLUMNS`] columns, left to right then down, each page at the top
/// left of its cell, cells `gap` mm apart (a negative or non-finite gap
/// counts as none).
/// The current page is at the origin; an unknown `active` keeps the grid
/// where the first page is at the origin.
pub fn layout(pages: &[(PageId, Size)], active: PageId, gap: f64) -> Vec<(PageId, Rect)> {
    let gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
    let n = pages.len();
    if n == 0 {
        return Vec::new();
    }
    let cols = n.clamp(1, COLUMNS);
    let rows = n.div_ceil(cols);
    let mut col_w = vec![0.0f64; cols];
    let mut row_h = vec![0.0f64; rows];
    for (i, (_, size)) in pages.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        if let Some(w) = col_w.get_mut(c) {
            *w = w.max(size.width.max(0.0));
        }
        if let Some(h) = row_h.get_mut(r) {
            *h = h.max(size.height.max(0.0));
        }
    }
    let mut placed: Vec<(PageId, Rect)> = Vec::with_capacity(n);
    for (i, (id, size)) in pages.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let x0: f64 = col_w.iter().take(c).sum::<f64>() + c as f64 * gap;
        let top: f64 = -(row_h.iter().take(r).sum::<f64>() + r as f64 * gap);
        let w = size.width.max(0.0);
        let h = size.height.max(0.0);
        placed.push((*id, Rect::new(x0, top - h, x0 + w, top)));
    }
    let (dx, dy) = placed
        .iter()
        .find(|(id, _)| *id == active)
        .or(placed.first())
        .map(|(_, r)| (r.x0, r.y0))
        .unwrap_or((0.0, 0.0));
    placed
        .into_iter()
        .map(|(id, r)| (id, Rect::new(r.x0 - dx, r.y0 - dy, r.x1 - dx, r.y1 - dy)))
        .collect()
}

/// Spreads per row of the Multipage View with facing pages: two, so the
/// grid is as wide as the four columns of single pages.
pub const SPREAD_COLUMNS: usize = 2;

/// The pages' places with facing pages (#1972, #1660), in the current
/// page's coordinates (mm, Y up): the spreads (see
/// [`tracedraw_core::spreads`]) in a grid of [`SPREAD_COLUMNS`], left to
/// right then down, `gap` mm apart; the two pages of a spread touch at
/// the spine. Every slot is as wide as the widest page, so a page added
/// never moves the pages before it; a left page ends at the spine, a
/// right page starts there, tops lined up in each row.
pub fn facing_layout(
    pages: &[(PageId, Size)],
    active: PageId,
    gap: f64,
    start: PageSide,
) -> Vec<(PageId, Rect)> {
    let all: Vec<usize> = (0..pages.len()).collect();
    facing_layout_of(pages, &all, active, gap, start)
}

/// [`facing_layout`] of the pages at `keep` (indices into `pages`, in
/// order) only, as the Multipage View shows a range of pages (#1440):
/// each page keeps its side of the spine, and the spreads they are in
/// follow each other with no room left for the spreads not shown.
pub fn facing_layout_of(
    pages: &[(PageId, Size)],
    keep: &[usize],
    active: PageId,
    gap: f64,
    start: PageSide,
) -> Vec<(PageId, Rect)> {
    let gap = if gap.is_finite() { gap.max(0.0) } else { 0.0 };
    let kept: Vec<(usize, PageId, Size)> = keep
        .iter()
        .filter_map(|&i| pages.get(i).map(|(id, size)| (i, *id, *size)))
        .collect();
    let n = kept.len();
    if n == 0 {
        return Vec::new();
    }
    let slot = kept
        .iter()
        .map(|(_, _, s)| s.width.max(0.0))
        .fold(0.0f64, f64::max);
    // The spreads shown, numbered from 0 in order.
    let mut shown: Vec<usize> = kept
        .iter()
        .map(|(i, _, _)| spreads::spread_number(*i, start))
        .collect();
    shown.dedup();
    let spread_of = |i: usize| {
        let k = spreads::spread_number(i, start);
        shown.iter().position(|s| *s == k).unwrap_or(0)
    };
    let row_of = |i: usize| spread_of(i) / SPREAD_COLUMNS;
    let rows = kept.last().map(|(i, _, _)| row_of(*i)).unwrap_or(0) + 1;
    let mut row_h = vec![0.0f64; rows];
    for (i, _, size) in &kept {
        if let Some(h) = row_h.get_mut(row_of(*i)) {
            *h = h.max(size.height.max(0.0));
        }
    }
    let mut placed: Vec<(PageId, Rect)> = Vec::with_capacity(n);
    for &(i, id, size) in &kept {
        let k = spread_of(i);
        let (r, c) = (k / SPREAD_COLUMNS, k % SPREAD_COLUMNS);
        let spine = c as f64 * (2.0 * slot + gap) + slot;
        let top: f64 = -(row_h.iter().take(r).sum::<f64>() + r as f64 * gap);
        let w = size.width.max(0.0);
        let h = size.height.max(0.0);
        let x0 = match spreads::side_of(i, start) {
            PageSide::Left => spine - w,
            PageSide::Right => spine,
        };
        placed.push((id, Rect::new(x0, top - h, x0 + w, top)));
    }
    let (dx, dy) = placed
        .iter()
        .find(|(id, _)| *id == active)
        .or(placed.first())
        .map(|(_, r)| (r.x0, r.y0))
        .unwrap_or((0.0, 0.0));
    placed
        .into_iter()
        .map(|(id, r)| (id, Rect::new(r.x0 - dx, r.y0 - dy, r.x1 - dx, r.y1 - dy)))
        .collect()
}

/// The page under a page-space point, if any.
pub fn page_at(placed: &[(PageId, Rect)], p: tracedraw_core::geometry::Point) -> Option<PageId> {
    placed
        .iter()
        .find(|(_, r)| p.x >= r.x0 && p.x <= r.x1 && p.y >= r.y0 && p.y <= r.y1)
        .map(|(id, _)| *id)
}

/// Everything the grid covers, for Zoom to Fit in this view.
pub fn bounds(placed: &[(PageId, Rect)]) -> Option<Rect> {
    let mut it = placed.iter().map(|(_, r)| *r);
    let first = it.next()?;
    Some(it.fold(first, |a, b| a.union(b)))
}

impl App {
    /// The pages the Multipage View shows (indices, in page order): the
    /// drawing's range of pages (#1440), or every page, and the current
    /// page always, so it can be edited.
    pub fn multipage_page_indices(&self) -> Vec<usize> {
        let doc = self.doc();
        let n = doc.pages.len();
        match doc.metadata.multipage_page_indices(n) {
            Some(mut keep) => {
                let current = self.page_index();
                if current < n && !keep.contains(&current) {
                    keep.push(current);
                    keep.sort_unstable();
                }
                keep
            }
            None => (0..n).collect(),
        }
    }

    /// The grid of pages around the current one (in spreads with facing
    /// pages): the pages the Multipage View shows.
    pub fn multipage_layout(&self) -> Vec<(PageId, Rect)> {
        self.pages_layout(&self.multipage_page_indices())
    }

    /// The grid of the pages at `keep` (indices in page order).
    fn pages_layout(&self, keep: &[usize]) -> Vec<(PageId, Rect)> {
        let doc = self.doc();
        let pages: Vec<(PageId, Size)> = doc.pages.iter().map(|p| (p.id, p.size)).collect();
        let gap = doc.metadata.page_spacing();
        match doc.facing() {
            Some(start) => facing_layout_of(&pages, keep, self.page, gap, start),
            None => {
                let kept: Vec<(PageId, Size)> =
                    keep.iter().filter_map(|&i| pages.get(i).copied()).collect();
                layout(&kept, self.page, gap)
            }
        }
    }

    /// The pages on the desktop and where, in the current page's
    /// coordinates: every page in the Multipage View, else the current
    /// page's spread with facing pages (#1972), else the current page.
    pub fn shown_pages(&self) -> Vec<(PageId, Rect)> {
        if self.multipage {
            return self.multipage_layout();
        }
        let doc = self.doc();
        if doc.facing().is_some() {
            let spread = doc.spread_at(self.page_index());
            let ids: Vec<PageId> = spread
                .pages()
                .filter_map(|i| doc.pages.get(i).map(|p| p.id))
                .collect();
            if ids.len() > 1 {
                let all: Vec<usize> = (0..doc.pages.len()).collect();
                return self
                    .pages_layout(&all)
                    .into_iter()
                    .filter(|(id, _)| ids.contains(id))
                    .collect();
            }
        }
        vec![(self.page, self.page_rect())]
    }

    /// Pages other than the current one are on the desktop: the
    /// Multipage View, or the current page's spread with facing pages.
    pub fn shows_other_pages(&self) -> bool {
        self.multipage || self.shown_pages().len() > 1
    }

    /// The box of the spread page `page` is in, where it is shown (the
    /// page's own box without facing pages); `None` when the page is not
    /// shown.
    pub fn shown_spread_rect(&self, page: PageId) -> Option<Rect> {
        let shown = self.shown_pages();
        let doc = self.doc();
        let index = doc.pages.iter().position(|p| p.id == page)?;
        let spread = doc.spread_at(index);
        let ids: Vec<PageId> = spread
            .pages()
            .filter_map(|i| doc.pages.get(i).map(|p| p.id))
            .collect();
        bounds(
            &shown
                .into_iter()
                .filter(|(id, _)| ids.contains(id) || *id == page)
                .collect::<Vec<_>>(),
        )
    }

    /// Layout > Facing Pages and Document Options > Layout: show the
    /// pages in spreads (`on`), the first page on `start`, as one undo
    /// step (a document property saved with the drawing). The view fits
    /// the spread anew. False when nothing changed.
    pub fn set_facing_pages(&mut self, on: bool, start: PageSide) -> bool {
        if !self.has_document() {
            return false;
        }
        let m = &self.doc().metadata;
        if m.facing_pages == on && m.facing_start == start {
            return false;
        }
        let mut metadata = m.clone();
        metadata.facing_pages = on;
        metadata.facing_start = start;
        self.run(tracedraw_core::Command::SetMetadata { metadata });
        self.page_rasters.borrow_mut().clear();
        self.fit_pending = true;
        true
    }

    /// The space between the pages of the Multipage View, mm.
    pub fn page_spacing(&self) -> f64 {
        self.doc().metadata.page_spacing()
    }

    /// Page > Page Spacing and Document Setup > Layout: set the space
    /// between the pages of the Multipage View, in mm, as one undo step
    /// (a document property, saved with the drawing). The current page
    /// stays where it is on screen. False when nothing changed.
    pub fn set_page_spacing(&mut self, mm: f64) -> bool {
        if !self.has_document() || !mm.is_finite() {
            return false;
        }
        let mm = mm.clamp(0.0, tracedraw_core::document::MAX_PAGE_SPACING);
        if (mm - self.page_spacing()).abs() < 1e-9 {
            return false;
        }
        let mut metadata = self.doc().metadata.clone();
        metadata.page_spacing = Some(mm);
        self.run(tracedraw_core::Command::SetMetadata { metadata });
        true
    }

    /// View > Pages in Multipage View and the page bar's button next to
    /// the Multipage View's (#1440): the pages the view shows, as a range
    /// of page numbers ("1-3, 7"), empty for every page. Saved with the
    /// drawing like the page spacing, one undo step; the view fits the
    /// pages anew. False when nothing changed or the range names no page
    /// of the drawing.
    pub fn set_multipage_pages(&mut self, range: &str) -> bool {
        if !self.has_document() {
            return false;
        }
        let range = range.trim();
        let count = self.doc().pages.len();
        if !range.is_empty() && tracedraw_core::document::parse_page_range(range, count).is_empty()
        {
            return false;
        }
        if self.doc().metadata.multipage_pages == range {
            return false;
        }
        let mut metadata = self.doc().metadata.clone();
        metadata.multipage_pages = range.to_string();
        self.run(tracedraw_core::Command::SetMetadata { metadata });
        // The current page among the chosen ones, so they show together.
        if let Some(chosen) = self.doc().metadata.multipage_page_indices(count) {
            if !chosen.contains(&self.page_index()) {
                if let Some(id) = chosen
                    .first()
                    .and_then(|i| self.doc().pages.get(*i))
                    .map(|p| p.id)
                {
                    self.page = id;
                    self.selection.clear();
                }
            }
        }
        self.page_rasters.borrow_mut().clear();
        if self.multipage {
            self.fit_pending = true;
        }
        true
    }

    /// The dialog choosing the Multipage View's pages, filled in with the
    /// drawing's range.
    pub fn multipage_pages_dialog(&self) -> crate::ui::dialogs::Dialog {
        let range = self.doc().metadata.multipage_pages.clone();
        crate::ui::dialogs::Dialog::MultipagePages {
            all: range.is_empty(),
            range,
        }
    }

    /// Turn the Multipage View on or off; turning it on shows the pages
    /// chosen for it (every page at first).
    pub fn set_multipage(&mut self, on: bool) {
        self.multipage = on;
        if !on {
            self.page_rasters.borrow_mut().clear();
            self.page_snapshot.borrow_mut().take();
        }
        if on {
            self.page_sorter = false;
        }
        self.fit_pending = true;
    }

    /// Make `page` the current page and keep every page where it is on
    /// screen: the view moves by the page's place in the grid.
    pub fn multipage_activate(&mut self, page: PageId) {
        if page == self.page {
            return;
        }
        let Some(r) = self
            .shown_pages()
            .into_iter()
            .find(|(id, _)| *id == page)
            .map(|(_, r)| r)
        else {
            // A page left out of the Multipage View's range (#1440): it
            // shows while it is current; the page that was current stays
            // where it is on screen if it still shows, else the view fits.
            if self.multipage && self.doc().pages.iter().any(|p| p.id == page) {
                let before = self.page;
                self.page = page;
                match self.shown_pages().into_iter().find(|(id, _)| *id == before) {
                    Some((_, old)) => {
                        let z = self.view.zoom;
                        self.view.origin -= egui::vec2(old.x0 as f32 * z, -(old.y0 as f32) * z);
                    }
                    None => self.fit_pending = true,
                }
                self.selection.clear();
                self.focus.clear();
                self.picked_layer = None;
            }
            return;
        };
        self.view.origin = self
            .view
            .to_screen(tracedraw_core::geometry::Point::new(r.x0, r.y0));
        self.page = page;
        self.selection.clear();
        self.focus.clear();
        self.picked_layer = None;
    }

    /// In the Multipage View and a spread of facing pages, a press on
    /// another page makes it current before the tool sees the press.
    /// True when the page changed. With facing pages an object reaching
    /// over the spine is picked where it lies (#1972): a press on an
    /// object of the current page keeps the page, and a press on the
    /// current page where only an object of another page of the spread
    /// lies makes that page current.
    pub fn multipage_press(&mut self, s: Pos2) -> bool {
        if !self.shows_other_pages() {
            return false;
        }
        let p = self.view.to_page(s);
        let shown = self.shown_pages();
        let facing = self.doc().facing().is_some();
        let under = page_at(&shown, p);
        if facing && self.hit_test(p).is_some() {
            return false;
        }
        let target = if facing {
            match under {
                Some(id) if id != self.page => Some(id),
                _ => self.spread_object_page_at(&shown, p).or_else(|| {
                    self.multipage
                        .then(|| self.shown_object_page_at(&shown, p))
                        .flatten()
                }),
            }
        } else {
            // What shows at the press is picked where it is, whatever
            // page holds it (#2055). The current page's objects are drawn
            // over the other pages', so one of them there keeps the page,
            // even over another page's paper or off every page; else an
            // object of another page there (one left on the desktop
            // beside its page, or reaching over the current page) makes
            // its page current, so the press then picks it; else another
            // page's paper makes that page current.
            if self.hit_test(p).is_some() {
                return false;
            }
            self.shown_object_page_at(&shown, p)
                .or(under.filter(|id| *id != self.page))
        };
        match target {
            Some(id) => {
                self.multipage_activate(id);
                true
            }
            None => false,
        }
    }

    /// Another shown page with a visible object that the press at `p`
    /// (the current page's coordinates) picks, on its page or off it: the
    /// object itself is hit, as the current page's objects are, not its
    /// box, so a press in the empty part of a wide object's box over
    /// another page goes to that page (#2055). The pages in reverse
    /// drawing order, so the frontmost page's object wins.
    fn shown_object_page_at(
        &self,
        shown: &[(PageId, Rect)],
        p: tracedraw_core::geometry::Point,
    ) -> Option<PageId> {
        let doc = self.doc();
        let tol = self.object_pick_tol();
        shown
            .iter()
            .rev()
            .filter(|(id, _)| *id != self.page)
            .filter_map(|(id, r)| doc.page(*id).ok().map(|page| (page, *r)))
            .find(|(page, r)| {
                let q = tracedraw_core::geometry::Point::new(p.x - r.x0, p.y - r.y0);
                page.layers
                    .iter()
                    .filter(|l| l.visible && !l.locked)
                    .flat_map(|l| l.shapes.iter())
                    .filter(|sh| sh.visible && !sh.locked)
                    .any(|sh| {
                        let b = sh.visual_bounds().inflate(tol, tol);
                        q.x >= b.x0
                            && q.x <= b.x1
                            && q.y >= b.y0
                            && q.y <= b.y1
                            && self.shape_hit(sh, q, tol)
                    })
            })
            .map(|(page, _)| page.id)
    }

    /// In the Multipage View, the page (its index and its place in the
    /// grid) that a move of the selection by `total`, let go, gives the
    /// selection to: another shown page whose paper holds the middle of
    /// the moved selection's box (#2055). None off every paper, over the
    /// current page, with facing pages (a spread is edited as one), and
    /// when an object selected is inside a group or on a master layer.
    pub fn multipage_drop_page(
        &self,
        total: tracedraw_core::geometry::Vec2,
    ) -> Option<(usize, Rect)> {
        if !self.multipage || self.doc().facing().is_some() || self.selection.is_empty() {
            return None;
        }
        let middle = self.selection_bounds()?.center() + total;
        let shown = self.shown_pages();
        let id = page_at(&shown, middle).filter(|id| *id != self.page)?;
        let (_, r) = shown.iter().find(|(p, _)| *p == id)?;
        let doc = self.doc();
        let own = doc.page(self.page).ok()?;
        let on_layers = self.selection.iter().all(|sel| {
            own.layers
                .iter()
                .any(|l| l.shapes.iter().any(|s| s.id == *sel))
        });
        if !on_layers {
            return None;
        }
        let idx = doc.pages.iter().position(|p| p.id == id)?;
        Some((idx, *r))
    }

    /// The selection given to page `idx`, which sits at `r` in the
    /// Multipage View's grid: moved into that page's own coordinates, so
    /// it stays where it shows, and onto its layer objects sent there go
    /// to; that page becomes current, nothing moving on screen, with the
    /// objects selected (#2055). False when nothing went.
    pub fn give_selection_to_shown_page(&mut self, idx: usize, r: Rect) -> bool {
        let Some(layer) = self.page_drop_layer(idx) else {
            self.status = crate::i18n::tr("shelf.no_layer");
            return false;
        };
        self.transform_selection(tracedraw_core::geometry::Affine::translate((-r.x0, -r.y0)));
        let ids = self.selection.clone();
        crate::ui::objects_docker::move_to_layer(self, &ids, layer);
        true
    }

    /// What the visible objects on the visible layers of `page` cover, in
    /// the page's own coordinates, with their outlines and effects; `None`
    /// for a page with nothing to show. The Multipage View draws another
    /// page's objects over this and the paper, so the ones it holds off
    /// its page show too (#2055).
    pub fn page_content_bounds(&self, page: PageId) -> Option<Rect> {
        let doc = self.doc();
        let page = doc.page(page).ok()?;
        page.layers
            .iter()
            .filter(|l| l.visible)
            .flat_map(|l| l.shapes.iter())
            .filter(|s| s.visible)
            .map(|s| s.visual_bounds())
            .filter(|b| b.is_finite() && b.width() >= 0.0 && b.height() >= 0.0)
            .reduce(|a, b| a.union(b))
    }

    /// Another page of the current spread whose visible object lies at
    /// `p` (the current page's coordinates), going by the objects' boxes.
    fn spread_object_page_at(
        &self,
        shown: &[(PageId, Rect)],
        p: tracedraw_core::geometry::Point,
    ) -> Option<PageId> {
        let doc = self.doc();
        let spread = doc.spread_at(self.page_index());
        spread
            .pages()
            .filter_map(|i| doc.pages.get(i))
            .filter(|page| page.id != self.page)
            .find(|page| {
                let Some((_, r)) = shown.iter().find(|(id, _)| *id == page.id) else {
                    return false;
                };
                let q = tracedraw_core::geometry::Point::new(p.x - r.x0, p.y - r.y0);
                page.layers
                    .iter()
                    .filter(|l| l.visible && !l.locked)
                    .flat_map(|l| l.shapes.iter())
                    .filter(|sh| sh.visible)
                    .any(|sh| {
                        let b = sh.visual_bounds();
                        q.x >= b.x0 && q.x <= b.x1 && q.y >= b.y0 && q.y <= b.y1
                    })
            })
            .map(|page| page.id)
    }

    /// The guidelines of the other pages in the Multipage View, each moved
    /// to where its page sits in the grid (the current page's
    /// coordinates) and given the page's box there, so they can be drawn
    /// within their page (#1448); the facing page's in a spread of facing
    /// pages too (#1972). None when no other page is shown; the current
    /// page's guidelines are drawn as always.
    pub fn other_page_guides(&self) -> Vec<(Guide, Rect)> {
        if !self.shows_other_pages() {
            return Vec::new();
        }
        let doc = self.doc();
        let mut out = Vec::new();
        for (id, r) in self.shown_pages() {
            if id == self.page {
                continue;
            }
            let Some(page) = doc.pages.iter().find(|p| p.id == id) else {
                continue;
            };
            // The page's own guidelines and the master page's (#1928),
            // those their eye shows (#1775).
            for g in doc.guides_on_page(page.id).unwrap_or_default().iter() {
                let mut g = *g;
                g.line = shifted(g.line, r.x0, r.y0);
                out.push((g, r));
            }
        }
        out
    }

    /// What Zoom to Fit and a new view fit: the page, or every page in the
    /// Multipage View.
    pub fn fit_rect(&self) -> Rect {
        if self.shows_other_pages() {
            if let Some(b) = bounds(&self.shown_pages()) {
                return b;
            }
        }
        self.page_rect()
    }
}

/// A guideline moved by `(dx, dy)`: a page's guideline where the page
/// sits in the grid.
fn shifted(line: GuideLine, dx: f64, dy: f64) -> GuideLine {
    match line {
        GuideLine::Horizontal { y } => GuideLine::Horizontal { y: y + dy },
        GuideLine::Vertical { x } => GuideLine::Vertical { x: x + dx },
        GuideLine::Angled { x, y, angle } => GuideLine::Angled {
            x: x + dx,
            y: y + dy,
            angle,
        },
    }
}

/// How long one frame may spend rendering the page and the pages beside
/// it, seconds; the pages left wait for the next frames, showing their
/// last picture meanwhile (#903). One render a frame always runs.
pub const RENDER_BUDGET: f64 = 0.010;

/// The part of a page to render, on screen: the whole paper while it
/// covers no more of the screen than the canvas does, so the picture
/// stays whole while the view moves; else only its visible part.
pub fn render_rect(paper: ERect, canvas: ERect) -> ERect {
    let clip = paper.intersect(canvas);
    let area = |r: ERect| r.width().max(0.0) * r.height().max(0.0);
    if area(paper) <= area(canvas) {
        paper
    } else {
        clip
    }
}

/// The pages on the desktop, on screen (rounded to pixels), with their
/// index in the drawing: those whose paper shows in `rect`, or all of
/// them (`rect` `None`).
fn other_papers(app: &App, rect: Option<ERect>) -> Vec<(usize, PageId, Rect, ERect)> {
    let doc = app.doc();
    app.shown_pages()
        .into_iter()
        .filter_map(|(id, r)| {
            let index = doc.pages.iter().position(|p| p.id == id)?;
            let paper = app.view.rect_to_screen(r);
            let paper = ERect::from_min_max(paper.min.round(), paper.max.round());
            rect.is_none_or(|rect| paper.expand(30.0).intersects(rect))
                .then_some((index, id, r, paper))
        })
        .collect()
}

/// Paints the shadows and the paper of the pages other than the current
/// one (the Multipage View, a spread of facing pages): all the shadows
/// first, then the papers, so no shadow falls on a page beside it.
/// Painted before the current page; their objects come after it (see
/// [`draw_other_pages`]), so an object of theirs that reaches over the
/// spine shows on the current page too (#1972).
pub fn draw_other_papers(app: &App, painter: &Painter, rect: ERect) {
    let papers: Vec<ERect> = other_papers(app, Some(rect))
        .into_iter()
        .filter(|(_, id, _, _)| *id != app.page)
        .map(|(_, _, _, paper)| paper)
        .collect();
    if app.show_page_shadow {
        for paper in &papers {
            painter.rect_filled(
                paper
                    .expand(1.0)
                    .translate(crate::theme::Tokens::PAGE_SHADOW_OFFSET),
                0.0,
                crate::theme::Tokens::page_shadow(),
            );
        }
    }
    for paper in &papers {
        painter.rect_filled(*paper, 0.0, crate::theme::Tokens::PAGE);
    }
}

/// Paints the objects of the pages other than the current one (each
/// page kept as its own texture), their borders and the page names
/// below. While the view moves each page shows its last picture scaled
/// into place, and the pages render anew once it rests, as many a frame
/// as [`RENDER_BUDGET`] allows, counted from `started` (before the
/// current page's render; `rendered` when that one rendered) (#902,
/// #903). With facing pages a page renders over its whole spread, so
/// what it holds across the spine shows (#1972).
pub fn draw_other_pages(
    app: &App,
    painter: &Painter,
    rect: ERect,
    started: web_time::Instant,
    rendered: bool,
) {
    // Every page: one whose paper is off the screen may hold objects off
    // it that show (#2055).
    let placed = other_papers(app, None);
    let doc = app.doc();
    let ctx = painter.ctx();
    let facing = doc.facing().is_some();
    let mut rasters = app.page_rasters.borrow_mut();
    rasters.retain(|id, _| doc.pages.iter().any(|p| p.id == *id));
    let mut rendered = rendered;
    for (index, id, _, paper) in placed.iter().copied() {
        if id != app.page {
            // The part of the desktop the page's objects show on: the
            // page, or its spread.
            let area = if facing {
                app.shown_spread_rect(id)
                    .map(|r| {
                        let r = app.view.rect_to_screen(r);
                        ERect::from_min_max(r.min.round(), r.max.round())
                    })
                    .unwrap_or(paper)
            } else {
                paper
            };
            let view = View {
                zoom: app.view.zoom,
                origin: paper.left_bottom(),
            };
            let raster = rasters.entry(id).or_default();
            // With the objects the page holds off its paper, which showed
            // only while the page was current, so they seemed to vanish
            // when another page became current (#2055). Measured once a
            // change of the drawing.
            let revision = app.engine.revision();
            let content = match raster.content {
                Some((r, c)) if r == revision => c,
                _ => {
                    let c = app.page_content_bounds(id);
                    raster.content = Some((revision, c));
                    c
                }
            };
            let area = match content {
                Some(b) => {
                    let r = view.rect_to_screen(b);
                    area.union(ERect::from_min_max(r.min.floor(), r.max.ceil()).expand(1.0))
                }
                None => area,
            };
            let clip = area.intersect(rect);
            if clip.width() >= 1.0 && clip.height() >= 1.0 {
                raster.follows_zoom = true;
                raster.defer = rendered && started.elapsed().as_secs_f64() > RENDER_BUDGET;
                // A render on its thread takes the drawing shared by all
                // the pages, copied once a change (the browser build draws
                // on the one thread).
                if cfg!(not(target_arch = "wasm32")) && !raster.defer && raster.has_picture() {
                    let revision = app.engine.revision();
                    let mut snap = app.page_snapshot.borrow_mut();
                    if snap.as_ref().is_none_or(|(r, _)| *r != revision) {
                        *snap = Some((revision, std::sync::Arc::new(doc.clone())));
                    }
                    raster.snapshot = snap.as_ref().map(|(_, d)| d.clone());
                }
                let before = raster.render_count();
                let shown = raster.texture(
                    ctx,
                    doc,
                    id,
                    &view,
                    render_rect(area, rect),
                    None,
                    app.engine.revision(),
                    app.wireframe,
                    app.simulate_overprints,
                    app.rasterize_complex_effects,
                    app.proof_colors,
                    None,
                    None,
                );
                rendered |= raster.render_count() != before;
                raster.defer = false;
                raster.snapshot = None;
                // The picture where its part of the page is, cut to the
                // page's visible part: an older picture shown while the
                // view moves covers another part of the page than now.
                if let Some((tex, at)) = shown {
                    painter.with_clip_rect(clip).image(
                        tex,
                        at,
                        ERect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
            if app.show_page_border {
                painter.rect_stroke(
                    paper,
                    0.0,
                    egui::Stroke::new(1.0, crate::theme::Tokens::page_border()),
                    egui::StrokeKind::Outside,
                );
            }
        }
        let name = doc
            .pages
            .get(index)
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| crate::i18n::trf("doc.page_n", &[("n", &(index + 1).to_string())]));
        let color = if id == app.page {
            crate::theme::Tokens::text()
        } else {
            crate::theme::Tokens::text_dim()
        };
        painter.text(
            paper.left_bottom() + egui::vec2(0.0, 6.0),
            egui::Align2::LEFT_TOP,
            format!("{}: {name}", index + 1),
            egui::FontId::proportional(12.0),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The space between pages of a drawing that sets none.
    const GAP_MM: f64 = tracedraw_core::document::DEFAULT_PAGE_SPACING;
    use tracedraw_core::geometry::Point;

    fn pages(n: usize) -> Vec<(PageId, Size)> {
        (0..n)
            .map(|i| (PageId(i as u64 + 1), Size::new(100.0, 50.0)))
            .collect()
    }

    #[test]
    fn pages_sit_in_a_grid_with_the_current_one_at_the_origin() {
        let ps = pages(6);
        let placed = layout(&ps, PageId(1), GAP_MM);
        // Four columns, 30 mm apart, then the next row going down.
        assert_eq!(placed[0].1, Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(placed[1].1, Rect::new(130.0, 0.0, 230.0, 50.0));
        assert_eq!(placed[3].1, Rect::new(390.0, 0.0, 490.0, 50.0));
        assert_eq!(placed[4].1, Rect::new(0.0, -80.0, 100.0, -30.0));
        // Another page current: the same grid, moved so it is at the origin.
        let placed = layout(&ps, PageId(6), GAP_MM);
        assert_eq!(placed[5].1, Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(placed[0].1, Rect::new(-130.0, 80.0, -30.0, 130.0));
        assert_eq!(page_at(&placed, Point::new(-50.0, 100.0)), Some(PageId(1)));
        assert_eq!(page_at(&placed, Point::new(-20.0, 100.0)), None);
        assert_eq!(bounds(&placed), Some(Rect::new(-130.0, 0.0, 360.0, 130.0)));
        assert!(layout(&[], PageId(1), GAP_MM).is_empty());
    }

    /// Pages added one by one go at the next place of a grid of four
    /// columns; the pages before never move (#1018). The grid used to have
    /// as many columns as the square root of the page count, so the 4th
    /// and the 10th page rearranged every page.
    #[test]
    fn adding_pages_never_moves_the_pages_before() {
        for n in 1..=13 {
            let before = layout(&pages(n), PageId(1), GAP_MM);
            let after = layout(&pages(n + 1), PageId(1), GAP_MM);
            assert_eq!(after[..n], before[..], "page {} added", n + 1);
            let (r, c) = (n / COLUMNS, n % COLUMNS);
            let x0 = c as f64 * 130.0;
            let y0 = -(r as f64) * 80.0;
            assert_eq!(after[n].1, Rect::new(x0, y0, x0 + 100.0, y0 + 50.0));
        }
    }

    #[test]
    fn pages_of_other_sizes_take_the_widest_and_tallest_cell() {
        let ps = vec![
            (PageId(1), Size::new(100.0, 50.0)),
            (PageId(2), Size::new(40.0, 80.0)),
            (PageId(3), Size::new(150.0, 30.0)),
        ];
        let placed = layout(&ps, PageId(1), GAP_MM);
        // One row: each column as wide as its page, every top lined up
        // with page 1's; the next row (page 5 on) would start below the
        // tallest, page 2.
        assert_eq!(placed[1].1, Rect::new(130.0, -30.0, 170.0, 50.0));
        assert_eq!(placed[2].1, Rect::new(200.0, 20.0, 350.0, 50.0));
        let mut five = ps.clone();
        five.push((PageId(4), Size::new(10.0, 10.0)));
        five.push((PageId(5), Size::new(150.0, 30.0)));
        let placed = layout(&five, PageId(1), GAP_MM);
        // Column 0 is now as wide as page 5, which sits a row below page
        // 2's bottom.
        assert_eq!(placed[1].1, Rect::new(180.0, -30.0, 220.0, 50.0));
        assert_eq!(placed[4].1, Rect::new(0.0, -90.0, 150.0, -60.0));
    }

    #[test]
    fn a_small_page_renders_whole_and_a_large_one_its_visible_part() {
        let canvas = ERect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        // Small and partly off the canvas: the whole paper, so the picture
        // stays whole while the view moves.
        let small = ERect::from_min_size(egui::pos2(-50.0, 100.0), egui::vec2(200.0, 150.0));
        assert_eq!(render_rect(small, canvas), small);
        // Larger than the canvas: only what shows.
        let large = ERect::from_min_size(egui::pos2(-500.0, -500.0), egui::vec2(2000.0, 1500.0));
        assert_eq!(render_rect(large, canvas), canvas);
    }

    /// One frame of the whole window at egui time `time`.
    fn frame_at(ctx: &egui::Context, app: &mut App, time: f64) {
        let input = egui::RawInput {
            screen_rect: Some(ERect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
            time: Some(time),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::root(app, ui));
        out.textures_delta.clear();
    }

    fn renders(app: &App) -> u64 {
        app.page_rasters
            .borrow()
            .values()
            .map(|r| r.render_count())
            .sum()
    }

    /// While the view zooms, the pages beside the current one show their
    /// last pictures scaled into place and render nothing; once the view
    /// rests they render anew (#902, #903).
    #[test]
    fn other_pages_render_once_the_zoom_rests() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        for _ in 0..3 {
            app.add_page();
            app.new_shape(tracedraw_core::ShapeKind::Rect {
                rect: Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            });
        }
        app.page = app.doc().pages[0].id;
        app.set_multipage(true);
        let mut t = 0.0;
        for _ in 0..20 {
            t += 0.02;
            frame_at(&ctx, &mut app, t);
        }
        let at_rest = renders(&app);
        assert!(at_rest >= 3, "every other page rendered: {at_rest}");
        // Still at rest: nothing rendered again.
        t += 0.02;
        frame_at(&ctx, &mut app, t);
        assert_eq!(renders(&app), at_rest);
        // A zoom, frame by frame: nothing rendered.
        for _ in 0..5 {
            app.view.zoom_at(egui::pos2(800.0, 500.0), 0.9);
            t += 0.016;
            frame_at(&ctx, &mut app, t);
        }
        assert_eq!(renders(&app), at_rest, "no render while zooming");
        // At rest: each page rendered again, one render on a thread at a
        // time in the whole program (other tests' renders may hold the
        // turn for a moment).
        for _ in 0..400 {
            t += 0.02;
            frame_at(&ctx, &mut app, t);
            if renders(&app) >= at_rest + 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(renders(&app) >= at_rest + 3);
    }

    /// The other pages' guidelines show in the Multipage View, each
    /// where its page sits in the grid and within that page; they used
    /// to show only while their page was the current one (#1448).
    #[test]
    fn the_other_pages_guidelines_show_where_their_pages_sit() {
        let mut app = App::headless();
        app.add_page();
        let first = app.doc().pages[0].id;
        let second = app.doc().pages[1].id;
        app.page = second;
        // Each page's own guidelines (shared ones are the default, #1775).
        app.choose_master_guides(false);
        app.add_guide(Guide::horizontal(10.0));
        app.add_guide(Guide::vertical(20.0));
        app.add_guide(
            GuideLine::Angled {
                x: 5.0,
                y: 6.0,
                angle: 30.0,
            }
            .into(),
        );
        app.page = first;
        app.add_guide(Guide::horizontal(40.0));
        // Off: nothing beyond the current page's own guidelines.
        app.set_multipage(false);
        assert!(app.other_page_guides().is_empty());
        app.set_multipage(true);
        let r = app
            .multipage_layout()
            .into_iter()
            .find(|(id, _)| *id == second)
            .map(|(_, r)| r)
            .expect("the second page's place");
        assert!(r.x0 > 0.0, "beside the first page: {r:?}");
        let shown = app.other_page_guides();
        assert_eq!(shown.len(), 3, "the second page's three, not the first's");
        assert!(shown.iter().all(|(_, page)| *page == r));
        let lines: Vec<GuideLine> = shown.iter().map(|(g, _)| g.line).collect();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(matches!(lines[0], GuideLine::Horizontal { y } if near(y, 10.0 + r.y0)));
        assert!(matches!(lines[1], GuideLine::Vertical { x } if near(x, 20.0 + r.x0)));
        assert!(matches!(
            lines[2],
            GuideLine::Angled { x, y, angle }
                if near(x, 5.0 + r.x0) && near(y, 6.0 + r.y0) && near(angle, 30.0)
        ));
        // The second page current: the first page's guideline, where
        // that page now sits (to the left of the origin).
        app.multipage_activate(second);
        let shown = app.other_page_guides();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].1.x0 < 0.0);
        assert!(
            matches!(shown[0].0.line, GuideLine::Horizontal { y } if near(y, 40.0 + shown[0].1.y0))
        );
        // A frame draws them within their page without trouble.
        let ctx = crate::theme::ui_context();
        app.show_welcome = false;
        app.show_guides = true;
        frame_at(&ctx, &mut app, 0.1);
        frame_at(&ctx, &mut app, 0.2);
    }

    /// Facing pages starting on the right: page 1 alone on the right of
    /// its spread, then 2-3, 4-5 touching at the spine, two spreads a
    /// row; a page added never moves the pages before it (#1972).
    #[test]
    fn facing_pages_sit_in_spreads_touching_at_the_spine() {
        let ps = pages(6);
        let placed = facing_layout(&ps, PageId(1), GAP_MM, PageSide::Right);
        // Page 1 on the right of the first spread, at the origin.
        assert_eq!(placed[0].1, Rect::new(0.0, 0.0, 100.0, 50.0));
        // Pages 2 and 3 form the second spread, touching.
        assert_eq!(placed[1].1, Rect::new(130.0, 0.0, 230.0, 50.0));
        assert_eq!(placed[2].1, Rect::new(230.0, 0.0, 330.0, 50.0));
        // Pages 4-5 start the next row; page 6 on the left of a spread.
        assert_eq!(placed[3].1, Rect::new(-100.0, -80.0, 0.0, -30.0));
        assert_eq!(placed[4].1, Rect::new(0.0, -80.0, 100.0, -30.0));
        assert_eq!(placed[5].1, Rect::new(130.0, -80.0, 230.0, -30.0));
        for n in 1..=9 {
            let before = facing_layout(&pages(n), PageId(1), GAP_MM, PageSide::Right);
            let after = facing_layout(&pages(n + 1), PageId(1), GAP_MM, PageSide::Right);
            assert_eq!(after[..n], before[..], "page {} added", n + 1);
        }
        // Starting on the left: 1-2 is the first spread.
        let left = facing_layout(&ps, PageId(1), GAP_MM, PageSide::Left);
        assert_eq!(left[0].1, Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(left[1].1, Rect::new(100.0, 0.0, 200.0, 50.0));
        assert_eq!(left[2].1, Rect::new(230.0, 0.0, 330.0, 50.0));
        assert!(facing_layout(&[], PageId(1), GAP_MM, PageSide::Left).is_empty());
    }

    /// An app with four pages, a rectangle on page 2 reaching over the
    /// spine onto page 3, page 2 current.
    fn facing_app() -> App {
        let mut app = App::headless();
        app.show_welcome = false;
        for _ in 0..3 {
            app.add_page();
        }
        app.page = app.doc().pages[1].id;
        let w = app.doc().pages[1].size.width;
        app.new_shape(tracedraw_core::ShapeKind::Rect {
            rect: Rect::new(w - 20.0, 10.0, w + 30.0, 40.0),
            radius: 0.0,
            corners: None,
        });
        app
    }

    /// A range of pages chosen for the Multipage View (#1440): only those
    /// pages and the current one are laid out, in page order and packed
    /// into the grid; the range is one undo step kept in the drawing, and
    /// choosing it makes a chosen page current.
    #[test]
    fn the_multipage_view_shows_the_chosen_pages_only() {
        let mut app = App::headless();
        for _ in 0..6 {
            app.add_page();
        }
        let ids: Vec<PageId> = app.doc().pages.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 7);
        app.set_multipage(true);
        assert_eq!(app.multipage_layout().len(), 7);
        app.goto_page(0);
        assert!(!app.set_multipage_pages("9-12"), "no page of the drawing");
        assert!(!app.set_multipage_pages("x"));
        assert!(app.set_multipage_pages(" 3-5, 7 "));
        assert!(!app.set_multipage_pages("3-5, 7"), "no change");
        assert_eq!(app.doc().metadata.multipage_pages, "3-5, 7");
        assert_eq!(app.page, ids[2], "the first chosen page is current");
        let placed = app.multipage_layout();
        let shown: Vec<PageId> = placed.iter().map(|(id, _)| *id).collect();
        assert_eq!(shown, vec![ids[2], ids[3], ids[4], ids[6]]);
        // Packed: page 7 takes the fourth cell, next to page 5.
        assert!(placed[3].1.x0 > placed[2].1.x0);
        assert_eq!(placed[3].1.y1, placed[0].1.y1);
        assert_eq!(app.shown_pages(), placed);
        // The current page always shows, in its place in page order; the
        // page that was current stays where it was on screen.
        let page3 = app.view.rect_to_screen(app.page_rect());
        app.goto_page(1);
        assert_eq!(app.page, ids[1]);
        let still = app
            .multipage_layout()
            .into_iter()
            .find(|(id, _)| *id == ids[2])
            .map(|(_, r)| app.view.rect_to_screen(r))
            .expect("page 3 shown");
        assert!(
            (still.min - page3.min).length() < 0.01,
            "{still:?} {page3:?}"
        );
        let shown: Vec<PageId> = app.multipage_layout().iter().map(|(id, _)| *id).collect();
        assert_eq!(shown, vec![ids[1], ids[2], ids[3], ids[4], ids[6]]);
        // Back to every page, and the range again with Undo.
        assert!(app.set_multipage_pages(""));
        assert_eq!(app.multipage_layout().len(), 7);
        app.undo();
        assert_eq!(app.doc().metadata.multipage_pages, "3-5, 7");
        // Out of the Multipage View the range changes nothing.
        app.set_multipage(false);
        assert_eq!(app.shown_pages().len(), 1);
    }

    /// With facing pages a range keeps each page on its side of the
    /// spine; the spreads shown follow each other (#1440).
    #[test]
    fn a_range_of_facing_pages_keeps_their_sides_and_packs_the_spreads() {
        let p = pages(7);
        // Right start: spreads [1], [2 3], [4 5], [6 7]; show 2 and 6-7.
        let all = facing_layout(&p, p[0].0, 10.0, PageSide::Right);
        let some = facing_layout_of(&p, &[1, 5, 6], p[1].0, 10.0, PageSide::Right);
        assert_eq!(some.len(), 3);
        let x0 = |placed: &[(PageId, Rect)], id: PageId| {
            placed
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, r)| r.x0)
                .expect("placed")
        };
        // Page 2 is a left page: it ends at the first spine, at the
        // origin as the current page.
        assert_eq!(some[0], (p[1].0, Rect::new(0.0, 0.0, 100.0, 50.0)));
        // Pages 6 and 7 form the next spread, beside it in the first row
        // (in the full layout their spread is in the second row).
        assert_eq!(x0(&some, p[5].0), 210.0);
        assert_eq!(x0(&some, p[6].0), 310.0);
        let y1 = |placed: &[(PageId, Rect)], id: PageId| {
            placed
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, r)| r.y1)
                .expect("placed")
        };
        assert!(y1(&all, p[5].0) < y1(&all, p[1].0));
        assert_eq!(some[1].1.y1, some[0].1.y1, "one row");
        // Every page kept: the same as the full layout.
        let every: Vec<usize> = (0..7).collect();
        assert_eq!(
            facing_layout_of(&p, &every, p[0].0, 10.0, PageSide::Right),
            all
        );
        assert!(facing_layout_of(&p, &[], p[0].0, 10.0, PageSide::Right).is_empty());
        assert!(facing_layout_of(&p, &[99], p[0].0, 10.0, PageSide::Right).is_empty());
    }

    /// Facing pages on: the desktop shows the current page's spread, the
    /// facing page beside it at the spine; Zoom to Fit takes the spread;
    /// switching it is one undo step, saved with the drawing (#1972).
    #[test]
    fn facing_pages_show_the_spread_beside_the_current_page() {
        let mut app = facing_app();
        let (p2, p3) = (app.doc().pages[1].id, app.doc().pages[2].id);
        let w = app.doc().pages[1].size.width;
        assert_eq!(app.shown_pages().len(), 1);
        assert!(!app.shows_other_pages());
        let undo = app.engine.undo_label();
        assert!(app.set_facing_pages(true, PageSide::Right));
        assert!(!app.set_facing_pages(true, PageSide::Right), "no change");
        assert!(app.doc().metadata.facing_pages);
        let shown = app.shown_pages();
        assert_eq!(shown.len(), 2);
        assert!(app.shows_other_pages());
        assert_eq!(shown[0], (p2, app.page_rect()));
        assert_eq!(shown[1].0, p3);
        assert!((shown[1].1.x0 - w).abs() < 1e-9, "page 3 at the spine");
        assert!((app.fit_rect().width() - 2.0 * w).abs() < 1e-9);
        assert_eq!(app.shown_spread_rect(p3), Some(app.fit_rect()));
        // Page 1 stands alone.
        app.goto_page(0);
        assert_eq!(app.shown_pages().len(), 1);
        // Starting on the left, page 1 pairs with page 2.
        assert!(app.set_facing_pages(true, PageSide::Left));
        assert_eq!(app.shown_pages().len(), 2);
        assert!(app.shown_pages()[1].1.x0 > 0.0);
        // Back to where it was in one undo step each.
        app.undo();
        app.undo();
        assert!(!app.doc().metadata.facing_pages);
        assert_eq!(app.engine.undo_label(), undo);
    }

    /// The other page of the spread becomes current where it is, from
    /// the page tabs and navigator or a press on it, the view not moving;
    /// a press on the current page's object where it reaches over the
    /// spine keeps the page, so the object can be picked there; adding
    /// a page reflows the spreads (#1972).
    #[test]
    fn the_facing_page_becomes_current_where_it_is() {
        let mut app = facing_app();
        app.set_facing_pages(true, PageSide::Right);
        app.fit_pending = false;
        let (p2, p3) = (app.doc().pages[1].id, app.doc().pages[2].id);
        let w = app.doc().pages[1].size.width;
        let corner = app.view.to_screen(Point::new(w, 0.0));
        // On the rectangle, over page 3: page 2 stays current.
        let on_rect = app.view.to_screen(Point::new(w + 10.0, 20.0));
        assert!(!app.multipage_press(on_rect));
        assert_eq!(app.page, p2);
        // On page 3 away from it: page 3 current, nothing moved.
        let on_p3 = app.view.to_screen(Point::new(w + 50.0, 100.0));
        assert!(app.multipage_press(on_p3));
        assert_eq!(app.page, p3);
        let now = app.view.to_screen(Point::new(0.0, 0.0));
        assert!((now - corner).length() < 1e-3);
        // Page 2's rectangle over page 3 (now current): page 2 again.
        let p = app.view.to_page(on_rect);
        assert!(p.x > 0.0 && app.hit_test(p).is_none());
        assert!(app.multipage_press(on_rect));
        assert_eq!(app.page, p2);
        // The navigator to page 3: the same place, no fit.
        app.goto_page(2);
        assert_eq!(app.page, p3);
        assert!(!app.fit_pending);
        // To page 4 (another spread): fitted to that spread.
        app.goto_page(3);
        assert!(app.fit_pending);
        assert_eq!(app.shown_pages().len(), 1, "page 4 alone on the left");
        app.add_page();
        assert_eq!(app.doc().pages.len(), 5);
        app.page = app.doc().pages[3].id;
        assert_eq!(app.shown_pages().len(), 2, "pages 4 and 5 now");
        // Off: nothing beside the page, presses do nothing.
        app.set_facing_pages(false, PageSide::Right);
        assert!(!app.multipage_press(on_p3));
        assert_eq!(app.fit_rect(), app.page_rect());
    }

    /// The facing page's guidelines show within it, snapping finds its
    /// objects, and frames draw the spread (objects across the spine,
    /// the bleed around the spread, Mask Outside Page) without trouble.
    #[test]
    fn the_spread_keeps_guides_snapping_and_drawing() {
        let ctx = crate::theme::ui_context();
        let mut app = facing_app();
        let p3 = app.doc().pages[2].id;
        app.page = p3;
        app.add_guide(Guide::vertical(20.0));
        app.page = app.doc().pages[1].id;
        assert!(app.other_page_guides().is_empty());
        app.set_facing_pages(true, PageSide::Right);
        let w = app.doc().pages[1].size.width;
        let shown = app.other_page_guides();
        assert_eq!(shown.len(), 1);
        assert!(
            matches!(shown[0].0.line, GuideLine::Vertical { x } if (x - (w + 20.0)).abs() < 1e-9)
        );
        app.show_guides = true;
        app.show_bleed = true;
        let mut m = app.doc().metadata.clone();
        m.bleed = 3.0;
        app.run(tracedraw_core::Command::SetMetadata { metadata: m });
        let mut t = 0.0;
        for _ in 0..10 {
            t += 0.05;
            frame_at(&ctx, &mut app, t);
        }
        app.mask_outside_page = true;
        frame_at(&ctx, &mut app, t + 0.05);
        app.set_multipage(true);
        assert_eq!(app.multipage_layout().len(), 4);
        frame_at(&ctx, &mut app, t + 0.1);
        frame_at(&ctx, &mut app, t + 0.15);
    }

    #[test]
    fn the_cells_are_as_far_apart_as_the_spacing() {
        let ps = pages(6);
        for gap in [0.0, 5.0, 12.5, 50.0] {
            let placed = layout(&ps, PageId(1), gap);
            assert_eq!(placed[0].1, Rect::new(0.0, 0.0, 100.0, 50.0));
            assert_eq!(placed[1].1, Rect::new(100.0 + gap, 0.0, 200.0 + gap, 50.0));
            assert_eq!(
                placed[3].1,
                Rect::new(300.0 + 3.0 * gap, 0.0, 400.0 + 3.0 * gap, 50.0)
            );
            assert_eq!(placed[4].1, Rect::new(0.0, -50.0 - gap, 100.0, -gap));
            assert_eq!(
                bounds(&placed),
                Some(Rect::new(0.0, -50.0 - gap, 400.0 + 3.0 * gap, 50.0))
            );
        }
        // A negative or broken spacing counts as none.
        assert_eq!(layout(&ps, PageId(1), -10.0), layout(&ps, PageId(1), 0.0));
        assert_eq!(
            layout(&ps, PageId(1), f64::NAN),
            layout(&ps, PageId(1), 0.0)
        );
    }

    /// The drawing's spacing drives the grid, hit testing and Zoom to
    /// Fit; changing it is one undo step and keeps the current page in
    /// place.
    #[test]
    fn the_drawing_s_page_spacing_moves_the_pages_apart() {
        let mut app = App::headless();
        app.add_page();
        let first = app.doc().pages[0].id;
        let second = app.doc().pages[1].id;
        app.page = first;
        app.set_multipage(true);
        let w = app.doc().pages[0].size.width;
        assert_eq!(app.page_spacing(), GAP_MM);
        assert_eq!(app.multipage_layout()[1].1.x0, w + 30.0);
        let fit_before = app.fit_rect().width();
        let undo_before = app.engine.undo_label();
        assert!(app.set_page_spacing(5.0));
        assert!(!app.set_page_spacing(5.0), "no change, no undo step");
        assert_eq!(app.doc().metadata.page_spacing, Some(5.0));
        let placed = app.multipage_layout();
        assert_eq!(placed[0].1.x0, 0.0, "the current page stays put");
        assert_eq!(placed[1].1.x0, w + 5.0);
        assert!((app.fit_rect().width() - (fit_before - 25.0)).abs() < 1e-9);
        // A point in the old gap is on the second page now.
        let p = Point::new(w + 10.0, 1.0);
        assert_eq!(page_at(&placed, p), Some(second));
        let s = app.view.to_screen(p);
        assert!(app.multipage_press(s));
        assert_eq!(app.page, second);
        app.multipage_activate(first);
        // One undo step back to the default.
        assert_ne!(app.engine.undo_label(), undo_before);
        app.undo();
        assert_eq!(app.page_spacing(), 30.0);
        assert_eq!(app.multipage_layout()[1].1.x0, w + 30.0);
        // Out of range values are kept in range.
        assert!(app.set_page_spacing(-3.0));
        assert_eq!(app.page_spacing(), 0.0);
        assert!(!app.set_page_spacing(f64::INFINITY));
    }

    #[test]
    fn a_click_on_another_page_makes_it_current_without_moving_the_view() {
        let mut app = App::headless();
        app.add_page();
        let first = app.doc().pages[0].id;
        let second = app.doc().pages[1].id;
        app.page = first;
        app.set_multipage(true);
        app.fit_pending = false;
        let placed = app.multipage_layout();
        let r = placed[1].1;
        let on_second = app.view.to_screen(Point::new(r.x0 + 5.0, r.y0 + 5.0));
        let before = app.view.to_screen(Point::new(r.x0, r.y0));
        // A press on the current page changes nothing.
        let on_first = app.view.to_screen(Point::new(5.0, 5.0));
        assert!(!app.multipage_press(on_first));
        assert!(app.multipage_press(on_second));
        assert_eq!(app.page, second);
        // The second page's corner is now the origin, at the same place.
        let after = app.view.to_screen(Point::new(0.0, 0.0));
        assert!((after - before).length() < 1e-3);
        let p = app.view.to_page(on_second);
        assert!((p.x - 5.0).abs() < 1e-3 && (p.y - 5.0).abs() < 1e-3);
        // Fit covers both pages.
        assert!(app.fit_rect().width() > app.page_rect().width());
        // Off: a press does nothing, Fit is the page again.
        app.set_multipage(false);
        assert!(!app.multipage_press(on_first));
        assert_eq!(app.fit_rect(), app.page_rect());
    }

    /// Two pages, the first current, each with an object off its paper,
    /// in the gap between the two: the first page's reaching over the
    /// second page's paper, the second's on the desktop left of its
    /// paper. Returns the app, the pages and the objects.
    fn objects_off_their_pages() -> (
        App,
        PageId,
        PageId,
        tracedraw_core::ShapeId,
        tracedraw_core::ShapeId,
    ) {
        let mut app = App::headless();
        app.show_welcome = false;
        app.add_page();
        let first = app.doc().pages[0].id;
        let second = app.doc().pages[1].id;
        app.page = first;
        app.set_multipage(true);
        app.fit_pending = false;
        let r2 = app.multipage_layout()[1].1;
        let w = app.page_rect().width();
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| tracedraw_core::ShapeKind::Rect {
            rect: Rect::new(x0, y0, x1, y1),
            radius: 0.0,
            corners: None,
        };
        // From the first page's right edge onto the second page's paper.
        let reaching = app
            .new_shape(rect(w - 10.0, 50.0, r2.x0 + 20.0, 70.0))
            .expect("rect");
        // The second page's, on the desktop between the pages, 20 mm
        // below the first's.
        app.page = second;
        let gap = r2.x0 - w;
        let off = app
            .new_shape(rect(-gap + 5.0, 20.0, -5.0, 40.0))
            .expect("rect");
        app.page = first;
        app.selection.clear();
        (app, first, second, reaching, off)
    }

    /// In the Multipage View every visible object is picked where it
    /// shows, whatever page holds it (#2055): an object of the current
    /// page over another page's paper is picked, not that page made
    /// current; an object another page holds off its paper makes that
    /// page current and is picked by the same press; another page's
    /// paper where nothing lies makes it current.
    #[test]
    fn a_press_picks_the_object_shown_whatever_page_holds_it() {
        let (mut app, first, second, reaching, off) = objects_off_their_pages();
        let r2 = app.multipage_layout()[1].1;
        // The first page's object, over the second page's paper.
        let on_reaching = app.view.to_screen(Point::new(r2.x0 + 10.0, 60.0));
        assert!(!app.multipage_press(on_reaching));
        assert_eq!(app.page, first);
        assert_eq!(app.hit_test(app.view.to_page(on_reaching)), Some(reaching));
        // The second page's object on the desktop.
        let on_off = app.view.to_screen(Point::new(r2.x0 - 10.0, 30.0));
        assert!(app.multipage_press(on_off));
        assert_eq!(app.page, second);
        assert_eq!(app.hit_test(app.view.to_page(on_off)), Some(off));
        // Back on the first page's object, now drawn as another page's:
        // the first page is current again and the object picked.
        assert!(app.multipage_press(on_reaching));
        assert_eq!(app.page, first);
        assert_eq!(app.hit_test(app.view.to_page(on_reaching)), Some(reaching));
        // The second page's paper where nothing lies.
        let on_paper = app.view.to_screen(Point::new(r2.x0 + 50.0, 150.0));
        assert!(app.multipage_press(on_paper));
        assert_eq!(app.page, second);
        // The desktop where nothing lies: nothing changes.
        let nowhere = app.view.to_screen(Point::new(-5.0, -40.0));
        assert!(!app.multipage_press(nowhere));
        assert_eq!(app.page, second);
    }

    /// The objects a page holds off its paper are drawn while another page
    /// is current: its picture covers them, measured once a change of the
    /// drawing; they showed only while their page was current, so they
    /// seemed to vanish (#2055).
    #[test]
    fn objects_off_another_page_are_drawn() {
        let ctx = crate::theme::ui_context();
        let (mut app, first, second, _, off) = objects_off_their_pages();
        let b = app.doc().find_shape(off).expect("off").visual_bounds();
        let content = app.page_content_bounds(second).expect("objects");
        assert!(
            content.contains(b.center()) && content.x0 <= b.x0,
            "{content:?}"
        );
        assert!(app.page_content_bounds(first).is_some());
        // Both pages fitted in the window.
        app.fit_pending = true;
        let mut t = 0.0;
        for _ in 0..20 {
            t += 0.02;
            frame_at(&ctx, &mut app, t);
        }
        let shown = app
            .page_rasters
            .borrow()
            .get(&second)
            .and_then(|r| r.shown_part())
            .expect("the second page drawn");
        assert!(
            shown.x0 <= b.x0 + 1e-6 && shown.y0 <= b.y0 + 1e-6 && shown.y1 >= b.y1 - 1e-6,
            "{shown:?} misses {b:?}"
        );
        // Measured at the drawing's revision, again after a change.
        let rev = app.engine.revision();
        let kept = app
            .page_rasters
            .borrow()
            .get(&second)
            .and_then(|r| r.content);
        assert_eq!(kept.map(|(r, _)| r), Some(rev));
    }

    /// Each open drawing keeps its own pictures of its other pages: the
    /// pages of a drawing never show another drawing's pages, not even
    /// for a moment after switching, though page ids repeat between
    /// drawings (#1663).
    #[test]
    fn each_drawing_keeps_its_own_other_page_pictures() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        let fill = |app: &mut App, w: f64| {
            for _ in 0..2 {
                app.add_page();
                app.new_shape(tracedraw_core::ShapeKind::Rect {
                    rect: Rect::new(10.0, 10.0, w, 40.0),
                    radius: 0.0,
                    corners: None,
                });
            }
            app.page = app.doc().pages[0].id;
        };
        fill(&mut app, 60.0);
        let a_pages: Vec<PageId> = app.doc().pages.iter().map(|p| p.id).collect();
        app.set_multipage(true);
        let mut t = 0.0;
        let mut rest = |app: &mut App| {
            for _ in 0..400 {
                t += 0.02;
                frame_at(&ctx, app, t);
                if renders(app) >= 2 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        rest(&mut app);
        let a_renders = renders(&app);
        assert!(a_renders >= 2, "the first drawing's other pages rendered");

        app.open_document(
            tracedraw_core::Document::new("Second", Size::new(123.0, 45.0)),
            None,
        );
        fill(&mut app, 90.0);
        let b_pages: Vec<PageId> = app.doc().pages.iter().map(|p| p.id).collect();
        // The ids the cache was keyed by repeat in the new drawing.
        assert!(b_pages.iter().any(|id| a_pages.contains(id)));
        // Nothing of the first drawing's pages is there to show.
        assert!(app.page_rasters.borrow().is_empty());
        assert!(app.page_snapshot.borrow().is_none());
        rest(&mut app);
        assert!(
            renders(&app) >= 2,
            "the second drawing's other pages rendered"
        );

        // Back to the first drawing: its own pictures, none of the
        // second's, and no render needed to show them.
        app.switch_document(0);
        assert_eq!(renders(&app), a_renders);
        let a_width = app.doc().pages[0].size.width;
        assert_ne!(a_width, 123.0);
        assert!(app
            .page_snapshot
            .borrow()
            .as_ref()
            .is_none_or(|(_, d)| d.pages[0].size.width == a_width));
        // And to the second again: its own pictures came along.
        app.switch_document(1);
        assert!(renders(&app) >= 2);
    }
}

//! The Palette Editor (Window > Color Palettes > Palette Editor, Edit
//! Palette... in a palette bar's menu, or a double click on a bar's empty
//! part): a palette's name and colours, edited on a copy that OK or Apply
//! gives the palette and Cancel (or Esc) drops (#2138).
//!
//! Swatches are selected as files in a list: a click picks one,
//! Ctrl+click adds or takes one away, Shift+click picks the run from the
//! last one clicked. Delete (the key or the button) takes every selected
//! colour out at once; Move Up and Move Down, or a swatch dragged to
//! another place, reorder them; with one colour selected its name,
//! value and spot ink are edited. Undo takes back the editor's last
//! change. Built-in and locked palettes are shown, not changed.
//!
//! Palettes are not part of a drawing and have no place in its undo
//! history: Cancel and the editor's Undo are what take a change back.
//! See docs/behavior/color-palettes.md.

use crate::app::App;
use crate::i18n::{tr, trf};
use crate::theme::Tokens;
use crate::ui::tips::RichTip;
use egui::Context;
use tracedraw_core::Color;

/// The palette index the editor gives the document palette, which is
/// the drawing's and not one of [`App::palettes`] (#2321).
pub const DOCUMENT: usize = usize::MAX;

/// How many changes the editor's Undo keeps.
const UNDO_DEPTH: usize = 100;

/// The palette's name and colours as edited.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Draft {
    /// The palette in [`App::palettes`] the copy was taken from.
    pub of: usize,
    pub name: String,
    pub colors: Vec<(String, Color)>,
}

/// What the last change was, so a name typed letter by letter or a
/// colour dragged in the picker is one Undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Name,
    ColorName(usize),
    ColorValue(usize),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaletteEditorState {
    pub palette: usize,
    /// The selected colours, in palette order.
    pub selected: Vec<usize>,
    /// Where a Shift+click run starts: the colour clicked last.
    pub anchor: Option<usize>,
    /// The copy being edited; taken from the palette when the editor
    /// shows it.
    pub draft: Option<Draft>,
    /// Earlier states of the copy, for Undo.
    pub undo: Vec<Draft>,
    last_edit: Option<Edit>,
    /// A swatch being dragged to another place.
    pub dragging: Option<usize>,
    /// The selected colour is scrolled into view once, when the editor
    /// opens on it.
    pub reveal: bool,
}

impl PaletteEditorState {
    /// The editor on palette `i`.
    pub fn on(i: usize) -> Self {
        PaletteEditorState {
            palette: i,
            ..Default::default()
        }
    }

    /// The editor on palette `i` with its colour `color` selected, in
    /// view, its name, value and spot ink ready to edit: a double click
    /// on a palette swatch, as in the reference program (#2580).
    pub fn on_color(i: usize, color: usize) -> Self {
        PaletteEditorState {
            palette: i,
            selected: vec![color],
            anchor: Some(color),
            reveal: true,
            ..Default::default()
        }
    }

    /// Keep the copy as it is for Undo, before a change. Changes of the
    /// same kind in a row (a name typed) make one step.
    fn remember(&mut self, edit: Option<Edit>) {
        if edit.is_some() && edit == self.last_edit {
            return;
        }
        self.last_edit = edit;
        if let Some(d) = &self.draft {
            self.undo.push(d.clone());
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
    }

    /// Undo the editor's last change.
    pub fn undo(&mut self) -> bool {
        let Some(d) = self.undo.pop() else {
            return false;
        };
        self.draft = Some(d);
        self.last_edit = None;
        let n = self.draft.as_ref().map_or(0, |d| d.colors.len());
        self.selected.retain(|i| *i < n);
        true
    }

    /// Delete every selected colour of the copy. Says how many went.
    pub fn delete_selected(&mut self) -> usize {
        if self.selected.is_empty() {
            return 0;
        }
        self.remember(None);
        let Some(d) = self.draft.as_mut() else {
            return 0;
        };
        let n = delete_entries(&mut d.colors, &self.selected);
        // The colour after the last one deleted is picked next, so the
        // key can go on deleting.
        let next = self
            .selected
            .first()
            .copied()
            .filter(|i| *i < d.colors.len());
        self.selected = next.into_iter().collect();
        self.anchor = next;
        n
    }

    /// Move the selected colours a place up (`up`) or down.
    pub fn move_selected(&mut self, up: bool) -> bool {
        let can = self.draft.as_ref().is_some_and(|d| {
            !self.selected.is_empty()
                && if up {
                    self.selected[0] > 0
                } else {
                    self.selected.last().is_some_and(|i| i + 1 < d.colors.len())
                }
        });
        if !can {
            return false;
        }
        self.remember(None);
        if let Some(d) = self.draft.as_mut() {
            self.selected = move_entries(&mut d.colors, &self.selected, up);
        }
        true
    }

    /// Move colour `from` to the place of `to` (a swatch dragged there).
    pub fn move_one(&mut self, from: usize, to: usize) -> bool {
        let n = self.draft.as_ref().map_or(0, |d| d.colors.len());
        if from == to || from >= n || to >= n {
            return false;
        }
        self.remember(None);
        if let Some(d) = self.draft.as_mut() {
            let e = d.colors.remove(from);
            d.colors.insert(to, e);
        }
        self.selected = vec![to];
        self.anchor = Some(to);
        true
    }

    /// Add a colour at the end and select it.
    pub fn add_color(&mut self, color: Color) {
        self.remember(None);
        if let Some(d) = self.draft.as_mut() {
            let n = d.colors.len() + 1;
            d.colors.push((
                trf("dialog.palette_color_name", &[("n", &n.to_string())]),
                color,
            ));
            let i = d.colors.len() - 1;
            self.selected = vec![i];
            self.anchor = Some(i);
        }
    }
}

/// A click on swatch `i`: picks it alone, with `toggle` (Ctrl) adds it
/// to the selection or takes it away, with `range` (Shift) picks the run
/// from the anchor to it (added to the selection with both).
pub fn click(
    selected: &mut Vec<usize>,
    anchor: &mut Option<usize>,
    i: usize,
    toggle: bool,
    range: bool,
) {
    match (range, *anchor) {
        (true, Some(a)) => {
            let run = a.min(i)..=a.max(i);
            if !toggle {
                selected.clear();
            }
            for k in run {
                if !selected.contains(&k) {
                    selected.push(k);
                }
            }
        }
        _ if toggle => {
            if let Some(p) = selected.iter().position(|k| *k == i) {
                selected.remove(p);
            } else {
                selected.push(i);
            }
            *anchor = Some(i);
        }
        _ => {
            *selected = vec![i];
            *anchor = Some(i);
        }
    }
    selected.sort_unstable();
}

/// Take the entries at `which` (any order) out of `colors`. Says how
/// many went.
pub fn delete_entries<T>(colors: &mut Vec<T>, which: &[usize]) -> usize {
    let before = colors.len();
    let mut k = 0;
    colors.retain(|_| {
        let keep = !which.contains(&k);
        k += 1;
        keep
    });
    before - colors.len()
}

/// Move the entries at `which` (sorted) a place up or down, together;
/// returns where they are now. Nothing moves past an end.
pub fn move_entries<T>(colors: &mut [T], which: &[usize], up: bool) -> Vec<usize> {
    let n = colors.len();
    let blocked = if up {
        which.first().is_none_or(|i| *i == 0)
    } else {
        which.last().is_none_or(|i| *i + 1 >= n)
    };
    if blocked {
        return which.to_vec();
    }
    if up {
        for &i in which {
            colors.swap(i - 1, i);
        }
        which.iter().map(|i| i - 1).collect()
    } else {
        for &i in which.iter().rev() {
            colors.swap(i, i + 1);
        }
        which.iter().map(|i| i + 1).collect()
    }
}

impl App {
    /// Open the Palette Editor on palette `i`.
    pub fn open_palette_editor(&mut self, i: usize) {
        if i < self.palettes.len() || (i == DOCUMENT && self.has_document()) {
            self.dialog = crate::ui::dialogs::Dialog::PaletteEditor(PaletteEditorState::on(i));
        }
    }

    /// [`Self::open_palette_editor`] with the palette's colour `color`
    /// selected (#2580).
    pub fn open_palette_editor_at(&mut self, i: usize, color: usize) {
        let len = if i == DOCUMENT {
            self.has_document().then(|| self.doc().palette.len())
        } else {
            self.palettes.get(i).map(|p| p.colors.len())
        };
        match len {
            Some(n) if color < n => {
                self.dialog = crate::ui::dialogs::Dialog::PaletteEditor(
                    PaletteEditorState::on_color(i, color),
                );
            }
            _ => self.open_palette_editor(i),
        }
    }

    /// Give the user's palette `i` these colours (OK or Apply in the
    /// editor, Delete Color on a bar): its file is saved again, the
    /// palettes made in TraceDraw are kept, its bar shows them. False for
    /// a built-in or locked palette, or when nothing changed.
    pub fn set_palette_colors(&mut self, i: usize, colors: Vec<(String, Color)>) -> bool {
        if !self.palette_takes_colors(i) {
            return false;
        }
        let Some(p) = self.palettes.get_mut(i) else {
            return false;
        };
        if p.colors == colors {
            return false;
        }
        p.colors = colors;
        if let Some(path) = p.path.clone() {
            if let Err(e) = crate::palette::save_palette(p, &path) {
                log::warn!("palette {}: {e}", path.display());
            }
        }
        self.store_user_palettes();
        self.rebuild_palette();
        true
    }

    /// Delete Color on a swatch of the user's palette `i`: colour
    /// `index` goes. False for a built-in or locked palette.
    pub fn delete_palette_color(&mut self, i: usize, index: usize) -> bool {
        let Some(p) = self.palettes.get(i) else {
            return false;
        };
        if index >= p.colors.len() || !self.palette_takes_colors(i) {
            return false;
        }
        let mut colors = p.colors.clone();
        let (name, _) = colors.remove(index);
        let palette = self.palette_name(i);
        let done = self.set_palette_colors(i, colors);
        if done {
            let name = if name.is_empty() {
                tr("palette.delete_color_unnamed")
            } else {
                name
            };
            self.status = trf(
                "palette.color_deleted",
                &[("name", &name), ("palette", &palette)],
            );
        }
        done
    }

    /// OK or Apply: the editor's copy goes to its palette (its name too,
    /// as Rename gives it). Says whether anything changed.
    pub fn apply_palette_draft(&mut self, d: &Draft) -> bool {
        if d.of == DOCUMENT {
            return self.apply_document_palette_draft(d);
        }
        let Some(p) = self.palettes.get(d.of) else {
            return false;
        };
        let id = crate::settings::palette_id(p);
        let mut changed = self.set_palette_colors(d.of, d.colors.clone());
        let name = d.name.trim();
        if !name.is_empty() && name != self.palette_name(d.of) && self.palette_takes_colors(d.of) {
            self.rename_palette(&id, name);
            changed = true;
        }
        if changed {
            self.settings.save();
        }
        changed
    }

    /// OK or Apply on the document palette: its colours go to the drawing
    /// as one undo step (Edit > Undo takes them back), its name to the
    /// settings as Rename gives it. The colours carry no names of their
    /// own; a spot colour goes by its ink's (#2321).
    fn apply_document_palette_draft(&mut self, d: &Draft) -> bool {
        if !self.has_document() {
            return false;
        }
        let mut changed = false;
        let colors: Vec<Color> = d.colors.iter().map(|(_, c)| *c).collect();
        if colors != self.doc().palette {
            self.run(tracedraw_core::Command::SetDocumentPalette { colors });
            self.doc_palette_current = None;
            changed = true;
        }
        let name = d.name.trim();
        if !name.is_empty() && name != self.document_palette_name() {
            self.rename_palette(crate::settings::DOCUMENT_PALETTE, name);
            self.settings.save();
            changed = true;
        }
        changed
    }

    /// Whether the editor may change palette `i`: the document palette of
    /// an open drawing, or a palette of the user's that is not locked.
    pub(crate) fn editor_takes_colors(&self, i: usize) -> bool {
        if i == DOCUMENT {
            self.has_document()
        } else {
            self.palette_takes_colors(i)
        }
    }

    /// The name the editor shows for palette `i`.
    pub(crate) fn editor_palette_name(&self, i: usize) -> String {
        if i == DOCUMENT {
            self.document_palette_name()
        } else {
            self.palette_name(i)
        }
    }

    /// The editor's copy of palette `i`.
    pub(crate) fn palette_draft(&self, i: usize) -> Option<Draft> {
        if i == DOCUMENT {
            if !self.has_document() {
                return None;
            }
            return Some(Draft {
                of: DOCUMENT,
                name: self.document_palette_name(),
                colors: self
                    .doc()
                    .palette
                    .iter()
                    .map(|c| (crate::ui::palette::name_of(self, *c), *c))
                    .collect(),
            });
        }
        let p = self.palettes.get(i)?;
        Some(Draft {
            of: i,
            name: self.palette_name(i),
            colors: p.colors.clone(),
        })
    }
}

/// Swatches across a row of the editor.
const COLS: usize = 12;
const SIDE: f32 = 18.0;
const GAP: f32 = 2.0;

pub fn dialog(app: &mut App, ctx: &Context, st: &mut PaletteEditorState, close: &mut bool) {
    let document = st.palette == DOCUMENT;
    if (document && !app.has_document()) || (!document && st.palette >= app.palettes.len()) {
        *close = true;
        return;
    }
    if st.draft.as_ref().is_none_or(|d| d.of != st.palette) {
        // Opened on a colour (a double click on a swatch, #2580), the
        // editor keeps it selected; another palette chosen starts with
        // none.
        let opening = st.draft.is_none();
        st.draft = app.palette_draft(st.palette);
        let n = st.draft.as_ref().map_or(0, |d| d.colors.len());
        if !opening || st.selected.iter().any(|&i| i >= n) {
            st.selected.clear();
            st.anchor = None;
            st.reveal = false;
        }
        st.undo.clear();
        st.last_edit = None;
        st.dragging = None;
    }
    let read_only = !app.editor_takes_colors(st.palette);
    let builtin = app.palettes.get(st.palette).is_some_and(|p| p.builtin);
    let dirty = st.draft != app.palette_draft(st.palette);
    let mut apply = false;
    let mut save_as = false;
    crate::ui::dialogs::window(ctx, tr("menu.window.palette_editor")).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(tr("dialog.palette"));
            // The document palette first, with a drawing open (#2321).
            let names: Vec<(usize, String)> = app
                .has_document()
                .then_some(DOCUMENT)
                .into_iter()
                .chain(0..app.palettes.len())
                .map(|i| (i, app.editor_palette_name(i)))
                .collect();
            ui.add_enabled_ui(!dirty, |ui| {
                crate::ui::keynav::ComboBox::from_id_salt("pal_sel")
                    .selected_text(app.editor_palette_name(st.palette))
                    .show_ui(ui, |ui| {
                        for (i, n) in &names {
                            if ui.selectable_label(st.palette == *i, n).clicked() {
                                st.palette = *i;
                            }
                        }
                    })
                    .response
                    .rich_tip(crate::ui::tips::pair(
                        "dialog.palette",
                        if dirty {
                            "dialog.palette_apply_first"
                        } else {
                            "dialog.palette_pick_desc"
                        },
                    ));
                if ui
                    .button(tr("dialog.new_palette"))
                    .rich_tip(crate::ui::tips::pair(
                        "dialog.new_palette_tip",
                        "dialog.new_palette_desc",
                    ))
                    .clicked()
                {
                    let name = app.next_palette_name();
                    app.palettes.push(crate::palette::Palette {
                        name,
                        key: "",
                        colors: Vec::new(),
                        builtin: false,
                        path: None,
                    });
                    app.store_user_palettes();
                    st.palette = app.palettes.len() - 1;
                }
            });
        });
        let Some(draft_name) = st.draft.as_ref().map(|d| d.name.clone()) else {
            return;
        };
        if !read_only {
            ui.horizontal(|ui| {
                ui.label(tr("dialog.name"));
                let mut name = draft_name;
                let resp = ui
                    .text_edit_singleline(&mut name)
                    .rich_tip(crate::ui::tips::pair(
                        "dialog.name",
                        "dialog.palette_name_desc",
                    ));
                if resp.changed() {
                    st.remember(Some(Edit::Name));
                    if let Some(d) = st.draft.as_mut() {
                        d.name = name;
                    }
                }
            });
        }
        swatch_grid(ui, st, read_only);
        selected_row(ui, st, read_only, document);
        // Delete with the keyboard while no field takes the key.
        let delete_key = !read_only
            && ui.memory(|m| m.focused().is_none())
            && ui.input(|i| i.key_pressed(egui::Key::Delete));
        if delete_key {
            st.delete_selected();
        }
        ui.add_space(4.0);
        if read_only {
            ui.label(
                egui::RichText::new(tr(if builtin {
                    "dialog.builtin_palette_hint"
                } else {
                    "dialog.locked_palette_hint"
                }))
                .color(Tokens::text_dim())
                .size(11.0),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(tr("dialog.add_color"))
                    .rich_tip(crate::ui::tips::keyed("dialog.add_color"))
                    .clicked()
                {
                    st.add_color(Color::rgb8(128, 128, 128));
                }
                let any = !st.selected.is_empty();
                if ui
                    .add_enabled(any, egui::Button::new(tr("dialog.remove_color")))
                    .rich_tip_always(crate::ui::tips::keyed("dialog.remove_color"))
                    .clicked()
                {
                    st.delete_selected();
                }
                let n = st.draft.as_ref().map_or(0, |d| d.colors.len());
                let up = st.selected.first().is_some_and(|i| *i > 0);
                let down = st.selected.last().is_some_and(|i| i + 1 < n);
                if ui
                    .add_enabled(up, egui::Button::new(tr("docker.move_up")))
                    .rich_tip_always(crate::ui::tips::pair(
                        "docker.move_up",
                        "dialog.palette_move_desc",
                    ))
                    .clicked()
                {
                    st.move_selected(true);
                }
                if ui
                    .add_enabled(down, egui::Button::new(tr("docker.move_down")))
                    .rich_tip_always(crate::ui::tips::pair(
                        "docker.move_down",
                        "dialog.palette_move_desc",
                    ))
                    .clicked()
                {
                    st.move_selected(false);
                }
                if ui
                    .add_enabled(!st.undo.is_empty(), egui::Button::new(tr("docker.undo")))
                    .rich_tip_always(crate::ui::tips::pair(
                        "docker.undo",
                        "dialog.palette_undo_desc",
                    ))
                    .clicked()
                {
                    st.undo();
                }
                if ui
                    .button(tr("dialog.save_palette"))
                    .rich_tip(crate::ui::tips::keyed("dialog.save_palette"))
                    .clicked()
                {
                    save_as = true;
                }
            });
        }
        ui.add_space(6.0);
        let (ok, apply_label, cancel) = (tr("dialog.ok"), tr("docker.apply"), tr("dialog.cancel"));
        match crate::ui::chrome::button_row(ui, &[&ok, &apply_label, &cancel], false) {
            Some(0) => {
                apply = true;
                *close = true;
            }
            Some(1) => apply = true,
            Some(_) => *close = true,
            None => {}
        }
    });
    let dirty = st.draft != app.palette_draft(st.palette);
    if (apply || save_as) && dirty && !read_only {
        if let Some(d) = st.draft.clone() {
            app.apply_palette_draft(&d);
            // The palette as it is now (a name given back by Rename).
            st.draft = app.palette_draft(st.palette);
            st.undo.clear();
            st.last_edit = None;
        }
    }
    // The save dialog, a download, or the system's save picker on
    // Android (#1402); the palette keeps the file it was saved as.
    if save_as && !read_only && document {
        // The document palette saved as a palette file of its own; the
        // drawing keeps its palette.
        let name = app.document_palette_name();
        let pal = crate::palette::Palette {
            name: name.clone(),
            key: "",
            colors: st
                .draft
                .as_ref()
                .map(|d| d.colors.clone())
                .unwrap_or_default(),
            builtin: false,
            path: None,
        };
        crate::files::Dialog::new()
            .add_filter(
                crate::palette::filter_label(&tr("file.palettes"), &["tdpal", "gpl"]),
                &["tdpal", "gpl"],
            )
            .set_file_name(format!("{name}.tdpal"))
            .save_file(app, move |app, p| {
                if let Err(e) = crate::palette::save_palette(&pal, &p) {
                    app.status = e.to_string();
                }
            });
    } else if save_as && !read_only {
        let index = st.palette;
        let name = app.palette_name(index);
        crate::files::Dialog::new()
            .add_filter(
                crate::palette::filter_label(&tr("file.palettes"), &["tdpal", "gpl"]),
                &["tdpal", "gpl"],
            )
            .set_file_name(format!("{name}.tdpal"))
            .save_file(app, move |app, p| {
                let Some(pal) = app.palettes.get_mut(index) else {
                    return;
                };
                match crate::palette::save_palette(pal, &p) {
                    Ok(()) => pal.path = Some(p),
                    Err(e) => app.status = e.to_string(),
                }
            });
    }
}

/// The swatches: a click selects (Ctrl, Shift as in a list), a drag
/// moves one to another place.
fn swatch_grid(ui: &mut egui::Ui, st: &mut PaletteEditorState, read_only: bool) {
    let Some(colors) = st.draft.as_ref().map(|d| d.colors.clone()) else {
        return;
    };
    let mods = ui.input(|i| i.modifiers);
    let mut rects = Vec::with_capacity(colors.len());
    let mut clicked = None;
    let mut drag_start = None;
    let mut drag_stop = false;
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            let rows = colors.len().div_ceil(COLS).max(1);
            let (area, _) = ui.allocate_exact_size(
                egui::vec2(COLS as f32 * (SIDE + GAP), rows as f32 * (SIDE + GAP)),
                egui::Sense::hover(),
            );
            for (i, (name, c)) in colors.iter().enumerate() {
                let at = area.min
                    + egui::vec2(
                        (i % COLS) as f32 * (SIDE + GAP),
                        (i / COLS) as f32 * (SIDE + GAP),
                    );
                let rect = egui::Rect::from_min_size(at, egui::vec2(SIDE, SIDE));
                rects.push(rect);
                let sense = if read_only {
                    egui::Sense::click()
                } else {
                    egui::Sense::click_and_drag()
                };
                let resp = ui.interact(rect, ui.id().with(("pal_swatch", i)), sense);
                ui.painter()
                    .rect_filled(rect, 2.0, crate::canvas::to_color32(*c));
                if matches!(c, Color::Spot { .. }) {
                    crate::ui::palette::spot_mark(ui.painter(), rect, *c);
                }
                if st.selected.contains(&i) {
                    if st.reveal {
                        ui.scroll_to_rect(rect, Some(egui::Align::Center));
                    }
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        egui::Stroke::new(2.0, Tokens::ACCENT),
                        egui::StrokeKind::Outside,
                    );
                    ui.painter().rect_stroke(
                        rect.shrink(1.0),
                        1.0,
                        egui::Stroke::new(1.0, egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
                let resp =
                    resp.rich_tip_with(|| format!("{name}\n{}", tr("dialog.palette_swatch_desc")));
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.drag_started() {
                    drag_start = Some(i);
                }
                if resp.drag_stopped() {
                    drag_stop = true;
                }
            }
        });
    st.reveal = false;
    if let Some(i) = clicked {
        click(
            &mut st.selected,
            &mut st.anchor,
            i,
            mods.command || mods.ctrl,
            mods.shift,
        );
    }
    if let Some(i) = drag_start {
        st.dragging = Some(i);
    }
    let pointer = ui.ctx().pointer_latest_pos();
    let target = pointer.and_then(|p| rects.iter().position(|r| r.expand(GAP / 2.0).contains(p)));
    if let (Some(from), Some(to)) = (st.dragging, target) {
        // Where the colour will go: a line before (or after) the swatch.
        if let Some(r) = rects.get(to) {
            let x = if to > from {
                r.right() + 1.0
            } else {
                r.left() - 1.0
            };
            ui.painter().line_segment(
                [
                    egui::pos2(x, r.top() - 1.0),
                    egui::pos2(x, r.bottom() + 1.0),
                ],
                egui::Stroke::new(2.0, Tokens::ACCENT),
            );
        }
    }
    if drag_stop {
        if let (Some(from), Some(to)) = (st.dragging.take(), target) {
            st.move_one(from, to);
        }
    }
}

/// The selected colour's name, value and spot ink; with several, how
/// many.
fn selected_row(ui: &mut egui::Ui, st: &mut PaletteEditorState, read_only: bool, document: bool) {
    match st.selected.as_slice() {
        [] => {}
        [i] => {
            let i = *i;
            let Some((name, c)) = st.draft.as_ref().and_then(|d| d.colors.get(i)).cloned() else {
                return;
            };
            ui.horizontal(|ui| {
                let mut new_name = name.clone();
                // The document palette keeps no names of its own and its
                // spot colours are the drawing's inks: there a name is
                // shown only, and a spot colour is changed with its ink
                // (#2321).
                let was_spot = matches!(c, Color::Spot { .. });
                let names = !read_only && !document;
                let renamed = ui
                    .add_enabled(names, egui::TextEdit::singleline(&mut new_name))
                    .rich_tip(crate::ui::tips::pair(
                        "dialog.color_name",
                        "dialog.color_name_desc",
                    ))
                    .changed();
                let mut edited = c;
                let value_editable = !(document && was_spot);
                let changed = ui
                    .add_enabled_ui(value_editable, |ui| {
                        crate::ui::tips::tip_over(
                            ui,
                            crate::ui::tips::pair("dialog.color_value", "dialog.color_value_desc"),
                            |ui| crate::ui::color_picker::color_button(ui, &mut edited),
                        )
                    })
                    .inner
                    && value_editable;
                // A spot colour: a named ink with this colour as its
                // process stand-in, printed on a plate of its own. A
                // spot colour given another colour stays one.
                let mut spot = was_spot;
                let toggled = ui
                    .add_enabled(
                        names,
                        egui::Checkbox::new(&mut spot, tr("dialog.spot_color")),
                    )
                    .rich_tip(crate::ui::tips::pair(
                        "dialog.spot_color",
                        "dialog.spot_color_desc",
                    ))
                    .changed();
                // Only what the user changed: an entry shown as it is
                // keeps its ink number and tint.
                if !read_only && (renamed || changed || toggled) {
                    let edit = if renamed {
                        Edit::ColorName(i)
                    } else {
                        Edit::ColorValue(i)
                    };
                    st.remember(if toggled { None } else { Some(edit) });
                    let base = if changed { edited } else { c };
                    let value = crate::ui::dialogs::palette_entry_color(&new_name, base, spot);
                    if let Some(e) = st.draft.as_mut().and_then(|d| d.colors.get_mut(i)) {
                        *e = (new_name, value);
                    }
                }
                ui.label(crate::app::color_description(
                    st.draft
                        .as_ref()
                        .and_then(|d| d.colors.get(i))
                        .map_or(c, |(_, c)| *c),
                ));
            });
        }
        many => {
            ui.label(trf(
                "dialog.palette_selected",
                &[("n", &many.len().to_string())],
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::dialogs::Dialog;

    #[test]
    fn clicks_select_as_in_a_list() {
        let (mut sel, mut anchor) = (Vec::new(), None);
        click(&mut sel, &mut anchor, 3, false, false);
        assert_eq!(sel, vec![3]);
        // Ctrl adds and takes away.
        click(&mut sel, &mut anchor, 7, true, false);
        click(&mut sel, &mut anchor, 1, true, false);
        assert_eq!(sel, vec![1, 3, 7]);
        click(&mut sel, &mut anchor, 3, true, false);
        assert_eq!(sel, vec![1, 7]);
        // Shift: the run from the last one clicked, in either direction.
        click(&mut sel, &mut anchor, 5, false, false);
        click(&mut sel, &mut anchor, 2, false, true);
        assert_eq!(sel, vec![2, 3, 4, 5]);
        click(&mut sel, &mut anchor, 8, false, true);
        assert_eq!(sel, vec![5, 6, 7, 8], "the anchor stays");
        // Ctrl+Shift adds the run to what is selected.
        click(&mut sel, &mut anchor, 0, true, false);
        click(&mut sel, &mut anchor, 1, true, true);
        assert_eq!(sel, vec![0, 1, 5, 6, 7, 8]);
        // A plain click: that one alone.
        click(&mut sel, &mut anchor, 6, false, false);
        assert_eq!(sel, vec![6]);
    }

    #[test]
    fn several_entries_go_or_move_at_once() {
        let mut v = vec!['a', 'b', 'c', 'd', 'e'];
        assert_eq!(delete_entries(&mut v, &[4, 0, 2]), 3);
        assert_eq!(v, vec!['b', 'd']);
        assert_eq!(delete_entries(&mut v, &[9]), 0);
        let mut v = vec!['a', 'b', 'c', 'd', 'e'];
        assert_eq!(move_entries(&mut v, &[1, 3], true), vec![0, 2]);
        assert_eq!(v, vec!['b', 'a', 'd', 'c', 'e']);
        assert_eq!(
            move_entries(&mut v, &[0, 2], true),
            vec![0, 2],
            "at the top"
        );
        assert_eq!(v, vec!['b', 'a', 'd', 'c', 'e']);
        assert_eq!(move_entries(&mut v, &[2, 3], false), vec![3, 4]);
        assert_eq!(v, vec!['b', 'a', 'e', 'd', 'c']);
        assert_eq!(
            move_entries(&mut v, &[3, 4], false),
            vec![3, 4],
            "at the end"
        );
    }

    fn red() -> Color {
        Color::rgb8(255, 0, 0)
    }

    /// A palette of the user's with five colours, and the editor on it.
    fn app_with_palette() -> (App, usize) {
        let mut app = App::headless();
        app.show_welcome = false;
        let i = app.new_palette("Mine").expect("palette");
        app.palettes[i].colors = (0..5u8)
            .map(|k| (format!("C{k}"), Color::rgb8(k * 40, 0, 0)))
            .collect();
        app.rebuild_palette();
        app.open_palette_editor(i);
        (app, i)
    }

    fn frame(ctx: &Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| crate::ui::dialogs::show(app, &ui.ctx().clone()));
        out.textures_delta.clear();
    }

    fn state(app: &mut App) -> &mut PaletteEditorState {
        match &mut app.dialog {
            Dialog::PaletteEditor(st) => st,
            d => panic!("not the editor: {d:?}"),
        }
    }

    fn names(app: &App, i: usize) -> Vec<String> {
        app.palettes[i]
            .colors
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Several colours selected go with the Delete key at once, but only
    /// into the palette on Apply or OK; Cancel drops the changes.
    #[test]
    fn the_editor_deletes_several_colours_and_applies_them_on_ok() {
        let ctx = crate::theme::ui_context();
        let (mut app, i) = app_with_palette();
        frame(&ctx, &mut app, Vec::new());
        state(&mut app).selected = vec![1, 3];
        let delete = egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut app, vec![delete]);
        let d = state(&mut app).draft.clone().expect("draft");
        assert_eq!(
            d.colors.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["C0", "C2", "C4"]
        );
        assert_eq!(names(&app, i).len(), 5, "not applied yet");
        // Undo in the editor brings them back.
        assert!(state(&mut app).undo());
        assert_eq!(
            state(&mut app).draft.as_ref().expect("draft").colors.len(),
            5
        );
        state(&mut app).selected = vec![1, 3];
        state(&mut app).delete_selected();
        // Cancel: the palette as it was.
        let mut cancelled = app.dialog.clone();
        app.dialog = Dialog::None;
        assert_eq!(names(&app, i).len(), 5);
        // OK: they go, and the bar shows the palette as it is.
        let Dialog::PaletteEditor(st) = &mut cancelled else {
            unreachable!()
        };
        let d = st.draft.clone().expect("draft");
        assert!(app.apply_palette_draft(&d));
        assert_eq!(names(&app, i), ["C0", "C2", "C4"]);
        let row = app
            .extra_palettes
            .iter()
            .find(|r| !r.document && r.index == i)
            .expect("bar");
        assert_eq!(row.colors.len(), 3);
    }

    /// The document palette is in the editor's list, first: its colours
    /// are edited on a copy that Apply gives the drawing as one undo step,
    /// and Edit Palette on its bar opens it there (#2321).
    #[test]
    fn the_document_palette_is_edited_in_the_editor() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.show_welcome = false;
        assert!(app.has_document());
        let blue = Color::rgb8(0, 0, 255);
        let green = Color::rgb8(0, 160, 0);
        app.add_to_document_palette(red());
        app.add_to_document_palette(blue);
        app.add_to_document_palette(green);
        app.open_palette_editor(DOCUMENT);
        frame(&ctx, &mut app, Vec::new());
        let d = state(&mut app).draft.clone().expect("the document palette");
        assert_eq!(d.of, DOCUMENT);
        assert_eq!(d.name, app.document_palette_name());
        assert_eq!(
            d.colors.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
            [red(), blue, green]
        );
        assert!(app.editor_takes_colors(DOCUMENT));
        // Blue out, green up, a grey added: nothing changes before Apply.
        state(&mut app).selected = vec![1];
        state(&mut app).delete_selected();
        state(&mut app).selected = vec![1];
        assert!(state(&mut app).move_selected(true));
        state(&mut app).add_color(Color::rgb8(128, 128, 128));
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.doc().palette, [red(), blue, green]);
        let steps = app.engine.history_labels().0.len();
        let d = state(&mut app).draft.clone().expect("draft");
        assert!(app.apply_palette_draft(&d));
        assert_eq!(
            app.doc().palette,
            [green, red(), Color::rgb8(128, 128, 128)]
        );
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        // Applied again unchanged: no step.
        assert!(!app.apply_palette_draft(&d));
        app.undo();
        assert_eq!(app.doc().palette, [red(), blue, green]);
        // A new name goes to the document palette's bar and menu.
        let mut named = d.clone();
        named.name = "Job colours".into();
        assert!(app.apply_palette_draft(&named));
        assert_eq!(app.document_palette_name(), "Job colours");
        // With no drawing open the editor does not offer it.
        app.dialog = Dialog::None;
        while app.has_document() {
            app.close_active_document();
        }
        app.open_palette_editor(DOCUMENT);
        assert!(matches!(app.dialog, Dialog::None));
        assert!(app.palette_draft(DOCUMENT).is_none());
    }

    /// The editor's own buttons: Move Up and Down, a drag, a rename, and
    /// Apply giving the palette the changes while it stays open.
    #[test]
    fn colours_are_reordered_and_renamed_then_applied() {
        let ctx = crate::theme::ui_context();
        let (mut app, i) = app_with_palette();
        frame(&ctx, &mut app, Vec::new());
        let original = app.palettes[i].colors.clone();
        let st = state(&mut app);
        st.selected = vec![2, 3];
        assert!(st.move_selected(true));
        assert_eq!(st.selected, vec![1, 2]);
        assert!(st.move_one(4, 0));
        assert_eq!(st.selected, vec![0]);
        st.remember(Some(Edit::ColorName(0)));
        if let Some(d) = st.draft.as_mut() {
            d.colors[0].0 = "Last".into();
        }
        let d = st.draft.clone().expect("draft");
        assert_eq!(
            d.colors.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["Last", "C0", "C2", "C3", "C1"]
        );
        // Undo: one step at a time.
        assert!(st.undo());
        assert_eq!(st.draft.as_ref().expect("draft").colors[0].0, "C4");
        assert!(st.undo());
        assert!(st.undo());
        assert_eq!(
            st.draft.as_ref().expect("draft").colors,
            original,
            "as it was"
        );
        assert!(!state(&mut app).undo(), "nothing more");
        // Apply: the palette changes, the editor stays.
        let st = state(&mut app);
        st.selected = vec![0];
        st.move_selected(false);
        let d = st.draft.clone().expect("draft");
        assert!(app.apply_palette_draft(&d));
        assert_eq!(names(&app, i), ["C1", "C0", "C2", "C3", "C4"]);
    }

    /// A built-in palette is shown, not changed.
    #[test]
    fn builtin_palettes_are_not_changed() {
        let mut app = App::headless();
        let b = app
            .palettes
            .iter()
            .position(|p| p.builtin)
            .expect("built in");
        let before = app.palettes[b].colors.clone();
        assert!(!app.set_palette_colors(b, Vec::new()));
        assert!(!app.delete_palette_color(b, 0));
        let d = Draft {
            of: b,
            name: "Other".into(),
            colors: vec![("x".into(), red())],
        };
        assert!(!app.apply_palette_draft(&d));
        assert_eq!(app.palettes[b].colors, before);
    }

    /// Delete Color on a bar's swatch takes that colour out of the
    /// user's palette; a locked palette keeps it.
    #[test]
    fn delete_color_on_a_swatch_takes_it_out() {
        let (mut app, i) = app_with_palette();
        assert!(app.delete_palette_color(i, 1));
        assert_eq!(names(&app, i), ["C0", "C2", "C3", "C4"]);
        assert!(!app.delete_palette_color(i, 9));
        app.toggle_palette_lock(i);
        assert!(!app.delete_palette_color(i, 0));
        assert_eq!(names(&app, i).len(), 4);
    }
}

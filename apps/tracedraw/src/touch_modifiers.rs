//! Shift, Ctrl and Alt for a finger (#1499). A touch screen has no keys
//! to hold, so a finger could not add an object or a node to the
//! selection, keep a drag's angle or proportions, pick inside a group or
//! the object behind. Where there is a touch screen (Android, or once a
//! finger has touched the window) a Shift, a Ctrl, an Alt and a Delete
//! key float over the bottom left corner of the drawing (#1996; up to
//! 0.42 the first three sat on the Standard toolbar), unless Options >
//! Display turns them off: a tap on Shift, Ctrl or Alt holds the key
//! down until the next tap on it, the key drawn pressed meanwhile, and
//! Delete deletes as the Delete key does. A finger can also hold Shift,
//! Ctrl or Alt down as a key is held, while another finger draws: the
//! key is let go with the finger (#2362). The keys held this way join
//! every frame's input (the app's raw input hook), so every tool reads
//! them as it reads the keyboard's. See docs/behavior/android.md.

use crate::app::App;

/// The keys held down from the keys on the drawing: tapped on until the
/// next tap, or held down by a finger on one of them (`pressed`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TouchModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub pressed: Option<ModKey>,
}

/// One of the three keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKey {
    Shift,
    Ctrl,
    Alt,
}

impl ModKey {
    #[cfg(test)]
    pub const ALL: [ModKey; 3] = [ModKey::Shift, ModKey::Ctrl, ModKey::Alt];

    /// The key of a Standard toolbar button's id.
    pub fn of_item(id: &str) -> Option<ModKey> {
        Some(match id {
            "shift" => ModKey::Shift,
            "ctrl" => ModKey::Ctrl,
            "alt" => ModKey::Alt,
            _ => return None,
        })
    }

    /// The button's id on the Standard toolbar.
    #[cfg(test)]
    pub fn item(self) -> &'static str {
        match self {
            ModKey::Shift => "shift",
            ModKey::Ctrl => "ctrl",
            ModKey::Alt => "alt",
        }
    }

    /// What the button reads: the key's name, as on a keyboard (key names
    /// are not translated, `docs/i18n.md`).
    pub fn label(self) -> &'static str {
        match self {
            ModKey::Shift => "Shift",
            ModKey::Ctrl => "Ctrl",
            ModKey::Alt => "Alt",
        }
    }

    /// The i18n key of the button's name (its tip is `<key>_desc`).
    pub fn name_key(self) -> &'static str {
        match self {
            ModKey::Shift => "toolbar.mod_shift",
            ModKey::Ctrl => "toolbar.mod_ctrl",
            ModKey::Alt => "toolbar.mod_alt",
        }
    }
}

impl TouchModifiers {
    /// Whether `key` is held.
    pub fn held(self, key: ModKey) -> bool {
        self.pressed == Some(key)
            || match key {
                ModKey::Shift => self.shift,
                ModKey::Ctrl => self.ctrl,
                ModKey::Alt => self.alt,
            }
    }

    /// A tap on `key`'s button: held down, or let go.
    pub fn toggle(&mut self, key: ModKey) {
        match key {
            ModKey::Shift => self.shift = !self.shift,
            ModKey::Ctrl => self.ctrl = !self.ctrl,
            ModKey::Alt => self.alt = !self.alt,
        }
    }

    /// Whether any key is held.
    #[cfg(test)]
    pub fn any(self) -> bool {
        self.shift || self.ctrl || self.alt || self.pressed.is_some()
    }

    /// The keys as egui's modifiers. Ctrl is also the command key, as
    /// the Ctrl key is everywhere but on a Mac.
    pub fn modifiers(self) -> egui::Modifiers {
        let ctrl = self.held(ModKey::Ctrl);
        egui::Modifiers {
            alt: self.held(ModKey::Alt),
            ctrl,
            shift: self.held(ModKey::Shift),
            mac_cmd: false,
            command: ctrl,
        }
    }

    /// Add the held keys to a frame's input (the app's raw input hook):
    /// to the keys the frame holds, which egui takes from the frame's
    /// `ModifiersChanged` events, and to those of each press, release and
    /// key event in it. The keyboard's own keys are kept between frames
    /// (in the context), so that letting a key go on the toolbar gives
    /// back what the keyboard holds.
    pub fn apply(self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let id = egui::Id::new("tracedraw-touch-modifiers");
        let (mut keyboard, applied) = ctx.data(|d| {
            d.get_temp::<(egui::Modifiers, TouchModifiers)>(id)
                .unwrap_or_default()
        });
        let held = self.modifiers();
        let mut told = false;
        for e in &mut raw.events {
            match e {
                egui::Event::ModifiersChanged(m) => {
                    keyboard = *m;
                    *m = keyboard | held;
                    told = true;
                }
                egui::Event::PointerButton { modifiers, .. }
                | egui::Event::Key { modifiers, .. }
                | egui::Event::MouseWheel { modifiers, .. } => *modifiers |= held,
                _ => {}
            }
        }
        if !told && applied != self {
            raw.events
                .insert(0, egui::Event::ModifiersChanged(keyboard | held));
        }
        ctx.data_mut(|d| d.insert_temp(id, (keyboard, self)));
    }
}

/// A finger held down on Shift, Ctrl or Alt on the drawing, as a key is
/// held (#2362).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyFinger {
    id: u64,
    key: ModKey,
    /// It came down with no other finger on the screen, so the frame's
    /// pointer follows it: its pointer events are left out.
    pointer: bool,
    /// Another finger touched the screen while it was down: when it
    /// lifts, the key is let go instead of tapped on.
    chord: bool,
}

/// The fingers on the screen as the keys on the drawing see them: a
/// finger held on Shift, Ctrl or Alt holds the key down while it stays,
/// and the finger that draws meanwhile moves the pointer. The pointer
/// follows only the first finger down, the one on the key, and two
/// fingers on the screen pan and zoom the drawing: the key's finger is
/// taken out of the frame's input and the other finger's moves, presses
/// and releases are given as the pointer's (#2362).
#[derive(Debug, Clone, Default)]
pub struct KeyFingers {
    /// The fingers on the screen, by touch id.
    down: Vec<u64>,
    key: Option<KeyFinger>,
    /// The finger drawing while the key's finger has the pointer.
    other: Option<u64>,
}

/// Take a finger held on Shift, Ctrl or Alt on the drawing out of the
/// frame's input `raw`, holding the key down for as long as it stays,
/// and give the moves of the finger drawing meanwhile as the pointer's.
/// A finger lifted from a key with no other finger down in the meantime
/// is a tap: the key is held until the next tap, as a tap on the
/// button does. Runs in the raw input hook, before
/// [`TouchModifiers::apply`].
pub fn fingers_on_keys(app: &mut App, raw: &mut egui::RawInput) {
    use egui::{Event, PointerButton, TouchPhase};
    let keys: Vec<(ModKey, egui::Rect)> = app
        .touch_keys_shown
        .iter()
        .filter_map(|(id, r)| ModKey::of_item(id).map(|k| (k, *r)))
        .collect();
    let f = &mut app.key_fingers;
    let mods = &mut app.touch_mods;
    // The pointer events that follow a touch event belong to that
    // finger, when it has the pointer.
    let mut swallow = false;
    let press = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut out = Vec::with_capacity(raw.events.len());
    for e in std::mem::take(&mut raw.events) {
        let Event::Touch { id, phase, pos, .. } = e else {
            let pointer = matches!(
                e,
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone
            );
            if !(pointer && swallow) {
                out.push(e);
            }
            continue;
        };
        let id = id.0;
        let on_key = f.key.filter(|k| k.id == id);
        swallow = on_key.is_some_and(|k| k.pointer) || f.other.is_some();
        match phase {
            TouchPhase::Start => {
                let key = keys
                    .iter()
                    .find(|(_, r)| r.contains(pos))
                    .map(|(k, _)| *k)
                    .filter(|_| f.key.is_none() && f.other.is_none());
                let first = f.down.is_empty();
                f.down.push(id);
                if let Some(key) = key {
                    f.key = Some(KeyFinger {
                        id,
                        key,
                        pointer: first,
                        chord: !first,
                    });
                    mods.pressed = Some(key);
                    swallow = first;
                    continue;
                }
                out.push(e);
                if let Some(k) = f.key.as_mut() {
                    k.chord = true;
                    if k.pointer && f.other.is_none() {
                        f.other = Some(id);
                        out.extend([Event::PointerMoved(pos), press(pos, true)]);
                    }
                }
            }
            TouchPhase::Move => {
                if on_key.is_some() {
                    continue;
                }
                out.push(e);
                if f.other == Some(id) {
                    out.push(Event::PointerMoved(pos));
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                f.down.retain(|d| *d != id);
                if let Some(k) = on_key {
                    f.key = None;
                    mods.pressed = None;
                    if !k.chord && phase == TouchPhase::End {
                        mods.toggle(k.key);
                    }
                    continue;
                }
                out.push(e);
                if f.other == Some(id) {
                    f.other = None;
                    out.extend([press(pos, false), Event::PointerGone]);
                }
            }
        }
    }
    raw.events = out;
}

/// Whether the buttons are shown: on Android, and wherever a finger has
/// touched the window (a touch screen laptop, a tablet in a browser).
pub fn offered(ctx: &egui::Context) -> bool {
    crate::files::ANDROID || ctx.input(|i| i.has_touch_screen())
}

/// The keys on the drawing: where they go, from the drawing's bottom left
/// corner (clear of the page tabs and the scroll bar), in points.
const KEYS_MARGIN: egui::Vec2 = egui::vec2(10.0, 40.0);

/// Shift, Ctrl, Alt and Delete over the bottom left corner of the drawing
/// `canvas`, where there is a touch screen and Options > Display shows
/// them (#1996). Their places go to `App::touch_keys_shown`.
pub fn keys_on_drawing(app: &mut App, ui: &mut egui::Ui, canvas: egui::Rect) {
    app.touch_keys_shown.clear();
    if !app.settings.touch_keys || !offered(ui.ctx()) {
        return;
    }
    let ctx = ui.ctx().clone();
    let at = egui::pos2(
        canvas.left() + KEYS_MARGIN.x,
        canvas.bottom() - KEYS_MARGIN.y,
    );
    let tip = crate::ui::toolbar::tip;
    let mut shown = Vec::new();
    egui::Area::new(egui::Id::new("touch_keys_on_drawing"))
        .order(egui::Order::Middle)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(at)
        .constrain_to(canvas)
        .show(&ctx, |ui| {
            egui::Frame::new()
                .fill(crate::theme::Tokens::panel())
                .stroke(egui::Stroke::new(
                    1.0,
                    crate::theme::Tokens::control_border(),
                ))
                .corner_radius(4.0)
                .inner_margin(3.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (id, key) in [
                            ("shift", ModKey::Shift),
                            ("ctrl", ModKey::Ctrl),
                            ("alt", ModKey::Alt),
                        ] {
                            let held = app.touch_mods.held(key);
                            let r = crate::ui::toolbar::mod_button(
                                ui,
                                key.label(),
                                &tip(key.name_key()),
                                held,
                            );
                            if r.clicked() {
                                app.touch_mods.toggle(key);
                            }
                            shown.push((id, r.rect));
                        }
                        let can = app.has_document() && app.can_delete();
                        let r = crate::ui::toolbar::std_button(
                            ui,
                            crate::ui::icons::Action::Delete,
                            &tip("toolbar.delete"),
                            can,
                            false,
                        );
                        if r.clicked() {
                            app.delete_command();
                        }
                        shown.push(("delete", r.rect));
                    });
                });
        });
    app.touch_keys_shown = shown;
}

/// Whether Alt is held from the toolbar, not on the keyboard: then it
/// does not put the bars in their customize mode (Alt held moves their
/// buttons), which would leave the Alt button unable to let go.
pub fn alt_from_toolbar(app: &App) -> bool {
    app.touch_mods.held(ModKey::Alt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos: egui::pos2(1.0, 2.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        }
    }

    /// A tap holds the key, the next one lets it go.
    #[test]
    fn a_tap_holds_and_the_next_lets_go() {
        let mut m = TouchModifiers::default();
        assert!(!m.any());
        for key in ModKey::ALL {
            m.toggle(key);
            assert!(m.held(key));
            assert_eq!(ModKey::of_item(key.item()), Some(key));
        }
        assert!(m.shift && m.ctrl && m.alt);
        m.toggle(ModKey::Ctrl);
        assert!(m.shift && !m.ctrl && m.alt);
        assert_eq!(ModKey::of_item("delete"), None);
    }

    /// The keys egui holds after a frame given `events` through `apply`.
    fn held_after(
        ctx: &egui::Context,
        m: TouchModifiers,
        events: Vec<egui::Event>,
    ) -> (egui::Modifiers, Vec<egui::Event>) {
        let mut raw = egui::RawInput {
            events,
            ..Default::default()
        };
        m.apply(ctx, &mut raw);
        let events = raw.events.clone();
        let mut out = ctx.run_ui(raw, |_| {});
        out.textures_delta.clear();
        (ctx.input(|i| i.modifiers), events)
    }

    /// The held keys join the frame's keys and every press in it; the
    /// keyboard's own keys stay, and letting go gives them back.
    #[test]
    fn held_keys_join_the_frames_input() {
        let ctx = egui::Context::default();
        let none = TouchModifiers::default();
        // Nothing held: the input is left as it came.
        let events = vec![
            egui::Event::ModifiersChanged(egui::Modifiers::ALT),
            press(egui::Modifiers::ALT),
        ];
        let (mods, out) = held_after(&ctx, none, events.clone());
        assert_eq!(out, events);
        assert_eq!(mods, egui::Modifiers::ALT);

        let m = TouchModifiers {
            shift: true,
            ctrl: true,
            alt: false,
            pressed: None,
        };
        let (mods, out) = held_after(
            &ctx,
            m,
            vec![
                press(egui::Modifiers::NONE),
                egui::Event::PointerMoved(egui::pos2(3.0, 4.0)),
            ],
        );
        assert!(mods.shift && mods.ctrl && mods.command, "{mods:?}");
        assert!(mods.alt, "the keyboard's Alt stays");
        match &out[1] {
            egui::Event::PointerButton { modifiers, .. } => {
                assert!(modifiers.shift && modifiers.ctrl && !modifiers.alt);
            }
            e => panic!("{e:?}"),
        }
        // Still held on a frame with no events.
        let (mods, _) = held_after(&ctx, m, Vec::new());
        assert!(mods.shift && mods.ctrl);
        // The keyboard lets Alt go while Shift is held from the toolbar.
        let (mods, _) = held_after(
            &ctx,
            m,
            vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        );
        assert!(mods.shift && !mods.alt);
        // Let go on the toolbar: nothing is held.
        let (mods, _) = held_after(&ctx, none, Vec::new());
        assert_eq!(mods, egui::Modifiers::NONE);
    }
}

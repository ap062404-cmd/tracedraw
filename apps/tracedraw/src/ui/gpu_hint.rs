//! The hint to turn on hardware acceleration: when Use hardware
//! acceleration is off in Options > General, so a software adapter draws
//! the window on the CPU, while this computer has a graphics card, and
//! working on the drawing turns out slow (most frames of a drag, a pan or
//! a zoom come under ten a second), TraceDraw says so once a run, names
//! the card and offers to turn the option on (from the next start) or to
//! open Options. "Do not show this message again" turns the hint off; it
//! is back on in Options > Global > Warnings. Desktop only. See
//! docs/behavior/about-and-startup.md.

// The browser draws with WebGL and never looks for the hint.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::app::App;
use std::collections::VecDeque;

/// A frame that comes more than this long after the last one, seconds,
/// while the drawing is being worked on, is slow: under ten a second.
pub const SLOW_FRAME: f32 = 0.1;

/// A gap longer than this, seconds, is a pause (the pointer held still
/// in a drag), not a slow frame, and is not counted.
pub const PAUSE: f32 = 1.0;

/// How many of the last frames of work are looked at.
pub const FRAMES: usize = 60;

/// How many of them must be slow for the hint.
pub const SLOW_FRAMES: usize = 30;

/// The frames of work of this run and whether they turned out slow.
#[derive(Debug, Default)]
pub struct SlowFrames {
    /// The last [`FRAMES`] frames of work, slow or not.
    recent: VecDeque<bool>,
    /// The frame before was one of work too: only then is the gap a
    /// frame's time and not the wait for the work to begin.
    last_busy: bool,
    /// Enough frames were slow; held until the hint shows.
    slow: bool,
    /// The hint showed this run.
    pub shown: bool,
}

impl SlowFrames {
    /// Note a frame that came `dt` seconds after the last one, `busy`
    /// when the drawing is being worked on in it (dragged, panned,
    /// zoomed). True once enough frames of work were slow.
    pub fn note(&mut self, busy: bool, dt: f32) -> bool {
        if busy && self.last_busy && dt < PAUSE {
            if self.recent.len() == FRAMES {
                self.recent.pop_front();
            }
            self.recent.push_back(dt > SLOW_FRAME);
            if self.recent.iter().filter(|s| **s).count() >= SLOW_FRAMES {
                self.slow = true;
            }
        }
        self.last_busy = busy;
        self.slow
    }
}

/// Whether the hint is looked for: a software adapter draws the window
/// because hardware acceleration is off (not because the card failed,
/// which turning it on would not mend), the hint is on in Options and it
/// has not shown this run.
pub fn wanted(software: bool, acceleration_on: bool, hint_on: bool, shown: bool) -> bool {
    software && !acceleration_on && hint_on && !shown
}

/// What a button of the hint does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Turn on Use hardware acceleration, from the next start.
    TurnOn,
    /// Options > General, where the option and the card are.
    Options,
    NotNow,
}

impl Answer {
    /// The button at `i`: Turn On, Options, Not Now.
    pub fn from_button(i: usize) -> Answer {
        match i {
            0 => Answer::TurnOn,
            1 => Answer::Options,
            _ => Answer::NotNow,
        }
    }
}

/// The hint answered, "Do not show this message again" ticked or not.
pub fn answer(app: &mut App, answer: Answer, dont_show: bool) {
    if dont_show {
        app.settings.warnings.hardware_acceleration = false;
    }
    match answer {
        Answer::TurnOn => {
            app.settings.hardware_acceleration = true;
            // The new choice is tried afresh at the next start, as when it
            // is ticked in Options.
            app.settings.working_renderer.clear();
            app.status = crate::i18n::tr("gpu_hint.turned_on");
        }
        Answer::Options => {
            app.open_options_page(crate::ui::dialogs::OptionsPage::General);
        }
        Answer::NotNow => {}
    }
    if dont_show || answer == Answer::TurnOn {
        app.settings.save();
    }
}

#[cfg(not(target_arch = "wasm32"))]
static STATE: std::sync::Mutex<Option<SlowFrames>> = std::sync::Mutex::new(None);

/// Whether the drawing is being worked on in this frame: a drag with the
/// pointer moving, a scroll or a zoom.
#[cfg(not(target_arch = "wasm32"))]
fn busy(i: &egui::InputState) -> bool {
    (i.pointer.any_down() && i.pointer.delta() != egui::Vec2::ZERO)
        || i.smooth_scroll_delta != egui::Vec2::ZERO
        || i.zoom_delta() != 1.0
}

/// Each frame: note how fast frames of work come, and once they are slow
/// show the hint, when nothing else is on the screen and the work ended.
#[cfg(not(target_arch = "wasm32"))]
pub fn update(app: &mut App, ctx: &egui::Context) {
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    let st = state.get_or_insert_with(SlowFrames::default);
    if !wanted(
        crate::gpu::software(),
        app.settings.hardware_acceleration,
        app.settings.warnings.hardware_acceleration,
        st.shown,
    ) {
        return;
    }
    // Looked for on a thread of its own, ready long before it is needed.
    let card = crate::gpu::card_found();
    let (is_busy, dt, down) = ctx.input(|i| (busy(i), i.unstable_dt, i.pointer.any_down()));
    if !st.note(is_busy, dt) || is_busy || down {
        return;
    }
    let Some(card) = card else {
        return;
    };
    if !matches!(app.dialog, crate::ui::dialogs::Dialog::None) {
        return;
    }
    st.shown = true;
    log::info!("frames come slowly on the software adapter; {card} could draw them");
    app.dialog = crate::ui::dialogs::Dialog::HardwareAccelerationHint {
        card: card.to_string(),
        dont_show: false,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_frames_of_work_bring_the_hint() {
        let mut f = SlowFrames::default();
        let mut hint = false;
        for _ in 0..SLOW_FRAMES {
            hint = f.note(true, 0.2);
        }
        // The first frame of work measures the wait for it, not a frame.
        assert!(!hint);
        assert!(f.note(true, 0.2));
        // Held until the hint shows, the work over.
        assert!(f.note(false, 5.0));
    }

    #[test]
    fn fast_frames_pauses_and_idle_frames_do_not() {
        let mut f = SlowFrames::default();
        for _ in 0..200 {
            assert!(!f.note(true, 0.033));
        }
        let mut f = SlowFrames::default();
        for _ in 0..200 {
            // The pointer held still in a drag, then moved on.
            assert!(!f.note(true, 2.0));
        }
        let mut f = SlowFrames::default();
        for _ in 0..200 {
            // Frames apart, with nothing worked on between them.
            assert!(!f.note(false, 0.5));
            assert!(!f.note(true, 0.5));
        }
    }

    #[test]
    fn a_few_slow_frames_among_fast_ones_do_not() {
        let mut f = SlowFrames::default();
        for i in 0..500 {
            // One frame in three slow.
            assert!(!f.note(true, if i % 3 == 0 { 0.3 } else { 0.03 }));
        }
    }

    #[test]
    fn the_hint_is_for_acceleration_turned_off_only() {
        assert!(wanted(true, false, true, false));
        // On, the card's driver failed: turning it on would not help.
        assert!(!wanted(true, true, true, false));
        // A card draws the window already.
        assert!(!wanted(false, false, true, false));
        // Turned off in Options > Global > Warnings, or shown this run.
        assert!(!wanted(true, false, false, false));
        assert!(!wanted(true, false, true, true));
    }

    #[test]
    fn turn_on_ticks_the_option_for_the_next_start() {
        let mut app = App::headless();
        app.settings.hardware_acceleration = false;
        app.settings.working_renderer = "software".into();
        answer(&mut app, Answer::from_button(0), false);
        assert!(app.settings.hardware_acceleration);
        assert!(app.settings.working_renderer.is_empty());
        assert!(app.settings.warnings.hardware_acceleration);
        assert_eq!(app.status, crate::i18n::tr("gpu_hint.turned_on"));
    }

    #[test]
    fn options_opens_the_general_page() {
        let mut app = App::headless();
        app.settings.hardware_acceleration = false;
        answer(&mut app, Answer::from_button(1), false);
        assert!(matches!(app.dialog, crate::ui::dialogs::Dialog::Options));
        assert_eq!(app.options_page, crate::ui::dialogs::OptionsPage::General);
        assert!(!app.settings.hardware_acceleration);
    }

    #[test]
    fn do_not_show_again_turns_the_hint_off() {
        let mut app = App::headless();
        app.settings.hardware_acceleration = false;
        answer(&mut app, Answer::from_button(2), true);
        assert!(!app.settings.warnings.hardware_acceleration);
        assert!(!app.settings.hardware_acceleration);
        // On by default, also in settings saved before it existed.
        assert!(crate::prefs::WarningPrefs::default().hardware_acceleration);
        let old: crate::prefs::WarningPrefs =
            serde_json::from_str(r#"{"unread_objects": false}"#).unwrap();
        assert!(old.hardware_acceleration);
    }

    #[test]
    fn the_hint_draws() {
        let ctx = crate::theme::ui_context();
        let mut app = App::headless();
        app.dialog = crate::ui::dialogs::Dialog::HardwareAccelerationHint {
            card: "NVIDIA GeForce RTX 3050".into(),
            dont_show: false,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            let mut out = ctx.run_ui(input.clone(), |ui| {
                crate::ui::dialogs::show(&mut app, ui.ctx());
            });
            out.textures_delta.clear();
        }
        assert!(matches!(
            app.dialog,
            crate::ui::dialogs::Dialog::HardwareAccelerationHint { .. }
        ));
        assert!(crate::i18n::trf("gpu_hint.text", &[("card", "Card X")]).contains("Card X"));
    }
}

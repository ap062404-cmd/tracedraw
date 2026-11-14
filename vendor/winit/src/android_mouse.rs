//! TraceDraw: a mouse on Android (#2152, #2161, see `vendor/README.md`).
//! winit's Android backend turns every pointer of a `MotionEvent` into a
//! `Touch` and drops the actions it has no touch for, so a Bluetooth or
//! USB mouse (or a touchpad) clicked like a finger whatever button was
//! pressed, its wheel did nothing and the pointer did not hover.
//!
//! [`MouseState::step`] says what a window is told for each motion event
//! of a mouse instead: where the pointer is, the buttons that went down
//! or up since the last event (from the event's button state, which every
//! Android version fills in, not from `ACTION_BUTTON_PRESS`, which only
//! Android 6 and later send) and the wheel's turn. The file depends on
//! nothing in the crate, so that TraceDraw's tests can include and run it.

/// `MotionEvent.BUTTON_PRIMARY`: the left button.
pub const PRIMARY: u32 = 1;
/// `MotionEvent.BUTTON_SECONDARY`: the right button.
pub const SECONDARY: u32 = 1 << 1;
/// `MotionEvent.BUTTON_TERTIARY`: the middle button (the wheel pressed).
pub const TERTIARY: u32 = 1 << 2;
/// `MotionEvent.BUTTON_BACK`.
pub const BACK: u32 = 1 << 3;
/// `MotionEvent.BUTTON_FORWARD`.
pub const FORWARD: u32 = 1 << 4;

/// The buttons of a mouse, in the order they are told when several
/// change at once.
const BUTTONS: [(u32, MouseButton); 5] = [
    (PRIMARY, MouseButton::Left),
    (SECONDARY, MouseButton::Right),
    (TERTIARY, MouseButton::Middle),
    (BACK, MouseButton::Back),
    (FORWARD, MouseButton::Forward),
];

/// Every bit of [`BUTTONS`] (a stylus's buttons are not a mouse's).
const ALL: u32 = PRIMARY | SECONDARY | TERTIARY | BACK | FORWARD;

/// A motion event's action (`MotionEvent.getActionMasked`), for a mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    /// The first button went down.
    Down,
    /// The pointer moved with a button held.
    Move,
    /// The last button came up.
    Up,
    /// The system took the gesture away.
    Cancel,
    /// The pointer came over the window with no button held.
    HoverEnter,
    /// The pointer moved with no button held.
    HoverMove,
    /// The pointer stopped hovering: it left the window, or a button is
    /// about to go down.
    HoverExit,
    /// The wheel turned.
    Scroll,
    /// A button went down (Android 6 and later, after `Down` for the
    /// first one).
    ButtonPress,
    /// A button came up (Android 6 and later, before `Up` for the last
    /// one).
    ButtonRelease,
}

/// A mouse button, as the window is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// What the window is told, in this order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseOut {
    /// The pointer came over the window.
    Entered,
    /// The pointer is at the event's place.
    Moved,
    /// A button went down or came up.
    Button { button: MouseButton, pressed: bool },
    /// The wheel turned by so many lines: `y` above zero is the wheel
    /// turned away from the hand (the view goes up), `x` above zero the
    /// view going left, as the other desktops report it.
    Wheel { x: f32, y: f32 },
    /// The pointer left the window.
    Left,
}

/// What is known of the mouse between events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MouseState {
    /// The buttons the window was told are down.
    held: u32,
    /// A `Down` came with no button in its state (some mice and remote
    /// input tools): the left button is taken as held until `Up`.
    assumed: bool,
    /// The window was told the pointer is over it.
    inside: bool,
    /// A `HoverExit` came: the pointer leaves unless a press follows
    /// (the system ends the hover before every `Down`).
    leaving: bool,
}

impl MouseState {
    /// What to tell the window for one motion event of a mouse: its
    /// `action`, its button state and the wheel's axes (`AXIS_HSCROLL`,
    /// `AXIS_VSCROLL`, which are zero but for `Scroll`).
    pub fn step(
        &mut self,
        action: MouseAction,
        buttons: u32,
        hscroll: f32,
        vscroll: f32,
    ) -> Vec<MouseOut> {
        let mut out = Vec::new();
        if action == MouseAction::HoverExit {
            // Told at the end of the batch of events, unless a press
            // follows ([`MouseState::flush`]).
            self.leaving = self.inside;
            return out;
        }
        self.leaving = false;
        if !self.inside {
            self.inside = true;
            out.push(MouseOut::Entered);
        }
        out.push(MouseOut::Moved);
        let mut now = buttons & ALL;
        match action {
            MouseAction::Down => {
                self.assumed = now == 0;
            }
            MouseAction::Up | MouseAction::Cancel => {
                self.assumed = false;
                now = 0;
            }
            MouseAction::HoverEnter | MouseAction::HoverMove => {
                // Hovering is moving with no button held.
                self.assumed = false;
                now = 0;
            }
            _ => {}
        }
        if self.assumed {
            now |= PRIMARY;
        }
        self.set_buttons(now, &mut out);
        if action == MouseAction::Scroll {
            let x = if hscroll.is_finite() { -hscroll } else { 0.0 };
            let y = if vscroll.is_finite() { vscroll } else { 0.0 };
            if x != 0.0 || y != 0.0 {
                out.push(MouseOut::Wheel { x, y });
            }
        }
        out
    }

    /// The end of a batch of input events: a pointer that stopped
    /// hovering and pressed nothing has left the window. Buttons still
    /// taken as held come up first.
    pub fn flush(&mut self) -> Vec<MouseOut> {
        let mut out = Vec::new();
        if self.leaving {
            self.leaving = false;
            self.inside = false;
            self.assumed = false;
            self.set_buttons(0, &mut out);
            out.push(MouseOut::Left);
        }
        out
    }

    /// Tell the buttons that changed between the ones held and `now`:
    /// releases first, then presses.
    fn set_buttons(&mut self, now: u32, out: &mut Vec<MouseOut>) {
        for pressed in [false, true] {
            for (bit, button) in BUTTONS {
                let was = self.held & bit != 0;
                let is = now & bit != 0;
                if was != is && is == pressed {
                    out.push(MouseOut::Button { button, pressed });
                }
            }
        }
        self.held = now;
    }
}

#[cfg(test)]
mod tests {
    use super::MouseAction::*;
    use super::*;

    fn press(button: MouseButton) -> MouseOut {
        MouseOut::Button {
            button,
            pressed: true,
        }
    }

    fn release(button: MouseButton) -> MouseOut {
        MouseOut::Button {
            button,
            pressed: false,
        }
    }

    fn step(m: &mut MouseState, action: MouseAction, buttons: u32) -> Vec<MouseOut> {
        m.step(action, buttons, 0.0, 0.0)
    }

    /// The pointer hovers: the window is told it came in and where it
    /// is, and no button.
    #[test]
    fn a_mouse_moved_with_no_button_moves_the_pointer() {
        let mut m = MouseState::default();
        assert_eq!(
            step(&mut m, HoverEnter, 0),
            vec![MouseOut::Entered, MouseOut::Moved]
        );
        assert_eq!(step(&mut m, HoverMove, 0), vec![MouseOut::Moved]);
        assert_eq!(step(&mut m, HoverMove, 0), vec![MouseOut::Moved]);
    }

    /// A left click as Android 6 and later send it: the hover ends, Down
    /// and ButtonPress, then ButtonRelease and Up, and the hover starts
    /// again. One press and one release of the left button, and the
    /// pointer never leaves the window.
    #[test]
    fn a_left_click_presses_and_releases_the_left_button_once() {
        let mut m = MouseState::default();
        step(&mut m, HoverMove, 0);
        let mut all = Vec::new();
        all.extend(step(&mut m, HoverExit, 0));
        all.extend(step(&mut m, Down, PRIMARY));
        all.extend(step(&mut m, ButtonPress, PRIMARY));
        all.extend(step(&mut m, Move, PRIMARY));
        all.extend(step(&mut m, ButtonRelease, 0));
        all.extend(step(&mut m, Up, 0));
        all.extend(step(&mut m, HoverEnter, 0));
        all.extend(step(&mut m, HoverMove, 0));
        all.extend(m.flush());
        let buttons: Vec<MouseOut> = all
            .iter()
            .copied()
            .filter(|o| matches!(o, MouseOut::Button { .. }))
            .collect();
        assert_eq!(
            buttons,
            vec![press(MouseButton::Left), release(MouseButton::Left)]
        );
        assert!(!all.contains(&MouseOut::Left) && !all.contains(&MouseOut::Entered));
        // The press comes after the pointer is put at its place.
        assert_eq!(all[0], MouseOut::Moved);
        assert_eq!(all[1], press(MouseButton::Left));
    }

    /// The right button is the right button, not a touch (#2152).
    #[test]
    fn a_right_click_presses_the_right_button() {
        let mut m = MouseState::default();
        step(&mut m, HoverMove, 0);
        step(&mut m, HoverExit, 0);
        assert_eq!(
            step(&mut m, Down, SECONDARY),
            vec![MouseOut::Moved, press(MouseButton::Right)]
        );
        assert_eq!(step(&mut m, ButtonPress, SECONDARY), vec![MouseOut::Moved]);
        assert_eq!(
            step(&mut m, ButtonRelease, 0),
            vec![MouseOut::Moved, release(MouseButton::Right)]
        );
        assert_eq!(step(&mut m, Up, 0), vec![MouseOut::Moved]);
    }

    /// Before Android 6 there is no ButtonPress or ButtonRelease: the
    /// button state of Down, Move and Up says it all.
    #[test]
    fn buttons_are_read_from_the_button_state_alone() {
        let mut m = MouseState::default();
        assert_eq!(
            step(&mut m, Down, TERTIARY),
            vec![
                MouseOut::Entered,
                MouseOut::Moved,
                press(MouseButton::Middle)
            ]
        );
        assert_eq!(step(&mut m, Move, TERTIARY), vec![MouseOut::Moved]);
        assert_eq!(
            step(&mut m, Up, 0),
            vec![MouseOut::Moved, release(MouseButton::Middle)]
        );
    }

    /// A second button pressed during a drag comes with no Down of its
    /// own, and the first one released leaves the other held.
    #[test]
    fn a_second_button_goes_down_and_up_during_a_drag() {
        let mut m = MouseState::default();
        step(&mut m, Down, PRIMARY);
        assert_eq!(
            step(&mut m, ButtonPress, PRIMARY | SECONDARY),
            vec![MouseOut::Moved, press(MouseButton::Right)]
        );
        assert_eq!(
            step(&mut m, Move, SECONDARY),
            vec![MouseOut::Moved, release(MouseButton::Left)]
        );
        assert_eq!(
            step(&mut m, Up, 0),
            vec![MouseOut::Moved, release(MouseButton::Right)]
        );
    }

    /// A Down that names no button (some mice, remote input) is a left
    /// click, held through its moves until Up.
    #[test]
    fn a_down_without_a_button_is_the_left_button() {
        let mut m = MouseState::default();
        step(&mut m, HoverMove, 0);
        assert_eq!(
            step(&mut m, Down, 0),
            vec![MouseOut::Moved, press(MouseButton::Left)]
        );
        assert_eq!(step(&mut m, Move, 0), vec![MouseOut::Moved]);
        assert_eq!(
            step(&mut m, Up, 0),
            vec![MouseOut::Moved, release(MouseButton::Left)]
        );
        assert_eq!(step(&mut m, HoverMove, 0), vec![MouseOut::Moved]);
    }

    /// The wheel (#2152, #2161): a notch away from the hand is one line
    /// up, towards the hand one line down; a tilt to the right moves the
    /// view left. The pointer is put at the wheel's place first, and no
    /// button changes.
    #[test]
    fn the_wheel_scrolls_by_lines() {
        let mut m = MouseState::default();
        step(&mut m, HoverMove, 0);
        assert_eq!(
            m.step(Scroll, 0, 0.0, 1.0),
            vec![MouseOut::Moved, MouseOut::Wheel { x: 0.0, y: 1.0 }]
        );
        assert_eq!(
            m.step(Scroll, 0, 0.0, -1.0),
            vec![MouseOut::Moved, MouseOut::Wheel { x: 0.0, y: -1.0 }]
        );
        assert_eq!(
            m.step(Scroll, 0, 1.0, 0.0),
            vec![MouseOut::Moved, MouseOut::Wheel { x: -1.0, y: 0.0 }]
        );
        // A scroll event with nothing in its axes, or nonsense, turns
        // nothing.
        assert_eq!(m.step(Scroll, 0, 0.0, 0.0), vec![MouseOut::Moved]);
        assert_eq!(
            m.step(Scroll, 0, f32::NAN, f32::INFINITY),
            vec![MouseOut::Moved]
        );
        // The wheel turned during a drag keeps the button held.
        step(&mut m, Down, PRIMARY);
        assert_eq!(
            m.step(Scroll, PRIMARY, 0.0, 1.0),
            vec![MouseOut::Moved, MouseOut::Wheel { x: 0.0, y: 1.0 }]
        );
    }

    /// The pointer leaves the window when its hover ends and no press
    /// follows; it comes in again with the next event.
    #[test]
    fn the_pointer_leaves_when_the_hover_ends_without_a_press() {
        let mut m = MouseState::default();
        assert!(m.flush().is_empty());
        step(&mut m, HoverMove, 0);
        assert!(step(&mut m, HoverExit, 0).is_empty());
        assert_eq!(m.flush(), vec![MouseOut::Left]);
        assert!(m.flush().is_empty());
        assert_eq!(
            step(&mut m, HoverEnter, 0),
            vec![MouseOut::Entered, MouseOut::Moved]
        );
        // A hover that ends before the pointer was ever told is nothing.
        let mut m = MouseState::default();
        step(&mut m, HoverExit, 0);
        assert!(m.flush().is_empty());
    }

    /// A gesture the system takes away lets go of every button; so does
    /// a hover event that comes while buttons were still taken as held
    /// (an Up that never came).
    #[test]
    fn a_cancel_or_a_hover_lets_go_of_every_button() {
        let mut m = MouseState::default();
        step(&mut m, Down, PRIMARY | SECONDARY);
        assert_eq!(
            step(&mut m, Cancel, PRIMARY | SECONDARY),
            vec![
                MouseOut::Moved,
                release(MouseButton::Left),
                release(MouseButton::Right)
            ]
        );
        step(&mut m, Down, PRIMARY);
        assert_eq!(
            step(&mut m, HoverMove, 0),
            vec![MouseOut::Moved, release(MouseButton::Left)]
        );
        // A stylus's button bits are not a mouse's.
        assert_eq!(step(&mut m, HoverMove, 1 << 5), vec![MouseOut::Moved]);
    }
}

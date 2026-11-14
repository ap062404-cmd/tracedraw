//! What a copy offers other programs on the X11 clipboard (#2288).
//!
//! The clipboard library offers one kind of content at a time, so a copy
//! put the selection there as SVG text alone, and a program that takes
//! a picture before text (a presentation or a word processor) pasted the
//! SVG source as text. Here TraceDraw owns the clipboard itself, as a
//! vector editor does, and offers the selection as `image/svg+xml`, as
//! `image/png` and as text (the same SVG, which TraceDraw's own paste and
//! other editors read). The picture is drawn only when a program asks
//! for it, so a copy costs no more than before; a large one goes out in
//! parts (INCR), as the X11 clipboard conventions ask.
//!
//! One connection and one hidden window serve every copy of the session
//! from a thread of their own; a later copy replaces what is offered and
//! takes the clipboard again. Another program's copy ends the offer.

use std::sync::{Arc, Mutex};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
    PropMode, Property, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
    SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// Draws the copied objects as a PNG, when a program asks for it.
pub type DrawPng = Box<dyn FnOnce() -> Option<Vec<u8>> + Send>;

/// The picture of a copy: still to draw, or drawn (`None`: it could not
/// be drawn).
enum Png {
    Pending(DrawPng),
    Drawn(Option<Arc<Vec<u8>>>),
}

/// What the clipboard offers while TraceDraw owns it. The picture has a
/// lock of its own: it is drawn under it, on the clipboard's thread, so
/// a copy made meanwhile never waits for the drawing.
struct Offered {
    svg: Arc<Vec<u8>>,
    png: Arc<Mutex<Png>>,
}

/// The picture, drawn on the first request.
fn drawn(png: &Mutex<Png>) -> Option<Arc<Vec<u8>>> {
    let mut png = png.lock().ok()?;
    if let Png::Pending(_) = *png {
        let Png::Pending(draw) = std::mem::replace(&mut *png, Png::Drawn(None)) else {
            return None;
        };
        *png = Png::Drawn(draw().map(Arc::new));
    }
    match &*png {
        Png::Drawn(png) => png.clone(),
        Png::Pending(_) => None,
    }
}

/// The names of the types offered, in the order of the TARGETS answer:
/// the vector first, then the picture, then text.
const SVG_TYPES: [&str; 1] = ["image/svg+xml"];
const PNG_TYPES: [&str; 1] = ["image/png"];
const TEXT_TYPES: [&str; 4] = [
    "UTF8_STRING",
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "text/plain",
];

/// The atoms the owner answers with.
#[derive(Clone, Copy)]
struct Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    incr: Atom,
    svg: Atom,
    png: Atom,
    text: [Atom; 4],
}

impl Atoms {
    fn new(conn: &RustConnection) -> Option<Self> {
        let intern = |name: &str| -> Option<Atom> {
            Some(
                conn.intern_atom(false, name.as_bytes())
                    .ok()?
                    .reply()
                    .ok()?
                    .atom,
            )
        };
        Some(Atoms {
            clipboard: intern("CLIPBOARD")?,
            targets: intern("TARGETS")?,
            timestamp: intern("TIMESTAMP")?,
            incr: intern("INCR")?,
            svg: intern(SVG_TYPES[0])?,
            png: intern(PNG_TYPES[0])?,
            text: [
                intern(TEXT_TYPES[0])?,
                intern(TEXT_TYPES[1])?,
                intern(TEXT_TYPES[2])?,
                intern(TEXT_TYPES[3])?,
            ],
        })
    }

    /// The TARGETS answer.
    fn offered(&self) -> Vec<Atom> {
        let mut out = vec![self.targets, self.timestamp, self.svg, self.png];
        out.extend_from_slice(&self.text);
        out
    }
}

/// The connection that owns the clipboard and what it offers.
struct Owner {
    conn: Arc<RustConnection>,
    window: Window,
    atoms: Atoms,
    offered: Arc<Mutex<Option<Offered>>>,
    /// Cleared when the serving thread stopped (the X server went away).
    alive: Arc<std::sync::atomic::AtomicBool>,
}

static OWNER: Mutex<Option<Owner>> = Mutex::new(None);

impl Owner {
    /// A connection to the X server of `display` (`None`: `DISPLAY`) with
    /// its hidden window and the thread that answers requests.
    fn start(display: Option<&str>) -> Option<Self> {
        let (conn, screen) = x11rb::connect(display).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let window = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .ok()?;
        let atoms = Atoms::new(&conn)?;
        conn.flush().ok()?;
        let conn = Arc::new(conn);
        let offered: Arc<Mutex<Option<Offered>>> = Arc::new(Mutex::new(None));
        let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let serving = Server {
            conn: conn.clone(),
            window,
            atoms,
            offered: offered.clone(),
            transfers: Vec::new(),
            part: part_size(&conn),
        };
        let flag = alive.clone();
        std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                serving.run();
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
            })
            .ok()?;
        Some(Owner {
            conn,
            window,
            atoms,
            offered,
            alive,
        })
    }

    /// Offer `svg` and the picture `png` draws, and take the clipboard;
    /// whether TraceDraw owns it now.
    fn offer(&self, svg: String, png: DrawPng) -> bool {
        if let Ok(mut o) = self.offered.lock() {
            *o = Some(Offered {
                svg: Arc::new(svg.into_bytes()),
                png: Arc::new(Mutex::new(Png::Pending(png))),
            });
        } else {
            return false;
        }
        let c = &self.conn;
        if c.set_selection_owner(self.window, self.atoms.clipboard, x11rb::CURRENT_TIME)
            .is_err()
            || c.flush().is_err()
        {
            return false;
        }
        c.get_selection_owner(self.atoms.clipboard)
            .ok()
            .and_then(|r| r.reply().ok())
            .is_some_and(|r| r.owner == self.window)
    }
}

/// The largest part of a transfer: well under the server's largest
/// request, as the conventions ask.
fn part_size(conn: &RustConnection) -> usize {
    (conn.maximum_request_bytes() / 4).clamp(4096, 256 * 1024)
}

/// A transfer in parts to one reader: its window and property, the type,
/// the data and how much went out.
struct Transfer {
    window: Window,
    property: Atom,
    kind: Atom,
    data: Arc<Vec<u8>>,
    sent: usize,
}

/// The thread's side of the owner.
struct Server {
    conn: Arc<RustConnection>,
    window: Window,
    atoms: Atoms,
    offered: Arc<Mutex<Option<Offered>>>,
    transfers: Vec<Transfer>,
    part: usize,
}

impl Server {
    /// Answer requests until the connection ends.
    fn run(mut self) {
        loop {
            let Ok(event) = self.conn.wait_for_event() else {
                return;
            };
            match event {
                Event::SelectionRequest(e) => self.answer(&e),
                Event::SelectionClear(e) if e.owner == self.window => {
                    // Another program copied: nothing is offered any more.
                    if let Ok(mut o) = self.offered.lock() {
                        *o = None;
                    }
                }
                Event::PropertyNotify(e) if e.state == Property::DELETE => {
                    self.next_part(e.window, e.atom);
                }
                _ => {}
            }
            if self.conn.flush().is_err() {
                return;
            }
        }
    }

    /// The data and its type for a request of `target`, `None` for a type
    /// not offered.
    fn data_for(&self, target: Atom) -> Option<(Atom, Arc<Vec<u8>>)> {
        let a = &self.atoms;
        let png = {
            let offered = self.offered.lock().ok()?;
            let offered = offered.as_ref()?;
            if target == a.svg || a.text.contains(&target) {
                return Some((target, offered.svg.clone()));
            }
            if target != a.png {
                return None;
            }
            offered.png.clone()
        };
        drawn(&png).map(|png| (target, png))
    }

    fn answer(&mut self, e: &SelectionRequestEvent) {
        let a = self.atoms;
        // Old readers name no property: the answer goes in the target's.
        let property = if e.property == x11rb::NONE {
            e.target
        } else {
            e.property
        };
        let holds = self.offered.lock().is_ok_and(|o| o.is_some());
        let done = holds
            && e.selection == a.clipboard
            && if e.target == a.targets {
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::ATOM,
                        &a.offered(),
                    )
                    .is_ok()
            } else if e.target == a.timestamp {
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        e.requestor,
                        property,
                        AtomEnum::INTEGER,
                        &[x11rb::CURRENT_TIME],
                    )
                    .is_ok()
            } else if let Some((kind, data)) = self.data_for(e.target) {
                self.send(e.requestor, property, kind, data)
            } else {
                false
            };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if done { property } else { x11rb::NONE },
        };
        let _ = self
            .conn
            .send_event(false, e.requestor, EventMask::NO_EVENT, notify);
    }

    /// Put `data` in the reader's property: at once when it fits one
    /// part, else announced as a transfer in parts, each sent when the
    /// reader has taken the one before.
    fn send(&mut self, window: Window, property: Atom, kind: Atom, data: Arc<Vec<u8>>) -> bool {
        let c = &self.conn;
        if data.len() <= self.part {
            return c
                .change_property8(PropMode::REPLACE, window, property, kind, &data)
                .is_ok();
        }
        // The reader's deletes of the property ask for the next parts.
        if c.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .is_err()
        {
            return false;
        }
        let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
        if c.change_property32(
            PropMode::REPLACE,
            window,
            property,
            self.atoms.incr,
            &[size],
        )
        .is_err()
        {
            return false;
        }
        self.transfers
            .retain(|t| !(t.window == window && t.property == property));
        self.transfers.push(Transfer {
            window,
            property,
            kind,
            data,
            sent: 0,
        });
        true
    }

    /// The reader of a transfer took a part: the next one, or the empty
    /// part that ends it.
    fn next_part(&mut self, window: Window, property: Atom) {
        let Some(at) = self
            .transfers
            .iter()
            .position(|t| t.window == window && t.property == property)
        else {
            return;
        };
        let t = &mut self.transfers[at];
        let end = (t.sent + self.part).min(t.data.len());
        let part = t.data.get(t.sent..end).unwrap_or(&[]);
        let ok = self
            .conn
            .change_property8(PropMode::REPLACE, window, property, t.kind, part)
            .is_ok();
        let finished = part.is_empty() || !ok;
        t.sent = end;
        if finished {
            let t = self.transfers.remove(at);
            let _ = self.conn.change_window_attributes(
                t.window,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            );
        }
    }
}

/// Offer a copy on the clipboard of the X server of `DISPLAY`: `svg` as
/// SVG and as text, and the picture `png` draws when a program asks for
/// one. False when there is no X server, on a Wayland desktop (whose
/// clipboard is not the X server's) or when the clipboard could not be
/// taken: the caller then puts the text there the usual way.
pub fn offer(svg: String, png: DrawPng) -> bool {
    if std::env::var_os("DISPLAY").is_none() || std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return false;
    }
    offer_on(None, svg, png)
}

/// Offer a copy on the clipboard of the X server a Wayland desktop runs
/// for X11 programs (`DISPLAY`), whose clipboard the desktop shares with
/// Wayland programs: for a compositor that does not let TraceDraw set the
/// Wayland clipboard with several types (#2288, `wayland_copy.rs`).
pub fn offer_through_xwayland(svg: String, png: DrawPng) -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    offer_on(None, svg, png)
}

fn offer_on(display: Option<&str>, svg: String, png: DrawPng) -> bool {
    let Ok(mut owner) = OWNER.lock() else {
        return false;
    };
    let gone = owner
        .as_ref()
        .is_some_and(|o| !o.alive.load(std::sync::atomic::Ordering::SeqCst));
    if owner.is_none() || gone {
        *owner = Owner::start(display);
    }
    owner.as_ref().is_some_and(|o| o.offer(svg, png))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x11_clipboard::tests_support::{read_target, x_server};

    /// A picture larger than one part, as a PNG.
    fn big_png() -> Vec<u8> {
        let img = image::RgbaImage::from_fn(700, 500, |x, y| {
            let v = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) as u8;
            image::Rgba([v, v.wrapping_mul(3), x as u8, 255])
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("png");
        out
    }

    /// A copy offers the selection as SVG, as a picture and as text, so a
    /// program that takes pictures pastes a picture, not the SVG source
    /// as text (#2288); the picture is drawn only when it is asked for,
    /// and a large one arrives whole in parts.
    #[test]
    fn a_copy_offers_svg_a_picture_and_text() {
        let Some(server) = x_server() else {
            eprintln!("no X server (Xvfb) here; skipped");
            return;
        };
        let svg =
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"5\" height=\"5\"/></svg>";
        let png = big_png();
        let drawn = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = drawn.clone();
        let picture = png.clone();
        assert!(offer_on(
            Some(&server.display),
            svg.to_string(),
            Box::new(move || {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(picture)
            }),
        ));
        let targets = read_target(&server.display, "TARGETS").expect("targets");
        for t in ["image/svg+xml", "image/png", "UTF8_STRING", "text/plain"] {
            assert!(
                targets.names.iter().any(|n| n == t),
                "{t}: {:?}",
                targets.names
            );
        }
        assert_eq!(drawn.load(std::sync::atomic::Ordering::SeqCst), 0);
        let text = read_target(&server.display, "UTF8_STRING").expect("text");
        assert_eq!(text.bytes, svg.as_bytes());
        let vector = read_target(&server.display, "image/svg+xml").expect("svg");
        assert_eq!(vector.bytes, svg.as_bytes());
        // The X11 clipboard reader TraceDraw pastes with finds the picture.
        let got = read_target(&server.display, "image/png").expect("png");
        assert!(got.parts > 1, "sent in parts: {}", got.parts);
        assert_eq!(got.bytes, png);
        let again = read_target(&server.display, "image/png").expect("png again");
        assert_eq!(again.bytes, png);
        assert_eq!(drawn.load(std::sync::atomic::Ordering::SeqCst), 1);
        // A later copy replaces what is offered.
        assert!(offer_on(
            Some(&server.display),
            "<svg/>".to_string(),
            Box::new(|| None),
        ));
        let text = read_target(&server.display, "UTF8_STRING").expect("text");
        assert_eq!(text.bytes, b"<svg/>");
        assert!(read_target(&server.display, "image/png").is_none());
        // Another program's copy ends the offer.
        crate::x11_clipboard::tests_support::take_clipboard(&server.display);
        let mut owner = OWNER.lock().expect("owner");
        if let Some(o) = owner.as_ref() {
            for _ in 0..200 {
                if o.offered.lock().is_ok_and(|x| x.is_none()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(o.offered.lock().is_ok_and(|x| x.is_none()));
        }
        // The next test starts its own owner on its own server.
        *owner = None;
    }
}

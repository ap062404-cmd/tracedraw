//! System clipboard and external bitmap editing.
//!
//! Copy puts the selection on the system clipboard next to the internal
//! clipboard, so other applications can paste it: as SVG text, and on X11
//! and Wayland desktops also as an SVG image and a PNG picture (#2288). Paste Special
//! offers the system clipboard's content as text, as a bitmap or as vector
//! objects (SVG). Image > Edit Bitmap hands a PNG to the system's image
//! editor and Update from Link reads it back.

use crate::app::App;
use crate::i18n::{tr, trf};
use tracedraw_core::{
    document::{ShapeKind, TextSpan},
    geometry::Point,
    Command, ShapeId,
};

/// What the system clipboard currently offers.
#[derive(Debug, Clone, Default)]
pub struct ClipboardContent {
    pub text: Option<String>,
    pub image: Option<image::RgbaImage>,
    /// Files copied in a file manager.
    pub files: Vec<std::path::PathBuf>,
    /// Vector data another drawing program copied: SVG, PDF or EMF bytes
    /// (#2514). Pasted as objects before the picture the same copy holds.
    pub vector: Option<Vec<u8>>,
}

impl ClipboardContent {
    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.image.is_none()
            && self.files.is_empty()
            && self.vector.is_none()
    }

    /// What a [`Mark`] keeps of this content.
    pub fn mark(&self, seq: Option<u32>) -> Mark {
        Mark {
            seq,
            text: hash_of(&self.text),
            files: hash_of(&self.files),
            image: self.image.as_ref().map(image_sample),
        }
    }

    pub fn is_svg(&self) -> bool {
        self.text
            .as_deref()
            .map(|t| t.trim_start().starts_with('<') && t.contains("<svg"))
            .unwrap_or(false)
    }
}

/// What the system clipboard held when TraceDraw copied, so that a paste
/// can tell whether something was copied elsewhere since (#183). Where
/// the system counts clipboard changes (Windows) the count decides and
/// nothing is read; elsewhere the text and the files decide, and a
/// picture is decoded and sampled only when there is neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mark {
    /// The system's change count (Windows), `None` where there is none.
    pub seq: Option<u32>,
    text: u64,
    files: u64,
    image: Option<u64>,
}

impl Mark {
    /// Whether the clipboard, now at change count `seq` with `text` and
    /// `files`, still holds what was marked; `image` decodes and samples
    /// its picture, called only when the rest cannot tell.
    pub fn still_holds(
        &self,
        seq: Option<u32>,
        text: &Option<String>,
        files: &[std::path::PathBuf],
        image: impl FnOnce() -> Option<u64>,
    ) -> bool {
        if let (Some(then), Some(now)) = (self.seq, seq) {
            return then == now;
        }
        if hash_of(text) != self.text || hash_of(&files) != self.files {
            return false;
        }
        // Equal text or files: the same copy (a picture copied elsewhere
        // replaces them).
        if text.is_some() || !files.is_empty() {
            return true;
        }
        image() == self.image
    }
}

fn hash_of<T: std::hash::Hash + ?Sized>(v: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// A picture's size and about 4096 of its bytes spread over it: enough to
/// tell two copied pictures apart without hashing 48 MB.
fn image_sample(img: &image::RgbaImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let raw = img.as_raw();
    let step = (raw.len() / 4096).max(1);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (img.width(), img.height()).hash(&mut h);
    for b in raw.iter().step_by(step) {
        b.hash(&mut h);
    }
    h.finish()
}

/// The system's clipboard change count: Windows counts every change, so
/// a paste can tell nothing changed without reading the clipboard.
#[cfg(windows)]
pub fn change_count() -> Option<u32> {
    // 32-bit Windows: the DLL exports the plain name, not the stdcall
    // decorated one raw-dylib imports by default (#1975).
    #[cfg_attr(
        target_arch = "x86",
        link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")
    )]
    #[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
    unsafe extern "system" {
        fn GetClipboardSequenceNumber() -> u32;
    }
    // SAFETY: no arguments, it only returns a number (0 without access to
    // the window station).
    let n = unsafe { GetClipboardSequenceNumber() };
    (n != 0).then_some(n)
}

#[cfg(not(windows))]
pub fn change_count() -> Option<u32> {
    None
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn read_system_text_and_files() -> ClipboardContent {
    read_text_and_files()
}

#[cfg(target_arch = "wasm32")]
fn read_system_text_and_files() -> ClipboardContent {
    ClipboardContent::default()
}

/// The system clipboard's text and files on Android, through the
/// activity's clipboard service: documents other applications copied
/// (content URIs) are named like the ones the document picker returns,
/// so pasting imports them; a lone picture is the clipboard's picture
/// ([`read_system_image`]), placed as a bitmap (#1403).
#[cfg(target_os = "android")]
fn read_system_text_and_files() -> ClipboardContent {
    let (files, _) = split_documents(crate::android::clipboard::documents());
    ClipboardContent {
        text: crate::android::clipboard::text(),
        image: None,
        files,
        vector: None,
    }
}

/// A document on the clipboard (Android): its content URI, MIME type and
/// shown name.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipDocument {
    pub uri: String,
    pub mime: String,
    pub name: String,
}

/// The documents `getClipboardDocuments` lists (Android): one per line,
/// "uri TAB mime TAB name".
#[cfg(any(target_os = "android", test))]
pub fn parse_clip_documents(lines: &str) -> Vec<ClipDocument> {
    lines
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let uri = parts.next()?.trim();
            if uri.is_empty() {
                return None;
            }
            Some(ClipDocument {
                uri: uri.to_string(),
                mime: parts.next().unwrap_or("").trim().to_string(),
                name: parts.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect()
}

/// The clipboard's documents as files to import, or the URI of its
/// picture when it holds one picture alone (copied from a gallery, a
/// browser or a chat: placed as a bitmap like a picture copied on the
/// desktop). Several documents are all imported.
#[cfg(any(target_os = "android", test))]
fn split_documents(docs: Vec<ClipDocument>) -> (Vec<std::path::PathBuf>, Option<String>) {
    if let [doc] = docs.as_slice() {
        if doc.mime.starts_with("image/") {
            return (Vec::new(), Some(doc.uri.clone()));
        }
    }
    let files = docs
        .iter()
        .map(|d| crate::files::document_path(&d.uri, &d.name))
        .collect();
    (files, None)
}

/// The system clipboard's text, read where the system lets an
/// application read it (not in a browser, which hands it only to paste
/// events).
pub fn system_text() -> Option<String> {
    read_system_text_and_files().text
}

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn read_system_image() -> Option<image::RgbaImage> {
    read_image()
}

#[cfg(target_arch = "wasm32")]
fn read_system_image() -> Option<image::RgbaImage> {
    None
}

/// The picture another application copied (Android): one image document
/// alone on the clipboard, decoded.
#[cfg(target_os = "android")]
fn read_system_image() -> Option<image::RgbaImage> {
    let (_, uri) = split_documents(crate::android::clipboard::documents());
    let bytes = crate::android::documents::read(&uri?).ok()?;
    match crate::files::decode_picture(&bytes) {
        Ok(img) => Some(img.into_rgba8()),
        Err(e) => {
            log::warn!("the copied picture cannot be read: {e}");
            None
        }
    }
}

/// The system clipboard as seen by the browser build: empty, because
/// browsers hand clipboard contents only to paste events and, in the
/// background, to a clipboard read ([`Handed`]).
#[cfg(target_arch = "wasm32")]
pub fn read_system() -> ClipboardContent {
    ClipboardContent::default()
}

/// The system clipboard on Android: its text, documents and picture
/// (`android.rs`).
#[cfg(target_os = "android")]
pub fn read_system() -> ClipboardContent {
    let mut content = read_system_text_and_files();
    content.image = read_system_image();
    content
}

/// Read text, files and image from the system clipboard; failures yield
/// `None` fields (headless sessions have no clipboard).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn read_system() -> ClipboardContent {
    let mut content = read_text_and_files();
    content.image = read_image();
    content
}

thread_local! {
    /// The one connection to the system clipboard, kept for the session
    /// (#1488): a new one for each read cost a new connection to the X
    /// server each time.
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    static SYSTEM_CLIPBOARD: std::cell::RefCell<Option<arboard::Clipboard>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with the system clipboard, opened on first use; `None` when it
/// cannot be opened (headless sessions have no clipboard).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn with_clipboard<T>(f: impl FnOnce(&mut arboard::Clipboard) -> T) -> Option<T> {
    SYSTEM_CLIPBOARD.with(|cell| {
        let mut cb = cell.try_borrow_mut().ok()?;
        if cb.is_none() {
            *cb = arboard::Clipboard::new().ok();
        }
        cb.as_mut().map(f)
    })
}

/// Whether the clipboard offers text and a list of files, where
/// TraceDraw reads its list of types before the clipboard library reads
/// it (an X11 desktop: no `WAYLAND_DISPLAY`); `None` elsewhere, where the
/// library is asked for both.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn x11_offers_text_and_files() -> Option<(bool, bool)> {
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return crate::x11_clipboard::offer().map(|o| (o.has_text(), o.has_files()));
    }
    None
}

/// Whether a paste shortcut may read the clipboard's text: asked by the
/// windowing layer before each one (`eframe::set_paste_text_gate`). On an
/// X11 desktop only when the clipboard offers text: a program that holds
/// a picture alone may answer a text request with the picture sent in
/// parts, which the text read stops after the first part, and then wait
/// for the rest and answer nobody, so the picture never pasted and the
/// window froze for the reads that waited (#1488).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn paste_may_read_text() -> bool {
    x11_offers_text_and_files().is_none_or(|(text, _)| text)
}

/// The clipboard's text and files, not its picture (decoding one costs
/// far more). On an X11 desktop each is asked for only when the
/// clipboard offers it (#1488, [`paste_may_read_text`]).
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn read_text_and_files() -> ClipboardContent {
    let (text_offered, files_offered) = x11_offers_text_and_files().unwrap_or((true, true));
    if !text_offered && !files_offered {
        return ClipboardContent::default();
    }
    with_clipboard(|cb| {
        let text = text_offered
            .then(|| cb.get_text().ok())
            .flatten()
            .filter(|t| !t.is_empty());
        let files = if files_offered {
            cb.get()
                .file_list()
                .unwrap_or_default()
                .into_iter()
                .map(without_line_end)
                .collect()
        } else {
            Vec::new()
        };
        ClipboardContent {
            text,
            image: None,
            files,
            vector: None,
        }
    })
    .map(|mut content| {
        content.vector = read_vector();
        content
    })
    .unwrap_or_default()
}

/// Vector data on the clipboard, as another drawing program copies it
/// beside a picture of it (#2514): SVG, PDF or an enhanced metafile.
/// The first of these offered is read, in that order.
#[cfg(windows)]
fn read_vector() -> Option<Vec<u8>> {
    crate::win_clipboard::read_vector()
}

/// Vector data on the clipboard of an X11 desktop (also beside a Wayland
/// one): SVG, PDF or EMF, the first offered (#2514).
#[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
fn read_vector() -> Option<Vec<u8>> {
    crate::x11_clipboard::read_vector()
}

#[cfg(not(any(
    windows,
    all(unix, not(any(target_os = "macos", target_os = "android")))
)))]
#[allow(dead_code)]
fn read_vector() -> Option<Vec<u8>> {
    None
}

/// The clipboard's picture, decoded. On an X11 desktop the clipboard's
/// types are read and its best picture type is asked for, with time for
/// a busy program that sends a large picture in parts (#1488, see
/// `x11_clipboard.rs`); elsewhere (Windows, macOS, a Wayland desktop),
/// and when that finds nothing, the clipboard library reads its PNG.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn read_image() -> Option<image::RgbaImage> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let x11_first = std::env::var_os("WAYLAND_DISPLAY").is_none();
    #[cfg(all(unix, not(target_os = "macos")))]
    if x11_first {
        if let Some(img) = crate::x11_clipboard::read_image() {
            return Some(img);
        }
    }
    let from_library = with_clipboard(|cb| cb.get_image().ok())
        .flatten()
        .and_then(|img| {
            image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
        });
    // A Wayland desktop whose clipboard the library cannot read: the
    // X11 one the desktop keeps beside it.
    #[cfg(all(unix, not(target_os = "macos")))]
    if from_library.is_none() && !x11_first {
        return crate::x11_clipboard::read_image();
    }
    from_library
}

/// A path from a copied file list without the carriage return its line
/// may keep: the list's lines end in CR LF and the clipboard library
/// splits it at LF only.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn without_line_end(path: std::path::PathBuf) -> std::path::PathBuf {
    match path.to_str() {
        Some(s) if s.ends_with(['\r', '\n']) => s.trim_end_matches(['\r', '\n']).into(),
        _ => path,
    }
}

/// The resolution of the picture a copy offers (#2288): 150 dpi, less
/// for a selection so large that its longer side would pass 4000 pixels.
#[cfg_attr(
    not(all(unix, not(any(target_os = "macos", target_os = "android")))),
    allow(dead_code)
)]
fn copy_picture_dpi(width_mm: f64, height_mm: f64) -> f64 {
    const DPI: f64 = 150.0;
    const MOST_PX: f64 = 4000.0;
    let side = width_mm.max(height_mm);
    if !side.is_finite() || side <= 0.0 {
        return DPI;
    }
    DPI.min(MOST_PX * 25.4 / side).max(1.0)
}

/// Put text on the system clipboard (best effort); whether it went there.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn write_text(text: &str) -> bool {
    with_clipboard(|cb| cb.set_text(text.to_string()).is_ok()).unwrap_or(false)
}

/// Put text on the system clipboard through the activity's clipboard
/// service (Android); whether it went there.
#[cfg(target_os = "android")]
pub fn write_text(text: &str) -> bool {
    crate::android::clipboard::set_text(text)
}

/// Put text on the system clipboard through egui (browser build).
#[cfg(target_arch = "wasm32")]
pub fn write_text(text: &str) -> bool {
    crate::web::copy_text(text);
    true
}

/// A file a browser handed over for a paste: a picture or a file copied
/// in another application (#1403).
#[derive(Debug, Clone, Default)]
pub struct PastedFile {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// What a browser handed over for one paste, read in the background: the
/// files of a paste event (Ctrl+V), or what the clipboard API read for
/// Edit > Paste (#1403).
#[derive(Debug, Clone, Default)]
pub struct Handed {
    pub text: Option<String>,
    pub files: Vec<PastedFile>,
    /// From Edit > Paste (a clipboard read), not from a paste event: its
    /// text may be the copy made here, which then pastes as objects.
    pub from_menu: bool,
    /// The browser did not let the clipboard be read (Edit > Paste).
    pub refused: bool,
    /// Where the pasted objects go (the page's context menu), page space.
    pub at: Option<Point>,
}

thread_local! {
    /// Pastes handed over since the last frame.
    static HANDED: std::cell::RefCell<Vec<Handed>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Pastes being read in the background.
    static AWAITED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A paste is being read in the background: the paste shortcut of this
/// frame must not paste the copy here as well (`App::paste_with_event_text`).
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn expect_handed() {
    AWAITED.with(|a| a.set(a.get() + 1));
}

/// A paste read in the background is ready for the next frame. Only the
/// files of a paste event were awaited ([`expect_handed`]): Edit > Paste
/// never holds the paste shortcut back, as the browser may wait for the
/// user to allow the clipboard to be read.
#[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
pub fn hand_over(handed: Handed) {
    if !handed.from_menu {
        AWAITED.with(|a| a.set(a.get().saturating_sub(1)));
    }
    HANDED.with(|h| h.borrow_mut().push(handed));
}

/// Whether a paste is still being read in the background.
pub fn handed_awaited() -> bool {
    AWAITED.with(|a| a.get() > 0)
}

/// The image file among `files` when it is the only file and decodes: a
/// picture copied in another application, placed as a bitmap.
fn lone_picture(files: &[PastedFile]) -> Option<image::RgbaImage> {
    let [file] = files else {
        return None;
    };
    if !file.mime.starts_with("image/") {
        return None;
    }
    match crate::files::decode_picture(&file.bytes) {
        Ok(img) => Some(img.into_rgba8()),
        Err(e) => {
            log::warn!("the pasted picture cannot be read: {e}");
            None
        }
    }
}

impl App {
    /// The selection as a standalone SVG document (page-sized viewport so
    /// positions survive a round trip).
    pub fn selection_svg(&self) -> Option<String> {
        let shapes = self.selected_shapes();
        if shapes.is_empty() {
            return None;
        }
        let mut tmp = tracedraw_core::Document::new("clipboard", self.page_size());
        tmp.symbols = self.doc().symbols.clone();
        let layer = tmp.pages[0].layers[0].id;
        if let Ok(l) = tmp.layer_mut(layer) {
            l.shapes = shapes;
        }
        Some(tracedraw_io::svg::page_to_svg(&tmp, 0))
    }

    /// Copy to the internal clipboard and, as SVG, to the system clipboard.
    pub fn copy_with_system(&mut self) {
        // While a text is being edited: its selected characters (#1377).
        if self.editing_text() {
            self.text_copy();
            return;
        }
        // Guidelines selected and no objects: they are copied (#376).
        if self.selection.is_empty() && self.copy_selected_guides() {
            return;
        }
        // Nothing selected: the last copy stays as it was.
        let Some(svg) = self.selection_svg() else {
            return;
        };
        self.copy();
        let written = self.offer_copy(&svg) || write_text(&svg);
        self.mark_system_clipboard(written.then_some(svg.as_str()));
    }

    /// Offer the copy `svg` to other programs as SVG, as a picture and as
    /// text, where TraceDraw owns the clipboard itself (an X11 clipboard,
    /// #2288; a Wayland one, `wayland_copy.rs`): a program that takes
    /// pictures pastes the drawing, not its SVG source as text. On X11 the
    /// picture is drawn only when a program asks for it; on Wayland on a
    /// thread of its own, right after the copy. False elsewhere, where the
    /// text goes there alone.
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
    fn offer_copy(&self, svg: &str) -> bool {
        let Some(bounds) = self.selection_bounds() else {
            return false;
        };
        let Some(alone) = self.selection_alone(bounds) else {
            return false;
        };
        let dpi = copy_picture_dpi(bounds.width(), bounds.height());
        let draw: crate::x11_copy::DrawPng = Box::new(move || {
            crate::ops2::render_alone_png(&alone, dpi, true, true).map(|(png, _, _)| png)
        });
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return crate::x11_copy::offer(svg.to_string(), draw);
        }
        // A Wayland desktop: its clipboard, else the X11 programs' one,
        // which the desktop shares.
        match crate::wayland_copy::offer(svg.to_string(), draw) {
            Ok(()) => true,
            Err(draw) => crate::x11_copy::offer_through_xwayland(svg.to_string(), draw),
        }
    }

    #[cfg(not(all(unix, not(any(target_os = "macos", target_os = "android")))))]
    fn offer_copy(&self, _svg: &str) -> bool {
        false
    }

    /// Remember what the system clipboard holds right after a copy here
    /// (`written`: the text the copy put there), so that a paste can tell
    /// whether something was copied elsewhere since. What was written is
    /// what the clipboard holds (writing text replaces everything there),
    /// so it is not read back; a copy that wrote nothing (nodes, guides)
    /// reads it, its picture only where the system keeps no change count
    /// and there is no text or file (#183).
    pub fn mark_system_clipboard(&mut self, written: Option<&str>) {
        let seq = change_count();
        let mark = match written {
            Some(text) => ClipboardContent {
                text: Some(text.to_string()),
                ..Default::default()
            }
            .mark(seq),
            None if seq.is_some() => Mark {
                seq,
                ..Default::default()
            },
            None => {
                let mut now = read_system_text_and_files();
                if now.text.is_none() && now.files.is_empty() {
                    now.image = read_system_image();
                }
                now.mark(seq)
            }
        };
        if let Some(cb) = self.clipboard.as_mut() {
            cb.system = mark;
        }
    }

    /// Paste the system clipboard's text as artistic text at the page
    /// centre. Characters copied while editing a text, while the
    /// clipboard still holds them, keep their formatting and their
    /// text's fill and outline (#1378).
    pub fn paste_as_text(&mut self, text: &str) {
        let c = self.page_rect().center();
        let clip = self
            .text_clip
            .as_ref()
            .filter(|clip| clip.plain == crate::text_editing::unify_line_ends(text))
            .cloned();
        let (spans, fill, stroke) = match clip {
            Some(clip) => (
                clip.spans
                    .into_iter()
                    .filter(|sp| !sp.text.is_empty())
                    .collect(),
                clip.fill,
                clip.stroke,
            ),
            None => (
                vec![TextSpan {
                    bold: self.text_bold,
                    italic: self.text_italic,
                    ..TextSpan::new(text, self.text_font.clone(), self.text_size_pt)
                }],
                tracedraw_core::Fill::Solid(tracedraw_core::Color::BLACK),
                None,
            ),
        };
        let kind = ShapeKind::Text {
            spans,
            origin: Point::new(c.x, c.y),
            frame: None,
            align: tracedraw_core::TextAlign::Left,
            para: Default::default(),
            on_path: None,
        };
        if let Some(id) = self.new_shape(kind) {
            self.run(Command::SetFill {
                shapes: vec![id],
                fill,
            });
            self.run(Command::SetStroke {
                shapes: vec![id],
                stroke,
            });
            self.select(vec![id]);
        }
    }

    /// Paste SVG text from the system clipboard as objects.
    pub fn paste_as_svg(&mut self, svg: &str) {
        let Some(layer) = self.active_layer() else {
            return;
        };
        match tracedraw_io::svg_import::parse(svg, &mut tracedraw_core::id::IdSource::default()) {
            Ok(imported) => {
                let mut cmds = Vec::new();
                let mut ids = Vec::new();
                for s in imported.shapes {
                    let id = self.engine.new_shape_id();
                    let shape = crate::app::reid_pub(s, id, &mut self.engine);
                    ids.push(id);
                    cmds.push(Command::AddShape { layer, shape });
                }
                if let Err(e) = self.engine.run_batch("Paste", &cmds) {
                    self.status = e.to_string();
                }
                self.select(ids);
            }
            Err(e) => self.status = trf("status.import_failed", &[("e", &e)]),
        }
    }

    /// Paste vector data another program copied (SVG, PDF or EMF, told
    /// apart by their own signatures) as objects, one undo step, selected
    /// (#2514). False when it holds nothing this reads, so the picture
    /// copied with it is pasted instead.
    pub fn paste_vector(&mut self, bytes: &[u8]) -> bool {
        let mut ids = tracedraw_core::id::IdSource::default();
        let shapes = if let Some(svg) = std::str::from_utf8(bytes)
            .ok()
            .map(|t| t.trim_start_matches('\u{feff}').trim_start())
            .filter(|t| t.starts_with('<') && t.contains("<svg"))
        {
            tracedraw_io::svg_import::parse(svg, &mut ids)
                .map(|i| i.shapes)
                .ok()
        } else if tracedraw_io::pdf_import::is_pdf(bytes) {
            tracedraw_io::pdf_import::parse(bytes, &mut ids)
                .ok()
                .and_then(|p| p.pages.into_iter().next())
                .map(|page| page.shapes)
        } else if tracedraw_io::emf::is_emf(bytes) || tracedraw_io::emf::is_wmf(bytes) {
            tracedraw_io::emf::parse(bytes, &mut ids)
                .map(|i| i.shapes)
                .ok()
        } else {
            None
        };
        let Some(shapes) = shapes.filter(|s| !s.is_empty()) else {
            return false;
        };
        let Some(layer) = self.active_layer() else {
            return false;
        };
        let mut cmds = Vec::new();
        let mut new_ids = Vec::new();
        for s in shapes {
            let id = self.engine.new_shape_id();
            let shape = crate::app::reid_pub(s, id, &mut self.engine);
            new_ids.push(id);
            cmds.push(Command::AddShape { layer, shape });
        }
        if let Err(e) = self.engine.run_batch("Paste", &cmds) {
            self.status = e.to_string();
            return false;
        }
        self.select(new_ids);
        true
    }

    /// Paste the system clipboard's image as a bitmap object.
    pub fn paste_as_bitmap(&mut self, img: image::RgbaImage) {
        if let Some(id) = self.place_image(img, Some(tr("kind.bitmap_short"))) {
            self.select(vec![id]);
        }
    }

    /// Edit > Paste: what was copied last, here or in another
    /// application.
    pub fn paste_any(&mut self) {
        self.paste_any_at(None);
    }

    /// Edit > Paste with the pasted objects centred on `at` (the page's
    /// context menu), when given. In a browser the clipboard is read in
    /// the background (its API asks the user the first time) and the
    /// paste happens on a later frame ([`App::take_handed_pastes`]);
    /// where it cannot be read, the copy here is pasted (#1403).
    pub fn paste_any_at(&mut self, at: Option<Point>) {
        #[cfg(target_arch = "wasm32")]
        if crate::web::read_clipboard(Handed {
            from_menu: true,
            at,
            ..Default::default()
        }) {
            return;
        }
        let before = self.selection.clone();
        self.paste_with_event_text(None);
        self.move_pasted_to(&before, at);
    }

    /// Centre what a paste selected on `at`, when given and the paste
    /// selected something new.
    fn move_pasted_to(&mut self, before: &[ShapeId], at: Option<Point>) {
        let Some(at) = at else {
            return;
        };
        if self.selection.is_empty() || self.selection == before {
            return;
        }
        if let Some(b) = self.selection_bounds() {
            let shapes = self.selection.clone();
            self.run(Command::TransformShapes {
                shapes,
                transform: tracedraw_core::Affine::translate(at - b.center()),
            });
        }
    }

    /// The pastes a browser handed over since the last frame (#1403):
    /// pictures and files of a paste event, and what Edit > Paste read
    /// from the clipboard. Called once per frame.
    pub fn take_handed_pastes(&mut self) {
        let handed: Vec<Handed> = HANDED.with(|h| std::mem::take(&mut *h.borrow_mut()));
        for h in handed {
            self.paste_handed(h);
        }
    }

    /// Paste what a browser handed over: a picture alone as a bitmap,
    /// other files imported, text as for a paste here.
    pub fn paste_handed(&mut self, h: Handed) {
        let before = self.selection.clone();
        if self.editing_text() {
            // Into the text at the caret; pictures do not go into text.
            if h.from_menu {
                self.text_paste_text(h.text.filter(|t| !t.is_empty()));
            }
            return;
        }
        if h.refused {
            self.status = tr("status.clipboard_refused");
        }
        let image = lone_picture(&h.files);
        let mut stored = Vec::new();
        if image.is_none() {
            for (i, f) in h.files.iter().enumerate() {
                let name = if f.name.trim().is_empty() {
                    format!("pasted-{i}")
                } else {
                    f.name.replace(['/', '\\'], "_")
                };
                let path = std::path::PathBuf::from("/upload/pasted").join(name);
                crate::files::store(&path, f.bytes.clone());
                stored.push(path);
            }
        }
        let text = h.text.filter(|t| !t.is_empty());
        if h.from_menu {
            // The clipboard may still hold the copy made here.
            self.paste_with(
                None,
                || ClipboardContent {
                    text,
                    image: None,
                    files: stored.clone(),
                    vector: None,
                },
                || image,
            );
        } else {
            self.paste_from_unmarked(ClipboardContent {
                text,
                image,
                files: stored.clone(),
                vector: None,
            });
        }
        for path in &stored {
            crate::files::forget(path);
        }
        self.move_pasted_to(&before, h.at);
    }

    /// Ctrl+V, with the text of the paste event when there was one (a
    /// browser offers the clipboard's text only that way).
    pub fn paste_with_event_text(&mut self, text: Option<String>) {
        // While a text is being edited: into the text at the caret
        // (#1377).
        if self.editing_text() {
            self.text_paste(text);
            return;
        }
        // Pictures or files of the paste event are being read (browser):
        // they are pasted when they arrive, not the copy here (#1403).
        if handed_awaited() {
            return;
        }
        if crate::files::WEB {
            let content = ClipboardContent {
                text: text.filter(|t| !t.is_empty()),
                ..Default::default()
            };
            self.paste_with(None, || content, || None);
        } else {
            self.paste_with(
                change_count(),
                read_system_text_and_files,
                read_system_image,
            );
        }
    }

    /// Paste with the clipboard at change count `seq`: `read` gives its
    /// text and files, `read_image` decodes its picture, called only when
    /// the change check needs it or it is what gets pasted (no files, no
    /// SVG), and at most once (#183).
    fn paste_with(
        &mut self,
        seq: Option<u32>,
        read: impl FnOnce() -> ClipboardContent,
        read_image: impl FnOnce() -> Option<image::RgbaImage>,
    ) {
        // Nothing copied anywhere since the copy here: nothing to read.
        if self
            .clipboard
            .as_ref()
            .is_some_and(|cb| seq.is_some() && cb.system.seq == seq)
        {
            self.paste();
            return;
        }
        let mut content = read();
        let mut read_image = Some(read_image);
        let mut take_image = || read_image.take().and_then(|f| f());
        if let Some(cb) = &self.clipboard {
            let held = cb
                .system
                .still_holds(seq, &content.text, &content.files, || {
                    content.image = take_image();
                    content.image.as_ref().map(image_sample)
                });
            if held {
                self.paste();
                return;
            }
        }
        // Vector data a drawing program copied with a picture of it:
        // objects, not the picture; the picture when the data cannot be
        // read (#2514).
        if content.files.is_empty() {
            if let Some(bytes) = content.vector.take() {
                if self.paste_vector(&bytes) {
                    return;
                }
            }
        }
        if content.image.is_none() && content.files.is_empty() && !content.is_svg() {
            content.image = take_image();
        }
        self.paste_from_unmarked(content);
    }

    /// [`App::paste_with`] on `content` (the tests' clipboard).
    #[cfg(test)]
    pub fn paste_from(&mut self, content: ClipboardContent) {
        let ClipboardContent {
            text,
            image,
            files,
            vector,
        } = content;
        self.paste_with(
            None,
            || ClipboardContent {
                text,
                image: None,
                files,
                vector,
            },
            || image,
        );
    }

    /// [`App::paste_in_view`] with `content` on the system clipboard (the
    /// tests' clipboard).
    #[cfg(test)]
    pub fn paste_in_view_from(&mut self, content: ClipboardContent) {
        let before = self.selection.clone();
        let at = self.view_center();
        self.paste_from(content);
        self.move_pasted_to(&before, Some(at));
    }

    /// What the system clipboard offers, or the copy here when it offers
    /// nothing.
    fn paste_from_unmarked(&mut self, content: ClipboardContent) {
        if content.is_empty() && self.clipboard.is_some() {
            self.paste();
            return;
        }
        // Characters copied while editing a text, where the clipboard
        // cannot be read (a browser's menu Paste): a text of them.
        if content.is_empty() {
            if let Some(plain) = self.text_clip.as_ref().map(|c| c.plain.clone()) {
                self.paste_as_text(&plain);
            }
            return;
        }
        if !content.files.is_empty() {
            for path in &content.files {
                self.import_path(path);
            }
            return;
        }
        if content.is_svg() {
            if let Some(t) = content.text {
                self.paste_as_svg(&t);
            }
        } else if let Some(img) = content.image {
            self.paste_as_bitmap(img);
        } else if let Some(t) = content.text {
            self.paste_as_text(&t);
        }
    }

    // ----- external bitmap editing -------------------------------------------------

    fn bitmap_link(&self, id: ShapeId) -> Option<String> {
        self.doc()
            .find_shape(id)?
            .data
            .iter()
            .find(|(k, _)| k == "bitmap.link")
            .map(|(_, v)| v.clone())
    }

    pub fn selected_bitmap(&self) -> Option<ShapeId> {
        let id = *self.selection.first()?;
        let s = self.doc().find_shape(id)?;
        matches!(s.kind, ShapeKind::Bitmap { .. }).then_some(id)
    }

    pub fn selected_bitmap_is_linked(&self) -> bool {
        self.selected_bitmap()
            .and_then(|id| self.bitmap_link(id))
            .is_some()
    }

    /// The selected bitmap's PNG, its effects included.
    #[cfg(test)]
    pub fn selected_bitmap_png(&self) -> Option<tracedraw_core::SharedBytes> {
        let id = self.selected_bitmap()?;
        match &self.doc().find_shape(id)?.kind {
            ShapeKind::Bitmap { png, .. } => Some(png.clone()),
            _ => None,
        }
    }

    /// Save Bitmap As (the bitmap's context menu): the picture as PNG,
    /// JPEG or TIFF (#1834), the kind of the file it came from offered
    /// first. A CMYK picture saved as JPEG or TIFF keeps its inks; see
    /// [`crate::bitmap_file`].
    pub fn save_bitmap_as(&mut self) {
        let Some(id) = self.selected_bitmap() else {
            return;
        };
        let Some(own) = self.doc().find_shape(id).map(crate::bitmap_file::own_kind) else {
            return;
        };
        let name = format!("{}.{}", self.document_name(), own.ext());
        let mut dialog = crate::files::Dialog::new();
        let others = crate::bitmap_file::BitmapFile::ALL
            .into_iter()
            .filter(|k| *k != own);
        for kind in std::iter::once(own).chain(others) {
            dialog = dialog.add_filter(kind.filter_name(), kind.exts());
        }
        dialog
            .set_file_name(name)
            .save_file(self, move |app, path| {
                app.status = match app.save_bitmap_to(id, &path, own) {
                    Ok(()) => crate::i18n::trf(
                        "status.exported",
                        &[("path", &path.display().to_string())],
                    ),
                    Err(e) => crate::i18n::trf("status.export_failed", &[("e", &e)]),
                };
            });
    }

    /// Write picture `id` to `path` as the kind its extension names, else
    /// as `fallback`.
    pub fn save_bitmap_to(
        &self,
        id: ShapeId,
        path: &std::path::Path,
        fallback: crate::bitmap_file::BitmapFile,
    ) -> Result<(), String> {
        let s = self.doc().find_shape(id).ok_or("no picture")?;
        let kind = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::bitmap_file::BitmapFile::from_name)
            .unwrap_or(fallback);
        let icc = crate::bitmap_file::is_cmyk(s)
            .then(|| self.bitmap_cmyk_profile())
            .flatten();
        let bytes = crate::bitmap_file::write(self.doc(), s, kind, icc.as_deref())?;
        crate::files::write(path, &bytes).map_err(|e| e.to_string())
    }

    /// The CMYK profile a CMYK picture's file carries: the one its inks
    /// are converted with, the drawing's own while it is used, else the
    /// one picked under Colour Management; none with the built-in
    /// conversions.
    fn bitmap_cmyk_profile(&self) -> Option<Vec<u8>> {
        let c = &self.settings.color;
        if c.builtin_engine {
            return None;
        }
        if c.open_cmyk == 0 && crate::app::carried_cmyk_profile(self.doc()).is_some() {
            return self.doc().cmyk_profile.as_ref().map(|p| p.data.to_vec());
        }
        self.settings_cmyk_profile().map(|p| p.data.to_vec())
    }

    /// Edit Bitmap's file of picture `id` in `dir`: a CMYK TIFF for a
    /// CMYK picture, so the other program gets its inks (#1834), else a
    /// PNG. The path written.
    pub fn write_bitmap_for_editing(
        &self,
        id: ShapeId,
        dir: &std::path::Path,
    ) -> Result<std::path::PathBuf, String> {
        use crate::bitmap_file::BitmapFile;
        let s = self.doc().find_shape(id).ok_or("no picture")?;
        let kind = if crate::bitmap_file::is_cmyk(s) {
            BitmapFile::Tiff
        } else {
            BitmapFile::Png
        };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("bitmap-{}.{}", id.raw(), kind.ext()));
        self.save_bitmap_to(id, &path, kind)?;
        Ok(path)
    }

    /// Image > Edit Bitmap: write the picture to a file next to the
    /// system's temporary files (a CMYK TIFF for a CMYK picture, else a
    /// PNG), link the object to it and open the system editor (on
    /// Android, an editing app through "open with").
    pub fn edit_bitmap_externally(&mut self) {
        if crate::files::WEB {
            // A browser cannot hand a file to another program.
            self.status = tr("status.not_in_browser");
            return;
        }
        let Some(id) = self.selected_bitmap() else {
            return;
        };
        let Some(s) = self.doc().find_shape(id).cloned() else {
            return;
        };
        let dir = std::env::temp_dir().join("tracedraw-edit");
        let path = match self.write_bitmap_for_editing(id, &dir) {
            Ok(p) => p,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let mut data = s.data.clone();
        data.retain(|(k, _)| k != "bitmap.link");
        data.push(("bitmap.link".into(), path.display().to_string()));
        self.run(Command::SetObjectData { shape: id, data });
        // Android hands the picture to an editing app the user picks, with
        // the right to write it back (#1398); the file's watch reads it
        // again when it does.
        #[cfg(target_os = "android")]
        {
            let mime = if path.extension().is_some_and(|e| e == "tif") {
                "image/tiff"
            } else {
                "image/png"
            };
            self.status = if crate::android::outside::edit(&path, mime) {
                tr("status.bitmap_sent_to_app")
            } else {
                tr("status.no_app_edits_pictures")
            };
        }
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        match crate::export::open_in_editor(&self.settings.bitmap_editor, &path) {
            Ok(()) => self.status = tr("status.bitmap_sent_to_editor"),
            Err(e) => self.status = e,
        }
    }

    /// Image > Update from Link: re-read the linked file into the object.
    pub fn update_bitmap_from_link(&mut self) {
        let Some(id) = self.selected_bitmap() else {
            return;
        };
        self.reload_linked_bitmap(id);
    }

    /// Re-read the file bitmap `id` is linked to into the object (Update
    /// from Link, or the file changed on disk). False when it is not
    /// linked or the file cannot be read (the status bar says so).
    pub fn reload_linked_bitmap(&mut self, id: ShapeId) -> bool {
        let Some(path) = self.bitmap_link(id) else {
            return false;
        };
        // A package's links are relative to its drawing (#852).
        let path = crate::package::resolve_link(&path, self.link_base().as_deref())
            .display()
            .to_string();
        // A CMYK TIFF keeps its inks (#1834).
        let read = crate::files::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| crate::bitmap_file::read(&b));
        let picture = match read {
            Ok(p) => p,
            Err(e) => {
                self.status = trf("status.could_not_read", &[("p", &path), ("e", &e)]);
                return false;
            }
        };
        let Some(s) = self.doc().find_shape(id).cloned() else {
            return false;
        };
        let ShapeKind::Bitmap { rect, .. } = s.kind else {
            return false;
        };
        let (w, h) = picture.rgba.dimensions();
        let Some((png, inks)) = picture.into_png() else {
            return false;
        };
        let mut cmds = vec![Command::SetShapeKind {
            shape: id,
            kind: ShapeKind::Bitmap {
                rect,
                width_px: w,
                height_px: h,
                png: png.into(),
                fx: None,
            },
        }];
        // A CMYK file makes it a CMYK picture with those inks; other
        // pixels leave the mode as it was and drop inks that no longer
        // fit.
        if inks.is_some() || s.picture_inks.is_some() {
            cmds.push(Command::SetPictureInks {
                shape: id,
                cmyk: s.cmyk_bitmap || inks.is_some(),
                inks,
            });
        }
        if cmds.len() == 1 {
            if let Some(cmd) = cmds.pop() {
                self.run(cmd);
            }
        } else if let Err(e) = self.engine.run_batch("Update from Link", &cmds) {
            self.status = e.to_string();
            return false;
        }
        self.status = tr("status.bitmap_updated");
        true
    }

    /// Linked bitmaps (Edit Bitmap, an imported file) follow their file:
    /// once a second the files' modification times are read, and a
    /// bitmap whose file changed since it was last seen is read again, so
    /// a save in the outside editor shows here at once. Desktop only.
    /// `now` is the frame's time; `ctx` keeps a frame coming while the
    /// watch is on.
    pub fn watch_linked_bitmaps(&mut self, ctx: Option<&egui::Context>, now: web_time::Instant) {
        if crate::files::WEB {
            return;
        }
        let gap = std::time::Duration::from_secs(1);
        if let Some(last) = self.linked_checked {
            let since = now.duration_since(last);
            if since < gap {
                if let Some(ctx) = ctx {
                    if !self.linked_mtimes.is_empty() {
                        ctx.request_repaint_after(gap - since);
                    }
                }
                return;
            }
        }
        self.linked_checked = Some(now);
        let linked: Vec<(ShapeId, String)> = self
            .doc()
            .all_layers()
            .flat_map(|l| l.shapes.iter())
            .filter_map(|s| {
                s.data
                    .iter()
                    .find(|(k, _)| k == "bitmap.link")
                    .map(|(_, v)| (s.id, v.clone()))
            })
            .collect();
        self.linked_mtimes
            .retain(|id, _| linked.iter().any(|(l, _)| l == id));
        let base = self.link_base();
        for (id, path) in linked {
            let path = crate::package::resolve_link(&path, base.as_deref());
            let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
                continue;
            };
            match self.linked_mtimes.insert(id, modified) {
                Some(seen) if seen != modified => {
                    self.reload_linked_bitmap(id);
                }
                _ => {}
            }
        }
        if let Some(ctx) = ctx {
            if !self.linked_mtimes.is_empty() {
                ctx.request_repaint_after(gap);
            }
        }
    }

    /// Image > Break Link: forget the file; the pixels stay embedded.
    pub fn break_bitmap_link(&mut self) {
        let Some(id) = self.selected_bitmap() else {
            return;
        };
        let Some(s) = self.doc().find_shape(id).cloned() else {
            return;
        };
        let mut data = s.data;
        data.retain(|(k, _)| k != "bitmap.link");
        self.run(Command::SetObjectData { shape: id, data });
    }
}

/// The paste key test of the windowing layer's gate (vendor/README.md,
/// #1488).
#[cfg(all(test, not(any(target_arch = "wasm32", target_os = "android"))))]
#[path = "../../../vendor/eframe/src/native/paste_gate.rs"]
mod eframe_paste_gate;

#[cfg(test)]
mod tests {
    use super::*;

    /// The picture a copy offers (#2288) is 150 dpi, and smaller for a
    /// selection whose longer side would pass 4000 pixels; the copied
    /// objects drawn alone fill it.
    #[test]
    fn the_picture_a_copy_offers_is_the_selection_drawn() {
        assert_eq!(copy_picture_dpi(100.0, 50.0), 150.0);
        let big = copy_picture_dpi(2000.0, 100.0);
        assert!((2000.0 * big / 25.4 - 4000.0).abs() < 1.0, "{big}");
        assert_eq!(copy_picture_dpi(0.0, 0.0), 150.0);
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.8, 35.4),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        let bounds = app.selection_bounds().expect("bounds");
        let alone = app.selection_alone(bounds).expect("alone");
        let (png, w, h) = crate::ops2::render_alone_png(&alone, 100.0, true, true).expect("png");
        // 2 by 1 inches, and the outline's width round it.
        assert!(w.abs_diff(200) <= 6 && h.abs_diff(100) <= 6, "{w} x {h}");
        let img = image::load_from_memory(&png).expect("decodes").to_rgba8();
        // A new rectangle has an outline and no fill: its edges are drawn.
        assert!(img.pixels().any(|p| p[3] > 0), "the rectangle is drawn");
    }

    fn shape_count(app: &App) -> usize {
        app.doc().all_layers().map(|l| l.shapes.len()).sum()
    }

    /// A drawing with one rectangle, copied here; the system clipboard
    /// then held `system`.
    fn copied(system: &ClipboardContent) -> App {
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        app.copy();
        if let Some(cb) = app.clipboard.as_mut() {
            cb.system = system.mark(None);
        }
        app
    }

    fn svg_text() -> ClipboardContent {
        ClipboardContent {
            text: Some("<svg xmlns=\"http://www.w3.org/2000/svg\"/>".into()),
            ..Default::default()
        }
    }

    fn picture() -> ClipboardContent {
        ClipboardContent {
            image: Some(image::RgbaImage::from_pixel(
                3,
                2,
                image::Rgba([1, 2, 3, 255]),
            )),
            ..Default::default()
        }
    }

    fn is_bitmap(app: &App) -> bool {
        app.selected_shapes()
            .first()
            .is_some_and(|s| matches!(s.kind, ShapeKind::Bitmap { .. }))
    }

    #[test]
    fn the_copy_here_is_pasted_while_the_system_clipboard_still_holds_it() {
        let mut app = copied(&svg_text());
        app.paste_from(svg_text());
        assert_eq!(shape_count(&app), 2);
        assert!(matches!(
            app.selected_shapes().first().map(|s| &s.kind),
            Some(ShapeKind::Rect { .. })
        ));
        // No system clipboard (a session without one): the copy here.
        app.paste_from(ClipboardContent::default());
        assert_eq!(shape_count(&app), 3);
    }

    #[test]
    fn a_picture_copied_elsewhere_afterwards_wins_over_the_copy_here() {
        let mut app = copied(&svg_text());
        app.paste_from(picture());
        assert_eq!(shape_count(&app), 2);
        assert!(is_bitmap(&app), "the picture, not the rectangle");
        // Text copied elsewhere becomes text.
        app.paste_from(ClipboardContent {
            text: Some("Hello".into()),
            ..Default::default()
        });
        assert!(matches!(
            app.selected_shapes().first().map(|s| &s.kind),
            Some(ShapeKind::Text { .. })
        ));
    }

    /// #2514: objects copied in another vector drawing program paste as
    /// objects with their fills and outlines, read from the vector data
    /// the copy holds beside its picture (SVG, PDF or an enhanced
    /// metafile); the picture only when there is no data this reads.
    /// They pasted as one bitmap.
    #[test]
    fn vectors_another_program_copied_paste_as_objects() {
        use tracedraw_core::{Color, Fill};
        // Two red rectangles in a drawing of their own, for the PDF and
        // the metafile another program would put on the clipboard.
        let mut other = App::headless();
        for x in [10.0, 60.0] {
            let id = other
                .new_shape(ShapeKind::Rect {
                    rect: tracedraw_core::geometry::Rect::new(x, 10.0, x + 30.0, 40.0),
                    radius: 0.0,
                    corners: None,
                })
                .expect("rect");
            other.run(Command::SetFill {
                shapes: vec![id],
                fill: Fill::Solid(Color::rgb8(255, 0, 0)),
            });
        }
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50"><rect x="10" y="10" width="30" height="30" fill="#ff0000"/><rect x="60" y="10" width="30" height="30" fill="#ff0000"/></svg>"##.to_vec();
        let pdf = tracedraw_io::pdf::document_to_pdf(other.doc());
        let emf = tracedraw_io::emf::page_to_emf(other.doc(), 0);
        for (name, bytes) in [("svg", svg), ("pdf", pdf), ("emf", emf)] {
            let mut app = App::headless();
            app.paste_from(ClipboardContent {
                vector: Some(bytes),
                ..picture()
            });
            assert!(!is_bitmap(&app), "{name}: objects, not the picture");
            // The rectangles, at whatever depth the reader put them.
            fn walk(s: &tracedraw_core::document::Shape, out: &mut Vec<Fill>) {
                match &s.kind {
                    ShapeKind::Group { children } => children.iter().for_each(|c| walk(c, out)),
                    ShapeKind::ClipFrame { contents, .. } => {
                        contents.iter().for_each(|c| walk(c, out))
                    }
                    _ => out.push(s.fill.clone()),
                }
            }
            let mut fills = Vec::new();
            for s in app.doc().all_layers().flat_map(|l| &l.shapes) {
                walk(s, &mut fills);
            }
            let red = fills
                .iter()
                .filter(
                    |f| matches!(f, Fill::Solid(c) if c.to_rgb8()[0] > 200 && c.to_rgb8()[1] < 60),
                )
                .count();
            assert!(red >= 2, "{name}: two red rectangles, got {fills:?}");
            assert!(!app.selection.is_empty(), "{name}: selected");
            // One undo step takes the paste back.
            app.undo();
            assert_eq!(shape_count(&app), 0, "{name}");
        }
        // Vector data this cannot read: the picture.
        let mut app = App::headless();
        app.paste_from(ClipboardContent {
            vector: Some(b"not a drawing".to_vec()),
            ..picture()
        });
        assert!(is_bitmap(&app));
    }

    /// Paste in View with nothing copied here pastes what another program
    /// copied (a picture), centred on the visible area, as Edit > Paste
    /// does; before, it pasted only a copy made here.
    #[test]
    fn paste_in_view_pastes_what_another_program_copied() {
        let mut app = App::headless();
        app.canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        assert!(app.clipboard.is_none());
        app.paste_in_view_from(picture());
        assert_eq!(shape_count(&app), 1);
        assert!(is_bitmap(&app), "the picture from the other program");
        let b = app.selection_bounds().expect("pasted");
        let c = app.view_center();
        assert!((b.center() - c).hypot() < 1e-6, "{:?} at {c:?}", b.center());
    }

    fn png_file(name: &str) -> PastedFile {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(5, 4, image::Rgba([200, 30, 30, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("png");
        PastedFile {
            name: name.into(),
            mime: "image/png".into(),
            bytes,
        }
    }

    fn is_rect(app: &App) -> bool {
        matches!(
            app.selected_shapes().first().map(|s| &s.kind),
            Some(ShapeKind::Rect { .. })
        )
    }

    /// #1403: a picture copied in another application comes with the
    /// browser's paste event as a file, read in the background; the
    /// paste shortcut's frame pastes nothing meanwhile (not the copy
    /// here), and the picture is placed as a bitmap when it arrives.
    #[test]
    fn a_picture_pasted_in_a_browser_is_placed_as_a_bitmap() {
        let mut app = copied(&svg_text());
        expect_handed();
        app.paste_with_event_text(None);
        assert_eq!(shape_count(&app), 1, "the copy here waits");
        hand_over(Handed {
            files: vec![png_file("image.png")],
            ..Default::default()
        });
        assert!(!handed_awaited());
        app.take_handed_pastes();
        assert_eq!(shape_count(&app), 2);
        assert!(is_bitmap(&app), "the picture, not the rectangle");
        // Taken once.
        app.take_handed_pastes();
        assert_eq!(shape_count(&app), 2);
        // A file that is no picture it can read pastes nothing.
        hand_over(Handed {
            files: vec![PastedFile {
                name: "broken.png".into(),
                mime: "image/png".into(),
                bytes: vec![1, 2, 3],
            }],
            ..Default::default()
        });
        app.take_handed_pastes();
        assert_eq!(shape_count(&app), 2);
        // The next paste is no longer held back.
        app.paste_with_event_text(None);
        assert_eq!(shape_count(&app), 3);
    }

    /// #1403: Edit > Paste in a browser reads the clipboard with its API:
    /// the copy made here while the clipboard still holds it, text and
    /// pictures copied elsewhere otherwise, and the copy here (with a
    /// word in the status bar) when the browser refuses.
    #[test]
    fn edit_paste_in_a_browser_takes_what_the_clipboard_api_read() {
        let mut app = copied(&svg_text());
        let menu = |text: Option<&str>, files: Vec<PastedFile>| Handed {
            text: text.map(String::from),
            files,
            from_menu: true,
            ..Default::default()
        };
        app.paste_handed(menu(svg_text().text.as_deref(), Vec::new()));
        assert_eq!(shape_count(&app), 2);
        assert!(is_rect(&app), "the copy here, as objects");
        app.paste_handed(menu(Some("Hello"), Vec::new()));
        assert!(matches!(
            app.selected_shapes().first().map(|s| &s.kind),
            Some(ShapeKind::Text { .. })
        ));
        app.paste_handed(menu(None, vec![png_file("clipboard.png")]));
        assert!(is_bitmap(&app));
        let n = shape_count(&app);
        app.paste_handed(Handed {
            refused: true,
            from_menu: true,
            ..Default::default()
        });
        assert_eq!(shape_count(&app), n + 1);
        assert!(is_rect(&app), "refused: the copy here");
        assert_eq!(app.status, tr("status.clipboard_refused"));
    }

    /// Edit > Paste waits for the browser (which may ask the user first)
    /// without holding back the paste shortcut; a paste event's files do.
    #[test]
    fn only_a_paste_events_files_hold_back_the_shortcut() {
        expect_handed();
        hand_over(Handed {
            from_menu: true,
            ..Default::default()
        });
        assert!(handed_awaited(), "the paste event's files still come");
        hand_over(Handed::default());
        assert!(!handed_awaited());
        HANDED.with(|h| h.borrow_mut().clear());
    }

    /// The page's context menu pastes where it was opened, and moves
    /// nothing when nothing was pasted (#1403: the browser pastes on a
    /// later frame, with the place kept for then).
    #[test]
    fn a_paste_from_the_context_menu_lands_where_it_was_opened() {
        let mut app = copied(&svg_text());
        let at = Point::new(150.0, 120.0);
        app.paste_handed(Handed {
            files: vec![png_file("image.png")],
            at: Some(at),
            ..Default::default()
        });
        let b = app.selection_bounds().expect("pasted");
        assert!((b.center().x - at.x).abs() < 1e-6 && (b.center().y - at.y).abs() < 1e-6);
        // Nothing pasted (nothing copied anywhere): the selected
        // objects stay where they are.
        let mut app = copied(&svg_text());
        app.clipboard = None;
        let before = app.selection_bounds().expect("selected");
        app.paste_handed(Handed {
            at: Some(Point::new(0.0, 0.0)),
            ..Default::default()
        });
        assert_eq!(app.selection_bounds(), Some(before));
    }

    /// While a text is being edited, a picture pasted from elsewhere
    /// does not go into it, and Edit > Paste's text does.
    #[test]
    fn a_picture_pasted_while_editing_text_is_left_out() {
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Text {
                spans: vec![TextSpan::new("ab", "Sans", 24.0)],
                origin: Point::new(20.0, 20.0),
                frame: None,
                align: tracedraw_core::TextAlign::Left,
                para: Default::default(),
                on_path: None,
            })
            .expect("text");
        app.select(vec![id]);
        app.begin_text_edit(id);
        assert!(app.editing_text());
        app.paste_handed(Handed {
            files: vec![png_file("image.png")],
            ..Default::default()
        });
        assert_eq!(shape_count(&app), 1);
        app.paste_handed(Handed {
            text: Some("cd".into()),
            from_menu: true,
            ..Default::default()
        });
        let text: String = match &app.doc().find_shape(id).expect("text").kind {
            ShapeKind::Text { spans, .. } => spans.iter().map(|s| s.text.as_str()).collect(),
            _ => String::new(),
        };
        assert!(text.contains("cd"), "{text}");
    }

    /// #1403 (Android): the clipboard's documents as the activity lists
    /// them; one picture alone is the clipboard's picture, anything else
    /// is imported as files.
    #[test]
    fn android_clipboard_documents_become_a_picture_or_files() {
        let docs = parse_clip_documents(
            "content://media/external/images/1\timage/jpeg\tIMG_1.jpg\n\ncontent://x/doc\t\tplan.svg",
        );
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].mime, "image/jpeg");
        assert_eq!(docs[1].name, "plan.svg");
        let (files, picture) = split_documents(docs[..1].to_vec());
        assert!(files.is_empty());
        assert_eq!(
            picture.as_deref(),
            Some("content://media/external/images/1")
        );
        let (files, picture) = split_documents(docs.clone());
        assert!(picture.is_none());
        assert_eq!(files.len(), 2);
        assert_eq!(
            crate::files::document_uri(&files[1]).as_deref(),
            Some("content://x/doc")
        );
        assert_eq!(
            files[1].file_name().and_then(|n| n.to_str()),
            Some("plan.svg")
        );
        assert!(parse_clip_documents("").is_empty());
    }

    #[test]
    fn files_copied_in_a_file_manager_are_imported() {
        let dir = std::env::temp_dir().join(format!("tracedraw-paste-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("dot.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 120, 215, 255]))
            .save(&path)
            .expect("png");
        let mut app = copied(&svg_text());
        app.paste_from(ClipboardContent {
            files: vec![path],
            ..Default::default()
        });
        assert_eq!(shape_count(&app), 2);
        assert!(is_bitmap(&app));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn copied_file_paths_lose_the_line_end() {
        use std::path::PathBuf;
        assert_eq!(
            without_line_end(PathBuf::from("/home/a/b c.psd\r")),
            PathBuf::from("/home/a/b c.psd")
        );
        assert_eq!(
            without_line_end(PathBuf::from("/home/a.png")),
            PathBuf::from("/home/a.png")
        );
    }

    #[test]
    fn the_mark_follows_the_content() {
        let empty = ClipboardContent::default();
        assert!(empty.is_empty());
        assert_eq!(empty.mark(None), ClipboardContent::default().mark(None));
        assert_ne!(empty.mark(None), svg_text().mark(None));
        assert_ne!(svg_text().mark(None), picture().mark(None));
        assert!(!picture().is_empty());
        // Pictures of one size told apart by their pixels.
        let mut other = picture();
        if let Some(img) = other.image.as_mut() {
            img.put_pixel(2, 1, image::Rgba([9, 9, 9, 255]));
        }
        assert_ne!(picture().mark(None), other.mark(None));
    }

    /// The change check reads as little as it can (#183): the system's
    /// change count decides alone; else equal text or files say the same
    /// copy without looking at the picture, and the picture is sampled
    /// only on a clipboard with neither.
    #[test]
    fn the_change_check_decodes_a_picture_only_when_it_must() {
        let no_image = || -> Option<u64> { panic!("the picture was decoded") };
        let counted = svg_text().mark(Some(7));
        assert!(counted.still_holds(Some(7), &None, &[], no_image));
        assert!(!counted.still_holds(Some(8), &svg_text().text, &[], no_image));
        let text = svg_text().mark(None);
        assert!(text.still_holds(None, &svg_text().text, &[], no_image));
        assert!(!text.still_holds(None, &Some("other".into()), &[], no_image));
        assert!(!text.still_holds(None, &None, &[], no_image));
        let pic = picture().mark(None);
        let sample = picture().image.as_ref().map(image_sample);
        let mut decoded = 0;
        assert!(pic.still_holds(None, &None, &[], || {
            decoded += 1;
            sample
        }));
        assert!(!pic.still_holds(None, &None, &[], || {
            decoded += 1;
            None
        }));
        assert_eq!(decoded, 2);
    }

    /// Pasting the copy here never decodes the picture of a clipboard that
    /// still holds the copy's text; a picture copied elsewhere is decoded
    /// once and pasted, files are imported without decoding one (#183).
    #[test]
    fn a_paste_decodes_the_picture_only_when_it_needs_it() {
        let mut app = copied(&svg_text());
        let no_image = || -> Option<image::RgbaImage> { panic!("the picture was decoded") };
        app.paste_with(None, svg_text, no_image);
        assert_eq!(shape_count(&app), 2);
        // The system's change count unchanged: nothing is read at all.
        if let Some(cb) = app.clipboard.as_mut() {
            cb.system.seq = Some(3);
        }
        app.paste_with(Some(3), || panic!("the clipboard was read"), no_image);
        assert_eq!(shape_count(&app), 3);
        // A picture copied elsewhere since: decoded once, pasted.
        let mut decoded = 0;
        app.paste_with(Some(4), ClipboardContent::default, || {
            decoded += 1;
            picture().image
        });
        assert_eq!(decoded, 1);
        assert!(is_bitmap(&app));
        // Text written by the copy here is what a paste reads back.
        let mut app = copied(&ClipboardContent::default());
        app.mark_system_clipboard(svg_text().text.as_deref());
        app.paste_with(None, svg_text, no_image);
        assert!(matches!(
            app.selected_shapes().first().map(|s| &s.kind),
            Some(ShapeKind::Rect { .. })
        ));
    }
}

/// What copy and paste cost with a 4000 x 3000 picture on the system
/// clipboard: run in a session with a clipboard (on Linux an X server,
/// such as `Xvfb :99` with `DISPLAY=:99`) with
/// `cargo test --release -p tracedraw -- --ignored clipboard_costs --nocapture`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod clipboard_bench {
    use super::*;

    fn picture_elsewhere(cb: &mut arboard::Clipboard, shade: u8) {
        let (w, h) = (4000usize, 3000usize);
        let bytes: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8 ^ shade).collect();
        cb.set_image(arboard::ImageData {
            width: w,
            height: h,
            bytes: bytes.into(),
        })
        .expect("a clipboard");
    }

    fn time(label: &str, f: impl FnOnce()) {
        let t = std::time::Instant::now();
        f();
        eprintln!("{label}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }

    #[test]
    #[ignore]
    fn clipboard_costs() {
        let Ok(mut cb) = arboard::Clipboard::new() else {
            eprintln!("no clipboard");
            return;
        };
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(10.0, 10.0, 60.0, 40.0),
                radius: 0.0,
                corners: None,
            })
            .expect("rect");
        app.select(vec![id]);
        picture_elsewhere(&mut cb, 0);
        time("Ctrl+C over a picture copied elsewhere", || {
            app.copy_with_system()
        });
        time("Ctrl+V of that copy", || app.paste_with_event_text(None));
        picture_elsewhere(&mut cb, 1);
        app.select(vec![id]);
        app.copy();
        time("a copy of nodes or guides (mark, picture kept)", || {
            app.mark_system_clipboard(None)
        });
        time("Ctrl+V of that copy, picture still there", || {
            app.paste_with_event_text(None)
        });
        time("reading and decoding the picture alone", || {
            let _ = read_system_image();
        });
        time("the change count (Windows; none elsewhere)", || {
            let _ = change_count();
        });
        picture_elsewhere(&mut cb, 2);
        let before = app
            .doc()
            .all_layers()
            .map(|l| l.shapes.len())
            .sum::<usize>();
        time("Ctrl+V of a picture copied elsewhere since", || {
            app.paste_with_event_text(None)
        });
        let after = app
            .doc()
            .all_layers()
            .map(|l| l.shapes.len())
            .sum::<usize>();
        assert_eq!(after, before + 1);
    }
}

#[cfg(test)]
mod linked_watch_tests {
    use crate::app::App;
    use tracedraw_core::geometry::Rect;
    use tracedraw_core::{Command, ShapeKind};

    fn png(w: u32, h: u32) -> Vec<u8> {
        tiny_skia::Pixmap::new(w, h)
            .and_then(|p| p.encode_png().ok())
            .unwrap_or_default()
    }

    /// A linked bitmap follows its file: a change on disk is read in on
    /// the next check (once a second), as Update from Link does; a file
    /// seen for the first time is only remembered (#284).
    #[test]
    fn a_linked_bitmap_is_read_again_when_its_file_changes() {
        if crate::files::WEB {
            return;
        }
        let dir = std::env::temp_dir().join(format!("tracedraw-link-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("picture.png");
        std::fs::write(&path, png(2, 2)).expect("png");
        let mut app = App::headless();
        let id = app
            .new_shape(ShapeKind::Bitmap {
                rect: Rect::new(10.0, 10.0, 30.0, 30.0),
                width_px: 2,
                height_px: 2,
                png: png(2, 2).into(),
                fx: None,
            })
            .expect("bitmap");
        app.run(Command::SetObjectData {
            shape: id,
            data: vec![("bitmap.link".into(), path.display().to_string())],
        });
        let width = |app: &App| match &app.doc().find_shape(id).expect("bitmap").kind {
            ShapeKind::Bitmap { width_px, .. } => *width_px,
            _ => 0,
        };
        let t0 = web_time::Instant::now();
        app.watch_linked_bitmaps(None, t0);
        assert_eq!(width(&app), 2, "first sight: remembered, not read");
        // The file changes (its time stamp moved on by two seconds).
        std::fs::write(&path, png(3, 3)).expect("png");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(later))
            .expect("mtime");
        // Within the second: not looked at yet.
        app.watch_linked_bitmaps(None, t0 + std::time::Duration::from_millis(500));
        assert_eq!(width(&app), 2);
        app.watch_linked_bitmaps(None, t0 + std::time::Duration::from_millis(1100));
        assert_eq!(width(&app), 3, "read again");
        // Unchanged since: left alone (no new undo step).
        let depth = app.engine.history_labels().0.len();
        app.watch_linked_bitmaps(None, t0 + std::time::Duration::from_millis(2200));
        assert_eq!(app.engine.history_labels().0.len(), depth);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

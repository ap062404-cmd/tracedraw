//! Vector data on the Windows clipboard (#2514). A drawing program
//! copies its objects in a vector format beside a picture of them: SVG
//! under the registered name `image/svg+xml`, PDF under `Portable
//! Document Format` (also `PDF` or `application/pdf`), or an enhanced
//! metafile (`CF_ENHMETAFILE`). The clipboard library reads text, files
//! and the picture only, so the paste came in as a bitmap; this reads the
//! first of those formats the clipboard offers, in that order.

use std::ffi::c_void;

// 32-bit Windows: the DLLs export the plain names, not the stdcall
// decorated ones raw-dylib imports by default (#1975).
#[cfg_attr(
    target_arch = "x86",
    link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated")
)]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn OpenClipboard(owner: *mut c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
}

#[cfg_attr(
    target_arch = "x86",
    link(
        name = "kernel32",
        kind = "raw-dylib",
        import_name_type = "undecorated"
    )
)]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GlobalLock(mem: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(mem: *mut c_void) -> i32;
    fn GlobalSize(mem: *mut c_void) -> usize;
}

#[cfg_attr(
    target_arch = "x86",
    link(name = "gdi32", kind = "raw-dylib", import_name_type = "undecorated")
)]
#[cfg_attr(not(target_arch = "x86"), link(name = "gdi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetEnhMetaFileBits(emf: *mut c_void, size: u32, data: *mut u8) -> u32;
}

/// The enhanced metafile format number.
const CF_ENHMETAFILE: u32 = 14;

/// The registered names of the SVG and PDF formats, in the order a paste
/// prefers them.
const NAMED_FORMATS: [&str; 5] = [
    "image/svg+xml",
    "Portable Document Format",
    "PDF",
    "application/pdf",
    "image/svg",
];

/// The largest copy read (bytes): more is not a drawing's copy.
const MAX_BYTES: usize = 256 * 1024 * 1024;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The clipboard, opened for this thread until dropped.
struct Open;

impl Open {
    /// Another program may hold the clipboard for a moment: a few tries.
    fn new() -> Option<Open> {
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // SAFETY: a null owner opens the clipboard for this thread.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Some(Open);
            }
        }
        None
    }

    /// The bytes of a memory block format.
    fn global_bytes(&self, format: u32) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open; the handle stays valid until it
        // closes and is only read between GlobalLock and GlobalUnlock,
        // within the size GlobalSize gives.
        unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                return None;
            }
            let mem = GetClipboardData(format);
            if mem.is_null() {
                return None;
            }
            let size = GlobalSize(mem);
            if size == 0 || size > MAX_BYTES {
                return None;
            }
            let ptr = GlobalLock(mem) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            GlobalUnlock(mem);
            Some(bytes)
        }
    }

    /// The enhanced metafile's bytes.
    fn metafile_bytes(&self) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open and owns the metafile handle; it
        // is only read, first for its size, then into a buffer that size.
        unsafe {
            if IsClipboardFormatAvailable(CF_ENHMETAFILE) == 0 {
                return None;
            }
            let emf = GetClipboardData(CF_ENHMETAFILE);
            if emf.is_null() {
                return None;
            }
            let size = GetEnhMetaFileBits(emf, 0, std::ptr::null_mut()) as usize;
            if size == 0 || size > MAX_BYTES {
                return None;
            }
            let mut bytes = vec![0u8; size];
            let got = GetEnhMetaFileBits(emf, size as u32, bytes.as_mut_ptr()) as usize;
            (got == size).then_some(bytes)
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard.
        unsafe {
            CloseClipboard();
        }
    }
}

/// A memory block may be larger than what was put in it: a PDF ends at
/// its last `%%EOF` line, an SVG text at its first NUL.
fn trimmed(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.starts_with(b"%PDF") {
        if let Some(i) = bytes.windows(5).rposition(|w| w == b"%%EOF") {
            let mut end = i + 5;
            while end < bytes.len() && matches!(bytes[end], b'\r' | b'\n') {
                end += 1;
            }
            bytes.truncate(end);
        }
    } else if let Some(i) = bytes.iter().position(|b| *b == 0) {
        bytes.truncate(i);
    }
    bytes
}

/// SVG, PDF or enhanced metafile bytes on the clipboard, the first
/// offered in that order; `None` when it offers none of them.
pub fn read_vector() -> Option<Vec<u8>> {
    let open = Open::new()?;
    for name in NAMED_FORMATS {
        let w = wide(name);
        // SAFETY: a NUL-terminated name; registering an existing name
        // gives its number.
        let format = unsafe { RegisterClipboardFormatW(w.as_ptr()) };
        if format == 0 {
            continue;
        }
        if let Some(bytes) = open.global_bytes(format).map(trimmed) {
            if !bytes.is_empty() {
                return Some(bytes);
            }
        }
    }
    open.metafile_bytes()
}

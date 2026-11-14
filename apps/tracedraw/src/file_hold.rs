//! Open drawings keep their files (#2130). On Windows a file can be
//! deleted, renamed or moved in the Explorer while a program has it
//! open, unless the program keeps a handle on it that does not share
//! that right. TraceDraw reads a drawing into memory and closes the
//! file, so nothing stopped the Explorer from deleting a drawing that
//! was open. Here every open drawing's file is kept open for reading,
//! shared with readers and writers but not with a delete or a rename:
//! the system then refuses those with its "file is open in TraceDraw"
//! message, while Save (which writes the file in place) and other
//! programs reading it go on as before. The hold ends when the drawing
//! is closed or saved under another name, and with the process.
//!
//! Other systems have no such hold: a file deleted there stays readable
//! by whoever has it open and the name is simply gone, so nothing is
//! kept open.
//!
//! This file uses the standard library only: `scripts/windows/spell-check`
//! compiles it alone to run its tests on a Windows runner.

use std::path::{Path, PathBuf};

/// A file kept open so that it cannot be deleted or renamed.
#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Hold(#[allow(dead_code)] std::fs::File);

/// Keep `path` open for reading, sharing reads and writes only. `None`
/// when the file cannot be opened (it is gone, a folder, not ours to
/// read) and on systems without such a hold.
#[cfg(windows)]
pub fn hold(path: &Path) -> Option<Hold> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_SHARE_READ | FILE_SHARE_WRITE, without FILE_SHARE_DELETE.
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_WRITE)
        .open(path)
        .ok()
        .map(Hold)
}

/// Keep `path` open: nothing on this system (see the module's notes).
#[cfg(not(windows))]
pub fn hold(_path: &Path) -> Option<Hold> {
    None
}

/// A hold for tests on any system: the file open for reading, when it
/// is there.
#[cfg(test)]
#[allow(dead_code)]
pub fn reading(path: &Path) -> Option<Hold> {
    std::fs::File::open(path).ok().map(Hold)
}

/// The holds on the files of the open drawings, made by `open`.
pub struct Holds<H = Hold> {
    open: fn(&Path) -> Option<H>,
    /// Each open drawing's file and its hold (`None`: it could not be
    /// held; not tried again until the drawing is saved).
    held: Vec<(PathBuf, Option<H>)>,
}

impl Default for Holds<Hold> {
    fn default() -> Self {
        Holds::new(hold)
    }
}

impl<H> Holds<H> {
    pub fn new(open: fn(&Path) -> Option<H>) -> Self {
        Holds {
            open,
            held: Vec::new(),
        }
    }

    /// Hold exactly `files`, the files of the open drawings: a file no
    /// longer among them is let go, a new one is opened, and the others
    /// keep the hold they have. Called every frame: with nothing changed
    /// it only compares the paths.
    pub fn sync<'a>(&mut self, files: impl Iterator<Item = &'a Path> + Clone) {
        let mut wanted = files.clone();
        let same = self
            .held
            .iter()
            .all(|(p, _)| wanted.next() == Some(p.as_path()))
            && wanted.next().is_none();
        if same {
            return;
        }
        let mut old = std::mem::take(&mut self.held);
        for path in files {
            if self.held.iter().any(|(p, _)| p == path) {
                continue;
            }
            let kept = old
                .iter()
                .position(|(p, _)| p == path)
                .map(|i| old.swap_remove(i));
            let entry = match kept {
                Some(entry) => entry,
                None => (path.to_path_buf(), (self.open)(path)),
            };
            self.held.push(entry);
        }
    }

    /// `path` was just written (Save, Save As): a file that could not be
    /// held before, because it was not there yet, is tried again at the
    /// next [`sync`](Self::sync).
    pub fn written(&mut self, path: &Path) {
        self.held.retain(|(p, h)| h.is_some() || p != path);
    }

    /// Whether `path` is held.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn holds(&self, path: &Path) -> bool {
        self.held.iter().any(|(p, h)| p == path && h.is_some())
    }

    /// How many files are kept track of, held or not.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.held.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A hold that counts itself while it lives.
    struct Counted;
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static OPENED: AtomicUsize = AtomicUsize::new(0);
    impl Drop for Counted {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    fn counted(path: &Path) -> Option<Counted> {
        // A name starting with "gone" stands for a file that is not there.
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("gone"))
        {
            return None;
        }
        OPENED.fetch_add(1, Ordering::SeqCst);
        LIVE.fetch_add(1, Ordering::SeqCst);
        Some(Counted)
    }

    /// The holds follow the open drawings: one for each file, kept while
    /// the drawing stays open, let go when it is closed or saved under
    /// another name, and nothing opened again while nothing changes.
    #[test]
    fn holds_follow_the_open_drawings() {
        let (a, b, c) = (
            Path::new("/x/a.tdraw"),
            Path::new("/x/b.cdr"),
            Path::new("/x/c.tdraw"),
        );
        let mut holds = Holds::new(counted);
        holds.sync([a, b].into_iter());
        assert!(holds.holds(a) && holds.holds(b));
        assert_eq!(
            (LIVE.load(Ordering::SeqCst), OPENED.load(Ordering::SeqCst)),
            (2, 2)
        );
        // Frames with the same drawings open nothing.
        for _ in 0..3 {
            holds.sync([a, b].into_iter());
        }
        assert_eq!(OPENED.load(Ordering::SeqCst), 2);
        // Another tab in front: the same files in another order.
        holds.sync([b, a].into_iter());
        assert_eq!(
            (LIVE.load(Ordering::SeqCst), OPENED.load(Ordering::SeqCst)),
            (2, 2)
        );
        // One closed, one saved under another name.
        holds.sync([c].into_iter());
        assert!(!holds.holds(a) && !holds.holds(b) && holds.holds(c));
        assert_eq!(
            (LIVE.load(Ordering::SeqCst), OPENED.load(Ordering::SeqCst)),
            (1, 3)
        );
        // Two windows of one drawing hold its file once.
        holds.sync([c, c].into_iter());
        assert_eq!((holds.len(), LIVE.load(Ordering::SeqCst)), (1, 1));
        // A file that is not there is not asked for at every frame, and is
        // tried again once the drawing is written.
        let gone = Path::new("/x/gone.tdraw");
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        fn asking(path: &Path) -> Option<Counted> {
            ASKED.fetch_add(1, Ordering::SeqCst);
            counted(path)
        }
        let mut other = Holds::new(asking);
        for _ in 0..3 {
            other.sync([gone].into_iter());
        }
        assert_eq!(ASKED.load(Ordering::SeqCst), 1);
        assert!(!other.holds(gone));
        other.written(gone);
        other.sync([gone].into_iter());
        assert_eq!(ASKED.load(Ordering::SeqCst), 2);
        // Writing a held file keeps its hold.
        holds.written(c);
        assert!(holds.holds(c));
        // No drawing open: nothing held.
        holds.sync(std::iter::empty());
        assert_eq!(LIVE.load(Ordering::SeqCst), 0);
    }

    /// On Windows a held file cannot be deleted, renamed or replaced by
    /// another file, yet is read and written in place (Save), and is free
    /// again once the hold is let go.
    #[cfg(windows)]
    #[test]
    fn a_held_file_cannot_be_deleted_or_renamed_but_is_saved() {
        let dir = std::env::temp_dir().join(format!("tracedraw-hold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        let file = dir.join("drawing.tdraw");
        let moved = dir.join("moved.tdraw");
        std::fs::write(&file, b"one").expect("written");
        let mut holds = Holds::default();
        holds.sync([file.as_path()].into_iter());
        assert!(holds.holds(&file));
        assert!(std::fs::remove_file(&file).is_err(), "deleted while open");
        assert!(
            std::fs::rename(&file, &moved).is_err(),
            "renamed while open"
        );
        std::fs::write(&moved, b"other").expect("written");
        assert!(
            std::fs::rename(&moved, &file).is_err(),
            "replaced while open"
        );
        assert!(file.is_file());
        // Save writes the file in place; anyone may read it.
        std::fs::write(&file, b"two, longer").expect("saved while held");
        assert_eq!(std::fs::read(&file).expect("read"), b"two, longer");
        std::fs::File::create(&file).expect("truncated while held");
        // The drawing closed: the file is anyone's again.
        holds.sync(std::iter::empty());
        std::fs::rename(&file, dir.join("renamed.tdraw")).expect("renamed once closed");
        std::fs::remove_file(dir.join("renamed.tdraw")).expect("deleted once closed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Elsewhere nothing is kept open.
    #[cfg(not(windows))]
    #[test]
    fn other_systems_keep_no_file_open() {
        let mut holds = Holds::default();
        let file = std::env::temp_dir().join("tracedraw-hold-none.tdraw");
        std::fs::write(&file, b"x").expect("written");
        holds.sync([file.as_path()].into_iter());
        assert!(!holds.holds(&file));
        std::fs::remove_file(&file).expect("deleted");
    }
}

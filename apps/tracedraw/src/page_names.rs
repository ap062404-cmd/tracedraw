//! Default page names follow the order of the pages (#2374, #2355).
//!
//! A page nobody named is called "Page 3" after the place it was made
//! at. When pages are added in between, moved or deleted, such names are
//! given again from the first page on, so the tabs always read Page 1,
//! Page 2, Page 3 from left to right. A page with a name of its own
//! keeps it.

use crate::app::App;
use tracedraw_core::{Command, Document};

/// The patterns a default page name is written in: the interface
/// language's, and the English one that files read without a name get.
fn patterns() -> Vec<String> {
    let mut all = vec![crate::i18n::tr("doc.page_n")];
    let english = "Page {n}".to_string();
    if !all.contains(&english) {
        all.push(english);
    }
    all
}

/// The number of `name` when it is written as `pattern` ("Page {n}").
fn number_in(name: &str, pattern: &str) -> Option<usize> {
    let (before, after) = pattern.split_once("{n}")?;
    let digits = name.strip_prefix(before)?.strip_suffix(after)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Whether `cmd` changes which page stands where.
pub fn changes_page_order(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::AddPage { .. }
            | Command::InsertPage { .. }
            | Command::DeletePage { .. }
            | Command::MovePage { .. }
            | Command::DuplicatePage { .. }
    )
}

/// The renames that make every default page name say where its page
/// stands.
pub fn renames(doc: &Document) -> Vec<Command> {
    let patterns = patterns();
    let mut out = Vec::new();
    for (i, page) in doc.pages.iter().enumerate() {
        let found = patterns
            .iter()
            .find_map(|p| number_in(&page.name, p).map(|n| (p, n)));
        if let Some((pattern, n)) = found {
            if n != i + 1 {
                out.push(Command::RenamePage {
                    page: page.id,
                    name: pattern.replace("{n}", &(i + 1).to_string()),
                });
            }
        }
    }
    out
}

impl App {
    /// Give the default page names again after the pages changed places,
    /// as part of the step that changed them (one Undo takes both back).
    pub fn renumber_default_pages(&mut self) {
        let cmds = renames(self.doc());
        if cmds.is_empty() {
            return;
        }
        if let Err(e) = self.engine.amend_batch("Rename Page", &cmds) {
            self.status = e.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(app: &App) -> Vec<String> {
        app.doc().pages.iter().map(|p| p.name.clone()).collect()
    }

    fn four_pages() -> App {
        crate::i18n::set_language("en");
        let mut app = App::headless();
        for _ in 0..3 {
            app.add_page();
        }
        assert_eq!(names(&app), ["Page 1", "Page 2", "Page 3", "Page 4"]);
        app
    }

    /// #2374: the second page moved to the front is Page 1 and the page
    /// it passed is Page 2; one Undo puts the pages and their names back.
    #[test]
    fn a_moved_page_takes_the_name_of_its_place() {
        let mut app = four_pages();
        let ids: Vec<_> = app.doc().pages.iter().map(|p| p.id).collect();
        let steps = app.engine.history_labels().0.len();
        app.run(Command::MovePage {
            page: ids[1],
            to: 0,
        });
        let order: Vec<_> = app.doc().pages.iter().map(|p| p.id).collect();
        assert_eq!(order, [ids[1], ids[0], ids[2], ids[3]]);
        assert_eq!(names(&app), ["Page 1", "Page 2", "Page 3", "Page 4"]);
        assert_eq!(app.engine.history_labels().0.len(), steps + 1);
        app.undo();
        let order: Vec<_> = app.doc().pages.iter().map(|p| p.id).collect();
        assert_eq!(order, ids);
        assert_eq!(names(&app), ["Page 1", "Page 2", "Page 3", "Page 4"]);
    }

    /// #2355: pages inserted before the current one read in rising
    /// order, the new page first.
    #[test]
    fn pages_inserted_in_front_are_numbered_from_the_left() {
        crate::i18n::set_language("en");
        let mut app = App::headless();
        for _ in 0..5 {
            app.goto_page(0);
            app.insert_page(false);
        }
        assert_eq!(
            names(&app),
            ["Page 1", "Page 2", "Page 3", "Page 4", "Page 5", "Page 6"]
        );
    }

    /// Deleting a page closes the gap in the numbers, with the Delete
    /// Page dialog's range too.
    #[test]
    fn deleting_pages_closes_the_gap() {
        let mut app = four_pages();
        app.goto_page(1);
        app.delete_page();
        assert_eq!(names(&app), ["Page 1", "Page 2", "Page 3"]);
        app.delete_pages(0, 1);
        assert_eq!(names(&app), ["Page 1"]);
    }

    /// A page with a name of its own keeps it wherever it goes; the
    /// others count it as a page.
    #[test]
    fn a_named_page_keeps_its_name() {
        let mut app = four_pages();
        let ids: Vec<_> = app.doc().pages.iter().map(|p| p.id).collect();
        app.run(Command::RenamePage {
            page: ids[3],
            name: "Cover".into(),
        });
        app.run(Command::MovePage {
            page: ids[3],
            to: 0,
        });
        assert_eq!(names(&app), ["Cover", "Page 2", "Page 3", "Page 4"]);
        app.run(Command::RenamePage {
            page: ids[0],
            name: "Page 12b".into(),
        });
        app.run(Command::MovePage {
            page: ids[0],
            to: 3,
        });
        assert_eq!(names(&app), ["Cover", "Page 2", "Page 3", "Page 12b"]);
    }

    /// The names are read in the interface's language and in English,
    /// and each keeps its language.
    #[test]
    fn default_names_are_read_in_the_interface_language() {
        let got = crate::i18n::with_thread_language("de", || {
            let mut app = App::headless();
            app.add_page();
            app.add_page();
            let ids: Vec<_> = app.doc().pages.iter().map(|p| p.id).collect();
            app.run(Command::RenamePage {
                page: ids[2],
                name: "Page 3".into(),
            });
            assert_eq!(names(&app), ["Seite 1", "Seite 2", "Page 3"]);
            app.run(Command::MovePage {
                page: ids[2],
                to: 0,
            });
            names(&app)
        });
        assert_eq!(got, ["Page 1", "Seite 2", "Seite 3"]);
    }

    #[test]
    fn only_whole_default_names_count() {
        assert_eq!(number_in("Page 12", "Page {n}"), Some(12));
        assert_eq!(number_in("Page 12b", "Page {n}"), None);
        assert_eq!(number_in("Page ", "Page {n}"), None);
        assert_eq!(number_in("Page 2 (copy)", "Page {n}"), None);
        assert_eq!(number_in("2. Seite", "{n}. Seite"), Some(2));
    }
}

//! The letters of the glyphs of a Devanagari font a PDF embeds, learned
//! from the same font installed on the system (#1806).
//!
//! A PDF from an office word processor shows Hindi as glyph numbers of
//! the font and gives their characters in a `ToUnicode` map that is often
//! wrong for conjuncts, half forms, the reph and the vowel signs (see
//! `pdf_import::unscramble_tounicode`), and that holds nothing at all for
//! a glyph whose characters it gave to another one. The embedded font is
//! a subset without the tables that say which letters make a glyph. But
//! the subset keeps the glyph numbers of the font it was cut from, and
//! that font is usually installed where the file is opened (the fonts an
//! operating system brings for Hindi). So the installed font is asked:
//! every letter, every letter with every sign, every pair of letters
//! joined by a virama, the reph over every letter and the signs that
//! join over a letter are laid out, and each glyph that comes out is
//! given the text that made it. No guess is involved: a glyph's text is
//! a text the font draws with that glyph.
//!
//! The installed font is used only when it is the font the file embeds:
//! the same number of glyphs and units per em, and for the glyphs the
//! file uses the same advance and the same bounding box.
//!
//! What a font taught is kept for the next file (a few thousand short
//! texts are laid out the first time).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tracedraw_core::document::text_outline::{self, GlyphFont, TextRequest};
use tracedraw_core::{ParagraphStyle, TextAlign, TextSpan};

const VIRAMA: char = '\u{094D}';
const NUKTA: char = '\u{093C}';
const SIGN_I: char = '\u{093F}';
const REPH: &str = "\u{0930}\u{094D}";
const DOTTED_CIRCLE: char = '\u{25CC}';

/// Probes laid out in one request; more would make one very long line.
const BATCH: usize = 400;
/// At most this many conjuncts are tried with a third letter.
const MAX_LIGATURES: usize = 400;
/// At most this many of the file's glyphs are compared with the
/// installed font's.
const MAX_COMPARED: usize = 64;

/// What a probe is, for reading its glyphs.
enum Probe {
    /// One character alone.
    Single(char),
    /// A consonant (index into the units) alone.
    Unit(usize),
    /// A consonant and a sign after it.
    Mark(usize, char),
    /// Two consonants joined by a virama.
    Pair(usize, usize),
    /// A conjunct and a consonant joined by a virama.
    After(String, usize),
    /// A consonant and a conjunct joined by a virama.
    Before,
    /// A cluster and the short i sign: the sign's glyph comes first.
    WithI,
    /// The reph over a consonant.
    Reph(usize),
    /// Signs on a consonant (index into the units): the vowel sign, the
    /// reph, the nasal sign.
    Signs(usize, Vec<String>),
}

/// What an installed font taught.
#[derive(Default, Clone)]
struct Learned {
    /// The text of each glyph.
    known: HashMap<u16, String>,
    /// The glyphs of each consonant alone.
    bases: Vec<Vec<u16>>,
    /// The conjuncts of two letters the font has a glyph for.
    ligatures: Vec<String>,
    /// The dotted circle a shaper puts under a sign with no letter.
    dotted: Option<u16>,
    /// Conjuncts of three letters were tried too.
    deep: bool,
}

/// What the fonts asked so far taught, by the font's data and face.
type Cache = Mutex<Vec<((usize, usize, u32), Arc<Learned>)>>;
static LEARNED: Cache = Mutex::new(Vec::new());

/// The glyphs of `text` in `family`, in drawn order, with the font they
/// come from; `None` without a text engine.
fn shape(
    family: &str,
    bold: bool,
    italic: bool,
    text: &str,
) -> Option<Vec<(u16, String, GlyphFont)>> {
    let spans = [TextSpan {
        bold,
        italic,
        ..TextSpan::new(text, family, 12.0)
    }];
    let para = ParagraphStyle::default();
    let layout = text_outline::glyphs(&TextRequest {
        spans: &spans,
        frame: None,
        align: TextAlign::Left,
        para: &para,
        on_path: None,
    })?;
    Some(
        layout
            .glyphs
            .into_iter()
            .map(|g| (g.glyph_id, g.text, g.font))
            .collect(),
    )
}

/// Whether the installed `font` is the font `embedded` was cut from, as
/// far as the glyphs `used` tell: the same glyph count and units per em,
/// and each used glyph the same advance and bounding box.
fn same_font(embedded: &ttf_parser::Face, font: &GlyphFont, used: &[u16]) -> bool {
    let Ok(installed) = ttf_parser::Face::parse(&font.data, font.index) else {
        return false;
    };
    if installed.number_of_glyphs() != embedded.number_of_glyphs()
        || installed.units_per_em() != embedded.units_per_em()
    {
        return false;
    }
    let mut compared = 0;
    for &g in used.iter().take(MAX_COMPARED) {
        let id = ttf_parser::GlyphId(g);
        let (a, b) = (
            embedded.glyph_bounding_box(id),
            installed.glyph_bounding_box(id),
        );
        let boxes = match (a, b) {
            (Some(a), Some(b)) => {
                (a.x_min, a.y_min, a.x_max, a.y_max) == (b.x_min, b.y_min, b.x_max, b.y_max)
            }
            (None, None) => true,
            _ => false,
        };
        if !boxes || embedded.glyph_hor_advance(id) != installed.glyph_hor_advance(id) {
            return false;
        }
        compared += usize::from(a.is_some());
    }
    // Something with an outline must have matched.
    compared > 0
}

/// The consonants probed: the letters, and those written with a nukta.
fn units() -> Vec<String> {
    let mut out: Vec<String> = ('\u{0915}'..='\u{0939}').map(String::from).collect();
    for c in "\u{0915}\u{0916}\u{0917}\u{091C}\u{0921}\u{0922}\u{092B}\u{092F}".chars() {
        out.push(format!("{c}{NUKTA}"));
    }
    out
}

/// The installed font the probes are laid out in.
struct Installed<'a> {
    family: &'a str,
    bold: bool,
    italic: bool,
    font: GlyphFont,
}

impl Installed<'_> {
    /// The glyph has no outline and no advance.
    fn blank(&self, glyph: u16) -> bool {
        let id = ttf_parser::GlyphId(glyph);
        ttf_parser::Face::parse(&self.font.data, self.font.index).is_ok_and(|f| {
            glyph != 0
                && f.glyph_bounding_box(id).is_none()
                && f.glyph_hor_advance(id).unwrap_or(0) == 0
        })
    }

    /// The glyph lists of `probes` (texts without spaces): one list for
    /// each probe, `None` for a probe some glyph of which came from
    /// another font or is the missing glyph.
    fn lay_out(&self, probes: &[(Probe, String)]) -> Option<Vec<Option<Vec<u16>>>> {
        let mut out = Vec::with_capacity(probes.len());
        for batch in probes.chunks(BATCH) {
            let text = batch
                .iter()
                .map(|p| p.1.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let glyphs = shape(self.family, self.bold, self.italic, &text)?;
            let mut groups: Vec<Option<Vec<u16>>> = vec![Some(Vec::new())];
            for (id, chars, from) in glyphs {
                if chars == " " {
                    groups.push(Some(Vec::new()));
                    continue;
                }
                let own = Arc::ptr_eq(&from.data, &self.font.data) && from.index == self.font.index;
                // A glyph that draws nothing and takes no room stands
                // for nothing (a font can put one where a sign was that
                // another glyph now carries).
                if own && self.blank(id) {
                    continue;
                }
                if let Some(last) = groups.last_mut() {
                    match last {
                        Some(g) if own && id != 0 => g.push(id),
                        _ => *last = None,
                    }
                }
            }
            if groups.len() != batch.len() {
                return None;
            }
            out.extend(groups);
        }
        Some(out)
    }

    /// Letters, signs, pairs, the reph and the signs that join.
    fn learn(&self) -> Option<Learned> {
        let units = units();
        let marks: Vec<char> = ('\u{0900}'..='\u{0903}')
            .chain('\u{093A}'..='\u{094F}')
            .chain('\u{0955}'..='\u{0957}')
            .chain(['\u{0962}', '\u{0963}'])
            .collect();
        let mut probes: Vec<(Probe, String)> =
            vec![(Probe::Single(DOTTED_CIRCLE), DOTTED_CIRCLE.into())];
        for (i, u) in units.iter().enumerate() {
            probes.push((Probe::Unit(i), u.clone()));
        }
        for c in ('\u{0904}'..='\u{0914}')
            .chain(['\u{093D}', '\u{0950}'])
            .chain('\u{0958}'..='\u{0961}')
            .chain('\u{0964}'..='\u{097F}')
        {
            probes.push((Probe::Single(c), c.into()));
        }
        for (i, u) in units.iter().enumerate() {
            for &m in &marks {
                // (No second nukta.)
                if m != NUKTA || !u.ends_with(NUKTA) {
                    probes.push((Probe::Mark(i, m), format!("{u}{m}")));
                }
            }
        }
        // The reph before the pairs: र्र is a reph over ra, not a form
        // of ra under it.
        for (b, ub) in units.iter().enumerate() {
            probes.push((Probe::Reph(b), format!("{REPH}{ub}")));
        }
        for (a, ua) in units.iter().enumerate() {
            for (b, ub) in units.iter().enumerate() {
                probes.push((Probe::Pair(a, b), format!("{ua}{VIRAMA}{ub}")));
            }
        }
        // Signs that join over a letter: a vowel sign with the reph,
        // with a nasal sign, with both. Their glyph can differ with the
        // letter's width.
        let above = [
            SIGN_I, '\u{0940}', '\u{0945}', '\u{0946}', '\u{0947}', '\u{0948}', '\u{0949}',
            '\u{094A}', '\u{094B}', '\u{094C}',
        ];
        let nasals = ['\u{0901}', '\u{0902}'];
        for (i, base) in units.iter().enumerate() {
            for reph in [false, true] {
                for m in std::iter::once(None).chain(above.iter().copied().map(Some)) {
                    for n in std::iter::once(None).chain(nasals.iter().copied().map(Some)) {
                        let mut signs = Vec::new();
                        signs.extend(m.map(String::from));
                        if reph {
                            signs.push(REPH.to_string());
                        }
                        signs.extend(n.map(String::from));
                        if signs.len() < 2 {
                            continue;
                        }
                        // As typed: the reph before the letter, the
                        // signs after it.
                        let after: String = m.into_iter().chain(n).collect();
                        let typed = format!("{}{base}{after}", if reph { REPH } else { "" });
                        probes.push((Probe::Signs(i, signs), typed));
                    }
                }
            }
        }
        let groups = self.lay_out(&probes)?;
        let mut l = Learned {
            bases: vec![Vec::new(); units.len()],
            ..Default::default()
        };
        let is_virama = |known: &HashMap<u16, String>, g: u16| {
            known.get(&g).is_some_and(|t| t.chars().eq([VIRAMA]))
        };
        for ((probe, typed), glyphs) in probes.iter().zip(&groups) {
            let Some(r) = glyphs else { continue };
            if let Probe::Single(DOTTED_CIRCLE) = probe {
                l.dotted = r.first().copied().filter(|_| r.len() == 1);
                continue;
            }
            // A sign the shaper would not put on its letter.
            if l.dotted.is_some_and(|d| r.contains(&d)) {
                continue;
            }
            match probe {
                Probe::Single(_) => {
                    if let [g] = r.as_slice() {
                        l.known.entry(*g).or_insert_with(|| typed.clone());
                    }
                }
                Probe::Unit(i) => {
                    l.bases[*i] = r.clone();
                    if let [g] = r.as_slice() {
                        l.known.entry(*g).or_insert_with(|| typed.clone());
                    }
                }
                Probe::Mark(i, m) => {
                    let base = &l.bases[*i];
                    if base.is_empty() {
                        continue;
                    }
                    if let [g] = r.as_slice() {
                        l.known.entry(*g).or_insert_with(|| typed.clone());
                    } else if r.len() == base.len() + 1 && r.starts_with(base) {
                        l.known.entry(r[r.len() - 1]).or_insert(m.to_string());
                    } else if r.len() == base.len() + 1 && r.ends_with(base) && *m == SIGN_I {
                        l.known.entry(r[0]).or_insert(m.to_string());
                    }
                }
                Probe::Reph(b) => {
                    let base = &l.bases[*b];
                    if !base.is_empty() && r.len() == base.len() + 1 && r.starts_with(base) {
                        l.known.entry(r[r.len() - 1]).or_insert(REPH.to_string());
                    }
                }
                Probe::Pair(a, b) => {
                    let (first, second) = (&l.bases[*a], &l.bases[*b]);
                    if first.is_empty() || second.is_empty() {
                        continue;
                    }
                    if let [g] = r.as_slice() {
                        if !l.known.contains_key(g) {
                            l.ligatures.push(typed.clone());
                            l.known.insert(*g, typed.clone());
                        }
                    } else if r.len() == second.len() + 1 && r.ends_with(second) {
                        // A half form before the second letter.
                        l.known
                            .entry(r[0])
                            .or_insert(format!("{}{VIRAMA}", units[*a]));
                    } else if r.len() == first.len() + 1
                        && r.starts_with(first)
                        && !is_virama(&l.known, r[r.len() - 1])
                    {
                        // A form of the second letter under or beside
                        // the first.
                        l.known
                            .entry(r[r.len() - 1])
                            .or_insert(format!("{VIRAMA}{}", units[*b]));
                    }
                }
                Probe::Signs(i, signs) => learn_signs(&mut l.known, r, &l.bases[*i], signs),
                Probe::After(..) | Probe::Before | Probe::WithI => {}
            }
        }
        // The short i sign has a glyph for every width of cluster:
        // before each pair (before each letter it was learned above).
        let mut more: Vec<(Probe, String)> = Vec::new();
        for ua in &units {
            for ub in &units {
                more.push((Probe::WithI, format!("{ua}{VIRAMA}{ub}{SIGN_I}")));
            }
        }
        self.learn_more(&mut l, &more);
        Some(l)
    }

    /// Conjuncts of three letters, tried from the conjuncts of two the
    /// font has a glyph for; then, while a glyph of `used` still has no
    /// text, the short i sign before them.
    fn learn_deep(&self, l: &mut Learned, used: &[u16]) {
        let units = units();
        let ligatures: Vec<String> = l.ligatures.iter().take(MAX_LIGATURES).cloned().collect();
        let mut more: Vec<(Probe, String)> = Vec::new();
        for lig in &ligatures {
            for (i, u) in units.iter().enumerate() {
                more.push((Probe::After(lig.clone(), i), format!("{lig}{VIRAMA}{u}")));
                more.push((Probe::Before, format!("{u}{VIRAMA}{lig}")));
            }
        }
        self.learn_more(l, &more);
        l.deep = true;
        if used.iter().all(|g| l.known.contains_key(g)) {
            return;
        }
        more.clear();
        for lig in &ligatures {
            for u in &units {
                more.push((Probe::WithI, format!("{lig}{VIRAMA}{u}{SIGN_I}")));
                more.push((Probe::WithI, format!("{u}{VIRAMA}{lig}{SIGN_I}")));
            }
        }
        self.learn_more(l, &more);
    }

    fn learn_more(&self, l: &mut Learned, more: &[(Probe, String)]) {
        let Some(groups) = self.lay_out(more) else {
            return;
        };
        // The conjuncts first, so that the i sign before one is told
        // from it.
        for pass in 0..2 {
            for ((probe, typed), glyphs) in more.iter().zip(&groups) {
                let Some(r) = glyphs else { continue };
                if l.dotted.is_some_and(|d| r.contains(&d)) {
                    continue;
                }
                match (pass, probe) {
                    (0, Probe::After(lig, i)) => {
                        let second = &l.bases[*i];
                        if let [g] = r.as_slice() {
                            l.known.entry(*g).or_insert_with(|| typed.clone());
                        } else if !second.is_empty()
                            && r.len() == second.len() + 1
                            && r.ends_with(second)
                        {
                            l.known.entry(r[0]).or_insert(format!("{lig}{VIRAMA}"));
                        }
                    }
                    (0, Probe::Before) => {
                        if let [g] = r.as_slice() {
                            l.known.entry(*g).or_insert_with(|| typed.clone());
                        }
                    }
                    // Drawn first, before glyphs all known.
                    (1, Probe::WithI)
                        if r.len() >= 2
                            && !l.known.contains_key(&r[0])
                            && r[1..].iter().all(|g| l.known.contains_key(g)) =>
                    {
                        l.known.insert(r[0], SIGN_I.to_string());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The glyphs `r` of a letter with `signs` (the vowel sign, the reph,
/// the nasal sign): when all but one are known, that one stands for the
/// signs the others do not, in that order. `pdf_import`'s
/// `devanagari_typed_order` puts such a text back as typed: the reph
/// before its letter, the short i sign after it, the nasal sign last.
fn learn_signs(known: &mut HashMap<u16, String>, r: &[u16], base: &[u16], signs: &[String]) {
    // The short i sign is the one sign drawn before its letter: a glyph
    // there, with signs of their own after the letter, is the i sign
    // alone (its glyph can differ when a nasal sign follows).
    if signs.first().is_some_and(|s| s.chars().eq([SIGN_I]))
        && !base.is_empty()
        && r.len() >= base.len() + 2
        && r[1..].starts_with(base)
    {
        known.entry(r[0]).or_insert(SIGN_I.to_string());
    }
    let unknown: Vec<u16> = r
        .iter()
        .copied()
        .filter(|g| !known.contains_key(g))
        .collect();
    let [g] = unknown.as_slice() else {
        return;
    };
    let covered: String = r.iter().filter_map(|g| known.get(g)).cloned().collect();
    let left: Vec<&str> = signs
        .iter()
        .map(String::as_str)
        .filter(|s| !covered.contains(s))
        .collect();
    if !left.is_empty() {
        known.insert(*g, left.concat());
    }
}

/// The text of each glyph of `used` (glyph ids of the embedded TrueType
/// font `embedded`, those that show Devanagari) that the installed font
/// of one of `families` draws some Devanagari text with. Empty when
/// there is no text engine, the font is not installed, or the installed
/// one is another font.
pub(crate) fn glyph_texts(
    embedded: &[u8],
    families: &[String],
    used: &[u16],
) -> HashMap<u16, String> {
    let Ok(face) = ttf_parser::Face::parse(embedded, 0) else {
        return HashMap::new();
    };
    // The installed font, in the style the file embeds.
    let mut found = None;
    'find: for family in families {
        for (bold, italic) in [(false, false), (true, false), (false, true), (true, true)] {
            let Some(glyphs) = shape(family, bold, italic, "\u{0915}") else {
                return HashMap::new();
            };
            if let [(_, _, font)] = glyphs.as_slice() {
                if same_font(&face, font, used) {
                    found = Some(Installed {
                        family,
                        bold,
                        italic,
                        font: font.clone(),
                    });
                    break 'find;
                }
            }
        }
    }
    let Some(installed) = found else {
        return HashMap::new();
    };
    let key = (
        Arc::as_ptr(&installed.font.data) as usize,
        installed.font.data.len(),
        installed.font.index,
    );
    let cached = |key| {
        let cache = LEARNED.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, l)| l.clone())
    };
    let store = |key, l: &Arc<Learned>| {
        let mut cache = LEARNED.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|(k, _)| *k != key);
        cache.push((key, l.clone()));
    };
    let mut learned = match cached(key) {
        Some(l) => l,
        None => {
            let Some(l) = installed.learn() else {
                return HashMap::new();
            };
            let l = Arc::new(l);
            store(key, &l);
            l
        }
    };
    // Glyphs of the file still without a text: conjuncts of three
    // letters.
    let left: Vec<u16> = used
        .iter()
        .copied()
        .filter(|g| !learned.known.contains_key(g) && !installed.blank(*g))
        .collect();
    if !learned.deep && !left.is_empty() {
        let mut l = (*learned).clone();
        installed.learn_deep(&mut l, &left);
        learned = Arc::new(l);
        store(key, &learned);
    }
    let used: HashSet<u16> = used.iter().copied().collect();
    let mut texts: HashMap<u16, String> = learned
        .known
        .iter()
        .filter(|(g, _)| used.contains(g))
        .map(|(g, t)| (*g, t.clone()))
        .collect();
    // A glyph that draws nothing and takes no room stands for nothing,
    // whatever the map gave it.
    for g in used {
        if installed.blank(g) {
            texts.insert(g, String::new());
        }
    }
    texts
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTO_DEVA: &[u8] = include_bytes!("../../text/testdata/NotoSansDevanagari-Regular.ttf");
    const QUICKSAND: &[u8] = include_bytes!("../../text/testdata/Quicksand-VF.ttf");

    fn font(data: &[u8]) -> GlyphFont {
        GlyphFont {
            data: Arc::new(data.to_vec()),
            index: 0,
            coords: Vec::new(),
        }
    }

    /// An installed font is asked only when it is the font the file
    /// embeds: another font under the same name, or no glyph with an
    /// outline to compare, and nothing is learned from it.
    #[test]
    fn only_the_font_the_file_embeds_is_asked() {
        let face = ttf_parser::Face::parse(NOTO_DEVA, 0).expect("font");
        let ka = face.glyph_index('\u{0915}').expect("ka").0;
        let space = face.glyph_index(' ').expect("space").0;
        assert!(same_font(&face, &font(NOTO_DEVA), &[ka, space]));
        assert!(!same_font(&face, &font(QUICKSAND), &[ka]));
        assert!(!same_font(&face, &font(NOTO_DEVA), &[]));
        assert!(!same_font(&face, &font(NOTO_DEVA), &[space]));
        // A font of the same family with another glyph changed.
        let mut other = NOTO_DEVA.to_vec();
        let hmtx = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"hmtx"))
            .expect("hmtx");
        let at = hmtx.as_ptr() as usize - NOTO_DEVA.as_ptr() as usize + 4 * usize::from(ka);
        other[at] ^= 1;
        assert!(!same_font(&face, &font(&other), &[ka]));
    }

    /// One glyph for several signs takes the signs the other glyphs of
    /// its cluster do not stand for, the vowel sign first, then the reph,
    /// then the nasal sign; the short i sign alone before its letter is
    /// told from the signs after the letter.
    #[test]
    fn a_glyph_of_joined_signs_takes_the_signs_left_over() {
        let known = || -> HashMap<u16, String> {
            [(1, "\u{0915}"), (2, "\u{0940}"), (3, REPH)]
                .into_iter()
                .map(|(g, t)| (g, t.to_string()))
                .collect()
        };
        let signs = |s: &[&str]| -> Vec<String> { s.iter().map(|s| s.to_string()).collect() };
        // क with one glyph for ी and the reph.
        let mut k = known();
        learn_signs(&mut k, &[1, 9], &[1], &signs(&["\u{0940}", REPH]));
        assert_eq!(k[&9], "\u{0940}\u{0930}\u{094D}");
        // क, ी, and one glyph for the reph and the anusvara.
        let mut k = known();
        learn_signs(
            &mut k,
            &[1, 2, 9],
            &[1],
            &signs(&["\u{0940}", REPH, "\u{0902}"]),
        );
        assert_eq!(k[&9], "\u{0930}\u{094D}\u{0902}");
        // One glyph before क for the short i sign and the anusvara.
        let mut k = known();
        learn_signs(&mut k, &[9, 1], &[1], &signs(&["\u{093F}", "\u{0902}"]));
        assert_eq!(k[&9], "\u{093F}\u{0902}");
        // The short i sign before क and an anusvara of its own after it.
        let mut k = known();
        learn_signs(&mut k, &[8, 1, 9], &[1], &signs(&["\u{093F}", "\u{0902}"]));
        assert_eq!((k[&8].as_str(), k[&9].as_str()), ("\u{093F}", "\u{0902}"));
        // Two glyphs nobody knows: nothing is made up.
        let mut k = known();
        learn_signs(&mut k, &[1, 8, 9], &[1], &signs(&["\u{0940}", REPH]));
        assert!(!k.contains_key(&8) && !k.contains_key(&9));
    }
}

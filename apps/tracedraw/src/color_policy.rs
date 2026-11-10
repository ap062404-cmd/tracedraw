//! Colour management of drawings: the ICC profiles found on this computer
//! (for the Colour Management dialog's lists), assigning or converting a
//! drawing's profiles, and the open and import policies. See
//! docs/behavior/colour-management.md.

use crate::app::App;
use tracedraw_core::icc::{ColorSpace, Intent, Profile, Transform};
use tracedraw_core::{Color, Command, Fill, Shape, ShapeKind};

/// The colour spaces the dialog lists profiles for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Space {
    Rgb,
    Cmyk,
    Gray,
}

/// An ICC profile file found on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct FoundProfile {
    pub name: String,
    pub path: String,
    pub space: Space,
}

/// The folders where systems keep colour profiles. Keep the search
/// platform-specific: scanning paths from other operating systems is
/// harmless but obscures which locations TraceDraw actually supports.
fn profile_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "linux")]
    {
        dirs.push("/usr/share/color/icc".into());
        dirs.push("/usr/local/share/color/icc".into());
        // colord keeps profiles installed through desktop colour management here.
        dirs.push("/var/lib/colord/icc".into());
        if let Some(home) = crate::files::home_dir() {
            dirs.push(home.join(".local/share/color/icc"));
            dirs.push(home.join(".local/share/icc"));
            dirs.push(home.join(".color/icc"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Respect non-default Windows installations instead of assuming
        // that the system drive is always C:.
        if let Some(root) = std::env::var_os("WINDIR") {
            dirs.push(std::path::PathBuf::from(root).join(
                "System32/spool/drivers/color",
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        dirs.push("/Library/ColorSync/Profiles".into());
        dirs.push("/System/Library/ColorSync/Profiles".into());
        if let Some(home) = crate::files::home_dir() {
            dirs.push(home.join("Library/ColorSync/Profiles"));
        }
    }

    dirs
}

/// Every readable RGB, CMYK and grayscale profile in the system folders
/// (and their subfolders, two levels down), sorted by name, each name
/// once.
pub fn system_profiles() -> Vec<FoundProfile> {
    let mut out: Vec<FoundProfile> = Vec::new();
    fn walk(dir: &std::path::Path, depth: usize, out: &mut Vec<FoundProfile>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 2 {
                    walk(&p, depth + 1, out);
                }
                continue;
            }
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "icc" && ext != "icm" {
                continue;
            }
            let Ok(bytes) = std::fs::read(&p) else {
                continue;
            };
            let Ok(profile) = Profile::parse(&bytes) else {
                continue;
            };
            let space = match profile.color_space() {
                ColorSpace::Rgb => Space::Rgb,
                ColorSpace::Cmyk => Space::Cmyk,
                ColorSpace::Gray => Space::Gray,
                _ => continue,
            };
            let name = profile
                .description()
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    p.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            out.push(FoundProfile {
                name,
                path: p.display().to_string(),
                space,
            });
        }
    }
    if crate::files::WEB {
        return out;
    }
    for d in profile_dirs() {
        walk(&d, 0, &mut out);
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.name == b.name && a.space == b.space);
    out
}

/// A named colour preset: the default profiles of a region.
pub struct Preset {
    pub key: &'static str,
    pub rgb: &'static str,
    pub cmyk: &'static str,
    pub gray: &'static str,
}

/// The built-in presets. Their profiles are used when a file of that name
/// is found on this computer; otherwise the name is kept and the built-in
/// formulas convert.
pub const PRESETS: [Preset; 4] = [
    Preset {
        key: "color_mgmt.preset_north_america",
        rgb: "sRGB IEC61966-2.1",
        cmyk: "U.S. Web Coated (SWOP) v2",
        gray: "Dot Gain 20%",
    },
    Preset {
        key: "color_mgmt.preset_europe",
        rgb: "sRGB IEC61966-2.1",
        cmyk: "Coated FOGRA39 (ISO 12647-2:2004)",
        gray: "Dot Gain 15%",
    },
    Preset {
        key: "color_mgmt.preset_japan",
        rgb: "sRGB IEC61966-2.1",
        cmyk: "Japan Color 2001 Coated",
        gray: "Dot Gain 15%",
    },
    Preset {
        key: "color_mgmt.preset_web",
        rgb: "sRGB IEC61966-2.1",
        cmyk: "Generic CMYK (open)",
        gray: "Gray Gamma 2.2",
    },
];

/// Every colour of a fill, changed by `f`.
pub fn map_fill(fill: &Fill, f: &dyn Fn(Color) -> Color) -> Fill {
    let mut fill = fill.clone();
    match &mut fill {
        Fill::Solid(c) => *c = f(*c),
        Fill::Fountain(g) => g.stops.iter_mut().for_each(|s| s.color = f(s.color)),
        Fill::Mesh(m) => m.nodes.iter_mut().for_each(|n| n.color = f(n.color)),
        _ => {}
    }
    fill
}

/// The commands that change every colour of `shapes` (and of the shapes
/// inside groups and clip frames) by `f`, for the shapes that change.
pub fn recolor_commands(shapes: &[Shape], f: &dyn Fn(Color) -> Color, out: &mut Vec<Command>) {
    for s in shapes {
        let fill = map_fill(&s.fill, f);
        if fill != s.fill {
            out.push(Command::SetFill {
                shapes: vec![s.id],
                fill,
            });
        }
        if let Some(st) = &s.stroke {
            let c = f(st.color);
            if c != st.color {
                let mut st = st.clone();
                st.color = c;
                out.push(Command::SetStroke {
                    shapes: vec![s.id],
                    stroke: Some(st),
                });
            }
        }
        match &s.kind {
            ShapeKind::Group { children } => recolor_commands(children, f, out),
            ShapeKind::ClipFrame { frame, contents } => {
                recolor_commands(std::slice::from_ref(frame.as_ref()), f, out);
                recolor_commands(contents, f, out);
            }
            _ => {}
        }
    }
}

/// Change the colours of `shape` (and its children) in place by `f`.
pub fn recolor_shape(shape: &mut Shape, f: &dyn Fn(Color) -> Color) {
    shape.fill = map_fill(&shape.fill, f);
    if let Some(st) = &mut shape.stroke {
        st.color = f(st.color);
    }
    match &mut shape.kind {
        ShapeKind::Group { children } => children.iter_mut().for_each(|c| recolor_shape(c, f)),
        ShapeKind::ClipFrame { frame, contents } => {
            recolor_shape(frame, f);
            contents.iter_mut().for_each(|c| recolor_shape(c, f));
        }
        _ => {}
    }
}

/// A conversion of CMYK colours from profile `from` to profile `to`
/// keeping their appearance; with `preserve_black`, colours made of black
/// ink only stay so. `None` when the profiles give no transform.
pub fn cmyk_converter(
    from: &Profile,
    to: &Profile,
    intent: Intent,
    preserve_black: bool,
) -> Option<impl Fn(Color) -> Color> {
    let t = Transform::new(from, to, intent, true)?;
    Some(move |c: Color| match c {
        Color::Cmyk { c, m, y, k } => {
            if preserve_black && c == 0.0 && m == 0.0 && y == 0.0 {
                return Color::Cmyk { c, m, y, k };
            }
            let v = t.transform(&[c, m, y, k]);
            if v.len() < 4 {
                return Color::Cmyk { c, m, y, k };
            }
            let q = |x: f32| (x.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
            Color::Cmyk {
                c: q(v[0]),
                m: q(v[1]),
                y: q(v[2]),
                k: q(v[3]),
            }
        }
        other => other,
    })
}

/// A profile from its file (`None` for an empty or unreadable path).
pub fn load(path: &str) -> Option<Profile> {
    if path.is_empty() {
        return None;
    }
    let bytes = crate::files::read(std::path::Path::new(path)).ok()?;
    Profile::parse(&bytes).ok()
}

impl App {
    /// Assign or convert the active drawing's profiles. With `convert`,
    /// CMYK colours are converted from the drawing's CMYK profile to the
    /// new one so they look the same; otherwise the values stay and only
    /// the profiles change (one undo step).
    pub fn set_document_profiles(
        &mut self,
        rgb: &FoundOrName,
        cmyk: &FoundOrName,
        gray: &FoundOrName,
        convert: bool,
    ) {
        let intent = Intent::from_name(&self.settings.color.intent).unwrap_or_default();
        let old = self
            .doc()
            .cmyk_profile
            .as_ref()
            .and_then(|p| Profile::parse(&p.data).ok())
            .or_else(|| load(&self.settings.color.cmyk_profile_path));
        let new_data = cmyk
            .path()
            .and_then(|p| crate::files::read(std::path::Path::new(p)).ok());
        let new_profile = new_data.as_ref().and_then(|d| Profile::parse(d).ok());
        let mut cmds = Vec::new();
        if convert {
            if let (Some(from), Some(to)) = (&old, &new_profile) {
                if let Some(f) =
                    cmyk_converter(from, to, intent, self.settings.color.preserve_pure_black)
                {
                    let shapes: Vec<Shape> = self
                        .doc()
                        .all_layers()
                        .flat_map(|l| l.shapes.iter().cloned())
                        .collect();
                    recolor_commands(&shapes, &f, &mut cmds);
                }
            }
        }
        let mut metadata = self.doc().metadata.clone();
        metadata.rgb_profile = rgb.name().to_string();
        metadata.cmyk_profile = cmyk.name().to_string();
        metadata.gray_profile = gray.name().to_string();
        cmds.push(Command::SetMetadata { metadata });
        // The CMYK profile's file goes with the drawing (saved in `.cdr`
        // files from version 16); a name only leaves the default one.
        cmds.push(Command::SetCmykProfile {
            profile: new_data
                .filter(|_| new_profile.is_some())
                .map(|data| tracedraw_core::DocumentProfile::new(cmyk.name(), data)),
        });
        if let Err(e) = self.engine.run_batch("Color Management", &cmds) {
            self.status = e.to_string();
            return;
        }
        self.apply_color_settings();
    }

    /// The open policy, right after a drawing opened: warnings for a
    /// missing or different CMYK profile, and Assign or Convert to the
    /// default profile.
    pub fn apply_open_policy(&mut self) {
        let c = self.settings.color.clone();
        let carried = self.doc().cmyk_profile.clone();
        match &carried {
            None if c.warn_open_missing => {
                self.status = crate::i18n::tr("color_mgmt.warn_missing");
            }
            Some(p) if c.warn_open_mismatch && p.name != c.cmyk_profile => {
                self.status = crate::i18n::trf(
                    "color_mgmt.warn_mismatch",
                    &[("doc", &p.name), ("default", &c.cmyk_profile)],
                );
            }
            _ => {}
        }
        let Some(carried) = carried else {
            return;
        };
        if c.open_cmyk == 0 {
            return;
        }
        // Assign or convert: the drawing takes the default profile, as the
        // opened state (no undo step, not marked changed).
        let mut doc = self.doc().clone();
        if c.open_cmyk == 2 {
            let intent = Intent::from_name(&c.intent).unwrap_or_default();
            if let (Ok(from), Some(to)) =
                (Profile::parse(&carried.data), load(&c.cmyk_profile_path))
            {
                if let Some(f) = cmyk_converter(&from, &to, intent, c.preserve_pure_black) {
                    for page in &mut doc.pages {
                        for layer in &mut page.layers {
                            layer.shapes.iter_mut().for_each(|s| recolor_shape(s, &f));
                        }
                    }
                }
            }
        }
        doc.cmyk_profile = None;
        doc.metadata.cmyk_profile = c.cmyk_profile.clone();
        self.engine.replace(doc);
        self.apply_color_settings();
    }

    /// The import policy for shapes coming from a drawing that carries
    /// `profile`: converted to the active drawing's CMYK profile, or kept
    /// as they are (assigned).
    pub fn apply_import_policy(
        &self,
        shapes: &mut [Shape],
        profile: Option<&tracedraw_core::DocumentProfile>,
    ) {
        let c = &self.settings.color;
        if c.import_cmyk != 0 {
            return;
        }
        let Some(from) = profile.and_then(|p| Profile::parse(&p.data).ok()) else {
            return;
        };
        let to = self
            .doc()
            .cmyk_profile
            .as_ref()
            .and_then(|p| Profile::parse(&p.data).ok())
            .or_else(|| load(&c.cmyk_profile_path));
        let Some(to) = to else {
            return;
        };
        let intent = Intent::from_name(&c.intent).unwrap_or_default();
        if let Some(f) = cmyk_converter(&from, &to, intent, c.preserve_pure_black) {
            shapes.iter_mut().for_each(|s| recolor_shape(s, &f));
        }
    }
}

/// A profile chosen in the dialog: one found on this computer, or a name
/// only (the built-in model, or a preset's profile not installed).
#[derive(Debug, Clone, PartialEq)]
pub enum FoundOrName {
    Found(FoundProfile),
    Name(String),
}

impl FoundOrName {
    pub fn name(&self) -> &str {
        match self {
            FoundOrName::Found(p) => &p.name,
            FoundOrName::Name(n) => n,
        }
    }

    pub fn path(&self) -> Option<&str> {
        match self {
            FoundOrName::Found(p) => Some(&p.path),
            FoundOrName::Name(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracedraw_core::icc::test_profiles as tp;

    #[test]
    fn profile_directories_are_scoped_to_the_target_platform() {
        let dirs = profile_dirs();
        #[cfg(target_os = "linux")]
        {
            assert!(dirs.iter().any(|p| p == std::path::Path::new("/usr/share/color/icc")));
            assert!(dirs.iter().any(|p| p == std::path::Path::new("/var/lib/colord/icc")));
            assert!(!dirs.iter().any(|p| p.to_string_lossy().contains("ColorSync")));
            assert!(!dirs.iter().any(|p| p.to_string_lossy().contains("spool/drivers/color")));
        }
        #[cfg(target_os = "windows")]
        {
            assert!(dirs.iter().all(|p| {
                std::env::var_os("WINDIR")
                    .map(|root| p.starts_with(std::path::PathBuf::from(root)))
                    .unwrap_or(true)
            }));
            assert!(!dirs.iter().any(|p| p.to_string_lossy().contains("ColorSync")));
            assert!(!dirs.iter().any(|p| p == std::path::Path::new("/usr/share/color/icc")));
        }
        #[cfg(target_os = "macos")]
        {
            assert!(dirs.iter().any(|p| p == std::path::Path::new("/Library/ColorSync/Profiles")));
            assert!(!dirs.iter().any(|p| p == std::path::Path::new("/usr/share/color/icc")));
        }
    }

    #[test]
    fn fills_and_outlines_of_nested_shapes_are_recoloured() {
        let mut ids = tracedraw_core::id::IdSource::default();
        let mut inner = Shape::new(
            ids.shape(),
            ShapeKind::Rect {
                rect: tracedraw_core::geometry::Rect::new(0.0, 0.0, 1.0, 1.0),
                radius: 0.0,
                corners: None,
            },
        );
        inner.fill = Fill::Solid(Color::cmyk_pct(0.0, 100.0, 0.0, 0.0));
        let group = Shape::new(
            ids.shape(),
            ShapeKind::Group {
                children: vec![inner.clone()],
            },
        );
        let to_red = |c: Color| match c {
            Color::Cmyk { .. } => Color::rgb8(255, 0, 0),
            o => o,
        };
        let mut cmds = Vec::new();
        recolor_commands(std::slice::from_ref(&group), &to_red, &mut cmds);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], Command::SetFill { shapes, fill: Fill::Solid(_) } if shapes == &vec![inner.id])
        );
        let mut g = group.clone();
        recolor_shape(&mut g, &to_red);
        let ShapeKind::Group { children } = &g.kind else {
            panic!("group");
        };
        assert_eq!(children[0].fill, Fill::Solid(Color::rgb8(255, 0, 0)));
    }

    #[test]
    fn cmyk_converts_between_profiles_and_pure_black_can_stay() {
        let p = Profile::parse(&tp::mba_cmyk_profile()).expect("profile");
        let f = cmyk_converter(&p, &p, Intent::RelativeColorimetric, true).expect("transform");
        // Black ink only stays as it is.
        let k = Color::Cmyk {
            c: 0.0,
            m: 0.0,
            y: 0.0,
            k: 1.0,
        };
        assert_eq!(f(k), k);
        // Other colours go through the profiles and stay CMYK.
        assert!(matches!(
            f(Color::cmyk_pct(20.0, 40.0, 60.0, 10.0)),
            Color::Cmyk { .. }
        ));
        // Non-CMYK colours are left alone.
        assert_eq!(f(Color::rgb8(1, 2, 3)), Color::rgb8(1, 2, 3));
    }

    #[test]
    fn presets_name_their_three_profiles() {
        for p in PRESETS {
            assert!(!p.rgb.is_empty() && !p.cmyk.is_empty() && !p.gray.is_empty());
        }
        assert_eq!(FoundOrName::Name("x".into()).path(), None);
    }
}

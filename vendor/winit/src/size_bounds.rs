//! TraceDraw: a window's minimum and maximum size kept in logical units,
//! so that they follow the monitor's scaling on Windows, and where a
//! window moved to a monitor of another scaling goes (see
//! `vendor/README.md`, #2487). The file has no dependency on the rest of
//! the crate, so that TraceDraw's tests can include and run it.

/// The first build of Windows 11.
const WINDOWS_11_BUILD: u32 = 22000;

/// Whether a window whose scaling changed (`WM_DPICHANGED`) takes the
/// rectangle Windows suggests as it is, on Windows build `build` (`None`
/// when the version is not Windows 10 or 11). Windows 11 works that
/// rectangle out for the window being dragged: the pointer keeps its
/// place on the title bar, across and along the monitors' edge, and the
/// window lies on the monitor whose scaling it takes. winit's own
/// rectangle, made for Windows 10, kept only the pointer's place across
/// the title bar and moved the window step by step onto that monitor:
/// on Windows 11 the window then stopped following the pointer, jumped
/// and went back and forth between the monitors (winit 0.31 takes the
/// suggested rectangle on Windows 11 too).
pub(crate) fn takes_suggested_rect(build: Option<u32>) -> bool {
    build.is_none_or(|b| b >= WINDOWS_11_BUILD)
}

/// A bound given in physical pixels (`width` by `height`) at the scaling
/// `scale`, in logical units: the same bound on a monitor of any
/// scaling. `None` when a number is not finite or the scaling is not
/// above zero: the bound is then kept as it was given.
pub(crate) fn logical_bound(width: f64, height: f64, scale: f64) -> Option<(f64, f64)> {
    if !(width.is_finite() && height.is_finite() && scale.is_finite()) || scale <= 0.0 {
        return None;
    }
    Some((width / scale, height / scale))
}

#[cfg(test)]
mod tests {
    use super::{logical_bound, takes_suggested_rect};

    #[test]
    fn windows_11_places_a_window_moved_to_another_scaling() {
        assert!(takes_suggested_rect(Some(22000)));
        assert!(takes_suggested_rect(Some(26100)));
        // Windows 10 keeps winit's own rectangle.
        assert!(!takes_suggested_rect(Some(19045)));
        assert!(!takes_suggested_rect(Some(10240)));
        // A version not read as Windows 10 or 11 (a later Windows).
        assert!(takes_suggested_rect(None));
    }

    #[test]
    fn a_bound_in_pixels_becomes_one_in_logical_units() {
        // 800 x 500 points asked on a monitor at 150 %: 1200 x 750 pixels.
        assert_eq!(logical_bound(1200.0, 750.0, 1.5), Some((800.0, 500.0)));
        // On a monitor at 100 % the bound is then 800 x 500 pixels, not
        // the 1200 x 750 that held the window larger than it was asked
        // to be, and at 200 % 1600 x 1000.
        let (w, h) = logical_bound(1200.0, 750.0, 1.5).expect("a bound");
        assert_eq!((w * 1.0, h * 1.0), (800.0, 500.0));
        assert_eq!((w * 2.0, h * 2.0), (1600.0, 1000.0));
        assert_eq!(logical_bound(800.0, 500.0, 1.0), Some((800.0, 500.0)));
    }

    #[test]
    fn a_scaling_that_makes_no_sense_leaves_the_bound_as_given() {
        assert_eq!(logical_bound(800.0, 500.0, 0.0), None);
        assert_eq!(logical_bound(800.0, 500.0, -1.0), None);
        assert_eq!(logical_bound(800.0, 500.0, f64::NAN), None);
        assert_eq!(logical_bound(f64::INFINITY, 500.0, 1.0), None);
    }
}

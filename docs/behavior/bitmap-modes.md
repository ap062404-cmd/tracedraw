# Bitmap colour modes

Image > Mode converts the selected bitmaps; every selected bitmap
changes in one undo step. The dialogs are windows the user sizes that
show the first selected bitmap before and after in the shared preview
(`before-after-preview.md`: the Preview list, zoom and pan, the copy as
large as the panes show; the copy's pixel sizes follow its scale), the
settings in the shared settings panel at the right (each slider under
its label, stretched to the panel's width) with the Reset, undo and
redo row under it: Reset puts the defaults back, the arrows step through
the settings states made in the dialog (a slider's drag is one step on
release), the history starts afresh when the dialog opens. Alpha is
kept in every mode.

| Item | Result |
|---|---|
| Black and White (1-bit)... | dialog below |
| Grayscale (8-bit) | luminance `0.299 R + 0.587 G + 0.114 B`, stored as an 8-bit grayscale PNG (with alpha when the bitmap has transparency); the bitmap is no longer CMYK nor keeps ink channels, so the status line names it Grayscale. Ticked while every selected bitmap is grayscale (#2330, `ops2::tests::image_mode_grayscale_converts_the_bitmap`) |
| Duotone (8-bit)... | dialog below |
| Indexed (8-bit)... | dialog below; the result is stored as an indexed PNG (a palette of up to 256 colours with their opacity), so the status line names it Indexed and the item is ticked while every selected bitmap is indexed; a bitmap left with more than 256 colours and opacities stays RGB. It is no longer CMYK nor keeps ink channels (#2607, `ops2::tests::image_mode_lab_and_indexed_convert_the_bitmap`) |
| RGB Color (24-bit) | the bitmap's colour mode becomes RGB (`Shape::cmyk_bitmap` off); pixels unchanged, a grayscale bitmap stored with three channels again. Ticked while every selected bitmap is RGB (neither CMYK nor grayscale) |
| Lab Color (24-bit) | each pixel through CIE Lab (D65) and back, unchanged within one level; the bitmap's colour mode becomes Lab, marked in its PNG (a `tEXt` chunk "TraceDraw color mode" = "Lab"), so the status line names it Lab and the item is ticked while every selected bitmap is Lab; no longer CMYK nor keeping ink channels. RGB Color stores it unmarked again (#2605) |
| CMYK Color (32-bit) | the colour mode becomes CMYK and the pixels show as the inks print them (through the colour management profiles and back), as Tools > Convert Colors does (#1254, `colour-management.md`). Ticked while every selected bitmap is CMYK |

## Black and White (1-bit)

Conversion methods:

- Line art: white when the gray level is at or above Threshold (0 to
  255, default 128).
- Ordered: an 8 x 8 Bayer matrix.
- Halftone (default): a screen of Round, Line or Square dots at Angle
  (default 45°) and Lines per inch (default 60) at the bitmap's
  resolution; the threshold field of round dots is
  `(cos u + cos v) / 4 + 0.5` in the screen's rotated cell coordinates.
- Cardinality-Distribution: a fixed per-pixel random threshold (a
  textured look).
- Jarvis, Stucki, Floyd-Steinberg: error diffusion with those kernels
  (divisors 48, 42 and 16).

Intensity (0 to 100, default 50; line art uses Threshold instead, and
the halftone screen has no slider) adds `(intensity - 50) * 2.55` to every gray level
before the conversion, so higher is lighter.

## Duotone (8-bit)

- Type: Monotone, Duotone (default), Tritone or Quadtone: one to four
  inks.
- Each ink has a colour (the swatch) and a tone curve: x is the gray
  level (0 black to 255 white), y the ink's coverage (0 to 100 %).
  Defaults: black ink from 100 % on black to 0 % on white; the second
  ink orange, 75 % on black, 50 % at the midpoint, 0 % on white; the
  third and fourth cyan and magenta at 55 % and 40 % on black.
- The curve editor: drag on empty space to add a point, drag a point to
  move it (the end points move up and down), double-click or drag it out
  of the grid to remove it. Curves are smooth (monotone cubic through the
  points). Show all draws the other inks' curves too.
- Inks print over one another: each channel is
  `prod(1 - coverage_i * (1 - ink_i))`.
- Load and Save keep the type, inks and curves in a JSON file.

## Indexed (8-bit)

- Palette: Uniform Colors (6 x 6 x 6 colours and 40 grays), Standard VGA
  (16), Adaptive and Optimized (median cut of the image's colours to
  Colors, 2 to 256, default 256; Adaptive splits the boxes with the
  widest range first, Optimized the most populous; an image with fewer
  colours keeps them all), Grayscale (256), System (VGA and the 216
  web-safe colours) and Custom (the document palette).
- Dithering: None, Ordered (Bayer, amplitude 64 levels times the
  intensity), Jarvis, Stucki, Floyd-Steinberg (the error spread times
  Dither intensity, 0 to 100, default 100).
- The processed palette is shown under the settings.
- Nearest colours are looked up through a 32 x 32 x 32 cache.

Not yet: an Overprint tab, range sensitivity and saved
conversion presets.

## Convert to Bitmap

Image > Convert to Bitmap (also Object > Convert to Bitmap) turns the
selection into one bitmap in one undo step:

- Resolution: a list (72, 96, 150, 200, 300, 400, 600 dpi) and a value
  (36 to 2400, default 300).
- Color mode: Black and White (1-bit), Grayscale (8-bit), Indexed
  (8-bit), RGB Color (24-bit, default), CMYK Color (32-bit). The result
  is a bitmap in that colour mode, which the status line shows (#1777;
  Indexed an indexed PNG while the colours fit a palette, #2607):
  Grayscale stores the luminance `0.299 R + 0.587 G + 0.114 B` as an
  8-bit grayscale PNG (with an alpha channel when the background is
  transparent), and a bitmap whose PNG is grayscale is in the Grayscale
  mode; CMYK marks the bitmap CMYK (`Shape::cmyk_bitmap`) and keeps the
  inks the objects print with (`Shape::picture_inks`, each process plate
  drawn as the coverage of its ink, edge pixels taken to their full
  opacity), and its pixels are the colours of those inks, so it shows as
  the inks print it (#1254) and a colour given in CMYK keeps its values
  and its look: the Color Eyedropper reads C0 M100 Y100 K0 back from a
  red of C0 M100 Y100 K0, not the C2 M99 Y96 of the screen pixels
  converted back. Past 65536 different inks the pixels go through the
  colour management profiles and back instead (#2381,
  `ops2::tests::convert_to_cmyk_bitmap_keeps_the_cmyk_values`). Black and
  white uses a 50 % threshold, or Floyd-Steinberg when Dithered; Indexed
  an optimized 256-colour palette, dithered by Floyd-Steinberg or not;
  both stay RGB pictures. A `.cdr` save still writes every picture's
  pixels as RGB (`docs/cdr-format.md`).
- Dithered: only for Black and White and Indexed (256 colours or fewer).
- Always overprint black: the bitmap is marked to overprint, so its black
  prints over the colours beneath (multiplied when overprints are
  simulated).
- Anti-aliasing (on): smooth edges; off renders hard pixel edges.
- Transparent background (on); off puts the selection on white.
- The uncompressed size is shown: `ceil(width x bits / 8) x height`.
- The bitmap covers all the selection draws: its box grown by the drop
  shadows of its objects (and of the objects in its groups), feathering
  included, so a shadow reaching past an object is kept whole rather
  than cut at the object's box (#2351,
  `ops2::tests::convert_to_bitmap_keeps_the_whole_drop_shadow`); and by
  the outlines as they are drawn (half the width beyond the path, all of
  it for an outline outside, mitred corners and arrowheads), so a thick
  outline is not cut (#2369,
  `ops2::tests::convert_to_bitmap_keeps_the_whole_thick_outline`).

## Monochrome bitmaps

A bitmap whose opaque pixels have at most two colours is monochrome.
With only monochrome bitmaps selected, a click on a palette colour
colours the background (the lighter colour, white at first) and a
right-click the foreground (the darker, black at first), in one undo
step ("Bitmap Color"); the object's own fill and outline are left alone.
A bitmap with effects or more colours takes the fill and outline as any
object does.

## Checks

- `bitmap_modes::tests::black_and_white_methods_give_two_levels_and_follow_tone`
- `bitmap_modes::tests::duotone_inks_multiply`
- `bitmap_modes::tests::paletted_uses_only_palette_colours`
- `bitmap_modes::tests::tone_curves_pass_through_their_points`
- `bitmap_modes::tests::lab_round_trips`
- `bitmap_modes::tests::conversions_apply_to_every_selected_bitmap_in_one_step`
- `ui::dialogs::tests::every_dialog_draws_and_accepts_enter` draws the
  three dialogs.
- `ops2::tests::convert_to_bitmap_gives_the_colour_mode_asked_for`:
  Grayscale gives a grayscale PNG and status, CMYK a CMYK bitmap with
  the printed pixels, RGB an RGB one (#1777).
- `ops2::tests::convert_to_bitmap_applies_mode_dither_and_overprint`:
  black and white gives only black and white, grayscale equal channels,
  overprint marks the bitmap, no anti-aliasing leaves no half-covered
  pixel, and the size estimate rounds bits up per row.
- `tests::without_anti_aliasing_edges_are_hard` (renderer).
- `bitmap_modes::tests::monochrome_bitmaps_take_palette_colours`: a
  click colours the white pixels, a right-click the black ones, undo
  brings them back.

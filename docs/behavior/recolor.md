# Recolor

Tools > Recolor... changes every colour of the selected objects at
once while keeping how light each colour is next to the others, so a
drawing shaded with highlights and shadows keeps its shading in its new
colours. It is enabled while objects are selected.

| Setting | Range | Default |
|---|---|---|
| Tint with one color (box and colour) | any colour | off, a mid blue |
| Hue | -180 to 180 degrees | 0 |
| Saturation | -100 to 100 % | 0 |
| Lightness | -100 to 100 % | 0 |
| Recolor spot colors too | on, off | off |

The change shows on the drawing while the dialog is open (an engine
preview, made again only when a value changes). OK keeps it as one undo
step and the status line says how many colours changed; Cancel, Esc
and the close button take it back with nothing left on the Undo list.
Reset puts the values back. Edit > Repeat repeats it on the selection
of the time and Edit > Repeat on Similar on the objects like the ones it
was done to (see `shortcuts.md`).

## What changes

Fills (uniform, every gradient stop, pattern and texture colours, mesh
nodes), outlines, text and character colours, table cells, and the
colours of effects and shadows, of the selected objects and of every
object inside their groups and clip frames: the colours the Convert
Colors command reaches, through the same walk
(`color_convert::map_shape_colors`). Registration colour never changes.
Spot colours stay unless Recolor spot colors too is on; then they become
process colours. The pixels of bitmaps stay (a bitmap effect such as
Hue/Saturation/Lightness changes those).

## Formulas

Each colour is taken to CIE L*a*b* (through the colour management for
CMYK colours) and to its polar form LCh: lightness L, chroma
C = sqrt(a^2 + b^2) and hue h = atan2(b, a).

1. Tint: h and C become the tint colour's; L stays the colour's own.
2. Hue: h + Hue.
3. Saturation: C x (1 + Saturation / 100); -100 gives grey, 100 twice the
   chroma.
4. Lightness: above 0, L + (100 - L) x Lightness / 100 (that share of the
   way to white); below 0, L + L x Lightness / 100 (toward black). The
   order of the colours from light to dark stays.

A colour outside the RGB gamut after the change keeps its L and h and
loses chroma (a bisection on C, 24 steps) until it fits. A grey (chroma
under 0.5) turned by Hue alone stays exactly as it was. The colour comes
back in its own model: a CMYK colour stays CMYK, an RGB colour RGB, a
grey stays grey unless it took a hue (a tint), and then becomes RGB.

Code: `tracedraw_core::recolor` (`Recolor::apply`, `recolor_document`),
`Command::Recolor`, `apps/tracedraw/src/ui/recolor.rs`. Tests:
`recolor::tests`, `ui::recolor::tests`.

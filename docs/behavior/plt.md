# HPGL plotter files (PLT)

Import (File > Open, File > Import) and export (File > Export) of
`.plt`, `.hpgl` and `.hgl` files.

## Units and orientation

1 plotter unit = 0.025 mm (40 per mm), origin at the bottom left, Y up,
which is the document's own orientation, so coordinates are scaled only.

## Reader

| Command | Behaviour |
|---|---|
| `IN`, `DF` | pen up, absolute mode, position (0, 0) |
| `SP n` | pen n; pens 1 to 8 default to black, red, green, yellow, blue, magenta, cyan, orange |
| `PW w [, n]` | pen width in mm for pen n (all pens without n); default 0.35 mm |
| `PC n, r, g, b` | pen colour |
| `PA`, `PR` | absolute or relative coordinates (also accept points) |
| `PU x,y,...`, `PD x,y,...` | pen up or down, then moves; every pen-down run becomes one open curve with the pen's outline and no fill |
| `CI r` | circle of radius r around the pen |
| `AA x,y,a`, `AR dx,dy,a` | arc of sweep a degrees around a centre, appended to the current stroke |
| `EA`, `ER` | rectangle outline (absolute or relative corner) |
| `RA`, `RR` | filled rectangle |
| `PM 0..2`, `EP`, `FP` | polygon mode; `EP` draws the edge, `FP` fills with the pen colour |
| `IP`, `SC` | scaling points and user units (uniform scaling; non-uniform is ignored with a warning) |
| `LB ...` ETX | labels are skipped (no text in the plotter font) |
| anything else | ignored |

Escape sequences (PJL) are skipped to the end of their line. Limits:
2,000,000 commands and 4,000,000 points. The page size is the drawing's
extent plus 10 mm (50 mm minimum).

## Writer

Every visible, printable layer of the page is flattened (0.02 mm
tolerance) into pen strokes: `IN;IP0,0,w,h;` then `PCn,r,g,b;PWw,n;`
per pen, one pen per distinct outline colour and width in order of first
use (more than 8 share pen 8), then `SPn;PUx,y;PDx,y,...;` per subpath.
Filled objects without an outline are drawn with their fill colour as a
0.25 mm pen; other fills are drawn as black outlines; bitmaps are
skipped. Text is written as the outlines of its glyphs.

## Export choices

The Export dialog shows these for PLT (#2652: it had none, where
CorelDRAW's HPGL export has a scale, the plotter's origin and units and
pen settings). They are kept with the format's other choices and in
export presets (`plt::PltOptions`, `ExportState::plt_options`).

| Choice | Effect |
|---|---|
| Scale, 1 to 10 000 % (100) | the drawing and the pen widths at that size in the file |
| Plotter origin: bottom left of page (default) or centre of page | the plotter's (0, 0); with the centre, `IP` runs from minus half the page to plus half |
| Plotter units, 100 to 100 000 per inch (1016) | 1016 is HPGL's own 0.025 mm; some cutters count 1000 |
| Curve resolution, 0.001 to 5 mm (0.02) | how far the straight strokes may stray from a curve |
| HPGL/2 pen colours and widths (on) | off: no `PC` or `PW`, plain HP-GL as older plotters and most vinyl cutters read it |
| One pen for every outline (off) | every outline with pen 1 (a cutter with one knife), no `SP2` and on |

The defaults write the file exactly as before
(`plt::tests::export_options_scale_centre_units_and_pens`).

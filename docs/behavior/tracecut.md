# TraceCut

TraceCut holds the tools for cutting plotters: preparing cut files,
nesting and measuring. Tools > TraceCut (and Launch > TraceCut on the
standard toolbar) opens it in a window of its own, resizable (880 x 600
at first, 660 x 460 at least). Like the other dialogs it keeps the rest
of the application waiting until it closes, and it works on the objects
selected when it opened.

Options > Global > General, **Show TraceCut in Launch and the Tools
menu** (on at first), hides both entries for those who do not cut.

The geometry is the `tracedraw-tracecut` crate (pure geometry over
page-space outlines in millimetres); the window turns its results into
commands. Contributed by LaMka21 (#170).

Every tool takes an object by the outline it draws: groups opened, a
clip frame by its frame, and an object whose outline a live effect
reshapes (an envelope, a perspective, a distortion) by the reshaped
outline, not the one it had before the effect. So an enveloped object
is cut where it shows, and nesting keeps what shows apart; it used to
be placed by its outline from before the envelope, and its neighbours
landed on the part the envelope had pulled out (#2227,
`tracedraw_tracecut::tests::an_enveloped_object_gives_the_outline_it_draws`,
`tracecut_nest::tests::an_enveloped_object_is_nested_by_what_it_draws`).
What an effect adds beside the object (contour steps, blend steps, a
shadow) is not part of its outline.

## Layout

- **Tool list** (left, 210 wide), grouped: Cut, Nesting, Measure, Make. A
  group with no tools yet is not shown. The tool picked last is picked
  again the next time the window opens (kept in the settings).
- **Right side**, top to bottom: the tool's name and what it does; the
  **Preset** row; the tool's settings; the **preview**; the status line;
  **Apply**, **OK** and **Cancel**.
- **Preview**: the outlines of the selected objects fitted in a white
  box, y up, in dark grey; what the tool found in red (outlines drawn
  thicker, places as rings 6 px wide). With nothing selected the box
  shows "Select the objects to cut, nest or measure" in its middle
  instead.
- **Zoom and pan in the preview**, as on the drawing:
  - the wheel over the preview zooms at the pointer, 1.15 times a
    notch, with or without Ctrl; a pinch (touch screen or touchpad)
    zooms at the fingers;
  - buttons in its top right corner: **Zoom in** and **Zoom out** (1.25
    times about the middle), **Zoom to fit** (the whole result: the
    objects, what was found, the moves), **Zoom to selection** (the
    selected objects as large as they fit) and **Real size (1:1)** (the
    screen's calibrated resolution, Options > Display > Ruler
    calibration, 96 per inch until calibrated);
  - with the pointer over the preview: **+** (or **=**) zooms in and
    **-** out, 1.25 times at the pointer, **0** fits;
  - a drag with the left or middle button pans (Space held changes
    nothing, so Space+drag pans too); two fingers pan on a touch screen;
  - a double click fits again.
  The zoom goes from a tenth of the fit to 2000 points a millimetre (or
  four times the fit when that is more). The outlines are drawn again
  at every zoom, so they stay sharp; zoomed in, objects whose box lies
  outside the preview are skipped. Each tool keeps its own zoom and
  place while the window is open; a change of the selection fits every
  tool again.
- **Box Maker** and **Folded Tray** need no selection: their preview
  shows the result alone and Apply and OK work with nothing selected.
  Folded Tray works its pattern out live, like the Measure tools: again
  only when a setting, the drawing or the selection changes, so the
  preview costs nothing from frame to frame. Its grooves are drawn in
  blue under the red cut lines.
- **Status line** (pale yellow): what was found, or "Select the objects
  to work on." with nothing selected, or "Apply shows the result here
  without changing the drawing; OK applies it." before Apply, or that
  the result is out of date.

## Apply, OK, Cancel

- **Apply** works the result out into the preview and the status line.
  It never changes the drawing.
- **OK** (and Enter) applies the result and closes the window. Anything
  that changes the drawing is one undo step. When Apply was not pressed,
  or the drawing, the selection, the tool or a setting changed since,
  OK works the result out again first.
- **Cancel** (Esc, the close button) closes without a change.
- With nothing selected Apply and OK are greyed out; their tips say to
  select the objects to work on first.

## Presets

Every tool has named presets of its settings, kept in
`tracecut_presets.json` in the settings folder (in memory only in the
browser), one list per tool. The list shows "(none)" until one is
picked; picking a preset loads it. The gear button shows a row with the
preset's name, **Save** (keeps the settings under that name, replacing a
preset of the same name, and makes it the current one), **Delete** and
**Defaults** (puts the tool's settings back). A preset saved by an
older version that lacks a setting keeps that setting's default; a
damaged file gives no presets and is logged.

## Cut tools

The Cut group lists, in this order: Plotter Output, CNC Toolpath,
Weeding, Bridges, Cut Order, CutContour, Registration Marks, Find
Duplicates and Find Crossings. A new window opens on Plotter Output.

### Plotter Output

The job is the outlines of the selection (groups opened, a clip frame
by its frame), flattened within 0.02 mm, written as HPGL: the
machine's start string, `VS` speed and `FS` force when not 0, then for
each outline `PU` to its first point and `PD` with up to 64 points per
command, then the end string. Coordinates are whole plotter units, 40 a
millimetre (0.025 mm), counted from the chosen **origin** corner of the
job's box (bottom left at first) into the job.

- **Machine**: the command set. HPGL (`IN;` ... `PU0,0;`), HPGL with pen
  select (`IN;PA;SP1;` ... `PU0,0;SP0;`), HPGL with a command per line.
  Choosing one keeps the knife settings; a profile whose commands differ
  shows as Custom. Machines are named by what they send; a cutter's own
  settings are kept as a preset.
- **Knife offset** (0.25 mm at first, 0 to 2): the holder moves to
  `P + d u` to cut `P` going in direction `u`; where the direction turns
  by more than 15 degrees it draws an arc of radius `d` round the node in
  steps of at most 10 degrees, so the blade turns on the spot.
- **Overcut** (1 mm): closed outlines are cut on past their start.
- **Passes** (1 to 10): each outline is cut again, the whole job over.
- **Speed**, **force**: sent as `VS` and `FS` in the cutter's own units;
  0 sends nothing.
- **Mirror** left to right, **Turn 90°** counter-clockwise.
- **Weeding frame** with its margin (5 mm): a rectangle round the job,
  cut last.
- **Plan the cut order** (on): nearest outline next from the job's
  bottom-left corner, with **Inner outlines first**; off, the stacking
  order. The drawing does not change (Cut Order changes it).
- **Send to**: **File** (OK saves a `.plt` where you say), **Port** (a
  device path such as `/dev/ttyUSB0`, `/dev/cu.usbserial` or `COM3`;
  `COMn` is opened as `\\.\COMn`, so ports past 9 work; nothing is
  created, the device must exist) or **Network** (`host` or
  `host:port`, port 9100 at first, raw TCP, 5 s to connect). Sending
  runs on a thread of its own and the status line says how it went.
  TraceDraw does not set the port's speed: set it to the cutter's once
  with the system's tools (or the cutter's driver). In the browser and
  on Android only File is offered.
- **Test cut**: cuts, at once and with these settings, a square of the
  **test square** side (20 mm at first, 5 to 200) with a triangle inside
  (its base a quarter of the side in from the square's, its apex a
  quarter below the top), the triangle first.

Apply shows the outlines and the moves with the knife up (blue), and
the status line gives the outlines, the length cut and travelled and the
job's size. OK saves or sends; the drawing does not change.

### CNC Toolpath

G-code for CNC routers and mills, from the outlines of the selection
(groups opened, a clip frame by its frame), flattened within the
**curve accuracy** (0.02 mm at first). The geometry is
`tracedraw_tracecut::cnc`.

- **Tool diameter** (3.175 mm): the tool's centre follows each closed
  outline offset by the radius (the offset Contour uses, round corners
  outside, sharp ones inside). A hole smaller than the tool gives no
  toolpath; the status line counts those left out.
- **Cut side**: **Automatic** (at first) cuts a closed outline lying
  inside an odd number of other closed outlines (a hole, of the same
  object or another) on its inside and every other closed outline on
  its outside; **Outside**, **Inside** and **On the line** cut every
  closed outline that way. Open curves are always cut on the line.
- **Direction**, the spindle turning clockwise (M3): **Conventional**
  (at first) keeps the part on the left of the tool (outside cuts run
  counter-clockwise, holes clockwise), **Climb** on its right.
- **Total depth** (6 mm) in passes no deeper than the **depth per
  pass** (2 mm): `n = ceil(depth / pass)` equal passes, pass `k` at
  `-depth k / n`. Each toolpath is cut to the full depth before the
  next one; a closed one goes down again where it ended, an open one
  goes back to its start at the safe height.
- **Safe height** (5 mm) above the material for the moves between
  cuts; **feed rate** (1000 mm/min) along the outlines, **plunge
  rate** (300 mm/min) going down; **spindle speed** (18000 rpm, 0
  leaves out `M3` and `M5`).
- **X0 Y0 at**: the bottom-left corner of the selection's box (at
  first) or the ruler origin; Z0 is the top of the material.
- **Holding tabs** (off): **tabs per outline** (4) on each closed
  outside cut, spread evenly by length (the first half a spacing from
  the start), each leaving **tab width** (5 mm) of material, so the
  toolpath rises for the width plus the tool diameter, and **tab
  height** (1.5 mm) at the bottom: passes deeper than `depth - height`
  rise to it over the tab. A toolpath the tabs would take more than
  nine tenths of gets fewer; tabs as high as the cut is deep are left
  out.
- **Order**: the toolpaths are planned like Cut Order (nearest next
  from X0 Y0, inner ones first), so holes are cut before the outline
  that frees the part.

The program: comments, `G21`, `G90`, `G94`, `G0 Z<safe>`, `M3 S<rpm>`;
for each toolpath `G0 X Y` to its start, `G1 Z F<plunge>` down and
`G1 X Y F<feed>` along (the feed written when it changes), `G0
Z<safe>` up; then `M5`, `G0` back over X0 Y0 and `M30`. Coordinates in
millimetres with three decimals. Apply shows the toolpaths in red and
the moves at the safe height in blue; the status line gives the
toolpaths, the passes, the length milled and the time at the feed and
plunge rates. OK saves a `.nc` file (also `.gcode`, `.ngc`, `.tap`)
where you say; the drawing does not change.

### Weeding

Extra cuts in the waste, added on OK as one group named Weeding of
hairlines (no fill) on the active layer, in one undo step ("Weeding").

- **Weeding lines** (at first): from each object, lines run out of its
  box (**lines per side**, 1 at first, up to 20, evenly spaced) until
  they meet a neighbour or the frame; every object is grown by the
  **gap** (2 mm) and the lines stop there, each end at an object pulled
  back by **shorten ends** (1 mm). Lines shorter than 0.5 mm and lines
  lying on another are dropped.
- **Weeding boxes**: a rectangle round each object grown by the box
  margins (2 mm each way); overlapping boxes are welded into one
  outline.
- **Weeding frame** (on) the margin (5 mm) round everything.

### Bridges

Short uncut stretches in the closed outlines of the selected shapes and
curves (groups opened; text, pictures and objects with live effects
left alone). Each closed outline longer than **only outlines longer
than** (0) gets **per outline** (2) bridges, or **one every** so much
length (at least one), each **bridge length** long (2 mm), spread
evenly by arc length, the first half a spacing from the start; a bridge
moves to the nearest straight segment within a quarter spacing. An
outline the bridges would take more than nine tenths of is left whole.
OK makes each object a curve of the open pieces between the bridges, in
one undo step ("Bridges"); the preview rings the bridges.

### Cut Order

For the selection, layer by layer (locked layers and objects left
alone): the objects in the order with the least travel with the knife
up (nearest first, then 2-opt on up to 2000 objects), with **inner
outlines first** (on) so a part does not move once free; inside shapes
and curves, their subpaths ordered the same way, each closed outline
starting at the node nearest the knife (or the bottom-left one, or as
it is) and running as drawn, all clockwise or all counter-clockwise.
OK reorders the objects in their layers and rewrites the changed shapes
as curves in one undo step ("Cut Order"); the status line gives the
travel before and after.

### CutContour

Marks the selected objects' outlines as cut lines for print and cut: OK
gives them an outline in the spot colour named **CutContour** (added to
the drawing's spot colours once, its stand-in 100 % magenta), a hairline
or the **outline width** given, and with **Remove the fill** no fill, in
one undo step ("CutContour"). Its swatch joins the document palette in
that step when it is not there yet, as a colour applied from the
Properties docker does (unless the document palette's Automatically
Update is off), so the CutContour colour can be picked again from it
(#1927). The swatch goes by the ink's name, CutContour, in the document
palette, the Palette Manager, the status bar and any palette it is added
to, never by the ink's number ("Spot 1"), so cutting RIPs that look for
the separation named CutContour exactly find it (#2186,
`tracecut_cut::tests::the_cut_contour_swatch_is_named_cut_contour`).
The name is kept on export: PDF writes a
Separation colour space named CutContour (as for every spot colour), EPS
and PostScript a Separation colour space with the ink's name and its
process stand-in, and SVG, which has no spot colours, writes the colour
and `data-spot-stroke="CutContour"` (`data-spot-fill` for a fill).

### Registration Marks

Filled black marks a cutter's optical sensor finds, added on OK as one
group named Registration marks in one undo step ("Registration
Marks"): **around the selection** (outside each corner of its box,
**distance** 10 mm away) or **inside the page** (inside each corner,
the distance in from both edges). **Mark**: square or circle of
**size** 5 mm, or an L whose arms (**arm length** 20 mm, **arm width**
1 mm) run along the corner. On a side longer than **extra marks every**
(400 mm, 0 for none) marks also go between the corners, evenly (a square
for the L). These are generic marks; no maker's layout is built in.

### Find Duplicates

Objects lying on top of another would be cut twice. Two objects are the
same when every point of either outline lies within the **tolerance**
(0.05 mm at first, 0.001 to 10 mm) of the other's and their boxes agree
within it; the outlines are flattened within a quarter of the tolerance
and divided so that points are at most the tolerance apart. Of equal
objects the lowest in the stacking order is the original.

**On OK**: **Delete them** (at first) deletes the repeats in one undo
step ("Delete Duplicates"), locked objects left alone; **Select them**
selects them and changes nothing.

### Find Crossings

An outline crossing itself cuts badly, and objects crossing each other
usually need a weld before cutting. The outlines are flattened within a
quarter of the **tolerance** (0.05 mm at first); neighbouring segments
of one subpath and segments running along each other do not count, and
crossings closer than the tolerance for the same objects are one. With
**Also between objects** (off at first) crossings between different
objects count too.

**On OK**: **Select them** (at first) selects the objects that have
crossings and changes nothing; **Fix them** fixes the crossings in one
undo step ("Fix Crossings"). Each outline crossing itself is split
where it crosses and rebuilt, read with the object's fill rule, as the
outlines of the area it fills (outer outlines and holes that cross
nowhere): a figure eight becomes two loops, a star drawn as one line
its outline or its points. Where two rebuilt loops would still touch at
a former crossing, a dot as wide as the tolerance is cut out there so
they come apart. With **Also between objects**, objects that cross each
other are welded into the lowest of them and the others deleted. Open
lines are kept as they are; curves, rectangles, ellipses and polygons
become curves, and groups, text, pictures and locked objects are left
alone.

**Move them apart** moves objects that cross each other apart in one
undo step ("Separate Crossings"); it looks for crossings between
objects whether or not Also between objects is ticked. In each set of
objects that cross one another (linked through any crossing) the locked
ones stay, or the lowest when none is locked; the others, from the
bottom up, each move the shortest way, of 32 directions evenly spread,
that leaves the **spacing** (2 mm at first, shown with this choice)
between its outlines and those of every object staying or moved before
it: in each direction the move goes out in steps of a sixteenth of the
object's size (at least half the spacing) to the first place that is
clear and is then narrowed down by halving 16 times. Outlines are
flattened within a quarter of the tolerance, and an object lying inside
another's area counts as touching it. Objects of any kind move; the
selected objects that crossed nothing stay and are kept clear of too,
as are outlines crossing only themselves. The moved objects are
selected; when nothing can move (no objects cross each other, or they
are locked) the status line says so and nothing changes
(`crossings::separate_crossing`,
`tracecut::tests::ok_moves_crossing_objects_apart_in_one_undo_step`).

## Nesting tools

Both place the selected objects (locked ones left alone) on the
material and move them there on OK, in one undo step ("Shape Nesting",
"Rectangle Nesting"); with **Draw the sheets** (on) the sheets, or the
length of roll used, are added as a group of hairline rectangles named
Sheets. An object that fits nowhere stays where it is and the status
line says how many. The preview shows the objects where they will go,
and the sheets.

**Material**: **sheets** of a **width** and **height** (1220 x 2440 mm
at first), laid side by side from the page's origin to the right, 20 mm
apart, as many as it takes; or a **roll** of a **width** with no end,
its length running to the right or up the page. **Margin** (0) is kept
free along the edges, **spacing** (2 mm) between objects, and the
layout starts from the **start corner** (bottom left).

### Shape Nesting

Objects by their true outlines: each becomes a region (its closed
outlines flattened, open curves by their boxes; holes filled unless
**use holes** is on, 0.5 mm **accuracy** at first; an open line inside
an outline cuts no hole there), grown by half the spacing and drawn on
a grid. The grown outline is kept as the polygon it is worked out as,
so its round corners and curves stay outside the object and no object
comes closer to another, or to the sheet's edge, than the spacing and
margin allow. Objects go largest first, each to the
lowest, then leftmost, place where it fits on the first sheet that has
one, slid down and left in exact geometry until it touches, in each
allowed **turn** (none, 0 and 180 degrees, every 90 at first, every 45);
the turn whose top ends lowest wins. Then, for up to the **search time**
(2 s at first) and 30 rounds, two objects of the order swap and the
layout is kept when it is no worse: fewer left out, fewer sheets, less
length on the last sheet, a lower sum of tops. The search is seeded, so
the same selection gives the same layout. Apply waits for the search.

### Rectangle Nesting

Objects by their bounding boxes, largest area first, each in the free
rectangle that leaves the shortest side over (on a roll, the lowest
place, then the leftmost); with **boxes may turn 90°** (on) a box may
turn when that fits better. Quick, for parts that are nearly
rectangular.

## Measure tools

They change nothing. Their results show at once in a box beside the
preview (230 wide), the first line also on the status line, without
Apply, and follow the drawing, the selection and the settings as they
change (worked out again only then). OK closes the window. Lengths and
areas are in the document's units.

### Perimeter and Area

The length of every outline (curves by arc length, the closing segment
of a closed outline included), the area by the even-odd rule within
each object (a closed outline inside an odd number of others is a hole:
the letter O counts its ring), the outer and hole areas, the box of the
selection and its area, and the counts of objects, closed and open
outlines and nodes. Objects are measured on their own: one lying on
another adds its own area.

### Cutting Time

- cutting = outline length / **cutting speed** (300 mm/s) x
  **slow-down factor** (1.2)
- moves = travel with the knife up / **travel speed** (600 mm/s) x the
  factor, the head going to the **nearest outline next** (on; else in
  stacking order) from 0,0, a closed outline entered at its nearest node
- lifts = outlines x **lift and lower** (0.2 s)
- total = (cutting + moves + lifts) x **passes** (1)

### Cost

Each object is a block, its box grown on every side by the **reserve
per side** (5 mm). The material is billed **by the roll** (at first:
the running metres of a roll of the **roll width**, 630 mm, the blocks
laid on it) or **by area** (square metres of the blocks), at the
**material price**; then, when not 0, **cutting per metre** of outline
(0.5), **weeding per m²** of the blocks (12), a price **per object**
and **machine time per hour** (the cutting time from the Cutting Time
settings). Everything is multiplied by the **copies**. Prices carry no
currency; the **currency** typed is shown after the amounts.

## Make tools

They add new objects for sign and sheet work, as one group of hairline
outlines (no fill) on the active layer, in one undo step named after
the tool; the new group is selected.

### Eyelets

Marks for eyelets (grommets), circles of the **diameter** (10 mm at
first), added as a group named Eyelets. The geometry is
`tracedraw_tracecut::eyelets`.

- **Along** **the selection's box** (at first): the box round all the
  selected objects, shrunk by the **distance from the edge** (15 mm,
  from the edge to an eyelet's centre); an eyelet in each of its four
  corners and more along its sides. An inset deeper than half the box
  puts the eyelets on its middle line.
- **Along each closed curve**: every closed outline of the selection is
  offset inward by the distance from the edge (the offset Contour
  uses); an eyelet goes on every corner of the result (a turn sharper
  than 30 degrees) and more along the stretches between corners. A
  curve with no corner, such as a circle, gets its eyelets spread
  evenly from its start. A curve too small for the inset gets none.
- **Place**: **at most this far apart** (at first, **spacing** 300 mm):
  a side or stretch of length `L` gets `ceil(L / spacing) - 1` eyelets
  between its corners; **a number per side** (**per side** 3, corners
  included, 2 to 500): `count - 2` between them (on a curve without
  corners, `count` in all). They are spread evenly by length, and
  eyelets closer than 0.001 mm are one.

The preview shows the circles; the status line counts them.

### Box Maker

The flat panels of a box with finger joints, for cutting from sheet
material with a laser or a CNC router, added as a group named Box. It
needs no selection. The geometry is `tracedraw_tracecut::boxes`.

- **Width** (200 mm), **depth** (150 mm) and **height** (100 mm) of the
  box; **sizes are** **outside** (at first) or **inside**, the room in
  the box: then the outside is `width + 2t` by `depth + 2t` by
  `height + 2t` (`height + t` with an open top), `t` the **material
  thickness** (6 mm).
- Panels: the front and the back (width by height), the left and right
  sides (depth by height), the bottom and, unless **open top** (off),
  the top (width by depth). Each is a closed outline at its outside
  size.
- Joints: along every edge where two panels meet, the length between
  the corners at its ends (`t` where a third panel meets it, 0 at an
  open top) is split into `n` equal stretches, `n` the odd number
  nearest under `length / finger width` (**finger width** 20 mm), at
  least 3 and at most 999. One panel keeps its material out to the edge
  on the even stretches and the corners, the other on the odd
  stretches, and each is cut back by `t` elsewhere, so every finger
  fills a slot of its mate. The front and the back own their joints
  with the sides, and the four walls own theirs with the bottom and the
  top, so every corner of the box belongs to the front or the back. An
  open box's walls have a straight top edge.
- A side shorter than four thicknesses makes no box: the status line
  says the box is too small for the material.
- Layout: the front, the back, the left and the right side in a row
  from the page's bottom-left corner, the **gap between panels** (5 mm)
  in from it and between them, the bottom and the top in a row above.

No allowance for the laser's kerf or the cutter's radius is made; cut
on the line with a laser, or with CNC Toolpath set to Outside.

### Folded Tray

The flat pattern of a tray folded from composite panel, such as a sign
face with returns or the body of a light box. It needs no selection.
The geometry is `tracedraw_tracecut::tray`. Its settings are kept in
the preferences (`tracecut.tray`) and come back the next time the
window opens; presets work as for every tool.

- **Face width** (600 mm) and **face height** (400 mm), finished,
  outside. **Size from selection** (greyed out with nothing selected)
  sets them to the box of the selected outlines, strokes left out.
- **Return depth** `d` (50 mm), finished, outside; with **depth per
  side** (off) the left, right, top and bottom returns (as seen from
  the front) each have their own.
- **Panel thickness** `t` (4 mm). **Groove allowance** `a`, the length
  one fold takes up: with **automatic allowance** (on) `t / 4`, else
  the value given (0 to 20 mm).
- Folds: between its fold lines the face is `(W - a)` by `(H - a)`; a
  return is `d - a/2` deep, `d - a` with ears on its edge; an ear is
  `e - a/2` long. Across the pattern the flat length is the finished
  lengths less `a` for each fold: `W + dl + dr - 2a` without ears.
- **Running through** **left and right** (at first) or **top and
  bottom**: those returns reach the corners; the other two stop the
  **corner relief** (4 mm) short of the through returns' fold lines,
  so their cut runs that far along the face's fold line. The square
  between two returns is cut away.
- **Overlap flaps** (off): each end of a through return gets a flap
  reaching **flap length** (20 mm, at most the next return's flat
  depth) past the corner, its sides the relief (at least 1 mm, at most
  a quarter of the depth) in from the return's fold line and edge, and
  a fold groove where it joins the return.
- **Mounting ears** (off) on **all returns**, **left and right** or
  **top and bottom**: **ear width** (40 mm) along the return's edge,
  **ear length** `e` (25 mm) out from it, with a fold groove on the
  edge. **By spacing** (400 mm) gives `ceil(L / spacing)` ears on a
  return whose edge is `L` long, **by count** (2, 1 to 100) that many;
  their centres split `L` evenly, half a step from each end, and an
  ear never takes more than its step less 2 mm. Each gets a hole of
  **ear hole** (5 mm, 0 for none) at **ear hole offset** (12 mm) from
  its fold line, kept 1 mm inside the ear.
- **Holes in returns** (off): **hole diameter** (6 mm) at **hole
  offset** (25 mm) from the face's fold line (kept inside the return),
  spread like the ears, at most **hole spacing** (300 mm) apart.
- **Window** (off): an opening **window inset** (30 mm) in from the
  face's fold lines, its corners rounded by **corner radius** (10 mm,
  at most half the opening).
- **Place**: **page corner** (the gap in from the page's bottom-left
  corner) or **beside the selection** (the gap to the right of the
  selection's box, bottoms level; the page corner with nothing
  selected); **gap** 10 mm.
- The pattern is drawn as seen from the back, the side that is routed:
  mirrored left to right, so the front's left return lies on the right.
- A return or an ear left no deeper than 1 mm by the allowance, a face
  side with no return left between its reliefs, or a window inset
  leaving no opening makes no pattern; the status line says which.

OK adds a group named Folded tray holding two groups: **Fold grooves**,
the open straight lines on the fold lines (the face's four sides, then
the ears' and flaps' folds), blue hairlines, the first pass with a
V-groove cutter; and **Cut**, the closed cut lines (the holes, the
window, then the outline, inner cuts first), red hairlines, the second
pass. CNC Toolpath cuts the open grooves on the line and the closed
outlines outside or inside; Cut Order and Plotter Output take them as
they take any outline. The status line gives the flat size and counts
the grooves and cut lines.

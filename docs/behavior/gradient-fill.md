# Gradient fill

## Controls and defaults

| Control | Default | Range |
|---|---|---|
| Type | Linear | Linear, Radial (elliptical), Conical, Square, Shaped |
| Stops | 2: colour 0% = object colour or black, 100% = white | 2..64 |
| Angle (linear, conical, square) | 0 (left to right, counter-clockwise positive) | -360..360 |
| Edge pad | 0% | 0..49% |
| Centre offset (radial, conical, square) | 0, 0 | -100..100% of half the bounding box |
| Repeat | None | None, Sawtooth, Mirror |
| Cycles (with a repeat) | 2 when a repeat is first chosen | 1..50 |
| Bilinear | off | |
| Invert colours | off | |
| Blend direction | Direct (straight through RGB) | Direct, Clockwise (hue increasing), Counterclockwise (hue decreasing) |
| Steps | unticked (256, smooth) | ticked: 2..255 colour bands (16 when first ticked, then the count last used) |
| Smooth transition | off | |
| Acceleration | 0 | -100..100 |

The editor shows the whole run as drawn (repeats, mirror, inversion and
the blend settings in) in a strip under the type. The Interactive Fill
property bar and the Transparency tool's gradient list offer the Shaped
type too; a shaped gradient has no handles on the canvas.

Blend settings (#1208, the rows after Bilinear and Invert colours in the
Properties docker): Blend direction as three buttons (the Blend effect's
words: Direct, Clockwise, Counterclockwise), Steps as a checkbox with
the count beside it while ticked, Smooth transition as a checkbox, and
Acceleration as a field. Each is a fill field of its own
(`FillField::BlendDirection`, `Steps`, `Smooth`, `Acceleration`), so a
change goes into each selected object's own gradient and is one undo
step (`dockers::tests::gradient_blend_rows_show_and_patch`,
`patch::tests::gradient_blend_settings_change_one_at_a_time`).

Shortcut: F11 opens the Edit Fill dialog; G is the Interactive Fill tool.

While Edit Fill or Outline Pen is open, the mouse wheel over the
drawing (outside the dialog) zooms or scrolls it as it does with no
dialog open, so the change can be looked at closely; clicks and drags
on the drawing still do nothing until the dialog closes (#1035,
`pick_drag_tests::the_wheel_zooms_the_drawing_under_edit_fill_and_outline_pen`).

Edit Fill dialog (#361, `ui/edit_fill.rs`): the fill kinds as pictures
across the top (none, uniform, gradient, pattern, texture, mesh), then
the kind's settings in a box of the same size for every kind
(`edit_fill::PAGE`, 724 x 330 points, the gradient page's room), so the
window keeps its size and place and OK and Cancel theirs as the kind
changes; it used to resize and move with each kind (#654). A page that
needs more room (a long translation) scrolls inside the box, and the
gradient band's hint wraps (`edit_fill::tests::the_window_keeps_its_size_for_every_kind`).
Uniform: the colour viewer and the colour picker's swatch, then the
colour's model (RGB, CMYK, HSB, HSL, Lab, Grayscale, YIQ; a choice converts
the colour) and a slider with a field for each of its values, and the hex
code (#468, `edit_fill::tests::a_uniform_fill_shows_its_model_and_values`).
Gradient: on the left a 180 point preview of the fill drawn by the
renderer on a checker, and the fill picker (the built-in gradients); on
the right the type, the repeat (Default, Repeat and mirror, Repeat with
a count), the band of colours with the nodes under it (click picks,
drag moves between the neighbours, double click on the band adds a node
in the colour and opacity there, on a node between the ends removes
it), Reverse (each node at `1 - pos`, midpoints mirrored), the picked
node's colour, transparency, position and midpoint, and Transform:
rotation, centre (radial, conical, square), edge pad, bilinear and
invert; a fill with free nodes from Interactive Fill gets "Back to angle
and center". The preview has the object's proportions (its box scaled to
50 mm on the longer side, a side at least an eighth of the other) and
shows the start, end and skew nodes the Interactive Fill tool shows on
the canvas, which a drag moves the same way (#939,
`edit_fill::drag_preview_node`): the start moves the fill (the centre of
an elliptical, conical or square fill whose nodes are not placed freely
sets its centre offset, kept within the box), the end turns and
stretches the axis (Ctrl in 15 degree steps), the skew node tilts it;
a drag anywhere else in the preview moves the start by as much as the
pointer moves.
The rotation and centre fields turn and move nodes placed freely, so the
fields and the preview agree (`turn_nodes`, `shift_nodes`), and OK sends
the placed nodes (or their removal) to every selected object as a fill
field of their own (`FillField::Handles`;
`a_drag_in_the_preview_turns_the_objects_gradient`). Pattern and texture: a preview and their editors. OK sends
the fields that changed to every selected object with Overprint fill in
one step (`App::apply_edit_fill`, `edit_fill_applies_what_was_changed`,
`nodes_are_added_moved_and_reversed`); with nothing selected it changes
the default fill. While the dialog is open the selection shows what it
holds: each change is applied as an engine
preview (`begin_preview`) and made anew on the next change; OK takes the
preview back and applies the fill as one step, and Cancel, Esc or the
close button take it back, leaving the drawing, the undo and redo
history and the saved state as they were (`App::preview_edit_fill`,
`the_fill_shows_live_while_the_dialog_is_open`, #436).
Dragging with Interactive Fill on an object (anywhere inside it, filled
or not) puts the start node at the press point and the end node at the
release point, keeping the kind and colours of a gradient the object
already has (otherwise a linear one from the object's solid colour, or
the default colour when it has none, to white: a red circle gives red
to white, #2498); the skew handle goes square to the line, as far from
the start as the end. A click on an object with a fill picks it and
keeps the fill (a gradient's nodes show); only an object without a fill
takes the default fill (#2498,
`fill_tool::tests::interactive_fill_starts_from_the_objects_colour`); a
click on the selected fill's line keeps that fill picked, unless another
object lies on top there, which is picked instead (#2508,
`ui::tests::interactive_fill_click_picks_the_object_over_the_selected_fill_line`); Ctrl constrains the angle to steps of the
constrain angle (15 degrees at first, #1661);
dragging a palette colour onto a stop recolours it; dragging a colour
onto the fill line adds a stop.

Property bar (Interactive Fill): the fill type (None, Uniform, Gradient,
Pattern, Texture, Mesh), then the type's fields: the colour for a
uniform fill; for a gradient the kind (Linear, Radial, Conical, Square),
the start and end colours, the angle and the edge pad. With a selection
the fields edit it; with nothing selected they set the default fill for
the next object. After the fields, Copy Fill: clicked, it waits for a
click on another object, and the selected objects take that object's
whole fill (a colour, a gradient with its stops, angle and transparency,
a pattern, a texture, a mesh fitted to each object), their outlines
untouched, one undo step; inside a group the object clicked gives it,
and a right click cancels (#2471,
`pick_drag_tests::copy_fill_gives_the_selection_the_fill_of_the_object_clicked`).
Beside it, a swatch of the selected object's fill: dragged onto another
object and let go, that object takes the whole fill the same way, the
selection staying as it was. While it is dragged the pointer shows
whether letting go changes anything and the object that would take the
fill is framed; let go over nothing or over the object it came from, it
changes nothing, and Esc drops it (#2523,
`pick_drag_tests::a_fill_dragged_from_the_bar_swatch_goes_to_the_object_under_it`).

Handles on the canvas (Interactive Fill, gradient-filled selection):
three nodes placed freely. The
start node and the end node are squares in the first and last colours,
joined by a dashed line with an arrow at the end; the skew handle is a
small round node joined to the start by a thin line. Radial fills draw
the dashed ellipse their frame makes, square fills the parallelogram.

- Start: moves the fill. A linear fill keeps its end where it was (the
  line gets shorter or longer and turns); radial, conical and square
  fills move whole, end and skew handle with the start.
- End: sets the length and the angle; the skew handle turns with the
  line. A linear fill's keeps its length; a radial, conical or square
  fill's grows and shrinks with the line, so a circle stays a circle and
  an ellipse keeps its ratio (#2373). With Ctrl or Shift the line turns
  in steps of the constrain angle about the start (a linear fill's start
  node, dragged so, turns it about the end).
- Skew handle: sets the second axis on its own, so a radial fill
  becomes an ellipse and a linear fill's bands slant.
- The line: a drag that starts on the line from the start to the end
  (within the pick distance of Options > Nodes and Handles, 6 pixels at
  first) away from the nodes and the midpoint sliders moves the whole
  fill: the start, the end and the skew handle all go as far as the
  pointer, so the line keeps its length and angle and the colour nodes
  ride on it. Ctrl keeps the move horizontal or vertical, whichever is
  longer. The nodes and the sliders win where they are on the line; the
  pointer is the move cursor over the line and the nodes; one undo step
  per drag (`App::drag_fountain_line`,
  `effect_line_tests::the_fill_line_moves_the_whole_fill_for_every_kind`).
  Linear, radial, conical and square fills alike; a shaped fill has no
  line, and pattern and texture fills have no frame on the canvas.

Colour nodes on the line (#330): every stop is a node on the line from
the start to the end, at `start + pos (end - start)` (the first is the
start node, the last the end node; the ones between are smaller
squares in their colours). Between each pair of nodes a short bar
across the line is the midpoint slider, at `pos_i + mid_i (pos_i+1 -
pos_i)`.

- Double-click on the line: adds a node there in the colour and
  opacity the fill has at that point, and picks it. Not within 0.0001
  of a node.
- Double-click on a node between the ends: removes it.
- Drag a node between the ends along the line: sets its position, kept
  0.001 inside its neighbours.
- Drag a midpoint slider: sets `mid` (0.01 to 0.99) of the blend.
- Click a node (the start and the end too): picks it. The node bar
  opens under it, compact (a slider groove of 70 points), with the
  node's colour (the swatch opens the colour picker) and the object's
  uniform transparency (0 % opaque to 100 % clear, the value of the
  Properties docker's Transparency; a run of slider changes is one undo
  step, #2507,
  `ui::tests::the_node_bar_is_compact_and_sets_the_objects_transparency`);
  the property bar shows the node's colour, transparency and position
  (the position only for nodes between the ends). A palette colour
  clicked while a node is picked goes to the node. The node bar's
  dotted grip (at its left) and the colour picker's (along its top)
  drag them off the object; the bar keeps that offset from its node,
  the picker stays where it was put while open, inside the window
  (`color_picker::tests::a_swatch_click_opens_the_popup_inside_the_window`).
- A palette colour dropped on a node recolours it; dropped on the line
  it adds a node of that colour (one undo step).

Until a node is moved the fill has no stored nodes and is placed by its
angle and offset as below; the first move stores the three nodes
(`Fountain::handles`, relative to the object's bounding box, so they
follow when the object is moved or scaled) and later moves keep them.
A drag on a node is one undo step.

## Turning and mirroring with the object

The fill is placed on the object's box, so a move or a scale carries it
along by itself. A turn, a mirror or a skew of the object (by the Pick
tool's handles, the property bar's angle and mirror buttons, the
Transformations docker, a group around it) carries the fill too, in the
same step: its three nodes are moved by the same transformation and
placed on the object's new box. When the angle and the offset place the
moved fill exactly as well (a quarter or half turn, a mirror of a
linear or radial fill) it stays placed by them, with the angle turned,
so the angle field of Edit Fill still turns it; otherwise its nodes are
stored as free nodes. A gradient transparency is carried the same way,
and a shaped fill follows the outline anyway. Before this the gradient
kept running the same way on the page whatever the object did (#775,
`command::tests::gradients_turn_and_mirror_with_their_object`,
`tests::a_gradient_is_drawn_turned_and_mirrored_with_its_object`).

## A new outline cuts the fill

A change of the outline itself (nodes moved, added or deleted with the
Shape tool, a curve's B-spline edited, an area cut off with the Crop
tool, an ellipse made a pie) keeps the fill where it lies on the page:
its nodes are mapped from the old box to the new one, so the part of
the fill over what is left stays as it was and the rest is cut off. A
gradient transparency is kept in place the same way. Before this the
fill and the transparency started again across the new box, squeezed
into it (#1491, `command::tests::a_new_outline_keeps_gradients_in_place`,
`crop_tool::tests::crop_cuts_the_gradient_transparency`). Text, groups,
tables and symbols place their paint their own way and are left as they
are.

A clip frame's own gradient transparency lies on its frame's box, so a
new outline of the frame (its nodes moved with the Shape tool) keeps
it in place the same way
(`command::tests::a_new_frame_outline_keeps_the_clip_frames_gradient_in_place`).

A picture is cropped by putting it in a clip frame (dragging its corner
nodes with the Shape tool, or the Crop tool). The picture inside keeps
its transparency and opacity, which stay on the picture's own box, so a
gradient transparency stays where it lies on the page however the
frame is cut; the frame itself drops them. Before, the frame kept a
copy of both, so the transparency was laid on twice, and the frame's
copy started again across the frame's box at each node moved (#1491,
`crop_tool::tests::cropping_a_see_through_picture_keeps_its_look`,
`shape_tool::bitmap_node_tests::moving_a_see_through_pictures_nodes_keeps_its_look`).

## Formulas

Bounding box `b` of the object in page space, centre `c`.

- Linear: axis from `c - h*(cos a, sin a)` to `c + h*(cos a, sin a)` with
  `h = (w*|cos a| + h*|sin a|) / 2`; `t = projection of p on the axis,
  normalised 0..1`.
- Radial: `t = |p - (c + offset)| / (max(w, h) / 2 * sqrt 2)`.
- Conical: `t = ((atan2(p - c') - a) mod 2pi) / 2pi`, mirrored so the
  colour at 0 and 1 is the same (`t = 1 - |2t - 1|`).
- Square: `t = max(|dx|, |dy|)` in the frame rotated by `a`, normalised
  by half the diagonal.
- Free nodes (start `s`, end `e`, skew handle `k`): the frame maps
  `(u, v)` to `s + u (e - s) + v (k - s)`; for a page point `p` take its
  `(u, v)` in that frame, then Linear `t = u`, Radial `t = hypot(u, v)`,
  Conical the mirrored angle of `(u, v)`, Square `t = max(|u|, |v|)`. A
  frame with no area is ignored (the angle and offset place the fill).
- Shaped: `t = d(p) / max d`, where `d` is the distance from `p` to the
  object's edge (0 outside) and `max d` the distance of the point
  deepest inside. The distances are exact Euclidean distances on a grid
  of at most 384 pixels across the object (`edge_distance` in the
  renderer, two passes of the 1-D lower envelope of parabolas), drawn
  smoothed; they do not change with zoom.
- The stop parameter `s` of `t` (`Fountain::stop_param`), in this order:
  1. Bilinear: `t = 1 - |2t - 1|` (first colour at both ends, last in
     the middle).
  2. Repeat with `n` cycles: `x = t * n`; Sawtooth `t = frac(x)` (1 at
     the very end); Mirror `t = frac(x)` on even cycles, `1 - frac(x)` on
     odd ones. No repeat: `n` is ignored.
  3. Edge pad `e`: `t = clamp((t - e) / (1 - 2e), 0, 1)` (per cycle).
  4. Invert colours: `s = 1 - t`.
- Acceleration `a` (-100..100, as a fraction -1..1) skews every stop's
  midpoint `m` (`Fountain::skewed_mid`): `a >= 0` gives `m (1 - a)`
  (100 pulls it to the start: the colours change fast at first), `a < 0`
  gives `m + (1 - m) (-a)` (-100 pushes it to the end). The stop's own
  midpoint from the Interactive Fill tool stays as it is; the two
  combine.
- Midpoint: between a stop with (skewed) midpoint `m` and the next, at
  `k` of the way (0..1) the blend weight is `w = k^g` with `m^g = 1/2`
  (`m = 0.5`: straight), `m` kept within 0.01..0.99.
- Smooth transition: the weight is eased, `w = 3w^2 - 2w^3`
  (`style::smoothstep`), so the colour change is flat at both stops and
  still half way at the midpoint. Black to white at 25 % is `0.15625`
  of the way (grey 40).
- Blend direction (`ColorBlend`, shared with the Blend effect): Direct
  mixes the two stops' colours in RGB by `w`; Clockwise and
  Counterclockwise go round the hue circle (`style::blend_color`): the
  hue turns from the first colour's to the second's by the way round
  that increases (Clockwise) or decreases (Counterclockwise) the hue,
  the saturation and brightness blend straight. Red to blue clockwise
  passes yellow and green (hue 0 to 240), counterclockwise magenta
  (0 down to -120). Colours without a hue (greys) blend as greys.
- Shaders and exports get every blend that does not run straight (a
  midpoint off the middle, an acceleration, the smooth transition, a hue
  direction) as 16 straight pieces along its curve
  (`Fountain::blended_stops`, `Fountain::MID_STEPS`); a hue sweep is
  straight in RGB within each sixth of the wheel, so the pieces follow
  it closely.
- Steps `n` (2..255; 256 means smooth): the stop parameter `s` is
  replaced by its band's colour position, band `i = floor(s n)` (the
  last band reaching to the very end) taking the colour at `i / (n - 1)`
  (`Fountain::band_param`), so the first and last bands are the end
  colours and a 4-step black to white fill is the greys 0, 85, 170, 255.
  The bands sit inside each repeat cycle and follow the inversion, since
  they are taken after the stop parameter.
- Opacity: each stop has one (1 opaque); it blends from stop to stop
  as the colour does. The renderer and SVG (`stop-opacity`) use it;
  PDF, EPS, EMF and `.cdr` draw the colours opaque.
- Colour at `s` is a linear interpolation in RGB between the two stops
  around it (found by bisection); stops are sorted by position; equal
  positions give a hard edge.
- Every drawing of the fill takes its colours through one method,
  `Fountain::sample_blended` (the stop parameter, the band, the sample
  in the blended stops computed once per fill), so the rasteriser's
  pixel-by-pixel fills (conical, square, shaped), the gradient shaders,
  the exports and the docker's strip agree.
- Gradient shaders and exports that know one run of stops take
  `Fountain::resolved_stops`: the same colours as stops over 0..1, with
  a sawtooth's jumps as hard edges (two stops at one position) and, when
  stepped, a pair of stops per band (`2 n` stops for one run).

## Checks

- `command::tests::gradients_turn_and_mirror_with_their_object` (core)
  and `tests::a_gradient_is_drawn_turned_and_mirrored_with_its_object`
  (render): turns, mirrors and moves carry the fill (#775).

- Given a 100x10 mm rectangle with stops black 0%, red 50%, white 100%
  and angle 0, when rendered at 1 px/mm, then pixel (50, 5) is red
  within 16/255 and pixel (2, 5) is black, pixel (97, 5) is white.
- Given a conical fill, when sampled at two angles 180 degrees apart,
  then the colours differ.
- Given a PDF export of a 3-stop fill, then the shading uses a
  stitching function with one bound at 0.3.
- Given a 40 x 20 mm rectangle with a 0 degree linear fill, then the
  start and end nodes are at (0, 10) and (40, 10) and the skew handle is
  square to the line; moving the start to (10, 10) leaves the end at
  (40, 10) and keeps the skew handle's offset from the start
  (`fill_tool::tests::linear_nodes_move_as_the_reference_moves_them`).
- Given a radial fill, moving its centre node moves the whole fill
  (`fill_tool::tests::the_centre_of_a_radial_fill_moves_it_whole`).
- Given a radial fill on a square, a drag from (5, 5) to (15, 5) keeps
  it radial and red to white, puts the start and end nodes at those
  points and the skew handle at (5, 15)
  (`fill_tool::tests::a_drag_puts_the_nodes_where_it_began_and_ended`).
- Given a black to white linear fill on that rectangle, double-clicking
  the line at (20, 10) adds a grey node at 0.5; dragging it to (30, 12)
  puts it at 0.75; the first midpoint slider dragged to (10, 10) sets
  its `mid` to 0.25 / 0.75; the node removed leaves two
  (`fill_tool::tests::colour_nodes_are_added_moved_and_removed`).
- Given the end node picked, a palette colour recolours only it; a
  colour dropped on the line at (10, 10) adds a node of it at 0.25, one
  undo step (`fill_tool::tests::a_picked_node_takes_colours_and_transparency`).
- Given a midpoint of 0.25 on black to white, the colour at 0.25 is mid
  grey and at 0.5 lighter; a stop at opacity 0 makes the fill clear
  there, on screen and in SVG
  (`midpoints_and_node_opacity_shape_the_blend`,
  `node_opacity_shows_through_the_gradient`,
  `node_opacity_goes_out_as_stop_opacity`).
- Given free nodes, the renderer, SVG (`gradientTransform`) and PDF (a
  unit shading drawn through the frame) place the fill as the nodes say
  (`free_fill_nodes_place_the_gradient`,
  `free_fill_nodes_go_out_as_a_gradient_transform`,
  `free_fill_nodes_draw_a_unit_shading_through_their_frame`).
- Given a 100 x 10 mm black to white linear fill with Sawtooth, 2 cycles,
  rendered at 1 px/mm, then pixel 48 is light and pixel 51 dark; with
  Mirror, 2 cycles (or Bilinear), the middle is white and both ends black;
  with Invert, the left end is white
  (`tests::linear_fountains_repeat_mirror_and_invert`).
- Given a shaped black to white fill on that strip, then the edge is
  dark, the middle light, and points at the same distance from the edge
  have the same colour; on a circle, rings of equal colour
  (`tests::shaped_fountains_follow_the_outline`).
- `style::tests::resolved_stops_draw_the_same_colours_in_one_run` checks
  every combination of repeat, cycles, bilinear, invert and edge pad
  against `color_at`.
- Given a 4-step black to white fill, then the colours are exactly the
  greys 0, 85, 170 and 255, with hard edges at 25, 50 and 75 %, and the
  resolved stops are a pair per band
  (`style::tests::stepped_fountains_draw_as_bands`); drawn on the
  100 x 10 mm strip (linear by the shader, square pixel by pixel) a
  pixel row holds exactly `steps` colours
  (`tests::stepped_and_hue_fountains_draw_their_colours`).
- Given red to blue, then clockwise the middle is green and a quarter of
  the way yellow, counterclockwise the middle is magenta
  (`style::tests::hue_blends_go_round_the_wheel`); the middle pixel of
  the strip is green, for a conical fill too.
- Given smooth transition on black to white, then the colour at 25 % is
  grey 40 (`smoothstep(0.25) = 0.15625`) and at 75 % grey 215
  (`style::tests::smooth_transition_eases_each_blend`).
- Given acceleration +100, then the colour is past half way at 6 %;
  +50 halves every midpoint, so black to white is mid grey at 25 % and,
  with a stop midpoint of 0.25, at 12.5 %
  (`style::tests::acceleration_skews_every_midpoint`).
- SVG: a 4-step fill is written with 8 stops, a hue blend of two
  colours with 17 and a run of more than 64 sampled stops with 64
  (`svg::tests::blend_settings_go_out_as_stops`). PDF: a hue blend is a
  type 0 function of 64 samples (3 bytes each, 4 in PDF/X-1a), a 4-step
  fill a stitching function of 4 constant segments
  (`pdf::tests::blend_settings_shade_through_sampled_and_stepped_functions`).
- The four settings round trip through `.tdraw` and are left out while
  default (`style::tests::fountains_without_the_options_read_and_write_as_before`,
  `tests::gradient_blend_settings_round_trip_in_native_files` in the io
  crate).

## Files and exports

The options are saved in `.tdraw` files (`repeat`, `repeats`,
`bilinear`, `invert`, kind `shaped`, and the blend settings
`blend_direction` (`Direct`, `Clockwise`, `CounterClockwise`), `steps`,
`smooth`, `acceleration`); they are left out when off or at their
defaults, so files without them read as before and the format version
stays.

| Format | Repeat, bilinear, invert | Blend direction, steps, smooth transition, acceleration | Shaped |
|---|---|---|---|
| SVG | worked into the stops | worked into the stops: a pair per band when stepped, else a sampled run of at most 64 stops (`svg::MAX_SAMPLED_STOPS`) | a radial gradient from the centre to the nearer sides, the inside colour at the centre |
| PDF | worked into the stitching function (hard edges leave out the empty segment) | stepped: the stitching function, one constant segment per band; a hue path, smooth transition or acceleration: a sampled function (type 0) of 64 colours (`pdf::SAMPLED_FUNCTION_SIZE`) | the renderer's pixels at 150 dpi, clipped by the path |
| EPS, EMF | worked into the stops | worked into the stops (`Fountain::to_plain`) | a square gradient from the centre out |
| `.cdr` | the closest plain gradient (`Fountain::to_plain`): the options worked into at most 64 stops, edge pad 0 | worked into the same stops | written as a square gradient with the stops reversed (inside at the centre) |

The `.cdr` byte layout does not change; a gradient without the options
is written exactly as before
(`write::tests::gradient_options_are_written_as_the_closest_plain_gradient`).

## Importing a gradient

The fountain fill editor's Import Gradient button reads a `.ggr` file
(`tracedraw_io::ggr`) and replaces the stops with it. Plain linear
segments with their middle halfway give their two ends; segments with
another blending (curved, sine, sphere, step), a middle elsewhere or an
HSV colouring are sampled into 17 stops. End colours given as the
foreground or background colour read as black and white. Transparency
in the file is not kept (fountain stops are opaque).

## Built-in gradients

The fill editor's Gradients section starts with Current colours: the
last fill colour picked to the last outline colour picked (the palette's
left and right clicks, black and white until then; `App::current_colors`,
checked by `app::tests::palette_picks_set_the_current_colours`). Then
come 29 swatches (Black to white,
Sunset, Ocean, Rainbow, Gold, Silver, Copper, Chrome, Fire, Ice, Forest,
Sky, Dusk, Aurora, Pastel, Neon, Peach, Mint, Berry, Lavender, Desert,
Steel, Sepia, Charcoal, then five landscapes: Dawn over the sea, Desert
dusk, Forest horizon, Night horizon, Snowy mountains;
`gradient_presets.rs`). A click replaces the stops and keeps the type,
angle and centre. The landscapes run from the ground or sea (0) through
a horizon near the middle to the sky (1); their swatches are drawn
bottom to top, and a click on a linear fill also sets the angle to 90
degrees so the sky is on top. Checked by
`gradient_presets::tests::every_preset_runs_from_0_to_1_in_order` and
`gradient_presets::tests::landscapes_run_ground_to_sky_at_ninety_degrees`.

## Placement in the Properties docker (#1200)

Under a fountain fill's settings, Placement puts the fill's nodes (the
start, end and skew nodes the Interactive Fill tool drags, `handles`) in
numbers:

- Rotate: the angle of the axis from the start to the end node, degrees,
  -180 to 180.
- W and H: the lengths of the axis and of the side (start to skew node)
  as percentages of their lengths in the default placement at the same
  rotation (`Fountain::default_handles`), 1 to 1000; a fresh fill reads
  100 % and 100 % whatever the box's shape, and a rotation typed in keeps
  the fill reaching across the box as the default does. The lock keeps W
  and H in proportion.
- Skew: how far the side leans from the axis's perpendicular, -89 to 89
  degrees.
- X and Y: the origin (the middle of the axis for a linear fill, the
  start node for radial, conical and square fills) from the box's
  centre, as percentages of the box's half width and half height.
- A change sets the nodes (`Fountain::set_handles`), which also keeps the
  angle and the centre offset the nearest the formats that know only
  those can show. A shaped fill has no placement.
- Each colour stop also has its opacity in the docker (0 to 100 %), as in
  the Edit Fill dialog; Overprint fill and an Edit Fill... button that
  opens the dialog sit under the fill section.

Checks: a fill without nodes reads 100, 100, 0, 0, 0, 0 for every kind and
its placement rebuilds the default nodes; W 50, H 120, rotate 30, skew 20,
X 25, Y -10 round trip through the nodes, the axis is half the default's
at 30 degrees and the fill's angle follows (tests in
`ui/fountain_place.rs`).

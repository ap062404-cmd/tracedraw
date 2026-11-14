# Drop shadow

The Drop Shadow tool puts a shadow behind an object; the shadow stays
live (it follows the object) until it is broken apart or cleared.

| Control | Default | Range |
|---|---|---|
| Presets | | Soft, Tight, Lifted, Glow, On the floor (right, left), On the wall behind |
| Kind | flat | flat, or from the bottom, top, left or right edge (perspective) |
| Offset (flat) | 2 mm, -2 mm | any |
| Angle (perspective) | 20 degrees from the bottom edge | any |
| Stretch (perspective) | 60 % | 0 to 1000 % of the object's extent across the edge |
| Fade (perspective) | 50 % | 0 to 100 % |
| Opacity | 50 % | 0 to 100 % |
| Feathering | 1.5 mm | 0 to 50 mm |
| Feather direction | middle | inside, middle, outside, average |
| Feather edges | linear | linear, squared, inverse squared, flat |
| Merge mode | Multiply | the merge modes of transparency |
| Colour | black | any |

Shadows in files saved before merge modes existed keep the Normal merge
they were drawn with; they load as flat shadows feathered in the middle
with linear edges, as before.

Saved as `.cdr`, a shadow goes out as a picture of it (150 dpi, with its
transparency; the merge mode is not in the picture) beneath the object,
which any program shows; from version 14 TraceDraw opens the object with
its shadow again (#1031, `behavior/files.md`). The app and the command
line pass the renderer's picture; where none is passed (a call of the
writer without pictures, `document_to_cdr_version`, or a shadow the
renderer gave no picture for) the writer draws it from the object's
outline, moved, feathered and shaded as below. A hidden object casts no
picture (its shadow still comes back with it), a paragraph text casts one
for the whole text, and the shadow of an object with a blend is not
written (the blend goes out alone, with a warning in the log).

## Drawing a shadow

- A drag from the middle of an object makes a flat shadow: its
  silhouette moved by the drag (offset = release point minus press
  point).
- A drag that starts in the outer fifth of the object next to an edge
  (the top and bottom edges first, then left and right) makes a
  perspective shadow standing on that edge: the edge stays where it is
  and the opposite side of the object falls on the pointer.
- A press on the edge itself or just outside it (within 11 screen
  pixels of the object's box, the reach of a handle) starts the same
  perspective shadow on the edge it is next to; a press there started
  nothing, so a shadow from the bottom, top, left or right could only be
  begun from inside the outer fifth
  (`App::shadow_drag_start`,
  `shadow_tool::tests::a_drag_from_just_outside_an_edge_stands_the_shadow_on_it`).
- While the pointer is over an object, or just outside one, with no
  drag held, the edge a drag from there would stand the shadow on is
  marked with a thick bar along it; in the middle of the object nothing
  is marked (a drag there lies the shadow flat), nor over the shadow,
  its line or its handles (`App::shadow_edge_hint`).
- One drag is one undo step. A drag that starts on one of several
  selected objects keeps the selection: each of them takes the shadow,
  a perspective one standing on its own edge (#2218). A drag on an
  object that is not selected takes that object alone.
- The drag starts on the object picked at the press, or else on the
  topmost object whose outline as drawn goes round the press: the hole
  of a ring whose inner outline runs the other way round (as the
  shaping operations leave it) picks nothing, and a press there started
  no shadow, so a ring took one only from its band. From the hole, as
  from anywhere in the middle, the shadow is flat (#2219,
  `App::shadow_drag_target`,
  `pick_drag_tests::shadow_drags_start_anywhere_inside_an_object`).
- A group or a clip frame takes a shadow of its own, drawn beneath
  everything it holds. Its silhouette is what it shows: each object of
  a group, and a clip frame's filled frame with its contents cut to the
  frame (an empty part of an unfilled frame, or contents outside the
  frame, cast none). The shadow is measured on the frame's box (the
  group's box). Break Drop Shadow Apart makes the same silhouette (#873).

- With the Pick tool a click on the drop shadow, away from its object,
  chooses the shadow on its own: it shows a box of its own, out to its
  feathered edge, with hollow handles instead of the object's, and
  Delete takes the shadow away and leaves the object selected. A click
  on the object chooses the object again (#2545,
  `pick_drag_tests::a_drop_shadow_is_chosen_apart_from_its_object`).
- An object with an envelope, a perspective or a distortion takes its
  shadow by the outline it shows: the box the tool measures on (which
  edge a press is next to, where the handles sit, where a perspective
  shadow stands and how far it reaches) is the box of the outline as
  drawn, the same the shadow is cast from, and the shadow is picked
  where it lies (`Shape::shadow_outline`, `Shape::shadow_box`). The
  tool went by the outline without the warp, so a press in the middle
  of a bent object could stand a perspective shadow on a far edge and
  the handles sat away from the object (#2217,
  `shadow_tool::tests::the_shadow_of_an_enveloped_object_goes_by_what_is_drawn`).
- An object whose live effects draw objects of their own (the body of
  an extrusion, the steps of a contour or a blend, a block shadow, the
  copies of a symmetry) casts the shadow of all of it, drawn once
  beneath all of it. The shadow is measured on the box of the object's
  own outline as drawn (of its plain outline when the effects draw the
  object among their objects, as a body turned in space does), and its
  layer reaches round everything the effects add. Before, the shadow
  was the object's alone and was drawn after what the effects put
  behind the object, so it lay over the body of an extrusion; a body
  turned in space cast none (#2215). A group's shadow takes in what
  the effects of its objects draw, and the shadow's picture (Break
  Drop Shadow Apart, PDF, `.cdr`) is the same silhouette, an envelope
  or another warp taken in
  (`tracedraw_render`
  `tests::an_extruded_object_casts_the_shadow_of_its_body_beneath_it`,
  `tests::the_shadow_picture_of_an_enveloped_object_is_of_what_is_drawn`).
  A picture with effects keeps the shadow of what its pixels show.

## Perspective geometry

With the edge's midpoint `A` (start handle), its inward unit normal
`n`, the object's extent across it `L` (height for top and bottom,
width for left and right), and the end point
`E = A + stretch L (cos angle, sin angle)`, a point `p` of the
silhouette goes to `p + s (D - L n)` with `D = E - A` and
`s = (p - A) . n / L` (`Shadow::silhouette_transform`). Points on the
edge stay, the far side lands on `E`. The opacity falls linearly along
`A` to `E` by the fade: `1 - fade * clamp(t, 0, 1)` where `t` is the
pixel's projection on `A E`.

## Feathering

The silhouette, filled with full coverage, is blurred with three box
passes of radius `feathering` (approximating a Gaussian); Average uses
one box pass of 1.7 times the radius (an even, straight ramp). The
blurred coverage `b` becomes `t`: middle and average `t = b`, outside
`t = min(1, 2 b)` (solid up to the outline, fading beyond), inside
`t = max(0, 2 b - 1)` (fading in from the outline). The edges shape it:
linear `t`, squared `t^2`, inverse squared `1 - (1 - t)^2`, flat `1`
wherever `t > 0.01`. The final alpha is `t * opacity * fade`, in the
shadow colour, merged with what lies beneath by the merge mode
(`Shadow::feather_value`; the renderer's `shadow_layer`).

## Handles

With the Drop Shadow tool the selected object's shadow shows a white
square at the start (the object's centre, or the edge's midpoint), a
square in the shadow colour at the end, a dashed line between them and
two sliders beside the line, each a tab 24 pixels long standing out from
the line: on the left of the line looking from the start to the end the
opacity slider (with a stroke of the shadow colour at the shadow's
opacity), on the right the feathering slider (with a soft stroke). Where
the two stand at the same point they make one bar across the line, one
half each.

- Dragging the end moves the offset, or the angle and stretch.
- Dragging the opacity slider sets the opacity by how far along the
  line it is (start = 0 %, end = 100 %, whole per cent).
- Dragging the feathering slider sets the feathering by how far along
  the line it is, `t`: `50 t^2` mm, to a tenth of a millimetre (start =
  0, end = 50 mm, the top of the property bar's range; 1.5 mm is at
  about a sixth, 12.5 mm half way), so the small values most shadows use
  get most of the line (`shadow_tool::feather_fraction`,
  `feather_at_fraction`).
- A slider is taken by a press within 6 pixels of its tab, from its
  foot on the line to its outer end; the one under the pointer or held
  is filled in the selection colour and labelled with its value
  ("Opacity: 50 %", "Feathering: 1.5 mm")
  (`shadow_tool::tests::the_opacity_and_feathering_sliders_lie_on_either_side_of_the_line`).
- Dragging the start to another edge stands the shadow on that edge;
  to the middle makes it flat. The end stays where it was. The start
  handle follows the pointer while it is held, flat or standing on any
  edge, and goes back to where the shadow starts when let go.
- Dragging the dashed line (within the pick distance of Options > Nodes
  and Handles, away from the squares and the sliders) moves the whole
  shadow: its end goes as far as the pointer, as a drag on the shadow
  itself does, and the start stays where it stands on the object (the
  centre or the edge). Ctrl keeps the move horizontal or vertical. The
  pointer is the move cursor over the line
  (`effect_line_tests::the_shadow_line_moves_the_shadow`).
- A palette colour dropped on the end square becomes the shadow colour
  (one undo step).
- Each drag of a handle or the line is one undo step of its own.

The squares are 12 pixels wide and a press within 11 pixels of one
takes it, whatever the zoom (#1323). Where handles lie closer than that
(a short offset, a short perspective shadow, a low opacity), a press
takes the nearest one, the end first where two lie on one spot: the end
used to take every press within its reach, so the start under it could
not be grabbed. Every handle is grabbed and dragged in every mode, flat
and on each edge
(`pick_drag_tests::shadow_handles_are_grabbed_and_dragged_in_every_mode`).

## Property bar applies at once

A change on the Drop Shadow property bar (Flat or an edge, the offset,
opacity, feathering, colour and the rest) goes to the selected objects
at once, also to objects that have no shadow yet, which then get one;
further changes merge into the same "Drop Shadow" undo step. With
nothing selected it sets the shadow new objects get. Before, a change
with no shadow there only set that default, so a flat shadow could not
be given directly (#1323,
`shadow_tool::tests::the_property_bar_applies_a_shadow_directly`).

Native ellipses, polygons and stars cast and pick their shadow by their
own outline, the same as after Convert to Curves, on every platform
(`shadow_tool::tests::native_shapes_pick_their_shadow_by_the_outline`,
`tracedraw_render` `tests::native_shapes_cast_the_shadow_of_their_outline`).

The silhouette is what the object shows: its fill, and its outline as
drawn (the band of a variable, calligraphic or placed outline, the
stroked line otherwise, with its dashes and arrowheads; a hairline one
pixel wide). An object with no fill casts the shadow of its outline
alone, so a ring drawn with the Variable Outline tool on a native ellipse
casts a ring, the same as after Convert to Curves; an object with
neither fill nor outline casts its filled outline. The shadow's layer
reaches round all the outline covers, so a wide outline's shadow is not
cut at the object's box. The shadow is still measured, and placed, on
the object's path
(`tracedraw_render` `tests::an_unfilled_object_casts_the_shadow_of_its_outline`).

The fill part is filled by the object's own fill rule, so a hole of the
object (a ring of two circles combined, the middle of a crossing star) is
a hole of its shadow, on the page, in a group's shadow and in the
shadow's picture; text, and a curve set to the nonzero rule, stay solid
where their contours overlap (#2216,
`tracedraw_render` `tests::a_drop_shadow_keeps_the_holes_of_its_object`,
`tracedraw_cdr` `shadow_image::tests::the_shadow_of_a_ring_has_its_hole`).

## Picking a shadow

A shadow is picked with its object, where it lies above what is beneath
(it is drawn over the objects under its own): a Pick tool click on the
shadow, away from the object, selects the object, as does a click with
a tool that selects on a click, so the shadow is then edited with the
Drop Shadow tool (its handles and property bar). The shadow counts inside its
silhouette, moved or sheared as it is drawn (text, groups, tables,
bitmaps, clip frames and symbols: their box), not in the feathered
fringe; a shadow of 0 % opacity cannot be picked. A Pick drag that
starts on the shadow moves the object, and the shadow with it.

With the Drop Shadow tool a click on a shadow selects its object and
shows the shadow's handles; a drag that starts on the shadow (not on a
handle or on the object) moves the whole shadow: its end goes as far as
the pointer does.

An object that casts a drop shadow of its own is never lost to another
object's shadow lying over it: a click on it picks it (Shift adds it),
with the Pick tool and with the Drop Shadow tool, so picking one
shadowed object after another never jumps to the neighbour whose shadow
falls there. A shadow still wins over an object of no shadow beneath it
(a background), as above
(`pick_drag_tests::a_click_on_an_object_with_a_shadow_never_picks_another_ones_shadow`).

## Editing a shadow

- A double click on a shadow with the Pick tool (away from its object,
  or where it lies over an object it is picked above) chooses the Drop
  Shadow tool with that object alone selected: its handles show and the
  property bar shows its settings. With the Drop Shadow tool chosen, a
  click or double click on an object with a shadow picks it.
- The handles and the property bar follow the first selected object that
  has a shadow, also with an object of no shadow picked before it; the
  bar shows the settings new shadows get only when no selected object has
  one.
- Edit Shadow, in the right-click menu of a selection with a shadowed
  object and as a button in the Drop Shadow section of the Properties
  docker (shown for such an object, with a line telling Flat or the edge
  and the opacity), does the same as the double click
  (`pick_drag_tests::a_double_click_on_a_shadow_edits_it_with_the_shadow_tool`).

## Drawing speed

A shadow is drawn on a layer of only the pixels it can reach (its moved
or sheared box widened by three blur radii and two pixels, within the
window), not one the size of the window, so a drag that changes it
every frame costs what the shadow covers. The picture is the same,
pixel for pixel.

## Break Drop Shadow Apart

Object > Break Drop Shadow Apart (Ctrl+K, which takes the place of
Break Apart while a selected object has a shadow), or Break apart on the
property bar: each selected top-level object's shadow becomes a bitmap
of its own at 150 dpi, placed just behind the object, named Shadow,
holding the shadow's colour, feathering, opacity and fade. A merge mode
other than Normal is kept as an opaque transparency with that merge
mode. The object loses its shadow. Everything happens in one undo step,
and the new bitmaps come out selected.

## PDF and printing

PDF export, Export as PDF and printing (which goes through the same
PDF) draw each shadow beneath its object as a picture of the blurred
silhouette at 300 dpi (fewer when the shadow is so large it would pass
40 million pixels), in the shadow's colour, with its feathering, fade
and opacity in a soft mask; PDF has no blur of its own. The merge mode
goes in as the PDF blend mode nearest to it (the table of
`transparency.md`, Merge modes: Multiply, Difference, Screen for Divide,
Lighten, Darken, Hue, Saturation, Color, Luminosity for Lightness,
Exclusion for Invert, Overlay, SoftLight, HardLight, ColorDodge,
ColorBurn; the others draw normally). The object stays vector on top.
The colour image downsampling asked for applies to the shadow's picture
too. A picture casts the shadow of what its pixels cover, as on the
page: a picture with a transparent background cast the shadow of its
whole box (d#2596,
`render::tests::the_shadow_picture_of_a_picture_follows_its_pixels`). An
object whose effects draw objects of their own (an extrusion's body, a
contour's steps) casts one shadow of all of it beneath all of it, as on
the page (`pdf::tests::an_extruded_object_casts_one_shadow_beneath_its_body`). PDF/X-1a and X-3 keep no transparency: an object with a shadow is
flattened with what lies beneath it, over the shadow's area (#1029).

## Checks

Checked by `shadow::tests` in the core (perspective geometry for the
bottom and top edges, flat offset, feather ramps, old files and preset
round trips, degenerate boxes), the renderer's
`tests::shadow_darkens_pixels_beside_the_object`,
`tests::shadow_merge_mode_multiplies_with_what_is_beneath`,
`tests::perspective_shadow_falls_on_the_floor_and_fades`,
`tests::clip_frames_and_groups_cast_their_shadows`,
`tests::shadows_on_their_own_pixels_match_a_layer_of_the_whole_window`,
`pdf::tests::a_drop_shadow_goes_in_beneath_its_object` and
`pdf::tests::a_drop_shadow_is_flattened_for_x1a` in the io crate,
`export::tests::a_drop_shadow_prints`, `shadow_tool::tests` in the application (drags, handles, colour drop,
presets, Break Apart with one undo step, a clip frame's shadow, picking
a shadow over what lies beneath) and
`pick_drag_tests::a_click_on_a_drop_shadow_picks_its_object` (Pick and
Drop Shadow tool clicks, a shadow dragged whole). Frame times:
`frame_times_of_a_shadow_drag`.

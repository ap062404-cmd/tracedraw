# Shape tool

Shortcut F10. Double-clicking the tool's toolbox button selects every
node of the selected curves.

A closed curve whose last segment ends back on its first node has that
node once: it shows both handles (the last segment's arriving one and the
first segment's leaving one), moves with the end lying on it, deletes by
joining the last and first segments, and Break opens the curve there.
Node counts in the status bar and the Objects panel count it once.

## How a curve shows

A curve selected with the Shape tool shows its outline in the node
colour (Options > Nodes and Handles, blue by default), its nodes as small
markers by type (squares for cusps by default) and the first node of each
subpath as a slightly larger triangle. A selected node's control
handles are dashed lines from the node on a white line twice as wide,
each ending in a solid arrow head with a white edge at its control point
(so they show on any colour, #670) (the curve tools show the handles of the curve
being drawn the same way). The arrow head is as long and as wide as a
node marker: 7, 9 or 11 pixels with Node size Small, Medium or Large
(`NodeSize::handle_head`, #1643). A press takes a node out to the
corners of its marker and a pixel beyond, and a control handle out to
the far corners of its arrow head and a pixel beyond (the pick distance
when that is more), so the reach grows with the node size; a press on a
curve with a thick outline takes its segment anywhere the outline
shows, out to its edge and the pick distance beyond (#1935,
`pick_drag_tests::nodes_and_handles_are_taken_on_their_whole_marker`,
`pick_drag_tests::a_thick_outline_takes_a_press_on_its_segment_out_to_its_edge`).
The arrow showing the curve's direction is off by default (Options >
Nodes and Handles).

With several objects selected, the one picked last shows its outline in
the node colour and the others in the secondary node colour (red by
default), also while their nodes are dragged, so the objects can be told
apart; the nodes of all of them can be chosen and moved, but end nodes of
two objects are not joined (#1007,
`pick_drag_tests::shape_tool_marks_the_last_picked_object_blue_and_the_others_red`).

An object drawn through an envelope or a perspective shows that
effect's frame only, not its own nodes, which lie where the object was
before the effect (#2213; see `envelope.md`, Envelope tool).

## Taking nodes and handles

A press takes the nearest node, or the nearest control handle of a
chosen node, within the pick distance of it: Options > Nodes and
Handles > Pick distance, 6 screen pixels by default (never less than
the node marker reaches). A control handle is also taken anywhere on
its arrow head, however long the node size makes it
(`NodePrefs::handle_pick_px`,
`pick_drag_tests::control_handle_heads_follow_the_node_size`). A segment of a selected curve is taken a
pixel further out. Distances are measured on the page, so a curve with
a scale of its own (text converted to curves, for one) is picked like
any other
(`pick_drag_tests::the_shape_tool_takes_the_nearest_node_of_a_scaled_curve`).
A click on a control handle keeps the chosen nodes as they are (it does
not choose the segment the handle lies on), and a drag on it moves only
the handle, the node staying where it is. A click takes a node before a
handle, so a handle retracted onto its node (or nearly) never swallows
a click, Ctrl+click or Shift+click on the node. A segment chosen by a click shows its two
handles (its start node's outgoing one and its end node's incoming
one), and they drag the same way (#639). A click that
takes nothing but lands within three pick distances of a node or a
handle of the selected curves (or a node of a selected rectangle,
ellipse or polygon) is a near miss: the selection stays. Only a click
further out on empty canvas deselects.

## Snapping

A dragged node snaps like a point placed by a drawing tool: to the
document grid, guidelines, the page and other objects (View > Snap To,
Options > Snapping), and to what stays put of the curves being edited:
their other nodes and the segments between two of them (the nodes being
dragged and the segments they bend are left out, #673). When several
nodes move, the one under the pointer snaps and the move follows it;
Ctrl keeps the move horizontal or vertical before and after
snapping. A dragged control point, and a node of a rectangle, ellipse or
polygon, snaps the same way.

## The pointer

A solid black arrow (drawn by TraceDraw on a white halo, its tip on the
hot spot) with a mark at its lower right for what a press would take:
a small hollow square over a node (of a curve, or of a rectangle,
ellipse or polygon, or an envelope, perspective or mesh node), a dot
over a control point of a selected node, a wave over a segment of a
selected curve, nothing elsewhere. While a node or a control point is
dragged the mark stays. Over the handles framing the selected nodes
(Stretch and Scale or Rotate and Skew Nodes) the pointer is the Pick
tool's: resize arrows, or the turning arrow at the corners and the slant
arrows at the sides, also while they are dragged.

## Right click

A right click on a node chooses it (unless it is chosen already), on a
segment the node that segment ends at (so To Line and To Curve act on
that segment), on another object that object; then the node menu opens:
Add, Delete, Join, Break, To Line, To Curve, Cusp, Smooth, Symmetrical,
Reverse Direction, Extract Subpath, Close Curve, Elastic Mode,
Properties. Delete takes the chosen nodes, the curve staying; with no node
chosen it deletes the selected object, as the Delete key does (#1982).

## Selecting nodes

| Action | Behaviour |
|---|---|
| Click node | selects it alone |
| Ctrl+click node | adds it to the node selection, or takes it out |
| Shift+click node | selects the run of nodes from the last selected one to it, along the subpath (the shorter way round on a closed one); Shift+clicking the run's end again selects the other way round; on another subpath it adds the node |
| Tab, Shift+Tab | the next or previous node of the curve, wrapping round |
| Marquee | Rectangular (default) or Freehand, picked in the property bar's Selection mode list; both are drawn as the Pick tool's marquee, selection blue dashes on a white line, so they show on blue artwork too (#1849, `pick_drag_tests::the_node_marquee_is_dashed_blue_on_white`); a freehand marquee selects the nodes inside the loop drawn (even-odd); Alt+drag draws a freehand marquee whatever the mode, also starting over a curve, and Alt+Shift adds its nodes to the chosen ones (#1341, `pick_drag_tests::alt_drag_with_the_shape_tool_is_a_freehand_node_marquee`). Any node marquee, rectangular or freehand, with Shift held adds the nodes, rectangle corners and text characters inside to the chosen ones instead of replacing them (#2331, `pick_drag_tests::shift_marquee_adds_character_nodes`). A marquee keeps the selected objects, also when it starts over another object (the one behind the curve, or the group holding it), so the curve being edited stays the one edited; only with nothing selected does a drag that starts over an object select it first (#1594, `pick_drag_tests::a_node_marquee_over_another_object_keeps_the_curve_being_edited`) |
| Right click during a marquee or freehand marquee | ignored, as with the Pick tool: the marquee goes on with the pointer while the left button is held and chooses the nodes inside when it goes up, whichever button is released first; no node menu opens and nothing of the marquee stays on the canvas (#1848, `pick_drag_tests::a_right_press_during_a_shape_tool_selection_box_opens_no_menu`) |
| Ctrl+A, Select all nodes button | every node of the selected curves |

## Moving and editing

| Action | Behaviour |
|---|---|
| Drag node | move the selected nodes; Ctrl constrains. A drag that starts on a node not yet chosen chooses it alone, with Ctrl too (Ctrl then only keeps the move to one axis, from the first drag, #1325, `pick_drag_tests::ctrl_drag_on_an_unchosen_node_moves_that_node_alone_constrained`); with Shift it adds the node (or the run up to it) to the chosen ones, which move together. While the pointer moves, the page keeps the picture it had before the drag and the curve being edited follows as the thin line of the node colour the tool draws over it (its outline, with any width nodes, brush, arrowheads or effects, is not made again every frame, #1259, #509); the picture with the moved nodes and the full outline is made once, on release, never while the pointer moves or pauses with the button held. A segment bend, a handle drag and a node transform drag the same way |
| Click segment | chooses that segment: it is drawn over in the main colour with its handles, its end node is not shown as chosen (To Line and To Curve act on the segment through it), and the spot clicked gets a small round mark; Add Node (+ or the property bar button) puts the new node at that spot |
| Drag segment | bends it: the point grabbed follows the pointer exactly while both end nodes stay; a straight (or closing) segment becomes a curve first; both control points move, the nearer one more (by `k (1 - t) d` and `k t d` with `k = 1 / (3 t (1 - t) ((1 - t)² + t²))`, `t` the grabbed point's place along the segment, kept within 0.02 to 0.98, `d` the drag); a smooth or symmetrical end node stays so, its other handle turning along, and a node made cusp keeps its other handle where it is even when its handles line up (the type chosen for the node counts, not how its handles read, #1140); one undo step (Bend Segment) |
| Drag handle | smooth nodes move the opposite handle to stay collinear; symmetrical also keep the length; with Ctrl or Shift the handle keeps to the constrain angle about its node (on the page), the node keeping its type (#510, `pick_drag_tests::ctrl_or_shift_keeps_a_handle_to_the_constrain_angle`); the page keeps its picture while the handle moves, as for a node drag (#1259) |
| Smooth, Symmetrical (property bar, node menu, C, S, Y) | a node between two curves gets its handles in line (symmetrical: of one length); between a straight segment and a curve, the curve's handle lines up with the straight segment, keeping its length, and the straight segment stays straight; between two straight segments both become curves (as To Curve makes them) and the node is smoothed, the closing line of a closed curve too, so the start node and the node before it round on both sides like every other corner (#1618, `nodes::segment_tests::smoothing_where_the_closing_line_meets_curves_both_sides`); an end node of an open curve stays as it is |
| Double-click segment | add a node |
| Double-click node | delete |
| Double-click a rectangle, ellipse or polygon | it becomes a curve; pictures, text, groups and clip frames stay as they are |
| Drag a picture's corner | the picture goes into a clip frame of its rectangle (Crop Bitmap) and the corner moves as a node of that frame; a clip frame's frame shows its nodes and edits like any curve, so a cropped picture can be reshaped further |
| Delete, Backspace, Edit > Delete, the node menu's Delete, the toolbar's Delete button | with a segment chosen by a click: deletes that segment, its nodes staying (an open curve splits there or loses its end, a closed one opens there, a curve left with no segment is deleted; one undo step, Delete Segment); otherwise deletes the selected nodes; when a deleted node sat between segments and either side was a curve, one cubic is refitted from the segment before to the segment after (least squares with the old end tangents held, Graphics Gems), so the curve is kept rather than collapsing to the far handles; the first node of a closed curve too, its last and first segments refitted into one (#472, `nodes::tests::deleting_the_start_of_a_closed_curve_keeps_the_shape`); a node between two straight segments leaves a straight join; two or more neighbouring nodes deleted together become one curve between the nodes either side, fitted by least squares to the old segments, so the outline keeps its general shape even when they were all lines: at an end where the run's segment or the kept node's other segment is a curve the old tangent is held (only the handle's length is fitted, so a smooth node stays smooth), and between lines the handle is free in both directions; a closed curve left with one node is that node alone; the whole deletion is one pass over the curve, however many nodes go (#675, `nodes::tests::neighbouring_line_nodes_deleted_together_become_a_curve`, `a_run_between_curves_keeps_the_kept_nodes_smooth`, `a_run_holds_the_tangent_where_a_curve_meets_it`, `lone_nodes_go_as_delete_node_takes_them`, `deleting_many_nodes_of_a_long_curve_is_one_pass`, `pick_drag_tests::deleting_two_neighbouring_line_nodes_leaves_a_curve`). Every route takes the chosen nodes and never the object they belong to: Edit > Delete removed the whole object while nodes were chosen (#1982, `pick_drag_tests::the_delete_key_with_a_chosen_node_takes_the_node_not_the_object`, `edit_delete_with_a_chosen_node_takes_the_node_not_the_object`, `the_node_menus_delete_with_a_chosen_node_takes_the_node_not_the_object`). With no node chosen, each of them deletes the selected object, one undo step bringing it back (`delete_with_the_shape_tool_and_no_node_chosen_removes_the_object`) |
| + / - | with nodes chosen: a node in the middle of each segment ending at them (for the start node of a closed curve, its closing segment), the chosen nodes staying chosen together with the new ones, so each press halves those segments again (a square with every node chosen: 4, 8, 16 nodes; #1326, `node_edit::tests::add_nodes_again_keeps_doubling`) / delete them (without chosen nodes they zoom) |
| C, S, Y (with nodes chosen) | C makes them cusp, S smooth and Y symmetrical, each key one type whatever the nodes were (C used to turn a cusp node smooth and S to make nodes symmetrical, so neither gave the type it stands for, #2210, `pick_drag_tests::node_keys_in_the_shape_tool`); while nodes are chosen these letters do not align objects or pick tools. The type chosen is kept with the curve (saved in `.tdraw`), so a cusp node may keep aligned handles and its handles still move apart; nodes never given a type read as their handles do (aligned: smooth; aligned and of equal length: symmetrical), and a curve that gains or loses nodes goes back to that |
| Shift+L, Shift+U, Shift+C, Shift+M, Shift+Y, Shift+B, Shift+J (with nodes chosen) | To Line, To Curve, Cusp, Smooth, Symmetrical, Break Curve, Join Nodes, as the property bar's buttons; also Object > Nodes (with + Add Node), so Options > Customization > Commands lists them and can give them other keys (#2001, `pick_drag_tests::node_commands_have_keys_of_their_own`) |
| Home, End (with nodes chosen) | the first or last node of the curve |
| Arrow keys (with nodes chosen) | move the chosen nodes by the nudge distance (Shift: super nudge, Ctrl: micro nudge), one undo step per press |
| To Line, To Curve (property bar, node menu) | the segment ending at each chosen node becomes a straight line or a curve with its handles at thirds; for the start node of a closed curve that is its closing segment, the last one back onto the start or the closing line the close itself draws, which becomes a curve of its own before the close (#1618, `nodes::tests::to_curve_and_to_line_take_the_closing_segment_through_the_start_node`, `shape_tool::bitmap_node_tests::to_curve_takes_the_closing_segment_through_the_start_node`) |
| Break (property bar) | split the subpath at the node; a closed curve opens there instead, as one open curve that starts and ends at the node, its other nodes (its first node too) staying joined and a closing line the close drew staying as a line; with several nodes of a closed curve chosen it opens at one and splits at the others (#1618, #2314, `nodes::tests::breaking_at_the_start_of_a_closed_subpath_opens_it_there`, `nodes::tests::breaking_a_closed_curve_at_a_middle_node_opens_it_there`, `nodes::tests::breaking_a_closed_curve_at_two_nodes_gives_two_pieces`) |
| Join (property bar) | join two selected end nodes of one curve into one node halfway between them, their handles moving along and no segment added: the two ends of one open subpath close it there, ends of two subpaths make one subpath; one undo step (Join Nodes). End nodes of two objects are not joined: the status bar says to combine them first (Object > Combine, Ctrl+L), after which they are two subpaths of one curve (#1007, `pick_drag_tests::join_takes_end_nodes_of_one_object_only`) |
| Drop an end node on another | dragging one end node and letting go within the pick distance of another end node of the same curve joins the two as Join does; on an end node of another object it only snaps there (#1007) |
| Extend Curve to Close | two selected end nodes are joined by a straight segment: the ends of one subpath close it, ends of two subpaths make one |
| Extract Subpath | the subpath of the selected node becomes its own curve |
| Reduce nodes | nodes go while the curve stays within 0.3 screen pixels of its original shape everywhere along it (not only where a node was): the stretch between two nodes that stay is fitted as one segment and split at an original node while it strays too far, then single nodes go, the one that changes the curve least first. The ends of an open curve stay; the start of a closed one can go. Of the curves with selected nodes (those nodes only), else of every selected curve (`nodes::reduce_tests`) |
| Curve smoothness (slider, 0 to 100) | drag it to remove nodes: the tolerance is (v / 100)² × 10% of the curve's larger side, worked from the curve as it was when the drag began, the selected nodes only when some are selected; one undo step per drag |
| Reduce nodes | RDP simplification (0.3 screen pixels) of the curves with selected nodes, else of every selected curve |
| Curve smoothness (slider, 0 to 100) | drag it, type a value or step it with the wheel or arrows to remove nodes: the tolerance is (v / 100)² × 10% of the curve's larger side, worked from the curve as it was at the slider's first change, the selected nodes only when some are selected; the slider keeps its value, and all its changes are one undo step, while nothing else changes; an undo, an edit of the curve or other objects or nodes chosen put it back at 0 over the curve as it then is (#1326, `node_edit::tests::the_smoothness_slider_keeps_its_value_between_changes`) |
| Align Nodes | a dialog with Align horizontal and Align vertical: the selected nodes move onto a common line. The align keys and the Object > Align commands work on two or more selected nodes too (#364): L, R to the leftmost or rightmost node's x, T, B to the top or bottom node's y, E (Align Centers Horizontally) to the middle of the nodes' box in y, Align Centers Vertically to its middle in x (the menu; the C key stays Cusp); nodes on several curves compare on the page; handles move with their node; one undo step "Align Nodes" (`align_keys_align_selected_nodes_in_the_shape_tool`) |
| Ctrl+C, Ctrl+X | with selected nodes, the segments between them are copied (cut: taken out of the curve, which opens) as new curves styled like their source |
| Ctrl+D | the same segments as new curves at the duplicate offset |

## Node transforms

The Stretch and Scale Nodes and Rotate and Skew Nodes buttons are
toggles. With one of them on and two or more nodes selected, eight
handles frame the selected nodes:

- Stretch and Scale: corner handles scale in proportion about the
  opposite corner, side handles stretch about the opposite side; Shift
  works about the frame's centre.
- Rotate and Skew: corner handles (curved arrows) rotate about the
  frame's centre by the angle the pointer turns; side handles (double
  arrows) skew along their side, the shear being the side's travel over
  half the frame.

Only the selected nodes and their handles move; each drag is one undo
step.

## Reflect and Elastic

- Reflect Nodes Horizontally and Vertically (toggles): moving a node
  moves the selected nodes on the other side of the selection's middle
  (vertical axis for horizontal reflection, horizontal axis for
  vertical) the mirrored way, so two sides of a shape widen or narrow
  together.
- Elastic mode (toggle): selected nodes move by
  `w = 1 - 0.5 × d / dmax` of the drag, `d` being a node's distance from
  the grabbed node and `dmax` the largest such distance; the grabbed node
  moves fully, the farthest one half as much.

## Corner widgets

A curve being edited shows a small white circle with a ring in the node
colour inside each corner (a node where two segments meet at an angle,
as the Corners panel counts them), 18 px from the node along the middle
of the corner (#635), when Options > Nodes and Handles > Show corner
rounding rings on curve nodes is on. That preference is on at first; it
can be turned off so the widgets do not get in the way of node editing
(#884), and it is saved with the other preferences. With it off no widget shows and a press where
one would sit works as any other press of the Shape tool. Rectangles
round their corners with their own handles either way.

- Every corner has one (a curve with more than 32 corners shows them
  only on chosen nodes, so a traced outline is not covered in them). A
  chosen corner's widget is filled with the node colour.
- A widget shows only where it can do something: a sharp corner whose
  largest rounding on its own (the shorter side times `tan(theta / 2)`)
  moves the corner's middle less than 2 screen pixels has none, so a node
  where the curve barely turns, or between very short sides, shows no
  widget that a drag would leave unchanged; zoomed in far enough, it
  shows (`tracedraw_core::corner_cut::corner_reach`,
  `tools2::tests::corner_widgets_show_only_where_they_round_something`).
  A side is measured through the smooth nodes of the curve, as far as
  the next corner (at most 8 segments): a corner with a smooth node
  close beside it, as the shaping commands leave where two circles
  cross near a node of one, has its widget (#2225,
  `corner_widgets::tests::a_corner_beside_a_smooth_node_shows_its_ring_and_rounds`).
- Options > Nodes and Handles > Round one corner at a time (on at first,
  under the rings option): a Ctrl+click (Cmd on macOS) on a widget
  chooses its corner, a second Ctrl+click takes it out, so one corner or
  several can be chosen. Turned off, the widgets work as before: with
  nodes chosen only their corners show one, a drag rounds every corner
  or the chosen nodes' corners, and Shift and Ctrl change nothing.
- Dragging a widget into its corner rounds that corner alone with a
  fillet, live. When its node is one of the chosen nodes, the chosen
  nodes' corners round together; with Shift held at the press, every
  corner of the curve rounds together. The radius is the one whose arc's middle has moved into
  the dragged corner as far as the pointer has moved along it
  (`r = d / (1 / sin(theta / 2) - 1)` for a corner of angle `theta`), at
  most the largest radius every rounded corner still fits (a side two
  rounded corners share is split between them). Dragging back takes the
  rounding back off. The status bar shows the radius. With a curved
  side the arc is the circle of that radius tangent to both sides (see
  `corners-and-joins.md`), so its middle lies nearer the corner or
  further than the pointer, and the largest radius is the largest
  circle that touches both sides within them (#2223). A side runs on
  through the smooth nodes of the curve to the next corner, and the
  arc with it: the segments it passes are taken whole, so the rounding
  does not stop at a node and leaves no sliver of a segment before one
  (#2224, #2226). Between sides that bend away from each other (the
  notch where two welded circles meet) every circle touches both,
  flatter the larger it is; there the largest radius is the one whose
  arc still turns a quarter of what the corner turns.
- Open curves round their inner corners the same way.
- The drag is one undo step, "Fillet". The rounded corners become arcs
  of the curve (nodes and segments, as the Corners panel makes them); the
  chosen nodes are cleared.
- A rounded corner keeps its widget, so its radius can change again
  later (#872). A rounded corner is a circular arc (one cubic or more,
  smooth where they join) between two straight sides, tangent to both,
  whose sides, run on, meet as far from both ends of the arc; the
  radius is read back from the arc. A corner rounded between curved
  sides, or a curve and a line, is one too (#2223,
  `corner_widgets::tests::corners_between_curves_keep_their_widgets_and_round_again`):
  an arc of up to six cubics tangent to both sides, whose sides carried
  on along their own curves (the same cubic past its end) cross at the
  sharp corner; an arc whose neighbour bends as it does (a quarter of a
  circle) is part of a round shape, not a rounded corner: the
  neighbour bends within a tenth of what the arc's radius says where
  they join and keeps within 4% of that radius of the arc's circle all
  along, and every piece of the arc bends within a twentieth of it. A
  circle cut by the shaping commands keeps its own nodes, and its
  pieces are none (a quarter circle drawn as one cubic bends 2.1% less
  at its ends than its radius says; #2222,
  `corner_cut::tests::pieces_of_round_shapes_are_not_rounded_corners`,
  `corner_widgets::tests::a_shape_cut_out_of_discs_keeps_one_ring_per_rounded_corner`).
  A side that
  is an arc of a circle (it turns 14 degrees or more, alone or with the
  segments it runs on through, and keeps to the circle within a
  thousandth of its radius) may have lost whole segments to the
  rounding; where its own cubic carried on and the circle part by more
  than a hundredth of the way to the corner, the circle is the side,
  and taking the rounding off draws it as an arc beside the side
  (#2224,
  `corner_cut::tests::a_fillet_runs_on_through_the_smooth_nodes_of_a_side`).
  The crossing of the sides is looked for from where the arc's
  tangents meet and then from nearer the arc, up to 5000 times a
  side's own length past its end, so a corner rounded as far as it
  goes (the arc nearly half a turn, or a side cut down to a
  thousandth of itself) keeps its widget (#2211,
  `corner_widgets::tests::a_corner_rounded_as_far_as_it_goes_keeps_its_ring`).
  Its widget
  sits 18 px off the
  middle of the arc, along the middle of the corner. Dragging it takes
  the corner back to the sharp corner its sides meet at and rounds that
  again, starting from the radius it had: that corner alone, the chosen
  nodes' corners when it is one of them, or with Shift every corner of
  the curve (sharp or rounded) takes the new radius (a rounded corner is
  chosen by any node of its arc). Dragging back past the corner makes it sharp. Scallops
  and chamfers are found again only right after they were cut (below).
- On a stretched or skewed curve corners round on the page, so a new
  rounding is round, also on a curve stretched after other corners were
  rounded. A corner rounded before the stretch keeps its stretched arc
  and its widget: dragging it changes that arc in the curve's own space,
  that corner on its own, until it is dragged back to sharp; rounded
  again, it is round (#1000,
  `pick_drag_tests::corner_widgets_round_stretched_curves_on_the_page`).
- The arcs are found from the curve itself, so a drawing saved and
  opened again, or a curve rounded with the Corners panel, keeps its
  widgets too; no extra data is stored.
- A plain click on a widget chooses its corner alone, and its widget is
  drawn filled (#2057); a click with Shift, Ctrl or Alt keeps the
  selection (Ctrl+click chooses as above).
- The property bar sets the rounding by value while a curve with a
  corner is selected (and the rings option is on): Corner rounding, a
  radius field in the drawing unit, shows the radius of the chosen
  corners, or of every corner with none chosen (0 for a sharp one,
  empty when they differ); a value typed there rounds those corners to
  it on the page, each rounded corner first taken back to its sharp
  corner, at most the largest radius every one of them still fits (the
  status bar shows the radius used), and 0 makes them sharp. The Clear
  Rounding button beside it makes them sharp. One undo step each
  ("Fillet", "Clear Rounding"); the chosen corners stay chosen (#2057,
  `pick_drag_tests::the_property_bar_sets_the_corner_rounding_of_curves`).
- Three buttons before the field choose the kind of corner it makes
  (#1147, `App::corner_bar_op`, Round at first): Round Corner as above,
  Scalloped Corner or Chamfered Corner, which cut the same corners as
  the Corners docker does (`corner_cut::cut_corners`), the value being
  the distance along each side; a distance some corner cannot take is
  cut down by tenths until every one of them takes it. One undo step
  ("Scallop", "Chamfer"). The curve as it was before the cut is kept
  while the curve stays as the cut made it (#2336,
  `corner_widgets::tests::scalloped_and_chamfered_corners_stay_editable`):
  the field then shows the distance, a new distance (typed or scrolled)
  or the other kind cuts that curve again at the same corners, so the
  nodes of one cut stay (before, each try cut the cuts and a rectangle
  grew to 120 and more nodes), 0 or Clear Rounding gives that curve
  back, and each corner cut keeps one ring in the middle of its cut
  (the corners the cut made have none). Dragging a ring makes the
  bar's kind of corner: round, or scalloped or chamfered with the cut's
  middle under the pointer, cut down as the field cuts it. Any other
  change of the curve ends this: its cut corners are then corners like
  any other.
- Checked by `pick_drag_tests::corner_widgets_round_every_corner_or_the_chosen_ones`,
  `pick_drag_tests::corner_widgets_follow_their_preference_off_by_default`,
  `pick_drag_tests::rounded_corners_keep_their_widgets_and_round_again`,
  `corner_cut::tests::rounded_corners_are_found_and_taken_back_off`,
  `corner_cut::tests::chosen_rounded_corners_are_taken_off_alone`,
  `corner_cut::tests::other_curves_are_not_rounded_corners`
  and `tracedraw_core` `corner_cut::tests::corners_and_the_largest_fillet_they_take`.

## Rectangles, ellipses and polygons

| Action | Behaviour |
|---|---|
| Drag a rectangle corner node | every corner grows or shrinks by the same amount, in the current corner style, until two corners fill a side; a square corner has one node, a cut one a node at each end. A node that falls on another is shown once: a corner rounded as far as its neighbours leave it room (a square's corner at its side's length) ends on their nodes, so a square with one corner at the most shows 3 nodes, the ends of the arc and the far corner, not 5 with two pairs stacked; the end of the arc stays, so dragging it brings the rounding back, and two cut corners meeting in the middle of a side share one node there (#1851, `kind_nodes::tests::a_corner_rounded_to_the_most_leaves_no_stacked_nodes`) |
| Ctrl+drag a corner node | that corner only, as far as the neighbouring corners leave it room on both its sides (past half the shorter side when they are smaller, a whole side when they are square; #1757, `kind_nodes::tests::one_corner_dragged_takes_what_its_neighbours_leave`); Ctrl counts as it is held during the drag, also pressed after the button (#1325) |
| Click a corner node; Shift+click or Ctrl+click; marquee or freehand marquee over corner nodes | chooses that corner alone; adds the corner to the chosen ones or takes it out; chooses the corners with a node inside (on every selected rectangle). Chosen corners' nodes are filled (#671) |
| Drag a chosen corner node | the chosen corners only, all by the same amount, so two or three corners round together and the others keep theirs (#671) |
| Drag an ellipse node | pie when the pointer is inside the ellipse, arc outside; a whole ellipse's node starts at the top and the way it turns picks which end it becomes; with Ctrl held (at the press or later) the angle goes in steps of the constrain angle (15 degrees at first; a whole ellipse's node counts its turn from the top) (#1340). An arc is an open curve: with the document's Fill open curves off it draws no fill on screen, in SVG and PDF, and is picked on its outline only; a pie keeps its fill (a live test of 0.41, `kind_nodes::tests::an_ellipse_made_an_arc_draws_no_fill_unless_open_curves_are_filled`) |
| Double-click an edge of a polygon or star | it becomes a curve and a node goes in at the spot, so no Convert to Curves is needed first (#2451, `pick_drag_tests::double_click_on_a_polygon_edge_adds_a_node`) |
| Drag an edge of a polygon or star | on the first move it becomes a curve and the edge bends as a curve's segment does; a plain click on the edge converts nothing (#2451, `pick_drag_tests::dragging_a_polygon_edge_bends_it`) |
| Drag a polygon node | its mirrored nodes follow: a vertex moves every vertex in or out (the points between them stay, and keep their turn); a point between two vertices moves all of those: in or out sharpens the star, and through the centre it comes out on the far side, an inverted star (negative sharpness, down to -0.99; dragged back through the centre it is a plain star again). The pointer's turn is read on the near or the far side of the centre, whichever is nearer the turn the points had at the press, so a drag straight through inverts the star rather than turning its points half a turn; with Ctrl the side is the one the pointer is on along the kept turn. It used to stop short of the centre, sharpness held to 0.001..0.999 (#1331, `kind_nodes::tests::inner_nodes_cross_the_centre_into_an_inverted_star`). Around the centre turns them (a twisted star; within 1 degree of straight it stays untwisted; Ctrl keeps the turn they have, held at the press or later; it was read at the press for rectangle corners only, so it did nothing on a star, #1331) |

Rectangles, ellipses and polygons show these nodes and the property bar
shows their controls (corners, pie and arc, points) while the Shape tool
is active, after the object's kind ("Rectangle", "Ellipse", "Polygon";
a curve shows "Curve" before the node buttons). The status bar says so
too: "Rectangle: drag a corner node to round the corners (Ctrl+Q
converts to curves)", and the like for ellipses and polygons, where a
curve shows "Curve: n Nodes". The curve being edited is also drawn in
the node colour with a larger first node, which a rectangle, ellipse or
polygon never has. Each drag is one undo step. Other objects show a Convert to
Curves button.

## B-splines

A curve drawn with the B-spline tool keeps its control points
(`Shape::spline`, local space). With the Shape tool it shows its outline
in the node colour, the control polygon as dashed lines in the secondary
node colour between the control points (the closing line too when the
spline is closed), and a node at each control point; the curve's own
nodes do not show and cannot be taken.

| Action | Behaviour |
|---|---|
| Click a control point | chooses it alone (drawn filled); Shift+click adds it, Ctrl+click adds or removes it |
| Marquee (or lasso) | chooses the control points inside, of every selected B-spline |
| Delete | removes the chosen control points, the curve kept (a spline left with fewer than two goes); one undo step "Delete Nodes" (`pick_drag_tests::bspline_control_points_are_chosen_and_deleted_alone`) |
| Drag a control point | the spline follows (`Command::SetSpline`: the outline becomes the spline of the points, open or closed as it was); a chosen point drags every chosen point of its spline with it; one undo step "Edit B-Spline" for the drag |
| Double-click the control polygon | a control point there, between the two the line joins, at the foot of the pointer on the line |
| Double-click a control point | removes it (two always stay) |
| C, or Cusp on the property bar, or Fixed Control Point on the right-click menu | makes the chosen control points fixed: the curve passes through a fixed point, with a corner there between the lines of the control polygon (the point counts three times in the cubic spline, `geometry::bspline_path_fixed`); a fixed point has a second frame around its node. C again, S (Smooth), or Floating Control Point makes them floating again: the curve only comes near them. One undo step "Fixed Control Point" or "Floating Control Point"; the outline stays a B-spline. The flags (`Shape::spline_fixed`, saved in `.tdraw`) follow their points through moves, added and removed points; the ends of an open spline are always on the curve (#1758, `pick_drag_tests::a_bspline_control_point_is_made_fixed_and_floating`) |
| Click the control polygon | keeps the selection, as a click near a node does |
| Convert To Curves (Ctrl+Q), or any other change of the outline (node editing, the brushes, a join) | the outline stays as it is and the control points go: an ordinary curve with the spline's nodes |

A B-spline drawn on from the end node of another curve becomes part of
that curve, and keeps no control points, as the other curve tools' lines
do.

## Text

With a text object selected the Shape tool shows a node per character
and the two spacing arrows; the property bar shows the chosen
characters' horizontal and vertical offsets and angle. See `text.md`,
Shape tool on text.

## Property bar

Selection mode | Add Node, Delete Node | Join Nodes, Break Curve | Convert
to Line, Convert to Curve | Cusp, Smooth, Symmetrical | Reverse
Direction, Extend Curve to Close, Extract Subpath, Close Curve | Stretch
and Scale Nodes, Rotate and Skew Nodes, Align Nodes | Reflect
Horizontally, Reflect Vertically, Elastic Mode | Select All Nodes |
Reduce Nodes, Curve smoothness | Round, Scalloped or Chamfered Corner,
Corner rounding, Clear Rounding (a curve with a corner selected, see
Corner widgets).

## Checks

- `pick_drag_tests::a_node_drag_keeps_the_pages_picture_until_release`
  (#1259): no page render while the pointer moves or pauses, one on
  release; `a_handle_drag_keeps_the_pages_picture_until_release`
  the same for a control handle; `frame_times_of_a_node_drag_with_a_rich_outline`
  (ignored, release) times a node drag on a curve with width nodes and
  arrowheads beside 300 texts.
- Given a rectangle converted to curves, when its two right corners are
  chosen and deleted together, then two nodes are left joined by a curve
  bulging out towards the old right side, and Undo brings the four back
  (`pick_drag_tests::deleting_two_neighbouring_line_nodes_leaves_a_curve`,
  `nodes::tests::neighbouring_line_nodes_deleted_together_become_a_curve`,
  `nodes::tests::a_run_through_the_start_of_a_closed_curve_becomes_a_curve`,
  `nodes::tests::runs_on_an_open_curve`).
- `pick_drag_tests::corner_widgets_round_every_corner_or_the_chosen_ones`
- `tracedraw_core` `corner_cut::tests::corners_and_the_largest_fillet_they_take`
- `pick_drag_tests::overlays_are_drawn_on_a_white_line`
- `pick_drag_tests::nodes_are_taken_within_the_pick_distance_and_a_near_miss_keeps_the_selection`
- `pick_drag_tests::the_shape_tool_says_a_rectangle_is_still_a_rectangle`
- `pick_drag_tests::a_right_click_in_the_shape_tool_takes_the_node_or_segment_under_it`
- `pick_drag_tests::dragging_a_segment_bends_it_and_a_click_marks_where_a_node_goes`
- `nodes::tests::bending_a_segment_moves_its_point_and_keeps_the_ends` (core)
- `nodes::tests::bending_a_segment_keeps_chosen_cusp_nodes_cusp` (core)
- `pick_drag_tests::c_s_and_y_set_the_type_of_the_chosen_nodes`
- `pick_drag_tests::the_pointer_shows_what_a_press_would_do` (marks, node
  transform handles)
- `node_edit::tests::ctrl_toggles_and_shift_selects_a_run`
- `node_edit::tests::handles_scale_rotate_and_skew_the_chosen_nodes`
- `node_edit::tests::reflect_and_elastic_moves`
- `node_edit::tests::segments_copy_cut_and_duplicate`
- `node_edit::tests::a_freehand_marquee_picks_the_nodes_inside`
- `node_edit::tests::extend_to_close_and_smoothness`
- `kind_nodes::tests::dragging_a_corner_rounds_all_corners_or_one`
- `pick_drag_tests::a_handle_clicked_then_dragged_leaves_its_node_in_place`
  (#639)
- `pick_drag_tests::a_chosen_segments_handles_drag` (#639)
- `pick_drag_tests::a_click_on_a_chosen_node_with_a_retracted_handle_takes_the_node`
  (#639)
- `kind_nodes::tests::chosen_corners_round_together` (#671)
- `kind_nodes::tests::single_corner_needs_ctrl_and_chosen_corners_go_together`
- `pick_drag_tests::ctrl_held_after_the_press_constrains_ellipse_and_star_nodes`
- `pick_drag_tests::chosen_rectangle_corners_round_together` (marquee,
  Shift+click, drag; #671)
- `kind_nodes::tests::rounded_corners_grow_by_the_same_amount`
- `kind_nodes::tests::corners_drag_in_page_units_on_scaled_rectangles`
- `kind_nodes::tests::the_ellipse_node_opens_a_pie_inside_and_an_arc_outside`
- `kind_nodes::tests::polygon_nodes_move_together`
- `kind_nodes::tests::points_between_the_vertices_twist_the_star`
- `kind_nodes::tests::inner_nodes_cross_the_centre_into_an_inverted_star` (#1331)
- `pick_drag_tests::a_bspline_keeps_its_control_points_for_the_shape_tool`
- `tracedraw_core::command::tests::a_spline_follows_its_control_points_until_the_outline_is_changed_otherwise`
- `geometry::tests::twisting_turns_the_points_between_the_vertices` (core): the
  vertices stay, the points between them keep their distance and turn by
  the twist; a plain polygon turned gains points at its sides' middles.

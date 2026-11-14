# Variable Outline tool

The Variable Outline tool (its own button in the toolbox, right under
the brush tools' flyout) makes an outline thicker or thinner along its
length with width nodes.

## Using it

- Select an object with a path (a curve, a rectangle, an ellipse, text
  converted to curves). A press away from the selected object's outline
  selects the object under the pointer.
- Drag across the outline: a width node is added where the drag starts
  (or the node under the pointer is picked) and the outline reaches to
  the pointer on both sides. With Shift (or Alt) held, only the side the
  pointer is on changes (#374); it counts while the drag goes on, so
  pressing or letting go of it mid-drag switches between one side and
  both (#492).
- A drag that starts on a node's centre slides the node along the path
  when it first moves along the path (kept between its neighbours), and
  changes its reach when it first moves across it; a drag from one of a
  node's reach handles (the dots at its ends) changes its reach. One undo
  step per drag (#479,
  `width_nodes_slide_along_the_path_and_their_handles_change_the_reach`).
  On a closed path (one closed subpath: an ellipse, a rectangle, a closed
  curve) a node slides on past the path's start and comes round, kept
  between the nodes either side of it round the loop; the nodes keep
  their path order and the picked ones stay picked (#1322,
  `a_node_slides_round_a_closed_path_past_its_start`).
- Picking several nodes: Shift+click picks the nodes from the picked one
  to the one clicked; Ctrl+click adds a node or takes it out; a plain
  click picks one alone. A drag that starts on a picked node changes
  every picked node the same way, and the property bar's widths set
  them all (`shift_and_ctrl_pick_nodes_and_shift_drags_one_side`).
- A press on the open path's end cap, within the outline's reach of an
  end, puts the node on the end itself (or takes the node already
  there), so a widened end keeps a cap as wide as the end. A node a
  little short of the end let the reach fall back to half the outline
  width past it, a bulb with a small cap on a neck (#2547,
  `a_press_on_the_end_cap_widens_the_end_itself`).
- Each drag is one undo step.
- Delete (or Backspace) removes the picked nodes, one step. An object with no
  outline gets a hairline one when its first node is added.

## Property bar

- Left width and Right width: how far the outline reaches on each side
  of the path at the picked node, in document units. The left is to the
  left of the path's direction.
- Delete Node removes the picked node; Clear Variable Outline removes
  every node of the selected objects, giving back the plain outline.

## Defaults and formulas

- A node sits at `t` (0 to 1) of the path's length over all its
  subpaths. A new node takes the reach the outline already has there.
- The ends of the path keep half the outline width on each side unless
  a node sits on them. The reach runs through the nodes (and those ends)
  on a monotone cubic (Fritsch and Carlson): between neighbouring nodes
  `a` and `b` a cubic Hermite curve with, at each node, the weighted
  harmonic mean of the slopes to its neighbours when they rise or fall
  the same way, else 0, and 0 at the ends of an open path. Through a run
  of nodes, each wider than the last, the reach sweeps on smoothly and
  never overshoots; at a node wider or narrower than both neighbours it
  is flat, so with one or two nodes it eases as `s = u^2 (3 - 2u)`,
  `u = (t - t_a) / (t_b - t_a)`. Each span eased that way on its own
  made a run of nodes a staircase, more bumps with every node added
  (#2547, `style::tests::a_run_of_width_nodes_widens_in_one_sweep`).
  A closed path (one closed subpath) has no ends: the last node and the
  first are neighbours across the path's start, and the reach eases
  between them the same way (the last node a turn back before the first,
  the first a turn on after the last) (#1322,
  `style::tests::width_around_a_loop_eases_across_its_seam`,
  `shaping::tests::a_closed_loops_variable_band_runs_on_across_its_seam`).
  A single node on a loop tapers as on an open path: its reach eases to
  half the outline width at the point half way round the loop
  (`style::loop_anchor`) and back. When the tool adds a second node to
  such a loop it also sets a node at that point, with that reach, so the
  far side keeps its taper (a live test of 0.41: a single node gave the
  whole loop its reach, so a closed object did not taper,
  `shaping::tests::a_closed_loops_variable_band_is_a_tapering_ring`).
- The band is built from the path flattened to 0.05 mm and cut into
  pieces no longer than 1/256 of its length (at least 0.05 mm), with
  `n = sqrt(6 dw / (8 * 0.005))` pieces (48 to 512) between
  neighbouring nodes whose reach differs by `dw` mm, however short that
  span, so the pieces stay within 0.005 mm of the eased edge (the ease is
  a smooth curve, not a few straight pieces; #492, #776,
  `shaping::tests::variable_band_eases_smoothly_between_close_nodes`),
  split at
  the nodes; corners get round joins on their outer side, the arc in
  steps of `2 acos(1 - 0.005 / r)` for a reach `r` (at most 256), so it
  stays within 0.005 mm of the circle. The ends of an open path take the
  outline's line cap (Outline Pen, Caps): butt ends stop at the end, a
  round cap is a half circle across the reach there (left plus right, so
  an uneven reach gets a cap spanning both sides), a square cap a box as
  deep as half that reach; a closed path has no ends and no caps. The
  band used to end square whatever the cap (#2390,
  `shaping::tests::a_variable_outline_keeps_the_line_caps`). The inner side of
  a corner is covered by the pieces either side: a round join there, as
  wide as the reach at the corner, stood out where the reach falls away
  after the corner and left a notch where it met the band (#776,
  `shaping::tests::a_wide_corner_node_leaves_no_notch_inside_and_a_smooth_arc_outside`,
  `a_reach_wider_than_the_curve_fills_its_inside`). A bend of the
  flattened path under 0.5 radians is part of a curve, not a corner: the
  segments either side take the halfway normal at it and the normal
  turns smoothly along each segment (its pieces cut finer where it turns,
  so their far edge stays within 0.005 mm of the arc), with no join. On
  each side a point reaches no further than where its normal meets the
  normal of the point next to it (the curve's centre there), and a piece
  whose own normals meet within the reach of both its ends runs out to
  that point: a reach on the inside of a curve wider than its radius
  fills the curve up to its centre. A reach past the centre zigzagged
  into a staircase edge with loose slivers and stood out past the far
  side of a loop (a live test of 0.41,
  `shaping::tests::a_closed_loops_variable_band_is_a_tapering_ring`).
  Across the point where a closed path closes the band runs on as
  anywhere else: nodes either side of it, or on it, give the same reach
  just before and just after it, with no step and no open end there
  (`shaping::tests::a_variable_band_tapers_across_the_closing_point`,
  `render::tests::a_variable_outline_on_an_ellipse_tapers_across_its_closing_point`).
  Holes in the band smaller than `(0.05 r)^2` for the widest reach `r`
  are filled. Each point of the band is worked out once and shared by
  the pieces either side, and every piece has its ends on the path as
  corners, so the pieces and joins meet without cracks (#627,
  `shaping::tests::a_variable_band_along_a_curve_has_no_cracks`).
- With "Scale with object", the reach scales with the object like the
  width.
- On screen the page is drawn in strips on several threads, each
  leaving out the objects that cannot reach it. A band counts as
  reaching as far as its widest node, however thin the outline's own
  width, so no strip leaves part of it out (#626,
  `tracedraw_render::band_tests::a_variable_outline_is_drawn_whole_across_the_strips`).
  Inside a group or a clip frame the same holds: a group is left out of
  a strip only when every object in it is (groups inside groups too),
  and a clip frame when its frame is, since the group's own bounds count
  no outline and used to cut a variable or wide outline inside it flat
  at the strip's edge
  (`tracedraw_render::band_tests::outlines_inside_groups_are_drawn_whole_across_the_strips`,
  `tracedraw_render::culling_tests::objects_out_of_sight_are_skipped`).
- The outline's colour, or its gradient, pattern or texture fill, paints
  the band. Convert Outline to Object (Ctrl+Shift+Q) gives the band as an
  object: built from the path flattened to 0.005 mm and fitted to a few
  curves, its butt ends straight lines and its round caps kept (#627,
  `tools2::tests::a_variable_outline_becomes_a_band_of_few_curves`). SVG and PDF exports write the band as a filled path; `.cdr`
  files are written with the plain width.
- A reach on the inner side of a bend that comes near the bend's radius
  (90 % of it or more) brings the two sides of the inner edge to a point:
  the hole of a ring turns heart-shaped. That is the shape, as in any
  offset, but it is taken for a fault, so while a drag changes a node's
  reach a note beside the pointer says why it happens and that dragging
  back rounds it off (`status_hint.variable_outline_fold`;
  `variable_outline::tests::the_tool_knows_when_the_inside_comes_to_a_point`,
  `variable_outline::tests::a_drag_past_the_centre_of_a_circle_brings_the_note`).

# Corners and Join Curves panels

## Corners (Window > Panels > Corners)

Fillets, scallops or chamfers the corners of the selected objects.

| Control | Default | Notes |
|---|---|---|
| Operation | Fillet | Fillet, Scallop or Chamfer |
| Radius | 2 mm | Fillet: the arc tangent to both sides, its centre equidistant from them. Scallop: measured from the corner, the notch is a quarter of that circle |
| Chamfer distance A, B | 2 mm, locked | A back along the side the path comes in on, B along the side it leaves on (the order the path is drawn in); the lock makes B equal to A |
| Preview | on | draws the result on the drawing; remembered in the settings |
| Apply | | disabled when nothing selected has a corner the values fit |

- A corner is a node where two segments meet at an angle; smooth and
  symmetrical nodes are not corners.
- Every corner is cut, unless the Shape tool is active and nodes of a
  curve are selected: then only those. Rectangles, ellipses, polygons and
  text become curves (the panel never selects their nodes).
- Corners are cut in the order the path is drawn. A corner whose sides
  are not long enough for the cut, counting the cuts already made on
  them, is skipped; the next corner may still fit.
- For a corner of angle `theta` between straight sides a fillet of
  radius `r` starts `r / tan(theta / 2)` from the corner on each side.
  With a curved side the arc is the circle of radius `r` tangent to both
  sides: its centre is where the two sides, each moved `r` towards the
  inside of the corner, meet (found by Newton steps from where straight
  sides would be touched), and each side is cut where the circle touches
  it. A radius larger than a side's own bend there, or whose circle
  touches a side beyond its end or beyond a cut already made, does not
  fit and the corner is skipped (#2223,
  `corner_cut::tests::fillets_between_curved_sides_are_found_and_taken_back_off`,
  `corner_cut::tests::the_largest_fillet_between_curved_sides_is_the_circle_that_fits`).
  Arcs are cubic Beziers, one per quarter turn.
- A side of a corner runs on through the smooth nodes of the curve, as
  far as the next corner (at most 8 segments). A fillet whose circle
  does not touch both segments at the corner is looked for further
  along the sides, the Newton steps going on from one segment to the
  next, and the segments between the corner and the arc are taken
  whole. A corner beside a smooth node (where the shaping commands cut
  a circle near one of its nodes) therefore rounds as far as its sides
  go, not only to the node (#2224,
  `corner_cut::tests::a_fillet_runs_on_through_the_smooth_nodes_of_a_side`,
  `corner_cut::tests::corners_of_shapes_cut_out_of_circles_round_and_are_found_again`).
  Scallops and chamfers still keep to the two segments at the corner.
- Scallops and chamfers measure their sizes along the sides, curved or
  straight.
- Corners are cut on the page: on a stretched or skewed object a fillet
  or scallop is round and the sizes are page millimetres, and the curve
  keeps its transform (#1002,
  `corners::tests::fillets_on_a_stretched_object_are_round`). The cut is
  part of the curve: it is not live and does not change if the object
  is stretched later.
- While the panel is open and Preview is ticked, the result is
  previewed on the drawing as a dashed outline in the selection colour.
  Untick Preview to see the objects as they are with the panel still
  open (#1759); Apply works the same either way.
- One undo step ("Fillet", "Scallop" or "Chamfer").

Rectangles keep their own corner settings when rounded from the property
bar or with the Shape tool (`shape-tools.md`); the panel is for curves.

## Join Curves (Object > Join Curves)

Joins the ends of the selected curves, and the open subpaths inside them.

| Control | Default | Notes |
|---|---|---|
| Mode | Extend | Extend, Chamfer, Fillet, Bezier Curve |
| Gap tolerance | 5 mm | ends further apart are not joined |
| Radius | 2 mm | Fillet only |

- The nearest pair of free ends within the gap is joined first, then the
  next, until no pair is left. The two ends of one subpath close it.
- Extend: both ends carry on along their directions to where they meet;
  a straight end moves there, a curved one gets a straight piece. When the
  directions do not meet ahead of both ends, a straight line joins them.
- Chamfer: a straight line between the ends.
- Fillet: as Extend, then the corner is rounded with the radius (skipped
  when the sides are too short).
- Bezier Curve: a curve leaving and arriving along the ends' directions,
  handles a third of the gap long.
- The joined curve replaces the curve selected last (with a marquee
  selection, the bottom-most curve) and keeps its fill and outline; the
  other curves are removed. One undo step.
- Nothing within the gap: nothing changes and the status bar says so.

## Checks

- `tracedraw_core` `corner_cut::tests` (areas of each cut, skipped
  corners, smooth nodes, chosen nodes, curved sides, bad values)
- `tracedraw_core` `join::tests` (each mode, gap, closing, several
  pieces into one closed outline)
- `corners::tests::the_docker_cuts_rectangles_as_curves_in_one_step`
- `corners::tests::the_shape_tool_limits_the_cut_to_chosen_nodes`
- `corners::tests::join_curves_keeps_the_last_selected_curve`

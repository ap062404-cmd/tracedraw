# Shape brushes: Smooth, Smear, Twirl, Attract and Repel, Smudge, Roughen

Drag over a selected curve (objects that are not curves are converted
first) to reshape the outline within the nib. Only selected objects
change: with nothing selected a drag does nothing and the status bar
says "Select an object first, then drag over it with the brush"; a
click selects (see "Clicks with drawing tools" in `toolbox.md`;
`brushes::tests::brushes_change_only_the_selected_objects`). The whole
drag is one undo step; the next drag is another.

## Common behaviour

- The nib is a circle of the bar's Nib size (radius, 10 mm at first)
  around the pointer on the page. The brush works on the outline as it
  lies on the page (the object's transform applied) and the result is
  taken back into the object's own space, so the nib, the moves and the
  settings' millimetres are what is seen whatever the object's scale: a
  10 mm nib reshapes 10 mm of a curve scaled five times, not 50 mm.
- The nib is also sized on the drawing. A drag that starts with Shift
  held (Shift down before the button) sizes it instead of brushing: the
  circle stays where the button went down and its edge follows the
  pointer across, right grows it, left shrinks it, by the distance the
  pointer moved on screen; the pointer shows a left-right arrow, the
  status bar the new size and the bar's Nib size follows. Shift pressed
  later in a stroke still swaps Twirl's turn and Attract and Repel. The
  wheel over the drawing with Shift held steps the nib by 10 % a notch
  (up grows it); while Options sets the wheel to scroll instead of
  zoom, where Shift+wheel scrolls sideways, Alt+wheel does it. Ctrl and
  the plain wheel keep their zoom or scroll. The nib stays between 0.5
  and 500 mm (`brushes::wheel_resizes`,
  `pick_drag_tests::shift_drag_and_the_wheel_size_the_brush_nib`).
- The effect falls off from full at the centre to nothing at the edge of
  the nib (`w = 1 - d / radius`; Attract and Repel ease it to nothing,
  `s = w^2 (3 - 2 w)`, so the curve bends smoothly at the nib's edge).
- Only the stretch of a curve within the nib's reach changes: the pieces
  of segments within the nib and a quarter of its radius beyond (the
  points there do not move, so the new stretch joins the old curve
  smoothly) are measured into points (a sixth of the nib apart for Smear
  and Twirl, an eighth for Smudge, a twelfth for Attract and Repel, half
  a millimetre for Smooth), moved and fitted back into curves, and the fitted curves are
  merged into as few as keep the stretch within 0.1 mm (0.01 mm for
  Attract and Repel, whose held steps move a few hundredths of a
  millimetre; `fit::fit_path`, whose splitting is bounded, so a long stroke never
  hangs, #1133; Smooth merges
  within its own tolerance), so a stroke leaves a handful of nodes, not
  one every few millimetres (`effects::brush_runs`, `fewer_curves`). A
  piece out of reach shorter than one and a half times the reach is
  rebuilt along with the piece in reach next to it, so a stroke that
  advances step by step does not leave a node, and a sliver of the old
  curve, at every step (`LEFTOVER_SHARE`). The pieces out of reach
  beyond that, and the subpaths out of reach, stay exactly as they were,
  nodes and handles included; open curves stay open, closed ones closed
  (a closed curve touched in part starts from a piece out of reach).
- Curves fitted through points (`geometry::smooth_path`, also the
  Freehand tool's) take the tangent at a point from its neighbours and
  handles a third of each segment long, so a long segment next to a
  short one neither overshoots nor flattens.

## Tools and their bars

Every bar starts with Nib size and ends with "Drag over a selected
object". The settings are kept in the preferences (`tools.brush_radius`,
`tools.roughen_amount`, `tools.brush`).

| Tool | Bar | What a step does |
|---|---|---|
| Smooth | Rate 1 to 100 % (20) | each point of the stretch in reach eased towards the average of the stretch around it (a quarter of the nib's radius each side, nearer points counting more) by three quarters of `w`, so jags and wobbles melt away a little with every step; then the points that matter within `0.05 + 1.45 (rate - 1) / 99` mm are kept (Ramer-Douglas-Peucker at half of it), a smooth curve fitted through them and merged into as few curves as stay within that tolerance |
| Smear | Pressure 1 to 100 % (100) | points move by the pointer's move times pressure times `w^2` |
| Twirl | Speed 1 to 100 % (50); Counterclockwise or Clockwise (Alt or Shift while dragging turns the other way) | points turn about the pointer by `speed / 1000` rad per mm the pointer moved, times `w`, and while the button is down by as much again as if the pointer moved 2π / 0.05 mm a second (at 50 % one whole turn a second at the centre, as CorelDRAW's Twirl winds the outline round; it went on as if the pointer moved 10 mm a second, a turn in 12 s, and read as slow, #2640). Held in one place, the turn so far is worked out afresh from the outline the hold found, its points measured `radius / 6 / (1 + turn)` apart, so a hold of several turns comes back as one smooth spiral rather than many small twirls each fitted again (`brushes::hold_tests::a_held_twirl_winds_the_outline_round_several_times`); a move starts afresh from the outline as it is |
| Attract and Repel | Speed 1 to 100 % (50); Attract or Repel (Alt or Shift while dragging swaps them) | the points of the stretch move straight towards (away from) the pointer, the same in every direction, by a dose `f` (0 to 1) that grows by `2 x speed x 0.006 / radius` per mm the pointer moved: a point `d` from the centre goes to `d (1 - f s)` (Attract, never past the centre) or `d + f s (radius - d) / 2` (Repel, never past the nib's edge); both keep the points' order along each ray, so the outline never folds. Each move works on the outline as it is; held still, it works from the outline the hold found, the dose growing as if the pointer moved 10 mm a second, so it pulls or pushes at a steady pace and comes to rest at `f = 1` (a 10 mm nib at 50 % after 1.7 s) |
| Smudge | Pressure 1 to 100 % (100) | the nib carries the outline under it along: the points within half the nib's radius of its centre go all the way with the pointer (times the pressure), and from there to the nib's edge the carried share eases to nothing (`u^2 (3 - 2 u)`), so a stroke draws the outline out into a tongue as wide as that core, the piece of outline it took at its end and its sides running smoothly into the outline that stayed. Each move of the pointer is made in small moves of an eighth of the radius, each from the outline the one before left, so a quick stroke (a few long moves) and a slow one give the same outline, and no small move shifts neighbouring points past each other (`effects::smudge`, see below) |
| Roughen | Frequency 1 to 10 spikes per 10 mm (5); Spike size (2 mm) | the whole stroke is worked out from the outline it began with and the places the nib went (a new place each half spacing the pointer moves): points `10 / frequency` mm apart along each subpath, counted from its start, are pushed off the curve along its normal by up to the spike size times `w` from the nearest place (the normal of the way the outline runs from the last place before the point to the first place after it that lie twice the larger of spacing and spike size away in a straight line, so on an outline an earlier stroke made jagged the spikes still stand out from the object rather than along the old spikes), the push from the point's place, and joined by straight lines so the spikes stay sharp; the nodes and the segments out of reach stay exactly. Going back over the same place or holding the button gives the same spikes, so a stroke adds at most one node per spacing of the stretch it covered. If the jagged outline then crosses itself or another closed subpath its box meets, each subpath that crosses itself becomes everything inside its outer edge (bits smaller than a spacing squared left out) and they are put together by the even-odd rule: every spike stays filled and a hole made of another subpath stays a hole (#2229: the even-odd fill showed the overlaps as white slivers over the object). Closed subpaths no bigger than the spike size are left as they are (`effects::roughen_stroke`) |

## Smudge

Smudge used to be a smoothing pass (0.2 mm) followed by one Smear push
of the whole pointer move, with Smear's weight, which peaks at the
nib's centre. What a stroke left then depended on how many pointer
events it came in: a quick stroke of a few long moves left a thin spike
behind the nib's centre, and a slow one, smoothed again at every event,
hardly moved the outline (a 10 mm nib dragged 30 mm out of a square's
side: a bulge of 11 mm in 1 mm moves). Now the outline under the nib's
core goes with it whatever the speed: the same drag gives a tongue 30 mm
long, 10 mm wide at its middle and near its end, in 1 mm moves, in 6 mm
moves or in one (#2228,
`tracedraw_core::effects::tests::smudge_draws_a_tongue_whatever_the_speed`).
Dragging along a thin part (a ring's wall narrower than the nib's core)
carries both of its edges, as a finger through wet paint does.

The nib pushes what lies before it: the outline behind its centre (the
pushing front goes as far as the pressure lets the outline follow)
stays where it is, easing in over a tenth of the radius. A stroke from
inside a shape draws its edge out, one from outside dents it, and a nib
wide enough to cover the far edge where the stroke starts no longer
drags that edge along behind it. Only the stretch of points the nib
moved is fitted back into curves; the outline before and after it
keeps its segments exactly. Refitting the whole stretch in reach at
every pointer event let the edge beside the nib creep and wobble, the
more the slower the stroke (#2546,
`tracedraw_core::effects::tests::smudge_leaves_the_outline_away_from_the_nib`).

A step measures the stretch in reach into points an eighth of the
radius apart and fits it back, as a Smear step does. On a stroke that
winds a ring's wall three times round it takes 0.4 ms a step in a
release build, far inside a frame; the old step took 0.07 ms there and
left 19 elements, having smoothed the stroke away.

## Checks

- `tracedraw_core::effects::tests::brushes_touch_only_the_subpaths_they_reach`
- `tracedraw_core::effects::tests::brushes_leave_the_segments_out_of_reach_exactly`
- `tracedraw_core::effects::tests::smooth_fits_jagged_lines_within_the_nib`
- `tracedraw_core::effects::tests::a_smooth_stroke_along_a_wobbly_spiral_leaves_few_nodes_on_it`
- `tracedraw_core::effects::tests::long_smudge_and_twirl_strokes_stay_light`
- `tracedraw_core::effects::tests::smudge_draws_a_tongue_whatever_the_speed`
- `brushes::tests::a_stroke_is_one_step_and_follows_the_settings`
- `brushes::tests::attract_repel_and_twirl_follow_their_modes`
- `brushes::hold_tests::attract_and_repel_come_to_rest_while_held`
- `tracedraw_core::effects::tests::attract_and_repel_are_the_same_in_every_direction`
- `tracedraw_core::effects::tests::attract_and_repel_stay_within_the_nib`
- `tracedraw_core::effects::tests::a_roughen_stroke_adds_spikes_once`
- `tracedraw_core::effects::tests::roughen_leaves_no_white_slivers`
- `pick_drag_tests::attract_works_from_the_press_and_settles_while_held`
- `pick_drag_tests::roughen_adds_detail_up_to_a_cap_per_stroke`
- `brushes::page_space_tests::the_nib_is_measured_on_the_page_whatever_the_object_scale`
- `brushes::page_space_tests::smooth_leaves_the_outline_outside_the_nib_alone`
- `ui::brush_bars::tests::brush_settings_scale_and_stay_in_range`
- `ui::brush_bars::tests::every_brush_bar_draws`

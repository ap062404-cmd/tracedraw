# Align and Distribute

The Align and Distribute panel (no key of its own since Ctrl+Shift+A
became Paste Style From, #1345; Options > Shortcuts can give it
one), the Object > Align and
Distribute menu and the keys L, R, T, B, E, C and P (with Shift they
distribute, see Distribute).

## Align

Six picture buttons: left edges, centres on a vertical line (the same
x), right edges; top edges, centres on a horizontal line (the same y),
bottom edges. Under them, what they align to:

| Align to | Target |
|---|---|
| Active object (default) | the last object selected; with one object selected, the page |
| Page edge | the page's edges (left edges to its left edge, centres to its middle, ...) |
| Page center | the page's centre point (left edges at its x, top edges at its y, ...) |
| Grid | each object on its own: the edge or centre goes to the nearest document grid line, counted from the ruler origin every grid spacing |

- Use Outline (off): the boxes aligned are the objects' bounds grown by
  half their outline width, so the outer edges of the outlines meet. A
  group counts its bounds only.
- The menu and the keys always align to the active object; P (Center to
  Page) always takes the page's centre.
- Each Align is one undo step ("Align"); objects already in place are
  left out of it.

## Distribute

Eight picture buttons: left edges, centres, gaps and right edges along
x; top edges, centres, gaps and bottom edges along y. Where the objects
are spread:

| Distribute to | Needs | Ends |
|---|---|---|
| Selection extent (default) | 3 objects | the first and last objects (ordered by the edge or centre distributed; by centre for gaps) stay; the others move so the edges or centres are evenly spaced, or the gaps are equal between the first's low edge and the last's high edge |
| Page extent | 2 objects | the first object's low edge (left, bottom) touches the page's, the last object's high edge (right, top) touches the page's; edges and centres in between are evenly spaced, or the gaps are equal across the page |

- With fewer objects the buttons are pale and a line says how many are
  needed.
- Use Outline applies here too.
- The menu distributes between the outermost objects, and so do the
  keys: Shift and the letter that aligns the same way (Shift+L, Shift+R,
  Shift+T and Shift+B the edges, Shift+E the centres across, Shift+C the
  centres down), Shift+P equal gaps across and Shift+A equal gaps down
  (#2147,
  `pick_drag_tests::shift_and_the_align_letters_distribute_the_selection`).
  With the Shape tool and nodes chosen the Shift letters stay node
  commands.
- Each Distribute is one undo step ("Distribute").
- Set spacing (#2005): the first object stays and the others follow it
  the spacing apart, as gaps or between edges or centres. Under the
  spacing box is a slider from 0 to 100 mm (in the ruler unit; the box
  takes any value), and the wheel over the box steps it by a tenth of the
  unit. Once a Distribute button has spread the selection with a set
  spacing (the button and the selection are kept in
  `App::distribute_last`), a new spacing from the box, the wheel or the
  slider spreads the same selection again at once, as a preview on the
  drawing (`Engine::begin_preview`; each new value takes the previous
  preview's place, so a frame costs one distribution only when the value
  changes). The adjustment is kept as one undo step when the slider is
  let go, the box is left (Enter or a click elsewhere) or the pointer
  leaves the spacing row after the wheel; Esc puts back the spacing and
  the drawing with nothing added to the history, and so does the docker
  closing or showing something else, or the selection changing, before
  it was kept (`align_docker::settle_gap`, once a frame). Without a
  Distribute button used on the selection the spacing only changes.
  Check: `ui::align_docker::tests::the_set_spacing_spreads_the_selection_live_in_one_undo_step`.

## Checks

- `ui::align_docker::tests::align_follows_the_target_the_docker_shows`
- `ui::align_docker::tests::use_outline_aligns_the_outer_edges_of_the_outlines`
- `ui::align_docker::tests::distribute_spreads_edges_centres_and_gaps`
- `ui::align_docker::tests::the_docker_draws_with_and_without_a_selection`

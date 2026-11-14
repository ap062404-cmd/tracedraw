# Pick tool

| Action | Behaviour |
|---|---|
| Click | select the topmost object under the pointer (its fill counts only when filled; otherwise its outline within 3 px, and a thick outline anywhere it shows, out to 3 px beyond its visible edge, at the width it is drawn (scaled with the object when the outline scales with it, all of the width on the outer or inner side of a closed object whose outline lies outside or inside it, the widest part of a variable-width outline; #1678, #1935, `pick_drag_tests::an_outline_reaches_as_far_as_it_is_drawn`, `pick_drag_tests::a_click_on_a_thick_outline_picks_the_object_away_from_its_centre_line`); text, tables and bitmaps count anywhere inside their bounds; a group or a symbol only where one of its objects is, and a clip frame on its frame (inside it too when it is filled or with Treat all objects as filled) and where its contents show, so a click in the empty part of a group on top takes what lies inside or behind it, and one in the gap between grouped unfilled objects takes nothing, #885, #2329, `pick_drag_tests::a_group_on_top_is_picked_only_where_its_objects_are`, `pick_drag_tests::the_gap_between_grouped_unfilled_circles_picks_nothing`, `pick_drag_tests::a_clip_frame_is_picked_on_its_frame_and_its_contents_only`); an object with live effects counts where it is drawn: its warped outline and the objects its effects add, and its selection box encloses them |
| Shift+click | add or remove from the selection |
| Ctrl+click | on a clip frame lying on a layer: Edit Contents, the contents taken out of the frame and the one under the pointer selected (all of them off the contents), see `clip-frame.md` (#982); otherwise the object inside a group, one level further in at each click (the first level inside the topmost group, then the group or object inside that, down to the innermost object); its handles are round, and moving, scaling, colouring or deleting it changes that object only (a group left empty goes); once chosen it is dragged on its own from anywhere on it without Ctrl, while a press on another part of the group takes the whole group (#849, `pick_drag_tests::object_chosen_in_a_group_drags_alone_from_anywhere_on_it`); Ctrl+Shift+click adds one more; a group and an object inside it are never selected together |
| Alt+click | the object beneath the selected one under the pointer, round to the top again after the bottom one (dig); with nothing there selected, the object beneath the topmost one, so one Alt+click on the object in front takes the one hidden behind it (the topmost itself when it is alone there; #1350, `pick_drag_tests::alt_click_takes_the_object_behind_the_front_one_at_once`); Ctrl+Alt digs through the objects inside groups and clip frames (a clip frame comes after its contents); Shift+Alt adds it |
| Click on a selected object (any of a selection), or on the centre mark | toggle the rotate/skew handles: curved arrows at the corners, straight double arrows at the sides, the centre of rotation in the middle |
| Drag on empty | marquee (a dashed blue frame); objects wholly inside are selected; with Alt held when the drag ends, every object the frame touches: its outline crosses or lies in the frame, or the frame lies in its fill (text, bitmaps, tables and clip frames anywhere in their box; a group when one of its objects counts), but an object the frame merely lies in is left out when the frame crosses or holds the outline of another, so a frame drawn round a small object over, or hidden behind, a larger filled one takes the small object alone (#1503, `select_region::tests::a_touching_drag_leaves_out_the_object_it_merely_lies_in`); with Alt held at the press the frame starts also over an object, selected or not (the object in front of a hidden one, a background under the objects; without Alt a press on an object moves it; #1341, #1350, `pick_drag_tests::alt_drag_selects_also_when_it_starts_over_an_object`, `pick_drag_tests::alt_drag_over_the_selected_front_object_selects_the_one_behind`); Shift keeps the selection and turns over the objects the frame takes (selected ones are deselected, the others added; the Freehand Pick lasso too, #401); Ctrl keeps the marquee square; with Options > Tools > Pick, Marquee selects touched objects, Alt works the other way round |
| Drag the centre mark (the "x") | moves the selection, also where the mark lies on nothing (an outline-only ring) |
| Space | from another tool, picks the Pick tool; on the Pick tool, goes back to the tool used before it (the Pick tool's key, whatever it is bound to in Options, works the same way); ignored during a drag (it drops a copy there) and while text is edited. A tool key held down counts once: the presses the keyboard repeats while it is held pick nothing (#1502, `pick_drag_tests::holding_space_drops_one_copy_and_picks_once`) |
| Drag object | move with the pointer (snapped); Ctrl (Cmd on macOS) or Shift keeps the move horizontal or vertical, whichever is longer (Shift since #2480; not with Alt, as Alt+Shift copies the outline on the drop); Space drops a copy where the selection is and the move goes on (also while scaling, turning or slanting: the copy takes the transform reached), one copy a press, however long Space is held; let go over the document palette to add the objects' colours to it instead of moving them |
| Corner handle | scale proportionally from the opposite corner, by where the pointer falls along the box's diagonal: `s = (|sx| w² + |sy| h²) / (w² + h²)` with `sx`, `sy` the two sides' own ratios (each keeps its sign, for mirroring), so the corner follows the pointer and a wide, low box such as a line of text does not grow at the short side's pace (#383); the handle follows the pointer from the frame the drag starts, not in a jump of the few pixels before a press becomes a drag; Shift from the centre; Ctrl in whole steps of 100 % (crossing the other side gives the mirror image at the same size); Alt scales freely |
| Side handle | stretch one axis from the opposite side (across it: mirrored); Shift from the centre; Ctrl in steps of 100 % |
| Snapping while scaling | with snapping on, the dragged corner or side handle snaps to other objects' points and edges, guidelines, the grid and the page (never to the selection's own points), and the snap mark shows where |
| Rotate handle (rotate mode, corners) | rotate about the centre of rotation; Ctrl in steps of the constrain angle (15 degrees); without Ctrl the pointer snaps to other objects' points only, never to edges or lines (#933, see `snapping.md`) |
| A node of the selection (rotate mode; also a segment's midpoint or an ellipse's quadrant, the points the pointer marks) | dragged, turns the selection about the centre of rotation, the node going round its circle towards the pointer (the turning arrows show over it); near another object's point the turn aims at it, near an edge the node lands where its circle crosses the edge, exactly on it; Ctrl in steps of the constrain angle; elsewhere on the selection a drag moves it as outside rotate mode (#933, `pick_drag_tests::a_node_dragged_in_rotate_mode_turns_the_selection_onto_a_line`) |
| Skew handle (rotate mode, sides) | slant: the top and bottom handles sideways, the left and right ones up and down, the opposite side fixed; Shift about the centre line; Ctrl in steps of the constrain angle |
| Centre of rotation (rotate mode, a ring with a dot) | drag it to turn about another point; it is kept with the object: rotate mode going off and on, clicking elsewhere and selecting the object again find it where it was put, and it travels with the object when that is moved, scaled or turned (kept in the object's own space; several objects selected together keep theirs as a place within their bounds; each drawing has its own; not saved in the file); double-click puts it back in the middle; a click on it keeps rotate mode. Dragged, it snaps (unless snapping is off) to the corners, the middles of the sides and the centre of the selection's box and to the points of every object near it, the selection's own and others' (nodes, midpoints, quadrants, centres), whether Snap to Objects is on or not, and the point taken is marked with its name (#2352, `snap::tests::the_centre_of_rotation_snaps_to_object_centres_and_edges`). A transform of the selection carries a moved centre with it, and a copy turned (or moved) by a drag ended with a right click gets the centre the original had, carried by the same transform. Edit > Repeat of a turn turns about the selection's moved centre when it has one, so turned copies repeated go round that point (#2353, `pick_drag_tests::repeat_turns_copies_about_the_moved_centre_of_rotation`); without a moved centre a turn repeats about the same place of the selection's box (#1289) |
| Right click during a move, scale, rotation or skew | the original stays and a copy takes the change (the copy becomes the selection); a second right click takes it back; the status bar says which, and the pointer carries a plus in a square (beside the box of a move, at the lower right of the scale, turn and slant pointers). The transform runs until the left button goes up, whichever button is released first or both in one frame, and wherever it is dropped (bare canvas, another object): the release is never taken for a click there, a held second right press does not start it again, and a palette swatch under the pointer does not take the right click as an outline colour (#528, `pick_drag_tests::a_right_click_copy_survives_any_release_order`, `a_right_click_copy_dropped_on_another_object_copies_the_dragged_one`, `a_right_click_during_a_ctrl_mirror_leaves_a_mirrored_copy` (Ctrl and a side handle dragged across: a mirrored copy), `a_right_click_during_a_turn_or_a_slant_leaves_the_original`, `a_held_second_right_press_takes_the_copy_back_and_the_move_goes_on`); in the browser too, where the left button let go while the right one is held sends no pointer release (#1334, `a_left_release_while_the_right_button_is_held_ends_the_drag`) |
| Right click during a marquee or a Freehand Pick lasso | no context menu or drop menu opens: the frame goes on while the left button is held and selects when it goes up, whichever button is released first or both in one frame (#1015, `pick_drag_tests::a_right_press_during_a_selection_box_does_not_leave_it_frozen`) |
| Right button held during a marquee | the marquee moves with the pointer, keeping its size, instead of being sized; with the right button up again the pointer sizes it from where it was moved to, and the left release selects what it encloses there. The Shape tool's node marquee moves the same way; a Freehand Pick lasso goes on being drawn (#2050, `pick_drag_tests::the_right_button_moves_the_selection_box_while_it_is_held`) |
| Esc during a move, scale, rotation or skew | cancels it; nothing changes |
| Arrows | nudge by the nudge distance (2.54 mm default); Shift by the super nudge distance (5.08 mm), Ctrl by the micro nudge distance (0.254 mm); all three in Document Setup > Rulers; with guidelines selected and no object, the guidelines move |
| Tab / Shift+Tab | cycle selection in stacking order, press after press: with nothing of the window focused, Tab is the drawing's and never moves the focus into the bars (#1667, `pick_drag_tests::tab_goes_from_object_to_object_press_after_press`) |
| + (also on the numeric keypad) | duplicate the selection in place (no offset), the copies selected, one undo step |
| Double-click | Shape tool on a curve; on text, the Text tool editing that text with the caret at the end |
| Esc | deselect |
| Double-click the tool button | select all objects |

The toolbox button of each flyout shows the tool last used from it; the
choice is kept between sessions (`toolbox` in the settings file).

Hidden and locked objects, and the objects on hidden or locked layers,
are never hit (a right click on a locked object that is topmost under
the pointer opens a menu of its own, Unlock Object, which unlocks and
selects it, and Unlock All Objects; the object is not selected while
locked). A locked object, or one inside a locked group, is never
selected in any way: its row in the Objects docker does not select it,
and objects imported from a file come in selected except those locked
in it (#2347,
`app::tests::locked_objects_imported_or_picked_in_the_list_are_never_selected`).
Of the others the topmost under the pointer is hit (the last
in the stacking order, which `.cdr` files keep: a text over a
rectangle is hit before it). Treat all objects as filled (Options >
Tools > Pick, and the property bar) is on by default, so an empty
rectangle over a text takes a click inside it; Alt+click reaches the
text beneath, or the option switched off leaves an empty object to
clicks on its outline.
Checks: `pick_drag_tests::text_over_a_rectangle_read_from_cdr_is_hit_first`,
`pick_drag_tests::an_unfilled_rectangle_over_text_takes_the_click_and_alt_reaches_the_text`,
`pick_drag_tests::ctrl_click_on_a_clip_frame_edits_its_contents`,
`pick_drag_tests::clipped_away_contents_are_not_hit`.

The status bar names an object chosen inside a group "Rectangle in a
group on Layer 1". Arrange commands (order, group, combine) act on the
objects on layers and leave an object chosen inside a group where it is.

The drag keeps going while the left button is held, also after a right
click (the windowing layer ends a drag when any button goes up); the
modifier keys are read at every step, so pressing or letting go of Shift,
Ctrl or Alt during the drag changes the result at once.

## The handles

The selection box has no frame of its own, so the objects' outlines
stay in view. Eight solid black square handles with a one-pixel white
edge (so they show on a dark drawing, #670), 10 interface points
across by default (Options > Nodes and Handles > Handle size: Small 6,
Medium 8, Large 10 points, #1285), stand just outside it: their centres
half a handle plus 1.5 points out from the box (a gap of 1.5 points), and
a press within half a handle plus 1 point of a centre (never less than
4 points) takes it
(`pick_drag_tests::the_handle_size_option_sets_the_handles_size_and_reach`).
The text flow tabs hang 3.5 points past the handles, whatever their
size. A corner or side handle scales from where it was
grabbed, so the box follows the pointer without a jump. Objects chosen
inside a group show round handles. The centre mark is a small cross.
The first node of each subpath of a selected object shows too, a
small hollow square (5 points, white with a dark edge): both rings of a
letter O, an ellipse's start, a rectangle's first corner; groups, texts
and pictures show none, nor does a selection of more than 1000
subpaths. So a selected object is told from the others at a glance and
where its outline starts shows, as in the reference program (#1145,
`canvas::tests::the_pick_tool_marks_where_the_selected_outlines_start`).

In rotate mode the handles are black arrows 2 points thick with solid
heads, their centres 9 points out from the box: a curved double arrow
around each corner, bulging outwards, and a straight double arrow along
each side. The centre of rotation is a ring 8 points in radius with a
dot in the middle.

## Large selections

With many objects selected (Select All on a drawing of thousands of
objects) the window keeps its frame rate: the selected objects are
gathered in one pass over the layers and kept, shared, while the
drawing, the page and the selection stay as they are
(`App::selected_rc`); the bars, the status bar and the canvas read that
one list, and the checks that ran once per selected object over the
whole drawing (its layer, whether it lies in a group, linked text
frames) take one pass too. With 3000 rectangles selected a frame went
from about 200 ms to about 7 ms (#367; check
`app::tests::the_selected_objects_are_gathered_once_and_kept_while_nothing_changes`).

## Selection groups

Ctrl and a number from 0 to 9 store the selection under that number;
the number alone selects those objects again (the ones the current page
shows), and pressed again while they are selected zooms to them. Alt and
the number add the selection to the group. Groups are saved with the
drawing; objects deleted since drop out. With text selected, Ctrl+8 and
Ctrl+2 keep changing the text size, and Ctrl+Shift+> and Ctrl+Shift+<
move its letters further apart or closer, 5 % of a space's width a
press (the Text docker's Character spacing), for a text fitted to a
path too and while a text is edited
(`text_nodes::tests::the_spacing_keys_step_the_character_spacing_of_selected_texts`,
#193).

## Invert Selection and Select Similar

Edit > Invert Selection (#2002) selects the objects Select All takes
(the top-level objects of the current page's unlocked layers; the
contents only while a clip frame's contents are edited) that are not
selected, and deselects the others; nothing selected selects all.

Edit > Select Similar takes the same objects, and the objects inside
their groups (at any depth, each on its own), when they are like one of
the selected objects (those inside a group, chosen with Ctrl+click,
count as references too). For Fill, Outline and Text Formatting a group
is not compared itself: a selected group stands for the objects in it,
and groups are never taken. For Object Type a group is a group: a
selected group finds the other groups (#2203).

| Item | Like when |
|---|---|
| Fill | the fill is equal (colour, gradient, pattern, texture, or both none) |
| Outline | the outline is equal (width, colour, dashes, caps, arrows...), or both have none |
| Text Formatting (Font and Size) | both are text and share a font family (any case) and size (to 0.01 pt) in some of their characters |
| Object Type | the same shape kind (rectangle, ellipse, polygon, curve, text, group, bitmap, clip frame, table, symbol), artistic and paragraph text apart |

Nothing selected, or no object alike: the selection stays. The status
bar tells how many objects are selected.

Each command is a selection step on the Undo list, ahead of the
drawing's steps: Edit > Undo shows its name ("Undo Invert Selection",
"Undo Select Similar (Fill)") and puts the selection back as it was;
Redo makes it again. A step only counts while the drawing (its revision),
the page and the selection are as the step left them: a change to the
drawing, a selection made by hand or another window forgets the steps,
and Undo goes on to the drawing's own steps. No drawing step is added,
so the drawing is not marked changed.

Checks: `select_similar::tests::invert_selection_takes_the_others_and_undo_puts_it_back`,
`select_similar::tests::select_similar_fill_outline_text_and_kind`,
`select_similar::tests::selection_steps_undo_in_order_and_end_with_a_drawing_change`,
`select_similar::tests::the_menu_items_run_by_key`.

## Node tracking

Options > Display > Enable node tracking (off by default): with the Pick
tool, the pointer over a node of a selected curve takes the Shape tool's
arrow with the node mark, and a drag there moves that node (snapping
like the Shape tool's node drags; Ctrl keeps it horizontal or vertical;
one undo step). The object can then no longer be moved by dragging it at
one of its nodes. Not in rotate mode.

## Dragging the selection

While the selection is moved, scaled, turned or slanted, the objects stay
where they are and their outline shows where they would go: a blue line
on a white halo, which reads as two light lines on a dark fill. The
outline is the one the release makes: a rectangle's corners that keep
their size stay round while it is stretched
(`Shape::page_path_after`). The handles return on release.

A move grabbed on a mark (see below) carries that point: a node,
midpoint, centre or the point of an edge lands on other objects' points
and edges and slides along an edge while the pointer stays within the
snapping radius. The edge snapped to lights up in blue while the
drag lasts. A scale or skew handle snaps the point of the box it
stands for, and turning snaps the pointer, to other objects' points and
edges, guidelines, the grid and the page (Ctrl turns in steps instead).

## Duplicate

Edit > Duplicate (Ctrl+D) copies the selection at the duplicate offset
(Options > Document > General; none at first, so the copy lies on the
original ready to be moved, scaled or turned; preferences that kept
the old default of 6.35 mm get none once, #1494) and selects the
copies. While the copies stay the selection, the next
Duplicate repeats the step from each original to its copy: the offset
if they were left alone, or the move, scale, turn or skew done to them
since (a smart duplicate): duplicate a box,
move the copy 20 mm, and each next Duplicate is 20 mm further; scale the
copy by 10 % and each next one is 10 % larger; turn it 9 degrees and
scale it and each next one turns and grows as much again. Deselecting
the copies (a click elsewhere, Esc, another object picked) ends it: the
next Duplicate is the offset again, also when the copies are picked
again. Guidelines duplicate the same way: a copy of selected guidelines
moved or turned (about the point where it crosses its original) is
moved or turned as much again by each next Duplicate (#559, #1494,
`app::tests::duplicate_repeats_what_was_done_to_the_copy`,
`app::tests::smart_duplicate_repeats_a_turn_and_scale_until_deselected`,
`guides::tests::duplicating_guidelines_repeats_their_move_and_turn`).
A deselect by accident is taken back by Undo: right after the copies
are deselected, with no change of the drawing and no other selection
made since, Edit > Undo selects the copies again and the next Duplicate
repeats the step again; no step of the drawing is undone. Otherwise Undo
undoes the last change as usual (#1494,
`app::tests::undo_after_a_deselect_brings_the_duplicate_repeat_back`).

## Snap marks over the selection

With snapping to objects on, the pointer resting over the selection (not
over a handle or the centre of rotation) shows the selection's point or
edge within the snapping radius, the one a drag would carry: its mark
and, with screen tips on, its name in lower case (node, midpoint,
center, edge and the other snapping modes in use). Points win over edges. Handles take a
press within 6 points of their centre, short of the selection box.

## The pointer

The pointer says what a press would do (code: `ui/cursors.rs`):

| Under the pointer | Pointer |
|---|---|
| A corner handle | diagonal resize arrows (top left and bottom right, or top right and bottom left) |
| A side handle | vertical (top, bottom) or horizontal (left, right) resize arrows |
| A selected object (or the selected part of a group under the pointer), the centre mark or the centre of rotation | move arrows |
| An object that is not selected | the arrow (the cross hair with Options > Tools > Pick > Cross hair): a press picks it first, and a drag then moves it |
| A horizontal or vertical guideline (where no object is) | up and down or sideways arrows across it; the move arrows on a slanted one; the arrow on a locked one |
| A rotation handle of a guideline in rotate mode | a turning arrow |
| A corner handle in rotate mode | a turning arrow |
| A side handle in rotate mode | two half arrows along the slant (sideways for top and bottom, up and down for the sides) |
| Anything else | the arrow (the cross hair with Options > Tools > Pick > Cross hair) |

While dragging, the pointer keeps the shape of what is being done (a
move: the white arrow with a dotted box under its tail, and a plus in a
square beside the box once a right click made the move leave the
original; it stays drawn after the right click until the left button
goes up; the move arrows for the centre of rotation; the handle's resize arrows, the turning arrow, the slant
arrows, the arrows across a guideline being moved or dragged out of a
ruler, the cross hair while the ruler origin is dragged). Resize and move arrows are the system's own; the moving
arrow, the turning and slant arrows are drawn by TraceDraw, black on a
white halo.

## Dragging with the right mouse button

Pressing the right button on an object (it is selected first if it was
not) and dragging shows the selection's outline in blue where it would
go, while the objects stay put; Ctrl keeps the drag horizontal or
vertical, Esc cancels it. On release a menu opens there:

| Item | Effect |
|---|---|
| Move Here | moves the selection by the drag |
| Copy Here | copies of the selection, moved by the drag, become the selection; always by the drag, also when the dragged objects are the copies of a Duplicate still selected (#2640, `drop_menu::tests::copy_here_from_copies_still_selected_lands_where_the_drag_ended`) |
| Copy Fill Here | the object under the pointer takes the fill of the first selected object |
| Copy Outline Here | ... its outline |
| Copy All Properties | ... its fill, outline and transparency |
| Place Inside | the object under the pointer becomes the clip frame of the selection |
| Cancel | nothing |

The four items that act on the object under the pointer are dimmed when
there is none. A right click without a drag opens the context menu. A
right click or a right-button drag on a selected object keeps the
selection, also where another object covers it (an object picked with
Alt+click or a marquee behind a larger one): the menu and the move are
for what is selected, not for the object in front; with no selected
object under the pointer, the topmost object there is selected first
(#1684, `pick_drag_tests::a_right_click_on_a_covered_selected_object_keeps_the_selection`).

Keys held as the right button is let go over another object skip the
menu, as in CorelDRAW: Shift gives that object the outline of the first
selected object (Copy Outline Here), Alt its fill (Copy Fill Here) and
Ctrl (Cmd on macOS) its fill, outline and transparency (Copy All
Properties). While the keys are held the object that would take them is
outlined in the selection colour and the pointer's note names the copy;
the selection stays where it was. Not over an object, the menu opens as
without keys, and Ctrl keeps the drag horizontal or vertical (discussion
#2066, `drop_menu::tests::right_drop_with_shift_alt_or_ctrl_copies_without_the_menu`).

The same copies without the menu, with the left button: start moving
the selection without a key (Alt as the drag starts makes a selection
box, #1341), then hold Alt and let go over another object, and that object
takes the fill of the first selected object (Copy Fill Here); hold
Alt+Shift, and it takes its outline (Copy Outline Here). The object that
would take it is outlined in the selection colour and the pointer's note
names what the drop does; the selection stays where it was, and each
copy is one undo step. Over a clip frame, Alt alone keeps its meaning of
putting the selection into the frame (#1337); Alt+Shift copies the
outline onto the frame (discussion #2066,
`pick_drag_tests::a_move_let_go_with_alt_copies_the_fill_or_the_outline`).

## Property bar

With nothing selected the bar shows the page fields (size, orientation,
units, nudge distance). With a selection it shows X, Y (lower left corner),
W, H, scale, angle, mirror buttons, the outline width, To Front, To Back
and Convert to Curves. Text selected on its own adds the text fields of the
Text tool after them. Bitmaps selected on their own replace Convert to
Curves with the bitmap commands: Edit Bitmap, Crop Bitmap (switches to the
Crop tool), Trace Bitmap (Automatic trace and the centreline and outline
presets), Straighten Image and Resample.

With two or more objects selected, a group, or a curve of several parts,
the bar ends with the arranging buttons: Group (Ctrl+G), Ungroup
(Ctrl+U) and Ungroup All (with a group in the selection), Combine
(Ctrl+L) and Break Apart (Ctrl+K, with a curve of several parts), the
shaping operations Weld, Trim, Intersect, Simplify, Front Minus Back,
Back Minus Front and Boundary (they act as in Object > Combine Shapes, see
`shaping.md`; Boundary works on one object too), and a button that opens the Align and
Distribute panel. Their pictures show a square behind a
circle: the result filled, an object that stays outlined, one that goes
away dashed.

## Dialogs

Enter presses OK in every modal dialog (Esc cancels). In the two
multi-line fields (QR code text and document notes) Enter inserts a line
break instead.

## Checks

- `pick_drag_tests::overlays_are_drawn_on_a_white_line`
- `envelope_edit::tests::enveloped_objects_are_hit_and_boxed_as_drawn`

- `ui::propbar::tests::shaping_and_grouping_show_for_several_objects_groups_and_parts`
- `pick_drag_tests::ctrl_keeps_a_move_straight`
- `pick_drag_tests::a_right_click_while_dragging_leaves_the_original`
- `pick_drag_tests::esc_cancels_a_drag`
- `pick_drag_tests::space_while_moving_drops_a_copy`
- `pick_drag_tests::space_alone_in_a_frame_drops_a_copy_while_scaling`
- `pick_drag_tests::shift_scales_from_the_centre_and_ctrl_mirrors`
- `pick_drag_tests::rotate_mode_skews_with_the_sides_and_turns_about_the_moved_centre`
- `pick_drag_tests::handle_modifiers_shape_the_scale`
- `pick_drag_tests::side_handles_skew`
- `pick_drag_tests::the_pointer_shows_what_a_press_would_do`
- `pick_drag_tests::the_status_bar_tells_how_far_how_much_and_what_angle`
- `pick_drag_tests::objects_dropped_on_the_document_palette_give_their_colours`
- `pick_drag_tests::ctrl_click_chooses_an_object_inside_a_group`
- `pick_drag_tests::alt_click_digs_to_the_object_beneath`
- `pick_drag_tests::alt_marquees_take_what_they_touch`
- `pick_drag_tests::the_touching_option_swaps_what_alt_does`
- `pick_drag_tests::clicking_the_selection_or_its_centre_mark_turns_the_handles`
- `pick_drag_tests::the_pointer_shows_moves_for_objects_and_guidelines`
- `pick_drag_tests::scale_handles_snap_to_other_objects_not_to_the_selection`
- `pick_drag_tests::plus_duplicates_in_place_and_edits_nodes_in_the_shape_tool`
- `pick_drag_tests::a_moved_centre_of_rotation_is_kept_with_the_object`
- `select_region::tests` (wholly inside, touching, groups, lassos)
- `command::tests::objects_inside_groups_can_be_changed_and_deleted` (core)
- `tests::the_drag_preview_reaches_objects_inside_groups` (render)
- `drop_menu::tests::move_here_and_copy_here`
- `drop_menu::tests::fill_outline_and_properties_go_onto_the_target`
- `drop_menu::tests::place_inside_makes_the_target_a_clip_frame`
- `drop_menu::tests::a_ctrl_drag_keeps_to_one_direction`

# Multipage View

View > Multipage View lays every page of the drawing out on the desktop
at once, so pages can be compared and edited side by side. The page
bar's Multipage View button, after the page controls, turns it on and
off too and is lit while it is on (#1019).

## Layout

- Pages sit in a grid of four columns, whatever the page count (fewer
  pages fill one row), left to right, then down, in page order. A page
  added goes at the next place and the pages before it stay where they
  were (#1018, `multipage::tests::adding_pages_never_moves_the_pages_before`).
- Each column is as wide as its widest page and each row as tall as its
  tallest; a page sits at the top left of its cell. Cells are the
  drawing's page spacing apart: 30 mm unless the drawing sets another
  (`multipage::tests::the_cells_are_as_far_apart_as_the_spacing`).
- Every page shows its number and name below it; the current page's in
  the normal text colour, the others dimmed.
- Objects outside a page's edges show for every page, where they lie
  beside it on the desktop: each other page's picture covers its paper
  and what its visible objects cover, measured once a change of the
  drawing (#2055, `multipage::tests::objects_off_another_page_are_drawn`).
  They showed only while their page was current, so they seemed to
  vanish when another page became current.
- Every page shows its own guidelines: the current page's across the
  whole window, as always, and each other page's within that page,
  where it sits in the grid (#1448,
  `multipage::tests::the_other_pages_guidelines_show_where_their_pages_sit`).
  Guidelines belong to their page; a click on another page makes it
  current, and its guidelines are then the ones picked, moved and
  snapped to.

- With facing pages the pages sit in spreads, two spreads a row (see
  `facing-pages.md`).

## Page spacing

- The space between pages is a document property
  (`Metadata::page_spacing`, mm), saved in `.tdraw` files only once set;
  a drawing without it (any file from before) uses 30 mm
  (`tracedraw_core` `the_page_spacing_defaults_to_30_mm_and_round_trips`).
  Values are kept between 0 and 1000 mm.
- Set from Page > Page Spacing (presets 0, 5, 10, 20, 30, 50 mm, or
  Custom... in the document's units) and Document Setup > Layout > Page
  spacing. Each change is one undo step (Document Properties); Save as
  Default keeps it for new drawings.
- The current page stays where it is on screen when the spacing changes;
  the others move. The grid, hit testing of the other pages, alignment
  to their objects, their guidelines, page labels and Zoom to Fit all
  follow the spacing
  (`multipage::tests::the_drawing_s_page_spacing_moves_the_pages_apart`).
- Printing and export work on pages, not on this view, so the spacing
  does not change them.

## Choosing the pages

- View > Pages in Multipage View and the page bar's button right after
  the Multipage View button (two pages with a tick, lit while a range is
  chosen) open a dialog: All pages, or Pages: with a range of page
  numbers such as `1-3, 7` (single pages and ranges, either way round,
  commas or semicolons between; pages past the end are left out). It
  tells how many pages the range shows; a range naming no page of the
  drawing keeps the dialog open. OK keeps the range and turns the view
  on (#1440).
- The range is a document property (`Metadata::multipage_pages`, the
  text as typed, empty for every page), saved in `.tdraw` files only
  once set, one undo step, like the page spacing. A range that names no
  page left (pages deleted since) shows every page
  (`tracedraw_core` `the_multipage_pages_are_kept_and_default_to_all`,
  `page_ranges_read_numbers_and_ranges_in_page_order`).
- The pages shown are packed into the grid in page order, as if the
  others were not there; with facing pages each page keeps its side of
  the spine and the spreads shown follow each other
  (`multipage::tests::the_multipage_view_shows_the_chosen_pages_only`,
  `multipage::tests::a_range_of_facing_pages_keeps_their_sides_and_packs_the_spreads`).
- The current page always shows. Choosing a range that leaves it out
  makes the first chosen page current; going to a page outside the range
  (page tabs, navigator, Page > Go To Page) shows it in its place in page
  order while it is current, the page that was current staying where it
  is on screen.
- The normal view and a spread of facing pages outside this view are not
  changed by the range.
- Checks: `ui::dialogs::tests::the_multipage_pages_dialog_keeps_the_range_and_shows_the_view`,
  `pick_drag_tests::the_page_bar_chooses_the_multipage_view_s_pages`.

## Working

- The current page is edited as in the normal view: rulers, guides,
  grids and the status bar refer to it.
- A press picks the object that shows there, whatever page holds it
  (#2055): an object of the current page (drawn over the other pages'
  objects) keeps the page, even over another page's paper or on the
  desktop; else an object of another page under the pointer (the object
  itself, not its box, as for the current page), on its page or off it,
  makes that page current before the tool acts, so the same click
  selects it; else a press on another page's paper makes it current, so
  a click selects there and a drag draws there. Nothing moves on screen
  (`multipage::tests::a_press_picks_the_object_shown_whatever_page_holds_it`).
- A move of the Pick tool let go with the middle of the moved objects'
  box over another page's paper gives them to that page: they stay where
  they show, now in that page's own place, on its topmost visible,
  unlocked layer, and that page becomes current with them selected, as
  one undo step with the move (Move to Page). Let go on the desktop or
  on the current page, they stay the current page's. Not with facing
  pages, nor for an object chosen inside a group or one on a master
  layer (#2055,
  `pick_drag_tests::an_object_dropped_on_another_page_of_the_multipage_view_becomes_that_page_s`).
- The page tabs and the navigator change the current page the same way.
- Zoom to Fit (F4) and turning the view on fit every page in the window.
- Page Overview and Multipage View exclude each other.

## Drawing

- Each other page keeps its own picture. While the view pans or zooms,
  the last picture is scaled and moved with its paper, and the pages are
  rendered anew once the view has rested for 0.12 s.
- A page that covers no more of the screen than the canvas is rendered
  whole; a larger one only where it shows.
- One frame spends at most 10 ms rendering the current page and the
  others; pages left over show their last picture and render in the next
  frames. One render runs every frame, so every page gets its turn.
- Renders on a thread run one at a time in the whole program, the page
  being edited first: when it had to wait, the turn stays kept for it
  for 1 s after the running render ends. A page refused its turn for
  that reason schedules a repaint for the moment the turn is free again,
  so it is rendered sharp at the current zoom without any pointer move or
  key press (#1384,
  `raster::tests::a_page_refused_a_kept_turn_asks_for_a_repaint`).

## Formula

With `cols = min(n, 4)`, page `i` is in row `i / cols`, column
`i % cols`; its left edge is the sum of the earlier column widths plus
`column * gap`, its top edge the sum of the earlier row heights plus
`row * gap` below the first row's top, `gap` being the page spacing
(30 mm by default). The grid is then moved so the
current page's bottom-left corner is the origin.

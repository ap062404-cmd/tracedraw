# Spot colours

A spot colour is a named ink printed on a plate of its own (a metallic,
a brand colour). The drawing keeps a list of its inks (`Document::inks`:
a number, a name and the ink's process stand-in, C, M, Y, K at full
strength), saved in `.tdraw`; a colour on an ink is `Color::Spot` (the
ink's number, a tint 0 to 100 % and the stand-in, so it shows and
converts without the list).

## Colour panel

Under Fill, Outline and the "..." menu, Spot inks lists the drawing's
inks: a swatch at the tint, the name, Fill and Outline (apply the ink at
the tint to the selection) and a cross that takes the ink off the list
(objects keep its stand-in). Tint (0 to 100 %, 100 by default) sets the
strength applied; while the palette view shows, the control sits under
its swatches instead (one Tint control in the panel). A name typed in Ink name and New spot ink add an ink
standing for the new colour (in process inks); names are unique. The
"..." menu's Add to Custom Spot Colors does the same with the name typed,
or with the colour's `#rrggbb` code when none is (see `color-docker.md`).

An ink's name in the list is a field: typing over it and pressing Enter,
or leaving the field, renames the ink in one undo step; an empty name or
one another ink has changes nothing. Objects and swatches refer to the
ink by its number, so they all take the new name, as do its separation
in PDF, EPS and PostScript and the plate it prints on (#2187,
`ui::color_docker::tests::a_spot_ink_is_renamed`). A spot colour's
swatch, in the document palette and elsewhere, is named by its ink, and
the status bar names a spot fill or outline by its ink and tint (#2186).

## Swatch marks (#2321, #2455)

A swatch tells a spot colour from a process colour by a small dot in its
lower right corner, and an overprinting one by a small ring in its lower
left corner; the dot and the ring are black on a light colour and white
on a dark one, and a swatch under 6 px has none.

- The dot (`ui::palette::spot_mark`) is on the palette bars, in the
  Palette Editor, on the fill and outline swatches of the status bar and
  on the fill-over-outline swatch of the Colour panel.
- The ring (`ui::palette::overprint_mark`) is on the fill and outline
  swatches of the status bar and of the Colour panel while every painted
  object of the selection overprints that fill or outline (Overprint fill
  and Overprint outline, `App::selection_overprint`). Palette swatches
  have none: overprint belongs to an object, not to a colour.

## Colour books

A palette file can be a colour book of spot inks: in an `.ase` palette,
an entry whose colour type is 1 (spot) becomes a spot colour named as
the entry, standing for its colour in process inks; global (0) and
normal (2) entries stay process colours. Its ink number comes from the
name (FNV-1a with the top bit set), so the same name is the same ink in
every drawing. Applying one (palette click, drag, the Colour panel) adds
the ink to the drawing's list, under the palette's name, as part of the
same undo step, unless the drawing has it already. Licensed colour
books are not bundled; a book the user owns opens like any palette
(Window > Color Palettes > Open Palette, `.tdpal`, `.gpl`, `.ase`,
`.aco`).

In the palette editor the Spot color box under the chosen colour makes
it a spot colour of the entry's name, the colour as its stand-in, or a
process colour again (its stand-in at its tint). Renaming a spot entry
renames its ink, the stand-in and tint kept; an entry only looked at
keeps its ink number. A `.tdpal` file keeps spot colours, and on loading
each one's ink number comes from its name, so a name edited in the file
is that name's ink.

## Output

- On screen and in bitmap, SVG and `.cdr` output a spot colour is its
  stand-in times the tint (each process ink scaled).
- PDF: each ink becomes one Separation colour space (its name, the
  DeviceCMYK alternate through a type 2 function from no ink to the
  stand-in), referred to from the page resources as `/CS<ink>`; the tint
  is the colour value. A spot colour whose ink is no longer on the list
  prints as its stand-in. PDF/X keeps the Separation spaces. Opened or
  imported again, each Separation space is the ink of its name again and
  each colour in it a spot colour at its tint (see `pdf-import.md`).
- PDF overprint: an object whose fill or outline overprints is drawn
  under a graphics state with `/op` (fill) and `/OP` (outline) and
  overprint mode `/OPM 1`, so the inks under it stay (ISO 32000-1,
  8.6.7).
- Separations (Print, or Export with separations): after the C, M, Y and
  K plates, a plate per ink of the list; a spot colour prints on its own
  plate only, at its tint, and on none of the process plates.

## Checks

- `color::tests::spot_colors_show_and_convert_through_their_stand_in` (core)
- `pdf::tests::spot_colors_print_on_separation_color_spaces` (io)
- `pdf::pdfx_tests::crop_marks_and_spot_colors_come_back_from_the_pdf` (io)
- `pdf_import::tests::separation_devicen_and_indexed_colors` (io)
- `app::tests::pdf_spot_inks_come_into_the_drawing`
- `export::tests::spot_inks_get_plates_of_their_own`
- `ui::color_docker::tests::spot_inks_are_added_applied_and_removed`
- `palette::tests::colour_book_spot_entries_are_spot_colours`
- `palette::tests::a_colour_book_spot_colour_brings_its_ink_along`
- `palette::tests::tdpal_spot_colours_keep_their_ink_by_name`
- `ui::dialogs::tests::palette_entries_turn_into_spot_colours_and_back`
- `pdf::tests::overprinting_objects_get_overprint_graphics_states` (io)
- `export::tests::a_selection_exported_alone_keeps_its_spot_inks`
- `ui::palette::tests::swatches_carry_the_spot_and_overprint_marks`
- `ui::palette::tests::the_overprint_ring_and_the_spot_dot_keep_apart`
- `ui::status::tests::the_status_swatches_carry_the_spot_and_overprint_marks`

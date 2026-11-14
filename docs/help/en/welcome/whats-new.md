# What's new

Each version of TraceDraw adds features and fixes. This page explains how to
find out what changed and lists the highlights of the latest versions.

## Seeing what is new in the program {#in-the-program}

- Choose [[menu.help > menu.help.whats_new]] to open the What's New page of
  the Welcome Screen. It lists the last ten versions, newest first, each
  with its date and a short title. Click a version to read a short summary
  of what changed; the newest one is open already. **Want to know more?**
  under a summary opens that version's page on GitHub with the full list of
  changes.
- Turn on [[menu.help > menu.help.highlight_whats_new]] to mark what is new
  in this version in the menus: a small dot appears at the end of each new
  command, beside the arrow of a submenu that leads to one, and over the
  title of a menu that holds one. The marks go away when you turn the
  command off, and the command is off again the next time TraceDraw starts.
- The complete list of changes of every version is in the project's
  [change log](https://github.com/tracedraw/tracedraw/blob/main/CHANGELOG.md).

## Highlights of recent versions {#highlights}

- **Extrusion, shadows and a round of fixes.** [[menu.effects > menu.effects.extrude]] keeps a body turned in space in place, and bevels on text no longer fold over; drop shadows keep the holes of rings and fall from an extruded object together with its body; [[menu.edit > menu.edit.select_similar]] looks inside groups; [[menu.bitmaps > menu.bitmaps.convert_to_bitmap]] keeps the picture's place in the stack; [[menu.view > menu.view.navigator]] is a menu command with the key N; .cdr files open with their tables, and Thai file names show their letters.
- **CNC toolpaths, Export and selection.** [[menu.tools > menu.tools.tracecut]] makes G-code for a CNC router and draws eyelets, finger-jointed boxes and folded trays; [[menu.file > menu.file.export]] asks for the file first, then shows the format's options, and [[menu.file > menu.file.export_with_options]] always shows them; [[menu.edit > menu.edit.select_similar]] and [[menu.edit > menu.edit.invert_selection]] change the selection; [[menu.object > menu.object.insert > menu.object.add_dimensions]] measures the selection; procedural fills, OpenType buttons and a Lao interface are new.
- **Facing pages and linked text.** [[menu.layout > menu.layout.facing_pages]] shows the pages side by side in spreads; [[menu.text > menu.text.link_content]] links texts so that typing in one changes the others; a PDF of several pages asks which pages to import; [[menu.edit > menu.edit.step_and_repeat]] can fill a page with copies.
- **Touch and phones.** Shift, Ctrl and Alt buttons on the Standard toolbar
  stay held for a finger; on Android the interface keeps clear of the camera
  cutout and the rounded corners; the Apple Pencil works in Safari on iPad.
- **Files.** .cdr files keep the fill rule of overlapping curves and open with
  their tab stops, multi-column text, guidelines shared by every page and
  master page objects.
- **Pictures, printing and the workspace.** The Crop tool's Crop picture
  pixels and [[menu.bitmaps > menu.bitmaps.crop_to_frame]] cut a picture's
  pixels; printing in grayscale and in reverse order;
  [[menu.view > menu.view.scrollbars]] hides the scrollbars.
- **TraceCut.** [[menu.tools > menu.tools.tracecut]] opens the tools for
  cutting plotters: plotter output, cut lines, nesting and measuring. See
  [TraceCut](../objects/tracecut.md).
- **Keyboard, phones and pens.** Menus and lists work from the keyboard. On
  Android phones the interface fits the screen and a hand resting on it no
  longer draws while a pen is used; a tablet pen works on Linux with Wayland.
- **Files.** .cdr files open with their tab stops, master pages, gradients and
  distortions; PDF pictures come in at full resolution; JPEG and TIFF export
  in CMYK.
- **Object thumbnails.** The [[docker.objects]] panel shows each object
  drawn small beside its name; Ctrl+wheel over the list or the slider at its
  bottom sets the size. See [Organizing objects and layers](../objects/objects-and-layers.md#objects-docker).
- **Tablets and Android.** The browser version installs as an app on a
  tablet or a desktop, and an Android app runs on tablets, phones and
  Chromebooks. See [Tablets and the Android app](../get-started/tablets-and-android.md).
- **Pen pressure.** A graphics tablet's pressure sets the width of Freehand
  lines and brush strokes. See
  [Drawing with a graphics tablet](../lines-shapes/pen-pressure.md).
- **Packages and missing fonts.** A package puts a drawing, its pictures and
  its fonts into one folder, and a dialog box chooses the fonts that stand in
  for missing ones. See [Packaging a drawing](../file-formats/package.md) and
  [Missing fonts](../text/missing-fonts.md).
- **Converting colors.** Every color of a selection or a drawing, bitmaps
  included, can be turned into CMYK or RGB in one step. See
  [Converting colors](../color/convert-colors.md).
- **Print work.** Booklets can be saddle-stitched or perfect bound, and PDF
  files can carry printers' marks. See [Printing booklets](../printing/booklet.md)
  and [Printers' marks in PDF files](../file-formats/pdf-prepress-marks.md).
- **Find by properties and fit objects to a path.** Find objects by fill,
  outline, width, font or type and replace them; place objects and copies
  along a path. See [Finding by properties](../objects/find-by-properties.md)
  and [Fitting objects to a path](../objects/fit-objects-to-path.md).
- **Before and After preview.** Every dialog box that changes a picture is a
  window you size, with a shared preview you zoom and pan. See
  [The Before and After preview](../bitmaps/before-after-preview.md).
- **Help in your language.** Every **?** button and <kbd>F1</kbd> open the
  page of this Help about the dialog box, panel or tool in front of you, in
  the language TraceDraw is set to.
- **Document info.** [[menu.file > menu.file.document_properties]]
  keeps the language, title, subject, author, copyright, tags, notes and
  rating of a drawing, over a full report of what the drawing holds. See
  [Document info](../pages-layout/document-properties.md).
- **Color management.** [[menu.tools > menu.tools.color_management]] sets
  the profiles of the drawing and the defaults for new ones, with policies
  for files that open or are imported. See
  [Color management](../color/color-management.md).
- **Barcodes and QR codes.** A three-step wizard places barcodes, and QR
  codes are live objects you edit in the [[docker.properties]] panel. See
  [Barcodes and QR codes](../objects/barcodes.md).
- **Effects.** The [[menu.effects]] menu groups the effect panels; blends
  can follow a path with fixed spacing and turn their colors clockwise or
  counterclockwise. See [Blends](../effects/blend.md).
- **Text on a path** has its own settings for orientation, placement,
  distance and offset. See [Adding and editing text](../text/adding-text.md).
- **Font Manager** organizes installed fonts, folders, favorites and
  collections. See [Fonts and glyphs](../text/fonts.md).

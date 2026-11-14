# Bitmap import

File > Import (Ctrl+I) of a PNG, JPEG, BMP, GIF, TIFF or WebP file
embeds the image as a bitmap object centred on the page and remembers
the file (Image > Update from Link reads it again). PNG files of 8 or
16 bits, grey or colour, with or without alpha or with a palette, and
baseline or progressive JPEG files all come in
(`bitmap_file::tests::every_common_png_and_jpeg_imports`, #2167).

A picture whose Exif orientation says it was taken with the camera
turned comes in upright, its width and height (and its resolution
across and down) swapped for a quarter turn
(`bitmap_file::tests::an_exif_orientation_stands_the_picture_upright`,
#2167).

A CMYK JPEG (four components) comes in as a CMYK picture with its inks,
as a CMYK TIFF does, its pixels showing the inks through the colour
management as the drawing's CMYK colours show, so exporting an object
as a CMYK JPEG and importing the file gives the colour the object had.
The channels are read as stored: inverted (255 is no ink) under an
Adobe `APP14` marker with transform 0, as the CMYK export and most
programs write them; Y, Cb, Cr of the inverted C, M, Y and an inverted
K under transform 2 (YCCK); as they are without the marker. Before,
the image library turned the file into RGB by itself, taking every
file as inverted and leaving the colour management out, so the picture
came in brighter than the same inks in the drawing (#2163,
`bitmap_file::tests::a_cmyk_jpeg_imports_with_its_inks_as_the_drawing_shows_them`).

## The bitmap editor

Edit Bitmap writes the picture to a file in the temporary folder, links
the object to it and opens it in the bitmap editor set in Tools >
Options > Global > File Locations (a program file: `gimp`, its `.exe` on
Windows, or an application bundle on macOS, opened with `open -a`;
Browse picks it, Clear empties it). Empty, the default, opens it with the
system's program for PNG files (#593, `export::tests::the_bitmap_editor_gets_the_file`).
The browser build has no outside editor.

The file is a PNG, or a CMYK TIFF for a CMYK picture (a `.cdr` CMYK
image, or one in the CMYK colour mode), holding the inks it prints from
as Save Bitmap As writes them, so the editor works on the inks and not
on RGB pixels (#1834); a picture with see-through pixels goes with its
alpha channel beside the inks, so the transparency survives the round
trip (#1925). A CMYK TIFF read back (8 bits per ink, with or
without alpha, any compression) gives the picture those inks and pixels
that show them through the colour management, and the CMYK mode; any
other file gives pixels only, and inks the picture kept are dropped,
since they no longer fit (`bitmap_file::tests::edit_bitmap_hands_over_and_reads_back_the_inks`).

## Linked files follow their changes

A linked bitmap (an imported file, or one sent to the system image
editor by Edit Bitmap) follows its file on the desktop: once a second
the files' modification times are read, and a bitmap whose file changed
since it was last seen is read again on the spot, as Update from Link
does (`App::watch_linked_bitmaps`; one undo step per change, the status
bar says "Bitmap updated from its file"). So a save in the outside
editor shows in the drawing at once. A file seen for the first time is
only remembered; a file that cannot be read is left alone. The browser
has no files to watch
(`clipboard::linked_watch_tests::a_linked_bitmap_is_read_again_when_its_file_changes`).

## Size

The bitmap comes in at the resolution its file records, so a 300 dpi
scan 1200 pixels wide is 101.6 mm wide:

| Format | Where the resolution comes from |
|---|---|
| PNG | the `pHYs` chunk, when its unit is the metre (pixels per metre x 0.0254) |
| JPEG | the JFIF header's density in dots per inch or per centimetre; else the EXIF `XResolution`/`YResolution` with their `ResolutionUnit` |
| TIFF | `XResolution`/`YResolution` with `ResolutionUnit` (inch or centimetre) |
| BMP | the header's pixels per metre |

Width in mm = pixels x 25.4 / horizontal dpi, height likewise with the
vertical dpi. A file that records no resolution, only an aspect ratio,
or a value outside 1 to 100000 dpi comes in at 96 dpi. Pasted images
and generated pictures are placed at 96 dpi.

Print-size pictures come in whole: decoding a picture file may take up
to 3 GB, 8 bytes for each of the 375 megapixels a PDF's picture is read
at at its own size (room for 16-bit samples, or a CMYK file's samples
beside the RGB result; `files::picture_decode_limit`). The image library's own
guard, 512 MiB, turned away a 1 m square at 300 dpi in CMYK or a
2.5 x 1 m banner at 300 dpi, and nothing came in (#2000,
`files::tests::print_size_pictures_decode_past_the_libraries_guard`,
`image_dpi::tests::a_picture_longer_than_a_metre_imports_at_its_size`).
The browser, Android and 32-bit systems keep the library's guard. A
file past the limit is not imported and the status bar says why.

## On screen and in exports

A picture is decoded once and kept (`crates/render/src/image_cache.rs`,
512 MB of decoded pixels in the desktop build, 256 MB in the browser;
the least recently drawn go first). Drawn at less than half its pixels'
size, it is drawn from a reduced copy instead: level `k` halves the
sides `k` times, each pixel the average of the four beneath it, and the
level used is `floor(log2(1 / s))` for `s` screen pixels per picture
pixel (at most 12), so it is sampled at between half and the whole of
its size. The reduced copies share the same memory budget and are made
when first needed. An upright picture (not turned or slanted) covers
whole pixels: its edges are rounded to the nearest pixel boundary, so
pictures laid side by side meet without a pale seam (#853,
`band_tests::pictures_side_by_side_meet_without_a_seam`,
`band_tests::a_picture_drawn_small_shows_the_average_of_its_pixels`).

## Checks

- Each reader gives the stored value and nothing for missing, broken or
  out-of-range data (`image_dpi::tests`).
- Given a PNG of 4 pixels at 300 dpi, when imported, then it is
  4 x 25.4 / 300 mm wide; a PNG without `pHYs` is 4 x 25.4 / 96 mm wide
  (`image_dpi::tests::import_places_a_bitmap_at_its_resolution`).

# pdfcraft-create

Layer L4. Create a PDF (Acrobat's Create a PDF tool), execution plan M10.2:

- `blank(width, height, pages)`: an empty document;
- `from_images(&[(name, bytes)])`: one page per image, sized from the image's resolution
  (PNG `pHYs`, JPEG JFIF density; 72 dpi when absent). JPEG data is embedded as is
  (`/DCTDecode`, grey, RGB or Adobe-inverted CMYK); PNG is decoded and stored with Flate,
  with transparency as a soft mask. An ICC profile in the file (JPEG `APP2 ICC_PROFILE`, PNG
  `iCCP`) that matches the image's colour space is kept as an `/ICCBased` colour space with
  the device space as `/Alternate`; images with the same profile share one profile object.
  JPEG metadata scanning skips marker fill bytes and reads application segments after the
  frame header, stopping at the first scan or end marker. Broken or mismatched ICC profiles
  still fall back to the device colour space;
- `from_images_with_resolution(images, ImageResolution::Dpi(dpi))`: override the page
  resolution (1–1200 dpi), without resampling or recompressing image pixels. Use 72 dpi
  for one point per pixel, or `ImageResolution::Embedded` for the same behavior as
  `from_images`. The largest page side is still capped at 14,400 points;
- `from_text(text, …)`: plain text set in Helvetica, wrapped and paginated. Text Helvetica
  can't draw is refused, naming the line and the character, never set as `?`.
- `from_text_escaped(text, …)`: the same for reports PdfCraft generates from documents
  (comment summaries, compare reports): such characters are written as `[U+XXXX]` and the first
  line says so (`ESCAPED_NOTE`).

Every function returns a `pdfcraft_cos::Document`; the caller writes it.

## Pages PDF previews

`pages_pdf_preview(bytes)` reads only `QuickLook/Preview.pdf` from a zipped `.pages`
file (128 MiB package, 10,000 entries, 256 MiB inflated preview). Nothing is extracted
onto disk and no external program or network service is called. The engine validates
that the result opens as a PDF and permits copying pages.

This is **preview extraction**, not a Pages/IWA renderer. The preview may be outdated
or incomplete. Missing, encrypted, oversized or damaged previews are refused; JPG
thumbnails are never substituted for the document. Export the document as PDF in
Pages when a current complete copy is needed. Folder-shaped macOS packages are not
accepted as files. ZIP64 and multi-disk archives are refused before directory allocation.

Open/Create and `doc_create_multiple`/`doc_batch` share this conversion. The GUI opens
an unsaved copy labeled “Pages PDF preview”; conversion results report the limitation.
The original `.pages` file is unchanged.

Apple documents the optional preview in the [Pages '09 save options](https://help.apple.com/pages/mac/4.3/help/English.lproj/pgs/sl14ea5fa6.html)
and [explicit PDF export](https://support.apple.com/guide/pages/tance1161f26/mac).

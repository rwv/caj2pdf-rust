# C8 link underline appearances in Wine

The missing dashed underlines are application-info **link annotations**, not
font glyphs or native body segments. This is now isolated independently of
the unsuccessful View-menu toggles. Issue #545 tracks their preservation;
closed #302 deliberately implemented only DOI, URL and annotation count.
Native conversion now preserves the measured profile as PDF Link annotations
with independent appearance Forms. The body, fonts, images and semantic text
remain unchanged. Unknown required appearances fail explicitly.

## Isolation on actual documents

For each of three C8 inputs, four variants preserve every byte preceding
the trailing `APPINFOSIGN` package: the original, a repack of identical XML,
XML with only the `NoteItems` children removed, and no application-info tail.
On the first and last page of each document:

- Repacking the unchanged XML retains exact pixels in all six views.
- Removing the notes removes the title/author/affiliation/keyword/reference
  underlines. Removing the complete package produces the same pixels.
- The six differences total 7,405 pixels, confined to the visible underline
  rows; all remaining body pixels are unchanged.

All six source/no-notes/difference views were visually inspected. The
13-document inventory has exactly three packages with these notes:

| Source prefix | Link annotations | Pages with annotations |
| --- | ---: | --- |
| `b9a64bf99e4b` | 35 | 1, 4 |
| `d6a23a9cfd0f` | 75 | 1, 2, 4, 5 |
| `90e7b47716c3` | 23 | 1, 2, 3, 4, 6 |

Each top-level item is `Type="Link"`, `LineStyle="4"`, `ShowType="1"`, with
one `RC` rectangle and a nested `UrlLink`. The containing `NoteItems` uses
`Unit="PointsM"` and version 1.0. Targets remain opaque data: no URL is opened.
Actual XML text and link destinations remain outside the repository.

The bundled help describes knowledge-link underlines, but toggling knowledge
links, forcing a page redraw and toggling annotation display did not remove
these particular marks. Both print and print-preview attempts require a
printer unavailable in the offline profile. Those attempts are not printing
or fidelity evidence; package isolation establishes the cause instead.

## Original coordinate and color controls

[`wine_link_controls.py`](../../scripts/research/wine_link_controls.py)
generates four independent documents: two declared page sizes, each with
links or an empty note list. Native fiducials remain identical. Each linked
document has two pages and four links per page; page 2 shifts each rectangle
edge by 1,000 stored units and reverses item order. URLs use `example.invalid`.
Black, gray-128 and red entries distinguish color handling. No external
document content, font data or vendor implementation is used.

At 75% and 125%, all eight linked views establish:

- Coordinates are hundredths of PDF points from the page's top left. Changing
  the declared page dimensions does not scale their positions. The page
  attribute controls which page paints the mark.
- For this profile, the visible underline follows the rectangle's bottom
  edge. Its observed device row is
  `floor(b / 100 × 96 / 72 × zoom / 100) + 2` within the independently
  extracted paper frame. This is a display observation, not a physical
  two-point margin rule.
- Colors follow `00BBGGRR`; the controlled ink is exact gray 128, black or
  red. Dash-dot-dot runs are `9,3,3,3,3,3` device pixels, alternating ink and
  gaps. The cadence and two-pixel gap do not scale with zoom.

All 32 controlled rectangle-origin predictions match. Four cold repeated
page visits are exact. All four generated sources and
their manifest reproduce byte-for-byte. Other line styles, visibility modes,
units, versions, color profiles and malformed rectangles remain unmeasured.

## PDF diagnostics and implementation

Both appended page strokes and PDF `/Link` annotations with normal appearance
Forms paint the marks in Wine. The tested six-entry PDF dash array appears
as uniform dashes; explicit short segments preserve dash-dot-dot structure.
The latter are preferred for further evaluation. The candidate uses a
nominal 96-DPI physical approximation: 6.75/2.25-point dash lengths and a
1.5-point gap. It takes no target zoom. A physical PDF pattern cannot retain
the native device-fixed cadence at every zoom.

Three diagnostic actual PDFs contain all 133 links. Fresh observations cover
all 11 affected pages. Body content, fonts, images and semantic text remain
unchanged; qpdf and independent text checks pass. The annotations preserve
their URI targets as data but no navigation is exercised. On inspected title
and reference pages, the opaque strokes are visibly too heavy. A separate
half-opacity diagnostic lightens them, while phase, thickness and cross-zoom
differences remain. Neither stroke treatment is adopted into production.

The exact differing-pixel counts increase when marks are added. This does
not rank an absent underline above a present one: object presence, placement
and normal-reading appearance determine acceptance. Exact RGB equality remains outside the acceptance gate; the physical
approximation and its limits are described below.

## Native link output

A 0.75-point filled diagnostic and then a 0.5-point filled treatment replace
hairline strokes. The latter preserves source RGB without opacity state and
reduces the excessive weight on title/reference pages. All 11 affected actual
pages and four original views (75%/125%, both pages) were inspected at reading
size. The marks are present in the correct locations; dash phase, lightness
and cross-zoom cadence remain approximate. These controls do not establish
native Windows equivalence or printing behavior.

The core now parses this profile with the existing bounded application-info
scanner and writes PDF `/Link` annotations, keeping page content and resources
unchanged. URIs are ASCII HTTP(S) data, entity-decoded and escaped as PDF hex
strings; they are never opened. Limits cap the XML, the 8,192-entry directory,
aggregate target storage, 4,096-byte attributes and per-page PDF references.
An in-place sort retains source order within each page and avoids scanning
the whole link directory for every page. No XML DOM, whole-input buffer,
source font change or new dependency is introduced.

Unknown required note profiles now fail native composition explicitly.
Malformed tails remain located warnings without partial metadata/links;
cancellation still fails. Inspection and the image-only diagnostic API keep
their previous metadata-only behavior. No public schema, option or ABI changes.
This conversion behavior change and its migration are in the release notes.

All 13 actual documents / 77 pages reconvert successfully, with qpdf and
113,418 source-order semantic characters verified. All existing raw content,
page boxes and resolved font/image resource graphs are exact. The three
annotated documents gain 133 links; the other ten PDFs remain byte-identical.
All 17 actual/control PDFs reproduce exactly with the final build. Fifteen
fresh production Wine views are pixel-identical to the inspected thin
prototype; this confirms implementation equivalence, not source raster equality.
Node and real Chromium checks cover the same 13 documents and four controls.

The [metadata receipt](wine-link-appearances-20261011.json) now retains
58 cold sessions, 57 admitted sessions and 131 admitted visits. One thin-PDF
startup failed numeric-layout readiness; it is excluded, and an independent
cold retry succeeds. The first production check exposed a zero-width terminal
rectangle caused by decimal subtraction. Quantizing to the serialized BBox
precision fixes it, with an original regression. Initial compile/test failures
are retained externally; final workspace tests pass (1,461, with seven ignored
optional tests excluded), plus one later write-failure regression. Final
targeted tests and strict clippy pass; JavaScript has 192 passes and no skips.

The pinned reference remains Wine 8.0 32-bit / CAJViewer 7.3.151 Simple,
1600 × 1200 at 96 DPI. Every admitted capture has confirmed numeric fields,
stable complete pages, intact input and cleanup without OOM. Existing
pikepdf diagnostics renumber object IDs; invariants compare resolved graphs
and raw payloads. Corpus, font, PDF, XML and screenshot data remain external.
No merge, complete document-format support or overall fidelity completion is
claimed: the separate ornament seam remains under #541.

Reproduce the original controls in a new directory:

```sh
python3 scripts/research/wine_link_controls.py /tmp/wine-link-controls
```

Open linked and empty variants separately at 75% and 125%, confirming numeric
page/zoom fields and two complete stable captures. Repeat in fresh copies of
the pinned profile. The receipt identifies the external diagnostic drivers;
the earlier PDF treatments remain diagnostics, while the bounded native
implementation adopts only the documented thin-filled Link appearance.

# C8 link underline appearances in Wine

The missing dashed underlines are application-info **link annotations**, not
font glyphs or native body segments. This is now isolated independently of
the unsuccessful View-menu toggles. Issue #545 tracks their preservation;
closed #302 deliberately implemented only DOI, URL and annotation count.
The converter still does not emit these annotations. Production remains the
rule-contrast implementation in `b19d62d`.

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

## PDF diagnostics and remaining work

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
differences remain. Neither treatment is adopted into production.

The exact differing-pixel counts increase when marks are added. This does
not rank an absent underline above a present one: object presence, placement
and normal-reading appearance determine acceptance. The current appearance
still needs a justified weight treatment, a bounded core parser/emitter,
malformed-tail handling and native/CLI/Node/browser verification. No completed
annotation compatibility or complete rendering fidelity is claimed.

The [metadata receipt](wine-link-appearances-20261011.json) retains 44 cold
sessions and 95 admitted visits, along with source,
PDF, code and capture hashes, admission checks, comparisons and failures.
The reference remains pinned Wine 8.0 32-bit / CAJViewer 7.3.151 Simple,
1600 × 1200 at 96 DPI. All admitted captures have confirmed numeric fields,
stable complete pages, intact inputs and container cleanup without OOM.
Initial PDF verification rejected renumbered indirect references; comparing
resolved resource graphs and raw stream payloads proves preservation without
claiming unchanged object IDs. The four half-opacity PDFs also reproduce
byte-for-byte. PDFs, screenshots, original fonts and corpus
documents remain external.

Reproduce the original controls in a new directory:

```sh
python3 scripts/research/wine_link_controls.py /tmp/wine-link-controls
```

Open linked and empty variants separately at 75% and 125%, confirming numeric
page/zoom fields and two complete stable captures. Repeat in fresh copies of
the pinned profile. The receipt identifies the external diagnostic drivers;
all generated PDF treatments are research-only. No production API, dependency
or memory behavior changes in this report.

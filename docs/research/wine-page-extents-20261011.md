# Wine page extents: refining the empirical coordinate unit

The global coordinate scale changes from `240/2473` to `72/742` PDF points
per raw unit. Independent C8 page-boundary controls select the candidate;
new dimensions and zooms then test it. All 77 pages in the existing native
inventory now have the same page extents as Windows CAJViewer 7.3 under the
pinned Wine profile. Every actual page still has pixel differences.

This is a scoped implementation for [#541](https://github.com/rwv/caj2pdf-rust/issues/541).
The [metadata receipt](wine-page-extents-20261011.json) records 67 new Wine
sessions, reused references, source/output hashes, exact page comparisons,
semantic checks and native/Node parity. It retains negative results and two
small pixel regressions. No fonts, corpus documents, PDFs or screenshots are
committed. This evidence does not establish native-Windows equivalence.

## Independent boundaries and held-out predictions

Four original sources have widths 4736, 5105, 5166 and 5168 raw units and
height 2800. Each contains the same 25 original rectangle glyphs at fixed
coordinates. Source and old PDF are opened separately at 75%, 100% and 150%,
on a 1600-by-1200 display at 96 DPI. Two further cold source repeats cover
width 5168 at 75% and 150%: 26 initial sessions altogether.

Page-white extents exclude the measured viewer border. A single physical
scale followed by nearest-device-pixel rounding must lie in
`[0.09702864885791715, 0.09703721841332029]` points per unit to satisfy the
initial width controls. The old `240/2473` is outside that interval;
`72/742` is inside. The interval does **not** uniquely prove that fraction,
a physical 742-DPI source format, or the viewer's internal rounding algorithm.
The prior long-interval [coordinate grid](wine-cjk-aliases-20261011.md)
could reject coarser alternatives but did not resolve this narrower error.

Three new square dimensions test predictions made after selecting the
candidate. The original scale fails at 2973/75%, 4038/100% and 5150/150%,
each by one pixel; the new scale predicts both axes for all nine views.
Cold source repeats of those three discriminating cases are pixel-identical.
The initial two cold repeats are also exact. Observed white-page widths are:

| Raw width | Raw height | 75% | 100% | 150% |
| ---: | ---: | ---: | ---: | ---: |
| 4736 | 2800 | 460 | 613 | 919 |
| 5105 | 2800 | 495 | 660 | 991 |
| 5166 | 2800 | 501 | 668 | 1003 |
| 5168 | 2800 | 501 | 669 | 1003 |
| 2973, held out | 2973 | 288 | 385 | 577 |
| 4038, held out | 4038 | 392 | 522 | 784 |
| 5150, held out | 5150 | 500 | 666 | 999 |

The initial height 2800 produces 272/362/543 pixels; square heights equal the
listed widths. The regression test stores all 42 observed axis extents as
literal measurements, independently of the implementation constant.

Seven fresh candidate PDF sessions cover four initial boundaries/sentinels
and the three discriminating held-out cases. Every extent matches. Five
source/PDF pairs are pixel-exact; the two 100% controls still differ by 330
pixels each. No page is resized, aligned to its text, or fitted separately.

## Actual documents and separate page-box control

All 13 documents / 77 pages are freshly converted and viewed with the
explicit mode-specific font packs from the [resource study](wine-font-resources-20261011.md).
All 35 previous extent mismatches disappear. Among the 42 pairs whose
extents already matched, 30 improve, ten remain unchanged, and two worsen:
`90e7b47716c3…` pages 4 and 5 increase by 136 and 335 differing pixels.
The comparable 42-page cohort has 97,609 fewer differing pixels in total.
No before/after pixel-count improvement is assigned to the other 35 pages,
whose old extents prevented a strict comparison. All 77 retain nonzero
pixel differences; ten image-only pages are not typography evidence.

A separate control changes only the old PDF MediaBox and translates content
to preserve every top-relative painted position. Across 14 pages in three
documents, the global coordinate change has fewer differing pixels on 13.
The exception is `03770ea1cdeb…` page 3: page-box-only gives 47,267 versus
48,729 with the global change. This distinguishes a page-border-only repair
from the shared coordinate correction without selecting per-document factors.

Qpdf accepts all 13 outputs. An independent source-order ToUnicode/ActualText
check preserves all 113,418 characters. Public Node conversion produces
byte-identical PDFs for every document with bounded chunks. The affected
Node and real Chromium geometry tests pass, and the complete JavaScript
suite passes all 190 tests. Locked native workspace tests pass 1,431 tests;
seven optional tests remain ignored and are not corpus compatibility passes.

## Additional formats and viewer stability

Fresh before/after conversions cover seven additional files. The 3-page C8,
37-page HN-A and 433-page NH outputs change. Wine checks all three C8 pages,
all 37 HN-A pages, and NH pages 1, 217 and 433: all 43 selected page extents
match the source. The remaining 430 NH pages were converted and qpdf-checked
but have no fresh viewer observation. Full-document conversion is not a
claim of full-document visual fidelity.

Fresh old-constant PDF sessions supply the HN-A and NH baselines; historical
PDF observations are reused for C8 only after proving PDF byte identity.
The HN-A baseline differs in extent on all 37 pages, so no old/new pixel-count
score is assigned. NH page 1 improves; the other two selected NH pages
are unchanged. Pixel differences remain in these additional formats.

The two CAJ files (75 and 64 pages), KDH (three pages) and PDF (three pages)
are byte-identical before and after this change and pass qpdf. They are
negative conversion controls, not fresh Wine typography observations.

A second cold candidate session for `90e7b47716c3…` reproduces all six pages
exactly. Its earlier intermittent startup failures remain recorded. The
`3f3b9b57d692…` observations retain the documented canonical bootstrap route.
Each admitted visit requires numeric page/zoom confirmation, two stable
captures, independent complete page bounds, unchanged input, no OOM and
container cleanup. Legacy source captures with unrelated UI components are
judged by the same independent complete-page crop used in the earlier study;
no raster tolerance or comparison boundary is widened.

## Reproduction, scope and provenance

Generate the seven exact original sources in a new directory:

```sh
python3 scripts/research/wine_page_extents.py /tmp/page-extent-controls
python3 scripts/research/wine_ornament_geometry.py /tmp/rectangle-controls
```

The first generator uses only the standard library and the project's
original MIT C8 framing; all seven hashes match the observed sources. The
second uses fontTools 4.62.1 to reproduce the original rectangle font at
`advance-1/HGBZ_CNKI.ttf`. In disposable Wine prefixes this font replaces
HGBZ only; source coordinates and other resources remain fixed. Use it for
both explicit `--font-cjk` and `--font-latin` roles when converting controls.
Use the receipt's zoom, source hash, driver hash, viewer image digest and
[reference profile](wine-font-resources-20261011.md) for comparisons.

The production change is one global constant shared by admitted HN/C8/NH
geometry. It does not change the separate `72/300` image-pixel or `75/301`
font-em factors. Native APIs, CLI, Node and browser retain their signatures,
I/O bounds and dependencies, but generated geometry and PDF hashes change.
Existing arithmetic tests are updated to exact rational results and binary64
reference values; their placement tolerance is unchanged. One content-matrix
test compares parsed numbers within `1e-12` rather than requiring identical
last-decimal serialization. This is not a viewer-pixel tolerance.

Production font subsetting remains unchanged. Original production source,
generators and tests are MIT work. No foreign converter, vendor implementation
or private HN/JBIG code was inspected or copied. #541 remains open for
recoverable font, glyph-placement and raster differences.

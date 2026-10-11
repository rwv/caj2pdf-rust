# Wine vertical CID origins and metric encodings

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
The [receipt](wine-vertical-origins-20261011.json) retains 35 cold Wine sessions,
metric definitions, exact frames, original-font hashes, reproduction and failures.
The preceding [transparency study](wine-transparency-controls-20261011.md) did
not change the remaining glyph-origin/axis differences.

## Result

Moving offsets into vertical CID metrics does not improve the measured source
fidelity. Of **23 mathematically geometry-equivalent views**, 19 retain the PDF
baseline exactly and four regress. None matches the native source. Ten deliberate
displacement controls and two exact cold source repeats are recorded separately.
No production change is adopted.

Wine responds differently to the tested metric representations. At all three
low-zoom views, the single-CID range form responds to a positive displacement;
the array form and broad `0..65535` range do not show that displacement. Default
metrics also respond. This is a scoped observation, not a universal declaration
of supported syntax or an identified internal parser cause. After choosing the
responsive representation, the equivalent origin decompositions still retain
baseline pixels at low and high zoom.

## Definitions and original controls

PDF vertical writing defines a position vector between horizontal and vertical
glyph origins. A CIDFont's `W2` supplies per-CID vectors; `DW2` supplies defaults,
with the default x component derived from half the glyph width. The mathematical
horizontal painting origin is therefore `(ox - a*vx/1000, oy - d*vy/1000)` for
the axis-aligned text matrices used here. These definitions come from Adobe's
[PDF Reference 1.5](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf),
under “Glyph Metrics in CIDFonts.” They do not prescribe how Wine quantizes
device pixels.

The inputs are the four independently authored HN-B axis fixtures and their
original-program PDFs from production implementation
`7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`. The checkout before this study is
`d9eadd2`; its later changes are research or a test-only race correction.
The low fixtures have 42 glyphs over seven profiles; the high fixtures have
16 over four profiles. Reversed fixtures also change positions. No actual
document or article text is added to this batch.

Each glyph uses a cloned Type 0/TrueType CID resource, retaining the original
font program, CID-to-GID map, ToUnicode map and glyph identity. Cloning with
horizontal encoding is a separate control. Vertical candidates use Identity-V.
The text scale is unchanged. Depending on the variant, the old origin, raw x
anchor, raw y anchor or both anchors appear in `Tm`; the metric vector supplies
the compensating displacement. All such decompositions are checked within
1e-25 points, including after serializing and rereading the PDF.

`pikepdf.Object.parse` preserves the decimal metric tokens. Direct array
construction from Decimal was observed to shorten a test value, so it is not
used for the variable metrics. This is a preparation precision check, not a
discovered production rounding defect.

## Staged discrimination

The first four 75% sessions compare horizontal clones, zero per-CID vertical
metrics, an equivalent y-origin split and a deliberate `(125,250)` metric vector.
The horizontal clone is exact to baseline. Zero array metrics move glyphs left
instead of retaining their horizontal origins; the positive array vector gives
the identical frame to those zero metrics. The y-split array candidate regresses.

Seven additional 75% sessions distinguish representations and default metrics:

| Metric control | Wine observation |
| --- | --- |
| Single CID with an array of three values | Positive and zero frames are identical |
| Range whose first and last CID are the painted CID | Positive vector changes the frame |
| Range `0..65535` | Same frame as zero array metrics |
| Default width 0, 500 or 1000, with equivalent x compensation | All three retain baseline pixels |
| Default y-origin compensation | Retains baseline pixels |
| Default y vector 250 without compensation | Changes the frame |

Default-metric controls deliberately remove per-CID `W`/`W2` entries and set
`DW`/`DW2`. Thus **PDF width metadata changes**, although the font program and
individual glyph's mathematical painting geometry remain unchanged. Each `Tj`
paints one glyph at an explicit matrix. These are diagnostics, not a proposed
general text-layout or accessibility solution.

The final 24 sessions use responsive single-CID ranges. Four equivalent cases
(zero, x split, y split, both split) cover low forward 75%/150% and reversed,
shifted 100%. The y split additionally covers high 600% and reversed 1100%.
Array/range/broad-range positive controls and zero-array references are repeated
at 150% and 100%. Two independent cold source repeats cover low 75% and high 600%.

| View | Baseline and every responsive equivalent range candidate: source differences |
| --- | ---: |
| Low forward 75% | 624 |
| Low forward 150% | 1,620 |
| Low reversed/shifted 100% | 1,314 |
| High forward 600%, y split | 2,947 |
| High reversed/shifted 1100%, y split | 5,701 |

All 14 responsive range views equal their PDF baselines. The horizontal clone
and four default-metric equivalents contribute the other five unchanged views.
The zero-array candidates regress from 624 to 1,940, 1,620 to 5,643, and 1,314
to 3,121 at the three low views. The 75% array y split regresses from 624 to
2,095. These four regressions remain in the receipt.

Relative to zero-array frames, the single-CID positive-range control changes
1,920 / 5,605 / 3,042 pixels at 75% / 150% / reversed 100%. The corresponding
array and broad-range positive controls change zero pixels. The default y
positive control changes 1,513 pixels relative to its default-metric baseline.
Those deliberate displacements are not fidelity candidates.

## Observation quality and reproduction

All 35 sessions are independent cold Windows CAJViewer 7.3.151 Simple / Wine 8.0
32-bit sessions using the established fonts/MSXML profile, offline and nonroot
with read-only input/root, a 2 GiB memory/swap limit and 600-second lifetime.
Display is 1600 × 1200 at 96 DPI, in single-page mode. Numeric page/zoom fields
are confirmed around two captures 0.6 seconds apart. All visits are complete and
stable; all sources remain intact and all containers are removed.

Complete-frame RGB comparison includes the observed page border, with no
registration, rescaling, tolerance or normalization. The inherited glyph-cell
analyzer initially asserts because 206 pixels of the array y-split candidate
escape its narrow diagnostic cells. Reanalysis of the **same captures** uses
the original low grid's midpoints (300 raw-unit column spacing and 280-unit row
spacing). Whole-frame scoring is unchanged. Two deliberate 75% displacement
controls still have one pixel outside those expanded diagnostic cells; that
pixel remains in their frame comparison. Glyph-cell diagnostics are not used
to claim full coverage for those controls. No GUI failure is replaced or hidden.

The original MIT helper
[`wine_vertical_origin_diagnostic.py`](../../scripts/research/wine_vertical_origin_diagnostic.py)
reproduces all **29 generated outputs, representing 25 distinct PDF hashes**,
byte-for-byte with identical metadata. Earlier helper prototypes are retained
by hash; the final helper also reproduces their outputs exactly. Font bytes,
page bounds and semantic text are verified, and every output passes qpdf.
The pilot/default PyMuPDF 1.27.2.2 renders show the declared positive shifts and
retain geometry-equivalent pixels; they are independent sanity observations,
not the requested Wine reference or proof about Wine's parser.

```sh
python3 scripts/research/wine_vertical_origin_diagnostic.py \
  /external/axis-raster-low-0-baseline.pdf /external/new-origin.pdf \
  --family low --kind r-y --receipt /external/new-origin.json
```

Use matching `--family high`/`--reverse` inputs for other fixtures. `v-zero` and
`v-y` select array probes, `r-*` selects single-CID range equivalents, and `dv-*`
selects deliberately changed default metrics. Named positive controls distinguish
the metric formats. The CLI lists all accepted variants; binaries remain external.

The preceding `d9eadd2` passes all four quality gates and nine regular native
jobs: 15 successes including inputs/assets, with 17 optional/release skips
excluded from passing evidence. This new research commit has separate CI and
adds no native/Node/browser runtime result because production is unchanged.
The existing 77-page actual cohort retains its remaining differences. #541
stays open; identifying an ineffective encoding is not acceptance. No private
HN/JBIG, foreign converter or vendor implementation is inspected or copied.
No production dependency, API, I/O strategy, support or release changes.

# Wine ornament spacing and PDF encoding controls

Follow-up to #541 and [the per-value glyph map](wine-ornaments-20261011.json).
The remaining repeated-pattern difference is reproducible with original
rectangles, so it is not explained by choosing the wrong vendor glyph. In the
measured Wine display profile, the repeated step is the nominal em in device
pixels rounded **down to an integer**. The current PDF retains fractional
origins and a zoom-independent nominal step. This is a display observation,
not a new authoritative physical-unit rule or a permission to fit output to
one screenshot.

The [metadata receipt](wine-ornament-spacing-20261011.json) retains each input,
font and page identity, exact comparisons, marker diagnostics and rejected PDF
encodings. External source documents and vendor outlines are not used by these
controls. The existing 77-page actual-input differences remain open.

## Original control

[`wine_ornament_geometry.py`](../../scripts/research/wine_ornament_geometry.py)
reproduces the observed C8 and both fonts byte for byte with fontTools 4.62.1.
It authors one 4,500 × 4,500-unit page, origin `(4652,4274)`, with nine ornament
rows. Each starts at x=4950 and spans 3,300 source units; successive y positions
are 400 units apart. The six style fields are 2, 3, 4, 5, 6 and 8. A second
field-5 row uses ornament 117, and two rows use explicit equal axes 34 and 40.
Other rows use ornament 1. Existing state restrictions remain unchanged.

The font is the project's original quarter-em-wide upper-half rectangle at
all four measured ornament aliases. The second font doubles the selected
advance from 1,000 to 2,000 units without changing cmap or outlines. Derived
metrics remain consistent: `hhea.advanceWidthMax` and OS/2 average width also
change, as does the `head` checksum. These are original generated fonts; vendor
font programs are not inputs.

The reference remains Wine 8.0 32-bit / CAJViewer 7.3.151 Simple in the pinned
offline container and font/MSXML profile, at 1600 × 1200 and 96 DPI. Each cold
session confirms numeric page/zoom fields and two stable, complete captures.
Source bytes, cleanup and OOM status are checked. The four zooms 50%, 75%,
100% and 150% were declared before capture. Each original source has a fresh
cold repeat; all four repeat comparisons are exact. The doubled-advance source
is also exact at 75% and 150%.

## Observed spacing

All 36 source-row observations agree with
`floor(em_points × 96/72 × zoom/100)`. The existing em model is unchanged:
style steps 28/31/35/42/48/63 multiplied by `75/301` points; explicit axes use
their own 34/40 values. The coordinate unit stays `240/2473` points. No factor
or per-document offset is estimated from these candidate pixels.

For the field-5 row (including the independently queried value-117 row):

| Zoom | Wine source pitch | PDF pitches | Source visible marks | PDF visible marks |
| ---: | ---: | --- | ---: | ---: |
| 50% | 6 px | 6/7 px | 36 | 31 |
| 75% | 10 px | 10/11 px | 33 | 31 |
| 100% | 13 px | 13/14 px | 33 | 31 |
| 150% | 20 px | 20/21 px | 33 | 31 |

Pitches and counts are diagnostics from separate connected components with
all RGB channels below 200; clipped final marks remain counted. They are not
an acceptance tolerance. Exact whole-page RGB comparisons retain every pixel
without registration, resampling or border removal. Baseline differences are
2,918 / 5,960 / 10,923 / 21,626 pixels at the four zooms. The equal page extents
of these controls do not resolve the actual cohort's separate width differences.

A fixed PDF advance cannot equal all these source steps after scaling. Even
one unchanged source has zoom-dependent mark counts. A 75%-specific offset or
reduced em would trade one display discrepancy for another and change the
physical model without independent evidence.

## Rejected PDF alternatives

A single text object containing 128 repeated aliases under the same original
clip tests whether the PDF viewer uses a different text-layout route from the
current separate text matrices. It retains the same embedded font and nominal
em. It does **not** acquire the source's integer step. At 50%, 100% and 150% its
page is exactly the baseline PDF; at 75% only three right-endpoint pixels
differ. Its source difference count increases from 5,960 to 5,963 at 75%
and is unchanged at the other three zooms.

Replacing only the original rectangle glyphs with equivalent PDF rectangle
paths also retains fractional positions. At 75% and 150% it increases the
source difference to 8,364 and 26,710 pixels. This does not justify adding a
font-outline renderer or replacing arbitrary ornament glyphs with guessed lines.

PDF tiling patterns provide an independent candidate: `TilingType` 1 and 3
permit constant device-grid spacing, while type 2 preserves the cell shape with
variable spacing. The specification does not promise the source's rounding
direction. See [Adobe PDF Reference 1.6, section 4.6](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).

The initial pattern candidates use the same font glyph, nominal em, first
origin and endpoint clip. All three tiling modes are tested at all four zooms.
A sparse vertical step keeps other vertical tiles outside the page. Further
controls use a one-em vertical step and paint strip, an identical rectangle
instead of the font, and equivalent point-space cells instead of a scaled
unit cell. The point-space controls add an ordinary red rectangle as a positive
page-painting control; whole-page parity is intentionally inapplicable to that
extra-fiducial diagnostic. Explicit named/array and uncolored DeviceRGB pattern
color spaces check a separate encoding route.

All 23 pattern sessions have stable complete pages but no ornament markers.
The seven sessions with the added red rectangle display that rectangle:
420 red pixels at 75% and 1,640 at 150%, using the recorded diagnostic threshold.
All thirteen unique PDF files pass qpdf and extract no text. MuPDF 1.27.2
paints the original markers in every file, including the rejected pattern
candidates. This distinguishes an observed Wine rendering failure from an
empty input; it does not establish that every PDF pattern is unsupported.
Blank pattern pages can have fewer changed pixels than incorrect visible
patterns, so their numerical difference counts are explicitly disqualified
as fidelity improvements.

## Reproduction

With fontTools 4.62.1 installed, generate original inputs in a new directory,
then use the CLI built from `5acaf1d6cbeb85528671e3de241ba437ce04400e`:

```sh
python3 scripts/research/wine_ornament_geometry.py /tmp/ornament-controls
caj2pdf /tmp/ornament-controls/ornament-metrics.caj \
  -o /tmp/ornament-controls/ornament-metrics.pdf \
  --no-bookmarks --no-system-fonts \
  --font-cjk /tmp/ornament-controls/advance-1/HGBZ_CNKI.ttf \
  --font-latin /tmp/ornament-controls/advance-1/HGBZ_CNKI.ttf \
  --font-decoration /tmp/ornament-controls/advance-1/HGBZ_CNKI.ttf \
  --decoration-char '崄' \
  --decoration-glyph 1=U+5D04 --decoration-glyph 2=U+5D05 \
  --decoration-glyph 46=U+5D41 --decoration-glyph 117=U+5D95
qpdf --check /tmp/ornament-controls/ornament-metrics.pdf
```

The generated manifest pins both font hashes and the C8 hash. These three
files and the CLI-produced PDF reproduce the observed bytes exactly. In
disposable copies of the existing Wine reference profile, replace only
`Program Files/Common Files/TTKN/Bin/Resource/cajfonts/HGBZ_CNKI.ttf` with the
generated advance-1 font. Open the original C8 and PDF separately at each
declared zoom, confirming page `1/1`, exact zoom and two stable complete frames.
Use a fresh profile for each source repeat. Repeat the source at 75% and 150%
with the advance-2 font. Do not install these geometric controls as real text
fonts. The metadata pins the container, CLI, driver, input and capture hashes;
the alternative PDF encodings are described above.

## Scope

No production placement rule is changed by this study. The original source,
advance controls and PDF alternatives are evidence for choosing the next
implementation step, not successful source/PDF compatibility tests. The font
selection and per-value mappings in #542 remain useful and unchanged. #541
stays open for remaining source-font, placement and rendering discrepancies.

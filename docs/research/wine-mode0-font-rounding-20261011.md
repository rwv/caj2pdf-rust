# Wine mode-0 font metrics and device rounding

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
Following the [page-padding correction](wine-mode0-page-padding-20261011.md),
this study separates remaining glyph-position differences from font vertical
metrics and equivalent PDF encodings. The [metadata receipt](wine-mode0-font-rounding-20261011.json)
retains **56 sessions: 52 admitted and four failed diagnostic startups**.
No production geometry or font program changes follow from these measurements.

## Fixed-outline font controls

Two original 72-glyph grids cover six styles, Han, ordinary Latin and alternate
Latin, with four small position increments per row. The second grid reverses
styles and alphabets, shifts positions, and is held out at 100%; the first is
observed at 75% and 150%. All use the corrected ten-point page padding and
the same pinned CAJViewer 7.3.151 Simple Wine profile at 1600 × 1200 / 96 dpi.

Five original geometric resource programs have identical full-em outer bounds
and distinct interior markers. Four font treatments change only vertical
metrics, from ascent/descent 1000/0 to 1250/250: Windows metrics only,
`hhea` only, typographic metrics only, or all three together. Signed descenders
use −250. Outlines, cmap, advances, units-per-em and names remain unchanged;
only the relevant tables and `head` checksum change. The generated PDFs retain
identical page boxes, content bytes, widths and semantic maps across treatments.

All eight treatment/zoom comparisons have **zero changed source pixels and
zero changed PDF pixels** relative to the baseline. Two cold source repeats
also agree exactly. A separate positive control translates only the original
glyph outline upward by 200 font units: it changes both source and PDF frames
at both zooms, demonstrating that the supplied treatment programs affect
rendering. The metric-only negative result is therefore not accepted merely
because two screenshots look similar.

The [OpenType OS/2 specification](https://learn.microsoft.com/en-us/typography/opentype/spec/os2)
distinguishes Windows and typographic vertical metrics; the
[CreateFont documentation](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-createfontw)
also distinguishes cell height from character height. These definitions motivated
the controls. They do not establish how CAJViewer draws document glyphs.

## Equivalent PDF transforms

Four encodings preserve the mathematical glyph transforms and font programs:
move the vertical scalar from `Tm` to `Tf`, move the whole transform to `cm`,
move only translation to `cm`, or express the Latin baseline component through
text rise `Ts`. Each is observed at 75%, 150%, and the held-out reversed 100%
grid. All **twelve frames are byte-exact to the original PDF frame** in cropped
RGB. Qpdf and extracted-text checks pass; the maximum factorization arithmetic
error is below 1e-20 point. None of these encodings resolves the discrepancy.

The baseline source/PDF grids differ by 3,940 pixels at 75%, 6,720 at 150%, and
4,037 in the reversed 100% case. Comparisons preserve complete page boundaries
and original pixels; no registration, resampling or tolerance is applied.

## High-zoom discriminator

Two further original six-glyph pages compare equal 35 × 35 and 42 × 42 axes
with unequal 42 × 35 axes. Complete pages fit at 300%, 600% and 1200%; reversed
styles/alphabets with shifted positions are held out at 900%. Source and PDF
glyph bounding-box dimensions agree at all four high zooms. Vertical position
differences remain 0–2 pixels rather than growing in proportion to zoom.

For identical geometric glyphs placed at the same raw y, all **156 CJK/Latin
pairs** across both grid families have the following source top-edge difference:

| Zoom | Observed Latin minus CJK y, pixels | Pair count |
| --- | ---: | ---: |
| 75% | 0 | 48 |
| 100%, held out | 1 | 48 |
| 150% | 1 | 48 |
| 300% | 3 | 3 |
| 600% | 7 | 3 |
| 900%, held out | 11 | 3 |
| 1200% | 15 | 3 |

Every value equals `floor(10 × 96/742 × zoom/100)`. This supports independently
quantizing the existing ten-raw-unit class offset in these controls. It does
not change the physical offset to five units: that would conflict with the
observed fifteen-pixel separation at 1200%. Neither a five-unit replacement
nor a constant screen-pixel translation was implemented.

The low-zoom unequal-axis width differences disappear in these high-zoom
controls. Their observed dimensions remain consistent with the existing
`75/301` point font-em model, rather than justifying a new global font-size
scale from one low-zoom bounding box. The relative class offset is not a complete
placement model: absolute glyph origin, bitmap bounds, axis quantization and
other shapes still require independent evidence. Actual-corpus improvement is
not claimed from these synthetic measurements.

## Diagnostic limitations and failures

Four initial Wine font/GDI diagnostic sessions reached the existing 64 MiB log
cap before canonical startup could be confirmed. They have no admitted page
views and remain failed experiments. A new harness enables font diagnostics
only for the viewer, streams away unrelated messages, and retains selected
font-creation/text-output arguments under an 8 MiB cap. All four resulting
frames exactly equal their unobserved baselines; retained logs stay below the
cap. These messages contain UI font creation, but no creation event names one
of the five measured content resources. The selected trace therefore does not
establish the content-drawing path or explain its rounding.

An initial local harness-preparation assertion and a missing receipt during
generator bootstrapping also failed before their intended runs. Corrected
preparation is recorded separately. No viewer binary, vendor implementation,
foreign converter or private parser was inspected or copied. Diagnostics and
all control mutations concern original synthetic documents and geometric fonts.
The archived CAJViewer resource programs remain unchanged.

## Reproduction and current scope

The original MIT generator reproduces four observed source files using only
the Python standard library. With the five hash-pinned original geometric
inputs and fontTools 4.62.1, it also reproduces all thirty baseline/treatment
font programs exactly. It refuses other input font hashes.

```sh
python3 scripts/research/wine_mode0_font_rounding.py /tmp/new-source-controls
python3 scripts/research/wine_mode0_font_rounding.py /tmp/new-font-controls \
  --geometric-fonts /external/original-marker-fonts
```

The receipt contains source/font/driver hashes, failed and admitted sessions,
capture identities, per-glyph bounds and class-offset measurements. Font
binaries, documents, PDFs and screenshots remain external. The stable-frame,
numeric-page/zoom, complete-boundary, source-integrity and cleanup admission
criteria are unchanged. All admitted sessions retain two identical viewport
captures and finish without OOM or retained containers.

Production commit `a2206d9` passed all eight required quality/Linux checks.
This follow-up adds original research code and evidence only; it does not
relabel prior actual-corpus observations as new tests or resolve #541's broader
placement/fidelity acceptance criterion. The next useful discriminator is the
conversion from physical glyph geometry to independently quantized device axes
and origins, with source-frame controls retained throughout.

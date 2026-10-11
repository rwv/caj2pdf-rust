# Wine Type 3 rasterization controls

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
Type 3 character procedures do not resolve the remaining Wine differences in
these controls. Nested original TrueType text with `d0` retains every baseline
pixel. The tested `d1` nested-text procedures paint no glyphs. Direct outline
paths paint but regress every complete frame. No production change follows.

The [receipt](wine-type3-rasterization-20261011.json) records 62 fresh Wine
sessions, two exact cold source repeats, 32 reproduced diagnostic PDFs and
all negative outcomes. These are original synthetic controls, not additional
actual-corpus compatibility tests. The existing 77 actual pages remain at
the [alphabet-origin implementation](wine-mode2-alphabet-origins-20261011.md),
all with remaining pixel differences.

## Question and controlled encodings

Previous `Tf`, `Tz`, `Ts` and `cm` factorization experiments retained baseline
pixels. Type 3 tests a further boundary: a character procedure can have its own
matrix and drawing operations. The
[PDF Reference, section 5.5.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
permits path and text objects inside these procedures. This motivates the
experiment; it does not establish the viewer's rendering implementation.

The four original axis-study HN-B controls contain isolated Han, Latin letters
and a digit, with equal/unequal em dimensions. Forward controls use 75%, 150%,
300%, 600% and 900%; reversed/shifted controls use 100% and 1100%. The pinned
reference remains Wine CAJViewer 7.3.151 Simple at 1600×1200 / 96 dpi. Fresh
baseline PDFs use production commit `7f8b7e7`, including corrected alphabet and
digit positions, with the same external original-font configuration.

Three encodings first preserve each existing TrueType glyph program and exact
mathematical transform:

- `direct`: an uncolored `d1` procedure contains the original font draw;
  the outer text matrix retains the complete baseline translation.
- `split`: `d1` keeps the recorded source origin outside the character procedure
  and moves its measured glyph displacement inside it. Their composition is
  unchanged to within 1e-25 point.
- `split-d0`: the same split geometry uses a `d0` procedure.

The first two paint no glyph ink in all fourteen views. One 900% split capture
also has different detected frame dimensions and is rejected for strict pixel
comparison; its evidence is retained without alignment or resizing. The other
thirteen complete frames explicitly record missing glyphs rather than counting
blank output as compatible rendering.

All seven `split-d0` frames are **byte-exact to their baseline RGB frames**.
Their combined difference from the source stays at 21,382 pixels. Moving the
origin into these procedures therefore does not select the desired independent
device rounding.

Two further 75% nested-text controls use separate Type 3 fonts per glyph,
the conventional 0.001 font matrix, and either square or independent-axis
normalization. Both remain blank. Two positive controls replace the inner text
with visible rectangles. Both paint exactly the same 4,104 interior pixels,
despite different coordinate encodings. This distinguishes the tested nested
text behavior from a general inability to draw Type 3 paths. It is not a claim
about every Type 3 font or other viewers.

## Original outline paths

The next controls replace nested text with paths from the **same original font
glyph IDs**. Quadratic segments are expressed as equivalent cubic PDF curves;
composite handling uses the font library's glyph drawing interface. The selected
glyphs contain no per-glyph TrueType instructions. Original embedded program
bytes remain unchanged, although the diagnostic now paints paths instead of
invoking those programs. Semantic text is preserved by the outer ToUnicode map.

Direct/split origins are each tested in point coordinates and normalized
independent axes. Every source glyph cell retains its declared bounds. Complete
page comparisons use strict RGB, without registration, resampling or tolerance;
tight glyph crops diagnose shape only.

| Outline treatment | Views | Baseline differing pixels | Candidate pixels | Improved views |
| --- | ---: | ---: | ---: | ---: |
| Direct, point coordinates | 7 | 21,382 | 105,925 | 0 |
| Direct, normalized axes | 7 | 21,382 | 105,959 | 0 |
| Split, point coordinates | 7 | 21,382 | 107,032 | 0 |
| Split, normalized axes | 7 | 21,382 | 107,052 | 0 |

Every one of the 28 complete frames regresses. None of 760 candidate glyph
observations has an exact tight bitmap. Two point-coordinate cells are missing
entirely in the reversed 100% control: `文` at 42×42 in the direct treatment,
and `A` at 28×28 in the split treatment. Axis normalization restores those two
glyphs, but still worsens whole-frame fidelity. These failures remain in the
receipt. Replacing font rendering with these equivalent outlines is not adopted.

## Validation, failures and reproduction

All 62 visits have confirmed numeric page/zoom controls and two stable viewport
captures. The one mismatched frame extent is excluded from strict comparison.
Both independent source repeats match exactly. Source integrity and cleanup
checks pass for all sessions; no experiment container remains.

Preparation initially accessed a pikepdf page after its temporary owner had
been destroyed. It failed before Wine; its output is retained separately.
Corrected preparation holds the page box while the PDF is open. Initial analysis
also stopped when it assumed that every candidate cell must contain a glyph.
The final analyzer records missing glyphs and the noncomparable frame explicitly.

The four-session normalization host runner received an unexplained termination
signal during setup (exit 143). Inspection confirmed that the original four
containers and viewers were still running. Their numeric fields were read;
the same sessions resumed at 75%, completed captures and cleaned up. No session
was restarted merely because the host observer stopped. The interruption,
original action logs and continuation remain recorded.

All diagnostic PDFs pass qpdf and independently preserve the source-order
semantic text. The original MIT helper reproduces all **32 diagnostic PDFs**
by page content, Type 3 dictionaries/procedures, page bounds and embedded font
bytes. PDF object numbering and file serialization are not claimed byte-exact.
It uses pikepdf and PyMuPDF, plus fontTools for outline paths. No runtime
dependency or public conversion API changes.

Generate the source fixtures with the existing axis-study generator, then
convert them with the measured original-font configuration and the alphabet
implementation. Given the matching `axis-raster-low-0` baseline PDF:

```sh
python3 scripts/research/wine_type3_diagnostic.py baseline.pdf direct.pdf \
  --family low
python3 scripts/research/wine_type3_diagnostic.py baseline.pdf split-d0.pdf \
  --family low --origin split --colored
python3 scripts/research/wine_type3_diagnostic.py baseline.pdf normalized.pdf \
  --family low --normalized axes
python3 scripts/research/wine_type3_diagnostic.py baseline.pdf paths.pdf \
  --family low --shape paths --origin split --normalized axes
python3 scripts/research/wine_type3_diagnostic.py baseline.pdf rectangles.pdf \
  --family low --shape rectangle --normalized point
```

Use `--family high` or `--reverse` for the corresponding generated fixture.
`--normalized square` reproduces the square nested-text treatment;
`--shape rectangle --normalized axes` reproduces the second positive control.
Omitting `--normalized` on paths reproduces the point-coordinate treatment.
The helper refuses an existing output and verifies text and original programs.
External fonts, source documents, PDFs and screenshots remain outside Git.

The production implementation `7f8b7e7` now has passing native quality, WASM,
MIT audit and coverage checks, plus all nine regular native platform jobs.
Seventeen optional/release jobs were skipped and are not counted as passes.
This research changes no converter code and claims no new actual-corpus or
runtime improvement. Keep #541 open and PR #542 draft: recoverable font/placement
fidelity and the remaining device-rounding/axis behavior still require work.

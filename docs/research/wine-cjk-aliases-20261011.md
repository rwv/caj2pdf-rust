# C8 CJK-mode alphabet aliases: Wine 7.3 observations

The converter now follows the measured CJK resource and glyph slot for all
62 raw `a0` digits and Latin letters while `80ce/0` is active. Previously it
queried an ASCII glyph in the base CJK font. For example, raw `a0c1` selects
U+7283 in the styled CJK font; its established document character remains
ASCII `A`. An explicit ActualText span preserves that distinction, and the
exact source alias does not increment the lossy-substitution count.

This is a scoped follow-up to [#541](https://github.com/rwv/caj2pdf-rust/issues/541).
The [metadata receipt](wine-cjk-aliases-20261011.json) retains 59 new Wine
sessions: 24 independent font-query sessions, 24 real-font visual sessions,
and 11 negative geometry/PDF controls. No corpus files, fonts or screenshots
are committed here. The reference remains the pinned Windows CAJViewer 7.3
Wine profile from the [font-resource study](wine-font-resources-20261011.md).

## Independent resource and slot observations

Separate low/high-byte geometric query fonts yield 760 observations and
371 distinct style/code/character-mode/family-override combinations. Every
forward/reverse pair agrees. Five profiles cover all 62 alphanumerics plus
fullwidth `A` and Han `中` sentinels:

| Raw style | Selected resource |
| --- | --- |
| `1084` | HGSS_CNKI |
| `0884` | HGHT_CNKI |
| `0c84` | HGKT_CNKI |
| `04c6` | SimSun |
| `0000`, explicit 34-by-34 axes | SimSun |

The mixed-family grid checks `801c/2`, `/3`, `/4`, the raw low-byte `0070`
exception and restoration through `80ce/1`. CJK-mode `a0` alphanumerics query
the ordinary two-byte GBK slot of their raw code in the selected resource.
Low-byte `0070` keeps HGSS's ordinary `p` slot; ordinary Latin mode restores
the separately measured Latin resource and alias. The system-font controls
replace only 64 authored marker glyphs in disposable SimSun/NSimSun copies,
preserving UI coverage; original and modified hashes are recorded.

These font queries establish painting behavior, not Wine clipboard semantics.
The converter retains its existing source ASCII decoding independently.
Missing optional style roles use the base CJK resource. Once a resource is
selected, a missing measured alias fails instead of drawing its ASCII glyph
or silently switching to another resource. No coordinate model changes.

## Real-font before/after verification

Each original control, old PDF, new PDF and fresh source repeat is opened in
a separate Wine session at 200%. Two stable captures, numeric page/zoom
readiness, complete page bounds, unchanged inputs, no OOM and container
cleanup are checked for every admitted session. All six cold source repeats
are pixel-identical. Strict differing-pixel counts use equal page extents;
there is no resizing or per-document fitting.

| Control | Before | After | Source repeat |
| --- | ---: | ---: | ---: |
| `1084` | 17,828 | 13,993 | 0 |
| `0884` | 22,005 | 17,526 | 0 |
| `0c84` | 16,217 | 12,132 | 0 |
| `04c6` | 27,211 | 9,078 | 0 |
| `0000` | 15,500 | 8,150 | 0 |
| Mixed families/modes | 14,603 | 9,008 | 0 |

All six improve by 20–67%, but none is pixel-exact. Qpdf and independent
ToUnicode/ActualText source-order checks pass for all twelve before/after
PDFs. Node produces identical bytes to native for all six corrected PDFs.
Original geometric fixtures additionally verify bounded reads, missing-alias
errors, semantic text, zero substitutions and browser/Node byte equality.

All 13 actual documents / 77 pages were reconverted: qpdf and independent
checks preserve all 113,418 characters, and every PDF is byte-identical to
the previous candidate. This inventory has no affected CJK-mode `a0`
combination. Its existing 35 extent differences and 42 equal-extent pixel
differences remain; unchanged PDFs reuse their earlier admitted observations.
Earlier Wine startup failures remain open and are not superseded by these
single-page controls.

## Rejected geometry hypotheses

Six sessions compare an original 54-square coordinate grid at 75% and 150%,
including fresh source repeats. Across long horizontal/vertical intervals,
the existing `240/2473` point unit stays within one device pixel of the
observed interval. Predeclared `0.096` and `0.1` alternatives fail this bound;
the latter reaches 19 horizontal pixels at 150%. Thresholds 128 and 192
agree. Source repeats are exact, while source/PDF differences remain 342
and 858 pixels. This rejects those alternatives; it does not establish an
internal rounding implementation or justify a fitted placement adjustment.

Five further sessions test equivalent PDF text factorizations: replace
`Tf=1, Tm=[a 0 c d x y]` with `Tf=d, Tm=[a/d 0 c/d 1 x y]`. Positions,
font/glyph identity and nominal size are retained. All seven observed pages
are pixel-identical to the baseline PDF: two grid views, three actual native
text pages and two image-only negative controls. Those first two image pages
do not count as typography evidence. All three unique PDFs pass qpdf and
preserve extracted text. No production geometry change follows these results.

## Reproduction and provenance

Run the original MIT generator into a new directory:

```sh
python3 scripts/research/wine_cjk_aliases.py /tmp/cjk-alias-controls
```

It uses only the Python standard library, reads no external input, and
reproduces all twelve CJK controls plus the coordinate grid byte-for-byte.
The receipt records every source hash. Query marker construction follows
the original [query generator](../../scripts/research/wine_font_query.py);
localized CJK names and the preserved-system-font controls retain the
resource-identification method from the prior study. The coordinate grid
uses the original full-em rectangle font from the
[ornament generator](../../scripts/research/wine_ornament_geometry.py).
All resource hashes, capture hashes, driver hashes, query results, negative
controls and source/output identities are retained in the receipt.

The framing derives from the project's own independently authored MIT
[C8 control generator](https://github.com/rwv/caj2pdf-samples/blob/d4769cb0e3883c136845e17399c14900054d51d3/research/cajviewer/c8_style_fixture.py),
reviewed per file. Production logic and rectangle/triangle test fonts are
original MIT work. No vendor/private HN/JBIG implementation, foreign
converter source, external glyph outline or corpus document was copied into
this repository. Font I/O remains ranged, output sequential, and runtime
font bounds and dependencies unchanged. #541 remains open.

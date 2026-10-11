# Wine mode-0 page padding

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
The [metadata receipt](wine-mode0-page-padding-20261011.json) separates a
confirmed page-boundary correction from unresolved glyph-position hypotheses.
HN-B mode-0 native pages add **10 PDF points** to each raw extent after applying
the independently established `72/742` point coordinate unit. Previously they
added 100 raw units, equivalent to about 9.7035 points. This correction changes
PDF page boxes and their corresponding bottom-origin coordinates in native,
CLI, Node and browser conversion, with either subset or full fonts.

## Independent boundary controls

The reference is CAJViewer 7.3.151 Simple under the same pinned Wine profile,
1600 × 1200 at 96 dpi. Three original dimension pairs are observed at 75%,
100% and 150%; a fourth pair at 100% was declared as held out before observation.
Each page has one CJK and one Latin geometric glyph. The source and PDF use
identical original geometric resource fonts. Padding candidates preserve the
raw coordinate unit, glyph programs, sizes and top-relative positions.

| Raw width × height | Zoom | Wine white-page width × height |
| --- | ---: | --- |
| 2500 × 2300 | 75% | 253 × 233 |
| 2500 × 2300 | 100% | 337 × 311 |
| 2500 × 2300 | 150% | 505 × 466 |
| 3100 × 3400 | 75% | 311 × 340 |
| 3100 × 3400 | 100% | 414 × 453 |
| 3100 × 3400 | 150% | 622 × 680 |
| 5105 × 4200 | 75% | 505 × 418 |
| 5105 × 4200 | 100% | 674 × 557 |
| 5105 × 4200 | 150% | 1011 × 835 |
| 4736 × 2800, held out | 100% | 626 × 376 |

White connected-component bounds are measured independently of the surrounding
frame, which adds four horizontal and five vertical pixels in these captures.
The previous formula matches both dimensions in 3/10 comparisons; the 10-point
formula matches **10/10**, fixing seven unequal-boundary comparisons. Strict
RGB differences use complete frames, without resizing, alignment or a fitted
tolerance. Unequal frames remain uncomparable. This is evidence for the
measured profile and dimensions, not every possible DPI, zoom or native Windows.

## Production verification

Thirteen fresh production-PDF Wine sessions cover the ten boundary comparisons,
all four actual mode-0 pages at 75%, and three cold repeated page views. Every
session passes numeric page/zoom checks, two stable captures, complete bounds,
source integrity and container cleanup without OOM. All ten production control
frames equal their prior 10-point diagnostic frames exactly; the three cold
repeat frames also match exactly.

The actual four-page PDF retains its previous 75% raster **byte-for-byte in
cropped RGB**. Source-versus-PDF differing pixels remain 81,934; 84,280; 89,658;
and 31,573. The two-glyph controls still differ by 122 pixels at 75%, 56 at
100%, and 207 at 150%. Matching page boundaries does not resolve these glyph
placement differences.

Fresh native conversions cover all thirteen inventoried native-text documents
in both full-original and default-subset configurations. Twelve mode-2 PDFs
in each configuration retain their prior bytes; the mode-0 PDF changes as
expected. Node produces identical bytes for all **26** current native outputs
using bounded 64 KiB writes. All 26 native PDFs pass qpdf. This is fresh
conversion evidence; the 73 unchanged mode-2 Wine page views are reused from
the [preceding comparison](wine-original-font-production-20261011.md).

Independent PDF inspection checks both versions of all four mode-0 pages:
only page extents and corresponding vertical coordinates change. Font programs,
CID mappings, widths, semantic mappings and other operators remain unchanged.
Extracted text agrees, and all 7,929 original source glyphs retain their semantic
order. The padding change adds no allocation, resource transport or API option.
The original font programs remain unmodified.

## Retained unsuccessful hypotheses

The complete experiment has 61 admitted Wine sessions and 68 page visits,
including the production sessions. The first thirteen sessions tested uniform
vertical text translations of 0/5/10/15 raw units across seven styles, with
reversed order and shifted positions held out at 100%. At 75%, increasing the
translation reduces aggregate differences but misplaces CJK glyphs while
improving Latin glyphs. The initial 150% source/PDF frames have unequal extents
and are not scored. No uniform glyph translation was implemented.

Four more source sessions translate the page origin and every glyph position
together to origins `(0, 0)` and `(0, 1)`, at 75% and 150%. All four source
frames equal the original-origin controls exactly; both converter PDFs are
also byte-identical. Absolute-origin rounding therefore does not explain the
observed differences in these controls.

An initial PDF preservation assertion compared exact decimal operands after
pikepdf serialization and failed at roughly 1e-14. An equally reserialized
zero-shift control separates that serialization effect from the intended
translation. All other matrix operands, glyph programs and semantic mappings
match that control; translation error is below 1e-9 point. A preparation attempt
also failed because the frozen CLI copy lacked its executable bit; no Wine
session began from it. These failures remain recorded.

The multi-style origin controls use original geometric resource programs.
Their optional system-font probe retains external metadata and unused glyphs
while replacing the painted control glyphs with original geometric shapes;
the entire system font is not claimed as project-authored. The simpler extent
controls use only the original geometric CJK/Latin resources. No external font,
corpus document, PDF or screenshot is committed here.

## Reproduction

The original MIT generator reproduces all eight measured source documents
byte-for-byte without external inputs:

```sh
python3 scripts/research/wine_mode0_page_extents.py /tmp/new-mode0-controls
```

The receipt identifies generated document/font hashes, candidates, executable
and driver hashes, admitted captures, failed hypotheses and runtime checks.
Use the same pinned viewer and resource profile to reproduce raster evidence;
the generator alone does not provide CAJViewer or its fonts. Page geometry
regressions assert the independent white-page dimensions above. Remaining
recoverable font/placement differences keep #541 open.

# RGB decoding and image-boundary counterexamples

Related: [#543](https://github.com/rwv/caj2pdf-rust/issues/543),
[#541](https://github.com/rwv/caj2pdf-rust/issues/541), and parent
[#406](https://github.com/rwv/caj2pdf-rust/issues/406).
The [receipt](wine-image-rgb-edge-decode-20261011.json) records every session,
comparison, original control, preserved-content identity and reproduction.

## Result

Full-resolution RGB decoding and explicit interpolation do not remove the
actual-document clipping counterexample. All **24 treatment-to-baseline
comparisons are pixel-exact when the clipping state is held constant**.
Four deliberately inverted-image views change, so unchanged treatment frames
are distinguishable from a failed image update. No decoder or clipping change
is adopted in production.

New independently generated RGB controls also regress under clipping. Type-1
images at 95% introduce two mismatches per page, reproduced after reversing
paint order. Combined with the earlier type-2 actual-document counterexample,
this rules out either matrix sign alone as a safe clipping selector. Other
new views improve or remain unchanged; an aggregate reduction is insufficient.

All **62 cold Wine sessions / 86 page visits** pass both viewport and independent
paper admission, stable captures and numeric page/zoom confirmation. Six cold
repeat visits are exact. There are no failed Wine sessions or rejected captures
in this batch. The retained preparation refusal happened before observation.
The production implementation remains `7f8b7e7`; all 77 previously inventoried
production pages still differ from Wine. This is scoped diagnostic evidence,
not a new full-cohort or runtime compatibility pass.

## Actual RGB document

The source SHA-256 is
`d6a23a9cfd0f37a71e245a3c96b2c3fafdf648f4bb1694f094cfe746b3f4f13d`.
Its five-page production PDF has SHA-256
`10b15f876e53f150858fdbc2ab773a0b3f49d83da6ec64ce027b758717e2c6ad`.
The [preceding cohort study](wine-image-clip-cohort-20261011.md) identified
opposite clipping responses on pages 2 and 3 and at different zooms.

Unlike the [earlier gray decode study](wine-jpeg-decode-controls-20261011.md),
this PDF has four DeviceRGB JPEGs with explicit `/ColorTransform 1`. The two
page-2 images are 1213 × 917 and 947 × 838; page 3 has 1102 × 786 and page 4
has 1121 × 908. Pages 2 and 3 are observed at 75% and 95%. All four image
resources are transformed and checked, including the unobserved page-4 image.

The [original MIT decode helper](../../scripts/research/wine_jpeg_decode_diagnostic.py)
now accepts RGB only with `--allow-rgb`. It permits absent JPEG parameters or
exactly `/ColorTransform 1`, rejects masks and decode remaps, checks mode and
dimensions, and keeps the ten-million-pixel bound per decoded image. Flate
stores Pillow's decoded RGB samples and removes DCT-specific parameters.
This selects a measured decoder path; it does not assert codec conformance.

Four representations are compared: retained JPEG, decoded Flate, retained JPEG
with `/Interpolate true`, and decoded Flate with that flag. Each also receives
the unchanged [unit-clip helper](../../scripts/research/wine_image_clip_diagnostic.py).
An inverted Flate image is observed with and without clipping at 75% as a
positive control. Source coordinates, page bounds, paint order, text operators,
eleven original embedded font programs and all 10,277 source-order semantic
characters remain unchanged in all ten PDFs. Image payloads intentionally
change only in decoded/inverted variants; clipping preserves each input payload.

Fresh native and original-PDF sessions establish all four reference views.
They equal the admitted historical frames exactly. Reserializing the retained
JPEG baseline also changes no frame. The source at 95% and original PDF at 75%
each receive a second cold session; all four repeated visits are exact.

Each row below applies to **all four representations**. Counts are strict,
unaligned whole-page RGB pixel differences from the native source.

| Page | Zoom | Unclipped | Clipped | Repaired | Introduced |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 75% | 72,476 | 72,310 | 166 | 0 |
| 3 | 75% | 62,621 | 62,805 | 0 | 184 |
| 2 | 95% | 86,142 | 85,679 | 463 | 0 |
| 3 | 95% | 57,452 | 57,219 | 233 | 0 |

Across 32 non-inverted actual comparisons, twelve improve, sixteen retain
baseline pixels and four regress. All remain nonzero. The four inverted views
are kept separately from fidelity candidates in the receipt summary.
These equalities do not prove native/PDF decoder equivalence, universal
interpolation-flag behavior or a particular native sampling implementation.

## Independent edge controls

The [original MIT generator](../../scripts/research/wine_image_edge_controls.py)
creates four C8 documents with 36 disjoint RGB panels each. Every page crosses
three source resolutions (33 × 25, 129 × 97 and 513 × 385), six declared
raw top-origin phase offsets (0, 1, 4, 7, 10, 11), and reversed black/white
first/last rows. Interior RGB gradients and stripes are generated directly.
JPEG quality is 100 with chroma subsampling disabled; decoded boundary row
hashes and extrema are recorded rather than assumed lossless.

Descriptor kinds 1 and 2 vary matrix direction. Each also has a file with the
same physical panel positions and payloads in reversed paint order. Every
source, baseline PDF and clipped PDF is observed at 75%, 95% and 150%.
Type-1/95% and type-2/reversed-order/150% native sessions receive cold repeats.

The table applies to both paint orders, whose entire source, baseline and
clipped frames match in all **18 order comparisons**.

| Descriptor | Zoom | Baseline | Clipped |
| ---: | ---: | ---: | ---: |
| 1 | 75% | 32,825 | 32,825 |
| 1 | 95% | 36,894 | 36,896 |
| 1 | 150% | 117,538 | 117,538 |
| 2 | 75% | 34,804 | 33,904 |
| 2 | 95% | 46,206 | 46,206 |
| 2 | 150% | 114,260 | 112,820 |

The twelve comparisons yield four improvements, six unchanged frames and two
regressions: 765,054 → 760,378 differing pixels. All still differ. The two
type-1/95% regressions each involve the 33 × 25 panels with phase offsets 1
and 7, black first row and white last row. Each affected panel introduces one
mismatch without repairing another. The regression therefore reproduces on
original controls with small images and either paint order.

The receipt partitions all twelve pages into **432 declared panel windows**;
94 change. They account for all 4,680 repaired and four introduced pixels.
No baseline-to-clipped change falls outside the declared windows. Windows only
localize the already measured full-page result: no image registration, phase
fitting or candidate-specific coordinate shift is used.

## Reproduction and limits

Both research helpers operate on external inputs/outputs. Example commands:

```sh
python3 scripts/research/wine_jpeg_decode_diagnostic.py input.pdf new-decode-dir --allow-rgb
python3 scripts/research/wine_image_clip_diagnostic.py new-decode-dir/flate.pdf new-flate-clip.pdf --receipt clip.json
python3 scripts/research/wine_image_edge_controls.py new-controls-dir
```

All ten actual PDFs, four original C8 controls, four baseline control PDFs and
four clipped control PDFs reproduce byte-for-byte with exact metadata. All
five legacy gray decode PDFs and metadata also reproduce exactly. RGB admission
checks pass two positive cases and ten expected refusals. The first preparation
attempt refused the actual source's explicit ColorTransform dictionary before
writing any PDF; that log is retained. Supporting only the known RGB dictionary
resolved preparation, without weakening Wine admission.

Every session verifies unchanged source/copy hashes, no observed OOM kill and
container removal. The existing known information-bar dismissal is used for
native actual-document views. Two captures per visit must be identical, and
numeric page/zoom fields must agree before and after them. All final references
pass both viewport and independent-paper checks. There are no replacements,
hidden incomplete captures or relaxed comparisons in this batch.

The preceding `a05e38e` research head completed 15 CI checks successfully;
17 optional/release skips are excluded. This study changes only original MIT
research helpers and evidence. Production code, APIs, dependencies and bounded
I/O remain unchanged, so runtime suites are not claimed as freshly rerun.
Current-head CI and final merge review remain separate obligations.

#543 remains open: a geometry-preserving correction across independent zooms,
origin phases, resolutions, signs and actual documents is still absent.
This study separates the tested RGB decoder/interpolation choices from the
clipping response and supplies reproducible counterexamples. It does not
satisfy the no-regression production acceptance criterion or close #541/#406.

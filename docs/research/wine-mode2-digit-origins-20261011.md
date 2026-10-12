# Mode-2 digit origins and alphabet controls

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
Mode-2 raw codes A0B0–A0B9 incorrectly used the ordinary alphabet's horizontal
and vertical offsets. The correction uses the recorded digit origin while
preserving the selected Latin resource, glyph aliases, semantic ASCII text,
font size and all other page content. It improves all 316 measured digit cells
and three actual pages, with no actual-page regression. Broader alphabet
placement and raster fidelity remain unresolved.

The [metadata receipt](wine-mode2-digit-origins-20261011.json) records 59 fresh
Wine sessions: 58 admitted sessions / 78 admitted visits, and one retained
rejected source-repeat visit. Five cold source-repeat views and two PDF-repeat
views match exactly. The reference remains the pinned CAJViewer 7.3.151 Simple
Wine profile at 1600×1200 / 96 dpi.

## Independent placement rule

The [preceding study](wine-axis-rasterization-20261011.md) showed growing
character-dependent errors even when `A/M/1/e` shared the same geometric glyph.
Fourteen further original HN-B controls cover all 62 ASCII alphanumeric
characters, both ordinary and alternate Latin resources, 35×35, 42×35 and
43×43 axes. Baselines use 600%; two reversed pages shifted by +11/+17 are
held out at 450%. Each glyph resets the relevant state; fixed cells contain
all ink. CJK anchors and exact cold source repeats constrain the comparison.

The ten digits consistently share a horizontal origin and a top aligned with
the recorded y coordinate. Unlike letters, their origin does not require a
character-specific or font-size-dependent alphabet offset. With `u = 72/742`
points per raw unit, page bounds `left,top`, raw position `x,y`, source origin
`ox,oy`, and the already-established font em height `h`:

```text
digit text-matrix x = left + (x - ox + 20) * u
digit text-matrix y = top - (y - oy) * u - h
```

The old alphabet rule added `width/8` horizontally and used a height-dependent
baseline offset relative to `y-15`. For example, at explicit 43×43 and 600%,
that put the identical geometric `1` ten pixels right and seven pixels up
from Wine in the preceding control. The digit correction removes these physical
offsets; it does not attempt to eliminate separate device-pixel rounding.

`native_digit_transform` first validates size/style through the existing Latin
path. The page writer selects it only for A0B0–A0B9 in ordinary Latin placement.
CJK character mode retains its CJK mapping/placement; mode 0, fullwidth A3 digits,
letters and symbols retain their separate rules. CJK-only axes/styles do not
become newly admitted digit profiles. Resource states 0, 3, 4, 28 and 31 keep
existing font selection. No glyph data, alias map or font program is modified.

The alphabet controls also confirm that one class-wide `width/8` rule remains
insufficient for letters. Ordinary/alternate geometric resources have matching
position patterns, but differences vary by letter and axis profile. These data
are retained for further independent controls; no four-letter offset table or
universal alphabet correction is inferred here.

## Predicted PDF and production controls

Twelve diagnostic PDF views change only the digit origins according to the
formula, before using the production implementation. Seven use the geometric
alphabet controls; five use unchanged CAJViewer programs with independent C8
styles, explicit 28/43 axes and zooms from 75% to 1100%. Qpdf and independent
source-order semantic checks pass. All 96 digit cells improve, reducing their
combined strict difference count from 228,323 to 18,596. Every non-digit cell
remains pixel-exact to its corresponding baseline.

Three further source documents exercise the production implementation in both
C8 and HN-B, implicit size 35 and explicit size 36. C8 covers all five admitted
Latin resource states; HN-B independently covers state 3 as well. All eight
resources use original geometric programs. The added three font families are
renamed copies of the project-authored geometric Latin control, with identical
outlines and metrics. A reversed/shifted C8 page at 250% is held out from the
300% baseline.

| Production control | Before, differing pixels | After |
| --- | ---: | ---: |
| C8, five states, 300% | 74,105 | 16,255 |
| Reversed/shifted C8, 250% | 52,829 | 10,339 |
| HN-B, state 3, 300% | 14,564 | 2,929 |

All 220 digit cells improve, from 127,735 to 15,760 differing pixels in total.
Non-digit cells remain unchanged. Together with the predicted diagnostics,
this is 316 improved digit cells and zero regressions. Complete-frame and
cell scores use strict RGB with fixed bounds, without alignment or resizing;
tight glyph comparisons are separately labeled shape diagnostics.

## Actual documents

All 13 inventoried documents are freshly converted with the corrected native
implementation and unchanged original-font recipes, including the corrected
mode-0 digit aliases. Ten PDFs remain byte-identical. Only 64 digit matrices
change across the other three PDFs; independent checks preserve font programs,
page boxes, text order and every other content operator. All 77 pages pass qpdf
and semantic checks, preserving 113,418 source-order characters.

All 22 pages of the three changed PDFs have fresh Wine observations. The other
55 byte-identical pages reuse their established observations and are explicitly
not counted as fresh GUI tests. Only the three pages containing the changed
digits have different pixels:

| Source SHA prefix / page | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| `166d00147923`, page 1 | 12,831 | 12,335 | 496 |
| `3f3b9b57d692`, page 1 | 13,922 | 13,595 | 327 |
| `e1b17805a87f`, page 1 | 31,220 | 30,707 | 513 |

The reduction is 1,336 pixels. Three pages improve, 74 retain their previous
score and pixels, and none regress. Extents still match on all 77 pages.
Every actual page still differs from Wine; this is a specific placement fix,
not complete rendering parity or an expanded corpus claim.

## Validation, failures and reproduction

Native CLI and Node produce byte-identical output for all 13 actual documents
and all three font-state controls. Real Chromium independently matches the CLI
for the three changed actual documents and all three controls. All 22 runtime
checks keep output chunks at or below 65,536 bytes. Initial parity setup omitted
the existing per-value ornament mappings; an unaffected actual document exposed
the mismatch. Correcting the harness to use identical options resolves it.

The first browser/JavaScript attempts could not bind their loopback test server
because of sandbox `EPERM`. Explicitly permitted local runs pass all 192
JavaScript tests with no skips. The native suite and all-feature coverage run
each pass 1,447 tests, with seven ignored optional-corpus tests excluded from
compatibility evidence. Coverage is 98.43% (32,573/33,092; required floor 90%).
Locked native/WASM release builds, all-feature Clippy, Rustdoc and WASM checks,
formatting, source inventory and documentation links pass.

One numerically ready, stable source-repeat capture contained the viewer's
information bar, changing the viewport and page centering. Complete-page
admission rejected it. Its files remain retained. A fresh isolated session
closes that observed bar through the GUI before capture; the resulting page
matches the established source exactly. The failed view is not counted as a
pass and no screenshot is cropped or aligned to conceal the bar.

The original MIT generator reproduces all 17 source documents and fixed-cell
manifests without external inputs:

```sh
python3 scripts/research/wine_digit_origins.py /tmp/new-digit-controls
```

Optional `--geometric-fonts /external/axis-study-base` uses fontTools and accepts
only the five checksum-pinned, original geometric baseline programs from the
preceding study. It reproduces the eight font-state controls byte-for-byte.
All 25 artifacts match the measured inputs exactly. Fonts, source documents,
PDFs and screenshots remain external. No vendor implementation or differently
licensed converter source was inspected or migrated.

No public API, new dependency, allocation or I/O strategy changes. Both subset
and full-program conversion use the same corrected placement path. Existing
original-font configuration remains required for the measured glyph identities.
Keep #541 open and PR #542 draft for unresolved alphabet origins and rasterization.

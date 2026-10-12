# Mode-2 alphabet origins measured in Wine

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
Raw A0 letters require character-dependent integer source offsets. Replacing
the former class-wide `width/8` compensation improves 37 of the 77 inventoried
actual pages, preserves the other 40 page rasters, and regresses none. The
reduction is 178,865 strict differing pixels from the preceding digit-origin
implementation. Every actual page still differs from Wine.

The [metadata receipt](wine-mode2-alphabet-origins-20261011.json) retains
61 fresh Wine sessions, including two sessions that fail full admission:
2,019 visits were planned, 2,013 recorded and 2,011 admitted. Two information-bar
captures were rejected; six planned startup-failure visits were unobserved.
Forty cold source-repeat views and two PDF-repeat views match exactly.
The reference is the pinned CAJViewer 7.3.151 Simple Wine profile at
1600×1200 / 96 dpi. Complete-page comparisons use strict RGB at fixed bounds,
without alignment, resizing or tolerance.

## Independent source-coordinate rule

The [digit study](wine-mode2-digit-origins-20261011.md) already separated digits
from letters. Four new HN-B controls put each of 62 ASCII alphanumeric characters
and two CJK anchors on its own page. Every geometric glyph has the same full-em
square outline and advance. At 5000% or 4000%, 35×35, 42×35 and 43×43 profiles
resolve unique integer horizontal offsets for all letters. A shifted, reversed
35×35 document with the alternate Latin resource independently preserves them.
The character dependence therefore does not come from the supplied outlines
or advances.

Two further 64-page C8 controls cover six sizes (22, 28, 34, 36, 38 and 40) and
three additional Latin resources (states 3, 28 and 31) at 35×35, 42×35 and 42×42.
All 576 state-control glyphs agree with the HN-B horizontal offsets. An initial
per-character proportional model, chosen as the midpoint of the 35/42/43
measurement intervals, misses 35 of 312 additional-size predictions. Those
failures are retained. The combined intervals instead admit unique integer
factors on a `5/128` grid. The six extra sizes helped select this rule; they
are corroborating evidence, not held-out validation of that selection.

With nominal integer width `w`, height `h`, and character factor `f`:

```text
x_offset = 20 + floor((w * f * 5 + 64) / 128)
y_offset = floor((h * 30 + 64) / 128)

Tm.x = left + (raw_x - origin_x + x_offset) * 72/742
Tm.y = top - (raw_y - origin_y - y_offset) * 72/742 - em_height
```

The integer divisions implement positive half-up rounding. The existing em
scale remains `nominal_size * 75/301` points. Factors are:

| Uppercase | Factor | Lowercase | Factor |
| --- | ---: | --- | ---: |
| `ABCDFGHKNOQRT` | 8 | `ace` | 15 |
| `EL` | 10 | `bdghkpqxyz` | 13 |
| `IJ` | 17 | `fs` | 17 |
| `M` | 5 | `ijl` | 20 |
| `PZ` | 9 | `m` | 4 |
| `S` | 11 | `nou` | 14 |
| `UV` | 6 | `rt` | 18 |
| `W` | 1 | `v` | 12 |
| `XY` | 7 | `w` | 7 |

The formula was frozen before three new 64-page C8 controls added sizes
4, 21, 24, 31, 48, 56, 63, 72 and 84, plus unequal 84×35, 35×84 and 28×72 axes.
They include alternate resources and reversed order. All 624 held-out letter
origins match the source-pixel prediction. Across all nine source documents,
all 1,612 geometric alphabet observations match it.

The source ink bounds also expose a separate device-rounding step. For these
full-em squares, `v = 96/742 * zoom/100` predicts:

```text
ink_left = floor((relative_x + x_offset) * v)
ink_top  = floor(relative_y * v) + 1 - floor(y_offset * v)
```

The source rounds the vertical position and displacement separately. Matching
these geometric origins does not imply that a fixed PDF matrix reproduces every
glyph bitmap or zoom. Device rounding and unequal-axis rasterization remain open.

## Diagnostic and production comparisons

The earlier x-only midpoint diagnostic improves many cells but regresses
26 geometric letter cells and two original-font cells. It is retained as a
failed general correction. The frozen x/y formula improves every one of the
624 held-out geometric letter cells; full-page differences fall from 6,327,082
to 318,622. Three additional views using unchanged original CAJViewer programs
fall from 3,885 to 2,987 differing pixels. Of their 54 letter cells, 31 improve,
21 are unchanged and two regress slightly at low zoom. Whole-frame improvement
does not conceal those two cell regressions.

Production `native_letter_transform` validates the existing Latin style/axis
path before replacing only the x/y offsets. It applies only to raw A0C1–A0DA
and A0E1–A0FA outside CJK character mode. CJK selection, fullwidth A3 letters,
digits, mode 0, font roles, glyph aliases, em sizes and semantic text retain
their separate behavior. Previously refused styles/axes remain refused.
The implementation adds constant tables and scalar arithmetic, with no API,
dependency, allocation or I/O-strategy change. Subset and full-program output
use the same placement path.

All nine controls are freshly converted by the production CLI. Independent
checks cover all 576 PDF pages: only 1,612 letter matrices change; font programs,
glyph identities, page bounds, source-order text and other operators remain
unchanged. Fresh Wine production views cover 186 selected pages:

| Production view group | Pages | Letter cells | Before pixels | After pixels |
| --- | ---: | ---: | ---: | ---: |
| High-zoom profiles, representative characters | 28 | 16 | 905,296 | 28,300 |
| Complete C8 size/state controls | 128 | 780 | 6,874,332 | 292,058 |
| Held-out size/axis controls, representative characters | 30 | 84 | 1,016,642 | 48,091 |

All 880 selected production letter cells improve. The table reports complete
page scores, including unchanged digits and CJK anchors. Unselected production
pages have semantic/operator validation, not new GUI observations.

## Actual documents and runtime parity

All 13 documents are freshly converted with unchanged original-font recipes.
Five PDFs (28 pages) remain byte-identical. The other eight PDFs change exactly
15,337 letter matrices; font programs, glyph/text identities, widths, page boxes
and every other content operator remain unchanged. All 77 pages pass qpdf and
preserve 113,418 source-order semantic characters.

All 49 pages of changed PDFs have fresh Wine observations. The 28 byte-identical
pages explicitly reuse established observations. At the pinned 75% zoom:

| Result | Pages |
| --- | ---: |
| Fewer differing pixels | 37 |
| Identical prior page raster | 40 |
| More differing pixels | 0 |

Total strict differences fall from 3,052,903 to 2,874,038. Page extents match on
all 77 pages, but every page still has pixel differences. The receipt publishes
page-level metrics and semantic hashes, not external article text or per-glyph
content traces. This is the existing cohort, not an expanded-corpus claim.

CLI, Node and real Chromium produce byte-identical PDFs for all 13 actual
documents and all nine controls: 22 Node and 22 Chromium comparisons. Every
output chunk is at most 65,536 bytes. These checks supply identical resource,
alias, symbol and decoration options across runtimes.

## Validation, failures and reproduction

The native suite and all-feature coverage each pass 1,449 tests. Seven ignored
optional-corpus tests are excluded from compatibility evidence. Coverage is
98.43% (32,616/33,135), above the actual 90% floor. The complete serial JavaScript
run passes all 192 tests, including real Chromium, with no skips. Locked native
and WASM release builds, all-feature WASM checks, Clippy and Rustdoc pass.

Retained failures distinguish preparation from viewer observations:

- Initial HN-B explicit size 22, C8 explicit size 28 and CJK-only style 114a
  were refused before Wine. Final controls use admitted C8 profiles, implicit
  28 and style 094a for 84-square Latin. No parser/renderer admission was relaxed.
- Initial x-only preparation collided on a filename reused at two zooms; unique
  zoom-qualified paths fixed preparation before any Wine observation.
- The first `90e7b47716c3` actual PDF session lost the viewer during zoom setup.
  Its crash/black-frame evidence is retained; no page was admitted and no OOM
  was observed. A fresh established bootstrap GUI route admitted all six pages.
- The first `e1b17805a87f` source repeat was stable and numerically ready but had
  an information bar. Both pages were rejected. A fresh session closing the
  observed bar through the GUI produced exact repeats, without crop relaxation.
- Two older placement assertions required the newly measured offsets. An initial
  new test incorrectly requested unsupported HN-B CJK character mode; the final
  test checks that behavior in C8, without extending admission.
- WASM compilation succeeded, but the default copy helper could not spawn cargo
  under sandbox `EPERM`. Explicit-source copying staged that successful build.
- The first concurrent JavaScript run passed 191 tests and failed an existing
  300 ms fake-hang timeout fixture under load. The full serial rerun passes 192;
  no JavaScript implementation or test was changed.

The original MIT generator reproduces nine source documents and their glyph
manifests without external inputs:

```sh
python3 scripts/research/wine_alphabet_origins.py /tmp/new-alphabet-controls
```

Optional `--geometric-fonts /external/axis-study-base` reuses the preceding
checksum-pinned, project-authored geometric programs through the existing
font-state generator. All nine sources and eight geometric fonts reproduce
byte-for-byte. No vendor font is edited. Fonts, PDFs, screenshots and corpus
documents stay external; no vendor implementation, foreign converter or private
HN/JBIG source was inspected or migrated.

Keep #541 open and PR #542 draft for remaining recoverable rendering differences.
Native Windows equivalence has not been tested. CI for the committed change is
reported separately; prior-head results do not substitute for it.

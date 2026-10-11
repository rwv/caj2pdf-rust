# Wine axis rasterization and character origins

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
This follow-up separates two remaining differences: unequal-axis glyph rasterization
and character-dependent Latin origins. The [metadata receipt](wine-axis-rasterization-20261011.json)
records 73 fresh Wine sessions / 73 stable, complete page visits, including four
exact cold source repeats. These are synthetic diagnostics, not additional
actual-corpus compatibility tests. Production implementation and original
resource programs remain unchanged.

## Independent axis quantization

Four original HN-B mode-2 documents isolate glyphs at equal and unequal axis
sizes. The low-zoom grid has seven profiles and six characters (`中文AM1e`):
35×35, 42×35, 42×42, 28×28, 43×28, 28×43 and 43×43. It uses 75% and 150%;
a reversed, shifted page is held out at 100%. The high-zoom grid has the four
explicit profiles and four characters (`中AM1`), observed at 300%, 600%, 900%
and reversed/shifted 1100%. Each glyph resets the relevant native state.
Source and PDF use identical original resource programs.

All equal-axis glyph bitmaps match after independently removing their translation
for diagnosis. All unequal-axis bitmaps differ. Complete-page fidelity retains
unaltered bounds and exact RGB comparison, without registration, scaling or a
tolerance. Earlier corrected mode-0 isolated glyph controls supply three more
reference frames at 75%, 150% and 100%.

For PDF text-matrix dimensions `a,d` in points, zoom percentage `Z` and 96 dpi,
a predicted diagnostic keeps height and origin unchanged and changes only width:

```text
scale = 96 * Z / 7200
X = floor(a * scale)
Y = floor(d * scale)
a' = d * X / Y
```

This tests independently quantized horizontal/vertical em sizes against a
height-first scaled bitmap. It changes unequal-axis matrices only. Embedded
font bytes, glyph identities, text and all other content remain unchanged.
All diagnostic PDFs pass qpdf and source-order semantic checks.

| Frame | Baseline differing pixels | Predicted width | Exact tight glyph bitmaps |
| --- | ---: | ---: | ---: |
| Low 75% | 799 | 534 | 36/42 |
| Low 150% | 2,641 | 2,152 | 42/42 |
| Reversed low 100% | 1,781 | 1,584 | 42/42 |
| High 300% | 3,438 | 3,225 | 16/16 |
| High 600% | 11,139 | 10,700 | 16/16 |
| High 900% | 23,741 | 23,203 | 16/16 |
| Reversed high 1100% | 30,014 | 28,704 | 16/16 |
| Mode 0, 75% | 1,091 | 1,083 | 24/36 |
| Mode 0, 150% | 3,002 | 2,611 | 36/36 |
| Reversed mode 0, 100% | 1,672 | 1,544 | 36/36 |

Across 298 glyph observations, exact tight bitmaps increase from 176 to 280.
The 18 remaining differences are all at 75%, in wide 42×35 profiles. Adding a
fixed positive `1/1024` to `X` repairs those 18, but introduces 15 differences
at other zooms/profiles. That guard is retained as a numerical-boundary probe,
not chosen as a general correction.

Two cross-zoom observations reject a fixed-width solution: the guarded PDF
prepared for 75%, observed at 150%, has 2,610 differing pixels and only 24/42
exact glyph shapes. The 150%-prepared PDF at 75% has 786 differing pixels and
24/42 exact shapes. At their intended zooms the respective values were 480
with 42/42 shapes and 2,153 with 40/42. Neither PDF fixes all zooms.

A separate equivalent PDF encoding moves vertical size into `Tf` and the
horizontal ratio into `Tz`, retaining a unit text matrix and the same origin.
The matrix-equivalence error is below 1e-25 points. All five observed complete
frames are byte-exact to the baseline RGB frames, so this encoding change does
not select the desired raster behavior. This evidence does not identify the
viewer content API or establish a deployable vector-PDF solution.

## Character-dependent origins

Position differences also grow with zoom, even when original-font shapes match.
At explicit 43×43 and 1100%, PDF-minus-source offsets are `(1,1)` for `A`,
`(8,1)` for `M` and `(20,-13)` for `1`. These cannot all be explained by one
pixel of device rounding.

Two original geometric grids test the cause using a single square glyph for
ordinary `A/M/1/e`, including both their semantic and private cmap entries.
CJK and alternate Latin are unchanged reference cells. Five font profiles keep
all ordinary-Latin global enclosing bounds at `[0,0,1200,1200]` in a 1000-unit em:

- Baseline: 1000-unit square, 1000-unit advance.
- Double advance: change advance to 2000, with consistent hhea maximum and OS/2 average.
- Half width: halve outline x coordinates, retaining the advance.
- Shift x: move the outline and consistent left bearing by +200.
- Shift y: move only outline y coordinates by +200.

Doubling the advance changes **zero pixels** in all four observed source frames
across the two grids. Halving the outline preserves its observed left/top
anchor; moving it changes the corresponding painted coordinates. All 48 CJK
and alternate-Latin reference-cell comparisons in the expanded grid remain
byte-exact. The fact that these font interventions do not remove the code-dependent
origin differences rules out correcting them solely with the supplied glyph's
advance or outline bounds. It does not prove an internal lookup-table or cache
implementation.

The expanded grid leaves enough room for every intervention. At explicit
43×43, its baseline source/PDF differences are:

| Character | 150% offset (x,y), pixels | 600% offset | Reversed/shifted 450% offset |
| --- | ---: | ---: | ---: |
| Ordinary `A` | (0,0) | (0,0) | (0,0) |
| `M` | (1,0) | (4,0) | (3,0) |
| `1` | (3,-1) | (10,-7) | (8,-5) |
| `e` | (-2,0) | (-9,0) | (-7,0) |
| CJK / alternate `A` | (0,0) | (0,0) | (0,0) |

Every listed bitmap has the same shape as its PDF counterpart. The complete
expanded frames still differ by 1,698, 22,252 and 12,079 pixels respectively.
No character-specific production correction is inferred from these four codes.
Broader independent code, position, style and state controls are still required.

## Failed diagnostics and admission

The first axis fixture used a C8 container with explicit 28/43 axes; conversion
refused it at byte 568. The retained experiment uses the admitted HN-B mode-2
container, without changing parser admission.

The first geometric grid's small 150% page is shorter than the old extractor's
200-pixel component cutoff. Its screenshots are complete and stable. For both
geometric grids, the extractor retains the same largest background-separated
component, verifies both audit bounds (including the detached left border),
checks that the component occupies over 98% of its bounds, rejects other content,
and checks bounds against declared page size plus fewer than eight border pixels.
For larger pages its crop is identical to the original extractor. No alignment
or resizing is introduced.

Six first-grid frames have ink outside their predeclared glyph cells. Those
whole frames remain admitted, but their truncated cell comparisons are rejected
and retained separately. A new grid with wider spacing and cells was declared
before its fresh sessions. All 14 non-repeat expanded frames contain every glyph
inside the declared cells; its cold source repeat is exact. Earlier truncated
cells are not rehabilitated by moving their boundaries.

Local FreeType sizing/transform probes did not exactly reproduce Wine and are
not evidence of the viewer's renderer implementation. The inspected resource
programs have no EBDT/EBLC tables; an embedded-bitmap explanation is unsupported.
No vendor implementation or foreign converter source was inspected.

## Reproduction and remaining work

Generate all eight original source controls with the standard library:

```sh
python3 scripts/research/wine_axis_rasters.py /tmp/new-axis-controls
```

The optional `--geometric-fonts /external/original-controls` accepts only the
five checksum-pinned, project-authored geometric fonts and needs fontTools.
It reproduces 25 font variants. All 33 artifacts and eight geometry manifests
match the observed inputs exactly; it never accepts vendor programs for mutation.

The separate pikepdf diagnostic reproduces all 24 prepared PDF content streams
and their unchanged embedded programs:

```sh
python3 scripts/research/wine_axis_pdf_diagnostic.py baseline.pdf predicted.pdf \
  --kind quantize --zoom 150
```

Use `--kind guard` for the fixed numerical guard or `--kind tz` for the equivalent
encoding control. Quantization is explicitly zoom-specific. Fonts, source
documents, PDFs and screenshots remain external; hashes and per-cell observations
are in the receipt. No native/WASM implementation changed, and no new runtime
parity or actual-corpus rendering improvement is claimed. Keep #541 open and
PR #542 draft: character-origin behavior and a general unequal-axis rendering
solution remain unresolved.

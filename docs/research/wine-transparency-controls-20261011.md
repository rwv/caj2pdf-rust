# Wine forms and isolated-transparency glyph controls

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
The [receipt](wine-transparency-controls-20261011.json) records 38 independent
cold Wine sessions, original-font identities, exact comparisons and reproduction.
This follows the [axis](wine-axis-rasterization-20261011.md) and
[Type 3](wine-type3-rasterization-20261011.md) experiments.

## Result

All **35 full-opacity candidate frames equal their PDF baselines exactly**.
Ordinary forms, isolated groups, splitting the glyph origin and moving unequal
axes into the outer transform do not repair the measured Wine differences.
Every source/PDF frame still differs. No production change is adopted.

Two cold source repeats are exact. A separate half-opacity control changes the
frame but leaves 31 of 42 glyph cells without visible ink; it is neither a
fidelity candidate nor a transparency-conformance pass. All 38 visits are
complete, stable and numeric-confirmed, with intact inputs and removed containers.

The independent MuPDF render changes under isolation while Wine's full-opacity
frames do not. This is a concrete reason to keep Wine as the requested visual
reference; a different renderer's response is not evidence of source fidelity.

## Hypothesis and controls

An isolated PDF group composites its contents with a transparent initial
backdrop, then combines that result with the enclosing content. This definition
does not prescribe a device-pixel quantization strategy. The experiment asks
whether that different compositing boundary changes Wine's glyph origin or
axis rounding, while keeping the same mathematical geometry. The definitions
come from Adobe's [PDF Reference 1.5](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf),
sections 7.3.4 and 7.5.5; no vendor implementation is inspected.

The four independently authored HN-B axis fixtures are unchanged. The low grid
contains 42 glyphs over seven equal/unequal size profiles; the high grid contains
16 glyphs over four profiles. The reversed fixtures also change positions.
Baseline PDFs use original external font programs and the production alphabet
implementation `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`.
The checkout before this study is `a51f97a`, which adds no rendering change.

Five variants are prepared for each fixture:

| Variant | Form organization | Glyph transform |
| --- | --- | --- |
| Ordinary direct form | One ordinary form per glyph | Translation outside, original axis sizes inside |
| Isolated direct | One isolated group per glyph | Same decomposition as ordinary direct |
| Isolated split | One isolated group per glyph | Raw coordinate anchor outside, remaining displacement inside |
| Isolated axes | One isolated group per glyph | Axis sizes and original translation outside; unit text matrix inside |
| Page isolated | One isolated group for the entire page | Original drawing stream unchanged inside |

Groups use `/S /Transparency`, `/I true`, `/K false` and `/CS /DeviceRGB`.
Per-glyph bounds have generous margins around the original em extent; there is
no intended glyph clipping. Form matrices use the identity default. All actual
transform values are written into content streams at full decimal precision,
avoiding dictionary-number rounding of those transforms. The decompositions
are checked within 1e-25 points. Original embedded programs, page bounds and
semantic glyph text remain identical. Color operators and paint order are
retained; no per-zoom geometry or source-pixel adjustment is introduced.

The 21st PDF is the low forward page-isolated variant with fill/stroke opacity
0.5 inside the form. Its deliberate appearance change checks sensitivity to
the added graphics state. It is analyzed separately from full-opacity variants.

## Wine observations

The fixed reference is Windows CAJViewer 7.3.151 Simple under Wine 8.0, 32-bit,
using the established fonts/MSXML profile. Sessions are offline, nonroot, with
read-only input/root, 2 GiB memory/swap, two CPUs, 256 PIDs and a 600-second
lifetime. The display is 1600 × 1200 at 96 DPI, in single-page mode.

Each session confirms the page and zoom through numeric controls, takes two
captures 0.6 seconds apart and checks the fields again. Complete page crops
include their observed borders. Exact RGB comparison uses no registration,
resizing, tolerance or normalization. Independent tight glyph crops describe
shape only; they never replace the complete-page fidelity score.

| Fixture | Zoom | Baseline and every full-opacity variant: differing pixels |
| --- | ---: | ---: |
| Low forward | 75% | 624 |
| Low forward | 150% | 1,620 |
| Low reversed/shifted | 100% | 1,314 |
| High forward | 300% | 1,366 |
| High forward | 600% | 2,947 |
| High forward | 900% | 7,810 |
| High reversed/shifted | 1100% | 5,701 |

Each variant retains the baseline's combined 21,382 differing pixels over the
seven views. All 950 treatment glyph observations also retain their baseline
measurements. The receipt stores the 190 unique glyph-view arrays once, with
references, only after checking full array equality across all five variants.
Cold native-source repeats at low 75% and high 600% match exactly. No planned
visit fails or is excluded from this batch.

The half-opacity control at low 75% changes 1,040 pixels relative to the PDF
baseline. Interior nonwhite pixels fall from 1,040 to 27; the remaining pixels
have grayscale value 161. Thirty-one glyph cells are empty. The output still
has all 42 semantic glyphs and passes qpdf, demonstrating why those checks
cannot establish visible completeness. The control proves an observable
response to opacity; it does not establish correct alpha compositing or
explain how the full-opacity groups are implemented.

PyMuPDF 1.27.2.2's separate 2× diagnostic renders retain ordinary-form pixels but
change the four isolated variants for each fixture. These secondary renders
are sanity observations, not Wine compatibility or native-source comparisons.
Their exact tool version, counts and all 21 cases are in the receipt.

## Reproduction and acceptance

The independently authored MIT helper
[`wine_transparency_diagnostic.py`](../../scripts/research/wine_transparency_diagnostic.py)
reuses the original axis fixture description and PDF diagnostic utilities. It
requires pikepdf, PyMuPDF and qpdf; it adds no production dependency. For example:

```sh
python3 scripts/research/wine_transparency_diagnostic.py \
  /external/axis-raster-low-0-baseline.pdf /external/new-isolated.pdf \
  --family low --kind isolated-split --receipt /external/new-receipt.json
```

Use `--family high` or `--reverse` for the corresponding input fixture.
`--kind page-isolated --alpha 0.5` creates the positive control. Output PDFs
must not already exist. All **21 generated PDFs reproduce byte-for-byte** in
an independent invocation, including the half-opacity control, and every
metadata field agrees. All pass font-byte/page-bound/semantic checks and qpdf.

The preceding commit `a51f97a` has all four quality gates and nine regular native
platform jobs successful: 15 successful checks including inputs/assets and 17
optional/release skips. Those skips are not passing evidence. This study changes
only an external research helper and documentation/metadata, so it adds no
native/JS runtime test result. Its own CI is separate from the preceding commit.

#541 remains open: device rounding and unequal-axis rasterization still differ,
and these alternative encodings provide no production correction. The existing
77-page actual cohort is not reacquired or relabeled as passing. No actual-corpus
or runtime-parity improvement is claimed. Fonts, fixture/PDF binaries and
screenshots remain external. No private HN/JBIG, foreign converter or vendor
implementation is inspected, copied or migrated; no release is requested.

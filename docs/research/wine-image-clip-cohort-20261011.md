# Full-cohort image clipping and coordinate-order controls

Unconditional image clipping is not safe to adopt. Across all 13 inventoried
documents and 77 pages, it improves five pages, preserves 71 and regresses one.
Differing pixels decrease only from **2,874,038 to 2,873,438**. On the regressing
page, clipping introduces 184 mismatches at 75%, but improves the same page at
95%. Moving the mathematically identical clip before the image matrix produces
the same pixels and does not resolve that counterexample.

This follows the [image-Form study](wine-image-forms-20261011.md) under
[#543](https://github.com/rwv/caj2pdf-rust/issues/543), parent #406 and related
#541. The [receipt](wine-image-clip-cohort-20261011.json) preserves the complete
cohort, controls, failed visits, reference checks and reproductions. Production
remains `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; all 77 production pages
still differ. No production correction is adopted.

## Preserve the actual cohort before comparing pixels

The [new original helper](../../scripts/research/wine_image_clip_diagnostic.py)
inserts `0 0 1 1 re W n` after each original axis-aligned image matrix and before
its image draw. It accepts no zoom and performs no image decoding or matrix
arithmetic. Removing exactly those inserted bytes restores every original
page-content stream. Original page boxes, font programs, image payloads,
semantic text and paint order are preserved.

Full-cohort preflight identified input features outside the earlier generic
pattern helper: 48 image-free pages and three image draws with the project's
`BilevelOverlay` graphics state. The new helper handles absent image resources
and preserves that state verbatim. All six entirely image-free PDFs are copied
byte-for-byte. There are 57 image draws in total: 31 bilevel resources, including
three Multiply overlays, and 26 JPEGs. The 13 output documents retain all
**113,418 source-order semantic characters** and pass qpdf.

The research helper deliberately accepts only this project's single-stream,
unrotated, zero-origin pages with unique axis-aligned image draws. It refuses
unsupported or repeated draws and existing diagnostic clipping. It is not a
general PDF editor or a new production conversion path.

## All 77 pages, with fresh reference observations

The same CAJViewer 7.3.151 Simple / Wine 8.0 reference retains its original
resource-font profile, 96 DPI and 1600×1200 display. The driver retains a
read-only root, disabled networking, 2 GiB memory, two CPUs, 256 processes and
a 600-second bound. Complete RGB page crops include borders; no alignment,
rescaling or pixel tolerance is used.

Both the candidate and native source have fresh admitted views of **every
page**. All new native crops equal their historical source crops, including
the counterexample. Twelve additional source/candidate visits independently
repeat three bilevel-overlay pages and the pinned JPEG/mixed pages.

Only six cohort pages change:

| Source hash prefix / page | Current PDF | Unit clip | Change |
| --- | ---: | ---: | ---: |
| `03770ea1cdeb` / 4 | 37,967 | 37,889 | -78 |
| `1b9aa912e0bc` / 3 | 27,235 | 27,124 | -111 |
| `7e3f2c0faebb` / 10 | 78,226 | 77,801 | -425 |
| `d6a23a9cfd0f` / 2 | 72,476 | 72,310 | -166 |
| `d6a23a9cfd0f` / 3 | 62,621 | 62,805 | **+184** |
| `e1b17805a87f` / 6 | 148,381 | 148,377 | -4 |

Each improvement repairs existing mismatches without introducing new ones in
that view. The regression introduces 184 new mismatches without repairing any.
Its difference is a single JPEG top-edge row, with page-crop bounds
`[32,406,216,407]` and maximum channel change 48. The preceding page's 166-pixel
improvement is also a top-edge row. Both retain the original negative vertical
image matrix; the regression cannot be excluded merely by selecting that sign.

## Independent controls and an identical clip in another coordinate system

Thirty original control views cover black grids, integer extents, reversed
resolution order, placement and size variations, type-1/type-2 JPEG direction,
gray ramps and signed x/y positions at 50/75/100/150%. Three actual bilevel
overlay pages add a 100% native/baseline/candidate comparison. Of these 33
comparisons, six improve and 27 retain their pixels; none regresses or becomes
exact. Differences decrease from 220,010 to 217,581. The two gray type-2 views
remove 864 / 432 mismatches, and four signed type-2 views remove
443 / 639 / 18 / 33. Black endpoint and earlier fixed-zoom counterexamples
remain unresolved. Five independent native repeat visits are exact.

After the actual regression appeared, the
[page-coordinate helper](../../scripts/research/wine_image_page_clip_diagnostic.py)
tests another evaluation order. Instead of a unit rectangle after `cm`, it
places `e f a d re W n` before the original `a 0 0 d e f cm`. All four rectangle
tokens are copied from the original image matrix, with no arithmetic, fitted
epsilon or target-zoom adjustment. Undoing the inserted rectangle tokens
restores original content bytes. A separate unit-clipped intermediate is
retained for reproduction.

All six changed cohort pages are observed with this construction at 75%.
The two changed pages of `d6a23a9cfd0f` also receive fresh source, baseline and
both clipping routes at 95%:

| Page / zoom | Current PDF | Unit clip | Page-coordinate clip |
| --- | ---: | ---: | ---: |
| 2 / 75% | 72,476 | 72,310 | 72,310 |
| 3 / 75% | 62,621 | 62,805 | 62,805 |
| 2 / 95% | 86,142 | 85,679 | 85,679 |
| 3 / 95% | 57,452 | 57,219 | 57,219 |

Every one of the **eight distinct page-coordinate views equals the corresponding
unit-clipped frame**, not merely its score. Fresh 75% source/baseline repeats
also agree. The same page's change from regression to improvement across zooms
rejects a universal clipping correction and gives no basis for a fitted
document-specific exception.
These observations do not identify the viewer's internal arithmetic.

## Admission and retained failures

| Stage | Cold session attempts | Admitted visits |
| --- | ---: | ---: |
| Full candidate cohort, repeats and bootstrap retry | 18 | 89 |
| Independent controls and bilevel follow-up | 36 | 44 |
| Coordinate-order study, alternate scale and information-bar retries | 18 | 20 |
| Fresh full native cohort | 13 | 77 |
| Total | **85** | **230** |

There are 248 requested visits: 230 are admitted, twelve captured visits are
rejected and six are unobserved after a terminal startup failure. The admitted
set contains 122 primary comparisons, 21 exact repeat visits and 87 reference
visits. Every source and isolated copy retains its hash; all 85 containers are
removed and none is OOM-killed.

One six-page candidate reaches a terminal numeric-layout startup failure
before capturing a page. Its retained replacement uses the already established
one-page bootstrap route; all six visits pass the unchanged final checks.

Eight 100% counterexample visits are incomplete in the fixed viewport and are
excluded. The same files are subsequently observed at 95%; these are new-scale
observations, not inferred 100% results. Source information bars additionally
make four 75/95% visits fail the whole-viewport check despite complete paper
crops. The existing close-bar retry driver admits four replacement visits.

Reference review finds seven historical native sessions with incomplete
whole-viewport flags despite complete independently extracted paper crops.
The final analysis requires **both** gates on both sides, so all 13 native
documents are observed afresh. All 77 new crops equal the historical crops;
final scores use the new source labels. Preliminary analyses that used only
the independent crop gate for references remain separate from the final
validated receipt. No ineligible reference contributes a final score.

Two preparation failures are also retained: an initial string/Path mismatch
in the semantic-verifier wrapper, and an overbroad page-coordinate verifier
that removed existing text clipping triples. Correcting argument type and
undoing only the inserted rectangle bytes leaves the originally generated
PDFs byte-identical. Both failures are corrected before any Wine observation
of the affected output.

## Reproduction and remaining work

The published original MIT helpers reproduce all 31 newly prepared comparison
PDFs and metadata, five page-coordinate intermediates, and three earlier
direct-clip PDFs byte-for-byte. Both command-line entry points reproduce an
output, and function ASTs match the final observed prototypes. Source/font/
image/content checks, qpdf, source inventory and documentation links pass.

These results rule out unconditional unit clipping and the tested equivalent
coordinate-order rewrite as general fixes. They do not justify a fitted
per-page switch or a zoom-specific geometry change. Endpoint sizes, sampling
and original-font raster differences remain under #543/#541/#406. Production
APIs, dependencies and bounded I/O are unchanged; no runtime-parity sweep,
merge or release is claimed. Corpus, fonts, PDFs and screenshots remain external.

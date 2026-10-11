# Wine clipped-image sampling and device-coordinate cropping

Computing a decoded-pixel crop from **truncated device endpoints**, rather than
from the world-coordinate page intersection, makes all three horizontal gray
grids and two independent horizontal RGB views exact. A separately measured
vertical-origin carry and adjacent-row edge diagnostic also make three
vertical gray grids and two vertical RGB views exact. These are **ten complete
original-control views** at 75/100/150%, including clipped images.

The construction does not yet generalize. A held-out 125% frame exposes extra
destination pixels; a later 175% frame also differs. Two attempts to bound
floating-point extents do not resolve both views and introduce regressions.
The diagnostics still change geometry or sampled pixels at an explicit zoom.
**No production correction is adopted.**

This continues [#543](https://github.com/rwv/caj2pdf-rust/issues/543) under #406,
related to #541, from the [descriptor/clipping study](wine-image-signed-clipping-20261011.md).
The [receipt](wine-image-device-clipping-20261011.json) retains all declarations,
observations, failed models, fixed-strip values, graph checks and reproductions.
Production remains `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; all 77 production
cohort pages still differ. The actual-document cohort is not rerun here.

## Reference and admission

The unchanged reference is CAJViewer 7.3.151 Simple on Wine 8.0 with the original
resource-font profile, 96 DPI and a 1600×1200 display. The existing driver keeps
the read-only root, network isolation, 2 GiB memory, two CPUs, 256 processes
and 600-second session bound. No readiness or page-admission rule changes.

| Stage | Cold sessions / admitted visits |
| --- | ---: |
| Original paired gray controls, PDFs, extent probes and repeats | 20 |
| Six device-crop models and encoding controls | 42 |
| Vertical-origin carry and guard padding | 6 |
| Independent horizontal RGB checks | 2 |
| Adjacent-row diagnostic and held-out 125% | 8 |
| Independent vertical RGB checks | 2 |
| Rounded-product bound and held-out 175% | 6 |
| Exact coefficient bounds at 125/175% | 2 |
| Total | **88** |

All 88 visits are complete, stable and numeric-confirmed. Every input and
isolated copy retains its hash; all containers are removed and none is
OOM-killed. There are 76 comparisons, eight new native source views and four
exact cold repeats: x at 150%, y at 100%, 125% and 175%. Earlier RGB source/PDF
views are reused explicitly. No GUI visit is rejected or retried.

Primary scores use complete RGB page crops including borders, without
alignment, rescaling or tolerance. Source-derived central strips are separate
diagnostics and cannot establish complete-frame equality.

## Same-payload pairs across the page boundary

The [original generator](../../scripts/research/wine_image_clipped_pairs.py)
writes two 3900×3900-unit C8 pages, one per clipped axis. Each has nine pairs:
source resolutions 33, 129 and 513 along the varying axis, crossed with raw
positions -123, -7 and -1. A partner lies 1484 raw units farther along that
axis. Both members use identical grayscale JPEG bytes and 1113-unit extents.
The perpendicular pixel dimension is 17, its physical extent is 140 units,
and panels are separated by 200 raw units. All descriptors are type 2.
The ramp uses `round(i*254/(N-1))`, encoded at JPEG quality 100.

The 1484-unit translation equals 144/192/288 device pixels at 75/100/150%,
and 240/336 at 125/175%. Pair comparisons therefore use declared integer
translations, not screenshot-fitted shifts. Different signed endpoint sizes
remain a competing explanation for a translation difference; equal physical
extents do not imply equal quantized native destination sizes.

Central strips exclude fixed page/image edge bands and check three identical
parallel traces with equal RGB channels. Under the preceding truncate/keep-x
or truncate/carry-y extent diagnostic, all **54 in-page partner traces** at
75/100/150% equal native values. Only 34 of 54 clipped traces do. Whole-frame
differences remain 1,952/3,059/23,940 for x and 1,924/3,078/4,284 for y.
This separates clipped sampling from the already matched in-page traces.

## Device-coordinate source windows

The [device-crop helper](../../scripts/research/wine_image_device_crop_diagnostic.py)
recovers integer source boxes within the previous serialization tolerance.
Let `u=72/742` points per raw unit and `q=zoom/75` pixels per point. Two endpoint
quantizers are predeclared: floor and truncation toward zero. For either `Q`:

```text
x0 = Q(x*u*q)          x1 = Q((x+w)*u*q) + 1
y0 = Q(y*u*q)          y1 = Q((y+h)*u*q) + 1
cx0 = max(0,x0)        cy0 = max(0,y0)
```

Only partial left/top clipping is admitted. With original JPEG dimensions
`W,H` and negative vertical PDF matrix direction, the encoded-pixel window is:

```text
[(cx0-x0)*W/(x1-x0), 0, W, H-(cy0-y0)*H/(y1-y0)]
```

Floor, ceil and nearest (`floor(v+1/2)`) independently quantize the window
edges, giving six declared models. Cropped images are decoded at full
resolution, bounded to ten million pixels each, and painted losslessly at
the visible device extent. Original JPEGs remain embedded. Unclipped axes
retain their original affine phase; their extents receive the declared
endpoint correction. No screenshot values determine coordinates or crops.

Six separate JPEG-to-Flate identity views preserve every baseline pixel.
For horizontal clipping, truncate/nearest is exact at all three zooms:

| x view | Current PDF | Previous extent diagnostic | Device crop, truncate/nearest |
| --- | ---: | ---: | ---: |
| 75% | 12,064 | 1,952 | **0** |
| 100% | 18,171 | 3,059 | **0** |
| 150% | 47,349 | 23,940 | **0** |

Truncate/floor also reaches zero at 75/100%, but leaves 9,212 pixels at 150%.
Truncate/ceil leaves 1,951/3,097/4,536 pixels. All floor-endpoint combinations
remain nonzero. The complete model table is retained in the receipt.

Applying the unchanged truncate/nearest rule to the prior independent
asymmetric RGB x-control makes both complete 75/150% frames exact, from
4,391/6,508 differing pixels. This extends the result beyond grayscale ramps,
but does not establish a universal codec or resampler.

## Vertical carry, a failed padding construction and an adjacent source row

The initial device crop leaves 8,332/14,147/14,752 pixels on the y grids.
The [follow-up helper](../../scripts/research/wine_image_device_crop_followup.py)
separately restores the previously measured y-origin carry when raw y is
negative but its truncated device coordinate is zero:

```text
f' = f - (truncate(y*u*q) - floor(y*u*q))/q
```

That reduces the whole-frame differences to **28, 57 and 140 pixels** at
75/100/150%. Only the 33-pixel-high image at y=-123 differs: the visible top
band contains an adjacent original JPEG row beyond the rounded source window.
The band spans 2/3/5 rows across native widths 14/19/28 respectively.

A first guard construction appends that original row to the resource while
compensating the matrix to preserve the previous visible pixel pitch and
bottom anchor. It regresses all three views to 3,695/6,384/21,672 pixels.
Retaining mathematical visible pitch is insufficient to preserve this
renderer’s sampling after the enlarged image is clipped again.

A separate diagnostic paints the adjacent original row over a top band of
`max(0, floor(visible_device_height/cropped_pixel_height)-1)` device rows.
This deliberately changes painted samples and adds an image draw; it is
not a presumed geometry-preserving equivalent. It makes all three complete
y grids exact. Applying the same rule to independent RGB y-controls also
reaches zero from 1,175/2,935 pixels at 75/150%. Their ratios predict no added
edge band, so these two checks exercise device cropping plus origin carry.

## Held-out zooms and numeric counterexamples

Before observing 125%, exact scale `120/742` predicts y endpoints -19/161,
a cropped source height of 30, and a four-row band of native width 23:
92 source-differing pixels. An initial hand calculation incorrectly expected
six rows; a preparation assertion failed before any 125% Wine visit. The
unchanged formula already gave four. The failed files/log remain retained;
the declaration was corrected in a new directory before observation.

At 125%, carry alone leaves 8,072 pixels, and the edge band removes the
predicted **92**, leaving **7,980**. Central in-page strips still equal native
values, but independent perpendicular spans show several 23-pixel native
panels drawn 24 pixels wide by the diagnostic PDF. This is why complete-frame
scores, rather than central-strip equality, remain authoritative.

In ordinary binary64 arithmetic, `13.8*(125/75)` is
`23.000000000000004`; similarly, `107.4*(125/75)` is just above 179.
The [extent diagnostic](../../scripts/research/wine_image_device_extent_rounding.py)
first steps a coefficient toward zero with `nextafter` only while its rounded
binary64 product would ceil beyond the declared integer extent. Images,
affine origins, font programs and other operators remain unchanged.

That improves 125% to 1,241 pixels, but a fresh 175% control regresses from
32,264 to 48,788. A second construction bounds both serialized decimal and
parsed binary64 coefficients by
`target/max(exact_rational_scale, binary64_scale)`, so even an unrounded
product cannot exceed the declared target. It leaves 3,212/48,788 pixels.

| Vertical view | Current PDF | Edge diagnostic | Rounded-product bound | Exact coefficient bound |
| --- | ---: | ---: | ---: | ---: |
| 125% | 44,866 | 7,980 | 1,241 | 3,212 |
| 175% | 94,430 | 32,264 | **48,788** | **48,788** |

The numeric constructions use declared matrices, not fitted pixel offsets.
Their failures reject a universal endpoint-serialization correction here.
They do not identify the viewer’s internal floating-point precision or prove
that the observed arithmetic example is its complete implementation path.
Both new native views reproduce exactly in cold sessions.

## Reproduction and input-admission correction

Both new original sources and their current baseline PDFs reproduce exactly.
All **64 diagnostic PDFs** and their metadata also reproduce byte-exactly.
Public helper functions match the observed prototypes, except the documented
local-to-public import name and the explicit RGB admission extension; prior
grayscale outputs remain exact under that extension. Original fonts, page
boxes and nonimage operators are checked, with added edge images recorded
separately. All PDFs pass qpdf. Font/document/PDF/screenshot bytes stay external.

The first RGB preparation attempt rejected the existing
`/DecodeParms << /ColorTransform 1 >>` profile. Six already prepared vertical
PDFs were retained. The crop helper now admits that explicit RGB profile as
well as default JPEG decoding, while rejecting other decode parameters and
image decode/mask overrides. No matrix/crop algorithm changed for the
previously observed grayscale inputs.

Inspection also found missing slash prefixes in the option checks of the
earlier orientation and world-coordinate pixel-clip research helpers.
Before decoding an image, their checks now reject actual `/Decode`, `/Mask`,
`/SMask` and unsupported decode parameters while admitting the known RGB profile. Six positive
admissions and fifteen explicit refusals pass; both corrected helpers retain
byte-exact admitted RGB outputs. These are research-helper corrections,
not production parser changes.

All new/changed code is original MIT work. No vendor implementation, foreign
converter or private HN/JBIG source is inspected or migrated. Production
dependencies, APIs and bounded I/O are unchanged; production runtime suites
were not rerun without a production change.

## Remaining implementation obligation

Ten complete control views are now explained by the combined endpoint,
device-crop, vertical-origin and edge-row diagnostics. The retained 125/175%
counterexamples prevent a general model claim. Right/bottom and simultaneous
clipping, descriptor-kind-1 normalization, applicable bilevel boundaries,
actual image-only/mixed pages and the full production cohort still need the
corresponding implementation evidence.

#543 remains open for a geometry-preserving correction without a fixed zoom
or per-document fitting, independent controls, actual-cohort nonregression,
bounded native/CLI/Node/browser checks and final review. #541/#406 remain open;
closed #534 still denotes completed unresolved classification, not fidelity.
No merge or release is included.

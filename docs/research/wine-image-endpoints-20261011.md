# Wine image endpoints and device-scale diagnostics

Original black-image controls isolate a native/PDF endpoint-size difference:
all **132 measured axes** follow inclusive native endpoints, while the PDF
image path follows the ceiling of the scaled extent. Preserving the original
affine top-left and replacing only the extent for a **declared zoom** makes
all six black-grid views exact and improves nine asymmetric-color views plus
nine actual-document views. However, two cross-zoom controls regress. These
diagnostic matrices change physical geometry and are **not a production fix**.

Implementation follow-up: [#543](https://github.com/rwv/caj2pdf-rust/issues/543),
under [#406](https://github.com/rwv/caj2pdf-rust/issues/406), related to #541 and
#441. Closed #534 completed its unresolved-classification scope; its closure
did not establish pixel fidelity. Draft #542 remains open. The production
baseline is `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; this study changes no
converter behavior, original fonts, public API or runtime dependencies.
All 77 pages in the existing production cohort still differ from Wine.

The [machine-readable receipt](wine-image-endpoints-20261011.json) includes
declarations, input/output hashes, every admitted visit, strict comparisons,
per-panel geometry, reproduction checks and retained failures. Documents,
fonts, generated PDFs and screenshots remain external.

## Reference and scope

The fixed reference is Windows CAJViewer 7.3.151 Simple under Wine 8.0
(Debian `8.0~repack-4`), with the unchanged original resource fonts and 96-DPI,
1600×1200 display. Each cold session uses the existing isolated profile,
network-disabled container, 2 GiB memory limit, two CPUs, 256-process limit,
read-only root and 600-second bound. The receipt pins the image and drivers.

| Stage | Cold sessions | Admitted page visits |
| --- | ---: | ---: |
| Original asymmetric sources and fractional black grids | 18 | 18 |
| Integer extents, rectangle/stencil routes and gray positive control | 20 | 20 |
| Snap origin and extent to the declared device grid | 18 | 24 |
| Retain affine origin; change only declared device extent | 19 | 27 |
| Reuse those same diagnostic PDFs at another zoom | 4 | 4 |
| Total | **79** | **93** |

Every visit has complete page bounds, confirmed page/zoom fields and two
identical stable frames. Every input hash and isolated copy remains intact;
all containers are removed and none reports OOM. There are 80 comparisons,
six fresh native-grid source views and seven exact cold-repeat views. The
repeats cover the asymmetric baseline at 50/75/100%, the integer grid at 150%,
and actual pages 1, 5 and 10 at 75%. There are no rejected GUI frames or retries.
Historical native source views are reused only after their admission/integrity
checks. They are not counted as new sessions.

Scoring compares untouched complete RGB page crops, including the page border,
with no alignment, rescaling, tolerance or pixel correction. Diagnostic panel
cells are fixed from source geometry; full-frame differences include every
pixel outside them. All black-grid ink is inside those cells. Font selection
and the existing 77-page production comparison are not rerun or relabeled.

## Original controls and endpoint rule

The three asymmetric RGB controls reproduce the project's existing original
MIT [generator](https://github.com/rwv/caj2pdf-samples/blob/1c7edbaaae9faee93951b6799dff6e83f554b543/research/cajviewer/wine_raster_rounding_controls.py)
and [image fixture](https://github.com/rwv/caj2pdf-samples/blob/1c7edbaaae9faee93951b6799dff6e83f554b543/research/cajviewer/c8_image_fixture.py)
byte-for-byte. Their 256×192 JPEG has black top/left edges and asymmetric blue
and red regions. The page is 5105×4800 raw units, origin (400,500), image extent
3500×2500. Controls change page width to 5106 or move the image to (420,520).
Fresh baseline PDFs use the current `72/742` point scale; historical PDFs using
`240/2473` are not reused as current candidates.

The original
[control generator](../../scripts/research/wine_image_endpoint_controls.py)
also emits two 16-panel grids and a six-panel integer grid. Uniform-black
grayscale JPEGs at 8×6, 32×24, 128×96 and 512×384 separate pixel resolution
from destination size. Fractional-grid widths are 420, 421, 427 and 443 raw
units; heights are 320, 327, 341 and 359. The second grid reverses resolution
order and shifts positions by (11,17). Integer-grid extents 371, 742 and 1113
produce integral device sizes at 75/100/150%, using both extreme resolutions.
Every decoded source pixel is black; the manifest declares all coordinates.

For raw origin `(x,y)`, extent `(w,h)` and `s = 96/742 * zoom/100`, observed
solid-black bounding boxes, measured relative to the existing page crop, are:

```text
native left   = floor(x*s)
native top    = 1 + floor(y*s)
native width  = floor((x+w)*s) - floor(x*s) + 1
native height = floor((y+h)*s) - floor(y*s) + 1

PDF width     = ceil(w*s)
PDF height    = ceil(h*s)
```

Native and baseline-PDF origins agree in these controls. All 132 native size
axes and all 132 PDF size axes match the respective rules; each panel is a
solid rectangle of black pixels. The one-pixel top term is in the measured
page-crop coordinate convention, not a proposed physical translation.
Reversed/shifted controls were declared before first model observation.
The subsequent integer controls were designed after initial results to
distinguish `ceil(size)` from `floor(size)+1`, before viewing their frames.
The receipt marks 68 axes from these phased controls; it does not represent
all 132 axes as a blinded validation set. No resolution-dependent size change
is observed in this range. Arbitrary colors, codecs and internal resampling
algorithms are not established by uniform black.

## Routes with unchanged mathematical area

The [black-image diagnostic](../../scripts/research/wine_black_image_diagnostic.py)
accepts only verified one-page uniform-black grayscale-JPEG controls. It keeps
original resources and page boxes while replacing each draw with either a
filled unit rectangle or an identically sized one-bit image stencil. A stencil
with zero samples and `Decode [0 1]` paints the current color under the
[PDF Reference, image masks](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf).
That specification defines painting semantics, not Wine's endpoint rounding.

| Grid / zoom | Baseline differing pixels | Rectangle | Black stencil |
| --- | ---: | ---: | ---: |
| Fractional / 75% | 659 | 1,076 | 659 |
| Fractional / 150% | 717 | 2,116 | 717 |
| Reversed/shifted / 100% | 624 | 1,426 | 624 |
| Integer / 75% | 870 | 876 | 870 |
| Integer / 100% | 1,158 | 1,164 | 1,158 |
| Integer / 150% | 1,734 | 1,740 | 1,734 |

All six rectangle views regress; all six black stencils reproduce baseline
RGB exactly. Painting the fractional 75% stencil in gray changes 22,228 pixels
relative to its PDF baseline, with all panels retained at RGB (128,128,128).
This deliberate positive control differs from the source by 22,887 pixels;
it demonstrates an observable color response, not a fidelity improvement.

## Device-scale diagnostics and retained counterexamples

The [device-grid helper](../../scripts/research/wine_image_device_grid.py)
reads current axis-aligned image matrices `(a,0,0,d,e,f)`, page height `H` and
declared device scale `q = zoom/75` pixels per PDF point. For `a > 0`, `d != 0`:

```text
left = e                         top = H - max(f, f+d)
x0 = floor(left*q)               x1 = floor((left+a)*q) + 1
y0 = floor(top*q)                y1 = floor((top+abs(d))*q) + 1
a' = (x1-x0)/q                   d' = sign(d)*(y1-y0)/q
```

The first variant also snaps the origin: `e' = x0/q`, and
`f' = H - (y1 if d>0 else y0)/q`. The second keeps the original affine top-left:
`e' = e`, `f' = f+d-d'` when `d>0`, otherwise `f' = f`. Exact rational arithmetic
and 30-place decimal output avoid fitting offsets from observed screenshots.
Image payloads, font programs, page boxes and all nonimage operators remain
unchanged. Qpdf accepts every output. These checks do not establish equivalent
physical geometry: both variants deliberately change extents at one scale.

Full-origin snapping improves 18 of 24 views and regresses six. All three
integer-grid views become exact, but the three fractional-grid views regress
to 680, 1,340 and 904 differing pixels. Lower-grid rows move up by one pixel
despite matching panel dimensions; the internal cause of that shift is not
proven. The asymmetric baseline/width controls also regress at 100%.

Retaining the affine origin improves **all 24 target-zoom views**: all six
black grids become exact (66 panel views), all nine asymmetric-color views
improve, and all nine actual views improve. The asymmetric baseline and
width-plus-one controls change from 532/702/1336 to 319/383/510 differing
pixels at 50/75/100%; the shifted control changes from 404/702/969 to
318/383/539. Nonuniform color residuals remain.

For unchanged actual source SHA-256
`7e3f2c0faebb8ddac2f4feff3cfc9249476ab170b43250ac00610b10aa46c74b`,
pages 1 and 5 are image-only and page 10 mixes text and images. The retained
baseline PDF and source views come from the
[JPEG diagnostic](wine-jpeg-decode-controls-20261011.md), with the same
production geometry and original font programs.

| Zoom / page | Baseline | Snapped origin + extent | Original origin + changed extent |
| --- | ---: | ---: | ---: |
| 50% / 1 | 60,368 | 56,496 | 58,014 |
| 50% / 5 | 83,964 | 79,353 | 81,330 |
| 50% / 10 | 52,162 | 26,270 | 26,270 |
| 75% / 1 | 128,982 | 116,224 | 116,594 |
| 75% / 5 | 183,080 | 170,516 | 171,054 |
| 75% / 10 | 78,226 | 18,417 | 18,417 |
| 100% / 1 | 183,148 | 180,945 | 177,231 |
| 100% / 5 | 284,197 | 279,137 | 275,183 |
| 100% / 10 | 163,116 | 56,812 | 56,812 |
| Total | **1,217,243** | **984,170** | **980,905** |

The extent-only diagnostic reduces the total by 236,338 pixels (19.4%);
every actual view still differs. This is nine views of one selected document,
not an improved production cohort or a new corpus sweep.

Reusing identical extent-only diagnostic PDF bytes at another zoom directly
tests whether that physical correction generalizes:

| Grid | Prepared zoom | Viewed zoom | Baseline | Diagnostic |
| --- | ---: | ---: | ---: | ---: |
| Fractional | 75% | 150% | 717 | **2,851** |
| Fractional | 150% | 75% | 659 | 434 |
| Integer | 75% | 100% | 1,158 | **1,170** |
| Integer | 100% | 75% | 870 | 0 |

Two regressions rule out adopting this target-zoom matrix adjustment as a
general PDF correction. The required next step is a geometry-preserving
drawing route or a separately validated endpoint/resampling implementation,
with cross-zoom, phase, signed-matrix and nonuniform-pixel controls. Installing
more fonts cannot explain the image-only black-grid endpoint discrepancy.

## Reproduction, provenance and limits

The three public helpers are original MIT research utilities using installed
Pillow, pikepdf and qpdf. The source generator uses project-owned C8 framing
facts and the original asymmetric pattern linked above. No vendor, private
HN/JBIG or differently licensed converter implementation was inspected or
migrated. The source generator creates fixed small documents, with at most
512×384 pixels per image; the black diagnostic bounds images to 1024×1024.
The PDF utilities operate on external research files and are not part of the
production bounded-I/O path. They add no converter dependency or release.

Example commands, using new external output paths:

```sh
python3 scripts/research/wine_image_endpoint_controls.py /tmp/new-image-controls
python3 scripts/research/wine_black_image_diagnostic.py \
  /path/to/black-grid.pdf /tmp/new-stencil.pdf --kind stencil
python3 scripts/research/wine_image_device_grid.py \
  /path/to/baseline.pdf /tmp/new-extent-75.pdf --zoom 75 --origin keep
```

All six original sources and their current-CLI baseline PDFs reproduce
byte-for-byte. Their declared source matrices agree within `1e-10` points.
The first diagnostic exact-file comparison failed: the 39,515-byte rectangle
PDFs differed at 62 byte positions, all in the single trailer `/ID` record.
The initial outputs and failure remain retained. Final helpers request
deterministic IDs. All **43** modified diagnostic outputs reproduce every byte
of the observed PDF except that trailer ID, with identical nonhash metadata;
**10** separate deterministic repeat outputs are byte-exact. Observed files
and capture identities were not replaced or relabeled. PDF parsing/graph
checks and reproduction are not additional Wine pixel passes.

Production Rust/JS tests and the 77-page sweep are not repeated because this
change only adds research helpers and evidence. Source inventory, helper
syntax, reproduction, receipt invariants and documentation links are checked.
Required CI for the preceding checkout `024d321` completed successfully:
four quality jobs and nine regular native jobs, plus inputs/assets checks;
17 skipped optional/release jobs are excluded from passing evidence. CI for
the publication commit is tracked separately in draft #542.

No general decoder/resampler cause, other-codec rule, production pixel parity
or completion of #543/#541/#406 is claimed. The target-zoom gains and the
cross-zoom failures remain equally part of the evidence.

# Wine image-only Form and transparency diagnostics

Tight image bounds remove some differing edge pixels while preserving original
image bytes and transform tokens. An ordinary unit-square clipping path
reproduces all ten matching tight-Form views, so these gains do not require a
Form XObject. Isolated transparency groups are not a
general correction: the mixed actual page improves slightly at 75% but
regresses at 100%. The measured image-only pages and black endpoint grid retain
their baseline differences. No production change is adopted.

This continues [#543](https://github.com/rwv/caj2pdf-rust/issues/543) under #406,
related to #541, after the [device-clipping study](wine-image-device-clipping-20261011.md).
The [receipt](wine-image-forms-20261011.json) retains every declaration,
comparison, pixel transition, cold repeat and reproduction. Production remains
`7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; every page in the existing 77-page
production cohort still differs. This is a scoped diagnostic, not a new
full-cohort or runtime-parity result.

## Construction and scope

The [original MIT helper](../../scripts/research/wine_image_form_diagnostic.py)
wraps each existing image in a separate Form. Each Form has an independent
resource dictionary containing the original image, so page-resource changes
cannot create a recursive resource graph. The Form matrix is omitted, leaving
its identity default. Exactly one of the inner and outer transforms is identity;
the other retains the original numeric tokens. No image is decoded, no zoom
argument is accepted and no coordinate is fitted to a screenshot.

| Kind | Form bounds | Transform placement | Group |
| --- | --- | --- | --- |
| `unit` | `[0,0,1,1]` | Original matrix outside | Ordinary |
| `unit-isolated` | `[0,0,1,1]` | Original matrix outside | Isolated RGB |
| `unit-isolated-wide` | `[-1,-1,2,2]` | Original matrix outside | Isolated RGB |
| `page` | Original page box | Original matrix inside | Ordinary |
| `page-isolated` | Original page box | Original matrix inside | Isolated RGB |

Opaque variants preserve net image matrices and mathematical visible support.
That does not imply raster identity: Form bounds add clipping, and groups add
compositing. These mechanisms follow the Form dictionary and transparency-group
definitions in sections 4.9.1 and 7.5.5 of the
[Adobe PDF Reference 1.5](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf).
The observations below determine their effect in the selected viewer.

All five variants are applied to four original sources: a black endpoint grid,
the previous paired x/y grayscale ramps and the asymmetric type-2 RGB image.
They also wrap the pinned actual PDF, observing image-only pages 1/5 and mixed
page 10. Its native source hash is
`7e3f2c0faebb8ddac2f4feff3cfc9249476ab170b43250ac00610b10aa46c74b`;
the unchanged baseline PDF hash is
`e04184c473b9d178eef8ade48b23ddf071afdd808670ee30472c27c6e3b36465`.

Two black/RGB half-opacity variants deliberately change painted values. These
positive controls paint nonblank interiors and change 22,228 / 82,620 Wine
pixels from their baseline PDFs. They are not fidelity candidates.

The helper verifies page boxes, original image payloads, font-program hashes,
nonimage operators and exact extracted-text hashes over every output document.
Qpdf accepts each output. The 27 Form PDFs reproduce byte-for-byte with the
published helper, including metadata. The public helper's function ASTs match
the observed prototype; its command-line interface also reproduces an output.

## Wine observations

The unchanged CAJViewer 7.3.151 Simple / Wine 8.0 reference uses the original
resource-font profile, 96 DPI and a 1600×1200 display. Sessions retain the
read-only root, disabled networking, 2 GiB memory, two CPUs, 256 processes and
600-second bound. Complete RGB page crops include borders and use no alignment,
rescaling or tolerance. Only complete, stable, numeric-confirmed visits count.

The 75% pilot has 29 cold sessions / 41 visits: 35 opaque comparisons, two
half-opacity positives and four native repeat visits. The adaptive follow-up
reuses identical PDFs at 150% for x/y gray and 100% for the actual source,
plus a fresh native repeat and a repeat of the mixed-page isolated-group result:
nine sessions / 17 visits. No coordinate or helper change occurs between scales.

Selected strict differences from Wine native source are:

| View | Current PDF | Tight ordinary Form | Tight isolated group | Generous isolated group |
| --- | ---: | ---: | ---: | ---: |
| Black grid, 75% | 659 | 659 | 659 | 659 |
| x gray, 75% | 12,064 | 11,222 | 11,222 | 12,064 |
| y gray, 75% | 19,961 | 19,961 | 19,961 | 19,961 |
| RGB, 75% | 937 | 937 | 937 | 937 |
| Actual page 10, 75% | 78,226 | 77,801 | 77,994 | 78,226 |
| x gray, 150% | 47,349 | 44,181 | Not observed | 47,349 |
| y gray, 150% | 55,781 | 55,781 | Not observed | 55,781 |
| Actual page 10, 100% | 163,116 | 162,204 | 163,705 | 163,116 |

All measured actual image-only views retain baseline pixels: page 1 differs by
128,982 / 183,148 pixels at 75/100%, and page 5 by 183,080 / 284,197. Page-sized
ordinary and isolated Forms retain every pilot baseline pixel. Of the 48
opaque Form comparisons, six improve, 41 retain their score and one regresses;
none is exact. Unobserved combinations are not counted as passing controls.

Aggregate pixel counts hide local regressions. At 75%, the tight isolated
group on page 10 repairs 666 mismatches but introduces 434, while another
6,638 changed pixels still differ. At 100% it repairs 1,746, introduces 2,335
and changes another 19,011 still-different pixels. Its independent 75% cold
repeat is exact. In contrast, the ordinary tight Form repairs 425 / 912 pixels
on the mixed page, with no newly introduced mismatches in those two views.
This does not establish safety on unmeasured pages or scales.

## Direct clipping separates the small improvement from the Form route

A final adaptive discriminator uses the unchanged
[image-route helper](../../scripts/research/wine_image_pattern_diagnostic.py)
to add `0 0 1 1 re W n` immediately after each original image matrix and before
its image draw. It preserves payloads, transforms, font programs, nonimage
operators, semantic text and page boxes. It adds no Form, transparency group,
sampling correction, zoom parameter or geometric adjustment.

Three PDFs cover x/y gray at 75/150% and actual pages 1/5/10 at 75/100%:
six fresh sessions / ten visits. **Every complete direct-clip frame equals
the corresponding tight ordinary Form frame.** Four views improve from the
baseline: x at 75/150% removes 842 / 3,168 differences, and actual page 10 at
75/100% removes 425 / 912. The other six views retain baseline pixels. All
three direct-clip PDFs and their metadata reproduce byte-for-byte. The earlier
route study had checked the black grid and actual page 1 at 75%; its unchanged
results did not cover these paired-gray and mixed-page views.

This makes clipping a simpler candidate for broader cohort evaluation. It is
not a solution to the remaining endpoint/sampling discrepancies, and the
unmeasured cohort cannot be inferred safe from these ten views.

Across all three stages there are **44 cold sessions / 68 admitted visits**:
58 opaque Form/direct-clip comparisons, two half-opacity positives and eight
exact repeat visits (seven native and one candidate). Ten opaque comparisons
improve, 47 retain their score and one regresses; none is exact. All inputs and
isolated copies retain their hashes, every container is removed and none is
OOM-killed. No GUI visit is rejected or retried. Thirty PDFs reproduce exactly.

## Independent renderer check

MuPDF 1.27.2, accessed through PyMuPDF 1.27.2.2, separately renders all pilot
variants at one and two pixels per point. Of 70 opaque views, 55 retain
baseline pixels and 15 differ; all four
half-opacity views change and all 74 outputs are nonblank. Tight bounds change
some gray and mixed-page edges. Isolated groups also change mixed-page values,
including generous and page-sized groups that retained Wine baseline pixels.
The receipt records exact sizes, hashes, counts and maximum channel changes.
MuPDF is a construction check, not a replacement source of truth, and its
results are not mixed into Wine scores.

## Acceptance limits

The results separate a small clipping response from a general endpoint or
sampling correction. They do not resolve the earlier zoom-dependent size and
source-window discrepancies. Original payloads, fonts, text and transform
tokens alone do not guarantee the same raster.

The full 77-page cohort, independent signed/bilevel controls, bounded runtime
parity for an implementation change and final review remain outstanding under
#543. No source corpus, font, PDF or screenshot bytes are committed. Production
APIs, dependencies, memory behavior and release state remain unchanged.

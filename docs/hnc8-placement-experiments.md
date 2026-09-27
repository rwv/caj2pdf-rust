<!-- SPDX-License-Identifier: MIT -->

# Additional-image placement experiments

This is the pre-experiment plan for [#110](https://github.com/rwv/caj2pdf-rust/issues/110).
The [#107 metadata oracle](hnc8-layout-oracle.md) measures 50 additional
type-2 JPEG draws on HN-A/C8 pages, but their x/y source fields remain
unknown. A measured PDF transform is an outcome, never an input to a
production placement rule. This investigation does not compose PDF pages or
alter the CLI, browser, or Node.js APIs.

## Fixed page split and evidence labels

Use HN-A pages 16, 22, 26, 29–36, 41, 47, 48 and 50 (15 pages, 16 additional
draws) and C8 pages 1–5 (5 pages, 20 draws) for discovery. Reserve HN-A pages
52–54, 57, 58, 60 and 61 (7 pages, 7 draws) and C8 pages 6–7 (2 pages,
7 draws) for the first validation pass. Both documents and every transform
are already public in the oracle: this is a predeclared page split, not a
blind independent-document test. Any formula revised after validation needs
newly generated validation cases.

Label evidence as **structural observation** (source byte-layout and checked
span), **correlation** (a candidate predicts measured transforms), or
**causal probe** (one structurally valid source change predictably changes
the black-box transform while unrelated geometry stays fixed). A rule needs
the exact source fields, units, origin, axis direction, orientation, rounding
and bounds. A lookup keyed by source ID, page number, image hash or observed
CTM is excluded.

## Hypotheses, in order

1. Check whether image order and checked pixel dimensions alone can explain
   the additional x/y translations. Fit only discovery draws and retain every
   attempted expression and its errors. The first-image page size and the
   300 dpi image scale are already pinned by #107; they are constraints rather
   than new placement evidence.
2. Treat page-row `+10`, `+12` and `+16` as unknown. Reuse the 12 #107
   one-byte results as negative observations. Do not assign coordinate
   meaning or run broad bit flips without an independently identified
   structure. A later field-specific test must state its exact offsets and
   validity checks before mutating a source.
3. Inventory bounded text-span or descriptor-adjacent metadata on the two
   discovery source pages HN-A 16 and C8 1. Record only lengths, hashes,
   candidate field offsets and structural checks. Search for independently
   framed numeric fields or commands; do not infer semantics from a coincident
   byte pattern. No raw text or source bytes enter Git.
4. Inventory type-2 JPEG marker metadata before entropy-coded scan data,
   including independently recognized density/APP fields. If a candidate
   field appears, specify whether a valid one-variable change is possible
   without changing image identity or pixel dimensions. Never mutate entropy
   data merely to force a PDF difference.
5. Construct original synthetic HN-A/C8 sources only if the independently
   measured container structure supports a bounded, valid document and the
   pinned black-box executable accepts it. Otherwise use temporary copies of
   the SHA-pinned discovery sources. No third-party converter code or
   pseudocode is consulted.

Freeze an exact probe list (source/page, field offset and width, new-value
rule, expected structural validity, predicted outcome) in this note before
running any new black-box mutations. The first batch may use at most 12
one-variable copies of the two discovery documents; additional batches need
a written reason and a new predeclared bound. Validate each copy against its
original byte by byte in 64 KiB chunks, permit only its declared changed
positions, and recheck source SHA-256 after conversion. Keep all copies and
PDFs outside Git and release artifacts.

The first causal-dependency batch is predeclared as four JFIF APP0 header
edits. Independent read-only marker checks found a baseline JFIF 1.01 APP0
segment whose two-byte JPEG length field has value 16
at the start of the second image on C8 page 1 and HN-A page 16, with unitless
1×1 density and no thumbnail. These are recognized JPEG metadata fields, not
inferred coordinate fields. The unit edit changes the single JFIF units byte
from 0 to 1; the density edit changes the low byte of big-endian Xdensity
from 1 to 2. Every other source byte must remain identical. The outcome is
unknown before the run; a PDF change does not itself establish an x/y rule.

| Probe | Source page/image | JPEG payload start | Field span | Absolute changed byte |
| --- | --- | ---: | --- | ---: |
| C8 units | C8 p1/i2 | 94,180 | `[94193, 1]` | 94,193 |
| C8 Xdensity | C8 p1/i2 | 94,180 | `[94194, 2]` | 94,195 |
| HN-A units | HN-A p16/i2 | 1,004,167 | `[1004180, 1]` | 1,004,180 |
| HN-A Xdensity | HN-A p16/i2 | 1,004,167 | `[1004181, 2]` | 1,004,182 |

Run each probe twice in fresh directories using the same pinned reference
environment and compare its PDF SHA-256, page count, MediaBoxes, image order,
stream hashes and ordered CTMs with the unmodified reference. The selected
JPEG's encoded-stream SHA-256 must change by construction; check that its
descriptor/order and pixel dimensions stay fixed and all other image streams
remain unchanged. Validate the mutated JPEG from SOI through APP0, SOF, SOS
and EOI before launching the converter. This batch tests
whether these two JPEG fields influence output; it does not test a rule
derived from the high-entropy text span.

## Read-only exploratory checks

The initial read-only analysis performed no new black-box conversion or
mutation. It verified the pinned HN-A/C8 source hashes before and after
bounded reads. Within each variant, all selected multi-image text spans have
distinct SHA-256 values. Their first 20 text bytes are constant across the
selected pages, but HN-A and C8 have different prefixes. Their Shannon
entropy is about 7.64–7.91 bits per byte and roughly 33–36% of bytes are
printable ASCII. No complete zlib, gzip or raw-deflate decode succeeded from
offsets 0, 20, 26, 28 or 32 under a 1 MiB output cap. This does not identify
the text codec or prove that coordinates are absent.

None of the 50 additional JPEGs has APP1–APP15 or COM metadata. Each has the
same JFIF APP0 segment (length field 16) before its scan, with version 1.01, unitless
1×1 density and no thumbnail. The 14-byte APP0 data after the length field
has SHA-256
`1fb2a1c85b30a2d812c0c54aa6662cee809224a2ee52ff70bfe57604a3e7a2c8`.
Therefore the observed APP0 fields cannot distinguish the 50 placements.
Bounded scans found no exact nonzero x/y coordinate representation in the
selected text spans as ASCII decimal, little-endian f32/f64, or little-endian
i32 fixed point at scales 1, 10, 100, 1,000 or 10,000; the scan covered PDF
points and 300 dpi pixels with top and bottom origins. This is a negative
check of those encodings only. None of the 20 discovery pages has a same-length
text-span partner within its variant for a simple whole-span swap.

Simple geometry-only controls also failed. At 0.001 pt tolerance over all six
CTM components, top-left, centered and bottom-right placement each matched
0/36 discovery and 0/14 validation additional draws. A separately fitted
variant-specific affine expression using image width, height and draw index
matched 1/36 discovery and 0/14 validation; that lone match is overfitting.
For example, HN-A pages 32 and 33 have similar supplemental JPEG sizes
(1,887×1,717 and 1,872×1,712 pixels), but their x/y origins differ by
25.2326/134.7028 pt. The first three additional draws on C8 page 1 move
upward rather than stacking downward. A possible coordinate lattice was
noticed only after inspecting all 50 outcomes; it is post hoc, has no source
field, and is not treated as validation evidence or a placement rule.

One bounded feasibility run assembled a 132,124-byte, one-page C8 temporary
source from the pinned C8 page-1 index row and its text/descriptor/image
chain, using at most 65,536 bytes per copy request. The row's absolute spans
were retained; unknown header space was zeroed. The independent source reader
confirmed the original text hash and five image hashes/dimensions, and the
pinned black-box converter produced a one-page, five-draw PDF accepted by
qpdf, MuPDF and Poppler. The temporary source/PDF hashes were
`13e4ff97b0969ceade7449455c59199fba30b0b7f31b0df384ca1b317746aa2b`
and `8d576e41d5888274593519ac64886b13884ca4a1bb86a760bdbc0b232578072c`.
The converted source reused text, image bytes and unknown row values from the
same pinned document, so this is a structural feasibility check and **not**
independent-document validation or a new placement-rule test. Both temporary
files were deleted after the run.

## Measurements and decision rule

For every requested input, pin and check the #22/#61 matrix, #107 oracle,
reference source/PDF hashes, Python checkout revision, package and native
library hashes, PDF tool hashes, command, environment and timeouts before and
after. A missing or changed explicit input fails; a clean clone reports
`NOT_RUN` and zero private comparisons. Capture subprocess output within a
fixed byte budget, kill/reap its process group on timeout or failure, and
measure peak ranged read, process RSS and temporary disk. Read one source
page at a time and keep bounded buffers.

Each probe report records its checked source ID, page/image, field span and
changed byte positions, original/mutant source SHA-256, conversion status,
PDF SHA-256 if produced, ordered image identity, MediaBox and all six CTM
components before/after, independent qpdf/MuPDF/Poppler agreement, and any
unsupported or skipped outcome. Private bytes, rendered pixels and generated
PDFs stay external. No skipped case counts as a pass.

A proposed rule must match all 36 discovery and 14 predeclared validation
additional draws within an explicitly stated numeric tolerance while also
matching image order/identity, scale and MediaBox. To claim a general rule,
require either two repeated, structurally valid one-variable causal probes
whose x/y change agrees with the prediction and leaves unrelated geometry
unchanged, or newly generated independent HN-A/C8 documents reserved until
the formula is frozen. Report exact supported variants, image types and
document counts. If no rule survives, report `UNKNOWN` with attempted,
passing, failing, skipped and unsupported counts. Keep #10 blocked and add
no guessed compositor.

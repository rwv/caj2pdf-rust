# Native C8 image-reference records

This records bounded parsing and independently controlled rendering for #242,
following #243/#244 and the delivered shared renderer #233. Complete-document
acceptance for the two additional profiles remains separate and unfinished.

## Observed framing

`810a/d300` differs from the older 28-byte `800a/d300` form:

| Offset in record | Field |
| --- | --- |
| 0 | `810a` tag, little-endian u16 |
| 2 | `d300` profile value |
| 4, 6, 8, 10 | Absolute x, y, width, height; unsigned words without `c000` markers |
| 12 | Flags; only zero is admitted |
| 14 | Reference byte length, unsigned u16, excluding NUL |
| 16 | Raw reference bytes, then NUL and zero padding to a four-byte record length |

The total record length is `align_up(16 + reference_length + 1, 4)`.
`NativeRecord::ImageReference` preserves coordinates and a source span for the
reference, excluding terminator/padding. It does not decode a pathname, open
an external file, or allocate a name buffer. The existing 28-byte buffer checks
the entire indexed span and padding before one event is delivered. Declared
image counts include both image-record forms.

Coordinates use the same absolute source units as the original image controls.
Subtract the document origin without a text-specific margin. Orientation must
follow the decoded image representation; no universal PDF transform is assigned
by the parser. Zero extents and other unusable geometry must be rejected by the
renderer, as with raw legacy coordinates.

## Independent controls

`tools/cajviewer/c8_image_reference_fixture.py` reuses the original MIT
asymmetric JPEG helper and generates 14 original inputs, with a following
glyph to expose lost record boundaries. Pinned offline CAJViewer 9.0.0 observations:

- Reference lengths 0, 3, 4, 5, 8 and 260 retain the same page pixels as the
  older image-record control. Length 260 independently exercises the length
  word's high byte. All repeat captures match.
- Separate +20 changes to x, y, width and height change the corresponding
  image position/extent by 75 screen pixels at displayed fit-width 2896%.
  The page frame stays `(449,231,1574,1093)`, with all four inner corners white.
- Two different original JPEGs display in descriptor order. Swapping reference
  names leaves the whole page identical; swapping descriptor payloads exchanges
  the visible images, leaving the following glyph unchanged. Repeats match.
  The core container reader independently validates both descriptor chains.

These observations establish the admitted framing and descriptor correspondence
for these controls, not universal format fidelity. The generator reproduces all
14 observed files byte for byte. No document content, font data or capture is
committed. The inherited original image helper is identical to the one used by
#235; there is one shared helper, not a second decoder or drawing framework.

External receipts remain in `caj2pdf-c8-size-scale-20261001`, including
`image-reference-fresh-comparison.json`, `image-reference-extent-measurements.json`
and `two-image-reference-chain-comparison.json`. An earlier run ended with the
viewer exiting and black screenshots; those trailing captures and its missing-NUL
probe are excluded. Initial two-image controls with an incorrectly constructed
descriptor table were also discarded; the checked controls use the established
interleaved descriptor/payload chain. These failed probes are not compatibility
passes or evidence that a real document is corrupt.

## Limits and remaining work

Unknown flags/profile values, truncated spans, nonzero terminators/padding and
image-count mismatches fail with source locations. Record/byte limits and
cancellation remain active during reference reads; any failure poisons the
cursor. Original Rust fixtures cover short reads, maximal u16 lengths, marker-like
name bytes, malformed padding, mid-payload failure/cancellation, and mixed old/new
image records. References remain opaque bytes and do not trigger resource lookup.

The actual additional C8 documents still require further control, drawing and
font semantics and complete-page rendering. Parser progress, descriptor traversal,
and independent original controls do not satisfy #242's 4-/5-page conversion or
CLI/Node/browser acceptance. The generator's existing-image baseline uses the
same codec and source-unit assumptions previously recorded under #233/#240;
full PDF rendering checks remain in those issues.

Direct probes at this parser revision accept 11 image-reference records in
unchanged SHA-pinned sources: three before the next unsupported record in
`4-[21]`, and eight in `4-[24]`. Their reference lengths are 29, 30 and 31 bytes.
The eight records cover all four indexed images on each of `4-[24]` pages 3
and 4. Later unadmitted controls still prevent complete page traversal, and
no full-document conversion pass is claimed. Receipts are
`4-[21]-direct-image-references.txt` and `4-[24]-direct-image-references.txt`
in the external native-profile inventory.

## Rendering integration

The C8 composer now routes `ImageReference` and the older image record through
one placement helper. It consumes already prepared document image resources in
descriptor order, subtracts the native origin, preserves source-unit extents and
uses the decoded representation's row orientation. The reference span is never
opened or used for resource lookup. Zero extents and skewed image state are
explicit errors; HN-B admission and image-after-text restrictions are unchanged.

Original Rust composer tests interleave text with two images, compare both row
orientations against the older representation, swap names without changing PDF
bytes, and reject zero dimensions/skewed placement. Existing parser malformed,
count, short-read and cancellation tests remain applicable. No new decoder,
name allocation, filesystem operation or image cache is introduced.

All 14 historical original controls reproduce byte for byte and pass actual
CLI/qpdf. Six name-length variants produce byte-identical PDFs to the old form;
the two-image name swap also preserves PDF bytes, while payload swapping changes
the result. Existing independent viewer receipts are reused rather than repeating
completed observations. Current external receipts live under
`caj2pdf-hnb-rendering-20261003/c8-image-reference-output/`.

Independent PDF inspection also verifies all four coordinate/extent changes
against the established 20-source-unit delta, and confirms that both original
JPEG payloads remain byte-exact and appear in descriptor order after a payload
swap (`geometry-checks.json`). This checks actual emitted resources and geometry,
not only agreement between two converter interfaces. Full corpus retry now
reaches page-2 byte 30176 in the four-page input (unverified glyph size); the
five-page input remains at page-2 byte 22528 (`a1de`). Neither publishes a final
PDF; these controls do not establish complete-document fidelity.

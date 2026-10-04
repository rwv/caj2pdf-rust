<!-- SPDX-License-Identifier: MIT -->

# Bounded image-only HN/C8 page composition

[Issue #117](https://github.com/rwv/caj2pdf-rust/issues/117) adds the opt-in
core `hnc8::convert_source_pages_pdf` API. It combines the checked container,
text framing, empirical geometry and image codecs in one `PdfDocument`.
CLI, browser and Node.js routing is implemented and validated in
[#10](https://github.com/rwv/caj2pdf-rust/issues/10). This API does not add
searchable text, type-1 images or general vendor layout
support. HN-A outlines can be requested with `include_bookmarks`; see the
[outline API and evidence](hnc8-outline-fields.md).

## Supported profile and source mapping

The HN-A/C8 path traverses the entire declared index from source page 1. Each
image-bearing page must have either the [uncompressed HN-A record profile](hnc8-uncompressed-text.md)
or one of the supported compressed text layouts: the tagged fixed glyph/tail
layout, or the [directly prefixed record stream](hnc8-direct-text.md). Each
compressed path requires a complete checksummed zlib frame and consistent
image records. The two header payload words may vary across documents; see
[compressed text framing](hnc8-compressed-text-header.md). Every image
must be type 0, type 2 or type 3. Since #184, declared header extents determine
the page box, and each image record's display extents determine its size.
Decoded pixel dimensions are independent; DIB storage padding is dropped by
the PDF writer. Zero extents are errors. See the
[source-geometry correction and remaining limits](cajviewer-hnc8-kdh.md#results-after-the-source-geometry-correction).
Raw position/extent words determine transforms in source descriptor order,
using the empirical `240 / 2473` points per unit; negative height,
fractional positions, overlap, repeated payloads and off-page draws are kept.
The coordinate factor is measured, not an authoritative physical source unit.
Unknown text profiles and unsupported draws are errors, never omissions.

The separately observed HN-B path accepts exactly one checked type-2 JPEG
on each image-bearing row. Its page box derives from JPEG dimensions, with
`W = f64(width) * 72.0 / 300.0` and `H = f64(height) * 72.0 / 300.0`; its
CTM is `[W, 0, 0, -H, 0, H]`. HN-B text is not
sent through the HN-A/C8 parser. Every HN-B row without an image produces a
visitor event with no PDF page and increments `no_image_pages`. The measured
six-row source therefore has the output-to-source mapping `[1, 6]`, with four
separately accounted no-image rows. This does not claim text conversion for
those rows. HN-B multi-image rows, other image types and a document with no
output image pages are unsupported. HN-A/C8 pages without images are refused
because this API cannot reproduce their text-only contents.

`ComposeVisitor::page` asynchronously borrows facts for the current source
row. Source and output page numbers are one-based. Successful image rows
include the ordered source descriptors, visible/display dimensions and all
six CTM components. A visitor can stream a mapping to a file or JavaScript
adapter without retaining an entire document map. `()` is a no-op visitor.
Verified repeated groups expose aliases rather than extra draws; see
[the grouping rule and API migration](hnc8-repeated-groups.md).

## Samples and orientation

Type-0 requires a separately supplied `QmTable`. JPEG-only documents need
neither that table nor a context allocation. Missing tables are located at
the first type-0 descriptor. The one reusable 1,024-entry context bank resets
for every image through the existing decoder.

The [row decoder](jbig1-type0-rows.md) emits top-first, MSB-first DIB-stride
rows, where bit 1 means black. It zeroes unused low visible bits and all DIB
padding. The PDF stream retains all DIB row bytes. Display width is the
visible width if `visible_bytes == dib_stride`, otherwise `dib_stride * 8`.
This distinguishes unused bits from extra padding bytes; page and image
geometry use the same rule.
This composer stores those rows bottom-first under a negative-height CTM.
That sample/transform convention must pass the predeclared full-reference
comparison; the older row-oracle check alone is insufficient. A caller-owned
random-access store reverses rows while decoding; its
bytes are then read forward into the padded bilevel XObject. The CTM remains
unchanged. JPEG streams are copied exactly, without sample reversal or
re-encoding; a whole-payload SHA comparison rejects changes after preflight.
The older selected-image converters retain their visible-width/top-first,
positive full-page orientation.

## I/O, storage and failure contract

The source is stable, seekable or ranged and output is sequential. A
forward-only source must first use a platform spool. Conversion holds current
page coordinates and image plans, then plans and PDF placements; these vectors
drop before the next page. Count times element size is checked before either
reserve, and actual capacities and their coexistence are checked afterward.
`ComposeBudget::max_page_metadata_bytes` caps these page vectors. Text decode
has its own `TextBudget`, including the locked backend reservation. The report
states their peaks separately; they are not process-memory measurements.

Each type-0 image preflights `stride * height` against
`max_row_store_bytes`. `max_row_store_io_bytes` bounds requested temporary
read/write lengths, including failed or short calls. Source, PDF, copy-buffer,
decoder and context allocations also honor their existing `Limits` and codec
budgets. Scratch decoding writes each padded byte once at its reversed row
offset; readback uses one I/O-sized buffer after the decoder's three rows have
dropped. This retains `O(current-page metadata + stride + contexts + I/O chunk)`
handler memory plus the PDF writer's bounded document indexes. The caller-owned
row workspace peaks at one padded bitmap; forward-input spools, staged output,
reference PDFs and diagnostic renders use separate platform/tool storage.
These bounds exclude allocator overhead, compiler
stacks, renderer memory and the PDF writer's documented small formatting
allocations; `Limits::max_allocation_bytes` is not a process RSS ceiling.

Scratch implements the existing `jbig2::text_composer::RandomAccessScratch`
contract. It has exclusive access, exact `set_len`, short positioned reads and
writes, and completed-write visibility on the same handle. Adapters belong to
their platform; the core opens no files. Every normally completed type-0 path
attempts truncation to zero and verifies the result, including failed
operations. If cleanup also fails, the primary error and cleanup error remain
available. A dropped pending future cannot await truncation. The owning
platform adapter must dispose of its temporary store and partial output using
its lifetime cleanup; the caller must not resume that session.

Every conversion error invalidates the whole partial PDF. `ComposeError`
contains source variant, page, image, absolute source offset and stage when
known, and retains typed inner errors. Short I/O advances only by accepted
bytes; zero progress, overreports, truncation, cancellation, source changes,
resource refusal, visitor failure and sink/store failure cannot become success.
Container metadata belongs to the same stable source; this API does not make
arbitrary concurrent index/payload rewrites safe.

## Verification and provenance

All new source and fixtures are original MIT code. No converter implementation
or private HN/JBIG module is copied or transliterated. Synthetic tests use invented records with valid format tags through the
public conversion API; there is no test-only prefix override. Documents, official QM states, opaque text prefixes,
decoded samples, PDFs and renders remain outside Git.

The optional external comparison follows the
[frozen, predeclared protocol](hnc8-page-composition-protocol.md) and
compares complete page metadata, all padded type-0 samples and every rendered
page against independently pinned references. A missing corpus is `NOT_RUN`
with zero compatibility passes. Synthetic I/O/layout success and selected JPEG
stream parity do not substitute for complete-page compatibility.

The [recorded complete-page evidence](hnc8-page-composition-evidence.md)
passes all 77 output pages, 74 padded Type0 arrays and 154 complete two-renderer
page comparisons on the revised native binary. HN-A/C8 use the pinned Python
references; HN-B uses an explicitly corrected Gray reference that preserves
all objects/streams except the two invalid legacy RGB declarations. All four
HN-B source/PDF Gray sample pairs also pass. The original legacy HN-B
comparison and historical failed attempts remain FAIL. The measured scope,
intentional legacy deviation and immutable receipts are recorded under
[child #122](https://github.com/rwv/caj2pdf-rust/issues/122). Official CAJViewer
image/text fixtures are a separate validation strategy. This API's production
family exposure remains gated.

## Type-3 source-page integration (#118)

The source composer reuses the selected-image type-3 decoder. It keeps only a
source digest per planned image, rechecks it before decoding and retains the
existing decoder source checks. `ComposeReport::type3_images` counts emitted
images; each visitor image exposes `type3_text_header_anomaly`. Strict text
headers remain the default. The named HN/C8 exception must be explicitly
selected with `options.type3.text_header_policy`.

For type-0/JPEG callers, passing `&mut scratch` still works. For type-3,
pass `ComposeWorkspaces { rows, type3: Some(ComposeType3Workspaces {
table, first, second, refined }) }`. All four stores implement the existing
`RandomAccessScratch` interface. The core creates serial live read/append
views of the three symbol stores; it creates no files, tasks or storage
factory. The platform adapter owns all backing storage. Stores are reset
before each image and all four resets are attempted after success/failure.
If both conversion and cleanup fail, both errors survive. Dropping a pending
future still requires the caller to dispose its stores and partial output.

`max_row_store_bytes` and `max_row_store_io_bytes` bound the aggregate type-3
stores for one image. The existing report fields include their aggregate
peak and successful physical reads/writes. Temporary backing may be files or
browser storage; these counters are not process memory measurements. Source
metadata, decoder contexts and bounded I/O buffers keep their separate limits.

The following #118 measurements predate #184, which removes displayed padding.
Type-3 decoding emits top-first visible-width rows. Whole DIB padding bytes
are streamed as white, with no second image bitmap. The equivalent transform
uses positive height and moves its origin down by that height. The observed
DIB display-width rule matches type-0: retain visible width when its packed
byte stride already equals `ceil(width / 32) * 4`; otherwise display all DIB
bytes. Original tests include repeated asymmetric type-3 images, mixed
0/2/3 pages, short store operations, failure cleanup, limits and cancellation.
An independent MuPDF render verifies every pixel of the original padded
first page; qpdf checks the two-page PDF.

### Actual external observation and remaining work

On 2026-09-29, the four type-3 images in external issue-58 (source SHA-256
`8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85`)
matched Python commit `8cbc3c5721acb762f739434eb3d206171dbb022a`'s extracted
image streams exactly after row reversal and white padding. Visible widths
2366/2352/2364/2352 become display widths 2366/2368/2364/2368.
The reference was run as a black box; its external JBIG2 dependency was built
outside the repository. No implementation source was inspected or imported.

The initial full-source Rust attempt failed at byte 160 on the directly
prefixed text layout. That concrete gap is now implemented; all four source
pages convert and pass qpdf. MuPDF matches 31,895,688 rendered pixels exactly.
Poppler has one-level grayscale differences attributable to equivalent row
orientation/CTM representations, confirmed with a separate control; it is
not recorded as exact equality. See [the protocol and results](hnc8-direct-text.md).
Selected real mixed pages now match an explicitly Gray-corrected reference;
see [repeated groups and migration](hnc8-repeated-groups.md). CAJViewer HN/C8
comparisons remain open.

### v0.x API migration

`ComposeOptions` adds `type3`; exhaustive struct literals must add
`type3: Type3PdfOptions::default()` (or use `..Default::default()`). Reports and
visitor images add the fields described above; update exhaustive patterns.
Existing ordinary function calls with `&mut scratch` remain valid. The
selected-image API, CLI and JS routing are unchanged. `options.type3`
provides decoder budgets/policy only; its selected-image `pixels_per_inch`
and `container` fields are ignored by source composition, whose geometry and
container limits use the existing source-page options.

The opt-in native example also accepts `--mq-table PATH` after its existing
arguments. The separately held MQ fixture remains outside Git; missing or
unresolved normative table provenance is not solved by caller injection.

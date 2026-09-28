<!-- SPDX-License-Identifier: MIT -->

# Bounded image-only HN/C8 page composition

[Issue #117](https://github.com/rwv/caj2pdf-rust/issues/117) adds the opt-in
core `hnc8::convert_source_pages_pdf` API. It combines the checked container,
text framing, empirical geometry and image codecs in one `PdfDocument`.
Production CLI, browser and Node.js format routing remains gated by parent
[#10](https://github.com/rwv/caj2pdf-rust/issues/10). This slice does not add
searchable text, outlines, type-1 images, type-3 mixed pages or general vendor
layout support.

## Supported profile and source mapping

The HN-A/C8 path traverses the entire declared index from source page 1. Each
image-bearing page must have the measured, SHA-fingerprinted text prefix,
complete checksummed zlib frame, record markers and image tail. Every image
must be type 0 or type 2. The first checked image determines the page box.
All raw coordinate words determine transforms in source descriptor order via
the [empirical placement rule](hnc8-placement-rule.md); negative height,
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

## Samples and orientation

Type-0 requires a separately supplied `QmTable`. JPEG-only documents need
neither that table nor a context allocation. Missing tables are located at
the first type-0 descriptor. The one reusable 1,024-entry context bank resets
for every image through the existing decoder.

The [row decoder](jbig1-type0-rows.md) emits top-first, MSB-first DIB-stride
rows, where bit 1 means black. It zeroes unused low visible bits and all DIB
padding. This composition profile keeps **all** `dib_stride * 8` samples in
the PDF image width. It does not crop to visible width and stretch the result.
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
or private HN/JBIG module is copied or transliterated. Synthetic tests use an
invented text-prefix digest through a private test seam; production profile
checks have no override. Documents, official QM states, opaque text prefixes,
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

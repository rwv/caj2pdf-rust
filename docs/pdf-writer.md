# Forward-only PDF writer

Issue [#5](https://github.com/rwv/caj2pdf-rust/issues/5) adds a narrow PDF 1.7
writer for converted page images and navigation outlines. It emits bytes only
through the core `SequentialSink`; the output handle does not need `Seek`.
The writer does not construct an in-memory PDF file or a general-purpose PDF
object graph.

## Serialization model

`PdfWriter` reserves generation-zero indirect object numbers, writes each
object once, counts accepted output bytes, and records one `u64` offset per
object. A stream whose length is unknown at its start refers to a later
indirect `/Length` object. Its payload is written in bounded chunks, then its
measured length is emitted after `endstream`. Binary payload bytes resembling
`endstream`, `endobj`, or `xref` are data; the reader uses the recorded stream
length to locate their boundary.

`finish` writes a classic cross-reference table, trailer, and `startxref`
after all objects. A classic xref entry has a ten-digit byte-offset field, so
this writer rejects files exceeding its 9,999,999,999-byte profile. It checks
object counts, positions, stream lengths, and total output limits before
serializing values that could overflow or truncate. The profile also caps each
stream length at 2,147,483,647 bytes and the reserved object count at
8,388,607, following the separate PDF 1.7 Annex C interoperability limits.
The offset index is subject to `Limits::max_allocation_bytes`, and total bytes
are subject to `Limits::max_output_bytes`. It returns typed errors; it cannot
roll back bytes already accepted by a caller-owned sink. A sink
failure poisons the low-level writer because the partial PDF is unusable.

The document builder streams page payloads from `RangedSource` into this
writer. It retains object offsets and bounded page/outline indexes, not page
image bytes or the resulting PDF. A fixed-fanout page tree keeps each `/Kids`
array bounded. Bookmark input arrives in document order; an outline depth
stack tracks parents and siblings without retaining every title. Each
read/write call uses at most the configured I/O chunk, capped at 1 MiB by the
core contract. The sink's awaited writes provide backpressure, and both source
reads and sink writes observe cancellation at I/O boundaries.

## Supported PDF profile

- PDF 1.7 header, ordinary indirect objects, a classic xref table, one trailer,
  a catalog, and a page tree.
- Image-only pages with caller-supplied dimensions. The initial image profile
  covers raw 8-bit grayscale/RGB samples and DCT-encoded JPEG in those color
  spaces. Three-component JPEG images explicitly use DCT `/ColorTransform 1`,
  matching the PDF 1.7 default for that component count; grayscale JPEGs use
  the default zero transform. The caller must establish the encoded JPEG's
  color interpretation, as the [HN/C8 selected type-2 diagnostic](hnc8-type2-pdf.md)
  does for its measured JFIF subset. `add_image_page` places one image across
  a new page; `add_image` emits a reusable image XObject without adding a page.
- Streamed 1 bpp images (`begin_bilevel_image`): `/DeviceGray`,
  `/BitsPerComponent 1`, `/Decode [1 0]` (a set bit is black), with source
  row padding beyond `ceil(width / 8)` bytes dropped. `add_page` places one or
  more finished images, each scaled to the whole page. The
  [HN/C8 type-0 note](hnc8-type0-pdf.md) and
  [selected type-3 note](hnc8-type3-pdf.md) record how the converters use them.
- Nested outline items with destinations to pages in the same document.
  Non-ASCII titles are serialized as UTF-16BE PDF text strings.

## Reusable images and affine pages

Issue [#116](https://github.com/rwv/caj2pdf-rust/issues/116) adds
`PdfDocument::add_image` and `add_placed_page` over the existing image and
page-tree emitters. A completed raw, JPEG or bilevel `ImageObject` may appear
multiple times on one page or on later pages. Reuse adds resource references
and draw commands, without reading or embedding its image bytes again.

```rust
use caj2pdf_core::pdf::ImagePlacement;

// `document`, `source`, image range/spec and page size belong to the caller.
let image = document.add_image(&mut source, offset, length, spec).await?;
document.add_placed_page(page, &[
    ImagePlacement { image, transform: [120.0, 0.0, 0.0, -60.0, 15.5, 100.25] },
    ImagePlacement { image, transform: [60.0, 0.0, 0.0, 30.0, -10.0, 5.0] },
]).await?;
```

Each six-component matrix `[a,b,c,d,e,f]` maps the image unit square to
PDF page coordinates: `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
Negative height, rotation, shear, fractional/negative translation and
off-page content are supported without clamping. Slice order is paint order;
isolated `q` / `cm` / `Do` / `Q` draws prevent transform accumulation.
Zero and singular matrices are intentionally accepted and can draw no visible
pixels. Matrix components must be finite, with absolute value at most
2,147,483,647, a named writer bound rather than a vendor layout guarantee.
They use shortest round-trip decimal syntax without exponent notation or
fixed-precision rounding; negative zero becomes `0`. PDF readers can have
lower numeric precision than Rust, especially at extreme magnitudes. Page
dimensions retain their existing 0.000001..=14400-point and six-decimal policy.

Image rows keep their existing convention: the first raw or supplied bilevel
row is the top of the image. A negative CTM changes orientation; this generic
writer does not reverse rows or contain HN coordinate factors. Format handlers
must establish their sample/CTM convention independently. These primitives
do not by themselves enable source-page HN/C8 conversion, which remains gated
by [#10](https://github.com/rwv/caj2pdf-rust/issues/10) and its children.

### Preflight, resources and failures

Handles retain `Copy`/`Clone`/`Eq` and privately carry a document identity.
Only a finished stream returns a handle. A checked process-wide pointer-sized
atomic counter supplies identities without wrap or reuse; exhaustion returns
a typed limit error before writing a document header. Foreign handles are
rejected even when their PDF object numbers coincide. No per-image registry
is retained. The existing documented same-document contract also applies to
`add_page`; its valid full-page behavior is unchanged.

`add_placed_page` accepts 1..=8,192 draws (`MAX_PAGE_IMAGE_PLACEMENTS`). It
checks every handle/matrix and the page dimensions, page count and required
object-index capacity before page emission, including page-tree rollover.
Capacity preflight does not reserve unused object numbers. Refused requests
leave output bytes unchanged and permit a corrected request. The legacy
`add_page` keeps its previous count/resource policy.

`add_image` checks exact raw length or nonempty JPEG length, the whole source
size, selected range and cumulative ranged-image read count against
`Limits.max_input_bytes` before emission. That cumulative check is specific
to the new method; `add_image_page` retains its prior input policy. Its single
reusable I/O buffer has a requested capacity at most
`min(length, Limits.io_chunk_bytes)` when first allocated, growing up to the
configured chunk for later images. All source/sink requests obey the chunk
ceiling and checked short-read/write helpers. Image/page output, object,
page-index and page-tree budgets use the existing writer checks.

The placement slice stays caller owned. Each matrix formatter has 2,117
bytes of inline capacity, and content/resource commands stream one draw at a
time. No complete image, placement copy or page-content vector is allocated.
The document retains the existing object-offset/page-ID indexes, bounded
page-tree groups, outline depth stack and one image buffer, plus one identity
and failure flag. `max_allocation_bytes` bounds individual dynamic requests;
it is not a bound on combined memory, allocator overhead, compiled future/stack
storage or process RSS.

An actual allocator, source, sink, output-limit or cancellation failure after
a new image/placed-page operation begins emission can leave a partial PDF.
Every subsequent document operation, including `finish`, then refuses: discard
the output and restart with a new document. The sink is never rolled back.
Preflight refusals and post-emission failures remain distinct typed errors.

This is intentionally smaller than a general PDF library. It does not edit an
existing PDF or emit fonts and selectable text, annotations, forms, embedded
files, encryption, signatures, transparency, optional content, object streams,
xref streams, incremental updates, or linearization. Those features require
separate design and tests before they can enter the output profile.

## Independent verification

The required native and coverage CI jobs install `qpdf`, MuPDF `mutool`,
Poppler `pdfinfo`/`pdfimages`, and libjpeg-turbo `cjpeg`.
`crates/caj2pdf-core/tests/pdf_validation.rs` writes new synthetic PDFs:
`qpdf --check` validates PDF structure, while MuPDF and Poppler independently
reopen pages, dimensions, images, and outlines. `cjpeg` encodes an original
tiny grayscale test image into a valid JPEG at test runtime; Poppler verifies
its exact PDF pass-through bytes, and MuPDF renders its decoded pixels. Missing
tools fail the test rather than making a skipped test look like compatibility
evidence. These executables run only in tests. The Rust conversion path does
not spawn a PDF program. The local versions used when adding the tests were
qpdf 12.2.0, mutool 1.25.1, Poppler 25.03.0, and libjpeg-turbo 2.1.5; CI
prints its installed versions.

The synthetic tests use original MIT test data produced at runtime. They do
not import files from the optional external CAJSamples corpus.

Issue #116's `pdf_placement_render.rs` additionally checks exact ordered
content matrices/resource references and unchanged JPEG streams with qpdf;
MuPDF and Poppler render original asymmetric bilevel/raw RGB/JPEG patterns,
positive/negative transforms, overlapping colors and reused objects.
`pdf_placement_bounds.rs` checks complete-request preflight, foreign handles,
work/input/object/output limits, bounded generated input, short/zero/invalid
reads, cancellation and failure state. These mandatory synthetic checks are
separate from optional private corpus compatibility; no external document
conversion is credited to this primitive.

## Specification sources

- [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf),
  Sections 3.2.7 (streams and indirect `/Length`), 3.4.3–3.4.4 (classic xref
  and trailer), 3.6.2 (page tree), 4.8 (images), 8.2.2 (outlines), and Annex C
  (interoperability limits).
- [Provenance register](provenance.md) records the exact rules implemented and
  the test-only independent validators.

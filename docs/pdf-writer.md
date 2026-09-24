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
  spaces. One image occupies each page.
- Nested outline items with destinations to pages in the same document.
  Non-ASCII titles are serialized as UTF-16BE PDF text strings.

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

## Specification sources

- [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf),
  Sections 3.2.7 (streams and indirect `/Length`), 3.4.3–3.4.4 (classic xref
  and trailer), 3.6.2 (page tree), 4.8 (images), 8.2.2 (outlines), and Annex C
  (interoperability limits).
- [Provenance register](provenance.md) records the exact rules implemented and
  the test-only independent validators.

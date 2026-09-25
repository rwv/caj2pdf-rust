<!-- SPDX-License-Identifier: MIT -->

# HN/C8 type-0 pages to PDF

This note covers the core slice of [issue #28](https://github.com/rwv/caj2pdf-rust/issues/28) that joins the [bounded container reader](hnc8-container.md), the [type-0 row decoder](jbig1-type0-rows.md), and the [forward-only PDF writer](pdf-writer.md). It is original MIT code. Its `QmTable` is **caller supplied**: the exact T.82 Table 24 states stay out of the source, tests, CLI, WASM, and packages while [#30](https://github.com/rwv/caj2pdf-rust/issues/30) is unresolved. The CLI and WASM engine therefore still reject HN and C8 input; a released converter needs a table source first.

## API

`hnc8::convert_type0_pdf(source, sink, table, options, limits, cancellation)` takes any `RangedSource`, a `SequentialSink`, a validated `QmTable`, `Type0PdfOptions`, the shared `Limits`, and a `Cancellation`. It returns a `Type0PdfReport` (the usual `ConversionReport`, the declared source page count, and the number of images) or a `Type0PdfError`.

`Type0PdfOptions` holds the output scale (`pixels_per_inch`, default 300), the multi-image policy (default `Reject`), and three budgets: the container `Budget`, the per-image `Type0Budget`, and an `ArithmeticBudget` applied to each image separately. The default arithmetic budget allows `max_pixels + max_height` symbols and 32 work units per symbol.

The row-level API from #55 is unchanged. `jbig1::read_type0_info` is new: it applies the decoder's span and 48-byte DIB checks without a context bank or sink and returns the width, height, and DIB stride. The converter uses it to write a PDF image dictionary before it constructs the decoder, which then rereads and rechecks those 48 bytes.

`PdfDocument` has two additive methods. `begin_bilevel_image(BilevelImageSpec)` opens a 1 bpp image stream and returns a `BilevelImageWriter`, which implements `SequentialSink`. Each input row has `row_stride` bytes; the writer keeps the first `ceil(width / 8)` bytes and drops the rest, so DIB 32-bit-aligned rows go straight in. `finish` requires exactly `row_stride × height` input bytes. `add_page(PageSpec, &[ImageObject])` then adds a page that draws each finished image over the whole page, in slice order. The existing `add_image_page` now uses the same page path.

## Conversion rules

Pages are read with `Hnc8Reader::open` from page 1, in index order, and emitted in that order. Each accepted image becomes one page.

| Source record | Behavior |
| --- | --- |
| Page with one type-0 image | One PDF page. |
| Page with several images, default `MultipleImages::Reject` | `MultipleImages(count)` error at the page row, before any descriptor is read. |
| Page with several images, `MultipleImages::SeparatePages` | Each image, in record order, becomes its own PDF page. No composition is attempted. |
| Page that declares no images | `NoImages` error at the page row. The text span is not decoded and no blank page is invented. |
| Image record type 1, 2, or 3 | `UnsupportedImageType(type)` error at its descriptor. These types have no measured codec assignment. |
| Unmeasured positive or negative type, malformed or truncated record | The container reader's typed error, with its own location. |

No placement geometry has been measured for a second image on a page, so the converter never composes images or guesses offsets. A failure stops conversion; no image is skipped, and no image can be attached to another page. The output already accepted by the sink is then an incomplete PDF that the caller must discard.

Text spans, the HN-A outline-like records, and the unknown page-row fields are not used. The output has no text layer and no bookmarks.

### Polarity, padding, and orientation

The accepted DIB palette is white at index 0 and black at index 1 ([#22 measurements](jbig1-oracle.md)), and the decoder emits those palette indices as bits, MSB first. The image XObject is `/DeviceGray`, `/BitsPerComponent 1`, `/Decode [1 0]`, so a set bit paints black and the stream carries the decoded bits unchanged. The PDF `/Width` is the DIB width, not `stride × 8`. Each PDF row is `ceil(width / 8)` bytes; the DIB's 32-bit alignment bytes are dropped. The unused low bits of a row's last byte are zero from the decoder, and PDF readers ignore them.

The decoder emits display-order rows. The first emitted row is the top of the page image, so the page draws the image with the ordinary `w 0 0 h 0 0 cm` matrix and no row buffer. This follows the #27 evidence summarized in the [bitstream investigation](jbig1-bitstream-investigation.md): a MuPDF render of a reference HN page equals the oracle's bottom-up memory rows reversed, and the Rust decoder's rows are that reversal. The synthetic tests below render asymmetric images and compare their top rows. A real-sample render comparison is still open, because it needs the corpus and the external table.

### Geometry

Each page is `width × 72 / pixels_per_inch` by `height × 72 / pixels_per_inch` points. The resolution is a caller choice, not a measured HN/C8 field; the DIB pixels-per-meter values are not interpreted. The PDF writer rejects a page dimension outside 0.000001 to 14,400 points with a located `Pdf` error.

## Errors

`Type0PdfError` carries the one-based source `page` and `image`, when known, and an absolute source `offset`. Container errors copy the reader's page, image, and offset. Image errors use the decoder's offset: a DIB field, a coded byte, or the span end. PDF errors use the image descriptor's offset. `Type0PdfErrorKind` separates invalid options, container, image, PDF output, context-bank allocation, unsupported type, multiple-image, and no-image failures. Nested errors are available through `std::error::Error::source`.

## Memory and I/O

The converter retains:

- the container cursor's fixed header, page-row, and descriptor buffers;
- three DIB-stride rows, 1,024 contexts, and the QM core's 256-byte buffer for the current image;
- the borrowed 113-state table;
- the PDF writer's one `u64` offset per object and one page ID per page.

None of these grow with the source size or an image's coded length. The per-page items are bounded by `Limits::max_pages` and `Limits::max_allocation_bytes`. Image rows go from the decoder to the sink one row at a time; the converter never holds a whole image, page, source, or PDF. Every read is a ranged read of at most `Limits::io_chunk_bytes`. A forward-only input must first be spooled by its platform adapter (see [I/O architecture](io-architecture.md)) or rejected.

A local release-mode probe (not committed) converted synthetic HN-B documents with 2,480 × 3,508 type-0 pages into a discarding sink. Peak child RSS was 10,236 KiB for 1 page, 10,276 KiB for 100 pages (109 MB of PDF), and 10,276 KiB for 1,000 pages (1.09 GB of PDF). The figure includes the test process and the in-memory synthetic source.

## Verification

[`tests/hnc8_type0_pdf.rs`](../crates/caj2pdf-core/tests/hnc8_type0_pdf.rs) builds C8, HN-A, and HN-B containers at run time. Its coded images come from an original test-only arithmetic encoder. The encoder follows the decoder's interval convention from the [arithmetic-core note](t82-arithmetic-core.md) and the row rule from the [row note](jbig1-type0-rows.md), and uses an invented adaptive 113-state table with conditional exchanges and MPS switches. The tests cover:

- widths 7, 8, 9, 31, 32, and 33 at heights 1 and 6, in all three layouts, with exact PDF stream bytes;
- a hand-specified width-9 image for bit order, padding, and top-to-bottom row order;
- multiple pages and page order, the multi-image policies, resolution scaling, and one-byte ranged reads;
- no-image pages, types 1–3, container errors, truncated spans, corrupt DIB fields, and impossible dimensions, each with its location;
- a sink failure at every write, cancellation at every check, and the shared, container, image, and arithmetic limits;
- the bilevel writer's split writes, stride checks, row-count checks, and unfinished-stream state.

`qpdf --check` validates the rendered cases. Poppler `pdftoppm -mono` and MuPDF `mutool draw` independently render every page, and the test compares the black pixels bit for bit. MuPDF renders at 72 dpi (one device pixel per image pixel). Poppler smooths a 1:1 bilevel blit, so it renders at 720 dpi and the test samples the centre of each 10 × 10 block. The local tool versions were qpdf 11.9.0, MuPDF 1.23.10, and Poppler 24.02.0. Missing tools fail the test.

These synthetic tests prove the integration and PDF encoding, not Table 24 compatibility. The optional #22 corpus comparison through this PDF path has not been run in a clean clone: it is `NOT_RUN`, zero cases, and not a PASS.

## Remaining #28 work

- Table 24 provenance (#30) and then CLI and browser/Node.js WASM wiring, including forward-only spooling for HN/C8 input.
- An opt-in corpus run through this PDF path that compares all 1,400 image hashes and renders selected HN and C8 pages, including multi-image pages.
- Measured placement for pages with several images, and any policy for pages without images.
- Codecs for record types 1–3.

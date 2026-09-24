# PDF input and repair profile

Issue [#6](https://github.com/rwv/caj2pdf-rust/issues/6) adds reusable PDF
operations for the CAJ-family converters. The PDF reader accepts a stable
`PdfRange` in a `RangedSource`; the writer uses a `SequentialSink`. Platform
adapters own file handles, browser `Blob` slices, HTTP range requests, and any
temporary spool for forward-only input.
`Limits.max_input_bytes` applies to the selected PDF range or the sum of
fragment spans, rather than unrelated bytes in a containing CAJ file.

## Supported input

- PDF 1.7 syntax with ordinary indirect objects, classic cross-reference
  tables, and incremental revisions linked by `/Prev`.
- Direct or indirect stream `/Length`; the reader skips exactly that many
  payload bytes before checking `endstream` and `endobj`. PDF-looking bytes
  inside a stream are payload, not structure.
- Catalog and page-tree links, declared page counts, and bounded object and
  page indexes. Page geometry accepts direct rectangles or indirect scalar
  rectangle objects. Encrypted input, xref streams, compressed object streams,
  and unrecognized structural damage return a located typed error.

Pass-through does not decode page content filters. The reader validates stream
framing and declared byte lengths, but it cannot establish that compressed
payloads decompress correctly. The independent validator checks the generated
and available corpus outputs exercised by tests; callers needing full payload
validation must use a PDF validator outside this conversion layer.

The reader recognizes two narrowly observed repair cases: a complete PDF
followed by a `WebFastLoadP` or `WebFastLoadW` footer, and identical duplicate
`/MediaBox` values in one `/Pages` dictionary. It verifies the original xref and object
graph before omitting the recognized footer. Unknown non-whitespace bytes
after EOF fail, including a truncated later incremental revision. It removes
the duplicate key by appending a replacement of that same indirect object
with one value. Conflicting values
or other duplicate keys are ambiguous and fail. The original page content
streams are not decoded or rewritten.

CAJ files that contain indirect object fragments but lack a complete PDF
header/xref use `FragmentPlan`: the CAJ format handler supplies complete,
nonoverlapping object byte spans and explicit page order. Reconstruction
validates those spans before writing a new PDF header and xref. It synthesizes
a missing Catalog or single missing Pages root only when the page-parent links
make the result unambiguous. A fragment Catalog with an existing `/Outlines`
link is rejected before output because this reconstruction path does not
validate an existing outline tree. The format handler can import independently
parsed CAJ bookmarks with `PdfOutlineAppender` after reconstruction. Indirect
fragment `/MediaBox` objects are also unsupported until the fragment plan can
validate their geometry. CAJ header, page-table, and TOC extraction belong
to [issue #7](https://github.com/rwv/caj2pdf-rust/issues/7); no end-to-end CAJ
compatibility is inferred from this PDF-layer test alone.

## Bookmarks and output

`copy_pdf` checks the PDF and copies its active bytes in bounded chunks. A
clean PDF is byte-identical at the destination. Recognized damage gets a
minimal incremental update. `PdfOutlineAppender` takes depth-first
`Bookmark` entries, checks each zero-based destination against the inspected
page order, and emits linked outline objects. It updates the existing Catalog
under the same indirect reference, preserving unrelated Catalog entries and
page streams. The output must be a distinct sink; a path-based caller should
stage output and rename it after `finish` succeeds.

If the input already has a nonempty outline tree, import preserves that tree
and reports zero imported bookmarks. A clean such input is copied byte for
byte. If recognized structural repair is needed, the repair still applies.
Unsupported encryption and PDFs with recognized certification or signature
indicators (`/Perms` or AcroForm `/SigFlags`) are rejected before the writer
claims success. The importer retains a bounded depth stack and emits
closed bookmark objects as entries arrive; it does not retain all titles.

On a source, sink, cancellation, or limit failure, the operation returns an
error and never a successful report. A sink may already contain a partial
PDF, so callers requiring an atomic path must use a temporary file. Required
tests reopen outputs with `qpdf` and MuPDF, compare page renders, and inject
sink failures. The optional external CAJSamples corpus is reported separately
and a missing corpus is never counted as a compatibility pass.

## Optional PDF-body corpus observation

On 2026-09-24, the two locally available PDF-body inputs at
`issue-33/test2.caj` and `issue-33/test3.caj` matched the SHA-256 values in
the [conformance matrix](../tests/conformance/matrix.json). With `qpdf`
12.2.0 and MuPDF 1.25.1, the Rust PDF layer normalized both outputs with a
clean `qpdf --check` result. MuPDF PNM renders at 36 dpi matched the input
page by page: 26/26 and 11/11. This measures only the PDF-body repair path;
the CAJ container conversion path is an issue #7 task. Neither the documents
nor the generated outputs are committed here.

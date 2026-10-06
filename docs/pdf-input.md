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
  tables, and incremental revisions linked by `/Prev`. The bounded xref-stream
  subset accepts direct `/Size`, `/W`, `/Index`, and `/Length`, unfiltered or
  `/FlateDecode` data, and ordinary free or uncompressed in-use entries. A
  later classic table may point back to an xref stream through `/Prev`.
- Direct or indirect stream `/Length`. In a complete PDF the xref resolves
  an indirect length, and `endstream` must follow the declared extent. A
  headerless CAJ fragment has no xref; its extent rule is described under
  [Stream extents in CAJ fragments](#stream-extents-in-caj-fragments).
  PDF-looking bytes inside a stream are payload once the extent is fixed;
  the reader does not re-parse them.
- Catalog and page-tree links, declared page counts, and bounded object and
  page indexes. Page geometry accepts direct rectangles or indirect scalar
  rectangle objects. Encrypted input, type-2 compressed object entries,
  object streams, unsupported xref filters or predictors, and unrecognized
  structural damage return a located typed error. This profile does not claim
  object-stream support because the observed KDH xref streams contain only
  type-0 and type-1 entries; compressed objects need a separate bounded reader.

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

The KDH PDF-body profile also normalizes a `stream` keyword followed by one
carriage return. After validating the referenced object and its declared
stream length, the copy path changes that one CR byte to LF; byte offsets and
stream payloads are unchanged. A stale `/Parent` on a `/Page` can be replaced
only if the reference is the sole dangling reference in that page and the
validated page-tree `/Kids` traversal identifies a unique actual parent. The
replacement is an incremental object update. Short incomplete object prefixes
in one observed input are permitted in otherwise whitespace-only gaps only
when the named object is free or is the immediately following live object.
Their exact bytes are checked during copying and replaced by the same number
of spaces. Other gap content remains an error. These repairs do not accept
unrelated dangling references, duplicate page-tree children, or cycles.

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

The CAJ converter scans the fragment once (issue
[#359](https://github.com/rwv/caj2pdf-rust/issues/359)). Each object's
inspection (span, role, references, page parent and whether it has a
`/MediaBox`) is carried through page-tree synthesis, link repair and
reconstruction instead of parsing the object again. Link repair reads only
the objects it rewrites. Generated objects, such as a synthesized `/Pages`
node, a repaired link or a blank page, are inspected from their own bytes.
Reconstruction checks span overlap and duplicate object numbers in one
place. Its memory is bounded by the object, page and bookmark counts
(`MAX_PDF_OBJECTS`, `Limits::max_pages`, `Limits::max_bookmarks`) and by
`Limits::max_allocation_bytes` on each buffer, not by a summed byte budget.

## Stream extents in CAJ fragments

The fragment scanner never decodes a stream payload to frame it:

1. A direct `/Length`, or an indirect one whose integer object was already
   scanned, is trusted when `endstream` and `endobj` follow it.
2. An indirect `/Length` whose integer object comes later is resolved by a
   forward search: the stream provisionally ends at the first `endstream`
   with a complete tail. The integer object must frame the stream at the
   same end; if it frames another end, the scan repeats with that length.
   If the scan fails while a provisional end is still unconfirmed, it
   repeats with the search moved past that `endstream`. At most 16 repeats
   are made; then the first error stands. A length whose integer object
   never appears is an error. Partial (`--allow-damaged`) and page-row
   candidate scans do not repeat.
3. Otherwise the stream extent is not confirmed, and recovery decides.

## Damaged-input recovery

All recovery rules sit behind one entry, `try_recover` in
`pdf/input/recovery.rs`, which the scanner calls when an object does not
parse or its stream extent is not confirmed. A rule either resumes at an
independently derived boundary, or defers the interrupted bytes until the
complete scan proves them an exact proper prefix of the indexed object with
the same number. A deferred prefix without that proof is an error.

- An interrupted stream followed by its complete replay: an `endstream`
  after the interrupted stream, less at most two end-of-line bytes and the
  declared or resolved `/Length`, fixes where the replay starts, at most
  64 KiB later. The replay must repeat the stream header, and the
  interrupted bytes must share at least one payload byte with it. Complete
  objects may sit between the two; exact duplicates are kept once.
- An understated direct `/Length`: the one complete terminator within 64
  bytes of the declared end is accepted, and the `/Length` digits are
  replaced in the copy by a same-width value. A final stream repaired this
  way must be the only terminator left in the scanned bytes.
- A syntax interruption: an exact prefix of an earlier object, of an integer
  replay, of a later copy offered by a page-table row, or a short cut at the
  parser's error boundary, under the bounds the rules document in code.

With `--allow-damaged` (below), a failure no rule recovers is recorded and
the scan resumes after an independently framed stream or at the page
table's next page dictionary.

## Header offset

Issue [#300](https://github.com/rwv/caj2pdf-rust/issues/300): when no CAJ,
KDH, HN, C8, or TEB signature starts at byte 0, `detect_source` also selects
PDF if the whole five-byte `%PDF-` marker lies within the first 1,024 bytes
(`PDF_HEADER_SEARCH_BYTES`). Observed leading bytes are a newline, a UTF-8
byte-order mark, and a junk line. Only this search reads past the first
five bytes, in reads of at most `io_chunk_bytes`. The other families still
match only at byte 0.

For a header at offset H, the CLI and WASM engine read the PDF through
`PdfRange { offset: H, length: size - H }`. Cross-reference, `startxref`, and
`/Prev` offsets are therefore relative to the header, which is correct for a
complete PDF with bytes prepended; `qpdf --check` 11.9.0 accepts such files
without reconstruction warnings. Offsets counted from byte 0 are not tried
as a fallback, so a file written that way fails as malformed PDF. The leading
bytes are not copied: the output starts at `%PDF-` and equals converting the
input without them. `Limits.max_input_bytes` applies from H. A JavaScript
caller that sets `format: "pdf"` skips detection, so it still requires the
header at byte 0.

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

## Optional KDH PDF-body check

The ignored `kdh_pdf_external` test is `NOT_RUN` in normal test runs. Supply
three independently decoded, trimmed PDF bodies named `issue-21.pdf`,
`issue-34.pdf`, and `issue-48.pdf` in one external directory, then run:

```sh
CAJ2PDF_KDH_PDF_DIR=/path/to/decoded-pdfs \
  cargo test --locked -p caj2pdf-core --test kdh_pdf_external -- --ignored --nocapture
```

At the pinned corpus revision used for issue #36, the local check returned
`qpdf --check` exit 0 for all three normalized outputs and matching 36-dpi
MuPDF PNM hashes on all 74 pages (6 + 67 + 1). This is PDF-body evidence;
the KDH wrapper and decryption path remain issue #11 work. The same outputs,
copied to the matrix-mapped paths outside Git, also returned inventory
`PASS` 3/3 and PDF `PASS` 3/3 from the corpus runner
`conformance.py --only-format KDH --pdf-dir ... --corpus-dir ...` (now in
[caj2pdf-samples](https://github.com/rwv/caj2pdf-samples/tree/main/research/scripts/conformance.py)):
all page counts, dimensions, outlines, and 74 rendered-page hashes matched
the pinned matrix. The first attempt used external symlinks and was rejected
by the harness's path-safety check; the reported pass used copied files.

## Explicit partial conversion of damaged CAJ inputs

`--allow-damaged` (JavaScript `allowDamaged: true`, Rust
`ConversionOptions::allow_damaged`) permits partial output for CAJ containers
with malformed embedded PDF objects. It is off by default. It does not add a
partial mode for PDF, KDH, HN, C8, or encrypted inputs.

After the existing bounded reconstruction fails, the partial path resumes at
an independently framed stream end or the page table's next page dictionary.
A table boundary can precede that dictionary by at most 64 bytes; its object
number and complete Page dictionary must agree with the table. This boundary
is used to discard content, never to claim lossless repair. No whole-document
buffer, marker search across the file, or external converter is used.

A dependency pass identifies pages affected by missing or malformed objects,
including shared fonts and images. Their contents, annotations, and resources
are replaced with an empty resource dictionary. Page object IDs, page order,
parentage, available page boxes, rotation, and bookmark targets are retained.
A shared damaged resource can require many blank pages; this mode does not
promise to recover every page a viewer can display. Missing or ambiguous page
geometry/boundaries, unsupported features, I/O errors, cancellation, and
resource-limit failures still abort conversion.

The CLI commits a valid PDF and returns **3** when it substituted any page.
Each warning identifies the one-based page and absolute input byte offset;
`--quiet` does not suppress these warnings. No substitutions means exit 0.
Rust's `omitted_pages` and JavaScript's `omittedPages` contain zero-based page
indices and absolute input offsets. JavaScript resolves with that report;
callers must check it before treating the output as complete.

Default corpus failures remain failures. An explicitly partial PDF is not a
successful lossless compatibility result.

### Pinned damaged-input observations

The five damaged CAJ entries in the pinned external corpus were checked on
2026-10-05. Each remained a strict-mode failure with no committed output.
Explicit partial mode returned 3, passed `qpdf --check`, retained every source
page object ID in table order, and had no Contents or Annots plus an empty
Resources dictionary on every reported blank page.

| Corpus case | Total pages | Blank substitutes |
| --- | ---: | ---: |
| issue-20 | 63 | 62 |
| issue-25 | 78 | 11 |
| issue-39 | 80 | 4 |
| issue-85 Mingtang | 234 | 15 |
| issue-90 `4-[6]` | 60 | 60 |

In particular, the current mode retains no page content for issue-90; a valid
PDF structure is not evidence of useful recovery. `pagesConverted` includes
blank substitutes. All omissions are reported even when every page is blank.
The other 12 pinned CAJ files retained their v0.4.0 output SHA-256 hashes under
both default options and `allowDamaged`. These are structural/hash checks,
not a new whole-document CAJViewer pixel comparison. Original corpus bytes
and resulting PDFs remain external to Git.

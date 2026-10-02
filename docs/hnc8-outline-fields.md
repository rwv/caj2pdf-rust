<!-- SPDX-License-Identifier: MIT -->

# HN-A outline fields: observed profile

HN-A field validation (#137), native outline integration and the two selected
complete-document checks (#119) are complete. The investigation and implementation
chronology below preserves the earlier failures and their later resolutions;
see [Native reader and PDF integration](#native-reader-and-pdf-integration).
C8/HN-B outline support remains unverified and is tracked separately by
[#221](https://github.com/rwv/caj2pdf-rust/issues/221).

## Method and results

Reference: Python converter commit `8cbc3c5721acb762f739434eb3d206171dbb022a`,
used only as an external black box. Tools: Python 3.13.5 (`gb18030`),
qpdf 12.2.0 and MuPDF 1.25.1. Existing original `parse_qpdf_outlines` and
`compare_outline_parsers` functions compared complete ordered outlines,
including exact UTF-8 title identity, depth, resolved page and nullable view.
MuPDF queried the object IDs selected by qpdf, then independently checked their
links, values and page references; object selection was not independent.

| Additional HN-A source | SHA-256 | PDF pages | Matching entries |
| --- | --- | ---: | ---: |
| issue-29 | `ede5eddb0e8ec1dea46c32a06e16ac12b874a141ca2d6669736495d2ac549261` | 48 | 48/48 |
| issue-69 | `57a3c60e1d8639955c625452398d2c8a32a46a51b815a44cc9a5e0351fcb8ef4` | 81 | 111/111 |

The candidate predicted all 159 entries. qpdf and MuPDF agreed independently.
Both source hashes were unchanged afterward. The ordered fingerprints are
`de3a95a5443d7ff10fdda940749b880f619b81b878770fe3a996bd0f530a6685`
and `793c7e4833032db9c94785116e8bcf97458e6185ffb7899a56eda971a2a4810c`.
An initial diagnostic used `XYZ` instead of the canonical `/XYZ` label; that
serialization error and its failed comparison are retained externally.

## Fields and admitted input

HN-A framing was already established: count at `0x158`, records at `0x15c`,
308 bytes per record, followed by the page index. Record-relative fields:

| Bytes | Supported interpretation | Evidence / policy |
| --- | --- | --- |
| `0..256` | Title through the first NUL, decoded strictly as GB18030 | 159 baseline matches; original ASCII, two-byte Chinese and four-byte emoji controls; 255-byte title plus NUL works. Empty titles work. Bytes after NUL are ignored. |
| `256..280` | Unknown | Nonzero in one baseline; do not require zeros or interpret as title data. |
| `280..292` | NUL-terminated ASCII decimal, one-based physical source page | Baselines, a changed destination and an 11-digit zero-padded value followed by NUL all agree. Leading zeros are permitted. Require `1..=source_page_count`. |
| `292..304` | Unknown | Nonzero in one baseline; preserve/ignore. |
| `304..308` | Positive little-endian level, one-based preorder depth | Baselines use 1–4. A legal 1-to-2 change produces exactly one deeper entry. Setting any higher byte to 1 makes the reference fail. |

For the native profile, require a NUL in each title/page field. Reject malformed
encoding, missing terminators, invalid decimal bytes and out-of-range pages.
Require the first level to be 1, positive levels within the configured depth cap,
and no increase greater than one. Interpreting the four level bytes as `u32`
is sufficient within this positive bounded profile; arbitrary signed values
outside it are not established as valid format values.

The encoding evidence supports reusing the repository's original GB18030 decoder,
whose mapping was independently generated from the same Python codec version.
It does not establish every historical GB18030 revision's behavior. Do not reuse
the CAJ record parser: its padding and empty-title policies differ.

All observed output destinations are `/XYZ [null, null, null]`. No source view
field was identified. Preserve this observed output default in #119 instead of
silently emitting `/Fit`. Map physical source pages through the actual emitted
page map; neither an empty outline nor this HN-A experiment justifies retargeting
a bookmark aimed at an omitted HN-B page.

## Targeted controls

Fifteen copied-input controls followed written byte edits and predictions.
Only the declared record spans changed; no checksum updates were needed for
the successful controls. This does not prove that every HN variant lacks checksums.

| Control | Actual result |
| --- | --- |
| 255-byte ASCII title plus NUL | Exact predicted title; other entries unchanged. |
| `Rust 中😀` title | Exact predicted Unicode title. |
| Embedded NUL followed by nonzero padding | Stops at NUL; padding is ignored. |
| Empty title | Retains the entry with an empty title. |
| Page 5 changed to 6 | Exactly one resolved destination changes. |
| Eleven decimal digits plus NUL | Leading-zero value resolves to page 6. |
| Legal level 1 changed to 2 | Exactly one entry moves to depth 1. |
| Destination 49 in the 48-page document | Converter exits 1. |
| Invalid GB18030 sequence | Converter exits 1. |
| Level bytes 305, 306 or 307 set to 1 (three runs) | Each converter exits 1. |
| 256 title bytes without NUL, followed by a sentinel | Prediction disproved: reference returns only 255 title bytes. |
| Twelve page digits without NUL, followed by a sentinel | Prediction disproved: reference drops the final digit; zero then resolves to the last PDF page. |
| Level jump from 1 to 3 | Prediction disproved: reference succeeds and emits depth 1, effectively collapsing the gap. |

The last three malformed-input behaviors are recorded compatibility differences.
The native reader should reject them explicitly under the profile above. Do not
silently reproduce truncation, zero-to-last-page mapping or hierarchy collapse.

Totals: two successful baseline conversions; fifteen control conversions,
ten producing PDFs and five failing. Both parsers agreed on all twelve produced
PDF outline lists. Seven positive controls matched their complete predictions;
three successful conversions contradicted the predictions as listed above.
No native conversion, image comparison or browser/Node compatibility was run.

## Bounds and implementation handoff

Each reference conversion used a separate directory, a 120-second timeout,
1 GiB address-space limit and 64 MiB file limit. Source copies/hashes used
64 KiB chunks; record observations read 308 bytes at a time. PDF queries had
15-second timeouts and were validated by the existing bounded parsers. These
are investigation limits, not measurements of Rust conversion memory.

#119 should expose a ranged record visitor with located errors and cancellation.
Honor `Limits.max_bookmarks`, bound decoded titles to at most 1,020 UTF-8 bytes
for the 255-byte encoded profile, and use a configurable depth cap (initially 64).
Retain preorder ancestry in O(depth) state plus a separately bounded source/output
map. Stream entries into the existing PDF outline API; do not retain all titles
or decode complete pages to recover bookmarks. Test short reads, count/offset
overflow, empty/Unicode titles, invalid encoding, missing parents, invalid or
omitted destinations and cancellation. Add an explicit nullable XYZ destination
to the existing writer and validate targets with independent PDF tools.

External evidence directory: `caj2pdf-outline-validation-20260929`. Its summary
SHA-256 is `099141cc7a2a66da0487d65194f54b93fbbf9c1f66386e1f1095623f9e993d01`;
it records source/output/script/control identities and actual results. Source,
mutated documents, copied titles, PDFs and query output remain external. No
converter implementation was read or copied. Existing original outline-parser
tests passed (42 tests); no runtime format implementation is introduced here.


## Native reader and PDF integration

`Hnc8Reader::visit_bookmarks(max_depth, output_pages, map_page, visitor)`
reads one 308-byte record at a time. The mapping takes a one-based physical
source page and returns a zero-based emitted page, or `None` for an omitted
page. Invalid/omitted destinations fail explicitly. Failure or cancellation
poisons the reader so partial visitor output cannot accidentally be replayed.

`ComposeOptions::include_bookmarks` opts into HN-A outlines after page output;
it defaults to `false`. The current HN-A composer emits every source page in
order, so its mapping is identity minus one. C8/HN-B opt-in fails before output.
`PdfDocument::add_bookmark_with_view` accepts `BookmarkView::Xyz`, emitting
`/XYZ null null null`; existing `add_bookmark` callers retain `/Fit`.

**Unstable API migration:** explicit `ComposeOptions` struct literals must add
`include_bookmarks: false` (or use `..Default::default()`). Set it to `true` to
request outlines. A conversion failure invalidates the partial PDF.
The diagnostic example accepts an optional final `--bookmarks` argument.
CLI and JavaScript family routing are separate work.

Original synthetic tests cover record fields, Unicode/empty titles, nesting,
page remapping, bounds, short reads, cancellation and visitor failures. qpdf
checks an emitted synthetic PDF's titles, hierarchy, destinations and views.
A composition test confirms enabling outlines preserves its JPEG stream.

External native outline-only PDFs with placeholder pages matched all 159
reference entries above through qpdf and MuPDF, including target pages and
nullable XYZ views. These PDFs validate the outline path, not page content.
Full composition attempts for both sources failed on page 1 at the existing
text-prefix profile check (source offsets 16092 and 36156). They are failures,
not skipped passes. Further page-profile support and complete-page comparison
remain necessary before claiming complete conversion for these sources.
Results and failed diagnostic logs remain outside Git under
`caj2pdf-native-outlines-20260929`.


Follow-up #163 replaces the document-specific compressed-header fingerprint
with structural validation. Issue-29 now completes all 48 pages and bookmarks,
including 96/96 exact 300-DPI page comparisons across MuPDF and Poppler; see
[compressed text framing](hnc8-compressed-text-header.md). The earlier failed
attempt remains recorded. Issue-69 uses a different uncompressed-text profile
and is still unsupported by composition; #119 remains open for that work.


The [uncompressed HN-A follow-up](hnc8-uncompressed-text.md) now completes
issue-69's 81 pages and 111 bookmarks. It corrects a type-0 display-width rule
and records the original reference's sixteen invalid JPEG color declarations.
All 162 complete-page comparisons pass against a separately corrected Gray
reference; historical failures remain failures. Together with issue-29, both
selected positive outline cases now pass through actual composed pages.
C8/HN-B outline semantics and official-viewer parity remain unproven.

# CAJ container observations

This note records format facts for [issue #7](https://github.com/rwv/caj2pdf-rust/issues/7).
The public [CAJ/HN overview](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E5%92%8C-HN),
[header and outline notes](https://github.com/caj2pdf/caj2pdf/wiki/%E6%96%87%E4%BB%B6%E5%9F%BA%E6%9C%AC%E4%BF%A1%E6%81%AF%E4%B8%8E%E5%A4%A7%E7%BA%B2),
and [CAJ page-content notes](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9)
are observations, not a complete specification. The measurements below were
made directly from local files matching the SHA-256 values in the
[external corpus matrix](../tests/conformance/matrix.json). The corpus is at
CAJSamples commit `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`; the
Python reference results use converter commit
`8cbc3c5721acb762f739434eb3d206171dbb022a`. Neither project supplied
implementation source for the Rust parser. No external document, derived PDF,
or outline text is stored in this repository.

## Header, table, and outline fields

All multibyte integers in the observed CAJ header and records are little
endian. Offsets and lengths below are absolute file byte positions unless
identified as relative to a record.

| Location | Observed field | Evidence and meaning |
| --- | --- | --- |
| `0x00` | `CAJ\0` signature | The [public format notes](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E5%92%8C-HN) distinguish CAJ from HN by the `CAJ` prefix. A `.caj` filename alone is insufficient: two `issue-33` corpus files are embedded PDFs. |
| `0x10` | 32-bit page count | Agrees with the reference source count in all ten successful CAJ cases below. |
| `0x14` | 32-bit page-table pointer | Points to `page_count` contiguous 12-byte rows in each observed case. |
| `0x110` | 32-bit outline-record count | Immediately followed by fixed 308-byte outline records starting at `0x114`. |

Bytes `0x04..0x08` are `01 00 02 00` in all ten successful CAJ cases. Their
meaning is unknown; this observation does not establish a version field.

Each observed page-table row has three 32-bit fields: absolute byte offset at
`+0`, byte length at `+4`, and page object ID at `+8`. In all ten successful
CAJ cases, the first row offset equals `page_table_pointer + 12 * page_count`
and starts with a PDF indirect object header. Adjacent rows are contiguous:
`row[i].offset + row[i].length == row[i+1].offset`. Five cases contain
zero-length rows. Their page IDs must not be discarded merely because the row
has no bytes. A row can begin within the preceding PDF object's terminator;
the rows are container spans, not guaranteed object boundaries.

The final row's end is a **hint**, not a proven PDF-body endpoint. In
`issue-20`, it is seven bytes before the end of the final stream payload. In
`issue-93`, 6,632 additional binary bytes follow it. Determine the end of a
valid PDF fragment from checked PDF structure and container bounds, not by
truncating unconditionally at the last row.

An outline record occupies 308 bytes:

| Relative bytes | Field | Observed interpretation |
| --- | --- | --- |
| `0..256` | Title | NUL-terminated, zero-padded encoded text. Public notes call it GBK; the Rust decoder supports strict GB18030, including the observed GBK two-byte repertoire. |
| `256..280` | Unknown | Preserve/ignore without requiring zero. All 93 records of `issue-20` have nonzero data here. |
| `280..292` | Page | NUL-terminated, zero-padded positive ASCII decimal. It is a **one-based** output page destination. |
| `292..304` | Unknown | Preserve/ignore without requiring zero. All 93 records of `issue-20` have nonzero data here too. |
| `304..308` | Level | Signed 32-bit integer. Observed levels start at 1 and represent increasing outline depth. |

Across the ten successful CAJ cases, all 603 titles decode strictly as GBK,
are nonempty, and have only zeros after their first NUL. No four-byte
GB18030 title occurs in this corpus; such decoding requires original synthetic
tests. The longest encoded title is 60 bytes, and the longest decoded title is
39 Unicode characters. All 603 page fields are unsigned decimal digits
followed only by zeros: 24 one-digit, 571 two-digit, and eight three-digit
values. They fall within the header page count; none uses zero, a sign,
whitespace, or a leading zero. Observed levels are 1–3 with no skipped level.
Repeated titles are meaningful records: `issue-20` has 18 repeated-title
occurrences on different pages, and its reference PDF retains 93 outlines.

## Successful CAJ sample measurements

The short sample labels below map to canonical paths in the
[matrix](../tests/conformance/matrix.json). “Remainder” is file size minus the
last page-table row's end, not a claimed amount of safely discardable data.
All ten have a contiguous page table and a first row immediately after it.

| Sample | SHA-256 | Pages | Source TOC / reference PDF outlines | Table pointer | Zero-length rows | Remainder |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `issue-2/a` | `bd7987d20537ba0265bc5eccb953a6c65a413f3954d27e59375ddcd748861b99` | 62 | 52 / 52 | `0x766c` | 0 | 0 |
| `issue-2/b` | `86529df25497bcf787505f60f7364e359796070a54a9be85f5874d8e6b090f91` | 78 | 58 / 58 | `0x5abc` | 0 | 0 |
| `issue-20` | `5a4432ed4878944c4aaa17f591b2a93d00014ea0bdacc9162bed1d88f8d61127` | 63 | 93 / 93 | `0x90f8` | 0 | 228 |
| `issue-40` | `633210e8e9faf05b677479d37709dd99a56c1b5ff817dfcfbbb350d5617c1d7b` | 78 | 40 / 40 | `0x1b71c` | 32 | 0 |
| `issue-44` | `fd89f649b4c70727255a76744e71a66c6621a1a2e81fd589f15840a107b1a135` | 108 | 57 / 57 | `0x29670` | 74 | 0 |
| `issue-46` | `5fe702bae7cf321178e85555bfc6e8b3a8b9f9074f5bf5d24ae2377b9638b5a8` | 109 | 38 / 38 | `0x7b8c` | 0 | 0 |
| `issue-49` | `964975c2d3269787be46de97ee0e574414d0e937de8a4190a923be0d02635c01` | 65 | 49 / 0 | `0x23c08` | 32 | 0 |
| `issue-73` | `fc8a5c20626b3492aad9afae62709019f2038a8a4c669bc7c94ae653837b0cba` | 84 | 100 / 0 | `0x957c` | 61 | 0 |
| `issue-88` | `88b47b4d6fd687f8393d3b9e0aad2389873c5a9cef0f9eeb4cf0bbfa32d08a52` | 110 | 65 / 65 | `0x6690` | 0 | 0 |
| `issue-93` | `446ccbf126afb94d3559c87f0dc2d684ffa6610bfe8c5803ffe5cd4797ff7105` | 63 | 51 / 51 | `0x14af8` | 35 | 6,632 |

The source/reference outline difference for `issue-49` and `issue-73` is an
observed reference conversion failure. At Python converter commit
`8cbc3c5721acb762f739434eb3d206171dbb022a` with PyPDF2 1.26.0,
`show` reports 49 and 100 source records. `convert` prints a `PdfReadError`
and warnings that PDF object `214 0` or `316 0`, respectively, is undefined,
yet exits with status zero and leaves a PDF with no `/Outlines`. The Rust
converter repairs only the broken link destinations pointing to omitted
pages and retains all valid source TOC records. Its 49 and 100 output
bookmarks each match the source title, level, and page. A reference PDF
outline comparison therefore fails for these two cases despite valid source
TOC conversion. Compatibility reports must state both facts.

## Reference page-order differences

The reference PDF permutes pages in `issue-40` and `issue-44`, though every
rendered source page occurs exactly once. Across all ten successful CAJ
samples, concatenating surviving source `/Pages` groups' `/Kids` arrays in
their physical order reproduces the complete CAJ page table order. For
`issue-40`, the reference order is source-table pages `1..=12`, `18..=78`,
then `13..=17`. For `issue-44`, it is `1..=9`, `25..=108`, `20..=24`,
`15..=19`, then `10..=14`. The source page
table and surviving PDF `/Pages /Kids` arrays agree on the source-table order.
The reference converter's temporary PDF already has the permutation before
MuPDF opens it. Changing page-table page IDs in a scratch copy of `issue-40`
did not change that temporary PDF; renaming two root-child object IDs changed
their names in its `/Kids` array but did not change the content-group order.
These black-box experiments rule out page-table ID and numeric object-ID
sorting as the cause. A general reference ordering rule remains unproven, so
the parser must not apply a sample-specific permutation as a format rule.

## One observed PDF-fragment anomaly

The `issue-20` PDF body contains six stream dictionaries whose literal
`/Length` values stop before the actual payload ends. The measured byte count
from immediately after `stream\r\n` to immediately before the following
`\r\nendstream` is larger as follows. Each length can be corrected in a local
scratch copy without changing its decimal width; this table does not authorize
a general “search for endstream” repair when payload bytes could contain that
marker.

| Object | Absolute `/Length` digit offset | Declared | Measured |
| ---: | ---: | ---: | ---: |
| 4 | 295128 | 40020 | 40022 |
| 9 | 336159 | 67215 | 67217 |
| 13 | 421704 | 103841 | 103847 |
| 53 | 532910 | 212747 | 212757 |
| 269 | 746923 | 18383 | 18387 |
| 142 | 880600 | 4585 | 4587 |

Its final table row is `(offset 981652, length 1643, page ID 44)`, giving
hint end `983295`. Seven stream-payload bytes follow before `\r\nendstream`;
the final `endobj` ends at `983320`, and XML begins at `983323`. The offsets
are observations for the SHA-256-pinned input above, not general CAJ rules.
The implementation accepts a unique, at-most-64-byte same-width length
correction and requires a repaired final object to reach source EOF or a
recognized XML trailer. A fake terminator followed by unknown bytes is a
typed ambiguous-repair error; the XML check is an observed compatibility
boundary, not proof that every malformed stream has a recoverable end.
The generated Rust and pinned Python reference PDFs both open in MuPDF and
both elicit the same 24 `qpdf --check` content-stream warnings about
`unexpected )`; the affected source content stream is retained unchanged.
For page 39, MuPDF reports identical zlib/font errors on both outputs, so a
successful rendered-page comparison remains unavailable there. The other 62
pages match the recorded reference rendering hashes. Do not count page 39 as
a passing render check.

## Encoding implementation provenance

[`gb18030.rs`](../crates/caj2pdf-core/src/caj/gb18030.rs) is original MIT
code. Its mapping data was generated through exhaustive black-box queries to
Python 3.13.5's `gb18030` decoder, with no inspection or copying of codec
source or tables: 23,940 two-byte candidates all mapped, and 1,587,600
syntactically possible four-byte candidates yielded 1,087,996 mapped values
coalesced into 207 contiguous ranges. Invalid and incomplete sequences are
rejected with a byte offset. This broader decoder policy is deliberate; the
observed successful CAJ titles alone establish only GBK-decodable input.

The Rust conversion path must keep the 256-byte title bound, checked table
arithmetic, seekable input, bounded PDF-fragment copying, and sequential
output. The [provenance register](provenance.md) records the corresponding
source and test-data boundaries.

<!-- SPDX-License-Identifier: MIT -->

# Bounded T.88 text-region headers

Issue [#69](https://github.com/rwv/caj2pdf-rust/issues/69) adds an original
MIT Rust parser for the data header of a JBIG2 **immediate text region**
(segment type 6). In the measured HN/C8 type-3 images this is segment #3,
referring to symbol dictionary #2. The parser establishes a typed input
boundary for a later text-instance decoder. It does not decode symbol
placements, composite rows, or pixels, and it is not a compatibility claim.

The only format source is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
English PDF SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`:
§7.4.1 (region segment information, Figures 28–29), §7.4.3.1 (Figure 35),
§7.4.3.1.1 (Figure 36 flags), §7.4.3.1.2 (Figure 37 Huffman flags),
§7.4.3.1.3 (refinement AT), and §7.4.3.1.4 (`SBNUMINSTANCES`).

## API

`jbig2::text::read_text_region_header(source, header, dictionary, limits,
budget, cancellation)` accepts a validated #42 `SegmentHeader` for the region,
the validated header of the dictionary it refers to, a caller-owned
`RangedSource`, `Limits`, a `TextRegionBudget`, and cancellation. It returns a
`TextRegionHeader`:

| Field | Meaning |
| --- | --- |
| `region` | Width, height, X/Y location, and external combination operator (§7.4.1). |
| `flags` | Every Figure 36 field: `SBHUFF`, `SBREFINE`, `LOGSBSTRIPS` (`strips()` gives `SBSTRIPS`), `REFCORNER`, `TRANSPOSED`, `SBCOMBOP`, `SBDEFPIXEL`, signed five-bit `SBDSOFFSET`, `SBRTEMPLATE`, and the raw value. |
| `huffman_flags` | Figure 37 selections, present only when `SBHUFF` is 1. |
| `refinement_at` | Two signed AT pairs, present only when `SBREFINE` is 1 and `SBRTEMPLATE` is 0. |
| `instances` | `SBNUMINSTANCES`. |
| `header_bytes`, `body` | Parsed header length and the exact, unread absolute body range. |

`unsupported_feature()` classifies legal configurations outside the target
decoder profile (arithmetic coding with refinement template 1 or no
refinement): Huffman regions and refinement template 0 with adaptive pixels.
The symbol-ID Huffman table (§7.4.3.1.5) belongs to a Huffman body and is not
parsed.

## Validation order

All framing checks complete **before any source read**:

1. `Limits` are valid, the request bound is nonzero, and the operation is not
   cancelled.
2. The segment type is 6. Types 4 (intermediate) and 7 (immediate lossless)
   share the syntax but are typed `Unsupported`; other types are malformed.
3. The page association is nonzero. The region refers to exactly one segment
   (other counts are typed `Unsupported`), which is the supplied dictionary:
   a type-0 segment with a smaller number, on the same page or global
   (page association 0).
4. The data length is within `Limits::max_input_bytes`; the segment header
   start does not underflow; the data end does not overflow and lies within
   the source.

The parser then reads at most 29 header bytes through bounded requests
(`min(budget.max_source_request_bytes, limits.io_chunk_bytes)`), checking
cancellation before and after each read. Short reads resume; zero-length
reads are `Truncated`; overreported lengths are `Malformed`; source errors
and source cancellation keep their type. Reserved region flags, region
combination values above 4, reserved Huffman bits, Huffman selector value 2
where Figure 37 forbids it, and refinement Huffman selections without
`SBREFINE` are malformed. Empty regions are typed `Unsupported`. Width,
height, pixel count, header bytes, instance count, and body length each have
a separate budget. An arithmetic body shorter than the two-byte MQ terminal
pair is `Truncated`.

Errors carry the segment number, the semantic field offset, and
`bytes_fetched`, the physical header bytes read so far. The parser keeps no
state between calls, so dropping a pending call leaves nothing to discard.

## Anomalous header policy

T.88 §7.4.3.1.1 requires `SBRTEMPLATE` to be 0 when `SBREFINE` is 0. The
measured corpus has one violation: raw flags `0xa40c` in HN `issue-43`,
page 11 image 1, record offset 930,673. The parser does not correct the bit.
It returns `MalformedFlags { field: "SBRTEMPLATE without SBREFINE", raw }`
located at the flags field, and the optional inventory records the raw value
and coordinate. A later text-instance decoder must make an explicit
compatibility decision for this image; this parser does not weaken the
standard's validation.

## Measured boundary and optional inventory

The SHA-pinned #42/#43 inventory has 546 type-3 images in five HN/C8
documents. Each has a type-6 region #3 on page 1 that refers to #2, covers
the page at `(0,0)` with OR, and uses a 23-byte header (17-byte region
information, two flag bytes, four-byte `SBNUMINSTANCES`). Fifteen raw flag
values occur, all arithmetic with eight strips, bottom-left corner, no
transposition, OR, default pixel 0, and refinement template 1; 545 set
`SBREFINE`. `SBNUMINSTANCES` ranges over 6–15,576 and sums to 354,063.
Segment data is 64–28,634 bytes; bodies are 41–28,611 bytes.

[`scripts/jbig2_text_region_headers.py`](../scripts/jbig2_text_region_headers.py)
checks these facts. See the
[conformance notes](../tests/conformance/README.md#optional-jbig2-text-region-header-inventory).
Metadata `PASS` never counts as text or pixel compatibility; that status
stays `NOT_RUN` with zero cases.

## Remaining work

Symbol-instance decoding (§6.4, Table 9), strip and reference-corner
placement, refinement of instances against #66's cross-store exported
catalog, row composition, the `0xa40c` compatibility decision, page
composition, and independent pixel parity remain open under
[#9](https://github.com/rwv/caj2pdf-rust/issues/9). Exact Table E.1 states
remain governed by [#44](https://github.com/rwv/caj2pdf-rust/issues/44).

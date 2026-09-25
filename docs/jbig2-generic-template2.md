# Bounded JBIG2 generic-region template 2

This note covers [issue #49](https://github.com/rwv/caj2pdf-rust/issues/49): an
original MIT implementation of one arithmetic **immediate generic region**
(segment type 38). It accepts template 2, TPGDON off, and the observed
adaptive pixel `(2, -1)`. It returns packed rows; it does not compose a page,
decode symbol dictionaries or text regions, parse an HN/C8 container, or
provide the exact T.88 probability table. The table remains caller-supplied
while [issue #44](https://github.com/rwv/caj2pdf-rust/issues/44) reviews its
MIT redistribution basis. Full 546-image generic-only parity belongs to
[issue #50](https://github.com/rwv/caj2pdf-rust/issues/50).

## Official source and context order

The sole algorithm source was [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
English PDF SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
Relevant clauses are §§6.2.5.2–6.2.5.4, 6.2.5.7, 7.4.1, 7.4.6.1–7.4.6.4,
Table 34, Figure 5, and E.3.7. No decoder source or numeric Table E.1 row
was copied from another project or bundled here.

The ten Figure 5 context pixels map to bits 9 through 0 in this fixed order:

| Bits | Source row | Columns relative to the target |
| --- | --- | --- |
| 9, 8, 7 | y-2 | x-1, x, x+1 |
| 6, 5, 4, 3, 2 | y-1 | x-2, x-1, x, x+1, adaptive x+2 |
| 1, 0 | current y | x-2, x-1 |

T.88 §6.2.5.3 permits any fixed bijective assignment of these pixels to
context bits. A pixel above the first row or outside the region width is
zero. The region context bank has 1,024 entries and resets to state index
zero/MPS zero at the start of each region. Decoding proceeds left-to-right,
top-to-bottom, one MQ decision per pixel. Each row is MSB-first; unused low
bits in its final byte are zero. The combination operator and placement are
reported as metadata; no target page is modified.

## API and limits

`GenericRegionDecoder::new` borrows a checked `SegmentHeader`, ranged source,
caller-supplied `MqTable` and exactly 1,024 `MqContexts`, sequential sink,
`Limits`, cancellation, `MqBudget`, and `GenericBudget`. The constructor
checks the declared segment span, 17-byte region information, generic flags,
signed AT coordinates, dimensions, coordinates, strides, pixel area, and MQ
span before creating the arithmetic decoder. Other segment types, MMR,
other templates, TPGDON, and other legal AT positions yield typed unsupported
errors; a forward/current adaptive reference is malformed. A header field or
MQ span ending early is rejected at its source location. The observed profile
uses region flags zero, generic flags `0x04`, and AT `(2, -1)`; accepting
other valid region combination operators does not imply page composition.

Call `decode_next_row` repeatedly, then `finish`. Only three packed rows
(current and two previous), 1,024 contexts, a 47-state caller table, and the
MQ core's fixed 256-byte input buffer are needed: `O(row_stride + contexts)`
working memory independent of image height or encoded span length. The
three-row/context/table/buffer total is checked against the allocation cap
before row allocation. Fallible row reserves, dimension/output/input/span/
symbol/work caps, and cancellation checks bound runtime and memory. Sink
writes are sequential and may be partial. Errors include segment and source
byte plus completed rows, decoded symbols, and physically written bytes.

A row future is marked poisoned before it awaits MQ or sink I/O. A failed or
dropped pending future leaves the decoder poisoned; further rows fail, and
the caller must discard its partial sink output. `finish` requires all
`width × height` decisions, checks `FF AC` only at the explicitly delimited
whole segment-data MQ span, and flushes after validation. In the observed
546-image directory inventory, all 546 generic-region data spans end in
`FF AC`; this says nothing about possible earlier internal substreams or
unobserved JBIG2 variants. `GenericReport` distinguishes the semantic MQ
input position from physically fetched bytes, including bounded prefetch and
terminal lookahead. The semantic byte need not reach the terminal marker.

## Verification

Synthetic tests use an invented 47-state MQ machine and independent literal
expectations for each of the ten context positions, both row edges, row
history at y=2, all-one packed rows with zero padding, partial writes,
malformed headers and terminal bytes, input/output/allocation/work bounds,
cancellation, and dropped futures. They test the API and image-model rules,
not T.88 or CAJ compatibility.

The ignored `generic_t88_external` test requires both a private official
Table E.1 fixture and SHA-verified external CAJSamples documents. In an
ordinary CI run it is **NOT_RUN**. Explicitly requested missing or changed
inputs fail. Run locally with:

```sh
CAJ2PDF_T88_H2_FIXTURE_FILE=/tmp/caj2pdf-t88-2000-h2-private.fixture \
CAJ2PDF_GENERIC_CORPUS_DIR=/tmp/caj2pdf-ranged-corpus \
cargo test --locked -p caj2pdf-core --test generic_t88_external -- --ignored --nocapture
```

On 2026-09-24, two independently identified generic-only spots passed:

| External source | Region | Packed-row SHA-256 |
| --- | --- | --- |
| C8 issue-58, page 1 image 1 | 2366 × 3368 | `dae0fec2ea4c15de4b70f590a6bb3629f8bf17c225f0d0d4427743a04084fcb6` |
| HN issue-43, page 2 image 1 | 2368 × 3431 | `72170496b556f7628b436b8e924e9bc4aa2815dc8d31106ab64e8ea0dbecde8f` |

The expected hashes were obtained by copying only each SHA-verified image's
page-info and generic segment into a temporary one-image PDF, then comparing
normalized PBM output from `pdfimages` and `mutool`. This is black-box tool
agreement; implementation independence between the tools is **UNVERIFIED**.
No corpus bytes, generated PDF/PBM, or official table/vector bytes were
committed. These two spots do not establish complete HN/C8 page parity.

<!-- SPDX-License-Identifier: MIT -->

# External Rust generic-region pixel parity

This is the optional conformance harness for [issue #50](https://github.com/rwv/caj2pdf-rust/issues/50).
It compares the original MIT [template-2 row decoder](../crates/caj2pdf-core/src/jbig2/generic.rs)
with the [generic-only black-box baseline](../tests/conformance/jbig2_generic_oracle.json).
The test covers **segment #4 alone** for 546 type-3 images in five documents;
it does not measure symbol dictionaries, text regions, page composition, or
complete HN/C8-to-PDF output. The exact T.88 Table E.1 states remain private
under `/tmp` pending [issue #44](https://github.com/rwv/caj2pdf-rust/issues/44).

## Method and failure rules

Run from the repository root with the pinned CAJSamples revision and a local
official-table fixture:

```sh
python3 scripts/jbig2_generic_parity.py \
  --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --json
```

The fixture is a private `T88-2000-H2`/`47` text file as described in
[the MQ note](t88-mq-core.md), with SHA-256
`bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57`.
The driver and native example require it to resolve below `/tmp` and never
write its state rows into Git, reports, or generated PDFs. To skip compilation
of the native probe, pass `--rust-bin target/release/examples/jbig2_generic_parity`.

The driver first validates the strict, hash-only #51 manifest and SHA-256 of
all 27 external sources. It re-runs the #51 oracle on every image: a fresh,
bounded one-page PDF with only original segments #0 and #4; warning-free
`qpdf --check`; native P4 output from Poppler and MuPDF; normalized packed
rows with MSB-first, `1=black`, and zero padding bits. It requires a `PASS`,
546 tool agreements, and zero failures. Tool binary identity drift is
reported separately; changed pixel hashes or other manifest semantics fail.
Agreement between Poppler and MuPDF does not establish distinct decoder
implementations; backend independence remains **UNVERIFIED**.

The driver then obtains a fresh #42 Rust directory inventory. Each image
coordinate, source identity, dimensions, generic profile, and exact original
header-plus-data span offset, length, and SHA-256 of #0 and #4 must match
the manifest. A bounded private metadata plan under `/tmp` drives the
native Rust probe. The probe hashes the selected original segments before
and after decoding, parses their headers, supplies the private MQ table to
the #49 decoder, and streams exactly one packed row at a time into a SHA-256
and black-pixel counter. It rejects altered dimensions, padding, row count,
pixel count, output bytes, pixel hash, black count, or source span. The
driver requires 546 matched coordinates and rehashes all 27 source files
after Rust finishes. A missing corpus or table on a clean clone reports
`NOT_RUN`, zero Rust matches, and zero tool agreements. Explicitly supplied
but invalid inputs, tool warnings, and mismatches report `FAIL`; no image is
silently skipped.

The [synthetic Python tests](../tests/conformance/test_jbig2_generic_parity.py)
check clean-clone status, exact span/source/profile joins, digest mismatch,
failure to count black-box `NOT_RUN` or warnings, and removal of the private
plan after decoder failure. #51 tests cover PDF tool warnings, normalized
pixel mismatch, manifest validation, source mutation, and temporary output
cleanup. #49 Rust tests cover bounded row decoding and error propagation.
Ordinary CI asserts the optional #50 run is `NOT_RUN` with zero matches.

## Measured external run

On **2026-09-25 UTC**, a same-run comparison completed with the pinned
CAJSamples revision `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07` and
the tool binaries recorded in the #51 manifest: qpdf 12.2.0, Poppler
`pdfimages` 25.03.0, and MuPDF `mutool` 1.25.1. All **546/546** generic-only
tool comparisons and **546/546** Rust rows-to-hash comparisons passed,
including 27 source SHA checks both before and after. There were zero
failures, skips, or semantic/toolchain drift. Rust decoded
**4,263,929,076** generic-region pixels. These results apply only to the
specified external corpus and generic-only profile.

| Measure | Smallest pixel case: issue-66 p1/i1 | Largest pixel case: issue-43 p3/i1 | Largest encoded #4: issue-43 p94/i1 | All cases / bound |
| --- | ---: | ---: | ---: | ---: |
| Dimensions | 848 × 251 | 2496 × 3522 | 2218 × 3328 | 546 images |
| Pixels | 212,848 | 8,790,912 | 7,381,504 | 4,263,929,076 |
| Packed row bytes | 106 | 312 | 278 | Maximum 312 |
| Original #4 header-plus-data bytes | 1,948 | 6,563 | 76,030 | Maximum 76,030 |
| Decoder source reads / bytes | 20 / 1,959 | 38 / 6,574 | 309 / 76,041 | 49,465 / 10,940,039 |
| MQ source bytes fetched | 1,917 | 6,532 | 75,999 | At most selected span |
| Rust case interval | 4 ms | 214 ms | 168 ms | Rust batch 90.255 s |

The standalone native Rust probe reached **1,932 KiB VmHWM** as reported by
Linux `/proc/self/status`. Its largest decoder source request was 256 bytes,
and its largest sink write was one 312-byte row. The separate selected-span
SHA checks use a 64 KiB buffer; their reads are excluded from the decoder
source metrics. The batch averaged **47.24 million pixels/s**, including its
selected-span hash checks but excluding the Python oracle and 27 whole-source
checks. The Python plan was 243,298 bytes in this run and capped at
2 MiB. It is removed after either outcome. The #51 external-tool stage
measured a maximum 76,108-byte selected record, 76,766-byte PDF, and
1,098,864-byte *single* PBM raster; these are separate file maxima and do
not constitute a hard total disk quota. External tool RSS is separate from
the Rust process VmHWM. The Rust core uses three packed rows, 1,024 MQ
contexts, a caller-supplied 47-state table, and a 256-byte MQ buffer, so
working memory scales with row width rather than image height.

`cargo check --target wasm32-unknown-unknown --workspace --all-targets` and
the release WASM boundary build verify target compilation. The generic
decoder has no JS runtime adapter yet, and this native probe uses OS files;
**WASM runtime memory was NOT_MEASURED**. Browser and Node.js conversion
parity remain separate tasks. No corpus bytes, table rows, PDF, PBM, decoded
pixels, or black-box tool binaries are committed or released.

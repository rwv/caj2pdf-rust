<!-- SPDX-License-Identifier: MIT -->

# Rust HN/C8 row-model result ledger

[`jbig1_row_model_results.jsonl`](jbig1_row_model_results.jsonl) records one
hash-only result for each of the 1,400 type-0 images in the pinned
[`jbig1_oracle.json`](jbig1_oracle.json) manifest. The manifest SHA-256 is
`e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a`.
Its 27 source records include 21 with images and six without images. Ledger
`index` runs from 0 to 1399 in manifest sample/image order;
`sample_index` and `image_index` are zero-based array positions. The one-based
`page` and `image` values repeat the matching manifest fields. Use those
indices to obtain the exact sample ID, source SHA-256, encoded-span offset and
length, width, height, stride, and expected pixel hashes. The ledger contains
no document, compressed stream, bitmap, PDF, or normative state-table bytes.

Each row's `hashes` booleans mean that the sampled source file, selected
DIB-plus-coded span, decoded 32-bit-stride bitmap, and cropped/masked visible
bits respectively matched the four SHA-256 values in the manifest. Source
files were hashed once per sample; every image span and both decoded layouts
were checked separately. `copied_rows + decoded_rows = height`, and the
observed arithmetic `symbols = height + width × decoded_rows`. `work` is the
arithmetic core's observed work counter. `bounds` records checks of valid
geometry, encoded length, raw bitmap length, observed symbols, and observed
work. All 1,400 rows have `status: "PASS"`, all hash and bound checks true.

An independently authored temporary Rust harness used
`caj2pdf-core::qm` at commit
`022e99190759805566e9e85a2c1ef45cf49e0d9b` with Rust 1.98.1. It
verified source and encoded hashes before decoding, used the measured
three-line row model described in [issue #27](https://github.com/rwv/caj2pdf-rust/issues/27),
held three bitmap rows and one comparison row, spooled display-order rows to
`/tmp`, then read them in reverse order for the manifest's DIB memory-order
hashes. The spool was removed after each image. Three nonblank HN/C8 canaries
also matched external oracle rows byte for byte. The complete temporary
batch report SHA-256 is
`fe5e2f6421d5af904ad28826067d8271c9183061762700dd69191e6b62bb914c`;
the temporary MIT Rust source SHA-256 is
`01e52ca11e6a764a242d68a1d6bb174c33bf6a6be4808242ebb22476dbe4904b`.
The ledger SHA-256 is
`e2bea810dd6dfade0d1e54c95737d834f5765c3b27f6e1900edaf4b4ba4e0a7a`.
The harness ran from `/tmp/caj27/rust-canary` with
`cargo run --release --offline -- --batch`, using the external corpus under
`/tmp/caj2pdf-ranged-corpus` and an external T.82 fixture SHA-256
`11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`.

Per-image hard caps were 64 MiB encoded bytes, 128 MiB raw bytes, width at
most 10,000, height at most 20,000, and at most 12,000,000 symbols. For a
given geometry, the symbol budget was `height × (width + 1)` and the work
budget was `32 × symbol_budget + 1024`. The largest observed values were
8,706,856 symbols and 9,058,535 work units; the largest selected encoded
span and raw bitmap were 79,390 and 1,094,808 bytes. These are enforced
bounds and observed counters, not a measured peak-RSS claim.

This ledger archives one external-corpus experiment; a clean clone without
the corpus and externally supplied T.82 state table must report `NOT_RUN` for
compatibility. The state table was supplied only at runtime and is absent
from this repository. Its MIT redistribution question remains open in
[issue #30](https://github.com/rwv/caj2pdf-rust/issues/30). A passing corpus
snapshot does not establish every CAJ variant or release support.

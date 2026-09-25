# External corpus baseline

[`matrix.json`](matrix.json) inventories the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository at commit
`7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`. It has 56 unique input
documents: 49 `.caj` files and 7 `.teb` files. The 51 entries under `type-*`
are symbolic-link aliases of real files and appear only in each sample's
`aliases` list. Five `.caj` files have no type alias. Eight separate `.pdf`
files and ten `.dat` image dumps are excluded from the input matrix.

## Identity and redistribution

CAJSamples does not declare a redistribution license for its documents. No
document, derived PDF, image, or outline text is committed here. Each sample
has a canonical relative `id`/`path`, a size, a Git blob OID, and a SHA-256
digest. The OID is the SHA-1 digest of `blob <size>\0` followed by the file
bytes. All 56 OIDs and SHA-256 digests were verified while streaming the
external files. The original [Python converter](https://github.com/rwv/caj2pdf)
was run as a black-box oracle; none of its source code was reused.

For 51 aliased samples, `detected_type` and `variant` follow the upstream type
index. The five unaliased samples were classified from their file headers,
matched against the pinned
[magic index](https://github.com/caj2pdf/CAJSamples/blob/7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07/magic).
Four used HTTP `Range: bytes=0-255` responses whose `Content-Range` totals
matched the Git tree sizes; the fifth was examined locally. The Python `show`
command confirmed types for the 49 files on which it succeeded.

Use a local external corpus checkout to verify the inventory:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples python3 scripts/conformance.py --json
```

A missing requested corpus or missing/mismatched file must fail. An unset
corpus must report `NOT_RUN`, never a compatibility pass.
`--only-format KDH` checks only the three pinned KDH inputs and their output
PDFs after validating the full matrix; its results make no claim about the
other formats.

## Optional JBIG2 dictionary header inventory

[`jbig2_dictionary_headers.json`](jbig2_dictionary_headers.json) contains
only source IDs, one-based image coordinates, numeric fields, absolute spans,
and SHA-256/Git-blob digests. It pins the #1/#2 symbol-dictionary header
observations in all 546 HN/C8 type-3 images. The original metadata
measurement has SHA-256
`e4897fcde9f0fea32471d58790bad8246776ed2ba1f1d8586f2f8b6adf65ad52`;
the `directory_report_sha256` field identifies its separate #42 inventory.
The header layout and flag interpretation follow
[ITU-T T.88 (02/2000), §7.4.2.1](https://www.itu.int/rec/T-REC-T.88-200002-S/en).
The script limits itself to the two measured flag profiles, `0x0800` and
`0x1802`; it does not decode either dictionary's bitmaps.

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  python3 scripts/jbig2_dictionary_headers.py --json
```

The optional runner checks the size, SHA-256, and Git blob ID of **all 27**
HN/C8 sources before and after reading records. It reuses the Rust #42
directory inventory to locate each image and segment, then reads only the
12-byte observed header of each dictionary. It streams SHA-256 over every
enclosing image and both dictionary data spans: 1,638 span checks for 546
images. It rejects any mismatch with the committed metadata and the existing
#43 image-span oracle. The JSON `metadata` status can be `PASS` independently
of `symbol_compatibility`, which remains `NOT_RUN` with zero checked cases
because no independent per-symbol pixel oracle is available. An optional
private Table E.1 fixture supports local diagnostic runs but is neither
required by this metadata runner nor a substitute for expected symbol
pixels. The full-image #43 and generic-only #51 hashes
do not prove first-dictionary symbol output. A clean clone reports
`NOT_RUN`/0 for both optional metadata and symbol compatibility; an explicitly
requested missing or changed corpus or manifest reports `FAIL`.

The pinned header inventory found #1 flags `0x0800`, AT `(2,-1)`, and 3–514
new/exported symbols. Dictionary #2 has flags `0x1802`, AT `(2,-1)`, 0–318
new symbols (57 zero-new cases), and 3–542 exported symbols. Its
refinement/aggregate mode remains unsupported by this slice. These counts
are metadata observations, not decoded-symbol or page-conversion results.

## Optional JBIG2 refinement-pixel evidence

Issue [#65](https://github.com/rwv/caj2pdf-rust/issues/65) adds a bounded
template-1 refinement-bitmap primitive. No independent per-symbol
refinement-pixel oracle is available, so
[`jbig2_refinement_oracle.py`](../../scripts/jbig2_refinement_oracle.py)
reports `NOT_RUN` with zero compatibility cases in a clean clone and in CI.
The #43 whole-image and #50 generic-only hashes cannot establish refinement
pixel parity. A supplied corpus is checked against the SHA-pinned dictionary
metadata inventory, but that remains metadata evidence only.

An optional private fixture may be supplied with both
`--fixture-file /path/to/file` and `--fixture-sha256 HEX`, or the matching
`CAJ2PDF_T88_REFINEMENT_FIXTURE_FILE` and
`CAJ2PDF_T88_REFINEMENT_FIXTURE_SHA256` environment variables. The file is
read in bounded chunks with a 4 MiB cap. Missing, empty, oversized, or
hash-mismatched material fails; a verified file still reports `NOT_RUN`/0
because no independent refinement-pixel comparison is wired. This identity
check is not a decoder compatibility test. A future oracle integration must
define independent expected pixels and compare them before it can report a
compatibility pass.

## Optional HN/C8 Rust container comparison

The independent [HN/C8 type-0 manifest](jbig1_oracle.json) pins 1,400
one-based page/image coordinates and absolute payload offsets and lengths.
Run the Rust container reader against the external HN/C8 subset with:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  python3 tests/conformance/hnc8_container_compare.py --json
```

The runner checks all 27 selected source sizes and SHA-256 values before
invoking Rust, inventories every image record, compares all 1,400 type-0
coordinates and spans to the manifest, and hashes all 27 files again after
the comparison. It inventories types 1, 2, and 3 as record discriminators,
without claiming to decode them. The three known malformed observations in
`issue-100` are checked by a separate diagnostic page probe; the normal
cursor must fail at its first malformed record. A missing or changed requested
corpus is `FAIL`. With no corpus selected, the result is `NOT_RUN`, with zero
compatibility passes. An already built native example may be supplied with
`--rust-bin /path/to/hnc8_container_inventory`; otherwise the runner builds
the repository's `caj2pdf-core` example. It records no external bytes, PDF,
or bitmap in the repository.

The JSON report keeps two independent invalid-record accounts. After the
source precheck, `baseline_invalid` reports the three exact historical #22
manifest observations, including their pinned descriptions; it is `NOT_RUN`
when the corpus is absent. `expected_invalid` reports the Rust reader's
diagnostic page probes. Neither account contributes type-0 compatibility
passes. Both must match their own pinned expectations for the run to pass.

For `issue-100` page 2, the Rust reader reports an unsupported, unmeasured
image type before interpreting the descriptor's remaining fields; the #22
manifest separately records its black-box out-of-source image observation.

## Python reference baseline

All 56 inputs were tested on 2026-09-24 with the unmodified Python converter
at commit `8cbc3c5721acb762f739434eb3d206171dbb022a`, Python 3.13.5,
PyPDF2 1.26.0, and MuPDF `mutool` 1.25.1 on Linux. The native
`libjbigdec.so` and `libjbig2codec.so` libraries were unavailable. Results
are tied to this environment and should be remeasured with pinned native
dependencies before release gating.

| Conversion status | Samples | Interpretation |
| --- | ---: | --- |
| `success` | 16 | A nonempty PDF was produced and inspected by `mutool`. |
| `unsupported` | 8 | Seven TEB inputs produced no file despite exit code 0; one pure-text HN input was explicitly rejected. |
| `error` | 8 | Six `mutool` PDF syntax failures, one page-index parse error, and one invalid HN image count/offset. |
| `skip` | 24 | The Python conversion could not load its native JBIG library; support remains unmeasured. |

Python `show` succeeded on 49 inputs and errored on all seven TEB inputs.
A `skip` has `expected_outcome: "unknown"`; it is never counted as
compatibility evidence. HN's image-only reference output does not establish
searchable text support.

Top-level `page_count` and `outline_count` are Python `show` source counts
where available. KDH and embedded-PDF `show` output lacks counts, so those
fields use counts from the successful output PDF. For a successful conversion,
`expected_pdf.page_count` and `expected_pdf.outline_count` always describe
the actual output PDF. Three reference conversions differ from source counts:

| Sample | Source pages/outlines | Output pages/outlines |
| --- | ---: | ---: |
| `issue-49` | 65 / 49 | 65 / 0 |
| `issue-65` | 6 / 0 | 2 / 0 |
| `issue-73` | 84 / 100 | 84 / 0 |

## Output fingerprints

The 16 valid reference PDFs have 933 pages in total. The matrix records every
output page dimension, output outline count, and a SHA-256 digest of normalized
outline hierarchy and destinations. The outline digest hashes one UTF-8,
newline-terminated, compact JSON object with sorted keys per entry; each
object contains depth, title, page, and destination, but only the digest is
stored.

Each page is rendered and hashed separately using stdout from
`mutool draw -q -L -B 128 -F pam -c rgb -r 72 -o - PDF PAGE`. This is a
bounded-memory PAM RGB render at 72 dpi, and each row records
`mutool version 1.25.1`. Fifteen PDFs have `render_coverage: "full"`.
The `issue-20` reference PDF has hashes for 62 of 63 pages; page 39 fails in
`mutool draw` with embedded-font/zlib errors and is marked `partial`.
A partial fingerprint must be reported as `NOT_RUN` for full visual
compatibility. Render hashes are renderer-version-specific.

The inventory and output checks are implemented in
[`scripts/conformance.py`](../../scripts/conformance.py). No reference PDF
or document corpus is stored in this repository.

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

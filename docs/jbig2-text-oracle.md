<!-- SPDX-License-Identifier: MIT -->

# External HN/C8 text-only JBIG2 pixel oracle

This [issue #85](https://github.com/rwv/caj2pdf-rust/issues/85) baseline
measures what the original JBIG2 page-information segment #0, symbol
dictionaries #1–#2, and immediate text region #3 render **without** generic
region #4. It supplies the Rust text-region composer with per-image
reference hashes. The external oracle itself is neither Rust parity nor a
full-image comparison.

The format reference is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
especially §§6.4 and 7.4.3. The consulted English PDF has SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
The executable uses only repository-owned MIT inventory, PDF, PBM, and
metadata helpers. No external decoder source, official arithmetic states,
document, encoded image, PDF, or decoded bitmap is in this repository.

## Input and record checks

The [pinned matrix](../tests/conformance/matrix.json) identifies 27 HN/C8
documents at CAJSamples revision `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`.
The runner checks all 27 sizes and SHA-256 hashes before and after each
requested run. A fresh [#42 Rust directory inventory](jbig2-directory.md)
must find exactly 546 unique type-3 coordinates across the five expected
image-bearing files (105, 4, 1, 208, and 228 records). Before invoking a
renderer, the runner checks the 48-byte DIB/palette, page dimensions, #43
full-record hash/profile, #69 text-header flags/instance aggregates, exact
raw segment headers, references, page association, declared lengths, and
contiguous source spans through the end of #4. If an existing manifest is
present, each selected #0–#3 span and its hash must match it.

For each case, separately hashed source spans are copied in at most 64 KiB
chunks into an isolated temporary record containing only the DIB and
segments #0–#3. The copied hashes and exact record size are checked again.
The repository-owned PDF writer then verifies the temporary record hash
while writing a one-image `/JBIG2Decode` PDF. `qpdf --check` must exit zero
without a warning. Poppler `pdfimages` and MuPDF `mutool draw -F pbm -r 72`
produce P4 PBMs. The existing normalization compares each top-to-bottom
row, MSB first, `1` for black, with unused low padding bits cleared. Both
pixel SHA-256 and black-pixel count must agree. Any disagreement is a `FAIL`.

The [hash-only manifest](../tests/conformance/jbig2_text_oracle.json) records
source identities, coordinates, dimensions, selected source span offsets,
lengths and hashes, raw text flags, instance counts, normalized pixel hashes,
black-pixel counts, and tool versions/binary hashes. Its validator rejects
unknown fields, missing fields, duplicate coordinates, source/profile drift,
bad span joins, and misplaced anomaly classification. It contains no encoded
or decoded external bytes. Dynamic `libjbig2dec` linkage evidence is also
recorded; decoder implementation independence remains `UNVERIFIED`.

## The anomalous header

545 headers match the measured T.88 text-header subset. HN `issue-43` page
11 image 1 has raw flags `0xa40c`: `SBREFINE=0` with `SBRTEMPLATE=1`, which
violates T.88 §7.4.3.1.1. The manifest labels its externally rendered pixels
`INTEROPERABILITY_NONCONFORMING`; they do not count as a strict Rust parser
pass. The other 545 are labeled `STANDARD_VALID`. The oracle never edits the
anomalous bit or asks the Rust parser to accept it.

## Reproduce and limits

From the repository root, with the external corpus and three separate tools
installed on a POSIX system:

```sh
python3 scripts/jbig2_text_oracle.py --corpus-dir /path/to/CAJSamples --json
```

The first run may create the manifest after all 546 cases agree:

```sh
python3 scripts/jbig2_text_oracle.py --corpus-dir /path/to/CAJSamples --write-manifest --json
```

When a manifest already exists, write mode still compares every semantic
result before replacing it; changed pixel hashes or counts fail. A matching
manifest may be rewritten to record the current external tool identities.

A clean clone without the optional corpus reports `NOT_RUN`, zero checked
cases, and zero tool agreements. An explicitly supplied missing, changed,
malformed, or out-of-profile corpus or manifest fails. A supplied corpus
requires all three tools. Setup and manifest errors set `FAIL` separately;
`completed`, `failed`, and `skipped` count image attempts and remain mutually
exclusive. The clean-clone synthetic tests verify these reporting rules but
are never counted as external compatibility cases.

Python reads source spans and PBM pixels in bounded chunks or rows. Per case,
the selected record is capped at 64 MiB, the PDF at 64 MiB plus 4 KiB, and
each PBM at the expected raster bytes plus a 512-byte header (up to 128 MiB
plus 512 bytes). Each external tool has a 60-second deadline, an 8 KiB
captured-diagnostic ceiling, and a POSIX child file-size limit appropriate
to its output. The runner checks the number and total size of temporary
files after each tool; all case files are deleted on success or failure.
These are file and elapsed-time controls, not a bound on the external tool's
resident memory, filesystem metadata, or transient allocation between checks.

On 2026-09-27 UTC the local SHA-pinned corpus run completed **546/546**
two-tool agreements, with **545 standard-header cases**, **one separately
labeled interoperability case**, zero case failures, zero skipped cases,
and 27/27 source hashes checked before and after. The largest selected
record was 30,293 bytes, temporary PDF 30,951 bytes, and single expected
PBM raster 1,098,864 bytes. The three tool identities and binary SHA-256
values are in the manifest. Backend implementation independence is
`UNVERIFIED`; agreement is independent of the Rust decoder but may involve
shared underlying code. The separate [#87 Rust composer
diagnostic](t88-text-composer.md) matched all 545 standards-valid text-only
regions; this does not make the external backends independent.

The [#9 parent](https://github.com/rwv/caj2pdf-rust/issues/9) remains open
for page composition, a separate anomaly policy, and the exact-state rights decision under
[#44](https://github.com/rwv/caj2pdf-rust/issues/44).

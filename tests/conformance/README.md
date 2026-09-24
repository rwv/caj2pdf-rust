# External corpus baseline

[`matrix.json`](matrix.json) inventories the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository at commit
`7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`. It has 56 unique input
documents: 49 `.caj` files and 7 `.teb` files. The 51 entries under `type-*`
are symbolic-link aliases of real files, so they appear only in each sample's
`aliases` list. Five `.caj` files have no type alias. Eight separate `.pdf`
files and ten `.dat` image dumps are not input samples.

The repository does not declare a redistribution license for these documents.
Keep the corpus and every derived PDF outside this repository. A sample's
`id` and `path` identify its real path within the pinned corpus. Its
`git_blob_oid` and `size_bytes` identify the file contents: the OID is the
SHA-1 digest of `blob <size>\0` followed by the file bytes. The optional
`sha256` is recorded only for files examined locally. The corpus runner
checks the pinned content hash when `CAJ2PDF_CORPUS_DIR` is set.

For the 51 aliased samples, `detected_type` and `variant` come from the
upstream type index. The other five were classified from their file headers:
one from a locally downloaded file and Python `show`, and four from HTTP
`Range: bytes=0-255` requests at the pinned corpus revision. The returned
`Content-Range` total was checked against the tree size, and the signatures
were matched against the pinned
[magic index](https://github.com/caj2pdf/CAJSamples/blob/7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07/magic).
These type labels do not assert that conversion was run. `unknown` and
`not_run` mean exactly that; neither is a compatibility pass.
`expected_outcome` records a measured Python result
when available, or the known reference-level TEB and pure-text HN limitation.
The Python reference's HN image output does not establish searchable text
support.

Six inputs were tested on 2026-09-24 with the unmodified
[Python converter](https://github.com/rwv/caj2pdf) at commit
`8cbc3c5721acb762f739434eb3d206171dbb022a`, Python 3.13.5,
PyPDF2 1.26.0, and MuPDF `mutool` 1.25.1 on Linux. `show` supplied page and
outline counts except for KDH, whose one-page count came from `pdfinfo` on a
temporary converted PDF. The two successful conversions were
`issue-20/文件名未知.caj` (CAJ) and `issue-48/ZZXX200402047.caj` (KDH). The
`issue-63` HN sample reported pure-text HN as unsupported. The `issue-66`
C8 conversion was skipped because the Python reference's native
`libjbig2codec.so` was unavailable; this says nothing about C8 support in a
complete environment. `issue-77` failed in `mutool` with PDF syntax errors,
and `issue-100` failed with an invalid image count/offset. These outcomes
are tied to the stated tool versions and should be remeasured under a pinned
reference environment before release gating.

For the two successful reference PDFs, `expected_pdf` records every page's
dimensions from `mutool pages`, a SHA-256 digest of normalized outline
hierarchy and destinations from `mutool show ... outline`, and a SHA-256 digest
of page 1 rendered as PAM RGB at 72 dpi. The outline digest hashes one UTF-8,
newline-terminated, compact JSON object with sorted keys per entry; the
objects contain depth, title, page, and destination, but only the final digest
is stored. The render digest hashes stdout from
`mutool draw -q -L -B 128 -F pam -c rgb -r 72 -o - PDF PAGE`. The parser and
renderer are in [`scripts/conformance.py`](../../scripts/conformance.py),
with `mutool version 1.25.1` recorded per sample. No document text, outline
titles, image pixels, or converted PDF is stored here. Render hashes are
specific to this renderer and version.

The full external corpus was not available for this baseline run. On
2026-09-24, a shallow Git clone transferred about 4.4 MiB in one minute, and
a direct raw download transferred 1,063,598 of 5,520,323 bytes in 60 seconds.
The other 50 conversion results therefore remain `not_run`; they are not
evidence of compatibility. Run the opt-in corpus check with a local checkout
at the pinned revision to populate and verify those results before using them
as release criteria.

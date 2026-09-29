<!-- SPDX-License-Identifier: MIT -->

# Compressed HN-A/C8 text header

The old reader required a SHA-256 match for the first 20 text-header bytes.
Those bytes include document-dependent values, so the check rejected otherwise
supported documents. This change replaces that whole-prefix fingerprint with
format checks. It does not change coordinate decoding or placement factors.

## Supported fields

Offsets are relative to the indexed page-text span:

| Bytes | Validation / use |
| --- | --- |
| 0..2 | Little-endian tag `0x8003`. |
| 2..4 | Variable payload word; not used for image placement. Meaning remains unknown. |
| 4..6 | Little-endian tag `0x8003`. |
| 6..8 | Variable payload word; not used for image placement. Meaning remains unknown. |
| 8..20 | ASCII `COMPRESSTEXT`. |
| 20..24 | Little-endian expanded length, checked against limits and actual output. |
| 24..end | One complete checksummed zlib stream; no trailing bytes. |

After decompression, existing record counts, marker checks and image-tail
framing still apply. Both tags and every compression-marker byte are required.
This does not admit arbitrary uncompressed text or guess its record layout.
Header payload words are deliberately not treated as dimensions or units.

## Independent evidence

Original byte observations compared the previously supported issue-21 HN-A
and issue-33 C8 profiles with issue-29. All three have the tags and marker
above but different payload words. All 48 issue-29 pages have complete zlib
frames and the existing fixed record/marker layout.

Source issue-29 SHA-256:
`ede5eddb0e8ec1dea46c32a06e16ac12b874a141ca2d6669736495d2ac549261`.
Two separate black-box controls changed only the first page's word at +2 or
+6 to the invented value 1. Both Python reference conversions produced PDFs
byte-identical to the unchanged reference (SHA-256
`6982d3e877a741a4bfb8810404b694ba272db9c5872391e6c654bafcd7e7703b`).
This supports ignoring these words for the measured conversion; their wider
format semantics remain unknown. The original source hash was unchanged.

The native conversion now completes all 48 issue-29 pages with 48 bookmarks.
qpdf accepts the output; qpdf and MuPDF resolve every ordered title, hierarchy,
destination and nullable XYZ view identically to the Python reference.
All 48 pages rendered at 300 DPI with antialiasing disabled match byte-for-byte
in MuPDF and Poppler: 96/96 complete-page comparisons. Pages were processed
sequentially and raster pairs removed after comparison. The comparison report
SHA-256 is `8a19f5d6e006561a0f2af75cbf033baf13b642116fa5562bad7d89d094d46170`.

Issue-69 is different: its 81 text spans do not have this compressed header.
The later [uncompressed-text reader](hnc8-uncompressed-text.md) handles that
profile separately. The compressed-header work does not establish type-3
integration or complete HN parity.

## Tests and provenance

Original synthetic records exercise the public text reader and public page
composer directly. The test-only fingerprint override and composition wrapper
were removed. Tests vary both payload words, corrupt each required tag/marker
byte, and retain existing checksum, expanded-length, limits, short-read,
cancellation and composition tests. Buffers and memory limits are unchanged.

All runtime changes and fixtures are original MIT code. Only input bytes and
black-box converter behavior were observed; no converter implementation was
read or copied. External documents, output PDFs and renders remain outside Git
under `caj2pdf-text-profile-20260929`. Observation/control scripts and results
are retained there. Reference controls had 120-second deadlines, 1 GiB address
space and 64 MiB output-file limits; copies and hashes used bounded chunks.

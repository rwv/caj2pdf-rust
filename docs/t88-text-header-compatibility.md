<!-- SPDX-License-Identifier: MIT -->

# Explicit HN/C8 text-header compatibility

Issue [#88](https://github.com/rwv/caj2pdf-rust/issues/88) defines one
opt-in exception to the strict T.88 text-region header parser. The format
source is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
§7.4.3.1.1. The consulted English PDF has SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
Its `SBRTEMPLATE` field controls refinement decoding when `SBREFINE=1`; when
`SBREFINE=0`, the field is required to be zero.

## Pinned observation

The external HN source `issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj`
has SHA-256
`826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134`.
At page 11 image 1, source record offset 930,673 and length 2,731,
immediate text segment #3 begins at 930,850. Its two text-flag bytes at
absolute source offset 930,879 are `a4 0c` (`0xa40c`). This sets
`SBRTEMPLATE=1` with `SBREFINE=0`; clearing only that bit would yield
`0x240c`. No external document or encoded segment payload is distributed here.

The strict `read_text_region_header` call reports
`MalformedFlags { field: "SBRTEMPLATE without SBREFINE", raw: 0xa40c }`
at source byte 930,879 after fetching 19 header bytes. The independent
[#85 text-only oracle](jbig2-text-oracle.md) labels this record
`INTEROPERABILITY_NONCONFORMING` and records normalized pixel SHA-256
`2ab59a7557aff144b35a2d3e27c8fb2ed5db306a593bd714b9edd1172c69324c`
with 3,718 black pixels and 234 declared symbol instances. Poppler and
MuPDF agreed on that text-only raster; their decoder implementation
independence remains unverified. These are hash and metadata observations,
not a standard-conforming header or a full-page comparison.

## API boundary

`read_text_region_header` remains strict. A caller that has independently
chosen HN/C8 interoperability may call
`read_text_region_header_with_policy(...,
TextHeaderPolicy::HnC8UnusedRefinementTemplate)`. Only the documented raw
`0xa40c` flags with refinement off are eligible; other malformed flag
combinations retain their strict errors. The returned `TextRegionHeader`
retains `flags.raw == 0xa40c` and the typed
`TextHeaderAnomaly::UnusedRefinementTemplate` marker. Its flags are never
silently rewritten to `0x240c`.

`TextInstanceDecoder::new` also remains strict. An opted-in caller must use
`TextInstanceDecoder::new_with_header_policy` with the same named policy,
because the decoder reparses the header against the source before reading
the MQ body. A header parsed under one policy cannot be slipped through a
decoder using another. The composer receives the checked header and exposes
the raw flags and anomaly marker in `TextComposeReport`. With
`SBREFINE=0`, the instance decoder does not read IARI or refinement deltas,
and the composer does not use the refinement template. The policy changes
header acceptance only; it does not normalize the body or relax segment
framing, reference, page, source, resource, cancellation, or poison checks.

The core default remains `TextHeaderPolicy::Strict`. HN/C8 document and page
integration under [#9](https://github.com/rwv/caj2pdf-rust/issues/9) must
choose this policy explicitly after its own container and segment profile
checks. The optional diagnostic makes that choice through
`--text-header-policy hn-c8-unused-refinement-template`. It cannot be
inferred merely from a `.caj` or `.hn` filename.

## Private pixel check

The optional [text-region diagnostic](../scripts/jbig2_text_region_parity.py)
accepts a SHA-pinned external CAJSamples corpus and a separately held
caller MQ state table. It checks all 27 document hashes and the table hash
before and after decoding. Strict mode expects 545 matching standard-valid
text-only regions and one located `0xa40c` header refusal. Opt-in mode
expects those same 545 matches plus one separately counted anomalous match
against the #85 SHA-256 and black-pixel baseline. The report keeps the
anomaly marker, raw flags, case result, and failures separate from strict
matches. It never counts this text-only comparison as complete page or PDF
parity. Without both private inputs the report is `NOT_RUN` with zero
attempts; explicitly supplied missing or changed inputs fail.

The test suite includes small original fixtures for the strict refusal,
precise opt-in acceptance, unchanged instance decisions with the unused bit
set, nearby malformed flags, and failure bounds. No external CAJSamples
file, decoded pixel data, or exact T.88 Table E.1 state is committed.

On 2026-09-27 UTC, the SHA-pinned private opt-in run completed all 546
text-only cases: **545/545 standards-valid regions** matched both #85 pixel
hashes and black-pixel counts, and the **one separately labeled** `0xa40c`
region completed 234 instances and 3,431 rows, matching the #85 hash above
and 3,718 black pixels. It used 1,015,576 bytes of scratch and output, a
296-byte maximum source request, and 2,727,936 bytes peak process RSS on
that machine. Across the run, maximum scratch was 1,098,864 bytes, maximum
request 312 bytes, and peak process RSS 2,764,800 bytes. All 27 source
hashes and the one private table hash matched before and after; there were
zero case failures or skips. This actual arithmetic and composition result
supports ignoring the unused refinement-template bit for this pinned body.
It does not make the header conforming or establish full-page/PDF parity.

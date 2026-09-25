# Provenance and dependency inventory

This register records the information sources, test material, and code origins
used by `caj2pdf-rust`. It is part of the acceptance evidence for
[issue #2](https://github.com/rwv/caj2pdf-rust/issues/2). Update the relevant
entry in the same pull request that adds a format rule, fixture, dependency,
or migrated source file. All project-owned source in this repository must be
MIT-eligible; the repository [LICENSE](../LICENSE) is not a substitute for
checking the provenance of each imported file.

## Format references

| Format or feature | Reference | Status and permitted use |
| --- | --- | --- |
| PDF output and PDF input | [ISO 32000-1:2008](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf) and the [PDF specification archive](https://pdfa.org/resource/pdf-specification-archive/) | Published format specifications. Record the exact PDF version and clauses used for each implementation change. Link to the documents; do not copy their text into source. |
| JBIG / JBIG2 bitstreams | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82) and [ITU-T T.88](https://www.itu.int/rec/T-REC-T.88/en) | Published coding recommendations. Implement the subset required by observed CAJ-family data as original MIT code. Do not reuse reference implementation source. |
| T.82 arithmetic SCD core and numeric states | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82), §6.2.5, §6.8.2.3/Table 24, §6.8.3, and §7.1/Table 26; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | Use the public algorithm to author original MIT Rust code. Keep Table 24's 113 exact numeric rows and the §7.1 vector outside the repository until their MIT redistribution basis is documented. The [core design](t82-arithmetic-core.md) records the external-table contract and local test procedure; standard conformance does not establish CAJ compatibility. |
| T.88 MQ arithmetic control flow and numeric states | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex E.2.6/Table E.1, E.2.9–E.2.10, E.3.1–E.3.6, and H.2/Table H.1; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | Original MIT decoder control flow with a caller-supplied 47-state table. The exact Table E.1 rows and Annex H vector/checkpoints remain outside Git, artifacts, and releases; their MIT redistribution basis remains open in [#44](https://github.com/rwv/caj2pdf-rust/issues/44). The [core note](t88-mq-core.md) records bounded API, differences from T.82, external-only fixture, and verification scope. Annex H.2 verifies arithmetic decisions, not CAJ/JBIG2 pixels. |
| T.88 non-IAID arithmetic integers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.1–A.2 and E.3, with symbol-dictionary usage in §§6.5 and 7.4.2 | Original MIT typed 13-bank integer decision layer over the existing caller-table MQ decoder. The [integer note](t88-arithmetic-integer.md) records its signed/OOB result, 512-context layout, 38-decision limit, and synthetic checks. No Table E.1 states, external dictionary trace, or HN/C8 compatibility claim is included. |
| T.88 template-2 arithmetic generic regions | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5.2–6.2.5.4, 6.2.5.7, 7.4.1, 7.4.6.1–7.4.6.4, Table 34, Figure 5, E.3.7 | Original MIT, bounded row decoder with caller-supplied MQ table. Two external generic-only HN/C8 pixel spots passed; all 546 remain for #50. The [region note](jbig2-generic-template2.md) records the context order, bounds, and external-only verification. |
| CAJ-family headers, pages, and outlines | [caj2pdf format notes](https://github.com/caj2pdf/caj2pdf/wiki), including [CAJ/HN identification](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E5%92%8C-HN), [basic information and outlines](https://github.com/caj2pdf/caj2pdf/wiki/%E6%96%87%E4%BB%B6%E5%9F%BA%E6%9C%AC%E4%BF%A1%E6%81%AF%E4%B8%8E%E5%A4%A7%E7%BA%B2), and [CAJ page content](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9) | Public observations, not a complete normative specification. [Repository-owned CAJ measurements](caj-format.md) pin ten successful sample digests and document TOC, page-table, and PDF-fragment exceptions independently. Do not copy parser source or pseudocode. |
| HN page layout | [caj2pdf HN format notes](https://github.com/caj2pdf/caj2pdf/wiki/HN-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9) | Incomplete public observations. Derive the parser from documented facts and independent tests; mark unresolved fields explicitly. |
| HN/C8 type-0 image wrapper and pixels | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), and [repository-owned oracle measurements](jbig1-oracle.md) | The standards describe public coding and DIB fields. The local corpus measurements pin the CAJ-family wrapper and output hashes. The external differently licensed native decoder is a black-box oracle only, never implementation source or a project dependency. |
| HN/C8 type-0 row primitive | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82-199303-I/en) §§6.5, 6.7.1, 6.8.3; [independent #27 observations](jbig1-row-model.md) | [Issue #55's decoder](jbig1-type0-rows.md) is original MIT code using a caller-supplied table and bounded rows. The exact T.82 Table 24 values and official vector stay external pending #30. Its opt-in 1,400-image check uses only hashes and a private runtime fixture; it is not a released HN/C8 converter. |
| HN/C8 type-3 JBIG2 profile and pixels | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), and [repository-owned oracle measurements](jbig2-oracle.md) | Five SHA-pinned external documents contain 546 type-3 image records. Their original MIT metadata inventory and optional pixel-hash runner record tool agreement only. Poppler, MuPDF, qpdf, documents, PDFs, bitmaps, and decoder code are not runtime or shipped dependencies. |
| HN/C8 type-38 generic-only JBIG2 pixels | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en) §§7.3, 7.4.1, 7.4.6, and 7.4.8 and [repository-owned generic-only measurements](jbig2-generic-oracle.md) | For the same 546 SHA-pinned type-3 records, original MIT tooling measures page-information segment #0 plus generic-region segment #4 alone. The hash-only manifest records black-box tool agreement, not Rust decoder parity or independent decoder implementations. No external source or generated bytes are distributed. |
| KDH wrapper and XOR payload | Three SHA-pinned CAJSamples files measured independently at commit `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07` | The [KDH format note](kdh-format.md) records exact identities, offset 254, the `FZHMEI` cycle, EOF/trailer measurements, and negative controls. The clean-room author derived this code without consulting converter source. |
| C8 and TEB variants | [caj2pdf format notes](https://github.com/caj2pdf/caj2pdf/wiki) and independently observed files | No complete normative specification is registered here. A pull request must explain each new rule and its test evidence; TEB is currently detection only. |

Issue #5 uses these PDF 1.7 facts from the published
[Adobe PDF Reference, version 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf):

| Implemented rule | Specification location | Independent check |
| --- | --- | --- |
| A stream dictionary can refer to a later indirect `/Length` object, allowing its payload to be emitted before its length is known. | Section 3.2.7, “Stream Objects,” and Example 3.1. | Generated streams are reopened with `qpdf --check` and MuPDF. |
| A classic cross-reference table records byte offsets to indirect objects; `startxref` points to that table, and the trailer identifies the document root. | Sections 3.4.3–3.4.4, “Cross-Reference Table” and “File Trailer.” | `qpdf --check` validates generated tables and trailers. |
| The writer caps stream lengths at 2,147,483,647 bytes and indirect objects at 8,388,607, the separate PDF 1.7 Annex C interoperability limits, in addition to the classic xref offset width. | Annex C, “Implementation Limits,” and Section 3.4.3. | Boundary tests reject values above the supported profile with a typed limit error. |
| A page tree supplies ordered page references and page geometry. | Section 3.6.2, “Page Tree.” | Poppler `pdfinfo -box` and MuPDF inspect page count and dimensions. |
| Image XObjects carry dimensions, color space, bits per component, and stream bytes. | Section 4.8, “Images,” including Table 4.39. | MuPDF opens generated image pages; Poppler `pdfimages` decodes synthetic pixels. |
| Outlines link hierarchical items to page destinations; non-ASCII human-readable titles can be UTF-16BE text strings with a byte-order marker. | Section 8.2.2, “Document Outline,” and Section 3.8, “Common Data Structures” (text strings). | MuPDF independently reads generated outline titles and destinations. |

Issue #6 uses the same published PDF 1.7 reference for indirect object
syntax and stream lengths (Sections 3.2.5–3.2.7), classic cross-reference
tables and incremental updates (Sections 3.4.3–3.4.5), document catalogs and
page trees (Sections 3.6.1–3.6.2), and outlines and destinations (Section
8.2.2). The reader and repair writer are new MIT code. The optional external
corpus includes two PDF-body inputs with independently observed, identical
duplicate `/MediaBox` entries in one `/Pages` dictionary and a
`WebFastLoadP` or `WebFastLoadW` footer after an otherwise complete `%%EOF`
marker:

| External sample SHA-256 | Observed anomaly | Independent observation |
| --- | --- | --- |
| `d82e49e39b8d74d36e6a96f50ee4f8cb2d1a5e6072735c345c1bbeffcef1e091` | 26-page PDF body, 9,853 bytes after EOF beginning with `WebFastLoadP`, duplicate identical `/MediaBox [0 0 612 792]`. | `qpdf --check` 12.2.0 recovers the trailing data but warns; a normalized temporary copy retains all 26 rendered page hashes. |
| `8ca4d3a2f42d926ba59b4e0d6c0a2cfd22a75a5c8df7cef05e890b7e79f6106f` | 11-page PDF body, 569 bytes after EOF beginning with `WebFastLoadW`, duplicate identical `/MediaBox [0 0 612 792]`. | Same validator behavior; a normalized temporary copy retains all 11 rendered page hashes. |

These observations describe only the local files matching the listed digests.
The external documents and derived PDF outputs are not included in this
repository. The independently authored
`tests/fixtures/repairable_duplicate_mediabox_tail.pdf` models those two
anomalies for required clean-clone tests; its generator is
`scripts/generate_fixtures.py` and it remains MIT-redistributable.

Issue #36 uses PDF 1.7 Section 3.4.7, “Cross-Reference Streams,” for the
`/Size`, `/W`, `/Index`, `/Prev`, and type-0/type-1 entry rules, and Section
3.4.5 for an incremental classic table pointing back to an earlier xref
stream. Only the `FlateDecode` subset needed by independently observed KDH
PDF bodies is implemented. No type-2 compressed object entry or object-stream
reader is claimed. Two observed inputs have identical duplicate `/MediaBox`
values; one also has six lone-CR separators after `stream`. The third has a
stale page `/Parent` pointing to a free object and four short, inactive object
prefixes between live objects. The latter cases are repaired only after the
active xref and unique page-tree links are checked. The synthetic PDF tests
are repository-owned MIT work; the external documents and normalized outputs
remain outside Git.

Issue #7's [CAJ format note](caj-format.md) records direct byte measurements
for ten Python-success CAJ files from the same external matrix, including
their SHA-256 digests, header and page-table fields, all 603 outline records,
six short PDF stream lengths in one sample, and the fact that a final page-
table row may end inside a PDF stream. The local corpus and derived PDFs remain
outside the repository. These observations are input evidence, not permission
to copy the Python/Go implementations or to apply an ambiguous PDF repair.

Issue #11 adds the original MIT `crates/caj2pdf-core/src/kdh.rs` ranged XOR
adapter, `tests/kdh_conversion.rs`, the native KDH examples, and the raw WASM
bridge and JavaScript proof changes. The KDH
offset, key, and EOF facts come only from the three external corpus files
whose SHA-256 values are registered in [the KDH note](kdh-format.md) and the
conformance matrix. The files were authored for this repository; no Python,
Go, or private Rust module was migrated. The temporary decoded PDFs, MuPDF
renders, and local sparse-tail copy are not committed. The new code adds no
runtime dependency.

The standalone GB18030 title decoder's mapping data was generated by querying
Python 3.13.5's `gb18030` codec as a **black box**, not by reading or copying
its implementation or tables. All 23,940 syntactically valid two-byte
candidates and 1,587,600 four-byte candidates were queried. The latter
produced 1,087,996 mapped values, represented as 207 contiguous ranges. The
decoder logic and generated mapping representation are original MIT source in
[`gb18030.rs`](../crates/caj2pdf-core/src/caj/gb18030.rs). The source corpus
itself contains no four-byte GB18030 title among the ten successful CAJ files,
so four-byte behavior is independently exercised with synthetic tests rather
than claimed as corpus compatibility.

The Python and Go projects below are behavioral references, not source-code
templates. A format fact may be cited with its location, but implementation
must be independently designed and tested. In particular, do not copy or
transliterate any Python, Go, FreeType, LGPL, GPL, or unlicensed code.

## Black-box reference tools

| Tool | Use | Provenance boundary |
| --- | --- | --- |
| [Python caj2pdf](https://github.com/rwv/caj2pdf) | Compare successful conversion results, page counts, outlines, and reported unsupported cases. | Run a pinned revision as an external oracle; capture the command, revision, input digest, and observed result. Do not import its source or bundled libraries. |
| External HN/C8 type-0 decoder from the pinned Python project | Measure raw 1 bpp image hashes for [issue #22](https://github.com/rwv/caj2pdf-rust/issues/22). | Build and run only in a disposable external environment. Record the revision, compiler, library digest, ABI, timeouts, and secondary PDF cross-check. Its GLWT-licensed implementation and binary must never enter this MIT repository, build, package, or release. |
| External standard T.82 command-line encoder/decoder | Test a finite set of reconstructed BIH/stripe hypotheses for [issue #27](https://github.com/rwv/caj2pdf-rust/issues/27). | Run `pbmtojbg`/`jbgtopbm` only as separately supplied black-box tools. Record binary digests and exact flags. Do not import their source, generated bitmaps, or executable binaries into the project or release. |
| Poppler `pdfimages`, MuPDF `mutool`, and `qpdf` | Check temporary single-image JBIG2 PDFs and compare normalized PBM pixels for [issue #43](https://github.com/rwv/caj2pdf-rust/issues/43) and generic-only [issue #51](https://github.com/rwv/caj2pdf-rust/issues/51). | Invoke only as external black-box development tools. Record versions and binary digests in the [full-image manifest](../tests/conformance/jbig2_oracle.json) and [generic-only manifest](../tests/conformance/jbig2_generic_oracle.json). Dynamic linkage differs, but independent decoder implementations are unverified; report tool agreement only. Do not read, copy, vendor, link, or ship their code or generated bytes. |
| [Go prototype](https://github.com/rwv/caj2pdf-go) | Compare the limited cases it implements when useful. | Pin the revision and record its limitations. Do not treat an unfinished result as proof of compatibility or import source. |
| PDF readers and validators | Independently validate generated PDF structure and rendering. | Record the exact tool and version in the test report when introduced. A reference converter alone cannot establish PDF validity. |

The private Rust prototype is a migration candidate only, not a baseline for
HN or JBIG. No part of its HN parser or CAJ-specific JBIG/JBIG2 decoder may
be migrated, even if the file appears otherwise reusable.

## Test material

| Origin | Repository status | Required record |
| --- | --- | --- |
| Small, independently authored synthetic fixtures | Allowed after their authorship and MIT redistribution rights are documented in the adding pull request. | Generator/source path, the behavior it exercises, and a meaningful assertion. |
| [CAJSamples](https://github.com/caj2pdf/CAJSamples) | External, optional compatibility corpus collected from issue reports. No document redistribution grant is documented for this project; do not commit, vendor, package, or fetch them in the required clean-clone CI path. | Corpus revision, selected relative paths or digests, reference-tool revision, and results. Report missing corpus tests as **skipped**, never as successful compatibility tests. |
| User-provided documents | Local testing only unless explicit redistribution rights are documented. | Record a digest and relevant format facts without publishing the document. |

Issue #2 added no fixtures or external corpus. Issue #3 adds original MIT
fixtures from [`scripts/generate_fixtures.py`](../scripts/generate_fixtures.py),
with conditions, hashes, and authorship recorded in the
[fixture manifest](../tests/fixtures/manifest.json) and
[fixture note](../tests/fixtures/README.md). The external corpus is indexed by
a [metadata-only matrix](../tests/conformance/matrix.json) at a pinned commit.
Required unit tests build and run from a clean clone without external CAJ
documents; a requested corpus run verifies local files separately.

Issue #5's `crates/caj2pdf-core/tests/pdf_validation.rs` creates original
synthetic PDF and PGM bytes during the test. Installed `cjpeg` encodes the PGM
as a valid grayscale JPEG at test runtime. No binary JPEG fixture is checked
in, and the test compares the extracted JPEG and independently rendered pixels.

## Source migration register

The issue #2 source files are `crates/caj2pdf-core/src/lib.rs`,
`crates/caj2pdf-cli/src/main.rs`, `crates/caj2pdf-cli/tests/unimplemented.rs`,
`crates/caj2pdf-wasm/src/lib.rs`, `scripts/check-coverage.sh`, and
`scripts/check-source-inventory.sh`. They are original code written for this
repository under MIT;
**no legacy source files have been migrated**. Register each proposed private
Rust file below before bringing its code into a pull request. A reviewer must
verify the original author and right to grant
MIT, the complete file history, incorporated snippets and generated content,
and transitive source it derives from. An uncertain origin means no migration;
write a fresh implementation instead.

| Destination file | Private source path and revision | Authorship and MIT-grant evidence | Third-party/derivation review | Reviewer and PR | Decision |
| --- | --- | --- | --- | --- | --- |
| None | — | — | — | — | No migration in issue #2. |

This register is per file, not per crate. A bulk statement that the private
repository is owned by one person does not satisfy the review. HN parsing and
CAJ-specific JBIG/JBIG2 decoding are categorically excluded from migration.

Issue #3 adds `scripts/generate_fixtures.py`, `scripts/conformance.py`, and
their tests as original MIT project code. No source is imported from the
Python or Go references, CAJSamples, or a private Rust prototype. The
fixture PDFs are generated from documented PDF syntax rather than converted
from external documents.

Issue #4 adds the original MIT I/O contract in
`crates/caj2pdf-core/src/{error,io,limits,native,operations}.rs`,
`crates/caj2pdf-core/examples/native_bounded_copy.rs`, and
`crates/caj2pdf-core/tests/io_contract.rs`; the original MIT raw WASM bridge
in `crates/caj2pdf-wasm/src/bridge.rs`, and the original MIT browser/Node I/O
proof adapters, examples, and tests in `js/io.mjs`, `js/node.mjs`,
`js/examples/`, and `js/test/*.test.mjs`.
No private or legacy implementation code, nor external format facts, were
imported. The I/O proof copies bytes; it does not establish CAJ-to-PDF
compatibility.

Issue #5 adds the original MIT forward-only PDF writer and document builder in
`crates/caj2pdf-core/src/pdf/{mod,writer,document}.rs`, exports them from
`crates/caj2pdf-core/src/lib.rs`, and adds original MIT tests in
`crates/caj2pdf-core/tests/{pdf_writer_low_level,pdf_document,pdf_validation}.rs`.
The tests generate their PDF inputs during execution; no source or document
was migrated from a reference converter, private prototype, or CAJSamples.

Issue #6 adds original MIT PDF input, incremental update, and fragment repair
code under `crates/caj2pdf-core/src/pdf/`, independent integration tests under
`crates/caj2pdf-core/tests/`, and a synthetic malformed PDF case to
`scripts/generate_fixtures.py`. No parser or decoder source from Python, Go,
the private Rust prototype, or a PDF library is migrated.

Issue #7's CAJ metadata, TOC parsing, GB18030 title decoding, fragment
scanner, page-tree reconstruction, and narrowly classified link/stream
repairs are independently authored MIT code under
`crates/caj2pdf-core/src/{caj,pdf/input,pdf/fragment.rs}`. The small native
CAJ example and the WASM/JavaScript error-category additions are also
original MIT code. The mapping values are generated from the black-box
queries documented above. No CAJ parser, PDF repair logic, or character-
decoder source was migrated from Python, Go, a private Rust module, or
another library.
The original MIT [`caj_conversion.rs`](../crates/caj2pdf-core/tests/caj_conversion.rs)
tests construct synthetic CAJ bytes at runtime from the independently recorded
fields in the [CAJ format note](caj-format.md), including a four-byte GB18030
title. They do not contain CAJSamples document bytes or a reference PDF.

Issue #22's [HN/C8 image-oracle note](jbig1-oracle.md) records independent
container and DIB byte measurements for 27 SHA-256-pinned external files, a
metadata-and-hash-only manifest of 1,400 type-0 images, and two secondary
PDF-image comparisons. The external GLWT-licensed library from the pinned
Python project was compiled and executed only under `/tmp` as a black-box
behavioral oracle. Its source, binary, output bitmaps, and reference PDFs are
not migrated, vendored, linked, or included in release artifacts. The
optional stdlib-only oracle runner and its synthetic tests are original MIT
project code. Future Rust JBIG1 logic must be independently authored from
the public T.82 recommendation and measured input/output behavior; the
external library is not an algorithm source.

Issue #27's [bitstream investigation](jbig1-bitstream-investigation.md) uses
the #22 source/hash inventory, separately supplied standard T.82 command
line tools, native decoder calls only as a black-box oracle, and observations
from reference PDFs and MuPDF renders. The optional
[finite standard probe](../scripts/jbig1_standard_probe.py) is original MIT
stdlib-only code; it uses the pinned inventory and a separately supplied
standard CLI and records only hashes and counts, not bitmap or bitstream
bytes. The investigation note includes a two-byte SCD observation produced
from an independently authored synthetic blank bitmap by the external
standard encoder; it includes no corpus payload bytes. A separate original
experiment under `/tmp` read
the official T.82 table and conformance vector at runtime from the standard
to test hypotheses; no literal standard table or vector is committed. The
standard's exact numeric state-table redistribution under MIT remains a
provenance decision for #26, not an implicit grant from this investigation.
The [Rust row-model result ledger](../tests/conformance/jbig1_row_model_results.md)
is a hash-only record of a separate, independently authored temporary Rust
experiment using the MIT `caj2pdf-core::qm` API. It checked all 27 pinned
source files, including six without type-0 images, and all 1,400 type-0 images
across the other 21 files. Its 1,400 manifest-indexed rows record exact hash
matches and bounded resource counters, not corpus or decoded bitmap bytes.
The temporary harness loaded the T.82 probability states only from an
external runtime fixture; the states, conformance vector, corpus documents,
and bitmap spools were not migrated. The source and full external report
digests are pinned in the ledger README. The Table 24 rights question remains
open in #30, so this result does not authorize bundling those numeric states.
The [portable opt-in Rust harness](../crates/caj2pdf-core/tests/qm_caj_oracle_external.rs)
is an original MIT adaptation of that research probe. It uses the already
MIT-licensed arithmetic core, bounded source hashing and row buffers, a
unique temporary spool, and an external hash-checked probability fixture.
It embeds no corpus bytes, oracle pixels, Table 24 rows, or official vector.
Issue #55 replaced the harness's experimental row assembly with the new
independently authored public [`jbig1::Type0Decoder`](../crates/caj2pdf-core/src/jbig1.rs).
The [row API note](jbig1-type0-rows.md) records the exact wrapper checks,
context order, streaming memory bound, and opt-in local 1,400-image result.
The new decoder's source, comments, and invented-table tests were authored
from T.82 and the already published #27 behavior specification; no private
Rust module or differently licensed converter source was migrated. An additive
QM snapshot counter records physically fetched bytes without adding table
states or standard-vector bytes. Table 24 remains external under #30.

Issue #26's [arithmetic-core design](t82-arithmetic-core.md) is derived from
the English ITU-T T.82 (03/1993) publication, official PDF SHA-256
`6d4280f4402ce285199b3835dda54e35372e8378e7352d2e88ab3ac420f46942`.
It covers §6.2.5, §6.8.2.3/Table 24, §6.8.3.1–§6.8.3.9, and §7.1/Table 26.
The ITU component list also includes Technical Corrigendum 1 (03/1995) and
Technical Corrigendum 2 (03/2001); their Table 24 impact is not yet assessed.
The proposed local official-vector fixture with three Table 26 checkpoints
is outside the repository at
`/tmp/caj26-official-vector-with-checkpoints.txt`, SHA-256
`11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`;
the path is an example from the current local study, not a CI dependency.
The original MIT [opt-in integration test](../crates/caj2pdf-core/tests/qm_official_external.rs)
passed all 256 §7.1 symbols and three Table 26 A/C/CT checkpoints with this
external fixture on 2026-09-24. Ordinary clean-clone tests ignore that check;
its absence must not be reported as standard conformance or CAJ support.
No Table 24 tuples, official §7.1 vector bytes, or generated equivalents are
approved for the MIT source tree, tests, build output, or release artifacts.
The [ITU guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf)
§2.2.2 discuss unrestricted implementation use of data structures and
streams, but do not explicitly classify this numeric table or grant MIT
redistribution of it; the cited guideline edition took effect in 2012,
after this 1993 standard.
The [ITU software declaration database](https://www.itu.int/net4/ipr/search.aspx?class=SW&sector=ITU)
does not list T.82 and disclaims completeness. T.82 is published as identical
to ISO/IEC 11544:1993, so any permission request must address whether ITU can
cover the joint material. Written rights confirmation or a qualified legal
assessment must precede any proposal to embed the exact table under MIT.
The explicit decision gate is [issue #30](https://github.com/rwv/caj2pdf-rust/issues/30),
which blocks completion of #26 and #28 while unresolved. A local conformance
pass with an externally supplied table does not satisfy that gate. As of
2026-09-24, the rights outcome is **unresolved** and independent review is
pending; this note is an interim research record, not an approval to embed
the table.

Issue #40 adds the original MIT segment-header reader in
[`crates/caj2pdf-core/src/jbig2/mod.rs`](../crates/caj2pdf-core/src/jbig2/mod.rs),
exports that module from the existing
[`lib.rs`](../crates/caj2pdf-core/src/lib.rs), adds independently chosen
synthetic header bytes and assertions in
[`jbig2_segment_header.rs`](../crates/caj2pdf-core/tests/jbig2_segment_header.rs),
and records the implementation boundary in the original English
[header note](jbig2-segment-header.md). Its format facts come only from the
English [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
§§5.4.2 and 7.1–7.3, with Annex D used to distinguish contiguous segments
from standalone file organizations. No source was migrated from Python, Go,
private Rust, or any third-party decoder. No T.88 example, Annex E table, or
Annex H vector bytes were copied into code, tests, or documentation. The
reader consumes only a caller-delimited segment and does not establish
JBIG2 pixel or HN/C8 compatibility; those external checks are `NOT_RUN`.
The T.88 Annex E numeric-state redistribution basis remains to be reviewed
before bundling exact states or an integrated decoder under MIT.

Issue #45 adds the original MIT, table-supplied MQ control-flow module in
[`crates/caj2pdf-core/src/jbig2/mq.rs`](../crates/caj2pdf-core/src/jbig2/mq.rs),
original invented-state tests in
[`mq_core.rs`](../crates/caj2pdf-core/tests/mq_core.rs), and an ignored
[external-only Annex H.2 test](../crates/caj2pdf-core/tests/mq_t88_external.rs).
The [T.88 MQ note](t88-mq-core.md) lists the exact official clauses, API
limits, fixture format and digest, and local conformance result. The official
PDF and its Table E.1/Annex H.2 extraction remain only under `/tmp`; source
contains a SHA-256 digest but no normative row, vector, or pixel bytes. No
Python, Go, private Rust, or differently licensed decoder source was read or
migrated. The module borrows the MIT I/O contracts in this repository and
adds no runtime dependency; the external harness uses the already registered
MIT-selected `sha2` development dependency. Standard-vector agreement does
not prove CAJ or JBIG2 image decoding. The source-distribution question for
the exact T.88 table is unresolved in [#44](https://github.com/rwv/caj2pdf-rust/issues/44):
the official text permits alternative implementations to reproduce normative
output, but the currently reviewed ITU materials provide no explicit MIT
redistribution grant for Table E.1. This is a provenance status, not a legal
conclusion about whether numeric state rows are copyrightable.

Issue #42 refactors the original header parser in
[`jbig2/mod.rs`](../crates/caj2pdf-core/src/jbig2/mod.rs) into one bounded
prefix reader and adds the original MIT
[`jbig2/directory.rs`](../crates/caj2pdf-core/src/jbig2/directory.rs).
Its segment-number, page-association, reference-type, and retention rules
come from the English T.88 (02/2000) §§7.1–7.4 and Annex D.3. The
independently chosen synthetic headers and assertions in
[`jbig2_directory.rs`](../crates/caj2pdf-core/tests/jbig2_directory.rs),
the Rust [inventory executable](../crates/caj2pdf-core/examples/jbig2_directory_inventory.rs),
the English [directory note](jbig2-directory.md), and the read-only
[`jbig2_directory_inventory.py`](../scripts/jbig2_directory_inventory.py)
driver are project-owned MIT work. The Python driver reuses only the existing
MIT `conformance.py` and `jbig1_oracle.py` helpers for pinned SHA checks and
HN/C8 container metadata; it never loads or calls the optional external
JBIG1 decoder. Rust checks every type-3 embedded directory. No legacy
Python/Go converter source, private Rust source, third-party decoder source,
official table/vector bytes, external document, or generated PDF was copied
or committed. The 546/546 external header metadata pass is separate from
decoded-pixel checks; pixel parity was `NOT_RUN` for this directory PR.

Issue #51 adds original MIT
[`jbig2_generic_oracle.py`](../scripts/jbig2_generic_oracle.py),
[synthetic tests](../tests/conformance/test_jbig2_generic_oracle.py),
and a [metadata-only manifest](../tests/conformance/jbig2_generic_oracle.json).
It reuses the MIT #42 inventory and #43 source, PDF, PBM, and tool helpers;
it never parses HN/C8 containers again or imports converter/decoder source.
The 546 SHA-pinned external cases are tested with only their original #0 and
#4 JBIG2 segments in temporary PDFs. Only source, selected-span, and
normalized-pixel hashes, geometry, black-pixel counts, and black-box tool
identities enter Git. The document bytes, JBIG2 bytes, PDFs, PBMs, tool
binaries, and normative T.88 state rows remain outside the repository. A
tool agreement is not proof of distinct backend code or Rust pixel parity.

Issue #49 adds the original MIT template-2 generic-region decoder in
[`jbig2/generic.rs`](../crates/caj2pdf-core/src/jbig2/generic.rs), its
invented-state synthetic tests in
[`jbig2_generic.rs`](../crates/caj2pdf-core/tests/jbig2_generic.rs), and an
ignored [external-only two-spot test](../crates/caj2pdf-core/tests/generic_t88_external.rs).
The [generic-region note](jbig2-generic-template2.md) records the official
T.88 clauses, context-bit assignment, bounds, failure semantics, and local
black-box pixel comparison. Only the official T.88 (02/2000) text and this
repository's MIT MQ and I/O APIs informed the implementation. The exact
Table E.1 rows, source CAJ documents, and PDF/PBM outputs remain external;
no differently licensed decoder or private Rust source was read or migrated.
The caller-supplied table's redistribution question remains open in #44.
Two SHA-verified generic-only spots passed locally during #49; ordinary CI
marks that optional two-spot check `NOT_RUN`. The full generic-only result is
recorded below under #50.

Issue #50 adds the original MIT
[`jbig2_generic_parity.py`](../scripts/jbig2_generic_parity.py) driver,
[`jbig2_generic_parity.rs`](../crates/caj2pdf-core/examples/jbig2_generic_parity.rs)
native probe, and [synthetic tests](../tests/conformance/test_jbig2_generic_parity.py).
The [parity note](jbig2-generic-parity.md) records its exact optional inputs,
failure semantics, 546/546 generic-only Rust-to-baseline result, native
memory and I/O measurements, and unmeasured WASM runtime memory. The work
reuses only this repository's MIT inventory, black-box oracle, and row
decoder. Official T.88 states are read from a SHA-pinned private `/tmp`
fixture at test time and never bundled. External documents and generated
PDF/PBM/pixel bytes stay outside Git. Rust generic-region hash agreement is
not full-image or complete conversion parity; #44 still governs table rights.

Issue #54 adds the original MIT Annex A.2 procedure in
[`jbig2/integer.rs`](../crates/caj2pdf-core/src/jbig2/integer.rs) and the
[integer note](t88-arithmetic-integer.md). Only the official T.88 (02/2000)
text and this repository's MIT MQ API informed the implementation. The
test bit streams and invented MQ state table are project-owned; no exact
Table E.1 states, Annex H bytes, external CAJ documents, converter source,
private Rust source, or differently licensed decoder code entered this
repository. An independently measured dictionary integer trace is not yet
available, so external integer compatibility is `NOT_RUN` with zero claimed
cases. The official probability table's distribution question remains in
#44; this caller-table layer does not claim symbol-dictionary decoding.
The follow-up reset rule comes from T.88 §7.4.2.2 steps 3–5 and 7: integer
statistics are zeroed for each symbol dictionary while generic/refinement
bitmap statistics can be restored or retained. The new integer-only reset
preserves appended model contexts; the full reset remains available.

## Dependency inventory and review

The issue #2 baseline contains three owned packages:

| Package | Role | License | Edition / minimum Rust | External dependencies |
| --- | --- | --- | --- | --- |
| `caj2pdf-core` | Platform-neutral library | MIT | 2024 / 1.85.0 | None |
| `caj2pdf-cli` | Linux executable | MIT | 2024 / 1.85.0 | None |
| `caj2pdf-wasm` | WASM/JavaScript boundary | MIT | 2024 / 1.85.0 | None |

The Rust standard library and compiler-provided target components are not
third-party Cargo dependencies. There is no npm package yet. The root
`Cargo.lock` is committed. Every future dependency change must update this
inventory with the package name, version, purpose, resolved features, license
expression, selected license grant, and native/WASM inclusion. For a
dual-licensed package such as
`MIT OR Apache-2.0`, explicitly select and record the **MIT** grant and retain
its license notice. A license string in a manifest is only a starting point:
read the distributed license files and inspect vendored code, generated files,
build scripts, proc macros, and native libraries before accepting an artifact.
An unknown license, missing evidence, or no MIT grant is a review failure
until resolved.

The issue #3 conformance and fixture scripts use only the Python standard
library. `mutool` is an optional local black-box PDF inspector and renderer
for requested output comparisons; neither its source nor its output is
distributed here. Its exact version belongs in each measured baseline.

Issue #4 retains the three-package, dependency-free Cargo lockfile. A
`wasm-bindgen` candidate was rejected: its transitive `unicode-ident`
generated tables require the Unicode license in addition to an MIT grant.
Pinning an older metadata version would not change the origin of those tables.
The chosen raw WASM ABI and JavaScript adapters are original MIT code with no
external Cargo or npm packages.

Issue #5 adds no runtime shell command or external code dependency. Its
integration tests invoke installed `qpdf`, MuPDF `mutool`, and Poppler
`pdfinfo` and `pdfimages` as independent PDF validators, and libjpeg-turbo
`cjpeg` to encode one synthetic JPEG at test runtime. These tools are not
linked, vendored, or distributed with this project. The local baseline used
`qpdf` 12.2.0, `mutool` 1.25.1, Poppler 25.03.0, and libjpeg-turbo 2.1.5.
Required CI installs them and prints their versions before tests and coverage.

Issue #26 adds `sha2` as a **dev dependency only** for the ignored
[`qm_official_external.rs`](../crates/caj2pdf-core/tests/qm_official_external.rs)
test. It rejects a separately supplied T.82 fixture above 16 KiB and checks
its pinned SHA-256 before parsing. The selected grant for each package below
is **MIT** from its distributed `LICENSE-MIT`; every inspected
package manifest says `MIT OR Apache-2.0`. The versions are pinned in
[`Cargo.lock`](../Cargo.lock). Feature and target scopes were checked with
`cargo tree --locked -p caj2pdf-core -e features` for Linux x86_64 and
`wasm32-unknown-unknown` on 2026-09-24.

| Package | Purpose and resolved features | License / selected grant | Inclusion |
| --- | --- | --- | --- |
| `sha2` 0.11.0 | SHA-256 of the external test fixture; direct `default-features = false`. | `MIT OR Apache-2.0` / MIT | Dev/test graph only; absent from normal core, CLI, and WASM artifacts. |
| `block-buffer` 0.12.1 | Digest block buffering; `default`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |
| `cfg-if` 1.0.5 | Hash implementation configuration; `default`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |
| `cpufeatures` 0.3.1 | CPU feature selection; `default`. | `MIT OR Apache-2.0` / MIT | Transitive native x86_64 test graph; absent from wasm32 test graph. |
| `crypto-common` 0.2.2 | Shared digest primitives; `default`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |
| `digest` 0.11.3 | Digest traits and block API; `default`, `block-api`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |
| `hybrid-array` 0.4.15 | Fixed-size digest storage; `default`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |
| `libc` 0.2.189 | OS interfaces for `cpufeatures` on selected architectures; default features disabled through that dependency. | `MIT OR Apache-2.0` / MIT | Locked target-specific transitive package; absent from Linux x86_64 and wasm32 graphs. |
| `typenum` 1.20.1 | Type-level block sizes; `default`, `const-generics`. | `MIT OR Apache-2.0` / MIT | Transitive test graph, Linux x86_64 and WASM test builds. |

The native and WASM license gates passed with `cargo-deny` 0.20.2. In the
local registry source scan, `libc` alone has a Rust `build.rs`; none of these
nine packages contains bundled native C/C++ source or binary artifacts.

Issue #36 adds [flate2 1.1.10](https://crates.io/crates/flate2/1.1.10) as a
normal core dependency with `default-features = false` and `rust_backend`.
The selected **MIT** grant and corresponding distributed license notice were
checked for it and each locked transitive dependency. The pure Rust backend
works in native and `wasm32-unknown-unknown` builds; no C library or runtime
subprocess is linked. `crc32fast` has a Rust build script that queries the
compiler version, not a runtime subprocess. The five runtime packages below
are present in both native and WASM graphs; `cfg-if` 1.0.5 is shared with the
existing test graph. No package code is copied into this repository.

| Package | Purpose and resolved features | License / selected grant |
| --- | --- | --- |
| `flate2` 1.1.10 | Bounded zlib decoding of xref streams; `rust_backend`, `miniz_oxide`, `any_impl`. | `MIT OR Apache-2.0` / MIT (`LICENSE-MIT`) |
| `miniz_oxide` 0.9.1 | Pure Rust Deflate backend; `default`, `with-alloc`, `simd`, `simd-adler32`. | `MIT OR Zlib OR Apache-2.0` / MIT (`LICENSE-MIT.md`) |
| `adler2` 2.0.1 | Adler-32 for zlib; `default`. | `0BSD OR MIT OR Apache-2.0` / MIT (`LICENSE-MIT`) |
| `crc32fast` 1.5.2 | CRC-32 for flate2; `default`. | `MIT OR Apache-2.0` / MIT (`LICENSE-MIT`) |
| `simd-adler32` 0.3.10 | Pure Rust Adler-32 acceleration; `default`. | `MIT` (`LICENSE.md`) |

No external official table/vector bytes or library source are vendored with
this dependency change.

For each pull request and release, regenerate the locked transitive inventory
for the Linux native target and `wasm32-unknown-unknown`, including target-
specific features, build dependencies, and dev dependencies relevant to tests.
The baseline CI uses these commands and [`deny.toml`](../deny.toml):

```sh
cargo tree --locked --all-features --target x86_64-unknown-linux-gnu -e all
cargo tree --locked --all-features --target wasm32-unknown-unknown -e all
cargo deny --locked --all-features --target x86_64-unknown-linux-gnu check licenses sources
cargo deny --locked --all-features --target wasm32-unknown-unknown check licenses sources
```

Inspect any exceptions to the automated license gate manually, and compare
the generated inventory with the actual distribution contents (CLI archive and
JS package). Record any selected MIT grant and required attribution in the
pull request. This manual review supplements the
checker; it cannot be replaced by a passing exit code.

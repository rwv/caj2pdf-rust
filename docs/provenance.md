# Provenance and dependency inventory

This file records the provenance of this repository's own source, fixtures,
test fonts and dependencies, and the rules for adding to them. Keep it
current with every change that adds a format rule, fixture, font, dependency
or proposed private-source migration ([AGENTS.md](../AGENTS.md)).

Research provenance — oracle and harness runs, CAJViewer automation,
per-issue investigation records and the per-issue source history up to
caj2pdf-rust commit `0abee3862f01756ee15f69a1b174a35208fc1e41` — moved with
the research tooling to caj2pdf-samples in
[#360](https://github.com/rwv/caj2pdf-rust/issues/360). The complete former
version of this file is kept verbatim there as the
[provenance archive](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/provenance-archive.md);
links to `research/…` notes below point into the same repository.

## Format references

| Format or feature | Reference | Status and permitted use |
| --- | --- | --- |
| PDF output and PDF input | [ISO 32000-1:2008](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf) and the [PDF specification archive](https://pdfa.org/resource/pdf-specification-archive/) | Published format specifications. Record the exact PDF version and clauses used for each implementation change. Link to the documents; do not copy their text into source. |
| JBIG / JBIG2 bitstreams | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82) and [ITU-T T.88](https://www.itu.int/rec/T-REC-T.88/en) | Published coding recommendations. Implement the subset required by observed CAJ-family data as original MIT code. Do not reuse reference implementation source. |
| T.82 arithmetic SCD core and numeric states | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82), §6.2.5, §6.8.2.3/Table 24, §6.8.3, and §7.1/Table 26; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | Use the public algorithm to author original MIT Rust code. Keep Table 24's 113 exact numeric rows and the §7.1 vector outside the repository until their MIT redistribution basis is documented. The [core design](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t82-arithmetic-core.md) records the external-table contract and local test procedure; standard conformance does not establish CAJ compatibility. |
| T.88 MQ arithmetic control flow and numeric states | [ITU-T T.88 (02/2000), unamended base edition](https://www.itu.int/rec/T-REC-T.88-200002-S/en), also ISO/IEC 14492:2001, Annex E.2.5/Table E.1, E.2.9–E.2.10, E.3.1–E.3.6, and H.2/Table H.1; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | The normative decoder behavior, published 47-state numeric rows, original MIT caller-table control flow, and external-only Annex H/HN/C8 conformance inputs are separate materials. Exact Table E.1 rows and Annex H vector/checkpoints remain outside Git, artifacts, and releases. The [#44 rights record](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-mq-rights.md), reviewed 2026-09-27 by Codex repository research and an independent Codex reviewer, remains **UNRESOLVED**: neither an exact-state MIT redistribution grant nor an independently derived 47-state model is established. This is a technical provenance review, not legal clearance. The [core note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-mq-core.md) records bounded API and external-only checks. Annex H.2 verifies arithmetic decisions, not CAJ/JBIG2 pixels; observed HN/C8 modes and reachable states are corpus observations, not universal guarantees. |
| T.88 non-IAID arithmetic integers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.1–A.2 and E.3, with symbol-dictionary usage in §§6.5 and 7.4.2 | Original MIT 13-bank integer decision layer over the shared MQ decoder. The [integer note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-arithmetic-integer.md) records its signed/OOB result, 512-context layout, 38-decision limit, and synthetic checks. No Table E.1 states, external dictionary trace, or HN/C8 compatibility claim is included. |
| T.88 fixed-length IAID symbol IDs | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.3 and E.3, §§6.4.2, 6.4.10, 6.5.8.2.3, 7.4.2–7.4.3 | Original MIT IAID decision layer over the shared MQ stream, with its contexts at a fixed offset of the coding unit. The [IAID note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-iaid.md) records its fixed-width context map, bounded allocation and work, reset policy, symbol-array guard, and synthetic checks. No official state rows, external trace, or HN/C8 parity claim is included. |
| T.88 direct-coded arithmetic symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5, 6.5.1–6.5.10, 7.4.2.1–7.4.2.2, Tables 16 and 28, Annex A.2 and E.3.7–E.3.8; [repository-owned header inventory](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded direct path of the one symbol-dictionary decoder. The [dictionary note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-symbol-dictionary-direct.md) records classification, MQ/context ownership, store contract, limits, and optional evidence. The same decoder refines the observed second dictionary (below). Exact Table E.1 rows remain external under #44; metadata checks do not establish symbol pixel parity. |
| T.88 template-1 generic refinement bitmaps | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.3.2–6.3.5, Table 6, Figure 13, §6.5.8.2/Table 18 | Original MIT, bounded single-reference bitmap primitive. The [refinement note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-refinement-template1.md) records the ten-pixel context mapping, typed IAID/GR context ownership, reference store, row memory, error contract, and synthetic tests; since #355 the reference store is an in-memory bitmap. The bitmap primitive alone does not decode a `0x1802` dictionary; #66 integrates its one-reference path. No external symbol-pixel oracle exists: refinement compatibility is `NOT_RUN`, zero cases. Exact Table E.1 rows remain external under #44. |
| T.88 arithmetic single-reference symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.10–6.4.11, 6.5.5–6.5.10, 7.4.2.1–7.4.2.2, Tables 17–18, Annex A; [repository-owned header inventory](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded refinement path of the same symbol-dictionary decoder for the observed `0x1802` second dictionary when every IAAI is one. The [integration note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-refinement-dictionary.md) records imported/new stores, ordered export handles, MQ state, limits, typed zero/aggregate refusals, and synthetic tests. The private 546-case trace is diagnostic; independent symbol-pixel compatibility remains `NOT_RUN`, zero proven cases. Exact Table E.1 rows remain external under #44. |
| T.88 text-region data headers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.3.1–7.4.3.1.4, Figures 28–29 and 35–38; committed #43 oracle text flags | Original MIT, bounded header parser with no body reads. The [text-region note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-region-header.md) records validation order and optional metadata inventory. Strict parsing remains the default; the [#88 policy note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-header-compatibility.md) documents one explicitly opted-in `0xa40c` HN/C8 exception and the preserved anomaly marker. Metadata alone establishes neither placement nor pixel compatibility. |
| T.88 arithmetic text instances | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.5–6.4.11, 7.4.3.1–7.4.3.2, Table 12, Annex A and E.3.7; [#85 hash-only text oracle](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig2-text-oracle.md) | Original MIT, bounded pull decoder and optional SHA-pinned control-flow diagnostic. The [instance note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-instances.md) records context ownership, strip/RI decisions, store handles, limits, failure contract, 545 complete strict-region traces, and one strict anomaly refusal. Its event fingerprint is not independent pixel evidence; #87 supplies a separate text-only pixel comparison. #44 governs exact Table E.1 rights. |
| T.88 text-region composition | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.1–6.4.5 and 7.4.3.2, Tables 9–11; [#85 hash-only text oracle](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig2-text-oracle.md) | Original MIT composition of checked #86 instances into a bounded page bitmap (caller-owned random-access scratch until #355, in memory since), followed by sequential packed-row output. The [composer note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-composer.md) records clipping, combination, adapter ownership, limits, and optional private pixel comparison. No external decoder code, document bytes, decoded bitmap, or exact Table E.1 states are committed. |
| Observed HN/C8 type-3 JBIG2 page composition | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.8, and 8.2; [#43 full-image oracle](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig2-oracle.md) | Original MIT parser/preflight and bounded OR row output for only the five-segment profile observed in 546 external records. The [page note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-observed-page-composition.md) records segment and region constraints, the text bitmap (caller-owned scratch until #355, in memory since), budgets, and failure semantics. The later [#95 private comparison](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig2-page-parity.md) matched the observed full-page pixels; clean-clone corpus parity remains `NOT_RUN`/zero. Exact MQ state rows remain external under #44. |
| T.88 template-2 arithmetic generic regions | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5.2–6.2.5.4, 6.2.5.7, 7.4.1, 7.4.6.1–7.4.6.4, Table 34, Figure 5, E.3.7 | Original MIT, bounded row decoder on the shared MQ decoder. Two external generic-only HN/C8 pixel spots passed; all 546 remain for #50. The [region note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig2-generic-template2.md) records the context order, bounds, and external-only verification. |
| CAJ-family headers, pages, and outlines | [caj2pdf format notes](https://github.com/caj2pdf/caj2pdf/wiki), including [CAJ/HN identification](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E5%92%8C-HN), [basic information and outlines](https://github.com/caj2pdf/caj2pdf/wiki/%E6%96%87%E4%BB%B6%E5%9F%BA%E6%9C%AC%E4%BF%A1%E6%81%AF%E4%B8%8E%E5%A4%A7%E7%BA%B2), and [CAJ page content](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9) | Public observations, not a complete normative specification. [Repository-owned CAJ measurements](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/caj-format.md) pin ten successful sample digests and document TOC, page-table, and PDF-fragment exceptions independently. Do not copy parser source or pseudocode. |
| HN/C8 container record reader | [Repository-owned #22 measurements](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-oracle.md), [#61 read-only interval inventory](https://github.com/rwv/caj2pdf-rust/issues/61#issuecomment-5825547234), and the [bounded container note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-container.md) | Original MIT Rust reader of only the three measured variants. The optional hash-only comparison checks external record coordinates, not conversion or codec support. Cross-page alias policy, unknown page fields, and resource ceilings are documented in the note. No Python, Go, private Rust, wiki decompilation, or differently licensed parser source was used. |
| HN/C8 and KDH structure report (`inspect --pages`) | The existing [container reader](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-container.md), page-text and native-record readers, the [application-info trailer observation](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-records.md#application-info-tail-and-source-coverage), and the [KDH signature note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/kdh-format.md) | Original MIT diagnostic glue (#301) that reports which existing reader accepts each page, with spans, counts and located errors only. It adds no format interpretation: the `APPINFOSIGN <decimal offset>` trailer is located with the #302 package reader's locator, and the report does not decode its section. Tests use synthetic containers only; no corpus bytes or reports are committed. |
| HN/C8 type-0 image wrapper and pixels | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), and [repository-owned oracle measurements](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-oracle.md) | The standards describe public coding and DIB fields. The local corpus measurements pin the CAJ-family wrapper and output hashes. The external differently licensed native decoder is a black-box oracle only, never implementation source or a project dependency. |
| HN/C8 type-0 row primitive | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82-199303-I/en) §§6.5, 6.7.1, 6.8.3; [independent #27 observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-row-model.md) | [Issue #55's decoder](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-type0-rows.md) is original MIT code using a caller-supplied table and bounded rows. The exact T.82 Table 24 values and official vector stay external pending #30. Its opt-in 1,400-image check uses only hashes and a private runtime fixture; it is not a released HN/C8 converter. |
| HN/C8 type-0 PDF pages | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82-199303-I/en) §6.8 (interval convention for the test-only encoder), [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §4.8 (1 bpp image samples, `/Decode`), and the repository's [#22](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-oracle.md)/[#27](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-bitstream-investigation.md) palette and orientation observations | The [#28 core converter](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-type0-pdf.md) is original MIT glue between the existing reader, row decoder, and PDF writer, with a caller-supplied table. Its polarity and top-down placement follow the repository's own measurements. The invented-table test encoder is original test code. No Table 24 rows, corpus bytes, pixels, or external decoder source are included. |
| HN/C8 type-2 JPEG marker profile | [CCITT/ISO T.81 Annex B](https://www.w3.org/Graphics/JPEG/itu-t81.pdf), [ITU T.81 catalog](https://www.itu.int/rec/T-REC-T.81), [ITU/ISO T.871 JFIF](https://www.itu.int/rec/T-REC-T.871-201105-I/en), and the original [#22/#61 container observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-container.md) | Original MIT marker/profile reader over a checked type-2 HN/C8 descriptor and bounded ranged input. It derives only functional marker syntax from the standards, with no copied tables, figures, examples, tests, or decoder software. The [profile note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-type2-jpeg.md) records the observed JFIF 1.01 compatibility subset and separates marker classification from JPEG entropy decoding and PDF color/placement. The final-source private run matched 1,085/1,085 pinned type-2 descriptors and headers, with 27/27 unchanged source identities and zero failed/unsupported/skipped records; this is no pixel or PDF parity claim. Private CAJSamples sources and JPEG payload bytes stay external; clean-clone corpus compatibility is `NOT_RUN`/zero. |
| KDH wrapper and XOR payload | Three SHA-pinned CAJSamples files measured independently at commit `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07` | The [KDH format note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/kdh-format.md) records exact identities, offset 254, the `FZHMEI` cycle, EOF/trailer measurements, and negative controls. The clean-room author derived this code without consulting converter source. |
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

The standalone GB18030 title decoder's mapping data was generated by querying
Python 3.13.5's `gb18030` codec as a **black box** (all 23,940 two-byte and
1,587,600 four-byte candidates), not by reading or copying its implementation
or tables. The decoder and its 207-range mapping are original MIT source in
[`gb18030.rs`](../crates/caj2pdf-core/src/gb18030.rs) and
[`gb18030/tables.rs`](../crates/caj2pdf-core/src/gb18030/tables.rs); four-byte titles are
exercised by synthetic tests, not claimed as corpus compatibility.

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
| Poppler `pdfimages`, MuPDF `mutool`, and `qpdf` | Check temporary single-image JBIG2 PDFs and compare normalized PBM pixels for [issue #43](https://github.com/rwv/caj2pdf-rust/issues/43), generic-only [issue #51](https://github.com/rwv/caj2pdf-rust/issues/51), and text-only [issue #85](https://github.com/rwv/caj2pdf-rust/issues/85). | Invoke only as external black-box development tools. Record versions and binary digests in the [full-image manifest](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_oracle.json), [generic-only manifest](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_generic_oracle.json), and [text-only manifest](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_text_oracle.json). Dynamic linkage differs, but independent decoder implementations are unverified; report tool agreement only. Do not read, copy, vendor, link, or ship their code or generated bytes. |
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

The per-issue source records that followed this register (issues #3
through #302) are in the
[provenance archive](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/provenance-archive.md#source-migration-register).
No private or legacy source file has been migrated since; a new migration is
registered in the table above.

## Original fixtures and test fonts

`tests/fixtures/` holds original MIT inputs generated by
[`scripts/generate_fixtures.py`](../scripts/generate_fixtures.py); the
[fixture manifest](../tests/fixtures/manifest.json) and
[fixture note](../tests/fixtures/README.md) record each file's hash, purpose
and authorship. `tests/fonts/` holds the original geometric and symbol test
fonts described in [its note](../tests/fonts/README.md); they are generated by
`crates/caj2pdf-core/tests/common/font_fixture.rs` and contain no external
glyph data. `tests/conformance/` keeps only the metadata the optional
corpus tests read ([note](../tests/conformance/README.md)).
`crates/caj2pdf-core/tests/pdf_validation.rs` builds its PDF and PGM inputs
at test time and has installed `cjpeg` encode the JPEG; no binary JPEG
fixture is committed.

## Fuzz targets (#294)

`fuzz/` is original MIT harness code. It depends on `libfuzzer-sys`
(MIT/Apache-2.0), which builds LLVM libFuzzer (Apache-2.0 with LLVM exception)
at fuzz-build time. Neither is vendored in this repository, linked into the CLI,
WASM or JS packages, or part of the release inventory. Seeds come from the
original synthetic fixtures in `tests/fixtures`.

## Caller-supplied TrueType metadata (#233)

The original MIT ranged adapter in `pdf/font.rs` follows Microsoft's
[OpenType SFNT structure](https://learn.microsoft.com/en-us/typography/opentype/spec/otff).
It retains only eight metric/character/name tables, at most 1 MiB combined, and
leaves the font program in the caller's ranged source. A maximum of 128 table
entries bounds directory work; table order, duplicates, ranges, alignment
and overlap are checked before payload allocation. Original synthetic
metadata tests contain no copied font outlines or external font data.
This is a resource primitive, not completed native C8 rendering or validation
of every glyph outline. The shared writer now embeds fonts and emits positioned glyphs, segments and
images. C8 style interpretation and complete six-page acceptance remain open
under #233; see [the output contract](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/pdf-native-text.md).

`xberg-ttf-parser` **1.1.0**, normal native and WASM dependency, supplies
borrowed `Face::from_raw_tables` and character/metric APIs. Default features
are disabled; only `std` is enabled. `cargo tree --edges normal,build` shows
no enabled dependencies. The published manifest, README, source API and
complete `LICENSE` were reviewed. The license is **MIT**, copyright
2025–2026 Kreuzberg, Inc. and 2018 Yevhenii Reizner and the ttf-parser
contributors. Preserve both notices through the existing packaging script.
The source is downloaded by Cargo, not vendored into this repository.
Optional layout/variation features and development dependencies are disabled.
No proprietary viewer font is bundled or used as source code.

The minimum Rust version 1.88.0 follows this dependency. Its upstream fixes
and the rejected `ttf-parser` and `read-fonts` alternatives are recorded in
the provenance archive. No license exception, advisory suppression or local
parser fork is used.

### Installed-font discovery (#339)

`crates/caj2pdf-cli/src/system_fonts.rs` is original MIT code. Its search
directories follow the public
[XDG Base Directory specification](https://specifications.freedesktop.org/basedir-spec/latest/)
and the documented macOS and Windows font folders; no Fontconfig or other
font-matching source is used or linked. Faces are matched by PostScript name
(OpenType `name` ID 6) through the existing core reader; the face count comes
from the TrueType collection header. No font is bundled or committed; tests rename the original
`geometric.ttf` fixture's PostScript name at test time and build synthetic
collections from it.

### Original embedded-font and mixed-page fixtures

`pdf/document/text.rs` is original MIT output glue over the existing sequential
writer. CIDFontType2/Identity-H, FontFile2, CIDToGIDMap, widths, ToUnicode,
text matrices and path operators follow Adobe's PDF 1.7 / ISO 32000-1
font and content-stream definitions, available through the
[PDF Association specification archive](https://pdfa.org/resource/pdf-specification-archive/).
Widths use 256-code blocks so neither the outer nor inner array exceeds the
recommended PDF array size. ToUnicode ranges increment only the last byte,
exclude surrogate code units and contain at most 32 entries per block.

The in-repository `drawing_font` test builder creates .notdef plus original
rectangle/triangle outlines, names, cmap, metric/location tables and SFNT
checksums; no external glyph designs or font bytes are used. Its labels `A`
and `中` test Unicode mapping, not letterform design. Test exports and
external fonts remain outside Git.

### Original TrueType subset writer (#335)

`pdf/font/subset.rs` is original MIT code. Table layout, checksums,
`checkSumAdjustment`, `loca` formats and composite-glyph component flags
follow the OpenType specification's
[`glyf`](https://learn.microsoft.com/en-us/typography/opentype/spec/glyf),
[`loca`](https://learn.microsoft.com/en-us/typography/opentype/spec/loca),
[`head`](https://learn.microsoft.com/en-us/typography/opentype/spec/head) and
[font file](https://learn.microsoft.com/en-us/typography/opentype/spec/otff)
chapters. The required `FontFile2` tables and the six-letter subset tag
follow ISO 32000-1 §9.9 and §9.6.4. No subsetter source (fontTools,
HarfBuzz, typst `subsetter` or others) was consulted or copied. Unit tests
build composite fonts from the original geometric fixture outlines; the
corpus comparison in [the native text note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/pdf-native-text.md)
uses caller fonts that remain outside Git.

### TrueType collection faces (#337)

`pdf/font.rs` reads the `ttcf` header (versions 1 and 2) and one face's
table directory as described in the OpenType
[font file](https://learn.microsoft.com/en-us/typography/opentype/spec/otff#font-collections)
chapter; the code is original MIT. The `collection.ttc` test fixture is
generated from the original geometric fonts by
`crates/caj2pdf-core/tests/common/font_fixture.rs`.

### CFF subset writer (#338)

`pdf/font/cff.rs` is original MIT code written from Adobe Technical Note
#5176 (*The Compact Font Format Specification*) and #5177 (*The Type 2
Charstring Format*), via the OpenType
[`CFF ` table](https://learn.microsoft.com/en-us/typography/opentype/spec/cff)
chapter, and ISO 32000-1 §9.7.4 for `CIDFontType0` and `FontFile3`. No CFF
subsetter or desubroutinizer source (fontTools, HarfBuzz, typst `subsetter`,
FreeType or others) was consulted or copied. The `geometric.otf` fixture and
its malformed variants are generated by the original
`crates/caj2pdf-core/tests/common/font_fixture.rs`.

## Native font role fallback and font directory (#290)

The CJK/Latin role fallback in `hnc8/native_page.rs`, the CLI `--fonts DIR`
mapping and their tests are original MIT code. The CJK-coded ranges are
standard Unicode block boundaries. Tests relabel the cmap of this project's
original geometric font; no external glyph data is added. The documented
free recipe (Droid Sans Fallback, Apache-2.0; DejaVu Sans, Bitstream Vera
license) was chosen by checking cmap coverage of the six pinned corpus inputs.
Those fonts stay external: they are not vendored, bundled or copied into
fixtures. Corpus documents and derived PDFs remain outside Git.

## Dependency inventory and review

Third-party crates under a license in the `deny.toml` allowlist (MIT,
Apache-2.0, BSD-2/3-Clause, ISC, Unicode-3.0, Zlib) need no per-file review:
record the crate, version, purpose and selected grant in the table below.
Per-file provenance review still applies to source copied into this
repository and to any CAJ-specific HN or JBIG decoding, which must remain an
independent reimplementation and never a transliteration of the Python or Go
converters.

The workspace contains three owned packages. The dependency column lists
direct third-party Cargo dependencies in the current graph:

| Package | Role | License | Edition / minimum Rust | External dependencies |
| --- | --- | --- | --- | --- |
| `caj2pdf-core` | Platform-neutral library | MIT | 2024 / 1.88.0 | `flate2`, `sha2`, `xberg-ttf-parser` (direct) |
| `caj2pdf-cli` | Native executable | MIT | 2024 / 1.88.0 | `clap`, `serde`, `serde_json`; Unix and Windows: `same-file`, `tempfile`; Unix: `signal-hook`; Windows: `ctrlc`, `winapi-util` |
| `caj2pdf-wasm` | WASM/JavaScript boundary | MIT | 2024 / 1.88.0 | None |

The Rust standard library and compiler-provided target components are not
third-party Cargo dependencies. The `js/` npm package (issue #13) has no
dependencies, dev dependencies, or install scripts. The root
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

Tests invoke installed `qpdf`, MuPDF `mutool`, Poppler (`pdfinfo`, `pdfimages`,
`pdftoppm`) and libjpeg-turbo (`cjpeg`, `djpeg`) as independent black-box
validators. They are not linked, vendored or distributed; required CI
installs them and prints their versions before tests and coverage.

`sha2` hashes the selected JPEG bytes read during marker preflight against
the bytes streamed into the PDF image object (fixed-size state), and pinned
external test fixtures. The selected grant for each package below is **MIT**
from its distributed `LICENSE-MIT`; versions are pinned in
[`Cargo.lock`](../Cargo.lock) and scopes were checked with `cargo tree`.

| Package | Purpose and resolved features | License / selected grant | Inclusion |
| --- | --- | --- | --- |
| `sha2` 0.11.0 | SHA-256 of selected JPEG preflight/copy bytes and external test fixtures; direct `default-features = false`. | `MIT OR Apache-2.0` / MIT | Normal core, CLI, and WASM graphs. |
| `block-buffer` 0.12.1 | Digest block buffering; `default`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs. |
| `cfg-if` 1.0.5 | Hash implementation configuration; `default`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs; also used by `flate2`. |
| `cpufeatures` 0.3.1 | CPU feature selection; `default`. | `MIT OR Apache-2.0` / MIT | Transitive native x86_64 graph; absent from wasm32 graph. |
| `crypto-common` 0.2.2 | Shared digest primitives; `default`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs. |
| `digest` 0.11.3 | Digest traits and block API; `default`, `block-api`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs. |
| `hybrid-array` 0.4.15 | Fixed-size digest storage; `default`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs. |
| `libc` 0.2.189 | OS interfaces for `cpufeatures` on selected architectures; default features disabled through that dependency. | `MIT OR Apache-2.0` / MIT | Locked target-specific transitive package; absent from Linux x86_64 and wasm32 graphs. |
| `typenum` 1.20.1 | Type-level block sizes; `default`, `const-generics`. | `MIT OR Apache-2.0` / MIT | Transitive normal core, CLI, and WASM graphs. |

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
cargo deny --locked --all-features check advisories bans
```

The advisory gate denies yanked crates and known advisories; any ignored
advisory must be listed in `deny.toml` with a reason. The bans gate denies
duplicate package versions and registry wildcard requirements.

Inspect any exceptions to the automated license gate manually, and compare
the generated inventory with the actual distribution contents (CLI archive and
JS package). Record any selected MIT grant and required attribution in the
pull request. This manual review supplements the
checker; it cannot be replaced by a passing exit code.

## PDF stream framing without codecs

Issue #359 removed the `fax` 0.3.0 dependency together with the Group-4,
JPEG, ASCII85 and Flate extent walkers. A headerless CAJ fragment stream now
ends at the `endstream` that its declared or later-resolved `/Length`
confirms, so no stream payload is decoded to frame it.

## CLI cooperative signal handling (review #193)

The CLI uses `signal-hook` 0.4.4 (MIT OR Apache-2.0, used under MIT), with
only its flag API and default features disabled. Its registry dependency
`signal-hook-registry` 1.4.8 is MIT OR Apache-2.0; its `errno` dependency is
MIT OR Apache-2.0. Existing `libc` is MIT OR Apache-2.0. Target-specific
`windows-sys` and `windows-link` are MIT OR Apache-2.0. Versions are pinned in
Cargo.lock. Original project glue only sets/checks cancellation flags; no
external handler implementation was copied into this repository.

## CLI argument parsing and JSON reports (#361)

The CLI parses its arguments with `clap` and writes `inspect --json` with
`serde` and `serde_json`, replacing the hand-written argument parser and JSON
string writer. They are direct dependencies of `caj2pdf-cli` only;
`caj2pdf-core` and `caj2pdf-wasm` do not use them, so they are absent from the
WASM build graph. Every crate below is selected under its MIT grant, whose
notice ships in the crate; versions are pinned in Cargo.lock and each
`rust-version` is at most 1.88.

| Crate | Version | License (selected) | Purpose and resolved features |
| --- | --- | --- | --- |
| `clap` | 4.6.7 | MIT OR Apache-2.0 (MIT) | Argument parser; default features off, `std`, `derive`, `help`, `usage`, `error-context` (no color, terminal or suggestion support) |
| `clap_builder` | 4.6.7 | MIT OR Apache-2.0 (MIT) | Parser runtime of `clap`; `std`, `help`, `usage`, `error-context` |
| `clap_lex` | 1.1.1 | MIT OR Apache-2.0 (MIT) | `OsStr` argument lexer of `clap` |
| `anstyle` | 1.0.14 | MIT OR Apache-2.0 (MIT) | Style types in `clap`'s help text; no escape codes are written |
| `clap_derive` | 4.6.7 | MIT OR Apache-2.0 (MIT) | Proc macro for `#[derive(Parser)]`; build time only |
| `serde` | 1.0.229 | MIT OR Apache-2.0 (MIT) | `Serialize` trait and derive; default features off, `std`, `derive` |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 (MIT) | Trait definitions re-exported by `serde`; `std`, `result` |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 (MIT) | Proc macro for `#[derive(Serialize)]`; build time only |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 (MIT) | JSON writer for the inspection report; default features off, `std` |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 (MIT) | Integer formatting in `serde_json` |
| `memchr` | 2.8.3 | Unlicense OR MIT (MIT) | Byte search in `serde_json`; `std` |
| `zmij` | 1.0.23 | MIT | Float formatting in `serde_json`; the report writes no floats |
| `heck` | 0.5.0 | MIT OR Apache-2.0 (MIT) | Case conversion in `clap_derive`; build time only |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 (MIT) | Proc-macro support; build time only |
| `quote` | 1.0.47 | MIT OR Apache-2.0 (MIT) | Proc-macro support; build time only |
| `syn` | 3.0.6 | MIT OR Apache-2.0 (MIT) | Proc-macro parser; build time only |
| `unicode-ident` | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 (MIT and Unicode-3.0) | Identifier tables for `proc-macro2`; build time only |

No source from these crates is copied into this repository. The JSON output
is unchanged byte for byte: a small `serde_json` formatter keeps the
`\u0008` and `\u000c` escapes of schema version 1.

## CLI temporary files and file identity (#378)

The CLI stages path output and spools forward-only input with `tempfile`,
and compares output and input files with `same-file`, replacing the
hand-written temporary-name scheme, its retry loop, the hard-link commit and
the per-platform identity code. Both are `cfg(any(unix, windows))`
dependencies of `caj2pdf-cli` only; `caj2pdf-core` and `caj2pdf-wasm` do not
use them, so they are absent from the WASM build graph. Every crate below is
selected under its MIT grant, whose notice ships in the crate; versions are
pinned in Cargo.lock and each `rust-version` is at most 1.88 (`same-file`
declares none and builds with 1.88.0).

| Crate | Version | License (selected) | Purpose and resolved features |
| --- | --- | --- | --- |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 (MIT) | Hidden staged output (`Builder::tempfile_in`, `TempPath::persist`, `TempPath::persist_noclobber`) and the anonymous stdin spool (`tempfile_in`); default features off, so `getrandom` is not used |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT (MIT) | Random temporary names in `tempfile`; `std`, `alloc` |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 (MIT) | Lazy statics in `tempfile`; `std`, `alloc`, `race` |
| `rustix` | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (MIT) | Unix system calls of `tempfile` (`O_TMPFILE`, exclusive rename, link, unlink); `std`, `alloc`, `fs`. Unix only. Its build script only probes the compiler for language features; it compiles no native code |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT (MIT) | Generated Linux system-call bindings of `rustix`; Linux and Android only |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 (MIT) | Flag types in `rustix`; `std`. Unix only (already in the Windows graph through `ctrlc`) |
| `same-file` | 1.0.6 | Unlicense OR MIT (MIT) | `Handle` identity: device and inode on Unix, volume serial number and file index on Windows; no features |

Other Unix targets reach the C library through the already audited `errno`
0.3.14 and `libc` 0.2.189. On Windows, `tempfile` uses the already audited
`windows-sys` 0.61.2 with `Win32_Foundation` and `Win32_Storage_FileSystem`,
both already enabled, and `same-file` uses `winapi-util` 0.1.11 (below). No
crate is build-time only, and no source from these crates is copied into
this repository.

## Windows adapter dependencies (#204)

The Windows CLI uses `ctrlc` 3.5.2 (MIT OR Apache-2.0, selected MIT) for safe
console interrupt registration and `winapi-util` 0.1.11 (Unlicense OR MIT,
selected MIT) for the file type query that tells disk files from pipes,
consoles and character devices. Since #378 volume/file identity comes from
`same-file`, which uses `winapi-util` itself. Both use the
already audited `windows-sys` 0.61.2 / `windows-link` 0.2.1 under MIT. Their
published MIT notices and Windows source paths were reviewed; no converter,
codec or vendored native implementation is imported. Project source retains
`forbid(unsafe_code)`; OS FFI is encapsulated by those dependencies.

Both direct dependencies are `cfg(windows)` only, with default features and no
optional features; they are absent from Unix and WASM build graphs. CI audits
the Windows graph explicitly.

Windows CI downloads upstream qpdf 12.4.2 and MuPDF 1.28.5 archives, pinned by
SHA-256 in `scripts/install-windows-test-tools.ps1`. They are independent test
programs (MuPDF runs under x64 emulation on Windows ARM64), never Cargo
dependencies or release contents. Their own upstream licenses remain distinct
from the MIT converter. Windows render tests remain enabled.

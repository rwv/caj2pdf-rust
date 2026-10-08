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

## GitHub corpus validation (#385)

The [reviewed post-fix report](https://github.com/rwv/caj2pdf-samples/blob/043e52cd37389b3f903426cd0b8c6d7564aedf43/research/notes/github-sweep-fixes-20261007.md),
[receipt](https://github.com/rwv/caj2pdf-samples/blob/043e52cd37389b3f903426cd0b8c6d7564aedf43/research/notes/github-sweep-fixes-20261007.json) and
[catalog](https://github.com/rwv/caj2pdf-samples/blob/043e52cd37389b3f903426cd0b8c6d7564aedf43/catalog.json) are pinned to samples merge
`043e52cd37389b3f903426cd0b8c6d7564aedf43`. The catalog SHA-256 is
`effede2cab4c1f04aac79d46517b10224ec7f65da0dda60464ed782c45bd2ed9`;
the catalog runner pins its identical content at `51417794ee80f60dc92335ece91e4160adf5ddb1`.
Source identities, sizes, download/archive locators and redistribution fields
are unchanged. The prior receipt is retained as historical evidence.

All 1,277 original candidates were actually attempted after the independently
authored fixes below: 1,227 convert, 39 fail and 11 remain unsupported. The
receipt separates qpdf warnings, strict-policy/limit refusals, proven damage,
encrypted containers, the valid unsupported xref predictor, and unresolved
ancillary order checks. It does not turn oracle recovery or blank substitution
into compatibility evidence. KDH oracle checks reuse the already measured
wrapper profile; no foreign converter implementation was consulted or copied.
No private module migration, new dependency, external document/derived content
or font bytes are introduced. [Conformance](conformance.md#github-corpus-post-fix-checkpoint-385)
records exact validation and untested scope; the report preserves every refusal
and the source-identical warning stream hash.

## Format references

| Format or feature | Reference | Status and permitted use |
| --- | --- | --- |
| PDF download footers (#386) | Independent bounded tail measurements of the [44 SHA-pinned inputs](https://github.com/rwv/caj2pdf-rust/issues/386) in the [GitHub sweep](https://github.com/rwv/caj2pdf-samples/blob/054e082e65e956ce3e90e464e5bb926b846b360d/research/notes/github-sample-sweep-20261007.md) | Original MIT recognizer and synthetic Rust/JavaScript controls. Admit exact `WebFastLoad` or UTF-8 BOM plus `FileProperty` with ordered plain-text `Doi`, `FileName`, `TableName`, `Type` leaves, optionally preceded by `WebFastLoad`. No general XML interpretation, new dependency, copied converter implementation or external document bytes. Existing bounded tail reads and PDF ambiguity guards remain. The other 16 observed inputs have PDF encryption dictionaries; 14 additionally contain HTML debris. Those refusals remain, with evidence recorded in the issue. |
| PDF output and PDF input | [ISO 32000-1:2008](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf) and the [PDF specification archive](https://pdfa.org/resource/pdf-specification-archive/) | Published format specifications. Record the exact PDF version and clauses used for each implementation change. Link to the documents; do not copy their text into source. |
| JBIG / JBIG2 bitstreams | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82) and [ITU-T T.88](https://www.itu.int/rec/T-REC-T.88/en) | Published coding recommendations. Implement the subset required by observed CAJ-family data as original MIT code. Do not reuse reference implementation source. |
| T.82 arithmetic SCD core and numeric states | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82), §6.2.5, §6.8.2.3/Table 24, §6.8.3, and §7.1/Table 26; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | Use the public algorithm to author original MIT Rust code. Keep Table 24's 113 exact numeric rows and the §7.1 vector outside the repository until their MIT redistribution basis is documented. The [core design](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t82-arithmetic-core.md) records the external-table contract and local test procedure; standard conformance does not establish CAJ compatibility. |
| T.88 MQ arithmetic control flow and numeric states | [ITU-T T.88 (02/2000), unamended base edition](https://www.itu.int/rec/T-REC-T.88-200002-S/en), also ISO/IEC 14492:2001, Annex E.2.5/Table E.1, E.2.9–E.2.10, E.3.1–E.3.6, and H.2/Table H.1; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | The normative decoder behavior, published 47-state numeric rows, original MIT caller-table control flow, and external-only Annex H/HN/C8 conformance inputs are separate materials. Exact Table E.1 rows and Annex H vector/checkpoints remain outside Git, artifacts, and releases. The [#44 rights record](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-mq-rights.md), reviewed 2026-09-27 by Codex repository research and an independent Codex reviewer, remains **UNRESOLVED**: neither an exact-state MIT redistribution grant nor an independently derived 47-state model is established. This is a technical provenance review, not legal clearance. The [core note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-mq-core.md) records bounded API and external-only checks. Annex H.2 verifies arithmetic decisions, not CAJ/JBIG2 pixels; observed HN/C8 modes and reachable states are corpus observations, not universal guarantees. |
| T.88 non-IAID arithmetic integers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.1–A.2 and E.3, with symbol-dictionary usage in §§6.5 and 7.4.2 | Original MIT 13-bank integer decision layer over the shared MQ decoder. The [integer note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-arithmetic-integer.md) records its signed/OOB result, 512-context layout, 38-decision limit, and synthetic checks. No Table E.1 states, external dictionary trace, or HN/C8 compatibility claim is included. |
| T.88 fixed-length IAID symbol IDs | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.3 and E.3, §§6.4.2, 6.4.10, 6.5.8.2.3, 7.4.2–7.4.3 | Original MIT IAID decision layer over the shared MQ stream, with its contexts at a fixed offset of the coding unit. The [IAID note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-iaid.md) records its fixed-width context map, bounded allocation and work, reset policy, symbol-array guard, and synthetic checks. No official state rows, external trace, or HN/C8 parity claim is included. |
| T.88 direct-coded arithmetic symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5, 6.5.1–6.5.10, 7.4.2.1–7.4.2.2, Tables 16 and 28, Annex A.2 and E.3.7–E.3.8; [repository-owned header inventory](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded direct path of the one symbol-dictionary decoder. The [dictionary note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-symbol-dictionary-direct.md) records classification, MQ/context ownership, store contract, limits, and optional evidence. The same decoder refines the observed second dictionary (below). Exact Table E.1 rows remain external under #44; metadata checks do not establish symbol pixel parity. |
| T.88 template-1 generic refinement bitmaps | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.3.2–6.3.5, Table 6, Figure 13, §6.5.8.2/Table 18 | Original MIT, bounded single-reference bitmap primitive. The [refinement note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-refinement-template1.md) records the ten-pixel context mapping, typed IAID/GR context ownership, reference store, row memory, error contract, and synthetic tests; since #355 the reference store is an in-memory bitmap. The bitmap primitive alone does not decode a `0x1802` dictionary; #66 integrates its one-reference path. No external symbol-pixel oracle exists: refinement compatibility is `NOT_RUN`, zero cases. Exact Table E.1 rows remain external under #44. |
| T.88 arithmetic single-reference symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.10–6.4.11, 6.5.5–6.5.10, 7.4.2.1–7.4.2.2, Tables 17–18, Annex A; [repository-owned header inventory](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded refinement path of the same symbol-dictionary decoder for the observed `0x1802` second dictionary when every IAAI is one. The [integration note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-refinement-dictionary.md) records imported/new stores, ordered export handles, MQ state, limits, typed zero/aggregate refusals, and synthetic tests. The private 546-case trace is diagnostic; independent symbol-pixel compatibility remains `NOT_RUN`, zero proven cases. Exact Table E.1 rows remain external under #44. |
| T.88 text-region data headers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.3.1–7.4.3.1.4, Figures 28–29 and 35–38; committed #43 oracle text flags | Original MIT, bounded header parser with no body reads. The [text-region note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-region-header.md) records validation order and optional metadata inventory. Strict parsing remains the default; the [#88 policy note](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t88-text-header-compatibility.md) documents the initial explicitly opted-in `0xa40c` HN/C8 exception and the preserved anomaly marker; [#389 extends only the standard displacement bits](#hnc8-empty-jbig2-content-389). Metadata alone establishes neither placement nor pixel compatibility. |
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
| HN-A full-page JPEG region records (#388) | [Pinned GitHub sweep identities](https://github.com/rwv/caj2pdf-samples/blob/054e082e65e956ce3e90e464e5bb926b846b360d/research/notes/github-sweep-20261007.json); independently authored geometric HN-A controls in offline CAJViewer | Original MIT parsing derived from all 1,610 pages in 12 inputs: 177 pages have one full-page JPEG placement followed by one or two region records, not additional image descriptors. Controls preserve identical viewer pixels when regions move or the measured placement flag changes between 0 and 1; moving the primary placement changes pixels. The bounded parser validates the prefix, glyph markers, complete placement/region records, sequential region identifiers, reserved words, page terminator and complete zlib frame before returning one coordinate. All 12 outputs pass qpdf; original JPEG bytes, full-page geometry and independently assembled reference-PDF pixels match on all 177 affected pages. Rust fixtures use invented words and original JPEGs; no foreign implementation, corpus content, font data, new dependency or allocation class is introduced. See [#388](https://github.com/rwv/caj2pdf-rust/issues/388) and the [checkpoint](conformance.md#hn-a-jpeg-region-checkpoint-388) for scope and runtime receipts. |
| Additional KDH/PDF profiles (#387, #393, #394) | [Pinned GitHub sweep identities](https://github.com/rwv/caj2pdf-samples/blob/054e082e65e956ce3e90e464e5bb926b846b360d/research/notes/github-sweep-20261007.json); [Adobe PDF Reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.3.pdf), Annex F.2.3; PDF 1.7 §§3.2.7, 8.2.2 and 8.5.3 | Independent measurements of 15 wrappers with field `01 00 00 00` at `0x28`, four forward xref chains and two local GoTo outline profiles. All use the existing signature, offset-254 XOR phase and complete PDF framing. Original MIT changes derive missing outline Prev and final-descendant Last repairs from the validated First/Next/Parent tree, preserve direct local GoTo destinations, and request stream-separator lookahead across read boundaries. Forward chains retain logical revision precedence and bounded cycle/extent checks; an ordinary incremental revision retires stale linearization hints. Invented Rust/JS fixtures cover positive cases, contradictory links, unsupported actions, cycles, limits and CRLF split at 512/1024 bytes. No converter implementation, private migration, external document bytes or new dependency. The 21-input run passed qpdf; MuPDF matched all 1,504 page identities/geometries, 1,003 outline entries/destinations and 9,648 raw stream payloads, plus 90 selected rendered pages (including every page of both 18-page patents). Unrendered pages have no pixel-parity claim. See [#387](https://github.com/rwv/caj2pdf-rust/issues/387), [#393](https://github.com/rwv/caj2pdf-rust/issues/393), [#394](https://github.com/rwv/caj2pdf-rust/issues/394) for revision/runtime receipts. |
| Additional C8 profiles (#380, #382) | [Original controls and measured rules](https://github.com/rwv/caj2pdf-samples/blob/4f262c5e2ff44bd2f52131fe449668cb131e36da/research/notes/c8-additional-profiles.md), with pinned source identities and a hash-only evidence manifest | Original MIT parser, placement and regression changes derived from bounded observations and black-box CAJViewer controls. Admit terminal encoded NULs, optional aligned image-name padding, measured axes/styles/symbols and decoration forms; keep unknown profiles explicit errors. Names remain source spans, with at most 28-byte parser reads. No private migration, converter/vendor implementation, corpus bytes, font outlines or derived document content is committed. No new dependencies or allocation class. Complete 10/5-page CLI/Node/browser outputs agree; selected layout/count evidence does not establish source-font or whole-document pixel parity. HN-B #381 remains separate. |
| HN-B magnesium article (#381) | [Original controls, measurements and source identity](https://github.com/rwv/caj2pdf-samples/blob/7dbdd623388521bea65111e3cbe4284f2afc8e37/research/notes/hnb-magnesium-profile.md); Adobe PDF Reference 1.7 §7.2.4/Table 7.2 and §10.8.3 | Original MIT size/symbol/metadata rules and type-3 admission, independently observed with authored viewer controls. First-image state selects opaque painting or bilevel-only Multiply. Raw A661 retains private-use U+E6C7; missing caller glyphs use an explicitly reported visual approximation with ActualText, without assigning semantic Unicode. No new dependencies, private-source migration, vendor implementation or external font/document bytes. Existing bounded decoding and sequential PDF output are retained; added state is constant per page/document and a flag per image handle. |
| CAA target descriptors and `.nh` extension (#424, #425) | [Pinned independent discovery and runtime receipt](https://github.com/rwv/caj2pdf-samples/blob/d9b42808e6a8d6ed7814d6970ac8253a590f513d/research/notes/caa-nh-discovery-20261008.md) | Original MIT recognizer based on 18 external ASCII descriptor identities: fixed field order, observed value shapes, complete LF/CRLF lines and NH/KDH labels. The fixture invents all numeric/opaque values; no external target or document bytes are copied. Reuses the 1 KiB detection buffer with chunked cancellable reads, no new allocation class or dependency. No target decoding/network resolution or vendor/converter implementation. A new `.nh` identity is HN-A and converts through existing code (433 pages, 365 bookmarks, matching CLI/Node/browser PDFs); CAS has no authentic sample and no registered signature. |
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
The HN-B type-3 regression uses the existing project-authored synthetic
fixture; #422 separates its portable assertions from external rendering.
Validator-less guests explicitly filter rendering as documented in
[platform validation](platforms.md#what-each-target-checks); filtered tests
are not compatibility passes. No fixture bytes, format facts, dependencies
or external source code were added.

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
| `flate2` 1.1.10 | Bounded zlib decoding of xref and object streams; `rust_backend`, `miniz_oxide`, `any_impl`. | `MIT OR Apache-2.0` / MIT (`LICENSE-MIT`) |
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

## HN/C8 empty JBIG2 content (#389)

The [GitHub sweep](https://github.com/rwv/caj2pdf-samples/blob/054e082e65e956ce3e90e464e5bb926b846b360d/research/notes/github-sample-sweep-20261007.md)
identified 25 C8 documents with an unused refinement-template flag. Bounded
segment inventories found 619 type-3 images, including 28 affected text headers:
`800c` (18), `840c` (4), `bc0c` (2), `880c`, `8c0c`, and `900c` (2).
Only the standard signed five-bit `SBDSOFFSET` differs from the previously
admitted `a40c` profile. T.88 §§7.4.3.1.1 and 6.4.11 still require strict
parsing to reject the unused template bit. The explicit HN/C8 policy preserves
raw flags and the anomaly, admits those displacement bits, and retains every
other profile/framing guard. Original MIT controls exercise all 32 offset
values with a nonzero symbol instance and compare canonical/policy decoding.

Ten documents then exposed empty direct and refinement dictionaries with
zero imported, new and exported symbols and only `ff ac` in their coded body.
T.88 §6.5.10 describes a zero IAEX run even for the empty dictionary. This
measured omission is accepted only by the internal HN/C8 composition entry
point, after ordinary header checks and only for an exact two-byte terminal
body with all three counts zero. The public strict dictionary decoder retains
its IAEX requirement. Original generated dictionaries test either/both empty
bodies with a nonblank generic region; wrong/truncated markers and nonzero
counts remain errors before an image is emitted. Three-byte source reads are
covered. No error is converted into a blank image.

All 28 source image payloads were wrapped externally for black-box decoding by
Poppler 25.03.0 (`libpoppler`, no `libjbig2dec`) and MuPDF 1.25.1
(`libmupdf`/`libjbig2dec`). Their full bilevel pixels agree. Native output from
original one-page C8 wrappers around those payloads agrees with both oracles
for all 28 images. The whole-document run is separately scoped in
[conformance](conformance.md#c8-jbig2-empty-content-checkpoint-389).
No other converter implementation was read, copied or transliterated. Document,
PDF, bitmap, font and oracle bytes remain outside Git. All committed controls
and changes are independently authored MIT source; dependencies, API shapes
and memory allocation bounds are unchanged.

## C8 uncompressed image records (#390)

The [GitHub sweep](https://github.com/rwv/caj2pdf-samples/blob/054e082e65e956ce3e90e464e5bb926b846b360d/research/notes/github-sweep-20261007.json)
reported 11 documents under an unsupported compressed-prefix diagnostic;
#389 exposed two more. Bounded page-index measurements found 19 affected
pages. Each has exactly one descriptor and starts with a 28-byte `800a/0000`
image record: four little-endian position/extent words at +4/+6/+8/+10,
followed by eight opaque words. Seven spans contain only that image record
and `8004`; the other twelve also contain the already supported tagged
records before `8004`. This is uncompressed direct framing, not a new zlib
prefix or permission to accept an empty/corrupt compressed stream. Descriptor
codecs are the existing JPEG (1), CAJ bilevel (0) and JBIG2 (3) profiles.

Independently authored geometric controls compared raw records with the same
records in a direct `COMPRESSTEXT` frame. Linux CAJViewer renders identical
page pixels for both. Changing the opaque image words from invented `10'`
to `79'` does not change those pixels; changing x/y/width/height moves and
scales the original colored shapes. The converter's raw/compressed PDF
regression is byte-identical, with a nonzero placement. Bounded streaming
record parsing is reused only for C8 with one descriptor and an initial
`800a/0000`. Native `800a/d300` stays on its existing native path. Other
initial values, counts, unknown subsequent tags and truncated records refuse.
No zlib validation rule changes. Chunk sizes 1, 2, 3, 11, 28 and 65,536 with
three-byte source reads are tested.

All 19 actual page renders and geometries match external one-page reference
containers that preserve source headers, image payloads and records, changing
only raw framing to the already supported direct compressed framing. All seven
JPEG payloads remain byte-identical. This confirms equivalence with existing
compressed rendering, not original-font fidelity. Real source documents,
reference containers, PDFs, screenshots and fonts stay outside Git; no foreign
converter implementation was read or migrated. All implementation and controls
are independently authored MIT code. See the [checkpoint](conformance.md#c8-uncompressed-image-record-checkpoint-390).

## C8 generic-only JBIG2 pages (#392)

The original #392 input and eight later #389 failures contain a two-segment
profile: page information 0/type 48 followed by generic region 1/type 38,
both associated with page 1 and neither referencing other segments. Page
flags are 1 (zero default, OR, eventually lossless), unstriped; the arithmetic
template-2 generic region covers the exact page at (0,0) with OR and the
already supported adaptive pixel (2,-1). The DIB and page dimensions agree.
There is no dictionary or text region to decode. This is a complete measured
profile, not permission to ignore arbitrary unknown segments.

The nine apparently blank source pages contain **19, 32, 46, 48, 55, 60, 63,
87 and 123 black pixels**. Poppler 25.03.0 and MuPDF 1.25.1
agree on every decoded pixel, and native output matches both. Each page is
fully decoded through the existing bounded generic-region decoder and exact
MQ terminal checks. The HN/C8 adapter validates the two-segment topology,
full-page geometry and page flags, then compares the decoder's actual header
with preflight before emitting rows. It never synthesizes a blank page or a
fake text/dictionary report. Existing five-segment decoding stays intact.
Generic-only output needs three generic rows and the existing bounded image
payload, with no new text bitmap or symbol dictionary allocation.

Original MIT controls remove only the empty dictionaries/text layer from a
nonblank synthetic five-segment image. Complete three-page PDFs remain
byte-identical, including adjacent pages and order, at widths 7/8/9/31/32/33
with short reads. Unknown segment types/numbers/associations/references,
extra segments, non-full-page geometry/operators, page flags, coding/adaptive
modes, damaged terminal bytes and pixel limits remain errors. All 27 actual
page-1/page-2/page-3 image bitmaps from the nine documents match independently
decoded Poppler source payloads, confirming both new and adjacent image
identity/order. These are bitmap comparisons, not universal document-layout
claims. All document/PDF/image/oracle bytes remain external; implementation
and fixtures are independently authored MIT, with no other converter source
read or migrated. See [conformance](conformance.md#c8-generic-only-jbig2-checkpoint-392).

## C8 four-page article records (#391)

The external SHA-256 `b9a64bf99e4bc496b976541a27128ee84109835d5f79f228a9c8ad159ec6829b`
([pinned locator and issue](https://github.com/rwv/caj2pdf-rust/issues/391))
contains six independently controlled native profiles. Clearing the initial
page-1 refusal exposed later title, punctuation, table and page-4 metadata
refusals; the final check uses the unchanged original, not a patched input.

Original mode-2 controls use origin `(4652,4274)`, a 700-by-450 page, invented
records, and original full-em geometric fonts with distinct resource markers.
These reuse the MIT `c8_additional_profiles_fixture.py` and
`c8_geometric_font.py` generators from caj2pdf-samples; geometric fonts replace
the viewer's bundled resource paths during the isolated control experiment.
System-font installation alone does not replace those bundled resources. No
vendor glyph outline or other converter implementation was read or copied.
The observations are:

- `8010/117` has the existing repeated-glyph geometry and endpoint clipping.
  With all aliases mapped to an original shape, its page pixels equal `/1`
  and `/46` at styles `1084`/`10a5` and lengths 200/600. Only those implicit
  style states are admitted for `/117`. The actual vendor ornaments differ:
  the existing default decorative alias remains an explicit substitute, not
  a claim of vendor-outline fidelity.
- CJK style `b94c` exactly matches explicit 84-by-109 axes in the viewer.
  Adjacent 84-square and 109-square controls distinguish the dimensions;
  ordinary Latin and punctuation classes remain unsupported for this style.
- At style `10a5`, `a1b6`/`a1b7` book-title marks match the opening-parenthesis
  control shifted by +4/-6 source units, respectively, in ordinary and
  alternate Latin resources. They therefore use total offsets `(30,-4)`
  and `(20,-4)` relative to the regular CJK origin. Other sizes stay refused.
- Style-`10a5` `a1fa` (right arrow) shares the symbol baseline and resets the
  Latin resource for itself and the following letter. Both initial resource
  states match the ordinary-resource comma control; subsequent explicit
  resource selection still takes effect. Explicit axes remain unsupported.
- Twelve-byte `8007/a380` and `/a382` records retain both endpoints and draw
  the same hairline as the existing `8006/a381` control, including shifted,
  reversed and diagonal pairs. Unknown neighboring values remain errors.
- `8072/d2e5` leaves glyph, line and decoration painting unchanged at both
  tested styles and resource states. Only that added metadata value is
  admitted; neighboring values remain refused.

The production change reuses bounded record reads, character decoding,
placement, clipping, font selection and sequential PDF painting. Original
Rust regressions cover short/truncated drawing records, atomic payloads,
complete PDF equivalence, subsequent text/resource state, title transforms
and refused neighbors. All new source is independently authored MIT; no
private-source migration, new dependency or document/font byte import occurs.
Control screenshots, source documents, PDFs and generated font files remain
external. The [conformance checkpoint](conformance.md#c8-native-article-checkpoint-391)
separates these observations from general document/ornament fidelity.


## PNG Up xref and compressed PDF metadata (#402, #404)

The independent implementation follows Adobe PDF Reference 1.7,
[sections 3.3 and 3.4.6–3.4.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf),
and the [PNG Up filter definition](https://www.w3.org/TR/PNG-Filters.html).
Predictor 12 is an encoding hint; decoding uses each row's algorithm byte.
The admitted byte-component, one-color profile requires tag 2 and a row
width equal to the sum of the xref field widths. Up adds the previous row
modulo 256, starting from a zero row independently for each stream.

The pinned [FuryMartin source](https://github.com/FuryMartin/caj2pdf-actions/blob/456a85d9a55690302e4dd10a95b6456afc1ceda6/file.caj)
is 79,463 bytes, SHA-256
`77b2ebe0a8d6cf9023b1427a0a5c4b3ff0f92b254bdee449c9a80cbc428e9fe3`.
Its unchanged KDH-decoded PDF is 79,208 bytes, SHA-256
`a5626ac265c7e543e2a59dfefc4ad05851c902299b344679ac346613ad5947c5`.
Xref objects 64 and 32 have 31 four-byte and 46 five-byte rows, respectively;
all 77 algorithm bytes are 2. Their type-2 entries reference 24 metadata
objects (33–45 and 65–75) in 13 direct-length Flate object streams. This
second dependency is tracked separately by #404. No source bytes are patched
to make the original pass.

Original MIT Rust fixtures independently encode positive, malformed,
truncated, overflow, limit, cancellation and revision controls. An integration
test asks the existing qpdf test tool to compress the project's original
`valid_nested_outline.pdf`, then checks one-byte source reads, unchanged
output bytes, outline validation and both page renders. This is generated
original test data, not a copied external document or converter algorithm.
The [conformance checkpoint](conformance.md#png-up-xref-and-object-stream-checkpoint-402-404)
records the external original's native/Node/Chromium and oracle comparisons.
All new code is independently authored MIT; there is no private-source
migration, foreign converter implementation, new dependency or committed
external document/font/render data. The existing selected MIT `flate2` grant
also covers the bounded object-stream inflation path.


## Identical page boxes in CAJ fragments (#407)

Four original CAJ inputs repeat the same direct MediaBox twice on a Page or
Pages dictionary. This is recoverable metadata duplication: it does not imply
missing page data. The pinned identities are:

| SHA-256 | Pages | Measured duplicate |
| --- | ---: | --- |
| `366f4d2f665253fc4398aafe70a69e75f4c8c2efc25c3de439b17e43267ef045` | 62 | Pages object 1, `[0 0 612 792]` |
| `6f30a4a0dc36d3c2dccdc876026098696f57678b394770e92c444e7e133c6e15` | 70 | Pages object 1, `[0 0 612 792]` |
| `9be1188adba1a3f496347f0850d841add927efa1fc4abfc188ebb5ab05048b0c` | 54 | Page object 3, `[0 0 595.28 841.89]` |
| `acca38898dc3346140723f69894d60d90a138bd7da9b3f62ab7671c92d334bd5` | 122 | Page object 3, `[0 0 595.28 841.89]` |

Source locators and unchanged input hashes are linked from
[#407](https://github.com/rwv/caj2pdf-rust/issues/407). The implementation
reuses the bounded dictionary-key index and page geometry parser; it records
only one relative metadata span and blanks the redundant pair during normal
sequential copying. It does not accept undefined conflicting duplicates,
change stream lengths, add whole-file buffering or reparse every source object.
The span occupies fixed metadata space charged through the existing indexes.

Original MIT tests cover Page and Pages, source/container offsets, one-byte
reads, two distinct rendered pages, malformed/conflicting/third duplicates,
other repeated keys, escaped names, allocation refusal and cancellation during
normalization. Independent real-document checks frame the original PDF object
body verbatim with a Catalog and any missing page-tree ancestors derived from
the existing Parent links and explicit CAJ page table. MuPDF then supplies the
source-body render oracle; qpdf independently reconstructs and validates its
own normalized copy. No converter-produced page content enters this oracle.
The [checkpoint](conformance.md#identical-fragment-page-box-checkpoint-407)
records the comparison scope and an observed qpdf-rewrite render difference.
All code and controls are independently authored MIT, with no private-source
migration, new dependency, foreign converter implementation or committed
external document/PDF/render/font bytes.


## Interrupted live PDF object prefixes (#410)

The unchanged KDH source `6cf520441256d3d0e8749cb4b49275bbfc57ff962839cd7e131055c99cca992e`
(`Extra2001/caj2pdf-actions/file.caj`) contains 12 interrupted object prefixes
between indexed objects. Independent offset-254 KDH decoding and qpdf's xref
inventory locate gaps of 8–68 bytes. Each trimmed prefix exactly matches the
complete generation-zero object selected by that xref; the counterparts are
page dictionaries and unsigned integers. The two dictionary gaps end inside
`/Type` and a nested font reference. An initial first-key-only candidate did
not complete this source and is not counted as a successful fix.

The implementation reuses the bounded PDF object parser and existing gap-patch
storage/copy-time byte checks. Only exact proper prefixes of complete non-stream
dictionaries/unsigned integers qualify for the new 128-byte rule. The original
64-byte free/adjacent orphan rule and shared retained-byte/allocation budgets
remain in place. No stream extent or payload search is introduced.

Original MIT controls cover indexed counterparts before/after the gap, partial
keys/references/integers, conflicts, missing/free targets, generation mismatch,
complete objects, excluded stream/other scalar profiles, one-byte reads,
128/129-byte bounds, exact PDF whitespace (including NUL, excluding vertical
TAB), cancellation checkpoints and source mutation during copy.
The [conformance checkpoint](conformance.md#interrupted-live-object-prefix-checkpoint-410)
compares the entire original through an independent decoded PDF oracle.
Existing CR stream-separator normalization and validated stale-parent repair
also apply to this document; these are separate, previously supported repairs.
No new dependency, foreign converter source, private-source migration, external
document, derived PDF, font or render bytes are included.


## Equivalent opacity resource references (#412)

Two original KDH files repeat a resource name inside a Page's direct
Resources/ExtGState dictionary. The different xref-selected references resolve
to the same direct CA/ca values:

| Source SHA-256 | Pages | Duplicate targets | Values |
| --- | ---: | --- | --- |
| `1673e115322421095a4463172335f98e0d71017307cf00ebb7fc258701044fa1` | 9 | 3 and 43, on 8 pages | both 0.08 / 0.08 |
| `ef0d77b2cdb2b9eeea105312bef7eef7727e7d8a055e7828d51fbd9a571cb17a` | 5 | 3 and 9, on 5 pages | 0.08 / 0.08 and 0.08000 / 0.08000 |

[Adobe's ExtGState documentation](https://opensource.adobe.com/dc-acrobat-sdk-docs/acrobatsdk/apireference/PDFEdit_Layer/PDEExtGState.html)
identifies CA/ca as the stroke/fill alpha values in the inclusive unit range.
The equivalence proof compares decimal digits exactly, with no float rounding.
It accepts only the measured opacity-only target dictionaries, bounded to 256
source bytes and 64 fractional decimal places; negative/exponent spellings,
other fields, indirect values and compressed/missing targets do not qualify.

The ordinary parser stays strict. An indexed-input retry collects at most one
pair of reference-valued duplicate names at the exact direct Resources/ExtGState
path. A non-stream Page and both live generation-zero targets must pass the
proof before the later pair is blanked in bounded metadata and reparsed strictly.
The copy appends a normalized Page revision through the existing repair path;
original source objects and streams remain present. Combined resource/stale-parent
repairs keep one final revision per object with the retained-byte budget adjusted.
An interrupted live prefix is compared against original source bytes even when
its complete counterpart has a proven resource repair.

Original MIT controls cover distinct visible opacity pages, exact decimal
neighbors, invalid references, wrong resource paths/types, extra duplicate
pairs, excluded target profiles, target bounds, cancellation, one-byte I/O and
combined repairs. All code is independently authored; there is no foreign
converter implementation, private-source migration or new dependency.
External documents, PDFs, fonts and pixels remain outside Git. The
[conformance checkpoint](conformance.md#equivalent-opacity-resource-checkpoint-412)
records independent decoding and source-content verification.

## Expanded GitHub source-image evidence (#406)

The [pinned samples receipt](https://github.com/rwv/caj2pdf-samples/blob/38a9bd62b32e198444e6596106e6e2db309834c4/research/notes/github-bitmap-oracles-20261008.json)
records source/PDF identities, bounded independent source inventory, every
page/image comparison, external tool identities and deliberate negative
controls for all 936 accepted originals missing image oracles. The original
MIT runner reuses the existing source extractor and black-box protocols.
The 10,077 type-0 and 3,150 type-3 descriptors are checked against external
decoder output; 2,481 JPEG descriptors retain exact encoded identity. Repeated
identical payloads reuse results within each document, with all descriptors
still checked in source page order.

The external type-0 oracle was rebuilt at the already documented revision
`8cbc3c5721acb762f739434eb3d206171dbb022a`, reproducing library SHA-256
`d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6`
and the pinned compiler identity. Its implementation was not read, copied,
transliterated or linked into the project. The library remains an external
behavioral oracle, never a product dependency or release artifact. Fresh
guarded workers with different prefills check each unique source payload.
The original type-3 wrapper copies the source JBIG2 stream for separately
installed Poppler/MuPDF tools; matching output is tool agreement, with decoder
implementation independence explicitly unverified.

Only original MIT tools/tests and metadata-only receipts are committed in
the samples repository. External documents, decoded pixels, PDFs, fonts,
foreign source and decoder binaries stay outside both repositories. The
[conformance checkpoint](conformance.md#expanded-source-bitmap-checkpoint-406)
retains five image-free pages as inapplicable to bitmap checks and links their
separate complete glyph proof. No new product dependency or format semantics
is inferred from these measurements.

## Absent optional CAJ link appearances (#417)

The unchanged [109-page CAJ source](https://github.com/Wilson-whu2010/caj2pdf-actions/blob/7f704d976511936b7cb9939b5ebddab38ff2ab6f/file.caj)
has SHA-256 `d3d8a89dc8ac9212a445935c55e29ac93224df1d157e891aebb2fb1f652906b6`.
Of 75 measured Link appearance references, two targets are absent: object 82
at source byte 1,935,124 points to 10887; object 518 at 2,316,599 points to 10886.
Their direct destinations are retained Pages 57 and 100. Both have the exact
zero-width border-style dictionary and a direct appearance dictionary containing
only the missing normal appearance. The
[Adobe PDF reference, section 3.2.8](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
defines nonexistent indirect targets as null, so the measured absent optional
appearance supplies no usable stream. This does not waive required references.

Original MIT candidate inspection reuses complete bounded object parsing,
source-byte rechecks and the existing repair budget/writer. The CAJ caller
proves the single target absent from the scanned object graph and the link
destination present in its explicit page inventory. Only the parsed AP pair
is removed; references aliased elsewhere in the object, other annotation or
border profiles, actions and unmeasured appearance states remain ineligible.
Original controls cover all cancellation checkpoints, one-byte reads, source
mutation, allocation bounds, strict neighboring profiles and earlier indirect
destination behavior. A live Link AP reference/stream is checked structurally;
MuPDF ignores that Link appearance in the synthetic control, so a neighboring
Square annotation supplies the visible live-appearance pixel control.

The independent source oracle copies bytes `[154216, 6552931)` verbatim and
adds only a Catalog pointing to the existing source Pages root 2. The CAJ
page table independently verifies the resulting page inventory. MuPDF reconstruction preserves all 109 page IDs. Qpdf's
reconstruction warnings are retained separately from its clean normalized output;
the original source-body PDF supplies the comparison baseline. No foreign
converter implementation, private migration, new product dependency or external
document/PDF/font/pixel bytes are included. The
[conformance checkpoint](conformance.md#absent-optional-link-appearance-checkpoint-417)
records whole-original comparisons and their renderer scope.

## Unescaped QITE source paths (#419)

The unchanged [139-page CAJ original](https://github.com/pbbbb12/caj2pdf-actions/blob/07567b3c2a864fa89161ef3753d823cd71f0aed1/file.caj)
(SHA-256 `b206e40da6df3fbe07ea6c7b40e900de5fe35b16f7e25029c3cfde38eb603d03`)
contains two malformed source-path strings: object 25588 at byte 451,327 and
object 25357 at 1,675,263. Each has an unescaped ASCII opening parenthesis
inside its single-line path, followed by the GBK bytes A3 A9 for a full-width
closing parenthesis. This leaves the PDF literal unclosed. The later reported
string-nesting error falls inside a correctly bounded JPEG; it does not prove
an image-length problem. The 138 incoming references are sole target occurrences
in retained Pages' direct QITE_pageid/F metadata (one and 137 respectively).
No source path is interpreted as a local file or accessed by the converter.

[Adobe's string documentation](https://opensource.adobe.com/dc-acrobat-sdk-docs/library/plugin/Plugins_Cos.html#literal-strings)
describes literal escaping and hexadecimal string syntax. Original MIT code
preserves each measured raw path byte by writing a hexadecimal string, only
after proving its exclusive metadata use. A 32-byte prefix probe precedes a
640-byte object bound; complete graph validation is capped at 64 path repairs
and retained bytes remain subject to Limits. Generation-zero, drive-prefixed
single-line .pdf paths with this exact unmatched-parenthesis profile qualify.
Backslashes, ASCII closing parentheses, other malformed strings and rendering
references do not. Correctly escaped strings and ordinary indexed PDF parsing
retain their existing behavior. Invalid candidates remain explicit damage
in partial mode, with dependent pages blanked; source changes and cancellation remain failures.

The independent original MIT oracle copies body `[218836, 13028326)` unchanged,
builds an explicit classic xref from 538 unique measured object headers and
adds only missing Pages root 2 (source children 25525/25524) and Catalog 25608.
The explicit xref avoids repair scans absorbing later objects into malformed
strings. All 139 source page-table identities and 258 raw streams are readable;
source warnings for the two path strings remain recorded. Converted page text,
boxes/rotation, all RGB renders at 72 dpi and link destinations match that
source framing in PyMuPDF 1.27.2.2. This establishes scoped page preservation,
not validity of the malformed original strings in every renderer. No foreign
converter implementation or private-source migration was used; no new dependency
or external document, PDF, pixel or font bytes are committed.

## Registry publishing action (#293)

The release workflow pins `rust-lang/crates-io-auth-action` v1 at
`c6f97d42243bad5fab37ca0427f495c86d5b1a18`. Its upstream MIT license is selected
from MIT OR Apache-2.0. It is CI-only authentication code, not linked into or
included in converter packages; no upstream source is copied. The action
exchanges GitHub OIDC identity for a temporary crates.io token and revokes it
when the job ends. npm uses its official CLI's OIDC and provenance support.
No new format facts, fixtures, product dependencies or private migrations.

## Malformed tiling-pattern matrices (#414)

Two unchanged originals, [163 pages](https://github.com/personqianduixue/Math_Model/blob/8783d0d822f89f98aa6182dd933cc2e9f3e2ddce/3-2%E7%AE%97%E6%B3%95-%E7%8E%B0%E4%BB%A3%E7%9A%84%E7%AE%97%E6%B3%95/%E7%B2%92%E5%AD%90%E7%BE%A4%E7%AE%97%E6%B3%95/%E7%B2%92%E5%AD%90%E7%BE%A4%E7%AE%97%E6%B3%95%E7%9A%84%E7%A0%94%E7%A9%B6%E5%8F%8A%E5%BA%94%E7%94%A8_%E5%88%98%E8%A1%8D%E6%B0%91.caj)
(SHA-256 `2423e0b8e64060bc55cf004da3c7f79b411739753e89e39ebefcea44b300b968`) and
[101 pages](https://github.com/angeladygaga/caj2pdf-actions/blob/436a7485a23665f73f6b70fbe080bb579cb7184c/file.caj)
(SHA-256 `f08947012a4882d3f62c9e3552f5a889d4bc7dee22f3a5119f22b7cb298b180f`), contain four Pattern objects
with Matrix `[0.72 0 0 -0.719999 -5e-006 842]`: objects 615/620/625 and 1307,
respectively. Their source resources reach page 20 and page 23. The
[Adobe PDF reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf)
requires six numeric Matrix elements and defines identity as the omitted
Matrix default. This does not prescribe recovery for an invalid exponent.

Original asymmetric-pattern controls distinguish decimal expansion, replacing
only the bad element with zero, and whole-Matrix identity. CAJViewer 9.0.0 and
Poppler use identity on the measured invalid-array controls; MuPDF instead
uses a zero element. The initial controls used an otherwise identity basis
and could not distinguish these behaviors. They are not sufficient evidence
for a zero-element repair. Small and amplified exponents, a nonidentity basis,
valid decimals, zero and identity were subsequently measured separately.

Nine fresh offline one-document viewer sessions compare both unchanged CAJ
originals, independently framed raw PDFs and explicit identity controls at the
two affected pages. Complete-page RGB crops at 50% agree, with two stable
captures each. Zero/decimal controls visibly change the first document's
pattern. Input hashes and cleanup pass; each isolated container remains below
748 MB and avoids OOM. Earlier multi-tab OOM/black captures, shifted sidebars
and an unsuccessful diagnostic CAJ view are retained as failed observations,
never passes. Vendor binaries, screenshots and documents remain external;
no vendor implementation was inspected, copied or migrated. Tool agreement
does not assert implementation independence or universal rendering fidelity.

Independent original MIT framing preserves source bodies `[46152, 4114695)`
and `[40908, 3705592)`, constructs explicit xrefs from 667/1,377 unique measured
headers and supplies absent Pages ancestors from original Parent/Kids and
source page-table order. Qpdf retains warnings for the four nonnumeric tokens.
All 264 Poppler RGB72 renders agree after explicit identity normalization;
all 1,366 raw stream payloads, source page IDs, text and effective geometry
remain intact. The unchanged originals produce the same PDFs as those measured
diagnostic controls. MuPDF's different handling of the invalid source Matrix
is expressly not counted as rendering agreement.

Original MIT recovery replaces only this measured array with explicit identity
and same-width whitespace padding. A 32-byte probe and a 512-byte header bound
precede strict parsing of the normalized dictionary. All eleven observed keys,
generation zero, the exact Matrix tokens, Pattern/Paint/TilingType 1, BBox
`[0 0 64 64]`, XStep/YStep 64, a generation-zero Resources reference, FlateDecode
and a direct 45-byte stream with a complete terminator are required. Duplicate,
extra, nested, oversized or different profiles are refused. Valid matrices and
ordinary indexed PDF parsing stay unchanged. Existing ranged patch output is
reused, with source rechecks, cancellation and retained-allocation limits;
stream content is never rewritten or buffered for this repair. No dependency
or external document, PDF, font or pixel data is added to the repository.

## Unresolved Indexed palette boundary (#420)

The [pinned investigation](https://github.com/rwv/caj2pdf-samples/blob/f1393308877014562bbb894e8b299f6917f1f004/research/notes/indexed-palette-boundary-20261008.md)
and its measurement receipt cover the unchanged 80-page CAJSamples `issue-39`
original, SHA-256
`5e1ea482a56a2df02a2a452ac97949824c726471c88157e78441a1201b3d8697`.
All 110 Indexed palettes and incoming references are inventoried; executed
page/Form/pattern paths reach 109. All 64 raw-short CMYK tables are used.
Palette 397 additionally decodes to 661 bytes instead of the required 663,
while malformed palette 471 terminates its first literal after 18 of 708
apparent raw bytes. These measurements follow the
[PDF reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf)
literal-string and Indexed lookup definitions.

Original MIT palette grids and 16 fresh isolated CAJViewer sessions distinguish
missing-image tolerance from color reconstruction. Raw-to-hex diagnostics
change source-viewer pixels on pages 25 and 27; short-table handling also
differs between Poppler and MuPDF. The 80-page diagnostic preserves 615 raw
streams, page identities/geometry and 72 source bookmarks, but retains
renderer warnings and fails complete raw-to-hex pixel/text agreement.
The unchanged original still fails strict conversion; explicit partial recovery
blanks pages 24, 25, 27 and 31. No diagnostic is counted as a new corpus pass.

This records an unresolved missing-color boundary, not a production repair or
proof that every reconstruction is impossible. Fix-specific full-regression,
Node and Chromium checks remain unrun. Only original observations and external
tool behavior were used; no foreign converter, private HN/JBIG or vendor
implementation was inspected or migrated. No document, PDF, palette, font,
pixel or vendor binary data is committed. APIs, dependencies, support claims
and release behavior remain unchanged.

## Redundant CAJ framing (#409, #434)

The [pinned report and per-input receipt](https://github.com/rwv/caj2pdf-samples/blob/26720c5d76fbb05b39400a65b35b3974b31c1931/research/notes/redundant-caj-framing-20261008.md)
record the measured source facts, original controls and final production
verification at `20292ab805a9d2cef066270af549635dce753dc5`.

Independent source-byte inspection of the unchanged 60-page CAJSamples
`issue-90/4-[6].caj`, upstream revision
`7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`, SHA-256
`dc3c3a651d4abaea61b8982e5165636f61b85f2c48f2eace3c6730e00dd963d3`,
establishes four separate framing conditions. No converter or vendor
implementation was used to derive them.

- Object 229 declares 98 bytes: 97 complete Flate bytes and LF. The subsequent
  CR and SPACE precede an exact stream/object tail. Retaining Length 98 avoids
  an unnecessary width-changing repair to 100. The bounded PDF whitespace
  tolerance changes no payload bytes and is not a general malformed-syntax
  conformance claim.
- A classic epilogue occupies `[2911107,2911776)`. After its CR, all 3,733
  encoded bytes at `[2911777,2915510)` equal source header `[144,3877)` under
  the measured FZHMEI XOR phase 2. CRLF precedes integer 4478 at 2915512,
  value 1592527, which equals image 4479's complete encoded extent. Old xref
  offsets and Root are stale; they do not define the reconstructed graph.
- Image 4474 at 1314164 ends after a 3,276-byte payload prefix. That prefix
  and its nine-field dictionary, except Length reference, match complete image
  4479. CRLF precedes complete Length integer 4468 at 1317618. This proves a
  shared prefix, not the absent image tail. Qpdf's complete selected-object
  graph has no incoming reference to 4474.
- Bytes `[2954967,2954978)` are only `4448 0 obj<`; CRLF then complete font
  4459 follows. There is no complete 4448 object or parsed incoming reference.
  The later interrupted 4459 and all other 29 metadata interruptions are
  exact prefixes of complete same-ID source objects.

Original MIT framing copies the entire source body `[47940,3491634)` verbatim
and builds a new xref from 395 complete object identities. Five complete
duplicate occurrences are exact. It adds only seven absent Pages ancestors
and a root/Catalog derived from original Parent links and CAJ page-table
order. Qpdf accepts the independently framed PDF without warnings. It retains
189 distinct raw streams, 60 page identities and 36 source bookmarks.

External same-width diagnostics first omitted each proved redundant range
to isolate scanner behavior. The resulting PDF and the subsequent unchanged
original's native conversion have identical SHA-256
`743afde6eb64ea8673b066a5d4320beb4f227b7199f37e8bfc92374f75e27d23`.
Against independent framing, all 395 object values, 189 raw streams, 60 page
IDs/boxes/rotations/text/links and 36 source bookmark titles/hierarchy/targets
agree. All 60 Poppler RGB72 page renders match exactly. Nine fresh offline
CAJViewer sessions compare original CAJ, independently framed PDF and the
diagnostic result on pages 1–3 at 50% zoom: three full-page crops match,
with two stable captures each, unchanged inputs, no OOM and complete cleanup.
This is three-page vendor evidence, not a whole-document vendor comparison
or proof of implementation independence between renderers.

Production rules are original MIT Rust with bounded ranged reads, sequential
output, complete-reference proof, independently anchored counterpart checks,
prefix/header rechecks and cancellation. Original synthetic tests include
opaque false headers/terminators, hidden/duplicate candidates, incoming and
non-reference lookalikes, metadata streams, malformed/overflowing/truncated
profiles, every cancellation checkpoint, short reads, limits and changing
sources. They contain no corpus bytes. Documents, derived PDFs, font/pixel
data and vendor binaries stay external. No new dependency, private migration,
foreign converter code or private HN/JBIG implementation is introduced.

The final 1,277-original / 2,126-attempt native run adds this one PASS and
preserves every previous successful PDF hash: 1,240 PASS, 28 FAIL and nine
UNSUPPORTED. Raw qpdf warnings, old ordering labels and missing checks remain
in the receipt; direct PDF hash comparisons retain the prior 936-source
image/text and 26-source ordering proofs within their original scope. The
unchanged original passes Node and Chromium with native-identical bytes and
empty OPFS after cleanup. A newer 433-page NH also remains byte-identical on
all three runtimes; 18 separate CAA descriptors retain intentional offline
refusals, and NH source-viewer pixels remain unexecuted. These are not counted
as additional frozen-corpus passes. Local checks pass 1,306 workspace and
166 JavaScript tests, with seven optional-corpus tests ignored; eight required
CI checks pass at the tested production commit. Review is self-review, not
independent approval, and the wider #406 correctness goal remains open.

## Damaged source streams in an accepted CAJ (#436)

The [pinned report and metadata receipt](https://github.com/rwv/caj2pdf-samples/blob/bf2e3d77b7aaf8a7d6cf722149938119ed0f1a30/research/notes/damaged-source-streams-20261008.md)
investigate the remaining qpdf-warning output in the full regression above.
The unchanged 63-page CAJSamples `issue-20/文件名未知.caj`, source SHA-256
`5a4432ed4878944c4aaa17f591b2a93d00014ea0bdacc9162bed1d88f8d61127`,
contains six damaged streams before conversion. Independent MIT framing
copies `[37868,983323)` verbatim, selects all 260 original objects and adds
only two missing Pages ancestors, their root and Catalog using source Parent
relationships and page-table order. The table's last nominal end, 983295,
cuts into the final stream and is not used as the complete body boundary.

Of 84 Flate streams, 78 pass strict zlib decoding. Streams 4, 9, 13, 269 and
142 reach raw DEFLATE EOF with incorrect Adler-32 checksums; stream 53 fails
with invalid code lengths before EOF, so its complete codec extent is not
established. Three JPEG streams have complete extents and decode without
warnings. Parsed resource paths register the damaged images on pages 2–4,
font 53 on pages 5–63, font 269 on page 5, and content 142 on page 39.
Resource registration is not proof of execution on every listed page.

Source content 142 starts at 880636. Its complete zlib frame is 4,586 bytes;
stored checksum `deae043d` differs from computed `4dd63b40`. The output's
4,587-byte stream equals that unchanged source frame plus LF. Independent
framing and output both retain the same 24 unexpected-parenthesis warnings
and qpdf exit 3. No data is normalized to hide this defect.
The accepted outcome follows the intentional codec-free framing change in
PR #369; it is not a regression from #435. Historical v0.4.0 codec rejection
does not describe the current opaque-payload framing policy.

All 260 original object values agree except repaired stream Length fields;
all 87 output payloads equal their independently located source bytes. All
63 page IDs/geometry/text/links and Poppler RGB72 images agree, with source
errors retained: Poppler exits zero while emitting 11,546 raw-framing and
6,908 output syntax-error lines. Matching damaged renderings and extracted
texts do not prove intact intended content. Fresh native, Node and Chromium
outputs have identical SHA-256
`58fc31644f1363fa695112bf2deb71e4069e990f53199e610938b31dc8052e6d`;
Chromium leaves no OPFS artifacts. The 93-bookmark source proof is inherited
from the identical pinned output, not re-executed.

Seven fresh offline CAJViewer sessions preserve input hashes, avoid OOM and
remove their containers. Raw/output page 39 crops match; a deliberately
emptied-content control differs by 20,839 pixels. Three original-CAJ attempts
cannot reach requested page 39 and are explicitly not source-viewer passes.
A separate unchanged CAJ navigation control reaches page 39. An upstream
2018 report likewise describes this attachment as damaged and mentions a
usable new download; no intact alternative was obtained or verified here.

This is evidence-only original MIT research: no foreign converter or vendor
implementation, private HN/JBIG code, document/font/pixel bytes, new dependency
or production behavior is introduced. Full corpus tests are not rerun for
this documentation change, and no new compatibility pass is counted. Review
is self-review. #436 stays open for a proven recovery policy and an intact
semantic oracle; these defects do not prove the whole source irrecoverable.

### Checksum-confirmed substitution candidate

A [subsequent diagnostic report](https://github.com/rwv/caj2pdf-samples/blob/8f1141f5e58099897bbf2a26b7e82624daec88fd/research/notes/stream-substitution-candidate-20261008.md)
advances that baseline. Sequence `ca a7 c2 e4` occurs 13 times exclusively
inside the six damaged streams. An exhaustive diagnostic tests all 65,536
two-byte replacements at stream 142's one occurrence, preserving its stored
checksum and all other bytes. Only `b5 f4` passes strict zlib EOF/checksum;
uniqueness is limited to this candidate family, not arbitrary byte edits.

Applying that same substitution at all 13 positions, without editing any
source metadata, restores all six original checksums and encoded Lengths,
all 63 original page-table offsets, the three image byte counts and both
font Length1 values. All 22 stored font table checksums also agree. The
983,497-byte diagnostic CAJ has SHA-256
`d5a23e59ad27807d8b8fbc9271dd9e4857d7c01c31a2682b014a13861b340a15`.
Its native PDF passes qpdf, and Poppler renders all 63 pages with no stderr.
All 81 unaffected payloads, page identities/geometry/links and 93 bookmarks
remain unchanged; the six damaged payloads explicitly change.

Six additional isolated CAJViewer sessions now reach diagnostic pages 3, 5
and 39; each modified-CAJ/output pair has identical full-page crops, stable
captures, unchanged inputs, no OOM and complete cleanup. These are modified
source observations, not an independently obtained intact alternative.
Production recovery remains unimplemented: neighboring refusals, bounded
resource/cancellation/changing-source controls, unchanged-original runtime
parity and full regression must precede any automatic transformation. The
historical corruption process and a general replacement policy are not
established. The candidate adds no unchanged-input compatibility pass.

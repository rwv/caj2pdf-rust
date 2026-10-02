# Provenance and dependency inventory

## Type-1 JPEG container profile (#224)

Source descriptors and six indexed payloads in the SHA-pinned issue-43 HN-A
input were inspected independently. Baseline JPEG structure and black-box
libjpeg-turbo decoding justify reuse of the existing original MIT JPEG path.
No converter source was consulted or copied. The JavaScript asymmetric JPEG
fixture reuses this project's original custom Huffman/DC fixture construction;
it contains no external image data. The existing #88 anomaly policy is selected
explicitly by HN/C8 platform adapters, without changing the generic decoder.
See [scope and measured results](hnc8-type1.md). External content remains private.

## Current-format baseline tools (#218)

`scripts/current_formats.py`, `scripts/current_format_order.py` and their
synthetic tests are original MIT implementations. They reuse this project's
source descriptors, bounded text/image helpers, normalized outline hashing and
pinned JBIG1/JBIG2 pixel hashes. No converter implementation was copied or
transliterated. CAJ page-table, CAJ/HN-A outline and KDH XOR observations are
already documented in the format notes. qpdf, MuPDF and Poppler run as external
black-box tools; their code is not bundled. The committed current CLI report
contains only inventory identifiers, metadata, errors and hashes. Source
files, converted PDFs, extracted pixels and outline text stay outside Git.
Page/image identity checks are narrower than rendered-page fidelity.

## 2026-09-29 standard numeric state adoption (#30/#44)

The owner explicitly instructed the project to use the state tables directly
and not send a rights inquiry. This supersedes the earlier project rule that
bundling must wait for an email or a separate original state derivation.
The earlier UNRESOLVED entries below are historical research, not the current
implementation gate. No rights-holder permission or legal determination is
claimed by this maintainer decision.

The adopted material is only the numerical interoperability parameters in
T.82 (03/1993) Table 24 (113 states) and T.88 (02/2000) Annex E Table E.1
(47 states). Values are taken from the project's already pinned, standard-derived
external records used for prior conformance checks. Rust declarations are
original MIT code. No reference decoder control flow, comments, arrangement,
standard prose, official test-vector bytes or external documents are imported.
The data is not described as independently invented or granted an MIT license
by another project's license. The project adopts the required numerical
parameters for implementing the formats and retains its MIT source license.

Observed upstream practice:

- [JBIG-KIT](https://www.cl.cam.ac.uk/~mgk25/jbigkit/) implements T.82 and
  distributes its implementation under GPL (with commercial licensing).
- [OpenJPEG](https://www.openjpeg.org/) embeds MQ state data for JPEG 2000;
  its [license](https://github.com/uclouvain/openjpeg/blob/master/LICENSE)
  is BSD-2-Clause.
- [PDF.js JBIG2](https://github.com/mozilla/pdf.js.jbig2/) distributes a
  PDFium-based decoder under Apache-2.0.

These are implementation precedents, not licenses for our source or evidence
of an ITU grant. No code was copied from these implementations. In particular,
GPL/BSD/Apache implementation code is not relabeled MIT.

`qm::STANDARD_STATES` and `jbig2::mq::STANDARD_STATES` expose the data through
existing state types. Fixed digest tests catch accidental numerical changes;
existing external conformance tests compare every row with the independently
pinned standard records. Official vectors remain external and optional tests
remain NOT_RUN when absent. CLI/WASM now default to these constants while retaining explicit overrides.
Original synthetic tests cover default and custom-table routes, incomplete
overrides, memory limits and scratch cleanup. No decoder logic changed.


## Type-3 shared image emission (#118)

The internal preflight/preparation/emission split reuses original MIT code
from `hnc8/convert_jbig2.rs`. The shared synthetic fixture builder was moved
from the existing original type-3 integration test; the new in-crate test
uses the same invented MQ states. No normative table, external document,
third-party implementation or new dependency is included.

## 2026-09-29 JavaScript delivery validation (#13)

The Node/browser example fixes, artifact tests, compile-only consumer checks,
and memory measurement script are original MIT source. They reuse the
repository's original synthetic PDF/CAJ fixtures and browser harness; no
external documents or converter code are added. CI uses TypeScript 5.9.3
(Apache-2.0) solely as an external type-checking tool, with MIT-licensed Node
22.18.6 and undici 6.21.0 declarations in the runner's temporary directory.
These tool packages are not vendored, imported at runtime or distributed with
the MIT npm package. Its runtime dependency list remains empty.


## 2026-09-29 decoded fixture comparison (#128)

`scripts/vendor_fixture_diff.py` and
`tests/conformance/test_vendor_fixture_diff.py` are independently authored
MIT code. They reuse this repository's original manifest validation and
original synthetic bundle generator. Pixel comparisons use declared decoded
payloads; no vendor image decoder, converter source, third-party implementation
or corpus content was copied or added. Only Python standard-library facilities
are used. The English usage document and CI smoke check are original as well.


## 2026-09-29 practical CAJViewer capture pilot

Original English report and reproduction instructions in
`cajviewer-capture-pilot.md`, with planning/conformance links. Commands reuse
existing original MIT controls and X11 client; no vendor implementation code
was inspected or copied. Only non-content external sample metadata and the
original control's expected text are published. All screenshots, vendor files,
external source documents and raw runtime logs remain external. No converter
or codec implementation changes are included.


## 2026-09-29 fixture planning simplification

Original English documentation only: PROJECT_PLAN.md, cajviewer-fixtures.md,
conformance.md and the historical capability-protocol notice. No converter,
codec, third-party source or vendor/corpus content is added. The unmerged #153
bootstrap draft is not included; that standalone task is cancelled. Existing
MIT requirements and historical evidence remain unchanged.

## Original capability protocol (#146)

The nine new modules `tools/cajviewer/capability_protocol.py`,
`capability_io.py`, `capability_x11.py`, `capability_clipboard.py`,
`capability_pages.py`, `capability_session.py`, `capability_collect.py`,
`capability_runtime.py` and `run_capabilities.py`, plus
`tests/conformance/test_cajviewer_capabilities.py`, are independently authored
original MIT source for this repository. Every file declares MIT. No private
module, Python/Go converter, vendor implementation, clipboard library or
HN/JBIG source was inspected, migrated, copied or transliterated for this work.

The implementation reuses existing original bounded process execution,
source-load validation and public PDF authoring facts. New public controls use
fabricated runtime JSON, X11/ICCCM byte transcripts, temporary files and the
existing original rectangle/Type3 fixture generator. They call production
validators, adapters, transport and state transitions rather than recreating
those implementations. Generated rasters, runtime inputs and application bytes
remain external. They are not fixtures from the vendor or optional corpus.
The sole reviewed original-control carrier passed 57 methods and 133 subtests
with zero failures, errors or skips, exit 0. All 112 ordered source audit rows
and both actual closing reviews passed. One carrier parent and one controlled
Python child ran; additional candidate forbidden effects were zero. Application
observations remain NOT_RUN and compatibility credit is zero. The
[closed control evidence](cajviewer-capability-protocol.md#code-acceptance-and-original-controls)
records artifact pins and the precise resource/measurement scope.

The wire client and selection state were written from the primary
[X.Org X11 core protocol](https://xorg.freedesktop.org/releases/X11R7.7/doc/xproto/x11protocol.html)
and [ICCCM selection conventions](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/icccm/icccm.html).
Only protocol facts (setup/reply/event layouts, properties, images, ownership,
TARGETS, TIMESTAMP and INCR) were used; no implementation snippets were imported.
TIMESTAMP is not assumed to be a persistent-manager content revision. These
sources establish no vendor headless/render/export flag or GUI binding.
Unavailable official documentation fetches and source-authoring/read refusals
remain in the preparation history; they are not application observations.
The early empty-argument guard also uses the official CPython 3.13
[argparse](https://raw.githubusercontent.com/python/cpython/v3.13.0/Lib/argparse.py)
and [gettext](https://raw.githubusercontent.com/python/cpython/v3.13.0/Lib/gettext.py)
fact that parser construction can consult ENV. The guard and refusing-ENV
controls are original code; no CPython implementation was copied.

[The capability design](cajviewer-capability-protocol.md) gives all eight #146
code gates, the producer-field consumer table, finite control/resource scope
and remaining runtime discoveries. Held-source and focused-control reviews
passed. Code acceptance requires exact committed-head reviews and the four
hosted gates. The three
consumed documentation files were snapshotted after the completed run and before
this publication; their historical byte identities remain external and unchanged.
Parent #124 still needs two actual independently closed
fresh sessions after a successful reviewed declared runtime view and a separate
finite frozen profile. Current v11 FAIL, the twelve previous launch outcomes and
unknown prior causes remain unchanged. No code completion or issue relationship
authorizes a launch. #126/#127 remain blocked; no converter/API/format support,
installed image or consumed plan is changed.
The sole v12 runtime-view phase also closed FAIL despite both original
public source loads passing. The fc-list helper exited 0 with 48 retained stderr
bytes, then failed validation with ValueError; exact message/cause UNKNOWN.
Both preservation reviews passed, operational status remained FAIL, inventory
did not complete and all 2,731 comparisons are NOT_RUN. No actual successful
runtime prerequisite or application proof exists. Independent native child and
blocker [#148](https://github.com/rwv/caj2pdf-rust/issues/148) tracks bounded
original inventory-helper diagnostics with resolved #125/#133/#143 prerequisites.
It does not authorize application work or alter #146's public-code scope.

## CAJViewer startup controls (#124)

The original MIT source in `scripts/cajviewer_canary{,_fixtures}.py`,
`tools/cajviewer/{Dockerfile,prepare.py,inventory.py,cajviewer_session.py,run.py}`
and the `test_cajviewer_*` test modules implements bounded external
preparation/startup and original runtime controls. PDF Type 3 glyph paths,
ToUnicode mappings, page objects, rotations and RGB controls are authored from
ISO 32000-1:2008 §§7.7.3, 8.9, 9.6.5 and 9.10.3. No vendor, legacy converter
or font implementation source is copied. X11 capture uses the public
`libX11.so.6` ABI in an external development environment.

The official Linux installer/manual/desktop metadata and public usage agreement
are provenance/usage references. All vendor runtime files, binaries, manuals,
fonts, raw UI captures and receipts remain external; no MIT redistribution
grant is inferred. The experimental Debian image omits installer hooks and
all bundled document entries. Public package dependencies are not relabeled
MIT. The [startup note](cajviewer-linux-startup.md) records exact preparation
pins, first metadata failure, scope and the unattempted capability requirements.
Child-only POSIX file limits and process-limit metadata use Python's public
`resource`/`subprocess` APIs and Linux `/proc`; original Python writer controls
test them without vendor execution or copied implementation.
The separately frozen external original XCB MIT-SHM control uses only the
[public XCB API](https://xcb.freedesktop.org/manual/group__XCB__Shm__API.html)
and [X11 MIT-SHM protocol](https://xorg.freedesktop.org/archive/X11R7.7/doc/xextproto/shm.html)
facts. It loads absolute pinned public Debian libraries, not vendor libraries,
and observes the display's shared-memory descriptor/failure under bounded
POSIX file limits. No external implementation is copied or inspected. Its
source/plan/closed receipt identities are in the startup note and its two
sessions report zero application launches and vendor passes. The current
session source applies the same finite child allowance to Xvfb and the app.
The fifth experimental image explicitly sets the documented
`QTWEBENGINE_DISABLE_SANDBOX=1` using the
[Qt 5.15 platform documentation](https://github.com/qt/qtwebengine/blob/v5.15.2/src/webengine/doc/src/qtwebengine-platform-notes.qdoc)
as a usage reference only. No Qt implementation is read or copied. The setting
does not change outer Docker isolation or prove vendor runtime support.
The owner-based visible-window observer uses the public
[xdotool 3.20160805.1 command manual](https://github.com/jordansissel/xdotool/blob/v3.20160805.1/xdotool.pod)
and Python's public `os.getpgid` API. Its original tests include two actual
project-owned Python process groups and owner/framing/deadline failure cases.
No xdotool, window-manager or vendor implementation source was used.
The preserved fifth-phase viewport payloads agree exactly and manual review
sees the original four-page PDF; they establish neither complete-page nor
text parity. All ten earlier reported startup FAIL outcomes remain intact.
The sixth frozen pair retains one additional helper failure and one owned
window observation, keeping document identity unverified and the pair FAIL.
The unretained search stderr leaves its cause unknown; no vendor or X11 error
class is inferred. All twelve reported attempts and the exhausted finite
ceiling are recorded in the startup note.
The public
[Dockerfile `COPY --chmod` reference](https://docs.docker.com/reference/dockerfile/#copy---chmod)
is a command usage reference for explicitly installing the two original
Python helpers as read-only mode 0444. This corrects a preserved app-zero
inventory failure caused by inheriting root-only host context permissions.

Issue [#133](https://github.com/rwv/caj2pdf-rust/issues/133) adds original MIT
terminal-helper diagnostics to the same two session/host adapters and
`tests/conformance/test_cajviewer_diagnostics.py`. It derives no new vendor
or X11 error classification. Fixed source locations, captured-byte hashes,
explicit incomplete prefixes and a terminal-only bounded base64 stderr
excerpt use Python's standard `hashlib`, `base64` and `json` APIs. The existing
256-KiB receipt limit is enforced before writing, with an explicit FAIL
refusal for an oversized complete ledger. The host preserves the primary
session failure when capture is absent and still requires cleanup/OOM and
artifact checks. Original Python process fixtures and simulated boundary
responses test this behavior without executing Docker, X11, viewer, converter
or private inputs. No external implementation, raw historical stderr or
vendor artifact was read, copied or embedded. All twelve earlier reported
application attempts, the mixed sixth FAIL and its unknown helper cause
remain preserved. The amended sources are not evidence of a new viewer
phase or complete-page/text compatibility; any such phase requires a
separately frozen protocol.
No Docker implementation or vendor file is copied into project source.

The [seventh-profile archival note](cajviewer-startup-diagnostics-v7.md)
records one independently reviewed public diagnostic-overlay preparation.
Its original MIT two-file context and finite external wrapper were authored
from repository-owned source and public Docker command references. Two
original synthetic-control runs retain the first overall FAIL (seven of
eight cases passed), followed by eight of eight passed after a fixture-only
correction. Neither is vendor compatibility evidence. The separately frozen
public phase then completed nine Docker clients with zero application,
inventory, X11 or vendor cases. Its exact consumed document/plan/pending/
token/receipt identities and unchanged image Config/six-layer lineage plus
one original COPY layer are recorded in the note. No proprietary implementation,
installer, corpus input, raw runtime/log/environment receipt, image or raster
is added to Git. The first failures and twelve historical launcher attempts
remain intact. This closed preparation supplies no new complete-page or text
fixture.

The separately frozen original inventory controls then passed eight groups
and 47 variants in one runner, using 20 synthetic helper callbacks and zero
actual child, Docker, environment/tool, runtime, application, vendor or
private-input actions. The first operational P2 phase closed FAIL after five
Docker clients and one incomplete inventory attempt. Its complete inline
envelope reports `FileNotFoundError` without a loading stage or missing path;
the specific cause remains UNKNOWN. Public source/tool/history, dynamic
closing and raw-output audits, protected caps/environment/user checks and
owned-container cleanup passed. They verify failure preservation and cleanup,
not a successful inventory or capability. All 2,731 comparisons remain NOT_RUN.
The archival note records the immutable metadata identities; original external
source, receipts, runtime and vendor artifacts remain outside Git.

Native child/blocker [#143](https://github.com/rwv/caj2pdf-rust/issues/143)
requires separately authored MIT accounting for exactly two pinned original
modules, distinguishing bounded read, pin, compile and execution attempts and
outcomes, plus original fault controls. No implementation is added by this
documentation update. Future controls and runtime diagnostics require their
own reviewed immutable finite plans; no source-load result authorizes startup
or image/text acquisition. #124's capability criteria and #126/#127 blockers
remain unchanged.

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
| T.88 MQ arithmetic control flow and numeric states | [ITU-T T.88 (02/2000), unamended base edition](https://www.itu.int/rec/T-REC-T.88-200002-S/en), also ISO/IEC 14492:2001, Annex E.2.5/Table E.1, E.2.9–E.2.10, E.3.1–E.3.6, and H.2/Table H.1; [ITU Software Copyright Guidelines](https://www.itu.int/dms_pub/itu-t/oth/04/04/T04040000040004PDFE.pdf) | The normative decoder behavior, published 47-state numeric rows, original MIT caller-table control flow, and external-only Annex H/HN/C8 conformance inputs are separate materials. Exact Table E.1 rows and Annex H vector/checkpoints remain outside Git, artifacts, and releases. The [#44 rights record](t88-mq-rights.md), reviewed 2026-09-27 by Codex repository research and an independent Codex reviewer, remains **UNRESOLVED**: neither an exact-state MIT redistribution grant nor an independently derived 47-state model is established. This is a technical provenance review, not legal clearance. The [core note](t88-mq-core.md) records bounded API and external-only checks. Annex H.2 verifies arithmetic decisions, not CAJ/JBIG2 pixels; observed HN/C8 modes and reachable states are corpus observations, not universal guarantees. |
| T.88 non-IAID arithmetic integers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.1–A.2 and E.3, with symbol-dictionary usage in §§6.5 and 7.4.2 | Original MIT typed 13-bank integer decision layer over the existing caller-table MQ decoder. The [integer note](t88-arithmetic-integer.md) records its signed/OOB result, 512-context layout, 38-decision limit, and synthetic checks. No Table E.1 states, external dictionary trace, or HN/C8 compatibility claim is included. |
| T.88 fixed-length IAID symbol IDs | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), Annex A.3 and E.3, §§6.4.2, 6.4.10, 6.5.8.2.3, 7.4.2–7.4.3 | Original MIT typed context owner and IAID decision layer over the existing caller-table MQ stream. The [IAID note](t88-iaid.md) records its fixed-width context map, bounded allocation and work, reset policy, symbol-array guard, and synthetic checks. No official state rows, external trace, or HN/C8 parity claim is included. |
| T.88 direct-coded arithmetic symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5, 6.5.1–6.5.10, 7.4.2.1–7.4.2.2, Tables 16 and 28, Annex A.2 and E.3.7–E.3.8; [repository-owned header inventory](../tests/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded caller-table first-dictionary primitive. The [dictionary note](t88-symbol-dictionary-direct.md) records classification, MQ/context ownership, store contract, limits, and optional evidence. The observed second refinement/aggregate dictionary remains typed unsupported. Exact Table E.1 rows remain external under #44; metadata checks do not establish symbol pixel parity. |
| T.88 template-1 generic refinement bitmaps | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.3.2–6.3.5, Table 6, Figure 13, §6.5.8.2/Table 18 | Original MIT, bounded caller-table single-reference bitmap primitive. The [refinement note](t88-refinement-template1.md) records the ten-pixel context mapping, typed IAID/GR context ownership, ranged reference store, row memory, poison/error contract, and synthetic tests. The bitmap primitive alone does not decode a `0x1802` dictionary; #66 integrates its one-reference path. No external symbol-pixel oracle exists: refinement compatibility is `NOT_RUN`, zero cases. Exact Table E.1 rows remain external under #44. |
| T.88 arithmetic single-reference symbol dictionaries | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.10–6.4.11, 6.5.5–6.5.10, 7.4.2.1–7.4.2.2, Tables 17–18, Annex A; [repository-owned header inventory](../tests/conformance/jbig2_dictionary_headers.json) | Original MIT, bounded caller-table decoder of the observed `0x1802` second dictionary when every IAAI is one. The [integration note](t88-refinement-dictionary.md) records imported/new stores, ordered export handles, MQ state, limits, typed zero/aggregate refusals, and synthetic tests. The private 546-case trace is diagnostic; independent symbol-pixel compatibility remains `NOT_RUN`, zero proven cases. Exact Table E.1 rows remain external under #44. |
| T.88 text-region data headers | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.3.1–7.4.3.1.4, Figures 28–29 and 35–38; committed #43 oracle text flags | Original MIT, bounded header parser with no body reads. The [text-region note](t88-text-region-header.md) records validation order and optional metadata inventory. Strict parsing remains the default; the [#88 policy note](t88-text-header-compatibility.md) documents one explicitly opted-in `0xa40c` HN/C8 exception and the preserved anomaly marker. Metadata alone establishes neither placement nor pixel compatibility. |
| T.88 arithmetic text instances | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.5–6.4.11, 7.4.3.1–7.4.3.2, Table 12, Annex A and E.3.7; [#85 hash-only text oracle](jbig2-text-oracle.md) | Original MIT, bounded caller-table pull decoder and optional SHA-pinned control-flow diagnostic. The [instance note](t88-text-instances.md) records context ownership, strip/RI decisions, store handles, limits, failure contract, 545 complete strict-region traces, and one strict anomaly refusal. Its event fingerprint is not independent pixel evidence; #87 supplies a separate text-only pixel comparison. #44 governs exact Table E.1 rights. |
| T.88 text-region composition | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.4.1–6.4.5 and 7.4.3.2, Tables 9–11; [#85 hash-only text oracle](jbig2-text-oracle.md) | Original MIT composition of checked #86 instances into caller-owned bounded random-access scratch, followed by sequential packed-row output. The [composer note](t88-text-composer.md) records clipping, combination, adapter ownership, limits, and optional private pixel comparison. No external decoder code, document bytes, decoded bitmap, or exact Table E.1 states are committed. |
| Observed HN/C8 type-3 JBIG2 page composition | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.8, and 8.2; [#43 full-image oracle](jbig2-oracle.md) | Original MIT parser/preflight and bounded OR row output for only the five-segment profile observed in 546 external records. The [page note](t88-observed-page-composition.md) records segment and region constraints, caller-owned text scratch, backpressure, budgets, and failure semantics. The later [#95 private comparison](jbig2-page-parity.md) matched the observed full-page pixels; clean-clone corpus parity remains `NOT_RUN`/zero. Exact MQ state rows remain external under #44. |
| Observed HN/C8 full-page JBIG2 pixel diagnostic | [#43 hash-only full-image oracle](jbig2-oracle.md), [#95 optional comparison](jbig2-page-parity.md), and [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§7.4.1, 7.4.8, and 8.2 | Original MIT, private-input-only comparison of packed full-page pixels for the observed five-segment subset. The private 27-file corpus and 47-state table remain outside Git and release artifacts. Reports may contain source, table, executable, and per-case pixel SHA-256 digests, failure hash comparisons, counts, and bounded resource measurements, never source bytes, table rows, or decoded pixels. The final-source #95 private run passed 545/545 strict full-page matches plus 1/1 separately labeled opt-in anomaly, with one strict anomaly-header refusal and unchanged source/table/executable hashes; a clean clone remains `NOT_RUN` with zero checked/matched pages. This establishes neither HN/C8 page placement, PDF output, universal JBIG2 support, nor independent oracle backends. #44 exact-state rights remain `UNRESOLVED`. |
| T.88 template-2 arithmetic generic regions | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), §§6.2.5.2–6.2.5.4, 6.2.5.7, 7.4.1, 7.4.6.1–7.4.6.4, Table 34, Figure 5, E.3.7 | Original MIT, bounded row decoder with caller-supplied MQ table. Two external generic-only HN/C8 pixel spots passed; all 546 remain for #50. The [region note](jbig2-generic-template2.md) records the context order, bounds, and external-only verification. |
| CAJ-family headers, pages, and outlines | [caj2pdf format notes](https://github.com/caj2pdf/caj2pdf/wiki), including [CAJ/HN identification](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E5%92%8C-HN), [basic information and outlines](https://github.com/caj2pdf/caj2pdf/wiki/%E6%96%87%E4%BB%B6%E5%9F%BA%E6%9C%AC%E4%BF%A1%E6%81%AF%E4%B8%8E%E5%A4%A7%E7%BA%B2), and [CAJ page content](https://github.com/caj2pdf/caj2pdf/wiki/CAJ-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9) | Public observations, not a complete normative specification. [Repository-owned CAJ measurements](caj-format.md) pin ten successful sample digests and document TOC, page-table, and PDF-fragment exceptions independently. Do not copy parser source or pseudocode. |
| HN/C8 source-page layout metadata | [#61 independent byte-layout observations](hnc8-container.md), [#107 black-box reference and PDF measurements](hnc8-layout-oracle.md), and the [27-source matrix](../tests/conformance/matrix.json) | Original MIT bounded source and PDF metadata extractors record checked spans, hashes, geometry and ordered draws. The external Python converter, PyPDF2, `libjbigdec.so`, qpdf, MuPDF, Poppler, source documents and generated PDFs are test-only black boxes, never migrated code or shipped assets. The deterministic 75-page/125-draw exploratory subset and the separate two-page HN-B case are measured; 50 additional-image x/y placements have no validated source-field rule. No multi-image compositor or general full-page parity is claimed. |
| HN/C8 container record reader | [Repository-owned #22 measurements](jbig1-oracle.md), [#61 read-only interval inventory](https://github.com/rwv/caj2pdf-rust/issues/61#issuecomment-5825547234), and the [bounded container note](hnc8-container.md) | Original MIT Rust reader of only the three measured variants. The optional hash-only comparison checks external record coordinates, not conversion or codec support. Cross-page alias policy, unknown page fields, and resource ceilings are documented in the note. No Python, Go, private Rust, wiki decompilation, or differently licensed parser source was used. |
| HN/C8 type-0 image wrapper and pixels | [ITU-T T.82](https://www.itu.int/rec/T-REC-T.82), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), and [repository-owned oracle measurements](jbig1-oracle.md) | The standards describe public coding and DIB fields. The local corpus measurements pin the CAJ-family wrapper and output hashes. The external differently licensed native decoder is a black-box oracle only, never implementation source or a project dependency. |
| HN/C8 type-0 row primitive | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82-199303-I/en) §§6.5, 6.7.1, 6.8.3; [independent #27 observations](jbig1-row-model.md) | [Issue #55's decoder](jbig1-type0-rows.md) is original MIT code using a caller-supplied table and bounded rows. The exact T.82 Table 24 values and official vector stay external pending #30. Its opt-in 1,400-image check uses only hashes and a private runtime fixture; it is not a released HN/C8 converter. |
| HN/C8 type-0 PDF pages | [ITU-T T.82 (03/1993)](https://www.itu.int/rec/T-REC-T.82-199303-I/en) §6.8 (interval convention for the test-only encoder), [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §4.8 (1 bpp image samples, `/Decode`), and the repository's [#22](jbig1-oracle.md)/[#27](jbig1-bitstream-investigation.md) palette and orientation observations | The [#28 core converter](hnc8-type0-pdf.md) is original MIT glue between the existing reader, row decoder, and PDF writer, with a caller-supplied table. Its polarity and top-down placement follow the repository's own measurements. The invented-table test encoder is original test code. No Table 24 rows, corpus bytes, pixels, or external decoder source are included. |
| HN/C8 selected type-0 PDF pixel diagnostic | [#22 hash-only image oracle](jbig1-oracle.md), [#100 optional PDF comparison](hnc8-type0-pdf-parity.md), and the original [#28 PDF writer path](hnc8-type0-pdf.md) | Original MIT selected-record API and synthetic tests reuse the caller-table decoder and PDF writer. The optional harness checks private source and image-span SHA-256 identities, reopens each temporary PDF with qpdf, extracts one-bit pixels with Poppler, and renders selected images with Poppler and MuPDF. The final-source private run matched 1,400/1,400 PDF-extracted visible/raw image hashes, verified all 27 source hashes before and after, and had zero failed/skipped/unsupported images; three fixed independent renders passed. The external corpus, exact T.82 Table 24 rows, official vector, output PDFs, and extracted pixels remain outside Git and releases. A clean clone reports `NOT_RUN` with zero corpus images. #30 still blocks bundling the table and standalone HN/C8 conversion. |
| HN/C8 type-2 JPEG marker profile | [CCITT/ISO T.81 Annex B](https://www.w3.org/Graphics/JPEG/itu-t81.pdf), [ITU T.81 catalog](https://www.itu.int/rec/T-REC-T.81), [ITU/ISO T.871 JFIF](https://www.itu.int/rec/T-REC-T.871-201105-I/en), and the original [#22/#61 container observations](hnc8-container.md) | Original MIT marker/profile reader over a checked type-2 HN/C8 descriptor and bounded ranged input. It derives only functional marker syntax from the standards, with no copied tables, figures, examples, tests, or decoder software. The [profile note](hnc8-type2-jpeg.md) records the observed JFIF 1.01 compatibility subset and separates marker classification from JPEG entropy decoding and PDF color/placement. The final-source private run matched 1,085/1,085 pinned type-2 descriptors and headers, with 27/27 unchanged source identities and zero failed/unsupported/skipped records; this is no pixel or PDF parity claim. Private CAJSamples sources and JPEG payload bytes stay external; clean-clone corpus compatibility is `NOT_RUN`/zero. |
| HN/C8 selected type-2 JPEG PDF diagnostic | [CCITT/ISO T.81 Annex B](https://www.w3.org/Graphics/JPEG/itu-t81.pdf), [ITU/ISO T.871 JFIF](https://www.itu.int/rec/T-REC-T.871-201105-I/en), [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §§3.3.7 and 4.8, and the original [#102 marker profile](hnc8-type2-jpeg.md) | Original MIT selected-record glue reuses the bounded HN/C8 reader and PDF writer. The three-component JFIF path explicitly selects PDF DCT `ColorTransform 1`; a bounded SHA-256 comparison binds the selected marker preflight to the streamed PDF image bytes. The [#104 final-source private run](hnc8-type2-pdf.md) matched 1,085/1,085 embedded JPEG streams, all 1,085 direct-JPEG versus `pdfimages` decoded and MuPDF page rasters pointwise, and 1,085/1,085 Poppler pages under a separately disclosed zero-slack 3×3 local-sampling rule; 27/27 source identities stayed unchanged and no image failed, skipped, or was unsupported. Poppler raw page pixels were not pointwise exact. These are one-selected-image diagnostic results, not HN/C8 page composition or standalone CLI/JS support. No external JPEG/PDF decoder source, corpus document or image bytes, or differently licensed converter implementation is included. |
| HN/C8 type-3 JBIG2 profile and pixels | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), and [repository-owned oracle measurements](jbig2-oracle.md) | Five SHA-pinned external documents contain 546 type-3 image records. Their original MIT metadata inventory and optional pixel-hash runner record tool agreement only. Poppler, MuPDF, qpdf, documents, PDFs, bitmaps, and decoder code are not runtime or shipped dependencies. |
| HN/C8 selected type-3 PDF diagnostic | [Original measured five-segment profile](t88-observed-page-composition.md), [#43 full-image oracle](jbig2-oracle.md), [#95 bounded packed-page decoder](jbig2-page-parity.md), and [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §4.8 | Original MIT selection and output glue joins repository-owned HN/C8, JBIG2 and PDF modules. The caller supplies the T.88 MQ table and bounded intermediate storage. [#106](hnc8-type3-pdf.md) checks one selected image per PDF page; it does not establish original page placement or complete source-document conversion. Exact normative table states, private documents, emitted PDFs, and external validator source remain outside Git and release artifacts. #44 rights remain unresolved. |
| HN/C8 type-38 generic-only JBIG2 pixels | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en) §§7.3, 7.4.1, 7.4.6, and 7.4.8 and [repository-owned generic-only measurements](jbig2-generic-oracle.md) | For the same 546 SHA-pinned type-3 records, original MIT tooling measures page-information segment #0 plus generic-region segment #4 alone. The hash-only manifest records black-box tool agreement, not Rust decoder parity or independent decoder implementations. No external source or generated bytes are distributed. |
| HN/C8 type-6 text-only JBIG2 pixels | [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en) §§6.4 and 7.4.3 and [repository-owned text-only measurements](jbig2-text-oracle.md) | Original MIT tooling measures segments #0–#3 without generic region #4 for the same 546 SHA-pinned records. The hash-only manifest separates 545 standard text headers from one `0xa40c` interoperability case. Rust matched all 545 strict-valid cases in #87 and the separately opted-in anomaly in #88; neither result verifies external decoder backend independence or full-page parity. No external source or generated bytes are distributed. |
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
[`gb18030.rs`](../crates/caj2pdf-core/src/gb18030.rs). The source corpus
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
| Poppler `pdfimages`, MuPDF `mutool`, and `qpdf` | Check temporary single-image JBIG2 PDFs and compare normalized PBM pixels for [issue #43](https://github.com/rwv/caj2pdf-rust/issues/43), generic-only [issue #51](https://github.com/rwv/caj2pdf-rust/issues/51), and text-only [issue #85](https://github.com/rwv/caj2pdf-rust/issues/85). | Invoke only as external black-box development tools. Record versions and binary digests in the [full-image manifest](../tests/conformance/jbig2_oracle.json), [generic-only manifest](../tests/conformance/jbig2_generic_oracle.json), and [text-only manifest](../tests/conformance/jbig2_text_oracle.json). Dynamic linkage differs, but independent decoder implementations are unverified; report tool agreement only. Do not read, copy, vendor, link, or ship their code or generated bytes. |
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

Issue #13 adds the original MIT WASM engine and exports in
`crates/caj2pdf-wasm/src/{lib,engine,bridge}.rs` and
`crates/caj2pdf-wasm/src/engine/tests.rs`, and the original MIT JavaScript
package in `js/{io,node,browser}.mjs`, its TypeScript declarations
`js/*.d.mts`, `js/package.json`, `js/examples/`, and `js/test/`. It moves
the CLI's leading-signature table into `caj2pdf_core::detect_format` so that
the CLI and the WASM engine share one detector; the table is unchanged and
uses only the signatures already recorded for the synthetic fixtures and in
the CAJ, KDH, and HN/C8 format notes. The tests build their
CAJ, KDH, and large PDF inputs at runtime from those recorded fields or
reuse the MIT fixtures in `tests/fixtures`; the OPFS tests use an original
in-memory test double. No code was taken from the Python or Go converters,
a private Rust module, or an npm package.

Issue #141 hardens those original stream adapters and adds the original MIT
internal `js/internal/spool-write.mjs` and controls in
`js/test/spool-boundary.test.mjs`. The controls use invented small bytes,
actual Web Streams and deterministic deferred write/cancellation promises.
Reader ownership follows the [WHATWG Streams API](https://streams.spec.whatwg.org/#default-reader-release-lock);
ordered write/progress handling follows the
[Node FileHandle API](https://nodejs.org/api/fs.html#filehandlewritebuffer-offset-length-position).
No implementation or dependency was copied. Package allowlist/import checks
include the helper without adding a public entry export. This public JS slice
leaves parent #13's HN/C8 integration and codec/provenance prerequisites unmet;
fault controls establish no vendor compatibility or new WASM heap measurement.

The issue #13 real-browser tests add original MIT
`js/test/{browser.test,browser-harness,browser-cases,package.test}.mjs` and
`js/scripts/copy-wasm.mjs`. The harness is a minimal Chrome DevTools
Protocol client written for this project over Node's built-in `node:http`,
`node:child_process`, and global `WebSocket`; no Playwright, Puppeteer, or
other npm package, and no code from one, is used. Chromium or Google Chrome
is an external test-only executable found on the machine (the CI runner's
preinstalled Google Chrome); it is not downloaded, vendored, linked, or
distributed. The browser inputs are the same runtime-built synthetic CAJ and
KDH inputs and MIT `tests/fixtures` files as the Node tests. The npm tarball
carries the WASM build of the MIT workspace; the copy in `js/` is gitignored.

The issue #13 optional JavaScript corpus runner adds original MIT
`js/scripts/corpus.mjs` and `js/test/corpus.test.mjs`. The runner uses only
Node built-ins and this package's public API, reads the committed
`tests/conformance/matrix.json` (identities, recorded reference outcomes,
and page counts only), and runs the external `qpdf` executable, when
installed, as a black-box validator. Its tests build a temporary synthetic
corpus and matrix from the runtime-built CAJ and KDH inputs and the MIT
`tests/fixtures` files, plus a stand-in `qpdf` script written at test time
to exercise warnings, timeouts, and interruption; no CAJSamples
document, derived PDF, or code from another converter is used or committed.

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

Issue #61 adds original MIT
[`hnc8.rs`](../crates/caj2pdf-core/src/hnc8.rs), its native metadata-only
example, [synthetic tests](../crates/caj2pdf-core/tests/hnc8_container.rs),
and the opt-in
[hash-only comparison](../tests/conformance/hnc8_container_compare.py).
The [container note](hnc8-container.md) records each adopted layout fact,
unknown field, alias policy, and resource bound. The 27 external document
sizes/hashes and 1,400 type-0 coordinates are inherited from the independent
#22 manifest; #61's separate 27-file interval inventory informed the
conservative protected-region rule. No CAJSamples document, encoded payload,
PDF, private decoder table, or external parser source is committed. No
private-source module is proposed for migration.

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

Issue #28's first core slice adds original MIT
[`hnc8/convert.rs`](../crates/caj2pdf-core/src/hnc8/convert.rs), the additive
`jbig1::read_type0_info` preflight, the streamed 1 bpp image and
multi-placement page methods in
[`pdf/document.rs`](../crates/caj2pdf-core/src/pdf/document.rs), and the
[synthetic integration tests](../crates/caj2pdf-core/tests/hnc8_type0_pdf.rs).
The [type-0 PDF note](hnc8-type0-pdf.md) records the adopted palette,
padding, orientation, geometry, and multi-image rules; each comes from the
PDF 1.7 reference or the repository's own #22/#27 observations. The tests'
arithmetic encoder was written for this repository from the T.82 interval
description and the published #27 row rule, and uses an invented adaptive
113-state table; it is not derived from JBIG-KIT, the Python or Go
converters, a private Rust module, or any other encoder. Test containers,
images, and PDFs are generated at run time. qpdf, Poppler, and MuPDF run
only as independent test-time validators. No private-source module is
proposed for migration, and Table 24 remains external under #30.

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
The T.88 Annex E numeric-state redistribution basis remains unresolved under
the [#44 rights record](t88-mq-rights.md); exact states or an integrated
decoder containing them cannot be bundled under MIT on this evidence.

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
added no runtime dependency at the time; the external harness uses the already
registered MIT-selected `sha2` dependency. Standard-vector agreement does
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
The follow-up reset rule comes from T.88 §7.4.2.2 steps 3–5 and 7: arithmetic
integer statistics are zeroed for each symbol dictionary while
generic/refinement bitmap statistics can be restored or retained. The #54
method resets its thirteen non-IAID banks and preserves appended model
contexts; its full reset remains available. The #60 typed owner adds a
dictionary reset that clears both those banks and IAID while preserving
appended bitmap contexts.

Issue #60 adds the original MIT Annex A.3 IAID procedure in
[`jbig2/iaid.rs`](../crates/caj2pdf-core/src/jbig2/iaid.rs), its original
[unit tests](../crates/caj2pdf-core/src/jbig2/iaid/tests.rs), and its
[public API tests](../crates/caj2pdf-core/tests/jbig2_iaid.rs). The
[IAID note](t88-iaid.md) records the exact official clauses, the fixed-width
context map, scoped reset behavior, symbol-array guard, and memory formula.
The official English T.88 PDF with SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`
and this repository's MIT MQ/integer modules were the only implementation
references. Its example decision bits and contexts were transcribed as facts
for an original test; no official byte vectors, Table E.1 rows, external
CAJSamples documents or pixels, Python/Go/private Rust source, or other
decoder source was copied or migrated. The 47-row table and encoded bytes in
tests are newly invented MIT fixtures. No independent IAID decision trace is
currently available, so external IAID compatibility is `NOT_RUN` with zero
checked cases. The exact Table E.1 rights question remains in #44. This
primitive does not decode symbol bitmaps, text regions, or pages.

Issue #62 adds the original MIT direct symbol-dictionary decoder in
[`jbig2/dictionary.rs`](../crates/caj2pdf-core/src/jbig2/dictionary.rs),
reuses this repository's MIT MQ, integer, segment-header, and template-2
pixel-context code, and records the design in the
[dictionary note](t88-symbol-dictionary-direct.md). Original tiny synthetic
tests use an invented 47-state machine and independently chosen decisions.
The original MIT
[native metrics probe](../crates/caj2pdf-core/examples/jbig2_dictionary_metrics.rs)
reads one SHA-pinned external first dictionary and a private caller table at
runtime, measures resident memory and temporary storage, and removes its
temporary file; it ships neither input nor official states.
The [metadata-only optional runner](../scripts/jbig2_dictionary_headers.py)
and [numeric/hash manifest](../tests/conformance/jbig2_dictionary_headers.json)
were independently measured across 546 first/second dictionary headers in
five type-3 source files. The runner also checks all 27 selected external
HN/C8 source identities before and after reading. The manifest and runner
contain no encoded segment or decoded pixel bytes. The official T.88
(02/2000) clauses cited
above and these repository-owned measurements were the only format sources.
No Python/Go/private Rust converter, third-party decoder source, exact
Table E.1 row, Annex H byte, external CAJ document, or generated output was
copied or migrated. The production core has no new dependency. The #2
refinement/aggregate dictionary is parsed only and returns typed unsupported;
text regions, page composition, and independent symbol-pixel parity remain
open. External symbol compatibility is `NOT_RUN` with zero checked cases;
[#44](https://github.com/rwv/caj2pdf-rust/issues/44) still governs exact
official MQ-state redistribution.

Issue #65 adds the original MIT template-1 refinement bitmap primitive in
[`jbig2/refinement.rs`](../crates/caj2pdf-core/src/jbig2/refinement.rs),
plus a small internal MQ poison/context-capacity hook. Its only algorithm
source was the official T.88 (02/2000) English PDF and its Figure 13 pixel
geometry, independently checked against the PDF image. The synthetic tests
were newly authored for this repository. No Python, Go, private Rust,
third-party decoder source, external symbol pixel, exact Table E.1 row, or
Annex H byte was copied or migrated. The optional check has no independent
refinement-pixel oracle, so clean-clone status is `NOT_RUN` with zero checked
cases; an external diagnostic cannot upgrade that status. There is no new
runtime or development dependency. The `0x1802` dictionary integration is
recorded below for #66; text/page work remains open under #9.

Issue #66 adds the original MIT second-dictionary integration in
[`jbig2/refinement_dictionary.rs`](../crates/caj2pdf-core/src/jbig2/refinement_dictionary.rs)
and an internal final-progress observer in `jbig2/refinement.rs`. Its
algorithm source was the official T.88 (02/2000) English PDF cited above;
the external five-file inventory supplies only independent header/count
measurements. Original synthetic unit tests use an invented 47-state MQ
table and independently specified packed reference pixels. The optional
diagnostic runner checks the SHA-pinned corpus and private caller-supplied
table without copying any external document or exact Table E.1 row into
Git. It reaches 546 complete second dictionaries and 8,642 complete
`IAAI=1` values, with zero `IAAI=0` or `IAAI>1` on that pinned corpus.
No independent per-symbol pixel oracle exists, so external symbol-pixel
compatibility remains `NOT_RUN` with zero proven cases. No Python, Go,
private Rust, third-party decoder source, corpus payload, generated bitmap,
exact official MQ row, or Annex H byte was copied or migrated. There is no
new Cargo dependency. Table 17 aggregation, text-region/page composition, and
independent integrated parity remain open under #9; exact table-rights
review remains open under #44.

Issue #69 adds the original MIT text-region header parser in
[`jbig2/text.rs`](../crates/caj2pdf-core/src/jbig2/text.rs) and the
metadata-only [`jbig2_text_region_headers.py`](../scripts/jbig2_text_region_headers.py).
The only format source was the official T.88 (02/2000) English PDF cited
above (SHA-256 `a94850aa…ec69`), plus repository-owned measurements already
pinned in the #43 oracle. The synthetic Rust and Python tests were newly
authored for this repository. No Python, Go, private Rust, third-party
decoder source, external CAJ byte, Annex H vector, or Table E.1 row was
copied or migrated, and no dependency was added. The inventory stores only
counts, flags, offsets, and lengths; text compatibility is `NOT_RUN` with
zero cases.

Issue #85 adds the original MIT optional
[`jbig2_text_oracle.py`](../scripts/jbig2_text_oracle.py), synthetic tests,
[scope note](jbig2-text-oracle.md), and a
[hash-only manifest](../tests/conformance/jbig2_text_oracle.json). It reuses
repository-owned #42, #43, and #69 readers and the existing PDF/PBM helpers;
no decoder implementation or new dependency was copied or added. The only
new external observations are black-box hashes, counts, and tool identities
for temporary #0–#3 renderings of the SHA-pinned corpus. The 2026-09-27 UTC
run found 546/546 Poppler/MuPDF agreements (545 standard headers, one
separately labeled `0xa40c` interoperability case), 27 source hashes before
and after, zero failures, and zero skipped cases. Backend independence
remains unverified; the later #87 Rust text-only comparison matched all 545
standards-valid cases. No CAJSamples
document, encoded segment, PDF, PBM, Python/Go/private Rust source, or exact
Table E.1 state was committed.

Issue #86 adds the original MIT
[`jbig2/text_instances.rs`](../crates/caj2pdf-core/src/jbig2/text_instances.rs),
its clean-clone tests, the caller-store continuation hook in the existing
refinement primitive, and the optional
[`jbig2_text_instance_diagnostic.py`](../scripts/jbig2_text_instance_diagnostic.py)
with a native example and Python evidence tests. The algorithm was authored
from the official T.88 clauses listed in the format table and the existing
repository-owned typed primitives. The #66 example was reused only because
it is MIT source authored in this repository. No Python/Go/private Rust or
external decoder code was copied, and no new dependency was added. The
optional private run completed 545 strict-valid region control traces with
353,829 instances; one malformed raw `0xa40c` header was refused. All 27
source SHA-256 hashes and the private table hash matched before and after.
Only event fingerprints and counts enter the diagnostic report; the
external documents, exact Table E.1 rows, and decoded bitmaps stay outside
Git. Its placement fingerprint is not an independent placement oracle; the
later #87 text-only pixel comparison is recorded below.

Issue #87 adds the original MIT
[`jbig2/text_composer.rs`](../crates/caj2pdf-core/src/jbig2/text_composer.rs),
clean-clone tests, and the optional
[`jbig2_text_region_parity.py`](../scripts/jbig2_text_region_parity.py)
with a native example. It follows the official T.88 clauses in the format
table and consumes the checked #86 stream; it does not copy or translate an
external compositor. Bitmap views carry content revisions to detect same-size
changes, and the caller owns the scratch store exclusively. The local private
run matched both hash and black-pixel
count for all 545 standards-valid text-only regions; it separately refused
the one malformed header. All 27 source hashes and the private table hash
matched before and after. The external documents, exact Table E.1 states,
and decoded bitmaps remain outside Git. Generic-region combination,
full-page rendering, and PDF integration remain open under #9.

Issue #88 adds the original MIT, explicitly selected HN/C8 text-header
policy described in [its scope note](t88-text-header-compatibility.md). The
only exception is the unused `SBRTEMPLATE` bit in the pinned raw `0xa40c`
header when `SBREFINE=0`; the ordinary parser and decoder remain strict.
The source coordinate, SHA-256, two flag bytes, located #69 refusal, and
independent #85 text-only pixel hash are recorded as metadata only. The
policy retains the raw flags and a typed anomaly marker through the
source-rechecked instance decoder and composer report. The optional
[`jbig2_text_region_parity.py`](../scripts/jbig2_text_region_parity.py)
diagnostic compares its one opt-in text-only output separately from the
545 standards-valid outputs using the privately held exact MQ table. No
external document, decoded pixels, decoder source, or exact Table E.1 rows
are added, and no new dependency is introduced. Page composition, PDF
integration, and exact-state redistribution rights remain separate work.

The 2026-09-27 UTC private run matched all 545 standards-valid #85 text-only
pixel hashes and black-pixel counts plus the one separately counted `0xa40c`
hash/count through the actual Rust arithmetic and composition path. It
verified 27 source hashes and one private table hash before and after,
with zero case failures or skips. The anomaly case completed 234 instances
and 3,431 rows; its peak RSS was 2,727,936 bytes and its largest I/O
request was 296 bytes. The all-case resource maxima were 1,098,864 scratch
bytes, 312 request bytes, and 2,764,800 bytes peak RSS on this machine.

Issue #12 replaces the placeholder `crates/caj2pdf-cli/src/main.rs` and
removes `crates/caj2pdf-cli/tests/unimplemented.rs`. It adds the original MIT
CLI sources `args.rs`, `document.rs`, `files.rs`, `json.rs`, `report.rs`, and
the unit tests in `tests.rs` under `crates/caj2pdf-cli/src`, plus the
process tests in `crates/caj2pdf-cli/tests/cli.rs`. They were written for
this repository against the core's public API; no Python, Go, private Rust,
or third-party CLI source was copied, transliterated, or migrated. The only
format facts they add are the leading-signature table, taken from the
signatures already recorded in the [fixture note](../tests/fixtures/README.md),
the [HN/C8 container note](hnc8-container.md), and the
[KDH note](kdh-format.md). The JSON encoder follows the published
[RFC 8259](https://www.rfc-editor.org/rfc/rfc8259) string grammar. The
process tests build tiny CAJ, KDH, C8, and HN containers at run time from
those notes and use the existing MIT PDF fixtures; no binary fixture is
added. Installed `qpdf` and MuPDF `mutool` validate generated PDFs as
independent black-box tools, as for issue #5.

Issue #106 adds original MIT selected type-3 conversion glue in
`crates/caj2pdf-core/src/hnc8/convert_jbig2.rs`, original synthetic tests in
`crates/caj2pdf-core/tests/hnc8_type3_pdf.rs`, and an optional private
diagnostic in `crates/caj2pdf-core/examples/jbig2_page_pdf.rs` and
`scripts/jbig2_page_pdf_parity.py`. The core composes existing MIT HN/C8,
JBIG2, and PDF APIs and receives exact MQ states and temporary backing from
the caller. No decoder or parser implementation was migrated from the Python,
Go, private Rust, or external PDF/JBIG2 tools. The synthetic test constructs
new byte-level fixtures from the format facts already recorded in this
repository, uses an invented MQ table, and does not include private corpus
bytes. The opt-in runner verifies external source/table hashes and leaves
documents, table states, generated PDFs, and rendered pixels outside Git and
releases. Its Poppler, MuPDF, and qpdf commands are test-only black-box
validators, not production dependencies. #44 still blocks bundling exact
normative MQ state rows.

Issue #107 adds original MIT, metadata-only measurement code in
`scripts/hnc8_layout_source.py`, `scripts/hnc8_layout_pdf.py`,
`scripts/hnc8_layout_reference.py` and `scripts/hnc8_layout_parity.py`, with
synthetic conformance tests and a compact hash/metadata oracle. The source
extractor uses only the repository's independent #22/#61 byte observations;
it does not copy or translate another converter's parser. The PDF extractor
independently cross-checks qpdf content-stream draws against MuPDF trace and
Poppler image-list outputs, and hashes raw image streams without retaining
them. Reference generation invokes a pinned external Python checkout and
native decoder solely as black-box tools. Their code, package files, source
documents, PDFs, rendered pixels and raw text stay outside Git and release
artifacts. The [layout note](hnc8-layout-oracle.md) records exact hashes,
versions, scope, resource measurements, controlled perturbations and the
unresolved image-placement rule. #30 and #44 rights questions remain open.

Issue #110's [placement plan](hnc8-placement-experiments.md) records
read-only structural checks on the same two SHA-pinned HN-A/C8 sources and
the public #107 CTMs. The observed JPEG APP0 fields are identical across
50 additional images. Four predeclared one-byte JFIF metadata probes, each
run twice, changed only the selected encoded JPEG stream in the reference
PDF; none changed a CTM. The original MIT diagnostic checks all 27 source
hashes, six baseline PDF hashes, executable/package hashes, command and
environment before and after. Its source copies and PDFs stay outside Git.
The committed-oracle-only geometry controls and limited text-span encoding
scans also found no placement rule. A separately predeclared C8/HN-A
text-component transplant batch ran twice per variant: all five target
additional-image translations copied the donor page's values while every
image byte, target scale, first-image placement and non-target page geometry
stayed fixed. That is causal evidence for the **combined opaque text and
index-row address/length component**, not for any individual coordinate
field or a validated compositor. The copies, reports and PDFs remain
external. No external converter implementation supplied code or pseudocode.

Issue #111 independently recognizes a complete zlib frame at text-relative
`+24`, a little-endian decoded-length field at `+20`, and an observed
decompressed record layout in the two #107 HN-A/C8 documents. The
[text-framing note](hnc8-text-source.md) records the corpus scope and exact
predeclared fixed-row controls. RFC 1950/1951 are functional framing
references; no RFC sample code, external converter parser, raw document text,
or private prefix bytes are copied. The original MIT diagnostic scripts use
the installed Python standard-library zlib module only for optional test
measurements; its implementation is not vendored, linked into the Rust/JS
runtime, or shipped as an asset. Synthetic text frames and rejection cases
are independently authored. Private decoded text, source copies and PDFs
remain external; committed format facts and hashes do not establish a
general placement rule or enable HN/C8 conversion.

The two fixed-row content controls isolate decoded text from row address and
length. A separately predeclared six-copy batch changes four individual
two-byte tail fields or only the RFC 1950 FLEVEL/FCHECK wrapper bits. The
four coordinate changes produce only the predicted selected-axis movement;
the two wrapper controls reproduce the baseline PDFs. These are independent
black-box interventions on two source documents, not independent-document
validation or a source-unit specification. The measured `240/2473` factor
was selected retrospectively from public reference geometry. Signedness,
negative coordinates and general applicability were unresolved when #111
closed; #112's subsequent evidence is scoped below.

Issue #112 adds an independently authored MIT native text-frame reader and
pure empirical placement evaluator. Prefix digests, marker values and field
offsets are format observations from #111, not converter implementation.
The parser streams the opaque sections and retains only bounded raw words
in source image order. Its synthetic tests substitute an invented prefix
digest through a private helper; the public parser keeps the observed
fingerprints strict, and no private prefix/text bytes are retained as fixtures.
The existing locked flate2 Rust backend and sha2 dependencies are reused
under their accepted MIT grants. No codec code or arithmetic-state tables
are copied or migrated.

The [placement-profile plan](hnc8-placement-rule.md) freezes the calibrated
`240/2473` factor, `0.24` point pixel scale and first-type-0 DIB stride width
before new field interventions or native comparisons. These observations
are scoped to the two HN-A/C8 reference documents, not an authoritative
physical-unit specification. The metadata-only native example consumes
source paths and independently checked bytes; reference CTMs, source IDs,
page indices and image hashes are never used as geometry lookup inputs.
Optional protocol reports and all private source copies/text/PDFs remain
external. Pure helpers and diagnostics do not enable production composition.

Four predeclared #112 high-bit/boundary controls distinguish unsigned words
from signed i16 on both coordinate axes and both inspected variants. Three
toggle only decoded bit 15; the C8 y control changes one two-byte logical
slot to 32,768. All other decoded bytes, original index rows/spans, image
bytes and unrelated output geometry are fixed. Eight repeatable black-box
runs match unsigned predictions, including off-page positions. The report
has SHA-256 `201e5ee16b8435777b8d2b14808747b876d9f58b4b11a3f7498476586e927b94`.

The source-only native diagnostic independently predicts all 75 page boxes,
75 first draws and 50 supplemental all-six transforms in those documents;
its report has SHA-256
`6d892bc3d41f23f71d503c689411d2bc2f9f30ecd0fb8bcd61b1c95ec5a10048`.
The binary, all 133 Rust/Cargo files, original sources, reference PDFs and
runtime/tool identities pass before/after audits. HN-B's six source rows are
counted separately as unsupported by this framing profile. The evaluator's
full raw-u16 mathematical domain and original synthetic endpoint tests do
not establish every possible vendor coordinate range or unseen layout.
Physical units and general document applicability remain unproven. No
private text/prefix bytes, source copies, reference converter code, decoded
pixels or arithmetic tables are introduced by these diagnostics.

Issue #116 extends the existing original MIT PDF writer with reusable
raw/JPEG image handles and ordered affine placements in `pdf/document.rs`,
`pdf/mod.rs` and `pdf/writer.rs`. The document identity, fixed-capacity decimal
formatter and object-index preflight are independently authored standard
library glue; no PDF library, converter implementation or new dependency is
copied or introduced. The functional PDF matrix/image/stream rules come from
the PDF 1.7 sections already referenced in the [writer note](pdf-writer.md).
Original source-only unit fixtures and `pdf_placement_bounds.rs` /
`pdf_placement_render.rs` generate tiny asymmetric images at runtime. Installed
qpdf, MuPDF, Poppler and libjpeg-turbo are independent test-only black boxes,
not linked or distributed assets. No HN factors, private source/image/PDF
bytes, arithmetic states or external document compatibility results are added
by this generic PDF primitive.

Issue #117 adds original MIT composition glue in `hnc8/compose.rs` and
`hnc8/image_emit.rs`, minimal shared codec emitters in the existing owned
type-0/JPEG adapters, and synthetic unit fixtures in `hnc8/compose/tests.rs`.
The optional `hnc8_page_composition` native example and
`scripts/hnc8_page_composition.py` verifier independently observe complete
source pages. The existing scoped text parser has a private invented-prefix
test seam only; public observed fingerprints are unchanged. No Python, Go,
private Rust HN/JBIG or other converter implementation is inspected, copied
or transliterated. No new Cargo/npm dependency or arithmetic table is added.

The composition rule uses the independently measured #107/#112 profile:
padded type-0 width, first-image page box, raw source words, descriptor order
and negative-height CTMs. The existing row API supplies top-first packed rows;
one caller-owned bounded random-access store reverses their row order while
keeping all padding samples. JPEG payloads remain exact and SHA-revalidated.
Original synthetic fixtures use invented QM states and a text-prefix digest,
not external table states or opaque document bytes. Independent PDF tools
remain runtime test-only executables, never linked or distributed code.

The [predeclared protocol](hnc8-page-composition-protocol.md) was committed
before private source/table/sample extraction, conversion or rendering. It
requires complete ordered metadata, all padded type-0 samples and every
pixel of all 75 HN-A/C8 and two HN-B output pages, with exact source/tool/
reference/native identities and before/after audits. Missing optional inputs
are `NOT_RUN` with zero compatibility passes. Private documents, text, state
values, samples, PDFs, renders and execution artifacts remain outside Git;
only reviewed metadata and outcome hashes belong in the evidence record.

The first comparison remains a recorded failure at the reference sample-
dictionary guard. A separately predeclared
[dictionary-only probe](hnc8-page-composition-dictionary-probe.md) observed
explicit Flate identity parameters, without converting, extracting samples
or rendering pages. The [controlled comparison amendment](hnc8-page-composition-identity-params-rerun.md)
accepts only that direct four-field, one-bit, one-color, Predictor=1 profile.
Its interpretation follows the linked Adobe PDF reference and qpdf inspection
documentation, not converter source. Original asymmetric runtime PDFs test
every padded bit through independent tools. The original failure and probe
metadata are pinned and audited; the native executable and Rust/Cargo source
must remain identical to the first attempt. Unsupported required comparisons
are explicit failing subsets, never compatibility passes. Full-page and
full-array evidence remains required.

## Dependency inventory and review

The workspace contains three owned packages. The dependency column lists
direct third-party Cargo dependencies in the current graph:

| Package | Role | License | Edition / minimum Rust | External dependencies |
| --- | --- | --- | --- | --- |
| `caj2pdf-core` | Platform-neutral library | MIT | 2024 / 1.88.0 | `fax`, `flate2`, `sha2`, `xberg-ttf-parser` (direct) |
| `caj2pdf-cli` | Native executable | MIT | 2024 / 1.88.0 | Unix: `signal-hook`; Windows: `ctrlc`, `winapi-util` |
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
Issue #104 also uses the installed Poppler `pdftoppm` and libjpeg-turbo
`djpeg` executables for optional rendered-pixel comparisons. These remain
test-only black-box tools; no decoder or renderer code is linked or shipped.

Issue #26 initially added `sha2` as a development dependency for the ignored
[`qm_official_external.rs`](../crates/caj2pdf-core/tests/qm_official_external.rs)
test. It rejects a separately supplied T.82 fixture above 16 KiB and checks
its pinned SHA-256 before parsing. Issue #104 moves the same locked `sha2`
version into the normal core dependency graph to compare the selected JPEG
bytes read during marker preflight with the bytes streamed into a PDF image
object. The hash retains fixed-size state and no image-sized allocation. The
selected grant for each package below is **MIT** from its distributed
`LICENSE-MIT`; every inspected
package manifest says `MIT OR Apache-2.0`. The versions are pinned in
[`Cargo.lock`](../Cargo.lock). Feature and target scopes were checked with
`cargo tree --locked -p caj2pdf-core -e features` for Linux x86_64 and
`wasm32-unknown-unknown` on 2026-09-27.

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

Issue #12 adds no Cargo dependency. The CLI's argument parser, JSON string
encoder, temporary-file handling, and terminal check use only the Rust
standard library (`std::io::IsTerminal`, `std::os::unix`), so the
`caj2pdf-cli` row above still lists no external dependency.

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

## Original external vendor manifest tooling (#125)

[scripts/vendor_fixtures.py](../scripts/vendor_fixtures.py) and
[tests/conformance/test_vendor_fixtures.py](../tests/conformance/test_vendor_fixtures.py)
are original MIT schema, integrity, regeneration and runtime-control code.
The versioned contract is documented in
[vendor-fixture-manifest.md](vendor-fixture-manifest.md). No converter or
proprietary implementation was read, copied, translated or linked to build
this tooling. It adds no Cargo or Python package dependency; it uses only
the Python standard library and POSIX file descriptors for confined I/O.

The original control generator invents two asymmetric small RGB arrays,
encoded PPM wrappers, raw/canonical/normalized Unicode byte records, and
non-executable source/installer/library/plugin/font/tool stand-ins at runtime.
Even its displayed build, container, commit and provenance URL are expressly
synthetic pins. They are integrity/schema controls and never vendor or format
compatibility evidence. None of the generated files is committed, and no
vendor installer, viewer, container, external document or decoder is launched.
Actual Python CLI children in its tests are reported separately as public
controls; no-input mode has zero file, process, vendor and comparison work.

The reader preserves distinct encoded artifact, raw complete-array, strict
Unicode and named normalization identities. Acquisition origin, physical
coverage, runtime settings and clipboard freshness remain reviewed external
declarations, bound by the receipt's canonical observation digest; hash
agreement alone cannot prove acquisition or pixel/text parity. Regeneration
streams verified inputs into fresh versions, retains predecessor/failure
history, emits a bounded redacted diff and requires exact bound review pins
before recording a reviewed attestation. Vendor/private content and full
receipts remain external; only reviewed safe catalog metadata may be public.
Current official-vendor acquisition and compatibility remain `NOT_RUN`.

## Original HN outline observation preparation (#119)

[scripts/hnc8_outline_observation.py](../scripts/hnc8_outline_observation.py)
and its [runtime controls](../tests/conformance/test_hnc8_outline_observation.py)
are original MIT diagnostic code. The preserved finite protocol and later
implementation gates are in [hnc8-outline-observation.md](hnc8-outline-observation.md).
This preparation adds no core decoder, native/JavaScript API or dependency.
It reuses only repository-owned MIT process/metadata primitives and the
original runtime PDF wrapper generator. No Python/Go/private Rust converter,
third-party codec/PDF implementation or proprietary application implementation
was read, copied or translated.

The controls invent all title strings, Unicode/whitespace, hierarchy,
destinations, page objects and source record fields at runtime. The invented
title offset differs from CAJ's established layout. They use installed public
qpdf/MuPDF tools only on those original PDFs to establish a narrow command
schema and expose the lossy outline-display behavior. The original MuPDF
single-line value reader is bounded metadata grammar, not a general PDF file
parser or stream decoder. CPython's installed strict UTF-8/GB18030/UTF-16
codecs provide capped diagnostic hypotheses; their runtime files are pinned
before a separately authorized observation, not copied or asserted to be the
source format's encoding.

At the preparation checkpoint, the hash-pinned public matrix and prior
Python-converter generation report provided source counts and candidate PDF
identities, not an independently
proved HN outline title/encoding/hierarchy/destination oracle. Raw private
records, titles, PDFs, command output and receipts remain external. The
adapter refuses DRAFT protocols and has a zero-work no-input mode. The original
controls are public preparation, with zero private/native/converter/vendor
activity. A separately authorized first Stage A ended FAIL before PDF queries
or title discovery; its exact report and unknown cause are preserved in the
protocol document. It is not successful compatibility or negative-field proof.
HN-A field semantics, C8/HN-B applicability, omitted destinations and native
outline parity remain unverified. Their later observation requires an
independently reviewed,
committed frozen executable/protocol and an immutable pre-private-audit
receipt; no skipped or zero-title-only case counts as compatibility.

The selected-source amendment is original MIT diagnostic/schema/control code,
not core HN outline decoding. It keeps all 27 source and six reference PDF
identity audits, but reads header/index fields only from the exact three public
layout-profile source hashes, including the HN-A discovery source. The other
24 are explicitly OUT_OF_SCOPE/NOT_RUN with zero semantic passes; selection
never uses historical success/skip/error labels. Every selected failure and
every required identity mismatch remains fatal. Fixed source-location/reason
enums expose only schema offsets, widths, ordinals and safe source identities;
they do not retain observed values, bytes, titles, paths or arbitrary exception
text. Original controls invent malformed rows/signatures/counts, raised/short/
zero/overreported reads, interruptions, out-of-scope invalid containers, and
an excluded identity mismatch. They prove diagnostic and scope behavior only.
The amendment's public preparation had no new private observation, preflight,
converter, native, vendor, decoder dependency or production API change. The
first frozen files and every historical external artifact remain unchanged.

The separately frozen selected-source Stage A is now closed with discovery
PASS, `POSITIVE_OUTLINE_OBSERVED` and compatibility UNVERIFIED. The
[closed report](hnc8-outline-stage-a-results.md) binds the exact frozen protocol
and external receipts, three header/index observations, six unchanged existing
PDFs, 12 independent qpdf/MuPDF outline queries, 24 validator children and one
runner. All 93 before/after identity records and closing runtime audits pass.
The positive HN-A repeats have 52 entries with identical complete fingerprints;
strict GB18030 at record-relative start zero matches all 52 titles in the finite
enumeration. This is retrospective correlation, not a validated field width,
codec version, hierarchy or destination grammar. Every observed destination
is `/XYZ [null, null, null]`; native `/Fit` emission needs explicit alignment.
No source/title/PDF bytes, private paths or query output are published. C8/HN-B
zero-entry references do not establish absence or omitted-page policy. The
24 unselected sources remain OUT_OF_SCOPE/NOT_RUN with zero semantic passes;
converter/native/render/vendor calls remain zero. The first FAIL/cause UNKNOWN
is preserved. Separate held-out and one-field validation must establish the
supported rule before original bounded native implementation. Native child
and blocker [#137](https://github.com/rwv/caj2pdf-rust/issues/137) defines those
separate evidence gates; #119 remains open with all six acceptance criteria unmet.

The [Stage B proposal](hnc8-outline-stage-b-proposal.md) is original MIT,
public-only protocol/design preparation for #137. It uses the reviewed closed
Stage A report and repository-owned source, supplies no candidate field grammar,
and adopts no executable contract or resource ceiling. The held-out identities
and partial resource calculations are inherited declarations, not newly audited
inputs or positive oracles. No external/private source, PDF, query output,
converter implementation or runtime/environment metadata was inspected for this
proposal; no tests, probes, conversions, controls or native changes were performed.
Two future independently reviewed freezes separate bounded positive baselines
from checksum-aware exact controls. Both remain NOT_RUN and require their own
actual-runtime/environment review and explicit execution authorization.

## CAJViewer public-module load accounting (#143)

The COMMON/LOADER/VALIDATOR fragments in
[tools/cajviewer/run.py](../tools/cajviewer/run.py) and
[test_cajviewer_source_loading.py](../tests/conformance/test_cajviewer_source_loading.py)
are independently authored original MIT code and controls. The
[source-load contract](cajviewer-source-loading.md) preserves the first sealed
P2 FAIL and its unknown missing-file cause. It permits only the two original
public MIT module paths, complete bounded byte identities, four ordered load
stages and fixed safe failure metadata. No proprietary, legacy-converter,
third-party loader or decoder implementation was inspected or copied.

Original controls invent module contents and receipt mutations; they do not
contain vendor files or private source/PDF data. The separately proposed inline
and host embed the exact reviewed repository fragments without adding an
installed module, image change or dependency. Source hashes identify bytes,
not format, vendor compatibility or completed inventory semantics. One
separately frozen isolated original-control runner passed all 25 methods
(19 mandatory, four actual inline and two actual host controls), with ten mocked
callbacks and zero actual child, Docker, ENV/tool probe, inventory, app, vendor,
private, native or converter actions. All 24 source audit rows and the closing
plan identity passed. Root and independent actual closing reviews passed;
invented fixtures were removed, with cleanup-refusal controls preserving known
ledgers and first reasons on failure. The report is 7,112 bytes, SHA-256
`f91f1d1930fe511d903e208006d14fd4216cac4f1cd0f89eae26090bcd016fde`,
and its consumed plan is 9,388 bytes, SHA-256
`6bbe1d82ec5678fa44b5f93a409417a81c61563edaf38902b53fc8a3c1bbe178`.
Their exact input/code basis remains immutable; later English publication edits
do not rewrite it. The limited application/virtual I/O meters do not measure
stdlib/kernel I/O or RSS. Exact-head mandatory hosted native/WASM/MIT/coverage
gates are separate required final-head evidence. Old frozen code, plans, reports
and the first P2 FAIL/cause UNKNOWN remain unchanged. The separately frozen
v11 phase is now closed FAIL, with exact ordered source-load observations
and successful Root/independent closing reviews, documented in
[the runtime-view report](cajviewer-runtime-view-v11.md). #124 is not completed
by this public diagnostic prerequisite.

The v11 report and project-plan update are original MIT English metadata
documentation. The four execute-escape controls used invented callbacks and
no real child, Docker, viewer or private input. The later inline entry
attempted to load exactly the two whitelisted original public modules in an
external pinned runtime; the first loaded and the second declared module read
failed. The host also uses original shared/process helpers; no vendor
implementation was examined. Historical v9 evidence describes
the original inventory module supplied through a pinned read-only bind;
it does not prove that module was installed in the bare image. A separate
transport amendment remains DRAFT, with no additional phase execution.
Old failed receipts and superseded planning/reader assumptions remain external
and unchanged. Vendor/application/runtime bytes, captures, clipboard data,
documents, complete receipts and raw environment values are not published.

## CAJViewer bounded inventory-helper diagnostics (#148)

This slice is independently authored original MIT public source, controls and
English documentation. It reuses only repository-owned original source-loading
and process/metadata primitives. The historical V12 failure is preserved:
helper status PASS and typed exit 0 did not satisfy required empty stderr;
the actual message and cause remain unknown. No proprietary application,
Python/Go/private converter, third-party loader/codec or private HN/JBIG
implementation was read, copied or translated for this work.

| File | Original ownership and basis |
| --- | --- |
| `tools/cajviewer/run.py` | Original MIT helper producer, independent host validator, duplicate-safe bounded parser, canonical entry and fixed assembly compaction; inherited owned public source-loader bytes and both module pins are preserved |
| `tests/conformance/test_cajviewer_inventory_diagnostics.py` | Original MIT control code with invented results and mutations; exercises the actual owned fragments, complete entry/finally and host observation path |
| `docs/cajviewer-inventory-diagnostics.md` | Original MIT English contract from the public issue and reviewed owned source interfaces; contains no actual stderr or runtime/private byte data |
| `PROJECT_PLAN.md` | Original MIT English checkpoint update; separates source readiness from runtime/capability proof |
| `docs/cajviewer-fixtures.md` | Original MIT English fixture-plan update; preserves native blockers and unverified actual acquisition |
| `docs/provenance.md` | Original MIT per-file provenance statement and scope record |

All control stderr/stdout, exceptions, malformed frames and result mutations
are invented. Public-helper stderr is retained verbatim only in a future
external envelope/receipt, under the 4,096-byte excerpt bound. It may contain
public-tool paths; actual bytes are not published, classified or normalized.
Stdout, arbitrary exception messages and tracebacks are not added to the
diagnostic. Hashes describe retained bytes and do not independently establish
that a smaller excerpt belongs to a larger capture.

The shared whitelist belongs to the original public source-loading COMMON;
producer and independent validator use its reviewed bytes. The producer keeps
separate read, retained-capture and diagnostic-excerpt counts. Unknown spawn
status is preserved even when a helper raises. Complete valid failure
accounting is retained before the host refuses the outer helper result, and
later closing/serialization failure does not replace the primary diagnostic.
The canonical entry installs no additional module and changes no image,
recipe, cache policy, resource cap or operational profile. Its source loader
and helper/inventory source pins stay unchanged.

Original mandatory controls and exact-head reviews/hosted gates establish
only source readiness. Their actual original test processes and synthetic
events must be reported separately from vendor compatibility. The completed
source child does not recover earlier stderr, replay a failed operation or
complete parent #124 or fixture epic #123. Actual runtime/viewer/private/native
compatibility remains NOT_RUN with zero passes for this slice. A later phase
requires separate exact frozen review and actual closing evidence. External
vendor/corpus files, raw environment values, captures and complete runtime
receipts remain outside Git.

## CAJViewer fixed host cache profile (#151)

This is independently authored original MIT source, controls and English
documentation. Its basis is the reviewed owned #148 diagnostic API, public
#151 contract and primary Fontconfig/XDG documentation linked in the
[integration contract](cajviewer-inventory-diagnostics.md#fixed-host-cache-profile-151).
No proprietary implementation, legacy converter, private HN/JBIG source,
vendor binary or private document was inspected or copied.

| File | Original ownership and basis |
| --- | --- |
| `tools/cajviewer/run.py` | Original pure host-only profile; copied declared ENV and exact Docker tokens are returned together outside all canonical fragments |
| `tests/conformance/test_cajviewer_inventory_cache.py` | Original controls using invented ENV/helper results and the actual profile/full entry APIs; no runtime/private bytes |
| `tests/conformance/test_cajviewer_inventory_diagnostics.py` | Original owned test helper gains injected expected, initial and closing ENV values; production entry/finally statements remain unchanged |
| `docs/cajviewer-inventory-diagnostics.md` | Original English profile/integration contract and paraphrased V13 finding; raw stderr stays external |
| `PROJECT_PLAN.md`, `docs/cajviewer-fixtures.md` | Original English checkpoint/native-prerequisite updates; source readiness remains separate from runtime/capability proof |
| `docs/provenance.md` | Original per-file provenance and unchanged-source scope record |

The API performs no filesystem, ambient environment, tool or process action.
The canonical entry, source loader, helper/inventory modules, image recipe,
runtime baseline and all caps remain unchanged. Actual assembly identity is a
required original control, not inferred from helper placement. Source controls
and hosted gates establish this child only. Parent #124's actual caller
amendment and separately frozen inventory attempt remain unimplemented and
NOT_RUN. No old receipt/cause is rewritten or operational phase replayed.
The fresh V13 cache finding is paraphrased; actual stderr, ENV maps,
vendor/corpus files and complete receipts remain external.

### CAJ indirect Flate lengths (#159)

The fragment-scanner changes and synthetic tests are original MIT work.
They reuse the already approved MIT flate2 dependency and existing PDF parser;
no converter source or new decoder dependency was imported. Format evidence
is independent inspection of the issue-77 bytes and zlib consumption recorded
in [the CAJ observations](caj-format.md#indirect-stream-lengths-159).
The synthetic compressed streams, integer references, malformed targets and
marker-containing payloads were authored for this repository. External CAJ,
viewer screenshots and derived text remain outside Git. The observed CCITT
stream is still unsupported; Flate progress is not document compatibility.

### HN-A outline field observations (#137)

[HN-A outline fields](hnc8-outline-fields.md) records two additional positive
black-box references and fifteen original copied-input controls. No Python,
Go or private Rust HN/parser implementation was read or copied. Only original
control strings, located numeric observations, counts and identities are public;
source/title/PDF/query bytes remain external. Title encoding evidence justifies
reusing the existing original GB18030 decoder for the stated profile. Unknown
record spans and C8/HN-B applicability remain uninterpreted. Native reader and
PDF destination changes belong to #119 and require their own tests/review.


### HN-A native outlines

The ranged HN-A outline visitor and nullable XYZ PDF destination support are
original MIT implementations based on the independent observations documented
in [HN-A outline fields](hnc8-outline-fields.md). The existing original GB18030
decoder was moved to a shared core module without changing its mapping.
No Python, Go or private HN/JBIG implementation was copied or transliterated.
Tests contain invented records and titles; external documents, title lists,
reference PDFs and caller-owned QM table data are not redistributed.

### Compressed HN text header validation

The compressed-header tags and variable-word handling were independently
observed from inputs and two black-box controls, documented in
[compressed text framing](hnc8-compressed-text-header.md). This original MIT
change removes document-specific prefix hashes and the test-only override;
synthetic tests use format tags with invented payload values. No converter
implementation or external document payload is copied into source fixtures.


### Uncompressed HN-A records and type-0 display width

The original MIT raw-record state machine and shared type-0 display-width
rule use independent input observations and black-box controls described in
[the format note](hnc8-uncompressed-text.md). No Python/Go/private converter
implementation or document text was copied. Fixtures are invented records
with format tags; reference data and runtime tables remain external.

## PDF CCITT indirect stream framing

The `fax` 0.3.0 dependency is MIT licensed (copyright 2021 pdf-rs
contributors; upstream https://github.com/pdf-rs/fax). Its public Huffman
maps and bit-reader types are used to measure Group-4 PDF stream extents.
The upstream MIT license and decoder/table APIs were inspected. The local
async framing walker uses bounded transition rows and the existing ranged
reader; it does not decode or retain full-page pixels. Original synthetic
unit inputs are generated with the dependency's encoder.

This is ordinary PDF CCITT support, not CAJ-specific JBIG or HN code. No
Python/Go/private converter implementation was used for this change. The
external issue-77 source supplied byte-level observations only and remains
outside Git. Successful framing alone does not establish document conversion
or viewer parity.

## Headerless CAJ repeated fragment recovery

Independent byte inspection of external CAJSamples issue 77 observed a
partial `14 0 obj << /Length` header after a complete integer object,
followed by an exact replay of that integer object. The partial header's
bytes through `/Length` match the beginning of an earlier complete object;
whitespace after the name differs. The implementation recognizes this
shape only at a known object boundary after ordinary parsing fails, within
256 bytes, with a unique prior stream object and an exact immediately preceding
integer replay. The prefix must match that stream header before its payload;
trailing whitespace is ignored. A second observed prefix ends partway through
`/Filter /FlateD`, so the rule compares header bytes without special-casing
the last dictionary name. It excludes these inactive bytes from the reconstruction
index rather than searching or patching stream payloads. Original synthetic
positive and negative fixtures contain no external document content.

The experimental conversion then reaches object 19 at byte 41833, whose
indirect length uses `/DCTDecode`. No complete PDF or viewer parity is
claimed by this recovery change. No legacy converter source was consulted.

## JPEG framing and further repeated fragments (issue 77)

The ordinary PDF DCTDecode path now uses original bounded JPEG marker
traversal, consistent with the project's existing original HN/C8 marker
reader. It skips length-delimited segments and accounts for entropy stuffing,
restart markers and multiple scans; it does not decode pixels or relax the
HN/C8 profile validator. Original framing fixtures deliberately exercise
marker structure without claiming decodable image pixels. No new dependency
or legacy implementation source is used.

Further independent byte observations require recovery beyond the initial
header-only/immediate-integer shape documented above:

- Truncated integer headers or endobj keywords followed by their complete
  copies. Recovery requires the reference and value measured from the latest
  framed stream, and a byte-identical prefix of the complete scalar object.
- Partial known page dictionaries and stream prefixes, followed by an exact
  copy of an earlier indexed integer. Compare at most 256 bytes at a failed
  object boundary against prior source spans; reject ambiguous candidates.
- Complete repeated objects. Compare every byte in bounded chunks and retain
  one identical copy in source order; differing copies remain errors.

The external file now passes object scanning and reference-length validation,
but conversion rejects page object 4 because no direct or inherited MediaBox
is available. An independent bounded byte inspection found no `/MediaBox`
name in this file. No page size is guessed and no output PDF or viewer parity
is claimed. The external corpus and generated artifacts remain outside Git.

## Missing inherited CAJ page boxes

Original vector controls and a pinned CAJViewer black-box experiment establish
its Letter fallback when MediaBox is absent. The converter writes that box
only on a synthesized CAJ page-tree root, preserving descendant overrides.
See [the experiment and selected real-page comparisons](cajviewer-page-boxes.md).
No vendor or legacy converter implementation was read or copied. The source
rule is a documented viewer-compatibility fallback, not inferred lost metadata.

## Type-3 source-page storage and geometry

The #118 integration and its serial scratch views are original MIT code,
reusing the repository's independently authored decoder and fixture builder.
No dependency was added. The four-image black-box observation, reference
revision and source digest are recorded in
[the source-page note](hnc8-page-composition.md#actual-external-observation-and-remaining-work).
External reference libraries, probability tables, documents and output
pixels remain outside the repository. That observation establishes selected
image padding/orientation only; the full-source attempt found an unsupported
text layout and is explicitly recorded as a failure. Original mixed-page
fixtures and independently rendered synthetic pixels are separate evidence.

## Direct compressed HN/C8 text records

The additional `hnc8/text/records.rs` parser and its tests are original MIT
code based on bounded byte observations, using the existing flate2 backend.
It does not decode or copy document text, legacy HN parsing code or probability
tables. [The direct-frame note](hnc8-direct-text.md) records the source identity,
black-box reference revision, four-page comparison and separate Poppler
orientation control. External documents, derived pixels, reference libraries
and diagnostic control PDFs remain outside Git. The failed earlier run and
the separately failed pull-72 reference attempt remain failures.

## Raw compact records and repeated image groups

The shared raw/direct consumer and bounded group comparison are original MIT
code derived from external source-byte observations and black-box output.
[The rule, sample identity and comparisons](hnc8-repeated-groups.md) distinguish
source mapping from Python page-count and color-declaration defects. No legacy
implementation was inspected or imported. Unit images and records use existing
original fixture builders; documents, tables, JPEG payloads and renders remain
external. No dependency was added.

## Native file-backed scratch

`native::FileScratch` and its filesystem tests are original MIT project code,
factored from the source-page example's existing file I/O. It uses only Rust
standard-library file operations and existing core limits; no external code,
codec states, document data or new dependency is introduced. The example now
retains only its resource counters around the reusable adapter.

## JavaScript random-access scratch

The Node FileHandle and browser OPFS scratch adapters, shared bounds checks,
original in-memory test bytes and real-worker tests are MIT project code.
They use standard platform I/O and introduce no dependency, codec state data,
document payload or reference implementation. Browser behavior follows the
[File System standard](https://fs.spec.whatwg.org/#api-filesystemsyncaccesshandle).


## HN/C8 WASM conversion bridge

The four-store request bridge, caller-table configuration, JS lifecycle handling
and original tiny HN fixture are MIT project code. The fixture uses an invented
constant probability model and asymmetric 101/010 pixel rows; it contains no
normative state rows or external document bytes. Runtime state validation is
not a distribution grant. QM/MQ data remain caller-supplied while #30/#44 are
unresolved. No dependency or legacy decoder source was added.

The real C8 Node/Chromium runs described in [JS validation](js-validation.md)
used externally supplied MQ data and an external corpus document. Those inputs,
output PDFs and private harness files remain outside Git and npm packages.


## Experimental HN/C8 CLI integration

The CLI state-file parser, page routing and tests are original MIT project
code. Anonymous file creation is factored from the existing stdin spool helper;
the existing native `FileScratch` and core converter provide storage and PDF
conversion. The tiny HN test uses the same original invented model/pixels as
the WASM fixture. No normative states, legacy implementation, external document
bytes or new dependency are committed. Runtime state files remain caller inputs
and are protected against output overwrite.


## HN/C8 metadata adapters

The CLI outline collector and WASM metadata visitor are original MIT adapters
around the existing independently measured HN-A record parser. Its declared
count accessor centralizes the already validated container offset calculation;
record contents still require visitor validation. Tiny original nested-bookmark
controls exercise metadata and PDF output without external state tables in Git.
No new format inference, decoder data or dependency is introduced. C8/HN-B
outline absence is deliberately not inferred.

## HN-A/C8 declared geometry (2026-09-29, #123)

[Controlled vendor observations](cajviewer-hnc8-kdh.md#controlled-geometry-checks)
independently varied HN-A header words at 0xa8/0xaa, C8 words at 0x20/0x22,
and raw image-record words at +8/+10. They establish separate page/display
extents for those observed profiles, not a normative physical unit or HN-B
layout. The parser retains these u16 words; composition uses the already
explicit empirical coordinate scale. Original synthetic tests vary extents
independently of encoded pixels, exercise supported text framings and reject
zero extents. Existing PDF row-stride handling removes DIB storage padding.

All implementation and tests are original MIT work based on source bytes and
black-box viewer interventions. No Python/Go/vendor decoder implementation,
external document, capture or codec probability table is included. Rust callers
constructing `hnc8::Header` or `RawTextCoordinate` must initialize the new
`page_size` or `width`/`height` fields; HN-B header geometry remains `None`.

## CLI cooperative signal handling (review #193)

The CLI uses `signal-hook` 0.3.18 (MIT OR Apache-2.0, used under MIT), with
only its flag API and default features disabled. Its registry dependency
`signal-hook-registry` 1.4.8 is MIT OR Apache-2.0; its `errno` dependency is
MIT OR Apache-2.0. Existing `libc` is MIT OR Apache-2.0. Target-specific
`windows-sys` and `windows-link` are MIT OR Apache-2.0. Versions are pinned in
Cargo.lock. Original project glue only sets/checks cancellation flags; no
external handler implementation was copied into this repository.

## Streaming bilevel PDF compression (#195)

Original MIT glue uses the existing `flate2` 1.1.10 Rust backend and locked
`miniz_oxide` 0.9.1 dependency; no new dependency or external implementation
source is copied. PDF `/FlateDecode` and indirect `/Length` use the existing
PDF 1.7 reference above. Inspected backend allocation structure: fixed
dictionary/hash buffers, code buffer, local output buffer and Huffman tables,
with no image-size allocation. A conservative 512 KiB reservation is checked
before `Compress::new`; re-audit it when changing the locked backend. As with
the existing inflater, backend allocation is infallible at the Rust allocator
level; the reservation rejects configured-budget violations, not OS OOM.
Project-owned output buffering is fallibly allocated and capped at 16 KiB.
The original synthetic fixtures and independent qpdf/render checks are extended
to decode Flate streams; external C8 evidence is hash-only in the
[compression report](bilevel-compression.md).


## Windows adapter dependencies (#204)

The Windows CLI uses `ctrlc` 3.5.2 (MIT OR Apache-2.0, selected MIT) for safe
console interrupt registration and `winapi-util` 0.1.11 (Unlicense OR MIT,
selected MIT) for file type and volume/file identity queries. Both use the
already audited `windows-sys` 0.61.2 / `windows-link` 0.2.1 under MIT. Their
published MIT notices and Windows source paths were reviewed; no converter,
codec or vendored native implementation is imported. Project source retains
`forbid(unsafe_code)`; OS FFI is encapsulated by those dependencies.

Both direct dependencies are `cfg(windows)` only, with default features and no
optional features enabled. They are absent from Unix and WASM build graphs.
Cargo.lock also resolves ctrlc's other-platform packages, but those do not
build through this Windows-only dependency. CI audits the Windows graph
explicitly. Native Windows regression tests cover Unicode paths, file identity
(including hard links), bounded spooling, staged-output cleanup and pipe I/O.

Windows CI downloads upstream qpdf 12.4.2 and MuPDF 1.28.5 archives, pinned by
SHA-256 in `scripts/install-windows-test-tools.ps1`. They are independent test
programs (MuPDF runs under x64 emulation on Windows ARM64), never Cargo
dependencies or release contents. Their own upstream licenses remain distinct
from the MIT converter. Windows render tests remain enabled.

## Extended platform matrix (#209–#211)

The additional workflows and target inventory are original MIT project glue.
Musl cross targets use the pinned Rust distribution's self-contained runtime
and LLVM linker, with explicit static CRT selection. LoongArch GNU uses the
Loongson build-tools 2025.08.08 GCC 15.1.0 / binutils 2.45 / glibc 2.42 archive,
pinned by SHA-256 in CI. Compilers, libc runtimes, emulators, VM images and PDF
validators retain their upstream licenses; they are not relabeled project MIT
source. No converter or decoder source is imported. Test-only counters use
pointer-width atomics so CPUs without 64-bit atomics can compile the same
regressions. Conversion limits and public APIs are unchanged.

Candidate VM/Tier 3 jobs are discovery evidence until explicitly promoted to
the release inventory. Building their std with a separately pinned nightly does
not change the main stable toolchain. Failed candidates are not compatibility
passes and cannot silently contribute release assets.

LoongArch runtime tests pin upstream QEMU 10.0.2 by SHA-256; the Ubuntu 24.04
QEMU 8.2 run produced incorrect resident-budget arithmetic while the same
source passed with QEMU 10 locally. ARMv6 hard-float uses the hashed Bootlin
2025.08 sysroot because Ubuntu armhf libraries require a newer CPU. VM tests
install upstream validators (including hashed NetBSD X libraries and an illumos
pkgsrc bootstrap) outside project artifacts. The Android adb test adapter is
original MIT Python glue; Android NDK/runtime and emulator tools are external.
Two device-unavailable renderer tests are explicitly filtered on Android, with
host qpdf/MuPDF validation of actual device output recorded separately.

The expanded container mapping and archive tests are original MIT project code.
Container emulation pins tonistiigi/binfmt qemu-v10.2.3-68 by OCI digest;
external emulator code is not copied into project source or release images.
Android uses the official NDK 28.2.13676358. RISC-V32 builds Rust std with its default features; its musl probe uses
the external SDK dynamic runtime because no bundled static unwinder is distributed.

The OPFS cleanup hardening and deterministic fault-injection tests are original
MIT code. Lock semantics were checked against the WHATWG File System Standard
(https://fs.spec.whatwg.org/) and Chromium's file-writer lifecycle. No browser
implementation was copied. A transient lock is a mitigation hypothesis for
issue #213, not a confirmed diagnosis of its single observed CI failure.

Further rare-platform probes use Bootlin stable-2025.08-1 SDK checksums
published by Bootlin. RISC-V32 also probes Debian's static QEMU 10.0.13 package,
SHA256 pinned and extracted locally without changing the host package sources.
The package is a CI tool and is not included in released archives or containers.

`scripts/install-bootlin.sh` is original MIT CI glue shared by required and
experimental matrices; it verifies SDK/emulator checksums and records the
actual host kernel/compiler. It does not contain toolchain implementation code.

## Release build attestations (issue #215)

The release workflow uses the MIT-licensed official `actions/attest` action at
`1e69f48acb82d1966a394da916b4c1698aa569d6` (v4). It is a CI tool; no upstream
implementation is copied into this repository or packaged with the converter.
The workflow integration and [verification documentation](build-provenance.md)
are original MIT work based on the official action inputs and GitHub CLI.
Build attestations authenticate released bytes and workflow identity; they do
not change the independent source/format/fixture rights records in this file.

### Native C8 text feasibility (#223 / #229)

The bounded `c8_text_probe.py` diagnostic and its tests are original MIT work.
It reuses the independent container index reader and treats Python's stdlib
GB18030 codec as a black box; no converter implementation was read or copied.
Character/position controls were verified by changing only selected words in
an external C8 source and observing the pinned offline viewer. Unknown font,
vector and character semantics remain explicit. See `native-text-feasibility.md`
for the bounded go/no-go decision and separate ordinary-copy/image-only
controls. External text, images, fonts and source mutations are not committed.

### C8 native-record inventory and field controls (#232)

`docs/c8-native-records.md` records an original bounded-span survey of the
SHA-pinned issue-66 source and four original single-word controls observed
through the pinned offline viewer. Whole-file comparisons verify mutation
boundaries; repeated captures are checked separately from navigation-dependent
raster differences. No reference converter source was read or copied. Only
field/count/hash observations are included; external documents, derived text,
mutants and captures stay outside Git. Record/style/font semantics still marked
unknown are not promoted to production support by this note.

The Rust `hnc8/native.rs` record visitor and its tests are original code written
from those bounded-span observations. Test positions, payloads and error cases
are invented, including embedded marker values to check atomic record framing.
The visitor retains raw codes/styles without copying another decoder or
claiming Unicode/font semantics. It reuses the existing MIT I/O, limits and
cursor-failure contract. No fonts or external document data are bundled.

The native character helper reuses the existing independently generated
GB18030 mapping through a shared, allocation-free two-byte lookup. Its A0
alphanumeric extension was established by original 62-character source
mutations and the pinned viewer's ordinary-copy output, checked against the
invented ASCII alphabet; the selected original text provides a second control.
Only encoding facts/hashes are committed. Viewer copy normalization and
unverified special/private-use mappings are not silently adopted. No viewer
source code or font data was read/copied into this implementation.

### Additional paired raw HN-A framing (#225)

The paired raw prefix and scoped zero-payload `0x80ce` control were measured
from the two indexed source spans recorded in `hnc8-uncompressed-text.md`.
No other converter implementation was read or copied. Rust record-boundary,
truncation, limit and cancellation tests and JavaScript mixed-image controls
are original MIT fixtures. External documents and derived PDFs remain outside
Git. The two selected HN-A source/output page frames were compared using the
pinned offline viewer; residual cover raster differences remain explicit.
C8 image-less rows contain visible text in that viewer, so their native record
interpretation/rendering is a distinct unresolved prerequisite (#229).
This change reuses the existing bounded compact-record reader.

### Compact HN-B index controls

`tools/cajviewer/hnb_index_fixture.py` and the reader extension are original
MIT code. The generator creates two invented pages with unequal lengths and
crosses 12/20-byte rows with two observed layout markers. It reads no external
document or converter source and includes no fonts, outlines or source text.
The independent viewer observations and admitted limits are recorded in
`docs/hnb-compact-index.md`. All external captures/documents remain outside Git.
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
under #233; see [the output contract](pdf-native-text.md).

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

This maintained distribution carries nine upstream fixes, including bounded
composite-glyph traversal and maximum-glyph-count `loca` handling. Its
[published README](https://docs.rs/crate/xberg-ttf-parser/1.1.0/source/README.md)
identifies each change; the upstream project continues to consume the
published crate. Our adapter reads metadata only; these upstream outline
fixes do not imply that we validate or render arbitrary font outlines.
The minimum Rust version becomes **1.88.0**, matching the dependency and the
updated workspace MSRV check. Native platform compiler pins are unchanged.

The former `ttf-parser 0.25.1` was rejected by the advisory gate under
RUSTSEC-2026-0192 (unmaintained). A local `read-fonts 0.44.0` migration passed
244 PDF tests but was rejected: its required `font-types` → `bytemuck_derive`
→ `proc-macro2` → `unicode-ident` build chain requires Unicode-3.0 in addition
to MIT. That experiment is not part of the shipped source or lockfile.
No license exception, advisory suppression or local parser fork is used.


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
rectangle/triangle outlines, original names, a two-character cmap, metric
and location tables, and SFNT checksums. No external glyph designs or font
bytes are used. Its labels `A` and `中` test Unicode mapping, not authentic
letterform design. The two-page mixed fixture tests baseline placement,
image-over-glyph ordering, a segment and font reuse; negative tests cover
missing/foreign resources, malformed names/maps, limits, I/O failure,
cancellation and dropped pending operations. Independent local qpdf,
Poppler and fontTools checks concern these original fixtures only, not
successful native C8 document conversion. Test exports and external fonts
remain outside Git.

### Original C8 style controls

`tools/cajviewer/c8_style_fixture.py` is original MIT fixture-generation code.
It writes an invented five-character, eight-row document from observed format
constants, without loading or transforming an external document. It contains
no copied converter implementation, font bytes or glyph outlines. The raw
header identifier is an observed format fact. Viewer observations and their
limits are recorded in `docs/c8-native-records.md`; screenshots remain external.

The same builder's additional control/coordinate variants insert independently
chosen records into the original rows. They establish raw framing observations
for `8072..8074`, `c053/c054` and `8010/1`; they do not copy source-page content
or infer permission to discard required rendering semantics. The Rust visitor
retains every newly admitted tag/value and uses its existing fixed buffer.

The original symbol/permutation controls establish the explicit `a0a6`,
`aab3`, and `aca3` character exceptions through visible glyphs and ordinary
viewer copy. The source-independent generator retains both column orders;
Unicode mappings are recorded as format observations, with no font data or
proprietary character-map implementation copied into the repository.


### External FreeType call observation

The C8 font-call follow-up uses an original MIT forwarding shim against public
FreeType declarations. Only public face names, units/em and numeric size and
transform arguments are recorded externally. No vendor implementation,
proprietary font program or glyph outline was copied into the project. Original
control-page captures agree exactly with the earlier uninstrumented captures.
These limited observations do not establish font redistribution rights or
complete C8 rendering support; see `docs/c8-native-records.md`.


### Original geometric-font size controls (#240)

`tools/cajviewer/c8_geometric_font.py` is independently authored MIT code.
Its square and half-square contours are constructed from coordinates, with
no external font input. Family/resource names and cmap aliases are factual
viewer observations; no vendor outline data or implementation is copied.
External MIT fontTools 4.62.1 generates the diagnostic fonts, not production
conversion output. The short `size-profile` extension to the original C8
fixture generator is also original MIT code. Generated fonts, documents and
viewer captures stay outside Git. The recorded zoom observations disprove
one preview hypothesis; they do not certify a production rendering rule.


The #240 compact/alternate segment controls extend the original C8 generator
using the already observed native record framing. Coordinate and raster-width
observations use original geometric inputs and the pinned offline viewer;
no vendor implementation is read. The public notes distinguish established
translations from the still-unapproved PDF rendering rules.

The `hnc8::Header::native_origin` field and its bounded C8 read are original
MIT implementation based on the coordinate/origin controls documented in
`c8-native-records.md`. Tests use asymmetric invented unsigned words, one-byte
reads, truncated origins and HN variants with uninterpreted bytes. No font
size, viewer margin or image mapping is inferred by this metadata addition.
An external original Rust segment diagnostic uses the shared visitor/writer;
its checked PDF and comparisons remain outside Git and are not full-profile
compatibility acceptance.

### Additional C8 encoded-string framing (#242)

`hnc8/native.rs`'s `80cc/01xx` extension, original Rust tests and
`tools/cajviewer/c8_encoded_prefix_fixture.py` were written independently
from indexed record observations and original controlled viewer inputs.
`docs/c8-encoded-prefix.md` records the accepted boundary and rejected
254-character probe. No other converter source, vendor outlines or external
text was copied. The visitor exposes a validated source span with unknown
semantics; it does not silently discard a resource or enable conversion.

### Additional native control boundaries (#242)

The short and eight-byte record extensions in `hnc8/native.rs`, their Rust
regressions and `tools/cajviewer/c8_native_control_fixture.py` are original
MIT work. Bounded source observations identified candidate tag/value pairs;
17 original two-glyph controls independently checked their boundaries in the
pinned offline viewer. `docs/c8-native-controls.md` records three visible
state changes and leaves transform semantics unresolved. No other converter
source, vendor outlines or external document text were copied. All captured
and derived external content remains outside Git.

### Native C8 image-reference framing (#242)

The `810a/d300` visitor branch and tests are independently written MIT code.
Original one-/two-image controls establish byte-length/alignment, coordinates
and embedded descriptor order without reading another converter implementation.
`tools/cajviewer/c8_image_fixture.py` is the identical original helper from #235;
`c8_image_reference_fixture.py` adds independently drawn color geometry and
invented reference names. `docs/c8-image-references.md` records successful
controls and excluded failed probes. External documents, captured pages and
reference bytes stay outside Git. No pathname resolution or viewer code is used.
### HN-B raw glyph-run traversal

The HN-B visitor extension and unit controls are original MIT implementation.
It reuses the independently generated two-page controls in
`tools/cajviewer/hnb_index_fixture.py` and their recorded ordinary CAJViewer
navigation observations in `docs/hnb-compact-index.md`. No converter code,
external text, document, font or capture is copied. Only the verified glyph-run
record subset and the independently controlled eight-byte prefix are admitted;
C8-only records retain a separate HN-B rejection. The prefix controls use
invented payloads, including terminal-looking words, and ordinary viewer
observations; they reuse `hnb_index_fixture.py` without external data.
Actual-source probes record bounded counts and error offsets, not extracted
text or successful conversion claims.

#### HN-B in-run controls

The HN-B native visitor extensions and `hnb-run-*` generator controls are original
MIT work. Evidence comes from independently constructed two-page documents and
ordinary pinned CAJViewer rendering with original geometric fonts. No converter
implementation, vendor font outlines or external document content was copied.
The implementation preserves raw record boundaries; it does not infer rendering
semantics from unchanged screenshots. See `docs/hnb-compact-index.md`.

The `hnb-8070-*` controls independently establish the two observed four-byte
`8070` forms. Their original generated bytes reproduce the viewer inputs;
value-dependent visible changes are recorded without assigning unverified
layout semantics. The same generator now shares its numeric-control loop
across `c053`, `80ce` and `8070`; no external bytes are embedded.

The subsequent `8071`, `8073`, `8072/c2c7` and `8067/9` HN-B controls
use sixteen additional original two-page inputs with independent following
position changes. Their repeated viewer observations and exact generated
byte parity establish framing only. All source document probes report raw
counts and located outcomes, without committing document text or claiming
complete conversion.

The HN-B implicit-style diagnostic is based on four original first-row
replacement controls, all with repeated ordinary viewer captures. Missing
`8002` is not treated as proven corruption; no default style is inferred.

### HN-B native image controls

The `800a/d300` admission and tests are original MIT code, based on independently
generated HN-B images and ordinary pinned CAJViewer observations.
`tools/cajviewer/hnb_image_fixture.py` creates asymmetric and solid-color JPEGs
from invented pixels and uses an invented glyph placement. Its asymmetric image
recipe reuses this project's original MIT C8 control design. All 13 generated
inputs were checked byte-for-byte against the external captured controls. No
vendor implementation, glyph outline, corpus image or extracted source text was
copied. Raw framing is admitted separately from unresolved image/text blending.

The following HN-B `8006/a383` and `8072/cdc1` admissions use seven additional
original controls in `hnb_index_fixture.py`, independently captured and repeated
in the pinned viewer. Bare/footer/next-y controls distinguish record boundaries;
no external implementation or document content supplied the fixture bytes.

The `801d/0003` and `8070/001c` extensions similarly use four original
bare/next-y controls and repeated viewer captures, preserving raw values without
inferring rendering semantics. Generated bytes match the captured originals.

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
under #233; see [the output contract](pdf-native-text.md).

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

This maintained distribution carries nine upstream fixes, including bounded
composite-glyph traversal and maximum-glyph-count `loca` handling. Its
[published README](https://docs.rs/crate/xberg-ttf-parser/1.1.0/source/README.md)
identifies each change; the upstream project continues to consume the
published crate. Our adapter reads metadata only; these upstream outline
fixes do not imply that we validate or render arbitrary font outlines.
The minimum Rust version becomes **1.88.0**, matching the dependency and the
updated workspace MSRV check. Native platform compiler pins are unchanged.

The former `ttf-parser 0.25.1` was rejected by the advisory gate under
RUSTSEC-2026-0192 (unmaintained). A local `read-fonts 0.44.0` migration passed
244 PDF tests but was rejected: its required `font-types` → `bytemuck_derive`
→ `proc-macro2` → `unicode-ident` build chain requires Unicode-3.0 in addition
to MIT. That experiment is not part of the shipped source or lockfile.
No license exception, advisory suppression or local parser fork is used.


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
rectangle/triangle outlines, original names, a two-character cmap, metric
and location tables, and SFNT checksums. No external glyph designs or font
bytes are used. Its labels `A` and `中` test Unicode mapping, not authentic
letterform design. The two-page mixed fixture tests baseline placement,
image-over-glyph ordering, a segment and font reuse; negative tests cover
missing/foreign resources, malformed names/maps, limits, I/O failure,
cancellation and dropped pending operations. Independent local qpdf,
Poppler and fontTools checks concern these original fixtures only, not
successful native C8 document conversion. Test exports and external fonts
remain outside Git.

### Required C8 size anchor controls

The six `anchor-field*-large-page` variants in the original MIT style fixture
generator fix a 500×500 page and two pairs of geometric glyphs. They were
authored from observed raw record framing, without external text or vendor
outlines. The existing original geometric fonts and pinned offline viewer
provide the observations recorded in `c8-native-records.md`; generated inputs,
font binaries and captures stay outside Git. These controls refine the #240
baseline investigation and do not establish complete C8 rendering.

### Scoped grayscale text

The grayscale glyph writer reuses the original font/content implementation
and PDF graphics-state/DeviceGray operators. Original rectangle/triangle fonts
verify color isolation, short writes and failed-output poisoning. The C8
reference observations use only generated geometric fonts and authored controls;
the public FreeType shim records bitmap statistics, not bitmap or outline data.
The separately retained original print raster supplies appearance evidence.
No external source code, fonts, spools or screenshots are included in Git.

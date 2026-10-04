<!-- SPDX-License-Identifier: MIT -->

# Issues #117/#122: rational pixel dimensions and complete three-profile rerun

Status: FROZEN BEFORE EXECUTION. Root and the independent reviewer approved the
exact original adapter and protocol after separate original controls. Only the
single root-invoked, no-retry phase declared below may follow this freeze.
Final execution receipts bind the exact parent/child environment and libraries
before private byte audits. Existing frozen helpers and historical reports
stay byte-identical. No private work occurred during author/reviewer controls.

## Reason and interpretation

The last corrected-reference phase failed and remains FAIL. Its report is
`/home/hzc/.cache/caj2pdf-issue117-hnb-corrected-reference/hnb-gray-md4bzpeg/probe-report.json`,
182160 bytes, SHA-256
`fa2fc149139b6833d06747bbf279af1d4a100fae01a9c5f9c1783a72c342d204`.
Its immutable receipt is 62561 bytes, SHA-256
`fb2b01155bd5199c99931fbd82fa1de6d3a986052f58fa911b04cbebacb080b4`.
It established nine preserved objects/four raw streams, two ColorSpace-only
corrections, six rows/two pages/two draws, two direct source decodes, four full
sample pairs and first-page MuPDF equality. The first Poppler canvas guard
failed; corrected/native Poppler output hashes also differed. Therefore
correcting that guard alone cannot establish pixel parity.

Original asymmetric runtime controls independently showed:

| Literal page points | MuPDF canvas | Poppler canvas |
| --- | --- | --- |
| 497.04 × 36.72 | 2071 × 153 | 2071 × 154 |
| 533.28 × 786.24 | 2222 × 3276 | 2223 × 3276 |

Original exact same-renderer pairs preserve every raster byte/pixel. A separate
original control changed only the first CTM width from `497.04` to
`497.03999999999996`; MuPDF stayed identical, while Poppler changed 154 pixels
despite the same 2071 × 154 canvas. This is a numeric boundary observation,
not an observation of the retained private rasters or proof of their fault.

The proposed original source arithmetic evaluates pixel dimensions as
`f64(pixel_count) * 72.0 / 300.0`, rather than multiplying a previously rounded
binary `0.24`. The integer multiplication is exact within the checked source
dimension domain; division rounds the rational 6/25 once. It retains the
empirical 0.24-point model and provides an independently explainable arithmetic
choice. It is not a vendor-unit claim, a source-ID rule, oracle lookup, CTM
adjustment, pixel tolerance or change to the caller-defined PDF placement API.
Any Rust change requires a rebuilt binary and all three profiles again. The
earlier 75-page evidence cannot serve as execution evidence for that binary.

## Explicit reference choice

Reuse the already-created corrected HN-B PDF, without rewriting it again:

`/home/hzc/.cache/caj2pdf-issue117-hnb-corrected-reference/hnb-gray-md4bzpeg/corrected-pdf/corrected-hnb.pdf`

826724 bytes, SHA-256
`2d423e1262142030b9b042a54735edc1776b132ae2fc54a3cbfc4b5f4d6f10fd`.

The two retained legacy HN-B PDFs remain unchanged audit inputs. HN-A/C8 use
their original run1 references. The corrected override must be an explicit
`corrected_hn_b` paths entry bound before private auditing; only HN-B chooses it. Change
only the two in-memory comparison ColorSpace facts from DeviceRGB to DeviceGray.
The public oracle file, source rows, source dimensions, boxes, source-to-output
mapping, all CTMs and encoded stream pins remain untouched. The native CLI
receives only SOURCE, OUTPUT, TABLE and SCRATCH paths, never this override or
any reference/oracle identifier, image array or transformation.

Independently re-prove legacy/corrected preservation before native calls:

- Two complete dictionary-only inventories using the exact pinned qpdf JSON
  command, at most 1 MiB/64 objects/16 streams/8192 nodes per document.
- Reuse only original MIT pure functions from immutable
  `/home/hzc/.cache/issue117-hnb-corrected-reference.py`, 77797 bytes, SHA-256
  `59e174ac567be52401b1877068b1a2c7ec5b3edccb20e5a6afebe9fc06e56273`.
  Its former `run()` is never invoked. Exact Decimal values, typed scalars,
  canonical generation-zero references, bounded complete bijection and direct/
  indirect Length checks must cover all nine objects and all four streams.
- Only original image objects 7/9 may change ColorSpace; dictionaries, page and
  content objects, encoded streams, masks, Decode/DecodeParms and trailer
  semantics remain protected by the frozen contract. Static ID, equivalent
  object references and checked writer bookkeeping are the declared exceptions.
- Eight opaque raw-stream digest calls establish every legacy/corrected stream
  SHA/length, with source JPEG pins additionally checked. Do not retain raw
  stream bytes. One qpdf syntax check completes this eleven-call preflight.

After a complete main-run PASS, independently inspect the fresh native HN-B
complete dictionary inventory and require the exact observed two Gray8 DCT
image dictionaries, no Decode/DecodeParms/masks and the two source payload
identities. Re-run two direct source JPEG decodes and four complete Gray sample
pairs against corrected/fresh-native PDFs using the original bounded sample
adapter. These additional mandatory observations do not run after an earlier
failed comparison. A strict legacy compatibility control must reject the
untouched DeviceRGB basis specifically at `jpeg_color_spaces`, with the exact
designated reference/native mismatch; geometry, unsupported-profile errors or
other failures cannot satisfy that control. Legacy compatibility stays FAIL.

## Full required workload

Exactly one native invocation per profile, in HN-A/C8/HN-B order, with no retry:

`NATIVE SOURCE OUTPUT_PDF TABLE_FILE SCRATCH_DIRECTORY`

The complete success gate is three native calls, zero converter calls,
81 source rows, 77 output boxes/pages, 127 ordered draws, 53 exact JPEG streams
and ColorSpace checks, 74 full padded Type0 arrays, and 154 full-page renderer
pairs from exactly 308 render launches. HN-A/C8 cover all 75 pages; HN-B source
rows `[1,2,3,4,5,6]`, mapping `[1,6]` and no-image rows `[2,3,4,5]` are required.
No first-page substitution or partial/skipped compatibility claim is permitted.
Additional required groups are nine preserved objects/four streams, two direct
HN-B source decodes, four HN-B sample pairs and one exact legacy rejection.

Every Type0 padded bit and every Gray source sample participates. Every full
P6 canvas byte/channel participates independently within each renderer. Compare
exact file size/SHA, payload SHA, dimensions, changed pixels/channels, maximum
and mean/absolute differences. All difference metrics are zero. Record nonwhite
counts, retaining the original policy for legitimate all-white HN-A/C8 pages;
ordered draws, geometry, streams and complete Type0 samples still prevent an
omitted page from passing. Both HN-B pages and source Gray arrays must be
nonwhite/variable. Box/all-six CTM tolerance remains 0.00005 point. No crop, rescale,
antialias change, omitted edge or tolerance expansion is allowed.

The proposed canvas admission is deliberately narrow: pinned MuPDF/Poppler
at 300 dpi, zero-origin positive boxes at least one point per side, source-
derived 1:1 integer nominal pixel grid and bounded finite dimensions. Parse
bounded P6 headers, require both same-renderer canvases identical and admit
only the demonstrated nominal N or N+1 extent per axis; N-1/N+2, unknown
renderers, malformed/tail data and out-of-profile boxes are located refusals.
Compare the entire admitted canvas, including any extra edge row/column. This
is an explicit renderer-grid admission condition, not reconstruction of a
guessed renderer lexer or permission for different pixels. The original
near-boundary controls must also prove a real CTM/pixel mismatch remains FAIL.

## Frozen source, input and tool identities

The original protocol remains
`docs/research/hnc8-page-composition-protocol.md`, SHA-256
`6b423115b903521ffc7d99f2ce592c45e1933649e941afd485fe6a51aa292c93`.
Resolve exact paths from its pinned reference report and the original committed
public matrix. Audit all 27 source rows and all six unchanged repeat PDFs before
and after, plus the corrected HN-B PDF as a separate seventh reference input.
Run2 PDFs are provenance repeats, not new independent compatibility evidence.

The historical byte audits cover the original failure report, dictionary
follow-up report, identity-params failure report/receipt, HN-B framing PASS
report/receipt, and corrected-reference failure report/receipt against their
already committed exact hashes. The immutable owned adapter/helper pin
constants identify their exact external paths. Parse report inputs under a
same-read 2 MiB bound. Keep every old frozen plan/helper identity as a separate
input. Historical PASS/FAIL labels are never overwritten.

Corpus root `/tmp/caj2pdf-ranged-corpus` and baseline report
`/tmp/caj2pdf-layout-reference-final-v7-report.json` remain original inputs.
The unchanged caller table is `/tmp/caj26-official-vector-with-checkpoints.txt`,
1745 bytes, cap 16384, SHA-256
`11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`.
No table value is stored in Git or report, and table use grants no distribution
right. Compiler/cache/build settings and all original native option/budget
values stay those of the original protocol.

The final independently reviewed public/native revision is
`8194264c048206b5f49b8c55714e06b6b1bd7cea`. Its four hosted jobs passed at that
exact head, including exact 27413/27413 source-line coverage in all 54 files
recorded in the Rust LCOV report.
These are public/native engineering gates, not this new private experiment.

| Input | Bytes/count | SHA-256 |
| --- | --- | --- |
| `/home/hzc/.cache/caj2pdf-issue117-rational-build/release/examples/hnc8_page_composition` | 940680 bytes | `e76f557009fea368714fc5866b996dc13b660df52a44598c640d89e0a844cb90` |
| Rust/Cargo source fingerprint | 139 files | `aa85136b67cc450957d610476c97aab76eb86b6c9a4730f7e261cdbbc5a9cb53` |
| `scripts/hnc8_page_composition.py` | fixed public source | `c9ec94a1577a694a19449ce208c0c4facb6fd1179f523f698cb142d267a6208b` |
| `tests/conformance/test_hnc8_page_composition.py` | fixed original tests | `2cc02af1d561332e3026e9388956e1f06edf04007cd21c8d2373654aa3bf04a4` |
| `scripts/hnc8_layout_pdf.py` | 36239 bytes | `5f920335514872b0a1a618bbfef4bb3d0c830ccdf9621544951b608ee42871d2` |
| `scripts/hnc8_layout_reference.py` | 45159 bytes | `7c3725394d999d391e770fabb1a91949a59987bd29d2f53b5fb067ddb9ad17fd` |
| `scripts/hnc8_placement_rule.py` | 39393 bytes | `419fe68d0547f2f6ff8bc55b26daaac7bec7843e6da51441c63afa92d402bf3b` |
| `/home/hzc/.cache/issue117-rational-native-build-receipt.json` | 1490 bytes | `36a109e25e3af2d12b0ef5894be849bbb745225d48c0dd184f42bf835e8b2096` |

The immutable build receipt explicitly records
`AFTER_COMPLETED_LOCKED_RELEASE_BUILD`: it binds the successful exact cargo
argv/target, executable, final source fingerprint, Cargo.lock/build-log hashes
and compiler/cargo metadata queried after completion. It does not claim a
pre-build environment freeze. The old `997d9504...` executable remains a
separately audited, uninvoked input; its file was not overwritten.

The external original MIT adapter is
`/home/hzc/.cache/issue117-rational-full-rerun.py`, 57754 bytes, reviewed SHA-256
`117f7f3d0139b7cc63ea256896c77c12116e8bb63d6ab737424150c97a642cb9`.
This byte-identical adapter and
`docs/research/hnc8-page-composition-rational-rerun.md` have completed independent review.
The immutable execution receipts bind the exact source and committed plan
identities before private byte audits.
No arbitrary native/source pair, corrected-reference path
or code fingerprint is admitted. Final receipts also bind effective parent/
child environments, resolved roots/argv, full native invocations and command
templates. The adapter receives no corpus content from its tests.

Retain the original executable pins for Python/git/qpdf/MuPDF/pdfinfo/pdfimages/
pdftoppm, plus djpeg SHA-256
`b04116b1b30d74bea2075bb0b1bda22d9ac3b4cc830df3176b974672b7b2259e`.
Freeze canonical shared libraries with ldd for those tools plus native before/
after (at most nine tool targets/eighteen startup queries). Disclose shared
djpeg/pdfimages JPEG backend; independent command execution is not decoder-
implementation independence. Tool versions are bounded public metadata.

## Single bounded controller and execution sequence

Use the existing owned serial controller for every nested metadata/extraction/
render call. The frozen phase consists of exactly one root-invoked Python
runner and at most 2047 measured children: aggregate at most 2048. The child
ledger covers preparation inside the adapter, preservation, three natives,
all validators/renders and closing probes; renders are a subset, never counted
twice. No additional root preparation/probe calls occur during this phase.
Public original self-tests, hashes/git/CI work and other pre-freeze preparation
are separate activities, explicitly excluded from execution evidence.

The 1800-second monotonic deadline starts at `run()`'s first timestamp and
covers its preparation, children, all final audits and report persistence.
Interpreter import/argument parsing precede this measured phase and are not
claimed within its duration. Check the deadline before/after persistence; a
late write invalidates the provisional artifact and returns FAIL. Invoking-
process RSS is only its separately labeled self VmHWM; child wait4/sample RSS
is not attributed to that process. A scoped `Commands`
factory replaces only the owned main runner's controller constructor, sharing
the outer monotonic deadline and actual-launch ledger. It restores the exact
previous constructor on success/failure/interruption. Nine closing startup
calls are reserved before work; no nested controller can reset the allowance
or deadline. The outer controller monitors the complete nested session disk
usage during every child's lifetime.

1. With no inputs, return NOT_RUN with zero audits/bytes/tools/counters and
   runner=0. A requested run reserves runner=1; child counts and aggregate
   children+runner are distinct reported fields.
2. Resolve/check public code, tools/native and source fingerprint, create fresh
   0700 session under `/home/hzc/.cache`, and run declared public startup probes.
   Freeze an immutable outer receipt including canonical libraries and exact
   derivation **before any private source/PDF/table/report byte audit**.
3. Complete pinned pre-audits, two inventories and the eleven-call preservation
   proof. Freeze corrected-reference admission and exact in-memory two-field
   basis. No hidden override/global source-ID dispatch is permitted.
4. Call the reviewed main runner once with explicit corrected HN-B override,
   the shared controller factory and frozen binary/source pins. Its own
   immutable generated-session receipt also precedes private baseline audits.
5. Only after main PASS, check fresh HN-B dictionaries, all complete source/PDF
   Gray sample pairs and the exact rejected legacy basis. Any failure prevents
   overall PASS and preserves first failed artifacts.
6. Finally rehash all original sources/six repeats/corrected reference/table,
   new native/code/protocol/helper/test/library/runtime inputs and both immutable
   receipts. Record failed/interrupted attempts, independent nullable wait4 RSS
   and 20-ms sampled RSS. Final deadline/count/audit checks precede overall PASS.

Keep original caps: native180s/128MiB RSS; tool60s for renderer and original
bounded metadata/extraction timeouts; 1GiB virtual; validators512MiB RSS;
native stdout1MiB; diagnostics32KiB; per-raster/file64MiB; nativePDF128MiB,
aggregatePDF256MiB; owned session512MiB; free-space reserve1GiB. Hash/full-array
requests stay <=65536, native/source-spool <=4096. Monitor caps for the entire
child lifetime and reap/kill groups on failure. Scratch/row stores stay bounded
and removed per image; do not hold whole-document image/text buffers.

Store the inner full safe report separately under a declared 2 MiB limit and
bind its identity in a bounded outer report (2 MiB), rather than duplicating
the entire tree. The outer main-attempt entries retain actual record number,
kind/status/exit/byte counts, nullable wait4/sample RSS and whole-phase number.
Each is bound by canonical-record SHA to the exact full command/attempt in the
immutable inner report. Check all these bindings before writing the inner
report and independently rehash that report finally. Complete outer commands
are retained directly. A maximum 2048-entry original synthetic ledger verifies
serialization bounds and detects changed full records. If final serialization,
disk reservation or writing refuses, return a <=64 KiB FAIL envelope with exact
counts/audit summaries and surviving receipt/report identities; do not claim a
persisted complete report. A write crossing the deadline retains its byte-
identical metadata as `invalidated-provisional-report.json`, explicitly
`NOT_ACCEPTED_EVIDENCE`; it is not an overall PASS report. A refused inner
report is also FAIL, never success.

Live per-raster monitoring/RLIMIT_FSIZE covers direct djpeg output as well as
pdfimages and full-page render files, in dedicated bounded sample directories.
Successful source/sample files are removed without retaining redundant arrays.
All private artifacts and original fixture files remain
external; no /tmp heavy artifacts, corpus/reference/pixel bytes, table values
or converter implementation enter Git. Delete successful sample/raster pairs
immediately. First failed pair and original failure report remain available.
Required attempted/passing/failing/skipped/unsupported counts are exact at the
comparison boundary; unsupported is failure and unstarted work is not PASS.

## Completed original controls and independent review

Author source-only controls passed 10/10 in 1.879 seconds. Root independently
passed the exact same 10/10 in 1.880 seconds; the independent reviewer passed
10/10 in 1.881 seconds and independently confirmed actual no-input zero counts.
All reviews used the exact adapter SHA above and the 19518-byte reviewed draft
SHA `2e4165b8944d65569e5df907138a15f2375abbc62960b76d14d5aa08bcb7f393`.
The final delta changes only freeze/review/pin wording and coverage scope.
Each separate self-test invocation used the 85 actual original child calls
enumerated below; none invoked private/native/converter work.

The original two-page fixture
uses fresh asymmetric grayscale cjpeg bytes, nine-object legacy/native PDF
graphs and exactly two qpdf ColorSpace updates. It proves all four streams,
four complete sample pairs and four complete same-renderer page pairs. Its
private three-profile/native events are explicitly mocked logical events:
actual native/converter calls and newly launched runner processes are zero.
The fixture calls `run()` within the self-test interpreter; the reserved runner
event is a synthetic budget event, not an additional actual process. Actual
tools are metered separately:
38 fixture-preparation calls, 44 orchestration calls including eight renders,
two launch-boundary calls and one failed sparse-file size control, total 85.
These fixtures are not private compatibility evidence.

Additional pure/bounded controls cover NOT_RUN with zero byte audits/tools,
pre-receipt audit ordering, main failure/remaining counts, public post-mutation,
post-audit deadline, global shared launch reservation/factory restoration,
aggregate nested disk, full 2048-entry serialization and changed-record
rejection, final write/size refusal, post-persistence deadline invalidation, and
live 64 MiB sample file refusal with
truthful failed-attempt status. Root and independent correctness/provenance
and simplification review found no remaining runtime or maintainability
blocker. Source/helper/native/main/test bytes remain unchanged. The public
engineering gates and original controls remain separate from private
compatibility evidence; no such PASS is claimed by this freeze.

Any unexpected result blocks #117/#122 until a separately reviewed amendment.
Even complete success is corrected-reference HN-B agreement and this measured
three-source scope, not exact legacy RGB-reference parity, type3 support,
unseen-format proof, table rights, production CLI/JS enablement or a release.

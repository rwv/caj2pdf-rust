# Conformance baseline

Current selected-page vendor results are in [CAJViewer fixtures](research/cajviewer-fixtures.md).
CAJ/PDF/KDH selected pages match. After #184, HN-A/C8 page-frame sizes match,
but exact pixels still differ; these profiles remain experimental. Capture
repeatability is scoped to each report. Historical launch/fixture issues
#124–#129 are closed or consolidated into #123.

The [corpus matrix](../tests/conformance/matrix.json) inventories unique inputs
from a pinned revision of the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository. Its
[provenance note](../tests/conformance/README.md) explains canonical paths,
type aliases, reference versions, and measured results. CAJSamples has no
redistribution grant recorded for this project. Keep its documents and every
PDF derived from them outside this repository.

## Current support and release status

This is the current CLI, Node and browser support summary. “Supported” is
limited to each documented input profile, not every file with that signature.
The same conversion core serves all three interfaces.

| Profile | Status on all three interfaces | Verified scope and limits |
| --- | --- | --- |
| PDF | Supported within the [PDF input profile](pdf-input.md) | Representative 11-page output is identical across interfaces; selected viewer pages 1 and 11 match. |
| CAJ | Supported within the [CLI profile](cli.md) | Representative 75-page output with 58 bookmarks is identical across interfaces; selected viewer pages 1 and 75 match. Source page order and valid bookmarks are preserved; legacy Python ordering is not planned (#21). |
| KDH | Supported for validated embedded PDFs | Representative one-page output is identical across interfaces and matches the selected viewer page. |
| HN-A | Experimental image-page conversion | The recorded current native corpus accepts 19/19 HN-A inputs, including paired raw/compressed framing; this is not whole-family support. HN-A source bookmarks are supported and image pages do not require fonts. The complete 163-page, 96-bookmark pre-compression output was identical across interfaces; the current compression checks below preserve decoded pixels and mapping. Declared page/display extents are used; selected frame sizes match, but exact pixels differ. Physical units remain empirical. |
| C8 | Experimental image pages and admitted native profiles | Compressed four-page image output and the six/four/five-page native profiles have [runtime and layout checkpoints](#unreleased-native-c8-checkpoint). Native pages require fonts: explicit, or installed ones the CLI finds. Requested bookmarks are omitted with a warning (no outline is written from unverified metadata); #303 tracks the missing outline evidence. Font/raster differences remain explicit. |
| HN-B | Experimental image pages and admitted native mode-0/mode-2 profiles | With explicit fonts (bookmarks are omitted with a warning), the selected 4/4/6-page documents convert through CLI/Node/Worker with identical per-document outputs. Native mode 2 supports leading images; image-after-text and mode-0 images remain errors. [Independent controls and scoped layout checks](research/hnb-compact-index.md) do not establish original-font pixel parity. |
| TEB, unrecognized layouts, unsupported image/native modes | Rejected | No OCR or silent omission fallback. Located errors identify unsupported HN/C8 content. |

This table describes current main, which v0.4.0 released. Earlier published
artifacts do not gain these capabilities.
The font-free corpus checkpoint below is separate from successful explicit-font
HN-B/C8 runs. Required caller fonts must be provided; a missing resource is not
proof of an unsupported parser profile. Unknown HN-B/C8 outlines remain unknown,
not confirmed empty, and require explicit omission.

The [real-font HN-B checkpoint](research/hnb-real-font-fidelity.md) records three
complete CLI/Node/Worker conversions, fresh selected viewer captures and
remaining substitute-font appearance differences. It does not establish
source-font pixel parity.

The [real-font C8 checkpoint](research/c8-real-font-fidelity.md) covers the admitted
6/4/5-page profiles across all interfaces. Some substitute-font English and
formula text visibly overlaps; font character coverage is not a guarantee
of source typography or readable spacing.

For usage, see the [CLI font flags](cli.md#native-c8-font-resources) and
[Node/Worker font sources](../js/README.md#explicit-fonts-for-native-c8-and-hn-b-pages).
[Unicode fidelity](research/hnc8-text-fidelity.md) distinguishes character transport,
selected source checks and reading-order/whitespace limits. PDF syntax checks,
matching adapter hashes and substituted-font layout checks do not establish
original-font pixel parity or unrestricted copy/search fidelity.

Arithmetic HN/C8 images use built-in standard QM/MQ states. Optional custom
state overrides remain supported. The owner-directed adoption and upstream
practice are recorded in [provenance](provenance.md); #189 completed #30/#44.
JS arithmetic image decoding needs bounded scratch stores; `withHnc8Scratch`
can manage their lifetime for Node or browser Workers.

[Viewer results](research/cajviewer-fixtures.md) record the pinned application and
selected-page scope. [Complete HN/C8 checks](js-validation.md#source-geometry-correction-repeat)
record historical output hashes, page counts, outline retention and image-stream checks.
The [streaming compression report](research/bilevel-compression.md) records current C8
hashes, fixed working memory, independent decoded-pixel equality and the explicit
scope of reused versus rerun evidence. Compression changes encoded bytes, not
page/outline mapping or decoded image bits.
Python-reference corpus expectations below are a separate compatibility
baseline, not a CAJViewer verdict. Missing optional inputs are `NOT_RUN`;
known pixel failures are not passing baselines.

The unreleased [type-1 JPEG extension](research/hnc8-type1.md) adds the measured HN-A/C8
profile and selects the existing HN/C8 text-header interoperability policy in
CLI/WASM. The frozen v0.3.1 results below are not overwritten by this change.

### v0.4.0 pre-tag Linux CI baseline (#328)

The [current CLI observations](../tests/conformance/current_cli_baseline.json)
use the Linux x86_64 GNU artifact from main `0153d22`, CI run `37251767647`.
All 56 source identities matched before and after the run. Default font-free
conversion succeeds for 38 documents: 19 HN, two C8, 12 CAJ, three KDH and two
PDF. Seven TEB and six HN-B/C8 inputs are unsupported with these options;
five damaged CAJ inputs fail. Explicit bookmark-omission repeats add two
successful attempts, not two additional documents.

All 40 successful attempts pass qpdf, page-count and source-order checks.
Source-outline checks pass for 31 attempts and are NOT_RUN for nine. These
are structural checks, not new viewer pixel comparisons. Each child has a
600-second timeout, 1 GiB address-space limit and 512 MiB output-file limit.
The report records the exact binary SHA-256. The Linux executable downloaded
from tag run `37330738513` at `56bf7cc` has the same full-file hash, so these
measurements apply to the tagged executable as well. This identity check does
not substitute for complete release inventory and attestation verification.

### Reproducible v0.3.1 CLI baseline (#218)

The [historical CLI observations](https://github.com/rwv/caj2pdf-rust/blob/0153d22a806b443a9cae01f9277dabbee483f01b/tests/conformance/current_cli_baseline.json)
record a completed repeat on all 56 pinned inputs using the published Linux
x86_64 v0.3.1 executable. Source hashes matched the inventory before and after
all attempts. These are historical v0.3.1 observations, separate from the
Python expectations. The optional runner independently checks page/image
identity and order; it does not perform new rendered-page pixel comparisons.

| Format | Inputs | Successful conversion with the recorded options |
| --- | ---: | ---: |
| HN | 22 | 16 |
| C8 | 5 | 2 (explicit bookmark omission) |
| CAJ | 17 | 11 |
| KDH | 3 | 3 |
| PDF | 2 | 2 |
| TEB | 7 | 0 (recognized unsupported) |

There were 64 conversion attempts: the default on every input, plus an
explicit `--no-bookmarks` attempt on the five C8 and three HN-B inputs.
Default conversion completed on 32 inputs; the explicit option added two C8
outputs. All 34 emitted PDFs have page counts matching current source
inspection. qpdf returns success on 33; issue-20 returns warning status 3,
so that output is not counted as a clean PDF-validation pass. Neither output
existence nor matching page counts alone establishes correct source-page order.
The separate source-order checks pass on all 34 outputs (2,630 pages): CAJ
page-table object IDs, PDF/KDH source page/content-object identities, and
2,128 HN/C8 images compared in page order using Poppler extraction and the
existing pinned codec pixel oracles. Repeated byte-identical source descriptor
groups are recorded separately; this check does not validate their coordinates
or deduplication semantics. All 27 CAJ/HN-A source-outline comparisons pass;
other source-outline comparisons remain NOT_RUN. The repeat produces the same
34 PDF hashes as the initial run.

The run used a 180-second per-conversion deadline, 1 GiB process address-space
ceiling and 512 MiB per-file ceiling. CLI scratch limits remained 64 MiB per
store across four stores. These are configured limits, not peak-memory
measurements. Outputs and raw logs remain outside Git. The POSIX runner needs
Python 3.11+, qpdf, MuPDF (`mutool`) and Poppler (`pdfimages`):

```sh
python3 scripts/current_formats.py \
  --corpus-dir /external/CAJSamples \
  --candidate /external/caj2pdf \
  --output-dir /external/new-format-run > /external/current-format-report.json
```

The output directory must be new and outside the checkout and corpus. With no
corpus argument or `CAJ2PDF_CORPUS_DIR`, it reports NOT_RUN. An explicitly
missing/mismatched corpus fails. COMPLETE means all attempts finished, including
FAIL/UNSUPPORTED results; it is not a compatibility pass. Conversion, PDF
validation, count/order, outlines and pixels have separate result fields.
Each child process has the stated limits; these are not a parent-harness RSS
bound. Image extracts are discarded after each page.

The following gaps describe the frozen v0.3.1 run, not current main. Their
implementation issues are now closed; current outcomes are recorded in the
optimized preflight below and the support summary above.

Reproduced HN/C8 gaps were linked to [#220](https://github.com/rwv/caj2pdf-rust/issues/220)
(image-less HN-B/C8 rows), [#224](https://github.com/rwv/caj2pdf-rust/issues/224)
(type-1 image records), and [#225](https://github.com/rwv/caj2pdf-rust/issues/225)
(additional page-text framing). Six CAJ inputs fail PDF parsing/repair and
were tracked by [#226](https://github.com/rwv/caj2pdf-rust/issues/226);
this does not establish whether their source content is recoverable. Existing
selected-page viewer results have not been rerun by this inventory.

### Fixed regression set selected after v0.3.1

Use the exact hashes/paths in the matrix, not similarly named replacement files.
The full runner remains 56 inputs; these are the small development controls,
not a requirement to capture every page in CAJViewer. The purpose column
records the original selection rationale; it does not override subsequent
explicit-font HN-B success or the diagnosed issue-20 corruption rejection.

| Matrix case | Purpose |
| --- | --- |
| `issue-21/实时网络流量异常检测算法研究和系统实现_林尚朕.caj` | Compressed HN-A text/image placement and source outlines. |
| `issue-69/12.caj` | Raw HN-A text records and multiple images. |
| `issue-76/基于星载合成孔径雷达干涉测量技术的数字高程模型生成研究_任坤.caj` | Full mixed type-0/JPEG/type-3 document and repeated descriptors. |
| `issue-58/混凝土道面评价指标分析_谢永亮.caj` | C8 conversion with explicit bookmark omission; existing viewer controls. |
| `issue-100/中国金融体制改革阶段研究_李卉.caj` | HN-B image-less-row refusal; completeness work must not hide it. |
| `issue-40/*`, `issue-44/*` | CAJ source order differs from historical Python; retain source order. |
| `issue-20/*` | Existing malformed-stream warning remains visible. |
| `issue-48/ZZXX200402047.caj` | KDH embedded-PDF control. |
| `issue-33/test3.caj` | PDF pass-through control. |

Each wildcard above resolves to the single matrix entry in that issue directory.
Add only the directly affected cases for #224 (issue-43), #225 (issue-7,
issue-85 Zhouli, issue-66) and #220 (issue-63/65 and issue-90 C8 rows).
Ordinary CI uses the existing original fixtures and new synthetic checks:

```sh
python3 -m unittest discover -s tests/conformance -p 'test_current_format*.py'
```

These tests detect reordered/missing/changed image identities, a real synthetic
PDF page permutation, malformed bitmap extents, oracle identity mismatches,
invalid inspection metadata, missing output and timeout/error classification.
They are not substitutes for an external corpus run or CAJViewer comparison.

### Optimized CLI preflight after the HN/C8 increments

The 2026-10-03 release-mode preflight of reviewed implementation `4910da9`
completed all 56 pinned inputs; input inventory and binary hashes were unchanged.
This deliberately repeats the original font-free invocation policy. It does not
replace the separate explicit-font HN-B/C8 checkpoints below.

| Family | Inputs | At least one successful conversion |
| --- | ---: | ---: |
| HN-A | 19 | 19 |
| HN-B | 3 | 0 |
| C8 | 5 | 2 (bookmark omission) |
| CAJ | 17 | 12 |
| KDH | 3 | 3 |
| PDF | 2 | 2 |
| TEB | 7 | 0 (recognized, unsupported) |

All successful conversions pass PDF syntax, page-count, source-order and
applicable source-outline checks. Rendered-page pixels are NOT_RUN in this
runner. The 38 successful inputs compare with 34 in the old baseline: issue-30,
issue-43, issue-7, issue-85 Zhouli and issue-92 now succeed; issue-20 now fails.
The issue-20 change must remain explicit: its old output had qpdf warning status
3, and the previously recorded 40,022-byte object-4 payload independently fails
zlib 1.3.1 checksum validation. Reading only its declared 40,020 bytes leaves
zlib incomplete. Current validated Length repair refuses this corrupt stream;
there is no new claim of an independently justified content repair. Historical
source-order/outline success was not full rendering acceptance.

The remaining font-free HN-B/C8 failures do not negate their separate
caller-font conversions, and are not new evidence for speculative format rules.
The earlier debug run was intentionally interrupted and is not a completed
compatibility result. External receipts and candidate binary hash are in
`caj2pdf-hnb-rendering-20261003/corpus-4910da9-release-preflight.json`, its
summary, and `issue20-zlib-check.json`. This is preflight evidence; #222 still
requires the actual packaged release candidate, peak-memory scope, bookmarks
and hardening delivery before release acceptance.

### Bounded malformed-input regression

The original public-conversion tests in `hnc8/compose/tests/malformed.rs`
exercise paired raw/compressed HN-A prefixes, compact/ordinary HN-B indexes and
C8 mixed pages. Each valid two-page control is paired with four later-page
faults: invalid span length, incomplete record, missing tail byte and zero-byte
source reads. Source and font adapters permit at most 4,096 calls per conversion,
with three-byte reads, seven-byte writes and 64-byte I/O chunks. Exceeding the
read budget fails the test itself, so it cannot masquerade as input rejection.

The cases verify page-2 errors after an emitted first page, unfinished PDF output
and empty scratch. Existing record/count/resource/decompression tests retain
their finer boundary checks; existing adapter tests cover cancellation and
cleanup. This is deterministic regression coverage for the admitted profiles,
not a proof of all parser paths, a corpus compatibility result or a security
certification. No external corpus, font or viewer is needed.

### Unreleased native C8 checkpoint

The raw six-page `issue-66` profile now has an explicit-font conversion path
through CLI, Node and a real browser Worker. All six pages convert through the
shared codecs and incremental writer; the resulting PDFs have matching hashes.
All six page renders received page-level source comparison, and independent
MuPDF tracing preserves the 6,638 decoded glyphs in source sequence. This is a
separate conditional checkpoint, not a rerun or reclassification of the earlier
font-free corpus baseline or an independent proof of every character mapping.

Caller-supplied fonts change face, weight, bearings and punctuation spacing.
Decoration contour multiplicity and zoom-dependent repetition are independently
classified with original controls. No source-font fidelity or pixel equality is
claimed. The additional `issue-90/4-[21].caj` and `4-[24].caj` profiles
complete all 4/5 pages with explicit marker resources through CLI/Node/Worker,
with matching output hashes and successful scratch cleanup. All nine pages
received source-layout inspection; PDF tracing preserves all 14,300 decoded
glyphs in order. This is conditional rendering/transport evidence, not an
independent transcription or a reclassification of the font-free baseline.
See [additional C8 controls](research/c8-native-controls.md#complete-document-runtime-and-visual-checkpoint-c2df122).
Bookmarks and unverified native profiles remain unsupported; unknown required
content fails explicitly. See the current status and original-control
evidence in [C8 native records](research/c8-native-records.md), and the resource contracts
in [CLI usage](cli.md#native-c8-font-resources) and [JavaScript usage](../js/README.md).
External documents, fonts and rendered evidence are not distributed. #242 owns
additional-profile final acceptance; #222 owns packaged release/corpus and
peak-memory acceptance.

### Packaged HN-B regression repeat

At core `4910da9`, the actual npm package repeats the explicit-font
`issue-100`, `issue-63` and `issue-65` documents through Node and real browser
Workers. All 4/4/6 pages complete, each output matches the current native CLI,
and scratch/OPFS cleanup passes. Compared with reviewed HN-B `385f3d2`, qpdf
QDF shows only additional F4–F7 font-resource references before the xref table;
content and embedded-resource streams are unchanged. All 14 page renders at
72 dpi are byte-identical. Changed PDF hashes reflect this serialization
change, not original-font parity or a new source-fidelity claim.

External receipts are `packaged4910da9-hnb-regression.json`,
`packaged4910da9-hnb-qdf-comparison.json` and `hnb-4910da9-cli-regression.json`
in the existing acceptance directory. #265 completes the next-gap triage:
no additional independent parser gap is demonstrated by the current corpus
with required caller resources. Unseen native modes and nonempty HN-B/C8
outlines remain unverified; #221 retains bookmark ownership.

### Representative memory preflight (2026-10-03)

Core `4910da9` and the actual extracted npm preflight tarball were measured on
both pinned `issue-76` HN-A inputs: Cao (65 pages, 54 bookmarks, 13,109,383 input
bytes) and Ren (163 pages, 96 bookmarks, 24,519,256 bytes). Source identities
remain in the corpus matrix. Native and Node outputs match the complete CLI
hashes, and temporary stores are empty and removed after conversion.

| Measurement | 65 pages | 163 pages |
| --- | ---: | ---: |
| Native kernel maximum process RSS (KiB) | 13,876 | 14,136 |
| Native accounted page metadata peak (bytes) | 840 | 10,080 |
| Native accounted text working peak (bytes) | 8,208 | 8,264 |
| Node WASM capacity high water (bytes) | 2,555,904 | 2,359,296 |
| Node I/O-boundary sampled process RSS (bytes) | 81,936,384 | 114,909,184 |
| Aggregate decoder scratch peak, native and Node (bytes) | 1,004,328 | 1,089,966 |

Native uses the original page-composition example with built-in standard QM/MQ
tables, release optimization, ranged input, sequential file output and 4 KiB
I/O. Each process is measured separately using Linux `wait4`; RSS includes the
whole diagnostic process. Accounted metadata/text capacities come from
`ComposeReport` and are not a full heap census. The large document exercises
161 type-3 and 49 JPEG draws.

Node uses the actual package, fresh instances, ranged file input, temporary
file scratch and a sequential hashing sink. Maximum input/output requests are
256 KiB. WASM initially reserves 1,245,184 bytes. Its capacity is not live heap,
and I/O-boundary RSS samples are not a kernel peak. Sampling adds runtime
overhead. Scratch is disk storage, reported separately from RAM.

The real browser Worker completed both inputs with identical output hashes,
page/bookmark counts, WASM/scratch peaks and request bounds; OPFS cleanup
passed. Input is spooled to OPFS and output is written sequentially there.
Output hashing happens after conversion and outside the measured path. The
first 163-page browser attempt exceeded its 300-second harness deadline and
remains INCOMPLETE; a separate retry with an 1,800-second deadline completed.
The successful retry does not turn the earlier timeout into a passing run.

These inputs differ in page complexity. A separate original 4/2,048-page
constant-complexity C8 preflight increases WASM capacity from 1,966,080 to
2,097,152 bytes while retaining an 8-byte scratch peak in Node and Worker.
Neither experiment establishes a universal constant-memory or browser RSS
bound. Full versioned release acceptance remains #222.

External scripts, receipts and the native measurement hash manifest are under
`caj2pdf-hnb-rendering-20261003/{native-memory-preflight,packaged-hna-memory}`;
no source documents or generated PDFs are committed.

### 738-page HN-A real-document regression (#284)

The shared catalog's `issue-111/56.caj` (238,910,818 bytes, SHA-256
`39fc809373630c0eeb30ac2471174fcdf976916893389c0b8679d50ca730dc4b`, from
upstream caj2pdf issue 111) is the largest real input checked so far. At main `509bb6e`, with the release CLI and default options:

| Path | Result | Time | Peak memory |
| --- | --- | ---: | ---: |
| Native CLI, ranged file | PASS, 738 pages, 1,450 bookmarks | 105 s | 10.1 MiB RSS |
| Node 22 WASM, ranged file | byte-identical | 368 s | 102 MiB RSS |
| Node 22 WASM, spooled standard input | byte-identical | 286 s | 103 MiB RSS |
| Chromium Worker, ranged Blob, OPFS scratch/output | byte-identical | 499 s | not measured |

The output is 268,737,204 bytes, SHA-256
`db8f4a970e0d51a985b4e807a19dac853f15fd75efb2d5ac26ed768dc3be0888`.
`qpdf --check` is clean, MuPDF renders all 738 pages, and all 1,450 outline
entries match the source titles, depths and pages. Through
`scripts/sample_catalog.py` the runner reports conversion, PDF, page-count and
source-outline PASS. Page-image order is NOT_RUN because the pinned pixel
oracles do not cover this document. The Worker read 430,981,910 input bytes,
wrote at most 256 KiB per output chunk, and left empty scratch and no OPFS
entries. Durations are single observations on one 4-vCPU Linux host; the
spooled Node and browser runs overlapped. Native CPU time goes to JBIG1 layer
decoding and Flate output, not unbounded buffering.

The earlier 90-second attempt was a budget timeout and stays INCOMPLETE; it was
not a converter defect. CAJViewer page comparison is NOT_RUN, so these checks
establish structure, bookmarks and cross-runtime identity, not pixel parity.
Per-run details are in the
[sample research notes](https://github.com/rwv/caj2pdf-samples/blob/main/RESEARCH.md).

### Native file-scratch system calls (#291)

`FileScratch` now caches its logical length and uses positioned reads/writes
instead of `metadata()` plus a seek per request. On the 163-page Ren
`issue-76` HN-A input, release CLIs at main `8cc6291` and with this change
both write SHA-256
`35ef00ccef40e815790ed72d259bcc9b220c726e9c060d85dffa958f86585526`.
`strace -c -f` counted 36,563,024 system calls before (11,522,787 `statx`,
11,583,746 `lseek`) and 13,517,464 after (7 `statx`, 60,966 `lseek`, all
from input reads). Wall time fell from 69–74 s to 57–58 s over two runs each
on a shared, loaded 4-vCPU host. Scratch traffic remains row-sized
read-modify-write pairs; no write buffer was added.

### Known Python-reference differences

These are accepted v0.1 differences, with full source hashes and measurements in
[the CAJ format record](research/caj-format.md). Rust preserves source page-table order
and valid source TOC records. Per the maintainer decision in
[#21](https://github.com/rwv/caj2pdf-rust/issues/21), no legacy Python
compatibility mode is planned. Reference output is comparison evidence, not
an authority for reproducing page permutations or dropped bookmarks.

| External case | Difference / limitation |
| --- | --- |
| `issue-40` | Python orders source pages `1–12, 18–78, 13–17`; Rust retains `1–78`. All source pages are present. |
| `issue-44` | Python orders `1–9, 25–108, 20–24, 15–19, 10–14`; Rust retains `1–108`. All source pages are present. |
| `issue-49` | Python emits zero outlines after a missing-object error; Rust preserves 49 valid source bookmarks. |
| `issue-73` | Python emits zero outlines after a PDF-read error; Rust preserves 100 valid source bookmarks. |
| `issue-20`, page 39 | MuPDF encounters the same zlib/font error on both outputs. This page is not a passing render comparison. |

### Historical v0.1 candidate evidence (#14)

- Original tests cover short I/O, malformed input, cancellation, bounded scratch,
  output cleanup, source-page omission refusal, compressed-stream finalization
  and exact decoded pixels. #196 adds CLI signal cleanup and complete JS Worker
  examples; #197 adds bounded compression and its memory regression.
- [JavaScript delivery](js-validation.md#verified-delivery-paths) and the current
  extracted-package tests cover Node/Chromium consumers. The artifact tests now
  convert both CAJ and C8 using the packaged WASM and scratch helper.
- [Memory measurements](js-validation.md#memory-and-temporary-storage) cover
  small/large original PDF inputs and historical full HN-A/C8 runs; the
  [compression report](research/bilevel-compression.md) measures the changed writer.
  Native RSS, sampled Node RSS, WASM linear memory and scratch caps have distinct
  scopes; no browser-process RSS bound or universal constant-memory claim is made.
- #196/#197 passed Native, WASM, MIT audit and exact 100% Rust line coverage.
  The final audit PR reruns the same gates; skipped private corpus remains NOT_RUN.
- Codec/default integration (#8/#9) is complete. [Candidate artifacts and hashes](../CHANGELOG.md#audited-v01-candidate-artifacts)
  were rebuilt after #197 and the example-path fix. Source inventory contains only documented
  original synthetic fixtures; external documents and vendor/build artifacts
  are absent. The numeric-state adoption record remains explicit in provenance.
- The candidate audit was completed and GitHub releases have since been
  published, most recently [v0.4.0](releases/v0.4.0.md). Native archives,
  JS/WASM and container artifacts include [build provenance](build-provenance.md).
  npm/crates.io publication remains separate; package publishing stays disabled.
  The measurements above are historical and are not a fresh whole-corpus run.
  [Issue #218](https://github.com/rwv/caj2pdf-rust/issues/218) tracks the current
  format baseline; [#217](https://github.com/rwv/caj2pdf-rust/issues/217) owns
  remaining HN/C8 improvements.

## Commands and status

From a clean clone, run the unit tests and check that the original MIT
fixtures match their generator:

```sh
python3 scripts/generate_fixtures.py --check
python3 -m unittest discover -s tests/fixtures -p 'test_*.py'
python3 -m unittest discover -s tests/conformance -p 'test_*.py'
python3 scripts/conformance.py
python3 scripts/jbig1_oracle.py --json
python3 scripts/jbig2_directory_inventory.py --json
```

The optional commands print `NOT_RUN` for external checks when their
inputs are absent. The PDF command prints `NOT_RUN` for the external corpus when
`CAJ2PDF_CORPUS_DIR` is unset. This is a visible skip, never a compatibility
pass. To request an inventory run, point the variable at a local checkout of
the pinned CAJSamples revision:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples python3 scripts/conformance.py --json
```

The [JBIG2 directory inventory](research/jbig2-directory.md#optional-external-metadata-inventory)
uses the same pinned corpus to check the HN/C8 type-3 segment headers through
the Rust core API. It reports `NOT_RUN` when the corpus is absent and does
not claim decoded-pixel compatibility.

The runner checks all canonical files, including size and Git blob hash, using
bounded reads. A missing file, changed hash, unreadable file, or path escaping
the corpus root fails the requested run. Type aliases do not cause duplicate
runs. The concise report distinguishes `PASS`, `FAIL`, `UNSUPPORTED`,
`EXCLUDED`, and `NOT_RUN` for the inventory and PDF checks. An inventory
`PASS` means only that the local corpus matches the pinned matrix. It is not
a Rust conversion result.

Once a converter produces PDFs, place them outside the repository and pass
`--pdf-dir /path/to/output`. Each output path mirrors the canonical input path
with a `.pdf` suffix: `issue-1/a.caj` maps to `issue-1/a.pdf`. The runner
compares available page counts, page dimensions, outline hierarchy and
destinations, and rendered-page hashes against recorded expectations. A
requested PDF comparison fails if an expected output or required inspection
tool is missing. A complete output `PASS` requires all five checks and a
recorded render hash for every page. Unknown reference outcomes or incomplete
successful rows report `NOT_RUN`; a requested `--pdf-dir` exits nonzero unless
the aggregate PDF status is `PASS`. Known reference errors are `EXCLUDED`
from the successful-conversion scope, while known unsupported inputs remain
`UNSUPPORTED`. Top-level page and outline counts are Python `show` observations;
`expected_pdf.page_count` and `expected_pdf.outline_count` are authoritative
for converted PDF output when they differ. `--json` provides a
machine-readable report for later release gating.

To verify one format as its implementation lands, add `--only-format KDH`
(or another detected format). The runner still validates the full matrix,
then checks only the selected corpus files and output PDFs. The JSON report
names `selected_format` and counts only that subset. Missing selected inputs
or requested outputs fail; omitted formats are outside the reported result. For KDH,
the selected baseline has three Python-success documents and 74 fully
fingerprinted pages:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  python3 scripts/conformance.py --only-format KDH \
  --pdf-dir /path/to/output --json
```

PDF inspection and rendering use a separately installed, version-recorded
`mutool` command. Its source and output PDFs are never vendored here. Exact
render hashes are comparable only with the recorded rendering options and
tool version; a different version requires rebaselining and review. The
synthetic [fixture manifest](../tests/fixtures/manifest.json) includes PDF
structure cases that can test these checks without an external document.

## JBIG1-like image oracle

The separate [type-0 image manifest](../tests/conformance/jbig1_oracle.json)
contains only pinned source/image metadata and decoded pixel hashes. Its
[observation note](research/jbig1-oracle.md) records the independent HN/C8 byte layout,
external decoder provenance, hash definitions, and secondary PDF extraction
checks. The opt-in runner uses the same external corpus plus a separately
built, **non-distributed** black-box native oracle:

```sh
python3 scripts/jbig1_oracle.py \
  --corpus-dir /path/to/CAJSamples \
  --oracle-lib /path/to/external/libjbigdec.so \
  --json
```

The ordinary Rust build and CI do not require that external library. A
clean-clone run validates the manifest's schema and reports the image work as
`NOT_RUN`; it does not claim pixel compatibility. A requested run verifies
input hashes, rediscovers image spans, and compares every requested image's
raw-stride and visible-bit hashes with isolated, timed decoder calls. The
pinned corpus currently has 1,400 measured type-0 images, plus three
separately recorded discovery errors in `issue-100` (one image descriptor and
two page rows). Even when all
1,400 images match, the three expected invalid records remain visible in the
discovery report as `expected_invalid_records: 3`, separate from pixel passes.
A changed or new invalid record fails discovery. No corpus
document, decoded bitmap, derived PDF, or differently licensed decoder binary
belongs in this repository or its release artifacts.

The optional [standard T.82 probe](../scripts/jbig1_standard_probe.py) tests a
finite set of constructed BIH/stripe settings against one selected HN/C8
image from that manifest. It requires an external standard `jbgtopbm` binary
and the pinned corpus; the executable stays outside this repository. This
optional probe runs on Linux/POSIX because it limits child output with
`RLIMIT_FSIZE`:

```sh
python3 scripts/jbig1_standard_probe.py \
  --corpus-dir /path/to/CAJSamples \
  --decoder /path/to/jbgtopbm \
  --sample-id issue-33/test1.caj --page 1 --json
```

With no probe options, it reports `NOT_RUN`; an incomplete explicit request
fails. `NO_MATCH_IN_TESTED_GRID` means none of the decoded outputs matched
the oracle's visible pixels. `VISIBLE_ONLY_IN_TESTED_GRID` means at least one
setting matched visible pixels but no setting matched the complete stride
hash in the same orientation. A match on an all-zero image is explicitly
`BLANK_MATCH_NON_DISCRIMINATING`; a visible-only blank result is
`VISIBLE_ONLY_BLANK_NON_DISCRIMINATING` and is not a full match. The
probe hashes each valid PBM both in its returned row order and with rows
reversed. In each order it compares visible pixels (unused low bits masked)
and the complete DIB stride against the manifest. Since PBM has no DIB row
padding, the stride comparison assumes zero padding while retaining the
PBM's actual unused low bits. `MATCH` requires both hashes to agree in the
same row order; `VISIBLE_MATCH_RAW_MISMATCH` means visible pixels agree but
the raw stride does not for an individual setting. The parser follows the
[Netpbm raw PBM format](https://netpbm.sourceforge.net/doc/pbm.html) for P4
magic, decimal dimensions, and whitespace or comments before the dimensions.
It also accepts a comment directly after the height digits. The first
whitespace after height is always the single raster delimiter; a `#` after
that byte belongs to the raster, so whitespace-then-comment after height is
rejected. The header is limited to 1,024 bytes, and exactly one image of the
expected raster length is required. If every setting fails to decode, the
report is `INCONCLUSIVE_NO_DECODABLE_SETTINGS` and exits nonzero. The
[experiment note](research/jbig1-bitstream-investigation.md) records the tested grid,
positive controls, refuted hypotheses, row-order evidence, and unresolved
CAJ-specific rules. Neither result claims full JBIG1 compatibility.

## Selected HN/C8 type-3 PDF pixels

The [#106 selected type-3 PDF diagnostic](research/hnc8-type3-pdf.md) converts one
checked HN/C8 JBIG2 image record into one bilevel PDF page with a
caller-supplied T.88 MQ table in the historical experiment. Current public
conversion uses built-in standard states; the table-injection diagnostic is
retained for comparison. The optional
[`jbig2_page_pdf_parity.py`](../scripts/jbig2_page_pdf_parity.py) runner
requires the pinned external CAJSamples corpus and private table, then checks
each selected PDF using `qpdf`, Poppler, and fixed MuPDF/Poppler render
canaries against the [#43 hash-only pixel oracle](research/jbig2-oracle.md). It keeps
strict-valid image matches separate from the single named opt-in `0xa40c`
case, and reports the expected strict refusal separately. A clean clone
reports `NOT_RUN` and zero PDF pixel compatibility matches. Source documents,
privately supplied diagnostic inputs, generated PDFs, and bitmaps remain
external. Standard numeric states were subsequently adopted in #189, as
recorded in [provenance](provenance.md). This check
does not establish multi-image HN/C8 page placement or independence of the
external oracle's decoder backends; [#107](https://github.com/rwv/caj2pdf-rust/issues/107)
tracks source-page layout measurement.

## HN/C8 source-page layout metadata

The [#107 layout oracle](research/hnc8-layout-oracle.md) is an opt-in, metadata-only
black-box comparison against a fixed Python reference revision. It checks
27 SHA-pinned HN/C8 sources, three deterministic reference PDFs, the original
75 pages/125 ordered image draws and a separate two-page HN-B omission case.
qpdf, MuPDF and Poppler independently check boxes, image order, transforms,
types and encoded-stream hashes. The committed oracle contains coordinates,
dimensions and hashes only; no private documents, PDFs, text or pixels. A
clean clone reports `NOT_RUN` and zero layout matches. That metadata-only
phase measured 50 extra-image placements without identifying their source
fields. Later [#112 empirical placement rules](research/hnc8-placement-rule.md) and
[#117 page composition](research/hnc8-page-composition.md) establish a bounded
caller-table diagnostic for the selected profiles. Their Python-reference
basis does not establish vendor page fidelity or full-family conversion.

## CAJViewer vendor fixtures

Follow the [simplified fixture plan](research/cajviewer-fixtures.md) and
[epic #123](https://github.com/rwv/caj2pdf-rust/issues/123). Prove one practical
capture recipe, save a small external image baseline, and collect ordinary-copy
text where available. Manual initial capture is acceptable. #128 comparison
can start with original fixtures; it depends only on completed #125.
Text unavailability is recorded and does not block the image route.

Selected complete-page checks have been run: CAJ/PDF/KDH selected pages
match; corrected HN-A/C8 frames match but exact pixels still differ. See
[the current fixture results](research/cajviewer-fixtures.md) for the measured pages
and repeatability limits. Unchecked pages and ordinary-copy text in the HN/C8
run remain NOT_RUN; this is not whole-document or whole-family parity.
The [V14 inventory](research/cajviewer-runtime-view-v14.md) and
[twelve earlier launch observations](research/cajviewer-linux-startup.md) are historical
startup evidence. #153 is cancelled as a standalone source-loading prerequisite.
[#219](https://github.com/rwv/caj2pdf-rust/issues/219) tracks improvement beyond
the completed selected-page scope of #123.

Keep vendor/corpus artifacts external, preserve full-page geometry and raw
text, and distinguish native capture from print-derived images and OCR.
Original fixtures run in ordinary CI. Requested missing inputs fail; optional
missing corpus is NOT_RUN. Publish actual coverage and limitations separately
from Python regression results. Preserve independent bookmark, licensing and
converter-memory release checks.

## Reference behavior

The [Python converter](https://github.com/rwv/caj2pdf) is a black-box
behavioral oracle at the revision named in the matrix, never an implementation
source. Reference `success`, `error`, `unsupported`, `skip`, and `not_run` are
distinct. A missing native decoder is an environment skip, not proof that a
format is unsupported. TEB conversion and pure-text HN are known reference
limitations. HN image output does not imply searchable text. Every release
report must state which optional corpus cases were actually run, the tool
versions, and the exact failures.

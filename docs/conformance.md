# Conformance baseline

Current selected-page vendor results are in [CAJViewer fixtures](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/cajviewer-fixtures.md).
CAJ/PDF/KDH selected pages match. After #184, HN-A/C8 page-frame sizes match,
but exact pixels still differ; these profiles remain experimental. Capture
repeatability is scoped to each report. Historical launch/fixture issues
#124–#129 are closed or consolidated into #123.

The [corpus matrix](../tests/conformance/matrix.json) inventories unique inputs
from a pinned revision of the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository. Its
[provenance note](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/README.md) explains canonical paths,
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
| C8 | Experimental image pages and admitted native profiles | Compressed four-page image output and the six/four/five-page native profiles have [runtime and layout checkpoints](#unreleased-native-c8-checkpoint); the unreleased [additional 10/5-page profiles](#additional-c8-sample-checkpoint-380-382) also complete. Native pages require fonts: explicit, or installed ones the CLI finds. Requested bookmarks are omitted with a warning (no outline is written from unverified metadata); #303 tracks the missing outline evidence. Font/raster differences remain explicit. |
| HN-B | Experimental image pages and admitted native mode-0/mode-2 profiles | With explicit fonts (bookmarks are omitted with a warning), the selected 4/4/6-page documents convert through CLI/Node/Worker with identical per-document outputs. Native mode 2 supports leading images; image-after-text and mode-0 images remain errors. [Independent controls and scoped layout checks](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnb-compact-index.md) do not establish original-font pixel parity. |
| TEB, unrecognized layouts, unsupported image/native modes | Rejected | No OCR or silent omission fallback. Located errors identify unsupported HN/C8 content. |

This table describes current main, which v0.4.0 released. Earlier published
artifacts do not gain these capabilities.
The font-free corpus checkpoint below is separate from successful explicit-font
HN-B/C8 runs. Required caller fonts must be provided; a missing resource is not
proof of an unsupported parser profile. Unknown HN-B/C8 outlines remain unknown,
not confirmed empty, and require explicit omission.

The [real-font HN-B checkpoint](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnb-real-font-fidelity.md) records three
complete CLI/Node/Worker conversions, fresh selected viewer captures and
remaining substitute-font appearance differences. It does not establish
source-font pixel parity.

The [real-font C8 checkpoint](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-real-font-fidelity.md) covers the admitted
6/4/5-page profiles across all interfaces. Some substitute-font English and
formula text visibly overlaps; font character coverage is not a guarantee
of source typography or readable spacing.

For usage, see the [CLI font flags](cli.md#native-c8-font-resources) and
[Node/Worker font sources](../js/README.md#explicit-fonts-for-native-c8-and-hn-b-pages).
[Unicode fidelity](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-text-fidelity.md) distinguishes character transport,
selected source checks and reading-order/whitespace limits. PDF syntax checks,
matching adapter hashes and substituted-font layout checks do not establish
original-font pixel parity or unrestricted copy/search fidelity.

Arithmetic HN/C8 images use built-in standard QM/MQ states. Optional custom
state overrides remain supported. The owner-directed adoption and upstream
practice are recorded in [provenance](provenance.md); #189 completed #30/#44.
Type-3 (JBIG2) bitmaps are held in memory, capped by the allocation limit;
since #355 no interface needs scratch storage.

[Viewer results](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/cajviewer-fixtures.md) record the pinned application and
selected-page scope. [Complete HN/C8 checks](js-validation.md#source-geometry-correction-repeat)
record historical output hashes, page counts, outline retention and image-stream checks.
The [streaming compression report](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/bilevel-compression.md) records current C8
hashes, fixed working memory, independent decoded-pixel equality and the explicit
scope of reused versus rerun evidence. Compression changes encoded bytes, not
page/outline mapping or decoded image bits.
Python-reference corpus expectations below are a separate compatibility
baseline, not a CAJViewer verdict. Missing optional inputs are `NOT_RUN`;
known pixel failures are not passing baselines.

The unreleased [type-1 JPEG extension](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-type1.md) adds the measured HN-A/C8
profile and selects the existing HN/C8 text-header interoperability policy in
CLI/WASM. The frozen v0.3.1 results below are not overwritten by this change.

### v0.4.0 pre-tag Linux CI baseline (#328)

The [current CLI observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/current_cli_baseline.json)
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
measurements. Outputs and raw logs remain outside Git. The POSIX runner, now
[`research/scripts/current_formats.py`](https://github.com/rwv/caj2pdf-samples/tree/main/research/scripts/current_formats.py) in
caj2pdf-samples, needs Python 3.11+, qpdf, MuPDF (`mutool`) and Poppler
(`pdfimages`); run it from a caj2pdf-rust checkout as that repository's
[research README](https://github.com/rwv/caj2pdf-samples/tree/main/research/README.md) describes:

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
Its synthetic checks moved with it to
[`research/conformance/`](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance/):

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
See [additional C8 controls](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-controls.md#complete-document-runtime-and-visual-checkpoint-c2df122).
Bookmarks and unverified native profiles remain unsupported; unknown required
content fails explicitly. See the current status and original-control
evidence in [C8 native records](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-records.md), and the resource contracts
in [CLI usage](cli.md#native-c8-font-resources) and [JavaScript usage](../js/README.md).
External documents, fonts and rendered evidence are not distributed. #242 owns
additional-profile final acceptance; #222 owns packaged release/corpus and
peak-memory acceptance.

### Additional C8 sample checkpoint (#380, #382)

The expanded #303 sample search found two independent C8 failures. With these
unreleased fixes, the unchanged restructured C8 and xue8 KVM originals convert
all 10/5 pages through CLI, Node and a real Chromium Worker. Each document has
identical output hashes across the three interfaces; both PDFs pass qpdf and
page-count checks. Per-page native/PDF inventories retain all 1,977/10,277
glyphs and 17/4 image draws. Browser OPFS cleanup succeeds. The
[pinned research note](https://github.com/rwv/caj2pdf-samples/blob/4f262c5e2ff44bd2f52131fe449668cb131e36da/research/notes/c8-additional-profiles.md)
records source hashes, original MIT controls, selected viewer/marker comparisons
and complete output receipts.

The new rules cover terminal encoded NULs, aligned image names with optional
padding, measured explicit sizes and punctuation, title size field 10 and
horizontal-decoration forms. Parsing retains bounded 28-byte reads and no
allocation proportional to name or document size. Native regressions cover
truncation, malformed padding and unknown geometry; Node and real browser
controls exercise both name forms and retain embedded JPEG bytes.

This is a two-document checkpoint, not a rerun of the optional full corpus
(`NOT_RUN`) or original-font pixel parity. Substitute fonts still differ in
weight, English spacing and decoration appearance; some Latin runs overlap.
The separately discovered 12-page HN-B article (#381) still needs verified
symbol/control semantics and image-after-text composition. Its private-use
character is unresolved. #303 still lacks a positive C8/HN-B stored-outline
sample; bookmark omission and its warning remain unchanged.

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
`sample_catalog.py` (now in [caj2pdf-samples](https://github.com/rwv/caj2pdf-samples/tree/main/research/scripts/sample_catalog.py)) the runner reports conversion, PDF, page-count and
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
read-modify-write pairs; no write buffer was added. #355 later removed
`FileScratch`: type-3 bitmaps are held in memory and the CLI creates no
scratch files.

### Known Python-reference differences

These are accepted v0.1 differences, with full source hashes and measurements in
[the CAJ format record](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/caj-format.md). Rust preserves source page-table order
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
  [compression report](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/bilevel-compression.md) measures the changed writer.
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

From a clean clone, the required checks build and run without external
documents:

```sh
cargo test --locked --workspace
python3 scripts/generate_fixtures.py --check
python3 -m unittest discover -s tests/fixtures -p 'test_*.py'
node --test js/test/*.test.mjs
```

The optional corpus checks that remain in this repository are the `#[ignore]`d
`*_external.rs` Rust tests and the JavaScript corpus runner. They read
`CAJ2PDF_CORPUS_DIR` (a local checkout of the pinned CAJSamples revision) and
the metadata kept in [`tests/conformance/`](../tests/conformance/README.md):

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  cargo test --locked -p caj2pdf-core --test hnc8_type2_jpeg_external -- --ignored
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples node js/scripts/corpus.mjs
```

An unset corpus is a visible `NOT_RUN` skip, never a compatibility pass; a
requested corpus with a missing or changed file fails.

The Python corpus runner (`conformance.py`, `current_formats.py`), the
JBIG1/JBIG2 and HN/C8 oracles, the layout and placement probes, the vendor
fixture comparison and the CAJViewer automation moved to
[caj2pdf-samples `research/`](https://github.com/rwv/caj2pdf-samples/tree/main/research/README.md)
in [#360](https://github.com/rwv/caj2pdf-rust/issues/360). Its README explains
how to run them against a caj2pdf CLI binary and a local corpus. The commands
and scope notes that used to follow here — the corpus runner, the JBIG1-like
image oracle, selected type-3 PDF pixels, HN/C8 source-page layout metadata
and CAJViewer vendor fixtures — are archived verbatim in the
[conformance command archive](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/conformance-commands-archive.md).
The results recorded above remain this repository's baseline.

## Reference behavior

The [Python converter](https://github.com/rwv/caj2pdf) is a black-box
behavioral oracle at the revision named in the matrix, never an implementation
source. Reference `success`, `error`, `unsupported`, `skip`, and `not_run` are
distinct. A missing native decoder is an environment skip, not proof that a
format is unsupported. TEB conversion and pure-text HN are known reference
limitations. HN image output does not imply searchable text. Every release
report must state which optional corpus cases were actually run, the tool
versions, and the exact failures.

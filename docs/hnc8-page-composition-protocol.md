# Issue #117: predeclared source-page composition validation

<!-- SPDX-License-Identifier: MIT -->

Status: FROZEN BEFORE EXECUTION, 2026-09-28 UTC. No private PDF extraction, rendering, source-byte inspection, table-byte inspection, or new converter call was performed while drafting and reviewing this plan. Public repository metadata and existing external JSON reports supplied the pins below. A separate execution receipt must bind the final original native example, Rust/Cargo source fingerprint, runner/tests, executable, compiler versions, effective environment and command to this committed plan before any private call.

## Scope and independence

Base: `09e77b9ff63487f7f5c7fe3512cc6dc90587aa6b` in `/tmp/caj2pdf-issue117-page-composition`.

Implement the native diagnostic and Python verifier independently under MIT. Inspect neither the Python/Go/private Rust nor other converter implementation. The native program receives only original source, output PDF, caller-supplied table and caller-owned scratch paths; it receives no oracle, reference PDF, source ID/hash dispatch, reference matrix or image/pixel array. The verifier separately reads the pinned public metadata and external PDFs. Reuse the six already-produced reference PDFs; **maximum new Python converter launches: zero**. No private source mutation, blind retry or tolerance expansion is authorized by this plan.

This evidence covers these already observed documents and profiles. It does not establish unseen formats, vendor-defined coordinate units, text/bookmark parity, an authorized bundled T.82 table, types 1/3, production CLI/JavaScript dispatch, or HN-B text conversion. The native diagnostic and caller-table API remain subject to #10, #30 and the release-policy gates.

## Pinned public and external inputs

Public files, relative to the repository:

| Input | Bytes | SHA-256 |
| --- | ---: | --- |
| `tests/conformance/matrix.json` | 237750 | `af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9` |
| `tests/conformance/hnc8_layout_oracle.json` | 99829 | `4b88befeecf9a68dd6eca4966c79ea8cdb130c43e3c6d92f4cb56fb34dfb665e` |
| `tests/conformance/jbig1_oracle.json` | 546731 | `e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a` |
| `scripts/hnc8_layout_pdf.py` | 33560 | `b8239236a87b8c1293745cb4eab7311320ca805f5eb98327d88581da8219f67e` |
| `scripts/hnc8_layout_reference.py` | 45159 | `7c3725394d999d391e770fabb1a91949a59987bd29d2f53b5fb067ddb9ad17fd` |
| `scripts/hnc8_placement_rule.py` | 39393 | `419fe68d0547f2f6ff8bc55b26daaac7bec7843e6da51441c63afa92d402bf3b` |

External baseline report: `/tmp/caj2pdf-layout-reference-final-v7-report.json`, 40896 bytes, SHA-256 `8cc9fce6c8c0f1f97f7d39453d0734d053fd543aa023a04ef835bf187918d959`. It records six deterministic PDFs and 27 source before/after audits. Resolve the following paths from the report and verify their exact hashes again; do not infer them from matching filenames.

Corpus root: `/tmp/caj2pdf-ranged-corpus`. Audit all 27 HN/C8 matrix rows, not only the three selected sources, before and after.

| Profile | Source path relative to corpus | Bytes | Source SHA-256 | Source rows / output pages / images |
| --- | --- | ---: | --- | --- |
| HN-A | `issue-21/实时网络流量异常检测算法研究和系统实现_林尚朕.caj` | 5753314 | `33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4` | 68 / 68 / 91 (67 Type0,24 JPEG) |
| C8 | `issue-33/test1.caj` | 2625902 | `35951c3775790c230c84e4312e8db7a57ca2980a04d35ff03d328806df7d622c` | 7 / 7 / 34 (7 Type0,27 JPEG) |
| HN-B | `issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj` | 862235 | `e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49` | 6 / 2 / 2 JPEG |

Baseline session: `/tmp/caj2pdf-layout-reference-final/hnc8-layout-run-mi3kj_io`.

| Profile | Baseline runs relative to session | Each PDF bytes | Each PDF SHA-256 |
| --- | --- | ---: | --- |
| HN-A | `hn_a-run1-3rgsr25v/output.pdf`; `hn_a-run2-ylai6rm4/output.pdf` | 8239244 | `833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40` |
| C8 | `c8-run1-w1y5xlqd/output.pdf`; `c8-run2-oof0dwe3/output.pdf` | 2759609 | `acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885` |
| HN-B | `hn_b-run1-komdoko5/output.pdf`; `hn_b-run2-fgy9eun7/output.pdf` | 826645 | `b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51` |

Hash-audit all six PDFs before/after. Extract metadata, samples and pixels from run1 of each profile; run2 is byte-identical and remains an audit input, not extra independent evidence.

Caller runtime table: `/tmp/caj26-official-vector-with-checkpoints.txt`, currently 1745 bytes, expected complete-file SHA-256 `11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`, maximum16384 bytes. Read it only after plan freeze. The loader validates the existing explicit `T82-1993` checkpoint-vector syntax and constructs caller-supplied `QmTable`; retain no table values in reports, source fixtures, CI or artifacts intended for Git. Its use establishes no redistribution authorization.

Existing metadata-only placement report, available for history, not an additional oracle: `/tmp/issue112-placement-rule-report.json`, 115511 bytes, SHA-256 `6d892bc3d41f23f71d503c689411d2bc2f9f30ecd0fb8bcd61b1c95ec5a10048`. #112 observed 75 boxes,125 ordered CTMs,maximum residual4.91306109964e-5pt. It did not establish full-page pixel parity or exact original padding.

## Tool and runtime pins

The following executables are required; a missing/mismatching/failing tool fails the requested run.

| Tool path | Version | SHA-256 |
| --- | --- | --- |
| `/usr/bin/python3.13` | Python3.13.5 | `889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9` |
| `/usr/bin/git` | 2.47.3 | `356db14e102d68a1a37d8a1ac577dfd678d45d46e92f468bef8b7154e7bfdc60` |
| `/usr/bin/qpdf` | 12.2.0 | `30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792` |
| `/usr/bin/mutool` | 1.25.1 | `b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7` |
| `/usr/bin/pdfinfo` | 25.03.0 | `a1a371340d7b76e7d501da9136cc9256dfdb9cbdf09300520d7a3b4465343e67` |
| `/usr/bin/pdfimages` | 25.03.0 | `213eba4a36ef021f49a0abc94292a7566baba8dfafef5017467166d9f06074f5` |
| `/usr/bin/pdftoppm` | 25.03.0 | `f22d753dfb4c31c9f0198d608982dac5b06a3c5d9a08d7d9bf0c6004f08a1a56` |

The reference-report audit may verify converter/library identities by hash only using the existing protocol; no converter is launched and no implementation is inspected. Historical reference checkout `/tmp/caj2pdf-python-oracle` was clean at `8cbc3c5721acb762f739434eb3d206171dbb022a`; converter file SHA `c2bede4bd4e1308fb9f7ec7e5593106c5b41188c337158a61cfadadc6f81f739`; reference `/tmp/caj2pdf-jbig-oracle/libjbigdec.so` SHA `d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6`. Historical PyPDF2 dependency `/tmp/caj2pdf-oracle-pydeps`, version1.26.0, 16-file tree SHA `4d33afdda9bb9ed730fba0355c42291d3380ce93f89cf4ec489338dcd385b36c` is provenance only; new conversion does not use it.

Hash system tool binaries before/after and record effective `LC_ALL=C`, `TZ=UTC`, `PYTHONHASHSEED=0`, `PYTHONDONTWRITEBYTECODE=1`, `PYTHONNOUSERSITE=1`, inherited environment digest, executable realpaths, version command outputs, and Rust/Cargo versions. Set `TMPDIR` to the external session scratch directory. Bind Python/native executable and original harness/source fingerprints before/after. Runtime-linked dependencies are outside the executable digest; record the resolved shared-library identity set in the execution receipt and audit it before/after if supported by the existing audit helper, or state that limitation explicitly.

## Original native caller contract

Build cache: `/home/hzc/.cache/caj2pdf-issue117-build`; `CARGO_TARGET_DIR` is set explicitly. Build the original `hnc8_page_composition` example in locked release mode. The execution receipt freezes the resulting executable/hash and full Rust/Cargo source fingerprint.

One native invocation per profile; maximum3 launches and no retries:

`NATIVE SOURCE OUTPUT_PDF TABLE_FILE SCRATCH_DIRECTORY`

All paths must be canonical and constrained to the audited input roots or newly created external session. The table is optional at the library boundary; this diagnostic supplies the audited caller table for every profile, including HN-B, without source-specific dispatch. Output sink and scratch adapter are platform wrappers, separate from core conversion. Use the existing original temporary-file `RandomAccessScratch` adapter. Truncate/reset row store between images, verify checked offsets/short I/O, and remove it on success/error/cancellation. It is random-access disk, never a whole-page/whole-image memory vector.

Call `convert_source_pages_pdf(source,sink,Some(&table),scratch,visitor,options,limits,cancel)` with explicit frozen parameters:

- `Limits`: I/O4096 bytes; input8GiB; output128MiB; maximum single checked handler allocation1MiB; pages4096; bookmarks4096.
- Container `Budget`: outline records4096; images per page256; total images16384; text span1MiB; image span64MiB.
- `TextBudget`: encoded span1MiB; decoded bytes1MiB; records65536; images256; working bytes1MiB.
- `JpegBudget`: payload64MiB; markers100000; work64MiB.
- `Type0Budget`: width32768; height32768; pixels12000000; context work120000000.
- `ArithmeticBudget`: symbols12032768; work385049600 (`12032768*32+1024`).
- `ComposeBudget`: page metadata65536 bytes; row store2097152 bytes; requested row-store I/O per image268435456 bytes.
- No arbitrary CTM adjustment, clipping, source-coordinate normalization or rounding. Use the original source-derived profile through core composition. The public semantic CTM threshold is absolute0.00005pt in every component.

The native visitor streams metadata for every source row, including HN-B image-free rows. It does not retain document-wide pixel/placement arrays. Bound stdout to1MiB and each line to2048 bytes. Output variant, source/output page counts, explicit source-to-output mapping, source page row/text/image metadata, image-order records/types/payload spans, visible/display dimensions, page boxes, and all six CTM components. Use shortest round-trip finite decimals. Include measured source/sink/scratch requests and bytes, page-metadata/text/row-store peaks, Type0/JPEG counts, and no-image row count. Runtime tables/text/sample bytes never appear in stdout or reports.

## Stage1: metadata, identity, order and full padded Type0 arrays

Read the committed oracle and matrix under1MiB caps. Independently reopen all3 selected baseline PDFs and all3 native PDFs with the existing bounded qpdf/MuPDF/Poppler metadata extractor. Do not trust native visitor output alone.

Require81/81 source rows accounted,77/77 output pages,127/127 ordered draws (HN-A68/91,C8 7/34,HN-B6source rows/2pages/2draws). HN-A/C8 mappings are1..68 and1..7. HN-B image counts must remain `[1,0,0,0,0,1]`, output mapping `[1,6]`, no-image source rows2..5 counted explicitly; their unsupported text is not a converted text pass.

For every output page and draw, compare boxes, all6 ordered CTM components, source descriptor/payload spans, image record numbers/types, visible width, display width, height and bit depth. CTM/box tolerance0.00005pt. Expected Type0 display width is the independently observed `stride_width=ceil(visible_width/32)*32`, not a scaled visible-width image. Compare all53 JPEG raw streams exactly against their original oracle payload SHA-256/length, independent qpdf and MuPDF extraction hashes. Native object IDs/resource names may differ; compare their ordered draw association and actual stream content, not reference ID numbers. Reference Type0 Indexed white/black palette and native one-bit DeviceGray with explicit inverse decode can be different dictionaries only when independently verified to map each stored bit to the same black/white sample. Never require encoded-Flate-byte equality; require decoded sample equality.

For each of the74 Type0 images, inspect bounded image dictionaries and extract complete filtered sample arrays with `qpdf --show-object=ID --filtered-stream-data PDF`. Only the measured one-bit, no-predictor, known black/white dictionary profile is accepted; anything else fails/unsupported, never guessed. Require exact sample byte count `display_width/8*height`, all padded columns and all stored rows. Normalize only proven palette/polarity into black=1. Compute forward and reverse-row hashes for orientation diagnosis. Compare every byte/bit at the same stored-row coordinate with native output; **zero bit differences required**. No trimming, masking, invented zeros, normalized padding, best-fit row reversal or skipped rows. Record forward/reverse digests and whether equality is forward, reverse-only or neither; only forward after declared dictionary normalization passes.

Separately invoke `pdfimages -f PAGE -l PAGE -j PDF PREFIX` for the pages containing Type0 images, parse every extracted Type0 P4/P5/P6 raster under strict exact dimensions/lengths, and canonicalize each proven binary black/white pixel into packed black=1. Require every grayscale/RGB component to be exactly0 or255, require RGB channels equal, and pack **every display-width sample**, including padding. P4 width is a multiple32, so there are no unused low bits. Match its complete array against qpdf's canonical sample array and the candidate's complete array. Locate images by ordered page/image association, never an unverified filename index. JPEG outputs from `-j` are byte-hashed/checked or discarded under the same cap; no JPEG image silently omitted. If Poppler exposes a different valid wrapper profile, stop and report it; do not silently widen accepted format rules.

This stage explicitly covers row direction and exact padding, which the older selected-image harness's visible-bit masking/zero-padding reconstruction did not prove. All74 images must pass both full-array observations before full-page compatibility can pass.

## Stage2: every complete-page pixel, two renderers

Render all77 complete pages (all75 HN-A/C8 and both HN-B outputs), for baseline and candidate, once with **each** required renderer. Maximum308 render launches: `77 pages *2 PDFs *2 renderers`. Do not render selected interior samples, ignore margins, use neighborhood envelopes, remove edge rows, align images post hoc, downsample comparisons or fit a flip/shift. Compare each renderer only with its own baseline at identical fixed options; cross-renderer hashes need not agree.

Commands, with1-based page `P`:

- MuPDF: `/usr/bin/mutool draw -q -r 300 -A 0 -c rgb -F pnm -o - PDF P`.
- Poppler: `/usr/bin/pdftoppm -r 300 -singlefile -aa no -aaVector no -f P -l P PDF` (no output prefix; bounded P6 output on stdout).

Accept only strict8-bit binary P6 RGB with exact expected dimensions, no trailing data, and a capped header.300dpi is frozen because the measured0.24pt/pixel model yields one device pixel per displayed source pixel and includes every padding sample. RGB preserves colored supplemental JPEGs. Disabling vector/text antialiasing avoids an uncontrolled renderer-setting difference; image interpolation remains each renderer's own fixed behavior on both PDFs. The rounded public CTMs are close but not mathematically identical to native shortest-decimal CTMs. At300dpi0.00005pt is at most0.000208334device pixels; **that is not permission to accept pixel differences**.

Pixel gate: exact equality of every RGB channel at every page coordinate, **maximum channel difference0, absolute-difference sum0, changed pixels0**. Also require identical complete raster SHA-256/byte count and exact expected width/height. Report worst channel difference, integer total absolute difference, changed channel/pixel counts, mean absolute channel difference, and nonwhite pixel counts for both images. These are diagnostics; no nonzero tolerance, neighborhood rule or alternate renderer setting converts a failure to a pass. If numerical phase or a renderer produces a difference, retain the failed pair and metadata, report FAIL/PARTIAL, and obtain a separately reviewed plan for follow-up rather than loosening this gate.

Validate the comparator on original MIT synthetic rasters before private execution: one changed first/last/edge pixel, reversed rows, omitted colored overlap, shifted/fractional/off-page placement, malformed/truncated/extra PNM bytes, different dimensions, all-white candidate against nonwhite baseline, and a metadata-missing blank draw. Exact pixel equality on a legitimately blank reference page is meaningful only together with that page's positive expected draw count and exact ordered geometry/complete-sample/stream gates; blank/omitted output cannot independently count as compatibility.

Public boxes predict603701450 pixels across77pages. Largest page2592x3285=8514720 pixels,25544160 RGB bytes. Total RGB payload across all308 outputs is7244417400 bytes, processed sequentially and not retained together. HN-A totals536502275 pixels, C8 59603040, HN-B7596135. Compare all channels in bounded stripes; at most two65536-byte pixel input buffers plus bounded PNM-header/row packing scratch. Save only hashes/statistics in public reports, never pixels.

## Workload and resource ceilings

- Maximum native launches3; completed/passing/failed counts recorded separately. Converter launches0. Maximum render launches308. Maximum total validator/tool launches2048, including metadata, image dictionaries/streams, extraction, rendering and version probes. Counters increment **before each attempted launch**, including launch failure/timeout/cancellation.
- Deadline for whole requested experiment1800s; native timeout180s each; metadata/sample tools45s each; page render60s each. No retry.
- Child virtual-address limit1GiB. Native RSS ceiling128MiB and validator/renderer RSS ceiling512MiB, monitored where Linux `/proc` is available; successful or reaped failed-child `wait4` peak reported separately from20ms sampled peaks. A missing enforcement/measurement must be explicit, never presented as0. Memory accounting separates native checked handler allocations, named managed buffers/fixed codec reservations, harness working buffers, native/child RSS, table and disk. `Limits.max_allocation_bytes` is not total process RSS or total combined allocation.
- Hash, subprocess-drain and full-array/pixel read chunks at most65536 bytes; public metadata1MiB perfile; native stdout1MiB; diagnostics32768 bytes perchild; perline2048 bytes. No raw private stdout/stderr in committed JSON; store only hashes/byte counts/status/command, and retain raw diagnostics only externally if needed.
- Each PDF max128MiB; aggregate newly written PDFs max256MiB. Each raster/extracted sample file max64MiB; page-extraction directory max128MiB. qpdf sample arrays max64MiB but observed maximum1064340 bytes; native row store max2MiB. Type0 HN-A maximum986400bytes, C8 maximum1064340bytes.
- External root `/home/hzc/.cache/caj2pdf-issue117-validation`; create a private0700 fresh session. All native PDFs, scratch files, sample arrays, rasters and diagnostic logs use this session. `TMPDIR` points there. No heavy artifact/build output in `/tmp`; existing inputs are read in place and never mutated.
- Require at least1GiB available on the session filesystem before starting and before each expensive stage. Hard owned-session disk ceiling512MiB, monitored every20ms during child execution plus checked writes/explicit file-size caps. Do not run extraction and raster pairs simultaneously. Process onepage/one image pair at a time, delete successful samples and raster pairs immediately, reset/remove native row scratch before the next image. Retain at most the first failed pair and diagnostic metadata; abort its profile instead of accumulating further failures. Native PDFs and the metadata-only receipt/report may remain external; success leaves no raster/sample/row-spool artifact. State the ceiling covers runner-owned session files, not arbitrary tool internals outside the redirected temporary directory.
- Use bounded pipe consumers for qpdf sample/renderer PNM stdout; do not buffer whole arrays/renders in Python memory. Baseline/candidate file comparison is streamed. Enforce byte caps while consuming, not only after tools exit. Kill the complete child process group on failure/timeout/cancel/cap.

## Audits, report schema and failure semantics

Protocol: `hnc8-source-page-composition-v1`, schema1. No-input invocation returns `NOT_RUN` immediately with empty profiles/artifacts/errors, every attempted/completed/passing/failed/skipped comparison counter0, no source/table/PDF/tool reads and no child launches. Planned counts may be nonzero; actual work counts must be0. Partial arguments produceFAIL, notNOT_RUN.

Before private work, audit all27 original matrix sources; all6 baseline PDFs; matrix/layout/JBIG metadata and reference report; runtime table; required tool/Python binaries and recorded linked-runtime identities; native executable; Rust/Cargo and runner fingerprints; effective environment. Record input sizes and hashes and exact roots. Repeat these checks in `finally` after success or failure. Missing/mismatched before/after audit preventsPASS. Do not count unsupported or skipped optional cases as compatibility.

Report `planned`, `counts`, `source_audit`, `baseline_audit`, `table_audit`, `environment_audit`, `input_audit`, `native_audit`, `profiles`, `resources`, `attempts`, `errors`, `scope`, `production_composition=NOT_ENABLED`. Each profile includes native command/output digest, all source rows/mapping, expected/observed ordered metadata, each image's padded-sample or JPEG identity checks, and each full-page/per-renderer pixel digest/metrics. Actual counts separately cover81 source rows,77 pages,127 draws,74 Type0 arrays,53 JPEG streams,154 page/renderer comparisons and308 render launches. Include attempts that failed before output completion; a parsed error/output-count mismatch is not a skipped success. Each attempt records monotonically increasing launch number, kind/profile/page/image, exact argv, start/end/duration, timeout, exit code or signal, stdout/stderr sizes/hashes, peak RSS (or explicitly unavailable), and externally retained artifact paths/size/hash. No private text/table/pixel/raw stream in the report.

Store execution receipt and final report under the external session/cache. Final top-levelPASS requires every declared profile, row, draw, sample and full-page pixel gate; zero failing/skipped required work; zero new converters; all pre/post auditsPASS; resources withincaps; cleanup confirmed. A requested failure isFAIL with honest completed/remaining counts and preserved first failure; root may publish a metadata-onlyPARTIAL summary, never label remaining pages successful. The root freezes final report SHA, execution receipt, reviewed source commit and exact hosted-gate head separately.

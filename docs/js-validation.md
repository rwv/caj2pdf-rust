# JavaScript delivery validation

Issue #13 covers the common browser/Node package and CAJ/KDH/PDF conversion.
HN/C8 integration acceptance is tracked in #10. Representative vendor-page
comparisons and whole-process release measurements remain #123/#14.
After #184, selected HN/C8 page-frame sizes match CAJViewer, but exact pixels
still differ. See [the corrected results](cajviewer-hnc8-kdh.md#results-after-the-source-geometry-correction).
Earlier conversion hashes and geometry checks below predate that correction;
[the final section](#source-geometry-correction-repeat) records the corrected runs.

## Verified delivery paths

- Original CAJ/KDH/PDF fixtures convert through Node file/stream adapters and
  actual Chromium File/WritableStream adapters. Tests validate output with
  qpdf. Missing external corpus remains `NOT_RUN`.
- #169 tests the runnable Node example, including stdin, initialization and
  conversion failures, and preservation of existing output.
- #170 extracts the real npm tarball and checks public exports, default WASM
  loading, and actual Chromium conversion using only shipped files.
- #171 tests the browser example's OPFS output cleanup, cancellation, writer
  initialization failure, replacement, and download disposal. Native OS file
  picker dialogs are not automated. A successful fallback output stays
  available until discarded or replaced; closing a tab cannot guarantee
  asynchronous cleanup.
- `js/test/types/*.mts` compile the browser/Node conversion and spool calls
  under strict TypeScript checks. These are compile-only consumer checks.
  CI installs TypeScript 5.9.3, Node types 22.18.6 and undici types 6.21.0 in
  a temporary tools directory. The compiler is a validation tool, not a
  shipped dependency. The npm package remains dependency-free and MIT.

## Memory and temporary storage

Measured on 2026-09-29 with Node 24.13.0, Chromium 154.0.8037.57, and the
locked release WASM core used by main at `8eb3609`. Both targets returned
identical results for the original one-page PDF fixtures below. Each row was
run once with a directly ranged Blob and once after spooling a forward-only
stream. Each conversion used a fresh WASM instance and discarded output
chunks after counting them.

| Input bytes | Initial WASM bytes | Peak WASM bytes | Maximum read/write bytes | Direct temporary bytes | Spooled temporary bytes |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 106,945 | 1,179,648 | 1,638,400 | 106,945 | 0 | 106,945 |
| 25,186,757 | 1,179,648 | 1,966,080 | 262,144 | 0 | 25,186,757 |

All eight runs returned one page and the expected output byte count. Node
spool directories and browser OPFS files were absent after disposal. The
measurement script asserts these outcomes, the chunk bound and less than
4 MiB WASM growth for these fixtures; it runs in the existing WASM CI job.

These numbers measure WASM linear memory and logical spool file sizes, not
Node/Chromium RSS, JavaScript heap, browser Blob storage, filesystem physical
allocation, or image-decoder peak memory. Test input generation and HTTP
fixture serving happen outside the measured WASM memory. Full-process and
HN/C8 image workloads remain release checks; these results do not establish
constant memory for every document.

Reproduce from the repository root after a locked release WASM build:

```sh
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
node js/scripts/measure-memory.mjs
node --test js/test/*.test.mjs
```

The script requires Chromium and uses the existing browser harness. Set
`CAJ2PDF_CHROME` to select its executable. Use a temporary filesystem with
adequate free space (`TMPDIR` selects it). The first local attempt failed
while fetching the larger Blob with `/tmp` at 98% usage; those attempts are
not passes. The recorded runs used a dedicated cache directory on a volume
with over 400 GiB free. No change to conversion code was needed.

Type-check without adding dependencies to the shipped package:

```sh
typecheck_dir="$(mktemp -d)"
npm install --prefix "$typecheck_dir" --ignore-scripts --no-audit --no-fund typescript@5.9.3 @types/node@22.18.6 undici-types@6.21.0
node "$typecheck_dir/node_modules/typescript/bin/tsc" --strict --noEmit --module NodeNext --target ES2022 --lib ES2022,DOM,DOM.Iterable --typeRoots "$typecheck_dir/node_modules/@types" js/test/types/*.mts
rm -rf "$typecheck_dir"
```

## Random-access scratch adapters (#10)

Original tests exercise real Node files, immediate OPFS reads in a real Chromium
Dedicated Worker, resize/reset reuse, caps and precise offsets, short/invalid
host I/O counts, storage failures and cancellation during pending Node I/O.
The worker closes/reopens the handle and verifies caller-owned file cleanup.
No external corpus is needed for these storage contract checks; they do not
claim HN/C8 WASM conversion. The existing JS suite includes the new tests.
CI additionally compiles `js/test/types-worker/*.mts` with `ES2022,WebWorker`
using the same pinned TypeScript installation. The actual npm tarball includes
the shared scratch validation helper; packaging tests check the file list.


## Experimental HN/C8 WASM integration (#10)

The existing poll/resume engine now routes the complete source-page converter
through four caller-owned scratch stores. Original CI fixtures check asymmetric
pixels after PDF extraction, one-byte short I/O, Node files and real Chromium
Dedicated Worker OPFS storage. Negative tests cover missing/invalid caller
configuration, source/sink/store failure, cancellation, cleanup failure and
instance reuse. Rust also rejects image-less HN-B source rows explicitly.
DOM/Node and WebWorker TypeScript consumers compile the HN/C8 options.

### External four-page C8 check

Input: CAJSamples `issue-58/混凝土道面评价指标分析_谢永亮.caj`, with the same
external MQ table and source identity recorded in the [direct-text comparison](hnc8-direct-text.md).
Both calls used `includeBookmarks: false`, 4096-byte I/O, four 64 MiB-capped
scratch adapters and the release WASM build. Native file-scratch output is the
previous independently checked reference for byte identity.

| Result | Node 24.13.0 | Chromium Dedicated Worker |
| --- | --- | --- |
| Converted pages / bookmarks | 4 / 0 | 4 / 0 |
| Input bytes read | 733,682 | 733,682 |
| Output bytes | 3,992,137 | 3,992,137 |
| Final WASM linear capacity | 1,376,256 bytes | 1,376,256 bytes |
| Observed conversion duration | 14.68 s | 24.62 s |
| Scratch extents after conversion | Four zeros | Four zeros |
| File cleanup | Caller closed handles; empty files retained for inspection | All handles closed and all input/output/scratch files removed |

Both output SHA-256 values equal the native result:
`fffa38e8f2cd675352108488117f13983f959ead7500f9f4ba1cab9a2e74ef1e`.
Node output passes `qpdf --check`; browser output is byte-identical. The browser
spooled the fetched input to OPFS and streamed PDF output to an OPFS writer.
Hashing read the completed PDF only after conversion; this validation allocation
is not part of the converter memory measurement. Node whole-process sampled
RSS reached 87,744,512 bytes (100 ms sampling), not a core-allocation or precise
kernel peak measurement. Durations are single observations, not benchmarks.

This is one C8 sample, not all-format compatibility. That conversion check
did not cover C8/HN-B bookmarks or metadata inspection. CLI integration is
validated in docs/cli.md; metadata inspection is covered below. Codec state
distribution and the adapter anomalous-header opt-in remain unresolved. The general JS corpus runner has no
caller-table configuration and explicitly reports its HN/C8 rows as NOT_RUN;
it must not count these as rejected-format compatibility passes.


## HN/C8 metadata inspection

`inspect` uses the existing HN/C8 reader and validates HN-A outline records one
at a time, discarding each title immediately in WASM. It requires no codec
states, image decoding or temporary stores. CLI inspection retains a bounded
outline for its existing text/JSON rendering. C8/HN-B return unknown bookmark
metadata; validated empty HN-A outlines return zero/empty. Malformed records
fail with their original source location and `HNC8` JS error code.

Original controls cover one-byte short reads, nested outlines, count/depth/
allocation/page limits, invalid destinations/records in the core suite,
cancellation and direct metadata-read bounds. CLI tests check the same outline
in the resulting PDF with independent tools. Real Chromium controls distinguish
HN-A's known count from C8/HN-B's unknown count. Rust exhaustive error matches
must now handle `Error::Hnc8Metadata`; no new raw WASM exports or JS result fields
are needed.

CLI and Node inspection agree on these external documents (metadata comparison,
not a new independent format oracle):

| Source SHA-256 | Variant | Declared pages | Validated bookmarks | Node bytes read |
| --- | --- | ---: | ---: | ---: |
| `8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85` | C8 | 4 | unknown | 13 |
| `46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa` | HN-A | 163 | 96 | 29,589 |
| `951b60efc58186018bbc5a8ec25c54c838405535ea9a6aae5c0588099f1c3574` | HN-A | 65 | 54 | 16,653 |

Each Node run used a fresh release WASM instance, 4096-byte I/O and file ranges;
final linear memory capacity was 1,310,720 bytes in all three runs. These counts
validate header/outline metadata and do not establish that every source page
can be converted. Documents and private result files remain external. Real
browser metadata checks use original fixtures, not these external documents.

## Complete multi-image HN-A public-interface check

The 2026-09-29 run uses the 24,519,256-byte, 163-page source identified in
[repeated HN image groups](hnc8-repeated-groups.md), SHA-256
`46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa`.
It has 210 image draws (161 type-3 and 49 JPEG), repeated descriptor groups,
and 96 HN-A bookmarks. Runtime conversion code is main revision `f092bdf`;
the acceptance PR changes tests and documentation only. The MQ states are
supplied externally, with no normative data added to Git or packages.

| Check | Native CLI | Node 24.13.0 | Chromium Worker |
| --- | ---: | ---: | ---: |
| Pages / bookmarks | 163 / 96 | 163 / 96 | 163 / 96 |
| Output bytes | 163,571,546 | 163,571,546 | 163,571,546 |
| Observed duration | 110.18 s | 625.96 s | 1133.76 s |
| Final WASM linear capacity | N/A | 1,507,328 bytes | 1,507,328 bytes |
| Final scratch sizes | Anonymous files removed | Four zero-byte files | Four zeros; all OPFS files removed |

All three output SHA-256 values:
`1cc9c1cef08ea7077af47c78429e186af9f46d55b2f6174c2ec544bc4533d7e6`.
The CLI PDF passes qpdf 12.2.0. All 96 PDF outline titles, hierarchy levels and
destinations match the raw source's 308-byte bookmark records. All 163 page
objects, content streams and image dictionaries/streams retain their order and
bytes from the previously validated core output (before bookmarks were added).
The unchanged 163,533,754-byte prefix contains all those page and stream objects;
their ranges were checked against the PDF cross-reference offsets.

MuPDF 1.25.1 rendered pages 1, 23, 69, 78, 144, 150 and 163 at 300 ppi,
grayscale, `-A 0`. Every pixel matches the recorded gray-corrected Python
reference described in the linked report. This is selected-page rendering
parity, not all-page pixel parity or a CAJViewer result. Identical public output
hashes allow these document checks to be shared across interfaces.

Native peak child RSS was 14,708 KiB (`resource.getrusage(RUSAGE_CHILDREN)`;
may include the Python launcher's pre-exec process footprint). Node whole-process
RSS sampled every 100 ms reached 104,964,096 bytes before output hashing.
These observations are not isolated core allocation measurements or benchmarks:
the conversions and some tests ran concurrently. WASM linear capacity excludes
JavaScript heap, browser process memory and filesystem caches. Validation reads
the completed PDF for hashing only after conversion; that allocation is outside
the reported conversion memory observation.

Reproduction uses the existing CLI with `--mq-states FILE` and bookmarks
left enabled. Node uses `fileHandleSource`, `nodeWritableSink`, and four
`fileHandleScratch` stores. Both JS targets cap each scratch store at 64 MiB;
JavaScript I/O chunks are
65,536 bytes. Source, state files, generated PDFs and private run reports stay
outside the repository. The CLI temporary directory was empty afterward;
Node closed all handles and retained its empty scratch files for inspection.
The browser spooled the source to OPFS with a 64 MiB input cap and streamed
output to an OPFS writer. It closed the handles and removed input, output and
all four scratch files; final OPFS enumeration was empty.
The existing core run records a 1,089,966-byte aggregate scratch peak; this
public-interface run records caps and cleanup, not a new peak-storage sample.

Native binary SHA-256:
`c7a8f5e21f77b064cc470e956beaed0751bbf88f56b89301db28e1d2283b2c9f`.
WASM binary SHA-256:
`961c1cf9530f751ee3e679546941c5c26aa5d86cdae99c2be8f766b44e1035d6`.

Ordinary CI uses the existing original HN fixture extended to two positioned
images and nested bookmarks. Node file-scratch and Chromium Worker OPFS tests
extract both asymmetric pixel streams with qpdf, check both draw transforms and
order, verify the outline tree/destinations, and check cleared scratch stores.
The fixture uses invented constant arithmetic states and no external document.
Missing external corpus still means NOT_RUN. C8/HN-B unknown outlines require
explicit bookmark omission; image-less HN-B rows and anomalous type-3 headers
remain explicit errors. OCR and searchable text remain outside v0.1.

## Source-geometry correction repeat

After #184 (`bdb89b0`), the complete HN-A and C8 documents above were converted
again through native CLI, Node 24.13.0 and Chromium 154.0.8037.57 Worker/OPFS.
The source documents, caller-owned MQ states, 65,536-byte chunks and four
64 MiB scratch limits are unchanged. Browser input spooling remains capped at
64 MiB; output is sequential. This is a repeat of the existing integration
checks after the geometry/padding correction, with no new harness.

| Result | HN-A | C8 |
| --- | ---: | ---: |
| Pages / bookmarks, all three interfaces | 163 / 96 | 4 / 0 (explicit omission) |
| Output bytes, all three interfaces | 162,515,239 | 3,978,812 |
| Node duration | 568.20 s | 13.89 s |
| Browser duration | 904.15 s | 21.48 s |
| Final WASM linear capacity, Node and browser | 1,507,328 bytes | 1,376,256 bytes |
| Node sampled whole-process RSS | 121,675,776 bytes | 76,394,496 bytes |
| Final scratch sizes | Four zeros | Four zeros |

HN-A native duration was 98.26 s and peak child RSS 14,700 KiB, with an empty
temporary directory afterward. The same pre-exec RSS and concurrent-run
measurement caveats from the earlier run apply; these are not isolated core
allocation measurements or performance benchmarks. Node closed its handles
and retained empty files; each browser run removed input, output and all scratch
files. These repeats measure configured storage caps and cleanup, not a new
peak-storage sample. #184 changes geometry and output padding, not the decoder
scratch algorithms whose earlier peak measurements are recorded above.

All three HN-A output hashes:
`f903d8a871fcbead87ab76a65e19685f9385573e75d1e9b5a6babf5175320356`.
All three C8 output hashes:
`a28f46d2534935999b30048cfe49c7fc606fa4e4851cfe1814b5f1860bc0f726`.
The [viewer report](cajviewer-hnc8-kdh.md#results-after-the-source-geometry-correction)
records remaining exact-pixel failures and independent content/bookmark checks.
These hashes supersede the old geometry outputs, not their historical evidence.

Native binary SHA-256:
`7e886dd497314a370987b464f6d9ddfbc3b382424bacc368d2fcf3a3c6981bb1`.
WASM binary SHA-256:
`6de6986d730d3ec55d6462bd43436ddc6c812ae61bdeb2b11e816c391a12a05c`.
External run records are in `caj2pdf-source-geometry/public-results.json`.
An earlier HN-A Node attempt exited with SIGTERM (143), without a completion
report. Its partial PDF/stores are retained under `node-interrupted`; it is
INTERRUPTED, not a compatibility pass. The successful fresh Node run above
was started only after termination was confirmed.

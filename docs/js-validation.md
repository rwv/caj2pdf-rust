# JavaScript delivery validation

Issue #13 covers the common browser/Node package and CAJ/KDH/PDF conversion.
HN/C8 integration remains #10; representative vendor-page comparisons and
whole-process release measurements remain #129/#14.

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

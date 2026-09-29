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

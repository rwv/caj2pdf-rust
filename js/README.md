# caj2pdf JavaScript package

Streaming PDF, CAJ, and KDH to PDF conversion for browsers and Node.js 22+.
The package drives the same Rust core as the native crate through a raw,
dependency-free WebAssembly ABI. It has no npm dependencies, and all
project-owned JavaScript and TypeScript declarations are MIT-licensed.

| Input | Conversion |
| --- | --- |
| PDF (`%PDF-`) | Validated, repaired where the core supports it, and copied. |
| CAJ (`CAJ`) | Reconstructed PDF with CAJ outline bookmarks. |
| KDH (`KDH`) | Decoded PDF, then the PDF path. |
| HN, C8, TEB | Recognized; rejected with `UnsupportedFormatError`. Image decoding is not implemented yet (issues #9 and #23). |
| Anything else | Rejected with `UnsupportedFormatError` (`format: null`). |

## Build and test

```sh
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
node --test js/test/*.test.mjs
```

`npm run build:wasm` (run inside `js/`) builds the module and copies it next
to the entry points as `caj2pdf_wasm.wasm` (`scripts/copy-wasm.mjs`, mode
`0644`), which is where `loadModule()` looks by default. That copy is
gitignored and is never committed; the `files` list puts it in the tarball,
and `prepack` refuses to pack without a WebAssembly module there. The
package is marked `private` until the release process
([release policy](../docs/release-policy.md)) runs `npm run build:wasm`,
removes `private`, and publishes.

## Usage

```js
// Node.js
import { open } from "node:fs/promises";
import { convert, fileHandleSource, loadModule, nodeWritableSink } from "caj2pdf-rust";

const module = await loadModule();
const input = await open("paper.caj", "r");
const output = (await open("paper.pdf", "wx")).createWriteStream();
try {
  const report = await convert(module, await fileHandleSource(input), nodeWritableSink(output));
  output.end();
  console.log(report.format, report.pagesConverted);
} finally {
  await input.close();
}
```

```js
// Browser
import { blobSource, convert, loadModule, webWritableSink } from "caj2pdf-rust";

const module = await loadModule();
const writer = (await (await showSaveFilePicker()).createWritable()).getWriter();
await convert(module, blobSource(file), webWritableSink(writer), { signal });
await writer.close();
```

The `.` export resolves to `node.mjs` under the `node` condition and to
`browser.mjs` elsewhere; `./node` and `./browser` select one explicitly. Both
re-export the platform-neutral API in `io.mjs`. Type declarations are in
`*.d.mts`. Runnable examples are [`examples/node.mjs`](examples/node.mjs)
(`node js/examples/node.mjs INPUT|- OUTPUT.pdf`) and
[`examples/browser.html`](examples/browser.html) (serve the repository root
over HTTP and open `/js/examples/browser.html`).

### API

- `convert(wasm, source, sink, options)` detects the format from at most five
  leading bytes (or uses `options.format`) and resolves with
  `{ format, inputBytesRead, outputBytesWritten, pagesConverted, bookmarksWritten }`.
- `inspect(wasm, source, options)` resolves with
  `{ format, pageCount, bookmarkCount, inputBytesRead }` without output.
  `bookmarkCount` is counted for CAJ and `null` for PDF and KDH.
- `wasm` is a `WebAssembly.Module` (each call instantiates its own instance,
  so calls may run concurrently), an `Instance`, or its exports. An instance
  runs one operation at a time and rejects a second concurrent one.
- Options: `format` (`"auto"` by default), `limits`, `chunkSize` (bytes per
  request, default 256 KiB, at most 1 MiB), `signal`, and
  `includeBookmarks` (default `true`).
- `limits`: `maxInputBytes` (8 GiB), `maxOutputBytes` (16 GiB),
  `maxAllocationBytes` (64 MiB; at most 256 MiB and at least `chunkSize`),
  `maxPages` and `maxBookmarks` (100,000). JavaScript validates them and the
  Rust engine enforces them.
- Errors are `Caj2PdfError` with a stable `code` (for example
  `MALFORMED_CAJ`, `PDF_LIMIT_EXCEEDED`, `TRUNCATED_INPUT`) and the core's
  located message. `UnsupportedFormatError` adds `format`. Source and sink
  errors and abort reasons propagate unchanged.
- `copyRange(wasm, source, sink, options)` is a bounded copy diagnostic for
  custom sources and sinks; it is not conversion.

## Sources and sinks

```ts
interface RangedSource {
  size: bigint; // stable snapshot, unsigned 64-bit
  readAt(offset: bigint, length: number, signal?: AbortSignal): Promise<Uint8Array>;
}
interface SequentialSink {
  writeChunk(bytes: Uint8Array, signal?: AbortSignal): Promise<number>; // bytes accepted
  flush(signal?: AbortSignal): Promise<void>;
}
```

| Adapter | Entry | Behavior |
| --- | --- | --- |
| `blobSource(blob)` | both | Awaits `blob.slice(start, end).arrayBuffer()` for one range only; never reads the whole Blob. Sizes above `Number.MAX_SAFE_INTEGER` are rejected. |
| `fileHandleSource(handle)` | Node | BigInt positioned reads on a caller-owned `FileHandle`; never closes it or moves its cursor. |
| `webWritableSink(writer)` | both | Awaits each `writer.write()` of a copied chunk; `flush` awaits `writer.ready`. Never closes the writer. |
| `nodeWritableSink(writable)` | Node | Awaits each write callback, so at most one chunk is queued. Never ends the stream. |
| `convertReadable(wasm, stream, sink, options)` / `spoolToTempFile` | Node | Spools a Node `Readable`, Web `ReadableStream`, or async iterable to a private file (mode `0600`) in a fresh `mkdtemp` directory under `os.tmpdir()` (or `options.tempDirectory`). |
| `convertReadableStream(wasm, stream, sink, options)` / `spoolToOpfs` | Browser | Spools a `ReadableStream` to a uniquely named Origin Private File System file, then reads it as a disk-backed `File`. |

A plain stream has no size or random access, so it is never converted
directly and never buffered whole in memory. The spool accepts at most
`maxSpoolBytes` (default `limits.maxInputBytes`, 8 GiB) and rejects with
`LIMIT_EXCEEDED` as soon as more arrives. `convertReadable` and
`convertReadableStream` remove the spool after success, failure, sink error,
or abort; the lower-level spool functions return `dispose()` for the caller.

### Browser storage support

The browser spool needs `navigator.storage.getDirectory()` and
`FileSystemFileHandle.createWritable()` in a secure context (HTTPS or
localhost). Chromium-based browsers and Firefox provide both on the main
thread; Safari's support for `createWritable()` depends on its version. The
automated tests confirm the spool, its bound, and its cleanup against the
real OPFS of headless Chromium on the main thread; Firefox, Safari, and
workers are not tested automatically.
When either API is missing, the spool
rejects with `RANDOM_ACCESS_REQUIRED` instead of falling back to memory; pass
a `Blob`/`File` (which browsers keep disk-backed) or a custom `readAt`
source. OPFS writes count against the origin's storage quota.

## Bounded memory and I/O

Rust requests one range or one write at a time. JavaScript awaits the source,
copies only that chunk into the fixed WASM staging buffer, and resumes Rust;
for output it passes a view of staging memory and awaits the sink before
resuming. A view is valid only until `writeChunk()` settles, so a custom sink
that retains bytes must copy them; the supplied sinks do. Requests never
exceed `chunkSize`. No API accepts or returns a whole-document byte array.
Format indexes (page tables, bookmarks, object offsets) are held in WASM
memory under `limits`.

Measured on Node 22.22.2 with the release build (the
`a larger PDF converts...` test prints these numbers): converting a
25,186,757-byte one-page PDF with the default 256 KiB chunk grew WASM memory
from 1,179,648 bytes after instantiation to a peak of 1,966,080 bytes
(+768 KiB, independent of the document length). The largest read request and
the largest write were each 262,144 bytes, over 97 writes. WASM memory never
shrinks, so the final `memory.buffer.byteLength` is the peak.

## Cancellation

`AbortSignal` is checked before every poll and after every awaited read,
write, and flush. The supplied adapters also race their pending I/O against
the signal, so a stalled Blob read, file read, or backpressured write stops
waiting as soon as the signal aborts. The abandoned operation may still
finish in the background; it only touches its own copied buffer. The
conversion rejects with `signal.reason` (an `AbortError` by default), the
Rust future is cancelled, and the instance is reset for reuse. Bytes a sink
already accepted are not withdrawn: callers own their output and should
delete or discard it after a rejection (the examples do). A custom source or
sink should honor its `signal` argument for prompt cancellation.

## Tests

`js/test/*.test.mjs` run with `node --test` against the real WASM build:

- `convert.test.mjs` converts synthetic CAJ, KDH, and PDF inputs with Blob
  sources plus Web sinks and FileHandle sources plus Node sinks, asserting
  every request is within the chunk size and validating each output with
  `qpdf --check` and `qpdf --show-npages` when `qpdf` is installed (otherwise
  a diagnostic reports the skip; the CI WASM job installs it). It
  also covers `inspect`, HN/C8/TEB rejection, typed errors, limits,
  cancellation, sink and source errors, and the memory measurement above.
- `spool.test.mjs` covers Node temp-file spooling from Node and Web streams,
  the spool bound, cleanup after success, failure, and abort, and the OPFS
  spool against an in-memory OPFS test double.
- `browser.test.mjs` runs `browser.mjs` in headless Chromium (below).
- `package.test.mjs` dry-runs `npm pack` on a temporary copy of the package
  with the WASM build and asserts the tarball holds exactly the entry points,
  declarations, `caj2pdf_wasm.wasm`, `package.json`, `LICENSE`, and
  `README.md`; that `loadModule()` finds the packaged module by default; and
  that packing without a valid WASM build fails.
- `adapters.test.mjs` and `wasm.test.mjs` cover the adapters and the raw ABI.
- `corpus.test.mjs` runs the optional corpus runner (below) against a
  synthetic corpus and matrix built at test time.

The other tests run the browser adapters on Node's `Blob`, `ReadableStream`,
and `WritableStream`. The inputs are synthetic MIT fixtures from
`tests/fixtures` or built at test time; no external corpus test runs here.

### Optional external corpus

[`scripts/corpus.mjs`](scripts/corpus.mjs) runs the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) documents inventoried in
[`tests/conformance/matrix.json`](../tests/conformance/matrix.json) through
this package. It never fetches the corpus, and no corpus file is committed:

```sh
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples node js/scripts/corpus.mjs \
  [--matrix PATH] [--wasm PATH] [--qpdf PATH]
```

The repository records no per-sample Rust outcome, so each entry's
expectation comes from the API's format contract and the matrix's
`expected_outcome`, classified as
[`scripts/conformance.py`](../scripts/conformance.py) does:

| Entry | `expectation` | Requirement | Outcome when met |
| --- | --- | --- | --- |
| HN, C8, or TEB (any reference outcome) | `unsupported` | `UnsupportedFormatError` for that format | `unsupported` |
| CAJ, KDH, or PDF; reference `success` | `convert` | Output validated with the reference output page count | `passed` |
| CAJ, KDH, or PDF; reference `error` or `unsupported` | `excluded` | None recorded; conversion runs and is reported as `observed` | `excluded` |
| CAJ, KDH, or PDF; reference `unknown` | `not_run` | None recorded; conversion runs and is reported as `observed` | `not_run` |

For each matrix entry, in order, it:

1. Checks every path component with `lstat` and refuses a symbolic link as
   the file or any parent directory (matrix paths have no `..` components),
   opens the file without following links, and requires the inode it
   checked. It then streams the file once through SHA-256 and the Git blob
   SHA-1 with one 1 MiB buffer and compares both and the size with the matrix.
2. Converts the same handle with `fileHandleSource` into `nodeWritableSink`
   over a private file (mode `0600`) in a fresh `mkdtemp` directory, which is
   removed after success, failure, timeout, or interruption. Each conversion
   has a 10-minute `AbortSignal.timeout` and a 4 GiB output limit.
3. When a conversion succeeds, requires the detected format to match,
   `qpdf --check` to exit 0 with no `WARNING:` line, and
   `qpdf --show-npages` to equal `pagesConverted`. A `convert` entry also
   requires the matrix page count (`expected_pdf.page_count`, else
   `page_count`). Each qpdf call has the same timeout and 1 MiB of captured
   output.

A failed requirement is `failed`. For `excluded` and `not_run` entries, a
typed input rejection (`Caj2PdfError` other than `CANCELLED`, `IO`,
`LIMIT_EXCEEDED`, or `UNKNOWN`) or a validated output is recorded in
`observed`; a timeout, I/O error, WASM trap, or invalid output still fails.
A qpdf warning fails, as in `check_qpdf_log` in
[`scripts/jbig2_oracle.py`](../scripts/jbig2_oracle.py) and the Rust KDH
corpus test. After all conversions it re-verifies every source by path.

Progress goes to stderr and a JSON report to stdout:
`{ status, reason, sample_count, qpdf, checked, passed, failed, unsupported, excluded, not_run, failures, results }`.
Each `results` row has `id`, `format`, `reference` (`expected_outcome`),
`expectation`, `outcome`, `stage`, `reason`, and `observed`.

| Situation | `status` | Exit |
| --- | --- | --- |
| `CAJ2PDF_CORPUS_DIR` unset or empty | `NOT_RUN`, all counts zero | 0 |
| Corpus missing, or any `failed` entry | `FAIL` | 1 |
| No failure, but a `not_run` entry (including outputs not validated because `qpdf` is missing) or no `passed` entry | `NOT_RUN` | 0 |
| No failure or `not_run` entry, at least one `passed` | `PASS` | 0 |
| Invalid matrix, arguments, or WASM module | setup error | 2 |
| SIGINT or SIGTERM | interrupted after cleanup; no report | 130 |

`unsupported` and `excluded` are never passes and, as in `conformance.py`,
do not block `PASS`. Page counts and `qpdf --check` do not compare page
order, rendering, or outlines with the reference PDFs, so the known
reference differences (`issue-40`/`issue-44` page order and
`issue-49`/`issue-73` outlines; see [CAJ format notes](../docs/caj-format.md))
are outside this check. A timeout aborts at the next I/O call; a WASM loop
that never returns to I/O is not interrupted. The CI WASM job runs the
script with an empty `CAJ2PDF_CORPUS_DIR` and asserts `NOT_RUN` with zero
counts.

### Real-browser tests

`browser.test.mjs` needs no npm packages. It serves the `js/` directory
read-only (GET and HEAD, no paths outside it, including through symbolic
links) on an ephemeral `http://127.0.0.1` port with `node:http` (a secure
context), starts
headless Chromium with a throwaway profile and `--remote-debugging-port=0`,
and drives it over the Chrome DevTools Protocol with Node's global
`WebSocket` ([`browser-harness.mjs`](test/browser-harness.mjs)). The page
imports `browser.mjs`, loads the freshly built WASM from its default URL,
and runs
[`browser-cases.mjs`](test/browser-cases.mjs):

- `File` sources to real `WritableStream` sinks for synthetic CAJ, KDH, and
  PDF inputs, with every read and write at most the 4 KiB chunk size. The
  outputs return to Node (base64 plus a SHA-256 computed with
  `crypto.subtle`) for `qpdf --check` and page counts.
- HN and C8 rejection with `UnsupportedFormatError`.
- `AbortSignal` cancellation while a `WritableStream` write is stalled.
- `convertReadableStream` through the real OPFS: one `caj2pdf-spool-*` file
  exists during conversion and none after success, the `maxSpoolBytes`
  bound, an unsupported input, or an abort.

Chromium is found through `CAJ2PDF_CHROME`, then `/usr/bin/google-chrome`,
`/usr/bin/chromium`, and Playwright's `/opt/pw-browsers` directory. Without
one the tests are skipped locally and fail when `CI` is set:

```sh
CAJ2PDF_CHROME=/path/to/chrome node --test js/test/browser.test.mjs
```

Chromium is started with `--no-sandbox`, `--no-proxy-server`, background
networking disabled, and host resolution restricted to `127.0.0.1`. Every
DevTools command has a 30-second timeout. After the run, or when the test
process exits early or receives `SIGINT` or `SIGTERM`, the Chromium process
group is killed and its throwaway profile removed. The CI WASM job runs
these tests with the runner's preinstalled Google Chrome on Node 22 and 24.
Firefox, Safari, and Web Workers are not covered, and the manual
[`examples/browser.html`](examples/browser.html) (file picker and save
dialog) is not automated.

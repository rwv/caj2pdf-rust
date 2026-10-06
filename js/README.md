# caj2pdf JavaScript package

Streaming CAJ-family to PDF conversion for browsers and Node.js 22+.
The package drives the same Rust core as the native crate through a raw,
dependency-free WebAssembly ABI. It has no npm dependencies, and all
project-owned JavaScript and TypeScript declarations are MIT-licensed.

| Input | Conversion |
| --- | --- |
| PDF (`%PDF-`) | Validated, repaired where the core supports it, and copied. |
| CAJ (`CAJ`) | Reconstructed PDF with CAJ outline bookmarks. |
| KDH (`KDH`) | Decoded PDF, then the PDF path. |
| HN, C8 | Experimental complete-page conversion with built-in standard codec tables and caller-owned scratch stores (below). `inspect` reads page counts and validated HN-A bookmark counts without codec tables. |
| TEB | Recognized; rejected with `UnsupportedFormatError`. |
| Anything else | Rejected with `UnsupportedFormatError` (`format: null`). |

## Build and test

```sh
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
node js/scripts/copy-wasm.mjs
node --test js/test/*.test.mjs
```

`npm run build:wasm` (run inside `js/`) builds the module and copies it next
to the entry points as `caj2pdf_wasm.wasm` (`scripts/copy-wasm.mjs`, mode
`0644`), which is where `loadModule()` looks by default. That copy is
gitignored and is never committed. Examples use this same packaged module path,
including when Cargo builds into a custom target directory. The `files` list puts it in the tarball,
and `prepack` refuses to pack without a WebAssembly module there. The
package is marked `private` until the release process
([release policy](../docs/release-policy.md)) runs `npm run build:wasm`,
removes `private`, and publishes.

## Usage

```js
// Node.js
import { open } from "node:fs/promises";
import { finished } from "node:stream/promises";
import { convert, fileHandleSource, loadModule, nodeWritableSink } from "caj2pdf-rust";

const module = await loadModule();
const input = await open("paper.caj", "r");
let output;
try {
  output = (await open("paper.pdf", "wx")).createWriteStream();
  const report = await convert(module, await fileHandleSource(input), nodeWritableSink(output));
  output.end();
  await finished(output);
  console.log(report.format, report.pagesConverted);
} finally {
  output?.destroy();
  if (output) await finished(output).catch(() => {});
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
(`node js/examples/node.mjs INPUT|- OUTPUT.pdf [--no-bookmarks]`) and
[`examples/browser.html`](examples/browser.html) (serve the repository root
over HTTP and open `/js/examples/browser.html`). The Node example also
removes its newly created output on failure; the short snippet above leaves
partial-output disposal to the caller.

The browser example runs one conversion at a time. Failed or cancelled
conversions remove their private OPFS output. When using its download
fallback, save the download and click **Discard download** to revoke the
URL and remove the private output; starting another conversion also removes
the previous download. Discard before closing the tab: browser shutdown
cannot reliably await storage cleanup. The save-picker path writes directly
to the file you choose.

### API

- `convert(wasm, source, sink, options)` detects the format from at most five
  leading bytes, or 1,024 when looking for a displaced `%PDF-` header
  ([header offset rule](../docs/pdf-input.md#header-offset)), or uses
  `options.format`, and resolves with
  `{ format, inputBytesRead, outputBytesWritten, pagesConverted, bookmarksWritten, outlineWarnings }`.
- `inspect(wasm, source, options)` resolves with
  `{ format, pageCount, bookmarkCount, outlineWarnings, applicationInfo, inputBytesRead }` without output.
  `bookmarkCount` is validated/countable for CAJ and HN-A; it is `null`
  for PDF, KDH, C8 and HN-B (unknown, not zero). HN-A validation streams
  one outline record at a time and reads no image payloads. No codec tables
  or scratch stores are required for HN/C8 inspection.
- `outlineWarnings` counts HN-A bookmark defects. An entry with an invalid
  title, page or zero level is skipped and its children are re-parented; a
  level that skips a parent is clamped. The rest of the outline and every page
  are still written, and `bookmarkCount` is what conversion writes. The count
  is `0` for other formats. Located per-entry reasons are listed by the CLI
  (see [docs/cli.md](../docs/cli.md#hn-a-bookmark-defects)); earlier
  releases rejected these documents.
- `applicationInfo` is `{ doi, url, noteCount }` from a C8 application-info
  package, the same values the CLI's `inspect` prints and conversion writes to
  the PDF `/Info`. It is `null` when there is no package, when the package is
  defective (conversion ignores it too), and for other formats. A missing
  field is `null`. The DOI is a CNKI identifier, not a verified DOI, and the
  URL is never followed. Detecting the package reads only the last 32 bytes
  of the source.
- `wasm` is a `WebAssembly.Module` (each call instantiates its own instance,
  so calls may run concurrently), an `Instance`, or its exports. An instance
  runs one operation at a time and rejects a second concurrent one.
- Options: `format` (`"auto"` by default), `limits`, `chunkSize` (bytes per
  request, default 256 KiB, at most 1 MiB), `signal`, and
  `includeBookmarks` (default `true`).
- `limits`: `maxInputBytes` (8 GiB), `maxOutputBytes` (16 GiB),
  `maxAllocationBytes` (64 MiB; at most 256 MiB and at least `chunkSize`; a single allocation limit, not a total memory budget),
  `maxPages` and `maxBookmarks` (100,000). JavaScript validates them and the
  Rust engine enforces them.
- Errors are `Caj2PdfError` with a stable `code` (for example
  `MALFORMED_CAJ`, `PDF_LIMIT_EXCEEDED`, `TRUNCATED_INPUT`) and the core's
  located message. `UnsupportedFormatError` adds `format`. Source and sink
  errors and abort reasons propagate unchanged.

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
The stream pump releases its owned Web reader on EOF or failure. On failure it
initiates cancellation before release; stalled or rejected producer cancellation
cannot delay or replace the primary error. Node spool writes validate positive,
in-range progress and check cancellation around each awaited partial write.

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

## Bilevel output compression

Bilevel image streams use lossless Flate compression. JPEG streams are copied
unchanged. PDF byte snapshots/hashes change even when pixels and layout do not.
For bilevel conversion, `limits.maxAllocationBytes` must be at least `512n *
1024n` for the fixed compressor reservation (the default 64 MiB already covers
this). This is a per-allocation requirement, not a whole-process memory budget.
See the [compression measurements](../docs/research/bilevel-compression.md).

## Scoped HN/C8 scratch

Both platform entry points export `withHnc8Scratch`. It creates four stores,
awaits your callback, then closes their handles and removes their private
folder on success, failure, or cancellation. Await all conversion work inside
the callback; the stores must not escape it. The default cap is 64 MiB **per
store** (up to 256 MiB of scratch), separate from WASM allocation limits.

```js
import { convert, withHnc8Scratch } from "caj2pdf-rust/node";
// Use "caj2pdf-rust/browser" inside a Dedicated Worker for OPFS storage.
const report = await withHnc8Scratch(
  (scratch) => convert(module, source, sink, { signal, hnc8: { scratch } }),
  { maxBytes: 64n * 1024n * 1024n },
);
```

Node optionally accepts `directory`; browser optionally accepts `storage`.
The caller still owns input/output and must abort or discard partial output
on failure. Cleanup failures reject the operation. Existing adapters below
remain available when you want to own scratch handles yourself.

The Node example now uses this scope for HN/C8 as well as CAJ/KDH/PDF.
The browser example transfers a backpressured output stream to
[`examples/browser-worker.mjs`](examples/browser-worker.mjs), performs
conversion and OPFS scratch work there, and forwards cancellation as a message.
It waits for cleanup before reporting completion; do not replace cancellation
with `worker.terminate()`. Browser use requires OPFS and Dedicated Workers.
HN/C8 rendering remains experimental with the documented layout limitations.
C8/HN-B outline layouts are unverified: with bookmarks requested they convert
with no outline and the report's `outlineOmitted` is `true`. Pass
`includeBookmarks: false` (`--no-bookmarks` in the Node example) to skip it.

## Random-access scratch for HN/C8 integration

`fileHandleScratch(handle, { maxBytes })` (Node, async) and
`syncAccessHandleScratch(handle, { maxBytes })` (browser, synchronous construction)
wrap caller-owned read/write storage. Both return the same async methods:
`resize(size, signal)`, `readAt(offset, length, signal)`,
`writeAt(offset, bytes, signal)`, and `flush(signal)`, plus a current `size`
BigInt. Reads/writes may complete a short prefix. Each request is at most
1 MiB and stays inside the explicitly resized extent. `maxBytes` is required
and must not exceed `Number.MAX_SAFE_INTEGER`; file truncation and browser
positions use exact Numbers. There is no whole-image allocation.

Grant the adapter exclusive access and await each operation before issuing
another. Keep write bytes unchanged until the promise settles. Node cancellation
waits for pending file I/O to finish before rejecting, so cleanup cannot race an
abandoned write or resize. Completed resize updates `size` even if cancellation
arrives during that operation. Node `flush` is an ordering barrier; it does not
fsync disposable storage. Closing and deleting files belong to the caller.

Browser scratch requires an OPFS access handle in a **Dedicated Worker**:

```js
import { syncAccessHandleScratch } from "caj2pdf-rust/browser";

// `file` is a caller-created OPFS FileSystemFileHandle in this worker.
const handle = await file.createSyncAccessHandle();
try {
  const scratch = syncAccessHandleScratch(handle, { maxBytes: 64n * 1024n ** 2n });
  await scratch.resize(4096n);
  await scratch.writeAt(0n, new Uint8Array([1, 2]));
  const bytes = await scratch.readAt(0n, 2);
  await scratch.flush();
} finally {
  handle.close(); // The caller also removes its temporary file.
}
```

The [File System standard](https://fs.spec.whatwg.org/#api-filesystemsyncaccesshandle)
defines this worker-only handle. It permits reads and writes against the same
live file; an unclosed `createWritable()` stream and a `getFile()` snapshot do
not provide that contract. Pass four independent adapters to `convert` as shown below.

## Experimental HN/C8 conversion

`convert` uses the built-in standard QM/MQ states. Provide `hnc8: { scratch }`
for arithmetic image decoding.

```js
// `source`, `sink`, `wasm` use the ordinary streaming API.
// `scratch` is a tuple of four distinct caller-owned adapters described above.
// No external state-table files are needed.
const result = await convert(wasm, source, sink, {
  includeBookmarks: false, // C8/HN-B outlines are unverified and would be omitted anyway.
  hnc8: { scratch },
});
```

Use Node file handles or live OPFS sync access handles in a Dedicated Worker.
Grant exclusive ownership of all four disposable stores for the operation.
Each store is capped at 64 MiB by the Rust composition budget; an adapter may
impose a smaller cap. Buffers remain bounded and output is sequential.
On success, failure or cancellation, the driver resets the Rust operation and
attempts to resize every supplied store to zero, without the cancelled signal.
If cleanup fails, `AggregateError.errors` preserves the conversion error first
(if any) and all cleanup errors. The caller must still close handles and remove
files in its own `finally`; cleanup cannot guarantee removal after host failure.

HN-A/C8 type-1 and type-2 JPEG images share the bounded validation/emission
path. The HN/C8 adapter also selects the documented unused-refinement-template
interoperability policy; generic JBIG2 parsing remains strict. See the
[type-1 profile and checks](../docs/research/hnc8-type1.md).

HN-A outlines are supported. C8/HN-B currently require explicit
`includeBookmarks: false`. Admitted native C8/HN-B text pages use the
[explicit font sources](#explicit-fonts-for-native-c8-and-hn-b-pages) below;
unverified content is rejected rather than silently omitted. The shared HN/C8
route admits only the documented unused-refinement-template anomaly. Other
malformed JBIG2 headers remain errors. Image-only pages receive no OCR text
layer; [Unicode, whitespace and reading-order limits](../docs/research/hnc8-text-fidelity.md)
remain separate from native glyph rendering. HN/C8 inspection validates metadata without implying
that the document can be converted. Located conversion and metadata failures use error code
`HNC8`. Standard numeric state adoption is recorded in #189.

### v0.x migration

HN-A/C8 output now follows declared page/image extents and omits DIB padding.
PDF page sizes, image widths and hashes change; regenerate affected snapshots.
Zero extents are rejected. The physical unit remains empirical; see the
[geometry correction](../docs/cli.md#source-geometry-correction-breaking-v0x).

HN/C8 conversion no longer always throws `UnsupportedFormatError`: callers must
handle `HNC8`, invalid configuration and missing scratch errors. Existing
PDF/CAJ/KDH calls do not need `hnc8`. Rust users must handle the new
`Error::Hnc8`, `Error::Hnc8Metadata`, `Status` and `Request` variants when matching exhaustively.
HN/C8 inspection now succeeds for valid metadata; malformed headers/outlines
use `HNC8` instead of a blanket unsupported-format error. C8/HN-B unknown
bookmark counts remain `null`, while validated empty HN-A outlines return zero.
Raw WASM hosts must implement statuses 6–9 (scratch read/write/resize/flush),
use `caj2pdf_io_request_store()` (1–4), and acknowledge resize through
`caj2pdf_io_complete_resize()`. The request offset holds the new extent for
resize; read/write reuse the staging buffer and completion exports.
The raw host owns cleanup if it cancels or drops a pending operation.

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

Current browser/Node small/large measurements, temporary-storage results,
and reproduction commands are in [JavaScript validation](../docs/js-validation.md).

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
  also covers `inspect`, TEB rejection, typed errors, limits,
  cancellation, sink and source errors, and the memory measurement above.
- `spool.test.mjs` covers Node temp-file spooling from Node and Web streams,
  the spool bound, cleanup after success, failure, and abort, and the OPFS
  spool against an in-memory OPFS test double.
- `browser.test.mjs` runs `browser.mjs` in headless Chromium (below).
- `browser-example.test.mjs` drives the actual HTML example in Chromium,
  checks failed/cancelled output cleanup, validates a downloaded PDF, and
  verifies replacement/discard removes OPFS files and revokes download URLs.
- `package.test.mjs` runs `npm pack` on a temporary copy of the package
  with the WASM build and asserts the tarball holds exactly the entry points,
  declarations, `caj2pdf_wasm.wasm`, `package.json`, `LICENSE`, and
  `README.md`. It extracts the actual tarball into a fresh consumer, checks
  Node package exports and default WASM loading, and converts CAJ through
  the packed browser entry in Chromium. Packing without a valid WASM build
  must fail.
- `examples.test.mjs` runs the Node example as a subprocess for CAJ/KDH/PDF/HN
  files and stdin, validates output PDFs, and checks missing/malformed inputs,
  existing-output preservation and usage errors.
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
| HN/C8 in the corpus runner | `not_run` | Runner has no caller-table/scratch configuration; conversion is not attempted | `NOT_RUN` |
| TEB | `unsupported` | `UnsupportedFormatError` | `unsupported` |
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
`issue-49`/`issue-73` outlines; see [CAJ format notes](../docs/research/caj-format.md))
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
- HN/C8 inspection validates source metadata; conversion uses the experimental image-page path.
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
Firefox, Safari, and Web Workers are not covered, and native file-picker/save dialogs are not automated. The example
page's OPFS fallback is tested through actual Chromium File and storage APIs.

An OPFS spool failure normally preserves the original error. Cleanup retries
only a transient `NoModificationAllowedError` (at most three attempts, with
10 ms and 50 ms delays). If the failed spool still cannot be removed, rejection
is an `AggregateError`: `cause` and `errors[0]` hold the original failure,
`errors[1]` holds the removal failure. The temporary file may then remain.

### Fonts for native C8 and HN-B pages

The admitted native C8 and HN-B text/vector profiles accept caller-owned
ranged font sources. Only `cjk` and `latin` are required:

```js
await convert(wasm, documentSource, outputSink, {
  includeBookmarks: false, // C8/HN-B bookmarks are not supported yet.
  hnc8: {
    scratch, // Existing four reusable stores for image decoding, when needed.
    fonts: {
      cjk: cjkFontSource,
      latin: latinFontSource,
      // Optional roles; absent ones use the fallback rule below.
      alternateLatin: alternateFontSource, // 801d/4 Latin resource.
      decoration: { source: decorationFontSource, character: "►" },
      symbols: symbolFontSource, // Semantic HN-B mode-0 symbols/space.
      latinState3: state3FontSource, // HN-B/C8 801d/3 Latin resource.
      latinState28: state28FontSource, // C8 801d/28 resource.
      latinState31: state31FontSource, // C8 801d/31 resource.
    },
  },
});
```

**Fallback rule** (implemented in the core, so CLI, Node and browser
behave the same): each glyph uses the font of the role the source selects.
If that role is absent, or its font does not map the character, CJK-coded
characters (CJK blocks and halfwidth/fullwidth forms) use `cjk` and every
other character, including the decoration alias `►`, uses `latin`. A glyph
missing from that font still fails with its page and source byte. There
is no system-font lookup. The CLI's `--fonts DIR` lookup is not part of
the JavaScript API: open the font files and pass them as sources.

Fonts may be supplied for any HN/C8 input. The core inspects the header and
the first page with text: only a C8/HN-B document in a native rendering mode
whose text holds native records uses the fonts. HN-A and other image documents are converted as images,
byte-identical to a conversion without `hnc8.fonts`, and the font sources
are not read. The CLI applies the same rule.

Tested free-font recipe for Node. The fonts are installed separately and
never bundled. See [docs/cli.md](../docs/cli.md#tested-free-font-recipe)
for the installation command and the supported font formats:

```js
import { open } from "node:fs/promises";
import { convert, fileHandleSource, withHnc8Scratch } from "caj2pdf-rust";

const cjk = await open("/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf", "r");
const latin = await open("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "r");
try {
  const fonts = { cjk: await fileHandleSource(cjk), latin: await fileHandleSource(latin) };
  await withHnc8Scratch((scratch) => convert(module, documentSource, outputSink, {
    includeBookmarks: false,
    hnc8: { scratch, fonts },
  }));
} finally {
  await cjk.close();
  await latin.close();
}
```

Each font uses the same `size: bigint` / `readAt(offset, length, signal)`
contract as the document. Fonts may have TrueType or CFF outlines. A role
may instead be `{ source, face }` to use face `face` of a collection
(`.ttc`); a plain source is face 0. Browser `blobSource` and Node `fileHandleSource`
work for fonts too. Reuse the same source object across roles to embed it
once. Character coverage alone does not guarantee compatible widths or
bearings, or prevent overlap at fixed source positions. See the
[same-resource controls](../docs/research/c8-real-font-fidelity.md#same-resource-control-follow-up).
The optional decoration character is a nonsemantic BMP alias, not document
text. Roles exist because source role selection differs from Unicode/script
selection.

Keep sources stable and open until conversion settles; the converter does
not close caller-owned resources. Forward-only fonts can use the existing
bounded spooling helpers; dispose their returned handles in `finally`.
Reads share one WASM staging buffer with document and scratch I/O. Errors
and cancellation use the existing cleanup path. The caller must discard
partial output after failure, including a final flush failure.

This enables only the independently admitted native profiles. HN-B mixed
images and unverified glyph/style combinations remain explicit errors;
source-font identity is not inferred. Use the
matching JavaScript and WASM builds; older WASM binaries cannot accept
font registration. Existing image-only conversions require no font options.

### Damaged CAJ inputs

Pass `{ allowDamaged: true }` to explicitly permit blank substitutes for
unrecoverable CAJ pages. Always inspect `report.omittedPages`: each entry has a
zero-based `pageIndex` and an absolute source `offset` (`bigint`). Conversion
resolves with the report when it produces a partial PDF; I/O, cancellation,
resource-limit, unsupported-feature, and unavailable-geometry errors still
reject. Shared damaged resources may affect multiple pages. The default is
strict conversion. This option applies to CAJ, not every recognized format.

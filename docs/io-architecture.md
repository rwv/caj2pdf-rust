# Bounded I/O architecture

The I/O contract was first chosen in
[issue #4](https://github.com/rwv/caj2pdf-rust/issues/4) and made synchronous
in [issue #355](https://github.com/rwv/caj2pdf-rust/issues/355).

## Contract

The core is synchronous. It reads a **sized random-access source** and writes
to any [`std::io::Write`](https://doc.rust-lang.org/std/io/trait.Write.html):

- `RangedSource::size()` is a stable `u64` snapshot for one operation;
  `read_at(offset, destination)` reads at an absolute offset and may return a
  short read. A zero-byte read before the known end is a truncated-input
  error. The core never reads a document whole.
- Output bytes are written in order; the core never seeks its output. A
  writer may accept fewer bytes than offered. The core flushes once, after
  the last byte.
- `Cancellation::is_cancelled()` is checked between rows, pages and I/O
  chunks. Cancellation returns a distinct error; it does not undo bytes
  already written. Callers that need atomic path output stage it outside the
  core and commit it only after success (the CLI does).

Bounded means capped by `Limits`, not spooled. One image payload, one symbol
dictionary or one page bitmap may be held in memory, each read once and
capped by `max_allocation_bytes`; HN/C8 bitmaps live in memory and no
conversion creates a temporary file. The default limits are 8 GiB input,
16 GiB output, 64 MiB for any single allocation (at most 256 MiB), and
100,000 pages or bookmarks. Reads and writes are at most 1 MiB per call; the
default chunk is 256 KiB. Memory budgets also include retained indexes,
bookmarks and decoder state.

Format engines are plain functions over `RangedSource`, `Write` and
`Cancellation`. Bookmark visits use a `BookmarkVisitor` rather than a
whole-outline vector. Conversion returns a `ConversionReport` with byte, page
and bookmark counts; inspection returns a bounded `DocumentInfo`. The core
`Error` distinguishes unsupported format, invalid input, truncated input,
resource limit, I/O failure, cancellation and located format errors.

## Adapters

| Environment | Input | Output | Forward-only input |
| --- | --- | --- | --- |
| Native Rust | `SeekableSource` over any `Read + Seek` (`File`, `Cursor<Vec<u8>>`), or `&[u8]` | Any `Write` (`File`, `BufWriter`, `Vec<u8>`) | The caller spools it (the CLI spools standard input to an anonymous temporary file) or rejects it. |
| Browser | `Blob`/`File`, read in the Worker with `FileReaderSync`; OPFS `FileSystemFileHandle`, read through a synchronous access handle | `webWritableSink(writer)` or any `{ writeChunk, flush }` sink on the calling thread | `convertReadableStream` spools to a bounded OPFS file and removes it; without OPFS writes it rejects with `RANDOM_ACCESS_REQUIRED`. |
| Node.js 22+ | File path, `file:` URL or descriptor, read in the Worker with `fs.readSync`; `Blob`, read on the calling thread | `nodeWritableSink(writable)` or any sink | `convertReadable` spools to a bounded private temporary file and removes it. |

`SeekableSource` snapshots the size of its handle. Adapters borrow or own
their handles; JavaScript inputs stay caller-owned and must stay unchanged
until the operation settles.

## Worker model

The WASM module exports synchronous operations. Each JavaScript `convert` or
`inspect` call starts a fresh Worker (`node:worker_threads` in Node.js), which
instantiates the compiled `WebAssembly.Module` and calls one export to
completion. The module imports five host functions:

| Import | Meaning |
| --- | --- |
| `caj2pdf_read(resource, offset, ptr, len) -> i32` | Copy at most `len` bytes of resource 0 (the document) or 1–8 (C8 fonts) into Rust memory; count or negative failure. Offsets are exact below 2^53. |
| `caj2pdf_write(ptr, len) -> i32` | Take Rust-owned bytes; the Worker copies them and returns the count. |
| `caj2pdf_flush() -> i32` | Output barrier after the last write. |
| `caj2pdf_progress(done, total)` | Thousandths of the document read so far. |
| `caj2pdf_cancelled() -> i32` | Nonzero once the caller aborted. |

The exports are `caj2pdf_convert` and `caj2pdf_inspect` (status 0 done,
1 failed, 2 invalid configuration, 3 busy), C8 font registration, the error
kind and message, report and inspection getters, and `caj2pdf_reset`. A
session holds one operation and its result until reset. No staging buffer or
allocator export is needed: reads land directly in Rust-owned slices and
writes pass pointers that the Worker copies before returning.

The Worker posts each output chunk (a transferable copy), each flush and
progress report to the calling thread, which awaits the caller's sink in
order; the operation settles after the final flush. When
`SharedArrayBuffer` is available (always in Node.js; in browsers when the
page is cross-origin isolated), a shared `Int32Array` carries cancellation,
sink acknowledgements (the Worker pauses after eight unacknowledged chunks)
and reads the calling thread serves. An abort then stops the Worker at its
next checkpoint; it closes its inputs before the promise rejects. Without
shared memory, progress and output still flow, but an abort terminates the
Worker at once and output is not throttled by the sink.

## Native C8 font resources

`C8FontSources` gives the core up to eight explicit ranged resources and role
indices; repeated roles can share an embedded font. Each source is read for
metadata before the first page and again, with only the drawn glyphs'
outlines, after the last page. JavaScript exposes named roles under
`hnc8.fonts` (inputs of the same kinds as the document) and deduplicates
identical inputs; it never discovers fonts or buffers a whole font.

Before converting, a raw host registers each font with
`caj2pdf_c8_add_font(size, face)` (read resource 1–8, 0 on rejection), assigns
zero-based role indices with `caj2pdf_c8_set_fonts(cjk, latin, alternate,
decoration, alias)` or `caj2pdf_c8_set_fonts_with_symbols(..., symbols)`
(`0xffffffff` marks an absent optional role), and supplies the optional
state-3, 28 and 31 Latin roles with `caj2pdf_c8_set_latin_state(state,
index)`. Absent optional roles use the CJK/Latin fallback of `C8PageFonts`; a
glyph missing from that font fails with its location.

The CLI and the WASM engine both call `hnc8::convert_document_pdf`, which
chooses the composer once per document: `uses_native_text` reads the header
and walks page rows and image descriptors to the first page with text, never
reading image payloads. When image composition is chosen, the fonts stay
unread.

## Verification

```sh
cargo test --locked --workspace
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
node js/scripts/copy-wasm.mjs
node --test js/test/*.test.mjs
```

The WASM engine tests call the session directly with an in-memory host. The
JavaScript tests run the actual module through the raw ABI and through the
public Worker API over paths, descriptors, Blobs and real OPFS handles, in
Node.js and headless Chromium (cross-origin isolated and not), including
short I/O, cancellation, spooling and typed errors. Firefox and Safari are
not tested automatically.

## References

- [Rust `Read`, `Seek`, and `Write`](https://doc.rust-lang.org/std/io/)
- [`FileReaderSync`](https://developer.mozilla.org/en-US/docs/Web/API/FileReaderSync)
- [`FileSystemSyncAccessHandle`](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemSyncAccessHandle)
- [Node.js `worker_threads`](https://nodejs.org/api/worker_threads.html)
- [Cross-origin isolation and `SharedArrayBuffer`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer#security_requirements)

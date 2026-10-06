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
- `Progress::is_cancelled()` is checked between rows, pages and I/O
  chunks. Cancellation returns a distinct error; it does not undo bytes
  already written. Callers that need atomic path output stage it outside the
  core and commit it only after success (the CLI does).
- `Progress::format()` receives the detected format once, before the
  operation refuses it or reads past the signature, and
  `Progress::input_read(done, total)` the furthest document byte read so
  far. `NeverCancel` reports nothing and never cancels.

Bounded means capped by `Limits`, not spooled. `Limits` is the only set of
resource bounds; each operation validates it once, and every other bound is
derived from its fields or from the format itself:

| Field | Default | Bounds |
| --- | --- | --- |
| `io_chunk_bytes` | 256 KiB | each read or write request, at most 1 MiB (`MAX_IO_CHUNK`) |
| `max_input_bytes` | 8 GiB | the selected input |
| `max_output_bytes` | 16 GiB | the output of one operation |
| `max_allocation_bytes` | 64 MiB | one allocation: one image payload, one JBIG2 symbol store, one page or region bitmap |
| `max_pages` | 100,000 | pages accepted from an input |
| `max_bookmarks` | 100,000 | bookmarks accepted from an input |
| `max_image_pixels` | 12,000,000 | one decoded image, page, region, refinement or symbol bitmap, and a text region's symbol instances |
| `max_symbols` | 8,192 | the symbols of one JBIG2 dictionary (new, exported, imported plus new), its height classes and its export runs |

One image payload, one symbol store or one page bitmap may be held in
memory, each read once; HN/C8 bitmaps live in memory and no conversion
creates a temporary file. Decoding work is bounded by these sizes: the
decoders count no arithmetic work or I/O calls of their own. An exceeded
limit fails with a located `LimitExceeded` error naming the resource. The
JavaScript `limits` option sets the first six fields; `max_image_pixels` and
`max_symbols` keep their defaults there.

## Facade

The CLI, the WASM engine and the fuzz targets call one facade in
`caj2pdf_core`; none of them dispatches on the format:

- `convert(source, sink, ConversionOptions, &Limits, &mut dyn Progress)`
  detects the format from its leading signature (or takes
  `ConversionOptions::format`), copies a PDF from its `%PDF-` header,
  rebuilds CAJ and KDH, and composes HN/C8 pages. It returns a
  `ConversionReport`: bytes read and written, pages, bookmarks, the CAJ pages
  replaced with blanks under `allow_damaged`, the HN-A outline warnings or
  the omitted C8/HN-B outline, the C8 application-info status and the HN/C8
  image counts per codec. An empty or unrecognized input, NH and TEB are
  refused with an `UnsupportedFormat` error that has no offset, context or
  reason.
- `inspect(source, &InspectOptions, &Limits, &mut dyn Progress)` returns a
  bounded `DocumentInfo`: format, HN/C8 variant, page count, outline presence
  and count, the CAJ or HN-A outline when asked, the C8 application-info
  package and, when asked, the KDH signature or HN/C8 page-index
  `Structure`. `inspect_pages` then streams one structural record per HN/C8
  page to a `PageVisitor`.
- `needs_fonts` says whether a document would draw native text;
  `read_outline` and `index_pdf` read the two inputs of an outline import.

Outlines stream through a `BookmarkVisitor` rather than a whole-outline
vector.

Every failure is one core `Error { kind, offset, context, reason }`. `kind`
is an `ErrorKind`: unsupported format, malformed, encrypted, truncated
(expected and available bytes), limit exceeded (resource, limit and
attempt), I/O or cancelled. `offset` is the source byte, when known.
`context` is a `Context` naming the format and its location: a CAJ record,
KDH, a PDF object (or an ambiguous repair), an HN/C8 variant, page, image,
JBIG2 segment and conversion stage (`Hnc8Stage`, `Type3Stage`), or a JBIG2
segment. `reason` is a static description. A decoder returns an unlocated
error; the format layer that knows the position fills in the offset and
context once, and an error located deeper keeps its own. The message names
the kind and format, then the byte, location, reason and details that are
present (for example `malformed CAJ at byte 276, record 3: empty CAJ TOC
title`); only an I/O error has a `source()`.

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

`ConversionOptions::fonts` gives the core up to eight explicit ranged
resources and role indices; repeated roles can share an embedded font. Each source is read for
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

`convert` chooses the composer once per document: it reads the header and
walks page rows and image descriptors to the first page with text, never
reading image payloads. When image composition is chosen, the fonts stay
unread and the PDF is the same as without fonts; fonts given for any other
format are refused. The CLI asks `needs_fonts` before it searches for
installed fonts; JavaScript registers the fonts its caller names.

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

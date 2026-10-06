# Bounded I/O architecture

This note records the I/O contract chosen in
[issue #4](https://github.com/rwv/caj2pdf-rust/issues/4). It is the contract
for later format parsers and PDF writers. Issue #4 provides adapters and a
bounded read/write proof of concept; it does not implement document conversion.

## Decision

The core accepts a **sized random-access source** and a **sequential output
sink**. Bounded means capped by `Limits`, not spooled: the core never reads a
document whole, but one image payload, one symbol dictionary or one page
bitmap may be held in memory. Spooling to temporary storage is a platform
adapter's job for forward-only input, not a core requirement for decoding
(issue #346 records the change from the earlier stricter reading). Input records can refer to earlier or later offsets, while PDF bytes
can be emitted in order. A source read names an absolute byte offset and
returns only the requested range. The sink writes in order and does not expose
seek. The adapters accept borrowed handles, so callers can retain ownership.

`RangedSource::size()` is a stable `u64` snapshot for one operation, and its
async `read_at(offset, destination)` may return a short read. The async
`SequentialSink::write(bytes)` may make partial progress; `flush()` completes
the operation. `Cancellation::is_cancelled()` is polled between I/O calls.
The native `SeekableSource` snapshots the size of a `Read + Seek` handle and
restores its original position during construction. `WriteSink` wraps any
`Write`, including a caller-owned borrowed handle.

The core contract is asynchronous. On native Rust, adapters implement it over
`Read + Seek` and `Write` without a browser or Node.js dependency. JavaScript
drives a pinned Rust future through a dependency-free raw WASM ABI: it polls
until Rust requests a read, write, or flush, awaits that operation, supplies
its bounded result, and polls again. The platform-neutral
[`engine`](../crates/caj2pdf-wasm/src/engine.rs) owns that future: it detects
the input format from its leading signature with the core's
`detect_source` (shared with the CLI) and runs the core PDF, CAJ, or
KDH engine (issue #13). A native caller
can drive its future with an executor of its choice; the core does not choose
an executor.

For native HN/C8 bitmap storage, `native::FileScratch::new(file, max_bytes)`
adopts a caller-created regular file and implements `RandomAccessScratch`.
The caller supplies a read/write handle and grants exclusive access during
conversion. Resizing is capped; positioned requests must fit both the declared
file extent and `MAX_IO_CHUNK`. The adapter allocates no image buffer and
preserves short I/O and OS errors. `into_inner()` returns the handle.
Dropping closes it but does not delete its path: the CLI or embedding caller
owns temporary-file creation/unlinking. These blocking operations suit native
file adapters; the asynchronous browser/Node bridge remains separate.
The source-page example uses this adapter for its four reusable stores.
This storage API does not resolve codec-state distribution or enable HN/C8
CLI/JS routing on its own; those remain #10 work.

The WASM instance permits one active operation. A fixed staging allocation
holds at most one configured chunk. The JS driver reads the requested range,
copies only that chunk to WASM memory, and resumes the future. On output, it
awaits the sink's accepted bytes or flush before resuming. The Rust future
owns its source and sink adapters, so it can remain pinned across polls
without a self-referential struct or unsafe Rust code. The ABI also exposes
cancellation, a typed error category, byte counters, and reset. Each caller
using the raw ABI must reset the engine after completion or failure; the
JavaScript driver in [`js/io.mjs`](../js/io.mjs) does this in a `finally`
block.

## Bounds and progress

The initial I/O target is at most **1 MiB per read or write call**. The
default working chunk is 256 KiB. The core checks source ranges using checked
arithmetic and enforces configured limits before allocating or requesting
data. The default limits are 8 GiB input, 16 GiB output, 64 MiB for any single
allocation, and 100,000 pages or bookmarks. Callers can set lower limits.
A source may return fewer bytes than requested; a zero-byte read before the
known end is a truncated-input error. A sink may accept fewer bytes than
offered; writing continues until the chunk is complete or returns an error.
The core copy helper requests the next chunk only after the previous write
has completed. It reuses one Rust chunk buffer; JavaScript and WASM boundary
copies can temporarily hold additional copies of that same bounded chunk.
Later conversion memory also includes format-specific indexes and decoder
state.

Cancellation is checked at I/O boundaries. A caller can stop before another
read or write. The JavaScript source and sink adapters check for an aborted
operation before and after their awaited calls. Cancellation returns a
distinct error; it does not promise to undo bytes already written.
Callers that need atomic path output must stage it outside the core and commit
it only after success.

## Platform adapters

| Environment | Input | Output | Forward-only input |
| --- | --- | --- | --- |
| Native Rust | Borrowed or owned `Read + Seek` with a known size | Borrowed or owned `Write`; no output seek | Must be spooled by a caller to seekable temporary storage or rejected explicitly. |
| Browser | `blobSource(blob)` uses `slice(start, end)` and awaits `arrayBuffer()` for that slice only | `webWritableSink(writer)` awaits each `WritableStream` write and leaves the writer open | `convertReadableStream` spools to a bounded Origin Private File System file and removes it; without OPFS writes it rejects with `RANDOM_ACCESS_REQUIRED`. |
| Node.js 22+ | `fileHandleSource(handle)` snapshots size and uses BigInt positioned reads on a caller-owned file handle | `nodeWritableSink(writable)` awaits each write callback and leaves the stream open | `convertReadable` spools a Node or Web stream to a bounded private temporary file and removes it. |

Browser `Blob.slice()` produces a subset of the input; it does not require a
whole-Blob `arrayBuffer()` call. A Blob's numeric size must fit JavaScript's
safe integer range before it can be represented exactly. Node.js 22 or newer
supports `FileHandle.read({ position: bigint, ... })`, allowing positioned
reads without converting a `u64` offset to a JavaScript number. The Node
adapter leaves the caller's handle open. JavaScript adapters belong outside
the core crate.

`convert` in [`js/io.mjs`](../js/io.mjs) drives the raw WASM bridge against
a source exposing `{ size: bigint, readAt(offset, length, signal) }` and a
sink exposing `writeChunk(bytes, signal)` and `flush(signal)`. It awaits each
bounded read and write; no whole-document byte array crosses the WASM
boundary. `js/node.mjs` and `js/browser.mjs` add platform adapters and
spools. A plain forward-only stream lacks the required size and `readAt`
method, so it is converted only through a bounded spool on durable
temporary storage and is never buffered whole in memory. The
[package guide](../js/README.md) documents the API, limits, and browser
storage support.

`AbortSignal` is checked before and after each awaited JavaScript operation,
and the supplied adapters race pending reads and writes against it, so a
stalled operation stops waiting at once. The abandoned operation may still
finish, but only into its own copied buffer. The driver then cancels and
resets the Rust engine without starting another I/O request. Partial output
remains the caller's responsibility.

## Error and operation contract

The core `Error` distinguishes unsupported format, invalid input, truncated
input, resource limit, I/O failure, cancellation, and random-access-required
errors. Its `DocumentOperations` trait defines async `inspect`,
`visit_bookmarks`, `convert`, and `import_bookmarks` methods. Bookmark visits
use a recipient rather than a whole-outline vector. Conversion returns a
`ConversionReport` with input/output byte counts and page/bookmark counts;
inspection returns a bounded `DocumentInfo`. These are signatures for later
format engines, not working conversion entry points yet. No operation accepts
a whole-document byte array as its primary interface.

The issue #4 `copy_range` helper is a bounded I/O proof. It is useful for
testing short reads, partial writes, cancellation, and limit enforcement,
but copying bytes is not PDF conversion.

The [native example](../crates/caj2pdf-core/examples/native_bounded_copy.rs)
passes borrowed `Read + Seek` and `Write` handles into these adapters and
requires no output seek. Run it with
`cargo run -p caj2pdf-core --example native_bounded_copy`.

## Verification

```sh
cargo test --locked -p caj2pdf-core -p caj2pdf-wasm
cargo check --locked -p caj2pdf-core --tests --examples --target wasm32-unknown-unknown
cargo build --locked --release --all-features -p caj2pdf-wasm --target wasm32-unknown-unknown
node --test js/test/*.test.mjs
```

The Rust engine tests drive the same poll/resume state machine natively.
The JavaScript tests instantiate the actual WASM module. They convert
synthetic PDF, CAJ, and KDH inputs through bounded `Blob.slice()` reads and a
real Node file handle, and exercise partial I/O, backpressure, spooling,
cancellation, and typed errors. The browser adapters run on Node's Blob and
Web Streams implementations, and `js/test/browser.test.mjs` also runs the
browser entry point in headless Chromium, including the real OPFS spool (see
[the JavaScript package guide](../js/README.md)). Firefox and Safari are not
tested automatically. No fixture here is an external CAJ document.

Memory budgets for format engines must include retained indexes, bookmarks,
and decoder state in addition to the I/O chunk.

## References

- [Rust `Read`, `Seek`, and `Write`](https://doc.rust-lang.org/std/io/)
- [Browser `Blob.slice()`](https://developer.mozilla.org/en-US/docs/Web/API/Blob/slice)
- [Node.js positioned `FileHandle.read`](https://nodejs.org/api/fs.html#filehandlereadbuffer-options)
- [Browser writable-stream writer](https://developer.mozilla.org/en-US/docs/Web/API/WritableStreamDefaultWriter/write)

## Native C8 font resources

`C8FontSources` gives the core up to eight explicit ranged resources and
role indices. Repeated roles can share an embedded font. Each source is read
for metadata before the first page and again, with only the drawn glyphs'
outlines, after the last page, so it must stay readable and unchanged until
conversion completes. JavaScript exposes
named roles under `hnc8.fonts` and deduplicates identical source objects.
It does not discover fonts, collect a whole font in a JavaScript buffer,
or create another scheduler. Existing spool helpers can turn a forward-only
font into a caller-owned ranged source with bounded temporary storage.

The CLI and the WASM engine both call `hnc8::convert_document_pdf`, which
chooses the composer once per document. With fonts, `uses_native_text`
reads the header (a document outside native composition's variants and
rendering modes stops there), then walks page rows and image descriptors
with one cursor to the first page with text. HN-B text selects native
composition; C8 text is classified with the bounded `inspect_text` readers
(the default `TextBudget`). Image payloads are never read and no text is
retained. When image composition is chosen, the fonts stay unread.

The WASM host registers resource sizes before the first poll using
`caj2pdf_c8_add_font(size, face)` (`face` selects a TrueType collection face, 0 otherwise; returns IDs 1–8; 0 means rejection), then assigns zero-based role
indices using `caj2pdf_c8_set_fonts(cjk, latin, alternate, decoration, alias)`.
A decoration index of `0xffffffff` means absent. `caj2pdf_io_request_resource()`
identifies each ordinary read: 0 is the document, 1–8 are registered fonts.
The staging buffer, pending-request slot, completion and cancellation rules
are shared with existing I/O. Register all sources before assigning roles. Registration is rejected after
roles are assigned or polling starts,
excess resources are rejected, and sizes/roles are validated.

**Unstable Rust API change:** `engine::Request::Read` now includes a
`resource: u32` field. Native hosts matching or constructing that variant
must handle it. Raw hosts using fonts must route reads by the new resource
getter; older hosts that register no fonts continue receiving document reads.
Use matching JS/WASM artifacts for the new font API. This does not change
the core `RangedSource` trait or existing image-only conversion options.

HN-B mode 0 can additionally use a semantic `symbols` font for spaces
and punctuation. It is optional. Absent optional roles, including the
alternate Latin role (`0xffffffff` in `caj2pdf_c8_set_fonts*`), use the
documented CJK/Latin fallback of `C8PageFonts`; a glyph missing from the
fallback font still fails with its location.
Raw WASM hosts supplying it call
`caj2pdf_c8_set_fonts_with_symbols(cjk, latin, alternate, decoration, alias, symbols)`.
The final argument is a zero-based resource index, or `0xffffffff` for absent.
The original five-argument export remains available and marks symbols absent.
JavaScript selects the new export only when `hnc8.fonts.symbols` is supplied.

**Unstable Rust API change:** `C8PageFonts` gains `symbols: Option<usize>`;
existing struct literals should set `None` unless supplying the resource.
`Engine::set_c8_fonts` gains a final symbol index (`u32::MAX` for absent).
Font resource capacity is eight; the four image scratch stores are unchanged.

HN-B/C8 state `801d/3` selects an explicitly supplied `latinState3` resource.
Register it with `caj2pdf_c8_set_latin_state3(index)` after the base roles and
before polling. The index is zero-based; omit the call when absent. Invalid,
duplicate or late registration fails. Existing font exports remain unchanged.
The state fails explicitly when its resource is absent. Original controls
establish the Latin resource change and matching bounds for the tested style;
they do not identify a vendor font or establish every punctuation mapping.

**Unstable Rust API change:** `C8PageFonts` also gains
`latin_state3: Option<usize>`; existing literals should use `None` unless
supplying that role. `Engine::set_c8_fonts` retains its current signature;
`Engine::set_c8_latin_state3` supplies the optional additional role.

### C8 extended Latin resources

The optional `latin_state28` and `latin_state31` Rust roles, JS `latinState28`
and `latinState31`, and CLI `--font-latin-state28` / `--font-latin-state31`
carry distinct caller-supplied ranged fonts for verified C8 states 28 and 31.
Register these after base WASM roles with
`caj2pdf_c8_set_latin_state(state, index)`; only states 3, 28 and 31 are
accepted, once each, before polling. The existing state-3 and base registration
exports retain their signatures. Missing required roles fail explicitly.

This unstable Rust API adds two `Option<usize>` fields to `C8PageFonts`.
Existing literal initializers should add `latin_state28: None` and
`latin_state31: None` unless those resources are supplied. The resource limit
is eight; shared source identities reuse existing embedding/spooling. Font
selection never opens an embedded filename or performs system discovery.

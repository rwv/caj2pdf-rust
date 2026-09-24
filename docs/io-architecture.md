# Bounded I/O architecture

This note records the I/O contract chosen in
[issue #4](https://github.com/rwv/caj2pdf-rust/issues/4). It is the contract
for later format parsers and PDF writers. Issue #4 provides adapters and a
bounded read/write proof of concept; it does not implement document conversion.

## Decision

The core accepts a **sized random-access source** and a **sequential output
sink**. Input records can refer to earlier or later offsets, while PDF bytes
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
its bounded result, and polls again. The current future runs `copy_range` as
an I/O proof. Later conversion code can use the same owned-future pattern;
the public conversion package belongs to
[issue #13](https://github.com/rwv/caj2pdf-rust/issues/13). A native caller
can drive its future with an executor of its choice; the core does not choose
an executor.

The WASM instance permits one active operation. A fixed staging allocation
holds at most one configured chunk. The JS driver reads the requested range,
copies only that chunk to WASM memory, and resumes the future. On output, it
awaits the sink's accepted bytes or flush before resuming. The Rust future
owns its source and sink adapters, so it can remain pinned across polls
without a self-referential struct or unsafe Rust code. The ABI also exposes
cancellation, a typed error category, byte counters, and reset. Each caller
using the raw ABI must reset the engine after completion or failure;
`copyRangeProof` does this in a `finally` block.

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
| Browser | `blobSource(blob)` uses `slice(start, end)` and awaits `arrayBuffer()` for that slice only | `webWritableSink(writer)` awaits each `WritableStream` write and leaves the writer open | Plain `ReadableStream` needs a bounded temporary spool supplied by the application or is rejected. |
| Node.js 22+ | `fileHandleSource(handle)` snapshots size and uses BigInt positioned reads on a caller-owned file handle | `nodeWritableSink(writable)` awaits each write callback and leaves the stream open | A nonseekable stream needs a bounded temporary spool supplied by the application or is rejected. |

Browser `Blob.slice()` produces a subset of the input; it does not require a
whole-Blob `arrayBuffer()` call. A Blob's numeric size must fit JavaScript's
safe integer range before it can be represented exactly. Node.js 22 or newer
supports `FileHandle.read({ position: bigint, ... })`, allowing positioned
reads without converting a `u64` offset to a JavaScript number. The Node
adapter leaves the caller's handle open. JavaScript adapters belong outside
the core crate.

`copyRangeProof` in [`js/io.mjs`](../js/io.mjs) drives the raw WASM bridge
against a source exposing `{ size: bigint, readAt(offset, length, signal) }`
and a sink exposing `writeChunk(bytes, signal)` and `flush(signal)`. It awaits
each bounded read and write; no whole-document byte array crosses the WASM
boundary. `js/node.mjs` supplies the Node adapters. These are I/O proofs, not
a JavaScript document-conversion API. A plain forward-only stream lacks the
required size and `readAt` method, so it is rejected unless an application
provides seekable temporary storage. Browser `ReadableStream` spooling and
the production conversion package are separate work.

`AbortSignal` is checked before and after each awaited JavaScript operation.
A `Blob.arrayBuffer()` or file read already in progress may finish before the
abort is observed. The driver then cancels and resets the Rust engine without
starting another I/O request. Partial output remains the caller's
responsibility.

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
cargo test --locked -p caj2pdf-core
cargo check --locked -p caj2pdf-core --tests --examples --target wasm32-unknown-unknown
cargo build --locked --release -p caj2pdf-wasm --target wasm32-unknown-unknown
node --test js/test/*.test.mjs
```

The JavaScript tests instantiate the actual WASM module. They exercise
bounded `Blob.slice()` reads, a real Node file handle, partial I/O,
backpressure, cancellation, and typed errors. The Blob test uses Node's Blob
implementation. [The JavaScript proof guide](../js/README.md) includes a
browser example that can be run manually; automated browser-runtime testing
is still needed for the production JavaScript package. No fixture in this
proof is an external CAJ document.

The format-specific parsers, PDF writer, and JavaScript conversion API will be
built on this contract in later issues. Their memory budgets must include
retained indexes, bookmarks, and decoder state in addition to the I/O chunk.

## References

- [Rust `Read`, `Seek`, and `Write`](https://doc.rust-lang.org/std/io/)
- [Browser `Blob.slice()`](https://developer.mozilla.org/en-US/docs/Web/API/Blob/slice)
- [Node.js positioned `FileHandle.read`](https://nodejs.org/api/fs.html#filehandlereadbuffer-options)
- [Browser writable-stream writer](https://developer.mozilla.org/en-US/docs/Web/API/WritableStreamDefaultWriter/write)

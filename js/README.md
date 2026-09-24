# JavaScript I/O proof

This directory demonstrates the I/O contract for future CAJ conversion. It
copies a requested range byte for byte through the real Rust core
`copy_range` future. It does **not** parse CAJ or produce PDF yet.

Build the raw WebAssembly module and run the Node.js 22+ tests from the
repository root:

```sh
cargo build --locked --release -p caj2pdf-wasm --target wasm32-unknown-unknown
node --test js/test/*.test.mjs
```

The browser proof is in [`examples/browser.html`](examples/browser.html). After
building, serve the repository root over HTTP (for example,
`python3 -m http.server 8000`) and open
`http://localhost:8000/js/examples/browser.html`. The Node proof is
[`examples/node.mjs`](examples/node.mjs); it copies a file to a new path as a
demonstration, without converting it.

## Source and sink contract

Both browser and Node sources expose:

```js
{
  size: bigint, // stable snapshot, unsigned 64-bit
  readAt(offset: bigint, length: number, signal?: AbortSignal): Promise<Uint8Array>
}
```

Each request is at most 1 MiB. The browser source awaits only
`blob.slice(start, end).arrayBuffer()` for that range; it rejects Blob sizes
above `Number.MAX_SAFE_INTEGER` because `Blob.slice()` takes Number offsets.
The Node source uses a caller-owned `FileHandle`, a BigInt size from
`stat({ bigint: true })`, and BigInt positioned reads. It never closes or
changes the handle's current position. A changed file may cause a short-read
error. A forward-only source needs a caller-managed, bounded temporary spool
before it can implement this contract; this proof rejects one otherwise.

Sinks expose `writeChunk(bytes: Uint8Array, signal?): Promise<number>` and
`flush(signal?): Promise<void>`. The proof awaits every write result before
requesting another read. The browser adapter accepts a caller-owned
`WritableStreamDefaultWriter`; the Node adapter accepts a caller-owned
`Writable`. Neither closes or ends the sink. Their `flush` is an I/O barrier,
not an `fsync` or document finalization operation.
The Node adapter catches errors associated with each awaited write; callers
remain responsible for later, unrelated stream errors after a write settles.

`copyRangeProof(instance, source, sink, options)` drives a single core future
through the raw WASM ABI. Rust requests a range, JavaScript awaits the
source, copies only that bounded chunk into WASM staging memory, and resumes
the Rust future. Rust then requests a write; JavaScript passes a bounded view
of staging memory and awaits the sink. The view stays valid until
`writeChunk()` resolves; a custom sink that retains bytes afterward must copy
them before resolving. The supplied Web and Node sinks make that copy. The
configured chunk defaults to 256 KiB and cannot exceed 1 MiB. Core and
staging each hold one chunk; the JavaScript source and supplied output sink
each hold at most one additional chunk while awaiting I/O. No complete
document buffer is created.
This proof uses the core's default 8 GiB input and 16 GiB output limits;
configurable JavaScript limits belong to the production API in issue #13.

One WASM instance handles one proof at a time. Use a separate instance for
concurrent proofs. `AbortSignal` is checked between awaited operations; an
already running Blob or file read cannot be interrupted by this adapter, and
bytes already accepted by a sink cannot be withdrawn. `copyRangeProof` always
resets the WASM state when it resolves or rejects. The production JavaScript
conversion API and multi-operation handles are later work.

This proof uses no npm or external Cargo dependencies. All project-owned
source here is MIT-licensed.

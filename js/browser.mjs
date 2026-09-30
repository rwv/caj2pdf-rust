// SPDX-License-Identifier: MIT

/** Browser entry point: the shared API plus an Origin Private File System spool. */
import { blobSource, Caj2PdfError, checkAbort, checkRange, convertSpooled, pumpChunks, requireChunkLength, requireSinkChunk, requireU64 } from "./io.mjs";
import { scratchCount, scratchSize } from "./internal/scratch.mjs";

export * from "./io.mjs";

/** Compile the packaged WASM module (or the one at `url`) once for reuse. */
export async function loadModule(url = new URL("./caj2pdf_wasm.wasm", import.meta.url)) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`could not fetch ${url}: HTTP ${response.status}`);
  }
  return WebAssembly.compileStreaming(response);
}

/**
 * Bounded scratch over a caller-owned OPFS FileSystemSyncAccessHandle, obtained
 * in a Dedicated Worker. Grant exclusive access and serialize calls. The caller
 * closes the handle and removes its file. No snapshots or image buffers are kept.
 */
export function syncAccessHandleScratch(handle, { maxBytes } = {}) {
  scratchSize(0n, maxBytes);
  if (["getSize", "truncate", "read", "write", "flush"].some((key) => typeof handle?.[key] !== "function")) {
    throw new TypeError("an OPFS synchronous access handle is required in a Dedicated Worker");
  }
  const initial = handle.getSize();
  if (!Number.isSafeInteger(initial) || initial < 0) throw new RangeError("invalid OPFS scratch size");
  let size = BigInt(initial);
  scratchSize(size, maxBytes);
  return Object.freeze({
    get size() { return size; },
    async resize(bytes, signal) {
      const length = scratchSize(bytes, maxBytes);
      checkAbort(signal);
      handle.truncate(length);
      size = bytes;
    },
    async readAt(offset, length, signal) {
      requireChunkLength(length, { allowZero: true });
      checkRange(size, offset, BigInt(length));
      checkAbort(signal);
      const bytes = new Uint8Array(length);
      const count = scratchCount(handle.read(bytes, { at: Number(offset) }), length);
      return bytes.subarray(0, count);
    },
    async writeAt(offset, bytes, signal) {
      requireSinkChunk(bytes);
      checkRange(size, offset, BigInt(bytes.byteLength));
      checkAbort(signal);
      return scratchCount(handle.write(bytes, { at: Number(offset) }), bytes.byteLength);
    },
    async flush(signal) {
      checkAbort(signal);
      handle.flush();
    },
  });
}

/**
 * Copy a forward-only `ReadableStream` into a uniquely named OPFS file,
 * rejecting beyond `maxBytes`, then expose it as a disk-backed Blob source.
 * `dispose()` removes the file; failures and aborts remove it at once.
 * Rejects with `RANDOM_ACCESS_REQUIRED` when OPFS or `createWritable()` is
 * unavailable, rather than buffering the stream in memory.
 */
export async function spoolToOpfs(stream, { maxBytes, signal, storage = globalThis.navigator?.storage } = {}) {
  requireU64(maxBytes, "maxBytes");
  if (typeof storage?.getDirectory !== "function") {
    throw new Caj2PdfError(
      "no durable temporary storage (OPFS) is available; pass a Blob/File or a readAt source instead",
      "RANDOM_ACCESS_REQUIRED",
    );
  }
  const root = await storage.getDirectory();
  const name = `caj2pdf-spool-${crypto.randomUUID()}`;
  const file = await root.getFileHandle(name, { create: true });
  let writable;
  const dispose = async () => {
    // A browser may briefly retain the writer lock after abort settles.
    // Bound retries to this specific lock error; surface permanent failures.
    for (let attempt = 0; ; attempt++) {
      try {
        await root.removeEntry(name);
        return;
      } catch (error) {
        if (error.name !== "NoModificationAllowedError" || attempt === 2) throw error;
        await new Promise((resolve) => setTimeout(resolve, attempt === 0 ? 10 : 50));
      }
    }
  };
  try {
    if (typeof file.createWritable !== "function") {
      throw new Caj2PdfError(
        "this browser cannot write OPFS files from this context; pass a Blob/File instead",
        "RANDOM_ACCESS_REQUIRED",
      );
    }
    writable = await file.createWritable();
    await pumpChunks(stream, (chunk) => writable.write(chunk), { maxBytes, signal });
    await writable.close();
    writable = undefined;
    return { source: blobSource(await file.getFile()), dispose };
  } catch (error) {
    await writable?.abort().catch(() => {});
    try {
      await dispose();
    } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], "OPFS spool failed and its temporary file could not be removed", { cause: error });
    }
    throw error;
  }
}

/**
 * Convert a forward-only `ReadableStream` through a bounded OPFS spool that
 * is removed on success, failure, or abort. `options.storage` overrides
 * `navigator.storage`.
 */
export function convertReadableStream(wasm, stream, sink, options = {}) {
  return convertSpooled(
    (input, spoolOptions) => spoolToOpfs(input, { ...spoolOptions, storage: options.storage }),
    wasm,
    stream,
    sink,
    options,
  );
}

/** Dedicated Worker only: run with four bounded OPFS stores and always dispose them. */
export async function withHnc8Scratch(operation, { maxBytes = 64n * 1024n * 1024n, storage = globalThis.navigator?.storage } = {}) {
  scratchSize(0n, maxBytes);
  const root = await storage.getDirectory();
  const directory = `caj2pdf-hnc8-${crypto.randomUUID()}`;
  const folder = await root.getDirectoryHandle(directory, { create: true });
  const handles = [];
  try {
    const scratch = [];
    for (let i = 0; i < 4; i++) {
      const file = await folder.getFileHandle(String(i), { create: true });
      const handle = await file.createSyncAccessHandle();
      handles.push(handle);
      scratch.push(syncAccessHandleScratch(handle, { maxBytes }));
    }
    return await operation(scratch);
  } finally {
    const closed = await Promise.allSettled(handles.map(async (handle) => handle.close()));
    await root.removeEntry(directory, { recursive: true });
    const errors = closed.filter((result) => result.status === "rejected").map((result) => result.reason);
    if (errors.length) throw new AggregateError(errors, "Could not close HN/C8 scratch handles");
  }
}

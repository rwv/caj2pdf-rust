// SPDX-License-Identifier: MIT

/** Node.js entry point: the shared API plus file, stream, and spool adapters. Requires Node 22+. */
import { mkdtemp, open, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  abortable,
  checkRange,
  convertSpooled,
  pumpChunks,
  requireChunkLength,
  requireSinkChunk,
  requireU64,
  TruncatedInputError,
} from "./io.mjs";

export * from "./io.mjs";

/** Compile the packaged WASM module (or the file at `url`) once for reuse. */
export async function loadModule(url = new URL("./caj2pdf_wasm.wasm", import.meta.url)) {
  return WebAssembly.compile(await readFile(url));
}

/** A positioned source over a caller-owned `node:fs/promises` FileHandle. */
export async function fileHandleSource(handle) {
  if (handle == null || typeof handle.stat !== "function" || typeof handle.read !== "function") {
    throw new TypeError("a readable FileHandle is required");
  }
  const stats = await handle.stat({ bigint: true });
  const size = requireU64(stats.size, "file size");
  return Object.freeze({
    size,
    async readAt(offset, length, signal) {
      requireChunkLength(length, { allowZero: true });
      checkRange(size, offset, BigInt(length));
      // A fresh buffer per read, so an abandoned (aborted) read owns its memory.
      const bytes = new Uint8Array(length);
      let done = 0;
      while (done < length) {
        const result = await abortable(handle.read({
          buffer: bytes,
          offset: done,
          length: length - done,
          position: offset + BigInt(done),
        }), signal);
        const count = result?.bytesRead;
        if (!Number.isSafeInteger(count) || count < 0 || count > length - done) {
          throw new RangeError("FileHandle returned an invalid read count");
        }
        if (count === 0) {
          throw new TruncatedInputError("file changed or ended before the requested range");
        }
        done += count;
      }
      return bytes;
    },
  });
}

/** Await each write callback before sending another bounded chunk. */
export function nodeWritableSink(writable) {
  if (writable == null || typeof writable.write !== "function") {
    throw new TypeError("a Node Writable stream is required");
  }
  return Object.freeze({
    async writeChunk(bytes, signal) {
      requireSinkChunk(bytes);
      // The stream can retain the bytes after WASM reuses its staging area.
      const owned = Buffer.from(bytes);
      await abortable(new Promise((resolve, reject) => {
        let pendingError;
        let cleanupScheduled = false;
        const scheduleCleanup = () => {
          if (cleanupScheduled) return;
          cleanupScheduled = true;
          // A Writable can invoke the callback before emitting its error
          // event on the next tick. Keep our listener through that event.
          setImmediate(() => {
            writable.off("error", onError);
            if (pendingError) reject(pendingError);
            else resolve();
          });
        };
        const onError = (error) => {
          pendingError = error;
          scheduleCleanup();
        };
        writable.on("error", onError);
        try {
          // Awaiting this callback is stricter than waiting for `drain` alone:
          // it keeps at most one supplied chunk in the Writable queue.
          writable.write(owned, (error) => {
            if (error) pendingError = error;
            scheduleCleanup();
          });
        } catch (error) {
          pendingError = error;
          scheduleCleanup();
        }
      }), signal);
      return bytes.byteLength;
    },
    async flush() {
      // The last write callback is the barrier. Ownership and finish/fsync
      // remain with the caller; this adapter never ends the stream.
    },
  });
}

/**
 * Copy a forward-only Node `Readable`, Web `ReadableStream`, or async
 * iterable into a private temporary file (mode 0600 in a fresh `mkdtemp`
 * directory under `os.tmpdir()`), rejecting beyond `maxBytes`. The returned
 * `dispose()` closes and removes it; failures and aborts remove it at once.
 */
export async function spoolToTempFile(stream, { maxBytes, signal, directory = tmpdir() } = {}) {
  requireU64(maxBytes, "maxBytes");
  const folder = await mkdtemp(join(directory, "caj2pdf-spool-"));
  let handle;
  const dispose = async () => {
    try {
      await handle?.close();
    } finally {
      await rm(folder, { recursive: true, force: true });
    }
  };
  try {
    handle = await open(join(folder, "input"), "wx+", 0o600);
    let position = 0;
    await pumpChunks(stream, async (chunk) => {
      let written = 0;
      while (written < chunk.byteLength) {
        const { bytesWritten } = await handle.write(chunk, written, chunk.byteLength - written, position);
        written += bytesWritten;
        position += bytesWritten;
      }
    }, { maxBytes, signal });
    const source = await fileHandleSource(handle);
    return { source, dispose, path: folder };
  } catch (error) {
    await dispose();
    throw error;
  }
}

/**
 * Convert a forward-only stream through a bounded temporary file that is
 * removed on success, failure, or abort. `maxSpoolBytes` defaults to
 * `limits.maxInputBytes` (8 GiB unless lowered).
 */
export function convertReadable(wasm, stream, sink, options = {}) {
  return convertSpooled(
    (input, spoolOptions) => spoolToTempFile(input, { ...spoolOptions, directory: options.tempDirectory }),
    wasm,
    stream,
    sink,
    options,
  );
}

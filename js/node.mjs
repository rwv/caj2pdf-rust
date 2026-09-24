// SPDX-License-Identifier: MIT

/** Node.js adapters using caller-owned handles and streams. Requires Node 22+. */
import {
  checkAbort,
  checkRange,
  requireChunkLength,
  requireU64,
  TruncatedInputError,
} from "./io.mjs";

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
      checkAbort(signal);
      const bytes = new Uint8Array(length);
      let done = 0;
      while (done < length) {
        const result = await handle.read({
          buffer: bytes,
          offset: done,
          length: length - done,
          position: offset + BigInt(done),
        });
        checkAbort(signal);
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
      if (!(bytes instanceof Uint8Array)) {
        throw new TypeError("writeChunk requires Uint8Array");
      }
      requireChunkLength(bytes.byteLength, { allowZero: true });
      checkAbort(signal);
      // The stream can retain the bytes after WASM reuses its staging area.
      const owned = Buffer.from(bytes);
      await new Promise((resolve, reject) => {
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
      });
      checkAbort(signal);
      return bytes.byteLength;
    },
    async flush(signal) {
      // The last write callback is the barrier. Ownership and finish/fsync
      // remain with the caller; this adapter never ends the stream.
      checkAbort(signal);
    },
  });
}

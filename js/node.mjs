// SPDX-License-Identifier: MIT

/**
 * Node.js entry point: the shared API plus file, stream, and spool adapters.
 * Each operation runs in a `node:worker_threads` Worker that reads a file
 * path or descriptor directly; a Blob is read on the calling thread and
 * handed to the Worker through shared memory. Requires Node 22+.
 */
import { fstat } from "node:fs";
import { mkdtemp, open, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { Worker } from "node:worker_threads";
import { abortable, pumpChunks, requireSinkChunk, requireU64 } from "./io.mjs";
import { writeSpoolChunk } from "./internal/spool-write.mjs";
import { convertSpooledWith, runOperation } from "./internal/run.mjs";

export * from "./io.mjs";

const fstatAsync = promisify(fstat);

/** Compile the packaged WASM module (or the file at `url`) once for reuse. */
export async function loadModule(url = new URL("./caj2pdf_wasm.wasm", import.meta.url)) {
  return WebAssembly.compile(await readFile(url));
}

function size(stats) {
  if (!stats.isFile()) throw new TypeError("the input must be a regular file");
  return Number(requireU64(stats.size, "file size"));
}

/**
 * The caller's Node options minus those that name an entry point, which a
 * Worker started from a file rejects (`node --input-type=module --eval`).
 */
function workerExecArgv() {
  const options = [];
  const argv = process.execArgv;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    if (["--eval", "-e", "--print", "-p", "--input-type"].includes(option)) {
      index++;
    } else if (!/^--(eval|print|input-type)=/.test(option)) {
      options.push(option);
    }
  }
  return options;
}

const platform = Object.freeze({
  async createWorker() {
    const worker = new Worker(new URL("./internal/worker.mjs", import.meta.url), { execArgv: workerExecArgv() });
    return {
      post: (message) => worker.postMessage(message),
      onMessage: (listener) => worker.on("message", listener),
      onError: (listener) => worker.on("error", listener),
      terminate: () => {
        worker.terminate();
      },
    };
  },
  async describe(input, role) {
    if (typeof input === "string" || input instanceof URL) {
      const path = input instanceof URL ? fileURLToPath(input) : input;
      return { kind: "path", path, size: size(await stat(path, { bigint: true })) };
    }
    if (Number.isSafeInteger(input) && input >= 0) {
      return { kind: "fd", fd: input, size: size(await fstatAsync(input, { bigint: true })) };
    }
    if (input instanceof Blob) {
      return { kind: "served", blob: input, size: input.size };
    }
    throw new TypeError(`${role} must be a file path, a file descriptor, or a Blob`);
  },
  async read(input, offset, length) {
    const buffer = await input.blob.slice(offset, offset + length).arrayBuffer();
    return new Uint8Array(buffer);
  },
});

/**
 * Convert PDF, CAJ, KDH or HN/C8 to PDF. `source` is a file path (string
 * or `file:` URL), an open file descriptor, or a Blob; the format is detected
 * from its leading signature unless `format` is set.
 */
export function convert(wasm, source, sink, options = {}) {
  return runOperation(platform, "convert", wasm, source, sink, options);
}

/** Read format, pages and validated CAJ/HN-A bookmark counts. No image decoding. */
export function inspect(wasm, source, options = {}) {
  return runOperation(platform, "inspect", wasm, source, null, options);
}

/** Spool with `spool`, convert, and always dispose the spool. */
export function convertSpooled(spool, wasm, stream, sink, options = {}) {
  return convertSpooledWith(convert, spool, wasm, stream, sink, options);
}

/** Await each write callback before sending another chunk. */
export function nodeWritableSink(writable) {
  if (writable == null || typeof writable.write !== "function") {
    throw new TypeError("a Node Writable stream is required");
  }
  return Object.freeze({
    async writeChunk(bytes, signal) {
      requireSinkChunk(bytes);
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
          writable.write(bytes, (error) => {
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
 * `source` is the file's path; `dispose()` removes it, and failures and
 * aborts remove it at once.
 */
export async function spoolToTempFile(stream, { maxBytes, signal, directory = tmpdir() } = {}) {
  requireU64(maxBytes, "maxBytes");
  const folder = await mkdtemp(join(directory, "caj2pdf-spool-"));
  const path = join(folder, "input");
  let handle;
  const dispose = async () => {
    try {
      await handle?.close();
    } finally {
      handle = undefined;
      await rm(folder, { recursive: true, force: true });
    }
  };
  try {
    handle = await open(path, "wx", 0o600);
    let position = 0;
    await pumpChunks(stream, async (chunk) => {
      position = await writeSpoolChunk(handle, chunk, position, signal);
    }, { maxBytes, signal });
    await handle.close();
    handle = undefined;
    return { source: path, dispose, path: folder };
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

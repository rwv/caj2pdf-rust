// SPDX-License-Identifier: MIT

/** Browser entry point: the shared API plus an Origin Private File System spool. */
import { blobSource, Caj2PdfError, convertSpooled, pumpChunks, requireU64 } from "./io.mjs";

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
  const dispose = () => root.removeEntry(name);
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
    await dispose().catch(() => {});
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

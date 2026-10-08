// SPDX-License-Identifier: MIT

/**
 * Browser-side cases for `browser.test.mjs`. This module runs inside
 * headless Chromium, imports the public browser entry point, and returns
 * JSON-serializable results to the Node test through the DevTools Protocol.
 * Inputs are served by the test at `/fixtures/<name>`.
 */
import {
  convert,
  convertReadableStream,
  inspect,
  loadModule,
  spoolToOpfs,
  webWritableSink,
} from "../browser.mjs";

// The default URL resolves `caj2pdf_wasm.wasm` next to `browser.mjs`.
const modulePromise = loadModule();
const CHUNK = 4096;

async function input(name) {
  const response = await fetch(`/fixtures/${name}`);
  if (!response.ok) throw new Error(`fixture ${name}: HTTP ${response.status}`);
  return new File([await response.blob()], name);
}

/** A real WritableStream that records its chunks and the largest write. */
function collector() {
  const chunks = [];
  let maxWrite = 0;
  const writer = new WritableStream({
    write(chunk) {
      maxWrite = Math.max(maxWrite, chunk.byteLength);
      chunks.push(chunk);
    },
  }).getWriter();
  return { writer, chunks, maxWrite: () => maxWrite };
}

async function encode(chunks) {
  const bytes = await new Blob(chunks).arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  let binary = "";
  const view = new Uint8Array(bytes);
  for (let index = 0; index < view.length; index += 0x8000) {
    binary += String.fromCharCode(...view.subarray(index, index + 0x8000));
  }
  return {
    length: view.length,
    sha256: [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join(""),
    base64: btoa(binary),
  };
}

function plainReport(report) {
  return Object.fromEntries(
    Object.entries(report).map(([key, value]) => [key, typeof value === "bigint" ? value.toString() : value]),
  );
}

function plainError(error) {
  return { name: error?.name, code: error?.code, format: error?.format, message: String(error?.message), errors: error instanceof AggregateError ? error.errors.map(plainError) : undefined };
}

async function settle(promise) {
  try {
    return { value: await promise };
  } catch (error) {
    return { error: plainError(error) };
  }
}

async function opfsEntries() {
  const root = await navigator.storage.getDirectory();
  const names = [];
  for await (const name of root.keys()) names.push(name);
  return names.filter((name) => name.startsWith("caj2pdf-spool-")).sort();
}

/** File source to WritableStream sink, with bounded-chunk and progress evidence. */
export async function convertFile(name) {
  const sink = collector();
  const progress = [];
  const report = await convert(await modulePromise, await input(name), webWritableSink(sink.writer), {
    chunkSize: CHUNK,
    progress: (fraction) => progress.push(fraction),
  });
  await sink.writer.close();
  const inspected = await inspect(await modulePromise, await input(name));
  return {
    report: plainReport(report),
    pageCount: inspected.pageCount,
    progress,
    maxWrite: sink.maxWrite(),
    output: await encode(sink.chunks),
  };
}

/** An OPFS file handle as the source; the conversion Worker opens it. */
export async function convertOpfsHandle(name) {
  const root = await navigator.storage.getDirectory();
  const entry = `caj2pdf-spool-handle-${crypto.randomUUID()}`;
  const handle = await root.getFileHandle(entry, { create: true });
  const sink = collector();
  let report;
  try {
    const writable = await handle.createWritable();
    await writable.write(await input(name));
    await writable.close();
    report = await convert(await modulePromise, handle, webWritableSink(sink.writer), { chunkSize: CHUNK });
    await sink.writer.close();
  } finally {
    await root.removeEntry(entry);
  }
  return { report: plainReport(report), after: await opfsEntries(), output: await encode(sink.chunks) };
}

/** A recognized but unsupported input must reject with its format. */
export async function reject(name) {
  const sink = collector();
  const result = await settle(convert(await modulePromise, await input(name), webWritableSink(sink.writer)));
  return { ...result, written: sink.chunks.length };
}

/** Descriptor metadata and the typed refusal through the same File adapter. */
export async function inspectDescriptor(name) {
  const info = await inspect(await modulePromise, await input(name));
  return { info: plainReport(info), ...await reject(name) };
}

/** A parser failure after spooling must release its stream and OPFS file. */
export async function rejectSpooled(name) {
  const sink = collector();
  const stream = (await input(name)).stream();
  const result = await settle(convertReadableStream(await modulePromise, stream, webWritableSink(sink.writer)));
  await sink.writer.close();
  return { ...result, written: sink.chunks.length, after: await opfsEntries(), unlocked: readerReleased(stream) };
}

/**
 * Abort while the real WritableStream applies backpressure (its first write
 * never settles); the conversion must reject promptly with the abort reason.
 */
export async function abortBackpressure(name) {
  const controller = new AbortController();
  let writes = 0;
  const writer = new WritableStream({
    write() {
      writes += 1;
      setTimeout(() => controller.abort(), 0);
      return new Promise(() => {});
    },
  }, { highWaterMark: 1 }).getWriter();
  const result = await settle(
    convert(await modulePromise, await input(name), webWritableSink(writer), {
      chunkSize: 256,
      signal: controller.signal,
    }),
  );
  return { ...result, writes };
}

/**
 * Spool a forward-only ReadableStream through the real OPFS, recording the
 * spool entries seen during the first write and after the conversion.
 */
export async function opfsConvert(name) {
  const sink = collector();
  let during;
  const inner = webWritableSink(sink.writer);
  const watching = {
    async writeChunk(bytes, signal) {
      during ??= await opfsEntries();
      return inner.writeChunk(bytes, signal);
    },
    flush: inner.flush,
  };
  const stream = (await input(name)).stream();
  const report = await convertReadableStream(await modulePromise, stream, watching, {
    chunkSize: CHUNK,
  });
  await sink.writer.close();
  return { report: plainReport(report), during, after: await opfsEntries(), unlocked: readerReleased(stream),
    output: await encode(sink.chunks) };
}

function readerReleased(stream) {
  if (stream.locked) return false;
  const reader = stream.getReader();
  reader.closed.catch(() => {});
  reader.releaseLock();
  return !stream.locked;
}

/** Bound exceeded, failed conversion, and abort all remove the OPFS spool. */
export async function opfsFailures(name) {
  const module = await modulePromise;
  const boundedStream = (await input(name)).stream();
  const bounded = await settle(
    convertReadableStream(module, boundedStream, webWritableSink(collector().writer), {
      maxSpoolBytes: 500,
    }),
  );
  const afterBound = await opfsEntries();
  const lowLevelStream = (await input(name)).stream();
  const lowLevel = await settle(spoolToOpfs(lowLevelStream, { maxBytes: 10n }));
  const afterLowLevel = await opfsEntries();
  const unsupportedStream = new Blob([Uint8Array.of(1, 2, 3)]).stream();
  const unsupported = await settle(convertReadableStream(module, unsupportedStream, webWritableSink(collector().writer)));
  const afterUnsupported = await opfsEntries();
  const controller = new AbortController();
  const aborting = {
    async writeChunk() {
      controller.abort();
      return 0;
    },
    async flush() {},
  };
  const abortedStream = (await input(name)).stream();
  const aborted = await settle(convertReadableStream(module, abortedStream, aborting, { signal: controller.signal }));
  const afterAbort = await opfsEntries();
  return { bounded, afterBound, lowLevel, afterLowLevel, unsupported, afterUnsupported, aborted, afterAbort,
    unlocked: [boundedStream, lowLevelStream, unsupportedStream, abortedStream].map(readerReleased) };
}

/** Run a case module in a real Dedicated Worker that calls the API itself. */
async function runWorker(path) {
  const worker = new Worker(path, { type: "module" });
  let timeout;
  try {
    return await new Promise((resolve, reject) => {
      timeout = setTimeout(() => reject(new Error(`${path} timed out`)), 15_000);
      worker.onmessage = ({ data }) => data.error ? reject(new Error(data.error)) : resolve(data);
      worker.onerror = (event) => reject(new Error(event.message));
    });
  } finally {
    clearTimeout(timeout);
    worker.terminate();
  }
}

export const cajRecoveryInWorker = () => runWorker("/test/caj-recovery-worker.mjs");
export const hnc8InWorker = () => runWorker("/test/hnc8-worker.mjs");

export async function inspectHnc8() {
  const { syntheticHn, unknownOutline } = await import("./hnc8-fixtures.mjs");
  const result = [];
  // Page 9 of a one-page document: that bookmark is skipped with a warning.
  const skipped = syntheticHn(true);
  skipped[0x15c + 308 + 280] = 57;
  for (const bytes of [syntheticHn(true), skipped, unknownOutline("c8"), unknownOutline("hn")]) {
    const info = await inspect(await modulePromise, new Blob([bytes]), { chunkSize: 3 });
    result.push({ format: info.format, pages: info.pageCount, bookmarks: info.bookmarkCount, warnings: info.outlineWarnings });
  }
  return result;
}

export async function convertDamaged(name) {
  const sink = collector();
  const report = await convert(await modulePromise, await input(name), webWritableSink(sink.writer), { allowDamaged: true, chunkSize: CHUNK });
  await sink.writer.close();
  return { omittedPages: report.omittedPages.map((page) => ({ pageIndex: page.pageIndex, offset: page.offset.toString() })), output: await encode(sink.chunks) };
}

/** Force abort after the Worker checks CANCEL but before it starts waiting. */
export async function abortBeforeAckWait(name) {
  const module = await modulePromise;
  const stream = (await input(name)).stream();
  const controller = new AbortController();
  const RealWorker = globalThis.Worker;
  const waits = [];
  let pauses = 0;
  globalThis.Worker = class extends RealWorker {
    constructor(url, options) {
      super(new URL("./abort-wait-worker.mjs", import.meta.url), options);
      this.addEventListener("message", (event) => {
        if (event.data.type === "test-before-ack-wait") {
          event.stopImmediatePropagation();
          pauses++;
          controller.abort();
          const gate = new Int32Array(event.data.gate);
          Atomics.store(gate, 0, 1);
          Atomics.notify(gate, 0);
        } else if (event.data.type === "test-after-ack-wait") {
          event.stopImmediatePropagation();
          waits.push(event.data.result);
        }
      });
    }
  };
  try {
    const result = await settle(convertReadableStream(module, stream, {
      // No write ACK may accidentally rescue the otherwise lost notification.
      writeChunk() { return new Promise(() => {}); },
      async flush() {},
    }, { signal: controller.signal }));
    return { ...result, pauses, waits, after: await opfsEntries(), unlocked: readerReleased(stream) };
  } finally {
    globalThis.Worker = RealWorker;
  }
}

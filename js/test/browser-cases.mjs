// SPDX-License-Identifier: MIT

/**
 * Browser-side cases for `browser.test.mjs`. This module runs inside
 * headless Chromium, imports the public browser entry point, and returns
 * JSON-serializable results to the Node test through the DevTools Protocol.
 * Inputs are served by the test at `/fixtures/<name>`.
 */
import {
  blobSource,
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

/** A File whose slices are recorded; whole-file reads are forbidden. */
function tracked(file, record) {
  return {
    size: file.size,
    slice(start, end) {
      record.maxRead = Math.max(record.maxRead, end - start);
      return file.slice(start, end);
    },
    arrayBuffer() {
      throw new Error("whole File.arrayBuffer() is forbidden");
    },
  };
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
  return { name: error?.name, code: error?.code, format: error?.format, message: String(error?.message) };
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

/** File source to WritableStream sink, with bounded-chunk evidence. */
export async function convertFile(name) {
  const record = { maxRead: 0 };
  const sink = collector();
  const source = blobSource(tracked(await input(name), record));
  const report = await convert(await modulePromise, source, webWritableSink(sink.writer), { chunkSize: CHUNK });
  await sink.writer.close();
  const inspected = await inspect(await modulePromise, blobSource(await input(name)));
  return {
    report: plainReport(report),
    pageCount: inspected.pageCount,
    maxRead: record.maxRead,
    maxWrite: sink.maxWrite(),
    output: await encode(sink.chunks),
  };
}

/** A recognized but unsupported input must reject with its format. */
export async function reject(name) {
  const sink = collector();
  const result = await settle(convert(await modulePromise, blobSource(await input(name)), webWritableSink(sink.writer)));
  return { ...result, written: sink.chunks.length };
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
    convert(await modulePromise, blobSource(await input(name)), webWritableSink(writer), {
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
  const report = await convertReadableStream(await modulePromise, (await input(name)).stream(), watching, {
    chunkSize: CHUNK,
  });
  await sink.writer.close();
  return { report: plainReport(report), during, after: await opfsEntries(), output: await encode(sink.chunks) };
}

/** Bound exceeded, failed conversion, and abort all remove the OPFS spool. */
export async function opfsFailures(name) {
  const module = await modulePromise;
  const bounded = await settle(
    convertReadableStream(module, (await input(name)).stream(), webWritableSink(collector().writer), {
      maxSpoolBytes: 500,
    }),
  );
  const afterBound = await opfsEntries();
  const lowLevel = await settle(spoolToOpfs((await input(name)).stream(), { maxBytes: 10n }));
  const afterLowLevel = await opfsEntries();
  const unsupported = await settle(
    convertReadableStream(module, new Blob([Uint8Array.of(1, 2, 3)]).stream(), webWritableSink(collector().writer)),
  );
  const afterUnsupported = await opfsEntries();
  const controller = new AbortController();
  const aborting = {
    async writeChunk() {
      controller.abort();
      return 0;
    },
    async flush() {},
  };
  const aborted = await settle(
    convertReadableStream(module, (await input(name)).stream(), aborting, { signal: controller.signal }),
  );
  const afterAbort = await opfsEntries();
  return { bounded, afterBound, lowLevel, afterLowLevel, unsupported, afterUnsupported, aborted, afterAbort };
}

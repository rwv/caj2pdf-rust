// SPDX-License-Identifier: MIT

// Small/large PDF measurements for Node and real Chromium; run after building WASM.
// Each row converts once through the public API (a Worker) for the output,
// write sizes and spool size, and once through the raw ABI on the calling
// thread, where WASM linear memory and the requested read sizes are visible.
import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import * as api from "../node.mjs";
import { largePdfBlob, wasmUrl } from "../test/helpers.mjs";
import { findChrome, launchChrome, openPage, startServer } from "../test/browser-harness.mjs";

/**
 * Convert `bytes` through the raw ABI with default limits; returns WASM
 * memory before and after, the largest read requested and the output size.
 * Serialized into the browser page, so it uses no outside bindings.
 */
async function rawMeasure(module, bytes, limits) {
  let memory;
  let maxRead = 0;
  let output = 0;
  const imports = {
    caj2pdf_read(resource, offset, pointer, length) {
      maxRead = Math.max(maxRead, length);
      const chunk = bytes.subarray(offset, offset + length);
      new Uint8Array(memory.buffer, pointer, length).set(chunk);
      return chunk.length;
    },
    caj2pdf_write(pointer, length) {
      output += length;
      return length;
    },
    caj2pdf_flush: () => 0,
    caj2pdf_progress() {},
    caj2pdf_cancelled: () => 0,
  };
  const { exports } = await WebAssembly.instantiate(module, { caj2pdf: imports });
  memory = exports.memory;
  const before = memory.buffer.byteLength;
  const status = exports.caj2pdf_convert(BigInt(bytes.length), 256 * 1024, 0, 1, ...limits);
  if (status !== 0) throw new Error(`raw conversion failed with status ${status}`);
  return { before, peak: memory.buffer.byteLength, maxRead, rawOutput: output };
}

const LIMITS = [
  api.DEFAULT_LIMITS.maxInputBytes,
  api.DEFAULT_LIMITS.maxOutputBytes,
  api.DEFAULT_LIMITS.maxAllocationBytes,
  api.DEFAULT_LIMITS.maxPages,
  api.DEFAULT_LIMITS.maxBookmarks,
];

const chrome = findChrome();
assert.ok(chrome, "Chromium is required; set CAJ2PDF_CHROME");
const inputs = [largePdfBlob(64 * 1024), largePdfBlob(24 * 1024 * 1024)];
const module = await api.loadModule(wasmUrl);
const results = [];
for (const blob of inputs) {
  const raw = await rawMeasure(module, new Uint8Array(await blob.arrayBuffer()), LIMITS);
  for (const forward of [false, true]) {
    const spool = forward ? await api.spoolToTempFile(blob.stream(), { maxBytes: BigInt(blob.size) }) : undefined;
    let maxWrite = 0;
    try {
      const tempBytes = spool ? (await stat(join(spool.path, "input"))).size : 0;
      const report = await api.convert(module, spool?.source ?? blob, {
        async writeChunk(bytes) { maxWrite = Math.max(maxWrite, bytes.length); return bytes.length; },
        async flush() {},
      });
      results.push({ target: "Node", input: blob.size, forward, ...raw, maxWrite, tempBytes, pages: report.pagesConverted, output: String(report.outputBytesWritten) });
    } finally {
      await spool?.dispose();
    }
    if (spool) {
      try { await readdir(spool.path); throw new Error("spool remains"); } catch (error) { if (error.code !== "ENOENT") throw error; }
    }
  }
}
const server = await startServer(fileURLToPath(new URL("..", import.meta.url)), {
  "/index.html": "<!doctype html><title>memory measurement</title>",
  "/small.pdf": new Uint8Array(await inputs[0].arrayBuffer()),
  "/large.pdf": new Uint8Array(await inputs[1].arrayBuffer()),
  "/caj2pdf_wasm.wasm": await readFile(wasmUrl),
});
let browser;
let browserVersion;
try {
  browser = await launchChrome(chrome);
  const page = await openPage(browser.cdp, `${server.origin}/index.html`);
  browserVersion = await page.evaluate("navigator.userAgent");
  results.push(...await page.evaluate(`(async () => {
    const rawMeasure = ${rawMeasure.toString()};
    const limits = [${LIMITS.map((limit) => `${limit}n`).join(", ")}].map((limit, index) => index < 3 ? limit : Number(limit));
    const api = await import("/browser.mjs");
    const module = await api.loadModule();
    const results = [];
    for (const name of ["small", "large"]) {
      const blob = await (await fetch("/" + name + ".pdf")).blob();
      const raw = await rawMeasure(module, new Uint8Array(await blob.arrayBuffer()), limits);
      for (const forward of [false, true]) {
        const response = await fetch("/" + name + ".pdf");
        const spool = forward ? await api.spoolToOpfs(response.body, {maxBytes: 32n * 1024n * 1024n}) : undefined;
        const source = spool?.source ?? await response.blob();
        const root = await navigator.storage.getDirectory();
        let tempBytes = 0, maxWrite = 0;
        for await (const handle of root.values()) tempBytes += (await handle.getFile()).size;
        try {
          const report = await api.convert(module, source, {async writeChunk(bytes) {maxWrite = Math.max(maxWrite, bytes.length); return bytes.length;}, async flush() {}});
          results.push({target: "Chromium", input: blob.size, forward, ...raw, maxWrite, tempBytes, pages: report.pagesConverted, output: String(report.outputBytesWritten)});
        } finally { await spool?.dispose(); }
        if ((await Array.fromAsync(root.keys())).length) throw new Error("spool remains");
      }
    }
    return results;
  })()`));
} finally { await browser?.close(); await server.close(); }
for (const row of results) {
  assert.equal(row.pages, 1);
  assert.equal(row.output, String(row.input));
  assert.equal(row.rawOutput, row.input);
  assert.ok(row.maxRead <= api.DEFAULT_IO_CHUNK && row.maxWrite <= api.DEFAULT_IO_CHUNK);
  assert.ok(row.peak - row.before < 4 * 1024 * 1024, "WASM growth exceeded the PDF fixture budget");
  assert.equal(row.tempBytes, row.forward ? row.input : 0);
}
console.log(JSON.stringify({node: process.version, browser: browserVersion, results}, null, 2));

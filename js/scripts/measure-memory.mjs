// SPDX-License-Identifier: MIT

// Small/large PDF measurements for Node and real Chromium; run after building WASM.
import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import * as api from "../node.mjs";
import { largePdfBlob, newInstance, wasmUrl } from "../test/helpers.mjs";
import { findChrome, launchChrome, openPage, startServer } from "../test/browser-harness.mjs";

const chrome = findChrome();
assert.ok(chrome, "Chromium is required; set CAJ2PDF_CHROME");
const inputs = [largePdfBlob(64 * 1024), largePdfBlob(24 * 1024 * 1024)];
const results = [];
for (const blob of inputs) {
  for (const forward of [false, true]) {
    const instance = await newInstance();
    const before = instance.exports.memory.buffer.byteLength;
    const spool = forward ? await api.spoolToTempFile(blob.stream(), { maxBytes: BigInt(blob.size) }) : undefined;
    const source = spool?.source ?? api.blobSource(blob);
    let maxRead = 0, maxWrite = 0;
    try {
      const tempBytes = spool ? (await stat(join(spool.path, "input"))).size : 0;
      const report = await api.convert(instance, {
        size: source.size,
        readAt(offset, length, signal) {
          maxRead = Math.max(maxRead, length);
          return source.readAt(offset, length, signal);
        },
      }, { async writeChunk(bytes) { maxWrite = Math.max(maxWrite, bytes.length); return bytes.length; }, async flush() {} });
      results.push({ target: "Node", input: blob.size, forward, before, peak: instance.exports.memory.buffer.byteLength, maxRead, maxWrite, tempBytes, pages: report.pagesConverted, output: String(report.outputBytesWritten) });
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
    const api = await import("/browser.mjs");
    const module = await api.loadModule();
    const results = [];
    for (const name of ["small", "large"]) {
      for (const forward of [false, true]) {
        const instance = await WebAssembly.instantiate(module);
        const before = instance.exports.memory.buffer.byteLength;
        const response = await fetch("/" + name + ".pdf");
        const spool = forward ? await api.spoolToOpfs(response.body, {maxBytes: 32n * 1024n * 1024n}) : undefined;
        const source = spool?.source ?? api.blobSource(await response.blob());
        const root = await navigator.storage.getDirectory();
        let tempBytes = 0, maxRead = 0, maxWrite = 0;
        for await (const handle of root.values()) tempBytes += (await handle.getFile()).size;
        try {
          const report = await api.convert(instance, {
            size: source.size,
            readAt(offset, length, signal) { maxRead = Math.max(maxRead, length); return source.readAt(offset, length, signal); },
          }, {async writeChunk(bytes) {maxWrite = Math.max(maxWrite, bytes.length); return bytes.length;}, async flush() {}});
          results.push({target: "Chromium", input: Number(source.size), forward, before, peak: instance.exports.memory.buffer.byteLength, maxRead, maxWrite, tempBytes, pages: report.pagesConverted, output: String(report.outputBytesWritten)});
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
  assert.ok(row.maxRead <= api.DEFAULT_IO_CHUNK && row.maxWrite <= api.DEFAULT_IO_CHUNK);
  assert.ok(row.peak - row.before < 4 * 1024 * 1024, "WASM growth exceeded the PDF fixture budget");
  assert.equal(row.tempBytes, row.forward ? row.input : 0);
}
console.log(JSON.stringify({node: process.version, browser: browserVersion, results}, null, 2));

// SPDX-License-Identifier: MIT
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { convert, inspect } from "../node.mjs";
import { wasmModule, validatePdf } from "./helpers.mjs";
import { findChrome, launchChrome, openPage, startServer } from "./browser-harness.mjs";

const source = await readFile(new URL("../../crates/caj2pdf-core/tests/fixtures/ttkn/authored.pdf", import.meta.url));
const response = (await readFile(new URL("../../crates/caj2pdf-core/tests/fixtures/ttkn/response.txt", import.meta.url), "utf8")).trim();
function sink() {
  const chunks = [];
  return { chunks, aborted: 0, flushed: 0,
    writeChunk(bytes) { chunks.push(Buffer.from(bytes)); return bytes.length; },
    flush() { this.flushed++; },
    abort() { this.aborted++; chunks.length = 0; },
  };
}

test("TTKN: explicit response decrypts strings and streams in a Node worker", async (t) => {
  const output = sink();
  const report = await convert(await wasmModule(), new Blob([source]), output, { ttknResponse: response, chunkSize: 17 });
  assert.equal(report.pagesConverted, 1);
  assert.equal(output.flushed, 1);
  const pdf = Buffer.concat(output.chunks);
  assert.ok(pdf.includes(Buffer.from("FEFF004100750074")));
  assert.ok(!pdf.includes(Buffer.from("/Encrypt")));
  await validatePdf(t, pdf, 1);
  for (const ttknResponse of [undefined, "0".repeat(32), response.toUpperCase()]) {
    const failed = sink();
    await assert.rejects(convert(await wasmModule(), new Blob([source]), failed, { ttknResponse }));
    assert.equal(failed.flushed, 0);
    assert.equal(failed.aborted, 0);
    assert.equal(failed.chunks.length, 0);
  }
  for (const ttknResponse of [null, 1, "bad", "z".repeat(32)]) {
    await assert.rejects(convert(await wasmModule(), new Blob([source]), sink(), { ttknResponse }), /ttknResponse must contain exactly/);
  }
  await assert.rejects(inspect(await wasmModule(), new Blob([source]), { ttknResponse: response }), /only supported for conversion/);
});

test("TTKN: Chromium matches Node and refuses incorrect responses before writing", async (t) => {
  const chrome = findChrome();
  if (!chrome) {
    assert.ok(!process.env.CI, "Chromium is required in CI");
    t.skip("Chromium unavailable");
    return;
  }
  const server = await startServer(fileURLToPath(new URL("../", import.meta.url)), {
    "/index.html": "<!doctype html><title>Original TTKN control</title>",
    "/input.pdf": source,
  }, { isolated: true });
  const browser = await launchChrome(chrome);
  try {
    const page = await openPage(browser.cdp, server.origin + "/index.html");
    const result = await page.evaluate(`(async () => {
      const {convert, loadModule} = await import('/browser.mjs');
      const wasm = await loadModule('/caj2pdf_wasm.wasm');
      const blob = await (await fetch('/input.pdf')).blob();
      const chunks = []; let aborted = 0;
      const report = await convert(wasm, blob, {writeChunk(b) {chunks.push(...b); return b.length;}, flush() {}}, {ttknResponse: ${JSON.stringify(response)}, chunkSize: 4096});
      let rejected = false;
      try { await convert(wasm, blob, {writeChunk() {}, flush() {}, abort() {aborted++;}}, {ttknResponse: '0'.repeat(32)}); }
      catch { rejected = true; }
      return {bytes: chunks, pages: report.pagesConverted, rejected, aborted};
    })()`);
    const node = sink();
    await convert(await wasmModule(), new Blob([source]), node, { ttknResponse: response });
    assert.deepEqual(Buffer.from(result.bytes), Buffer.concat(node.chunks));
    assert.equal(result.pages, 1);
    assert.equal(result.rejected, true);
    assert.equal(result.aborted, 0);
    assert.deepEqual(page.errors, []);
  } finally {
    await browser.close();
    await server.close();
  }
});

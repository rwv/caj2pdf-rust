// SPDX-License-Identifier: MIT

// Real-browser integration tests: the public browser entry point runs in
// headless Chromium over the Chrome DevTools Protocol, served from
// http://127.0.0.1 (a secure context, so the real OPFS is available).
// Outputs come back to Node for qpdf validation. Without Chromium the tests
// skip locally and fail under CI; set CAJ2PDF_CHROME to choose a browser.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { findChrome, launchChrome, openPage, startServer } from "./browser-harness.mjs";
import { fixture, syntheticCaj, syntheticKdh, validatePdf, wasmUrl } from "./helpers.mjs";

const chrome = findChrome();
if (chrome == null && process.env.CI) {
  throw new Error("Chromium is required in CI: set CAJ2PDF_CHROME or install google-chrome");
}
const skip = chrome == null && "no Chromium found; set CAJ2PDF_CHROME to run the real-browser tests";

let server;
let browser;
let page;

before(async () => {
  if (skip) return;
  const fixtures = {
    "/fixtures/input.caj": syntheticCaj(),
    "/fixtures/input.kdh": (await syntheticKdh()).wrapped,
    "/fixtures/input.pdf": await fixture("valid_nested_outline.pdf"),
    "/fixtures/input.hn": await fixture("truncated_hn.hn"),
    "/fixtures/input.c8": await fixture("truncated_c8.c8"),
    "/js/caj2pdf_wasm.wasm": await readFile(wasmUrl),
    "/index.html": "<!doctype html><meta charset=utf-8><title>caj2pdf browser tests</title>",
  };
  server = await startServer(fileURLToPath(new URL("../../", import.meta.url)), fixtures);
  browser = await launchChrome(chrome);
  page = await openPage(browser.cdp, `${server.origin}/index.html`);
}, { timeout: 90_000 });

after(async () => {
  await browser?.close();
  await server?.close();
});

/** Run an exported case from `browser-cases.mjs` in the page. */
function run(name, ...args) {
  return page.evaluate(
    `import("/js/test/browser-cases.mjs").then((cases) => cases[${JSON.stringify(name)}](...${JSON.stringify(args)}))`,
  );
}

function decode(output) {
  const bytes = new Uint8Array(Buffer.from(output.base64, "base64"));
  assert.equal(bytes.length, output.length);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), output.sha256);
  return bytes;
}

const options = { skip, timeout: 60_000 };

test("Chromium: File sources and WritableStream sinks convert CAJ, KDH, and PDF", options, async (t) => {
  for (const [name, format, pages, bookmarks] of [
    ["input.caj", "caj", 2, 1],
    ["input.kdh", "kdh", 2, 0],
    ["input.pdf", "pdf", 2, 0],
  ]) {
    await t.test(format, async (t) => {
      const result = await run("convertFile", name);
      assert.equal(result.report.format, format);
      assert.equal(result.report.pagesConverted, pages);
      assert.equal(result.report.bookmarksWritten, bookmarks);
      assert.equal(result.pageCount, pages);
      assert.equal(result.report.outputBytesWritten, String(result.output.length));
      assert.ok(result.maxRead > 0 && result.maxRead <= 4096, `max read ${result.maxRead}`);
      assert.ok(result.maxWrite > 0 && result.maxWrite <= 4096, `max write ${result.maxWrite}`);
      await validatePdf(t, decode(result.output), pages);
    });
  }
});

test("Chromium: HN and C8 inputs are rejected as unsupported", options, async () => {
  for (const format of ["hn", "c8"]) {
    const result = await run("reject", `input.${format}`);
    assert.deepEqual(
      { name: result.error?.name, code: result.error?.code, format: result.error?.format },
      { name: "UnsupportedFormatError", code: "UNSUPPORTED_FORMAT", format },
    );
    assert.equal(result.written, 0);
  }
});

test("Chromium: AbortSignal stops a conversion stalled on sink backpressure", options, async () => {
  const result = await run("abortBackpressure", "input.caj");
  assert.equal(result.error?.name, "AbortError");
  assert.equal(result.writes, 1);
});

test("Chromium: a ReadableStream spools through real OPFS and is removed", options, async (t) => {
  const result = await run("opfsConvert", "input.caj");
  assert.equal(result.report.format, "caj");
  assert.equal(result.report.pagesConverted, 2);
  assert.equal(result.during.length, 1, "one OPFS spool file exists during conversion");
  assert.match(result.during[0], /^caj2pdf-spool-/);
  assert.deepEqual(result.after, []);
  await validatePdf(t, decode(result.output), 2);
});

test("Chromium: OPFS spool bound, failure, and abort remove the spool", options, async () => {
  const result = await run("opfsFailures", "input.caj");
  assert.equal(result.bounded.error?.code, "LIMIT_EXCEEDED");
  assert.match(result.bounded.error.message, /spool limit of 500 bytes/);
  assert.equal(result.lowLevel.error?.code, "LIMIT_EXCEEDED");
  assert.deepEqual(
    { name: result.unsupported.error?.name, format: result.unsupported.error?.format },
    { name: "UnsupportedFormatError", format: null },
  );
  assert.equal(result.aborted.error?.name, "AbortError");
  for (const key of ["afterBound", "afterLowLevel", "afterUnsupported", "afterAbort"]) {
    assert.deepEqual(result[key], [], `${key}: no OPFS spool remains`);
  }
});

test("Chromium: the page raised no uncaught exceptions", options, () => {
  assert.deepEqual(page.errors, []);
});

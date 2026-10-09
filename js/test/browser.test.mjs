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
import { fixture, pageText, syntheticFramedFooter, syntheticDamagedCaj, syntheticCaj, syntheticAscii85Caj, syntheticFlateReplayCaj, syntheticRecoveredCaj, syntheticLaterCopyCaj, syntheticKdh, validatePdf, validateMultiImageHn, validateType1Hn, wasmUrl } from "./helpers.mjs";

const chrome = findChrome();
if (chrome == null && process.env.CI) {
  throw new Error("Chromium is required in CI: set CAJ2PDF_CHROME or install google-chrome");
}
const skip = chrome == null && "no Chromium found; set CAJ2PDF_CHROME to run the real-browser tests";

let server;
let browser;
let page;

// `after` awaits these even if `before` timed out while they were pending,
// so a late-starting server or browser is still closed.
before(async () => {
  if (skip) return;
  const pdf = await fixture("valid_out_of_order_objects.pdf");
  const footer = new TextEncoder().encode("WebFastLoad\uFEFF<FileProperty><Doi /><FileName>original-test</FileName><TableName>TEST</TableName><Type>1</Type></FileProperty>");
  const fixtures = {
    "/fixtures/input.caa": await fixture("target_descriptor.caa"),
    "/fixtures/framed-footer.pdf": syntheticFramedFooter(pdf),
    "/fixtures/footer.pdf": new Uint8Array([...pdf, ...footer]),
    "/fixtures/symbols.ttf": await readFile(new URL("../../tests/fonts/symbols.ttf", import.meta.url)),
    "/fixtures/geometric.ttf": await readFile(new URL("../../tests/fonts/geometric.ttf", import.meta.url)),
    "/fixtures/input.caj": syntheticCaj(),
    "/fixtures/damaged.caj": syntheticDamagedCaj(),
    "/fixtures/ascii85.caj": syntheticAscii85Caj(),
    "/fixtures/adjacent-flate.caj": syntheticFlateReplayCaj({ anchor: null }),
    "/fixtures/adjacent-flate-clean.caj": syntheticFlateReplayCaj({ anchor: null, interrupted: false }),
    "/fixtures/array-replay.caj": syntheticFlateReplayCaj({ anchor: "array", padding: "\n" }),
    "/fixtures/array-clean.caj": syntheticFlateReplayCaj({ anchor: "array", padding: "\n", interrupted: false }),
    "/fixtures/deferred-replay.caj": syntheticFlateReplayCaj({ anchor: "deferred" }),
    "/fixtures/deferred-clean.caj": syntheticFlateReplayCaj({ anchor: "deferred", interrupted: false }),
    "/fixtures/deferred-broken.caj": syntheticFlateReplayCaj({ anchor: "deferred", broken: true }),
    "/fixtures/deferred-broken-clean.caj": syntheticFlateReplayCaj({ anchor: "deferred", broken: true, interrupted: false }),
    "/fixtures/scalar-replay.caj": syntheticFlateReplayCaj(),
    "/fixtures/scalar-clean.caj": syntheticFlateReplayCaj({ interrupted: false }),
    "/fixtures/scalar-broken.caj": syntheticFlateReplayCaj({ broken: true }),
    "/fixtures/scalar-broken-clean.caj": syntheticFlateReplayCaj({ broken: true, interrupted: false }),
    "/fixtures/keyword-cut.caj": syntheticAscii85Caj({ cut: "keyword" }),
    "/fixtures/reference-cut.caj": syntheticAscii85Caj({ cut: "reference" }),
    "/fixtures/ascii85-clean.caj": syntheticAscii85Caj({ interrupted: false }),
    "/fixtures/ascii85-broken.caj": syntheticAscii85Caj({ broken: true }),
    "/fixtures/ascii85-broken-clean.caj": syntheticAscii85Caj({ broken: true, interrupted: false }),
    "/fixtures/recovered.caj": syntheticRecoveredCaj(),
    "/fixtures/later-copy.caj": syntheticLaterCopyCaj(),
    "/fixtures/broken-later-copy.caj": syntheticLaterCopyCaj(true),
    "/fixtures/broken-recovery.caj": syntheticRecoveredCaj(true),
    "/fixtures/input.kdh": (await syntheticKdh()).wrapped,
    "/fixtures/profile1.kdh": (await syntheticKdh([1, 0, 0, 0])).wrapped,
    "/fixtures/input.pdf": await fixture("valid_nested_outline.pdf"),
    "/fixtures/input.hn": await fixture("truncated_hn.hn"),
    "/fixtures/input.c8": await fixture("truncated_c8.c8"),
    // The fresh build, not a possibly stale js/caj2pdf_wasm.wasm copy.
    "/caj2pdf_wasm.wasm": await readFile(wasmUrl),
    "/index.html": "<!doctype html><meta charset=utf-8><title>caj2pdf browser tests</title>",
  };
  // Only the package directory is served, cross-origin isolated so that the
  // Worker shares its control block; browser-example.test.mjs and
  // package.test.mjs run without isolation.
  server = startServer(fileURLToPath(new URL("..", import.meta.url)), fixtures, { isolated: true });
  browser = launchChrome(chrome);
  page = await openPage((await browser).cdp, `${(await server).origin}/index.html`);
}, { timeout: 90_000 });

after(async () => {
  await (await browser?.catch(() => undefined))?.close();
  await (await server?.catch(() => undefined))?.close();
});

/** Run an exported case from `browser-cases.mjs` in the page. */
function run(name, ...args) {
  return page.evaluate(
    `import("/test/browser-cases.mjs").then((cases) => cases[${JSON.stringify(name)}](...${JSON.stringify(args)}))`,
  );
}

function decode(output) {
  const bytes = new Uint8Array(Buffer.from(output.base64, "base64"));
  assert.equal(bytes.length, output.length);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), output.sha256);
  return bytes;
}

const options = { skip, timeout: 60_000 };

test("Chromium: CAA inspection succeeds and conversion refuses without writing", options, async () => {
  const result = await run("inspectDescriptor", "input.caa");
  assert.equal(result.info.format, "caa");
  assert.equal(result.info.pageCount, null);
  assert.equal(result.info.bookmarkCount, null);
  assert.equal(result.error?.name, "UnsupportedFormatError");
  assert.equal(result.error?.code, "UNSUPPORTED_FORMAT");
  assert.equal(result.error?.format, "caa");
  assert.match(result.error.message, /obtain the referenced document/);
  assert.equal(result.written, 0);
});

test("Chromium: File sources and WritableStream sinks convert CAJ, KDH, and PDF", options, async (t) => {
  for (const [name, format, pages, bookmarks] of [
    ["footer.pdf", "pdf", 2, 0],
    ["framed-footer.pdf", "pdf", 2, 0],
    ["input.caj", "caj", 2, 1],
    ["recovered.caj", "caj", 2, 1],
    ["input.kdh", "kdh", 2, 0],
    ["profile1.kdh", "kdh", 2, 0],
    ["input.pdf", "pdf", 2, 0],
  ]) {
    await t.test(format, async (t) => {
      const result = await run("convertFile", name);
      assert.equal(result.report.format, format);
      assert.equal(result.report.pagesConverted, pages);
      assert.equal(result.report.bookmarksWritten, bookmarks);
      assert.equal(result.pageCount, pages);
      assert.equal(result.report.outputBytesWritten, String(result.output.length));
      assert.ok(result.progress.length > 0 && result.progress.at(-1) === 1, JSON.stringify(result.progress));
      assert.ok(result.maxWrite > 0 && result.maxWrite <= 4096, `max write ${result.maxWrite}`);
      if (name === "footer.pdf" || name === "framed-footer.pdf" || name === "profile1.kdh") assert.deepEqual(decode(result.output), await fixture("valid_out_of_order_objects.pdf"));
      await validatePdf(t, decode(result.output), pages);
    });
  }
});

test("Chromium: malformed HN and C8 inputs return located conversion errors", options, async () => {
  for (const format of ["hn", "c8"]) {
    const result = await run("reject", `input.${format}`);
    assert.deepEqual(
      { name: result.error?.name, code: result.error?.code, format: result.error?.format },
      { name: "Caj2PdfError", code: "HNC8", format: undefined },
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
  assert.equal(result.unlocked, true, "the source reader is released and can be reacquired");
  await validatePdf(t, decode(result.output), 2);
});

test("Chromium: OPFS spool bound, failure, and abort remove the spool", options, async () => {
  const result = await run("opfsFailures", "input.caj");
  assert.equal(result.bounded.error?.code, "LIMIT_EXCEEDED", JSON.stringify(result.bounded));
  assert.match(result.bounded.error.message, /spool limit of 500 bytes/);
  assert.equal(result.lowLevel.error?.code, "LIMIT_EXCEEDED");
  assert.deepEqual(
    { name: result.unsupported.error?.name, format: result.unsupported.error?.format },
    { name: "UnsupportedFormatError", format: null },
  );
  assert.equal(result.aborted.error?.name, "AbortError");
  assert.deepEqual(result.unlocked, [true, true, true, true]);
  for (const key of ["afterBound", "afterLowLevel", "afterUnsupported", "afterAbort"]) {
    assert.deepEqual(result[key], [], `${key}: no OPFS spool remains`);
  }
});

test("Chromium: the page is cross-origin isolated", options, async () => {
  assert.equal(await page.evaluate("crossOriginIsolated"), true);
});

test("Chromium: an OPFS file handle is a source", options, async (t) => {
  const result = await run("convertOpfsHandle", "input.caj");
  assert.equal(result.report.pagesConverted, 2);
  assert.deepEqual(result.after, []);
  await validatePdf(t, decode(result.output), 2);
});

test("Chromium: the page raised no uncaught exceptions", options, () => {
  assert.deepEqual(page.errors, []);
});

test("Chromium: HN/C8 converts from a caller Worker with OPFS and Blob fonts", options, async (t) => {
  const result = await run("hnc8InWorker");
  assert.equal(result.error, undefined);
  assert.equal(result.pages, 1);
  await validateMultiImageHn(t, new Uint8Array(result.pdf));
  assert.equal(result.type1Pages, 1);
  await validateType1Hn(t, new Uint8Array(result.type1Pdf));
  const { syntheticType1Hn } = await import("./hnc8-fixtures.mjs");
  assert.ok(Buffer.from(result.type1Pdf).includes(syntheticType1Hn().jpeg));
  assert.deepEqual(result.remainingEntries, []);
  assert.deepEqual(result.fontFailures, ["missing-glyph", "read-error", "cancel"]);
  const late = pageText(Buffer.from(result.latePdf));
  assert.ok(late.includes("<0041> Tj"), "late HN-B failure keeps the finished first page");
  assert.ok(!late.includes("%%EOF"), "late HN-B failure must not finalize the PDF");
  const hnbProfile = Buffer.from(result.hnbProfilePdf);
  assert.ok(pageText(hnbProfile).includes("/ActualText <FEFFE6C7>"));
  assert.equal(pageText(hnbProfile).match(/<0041> Tj/g).length, 2);
  await validatePdf(t, hnbProfile, 1);
  const profile = Buffer.from(result.profilePdf);
  assert.equal(pageText(profile).match(/<0041> Tj/g).length, 6);
  assert.ok(profile.includes(syntheticType1Hn().jpeg));
  await validatePdf(t, profile, 1);
  assert.equal(result.nativePdfs.length, 10);
  for (const [index, bytes] of result.nativePdfs.entries()) {
    const pdf = Buffer.from(bytes);
    const text = pageText(pdf);
    assert.equal(text.match(/\/FontFile2 /g).length, index === 3 || index === 5 || index >= 7 ? 2 : 1);
    assert.equal(text.match(/<0041> Tj/g).length, [1, 2, 2, 2, 2, 2, 2, 1, 1, 1][index]);
    if (index === 3) {
      assert.equal(text.match(/<0020> Tj/g).length, 2);
      assert.equal(text.match(/<FF1A> Tj/g).length, 2);
    }
    if (index === 4) {
      assert.ok(pdf.includes(syntheticType1Hn().jpeg));
      assert.ok(text.indexOf("/Im0 Do") >= 0 && text.indexOf("/Im0 Do") < text.indexOf("<0041> Tj"));
    }
    await validatePdf(t, pdf, [1, 1, 2, 2, 1, 2, 2, 1, 1, 1][index]);
  }
  assert.equal(result.standardPages, 1);
  await validatePdf(t, new Uint8Array(result.standardPdf), 1);
});

test("Chromium: HN/C8 inspection distinguishes validated and unknown outlines", options, async () => {
  assert.deepEqual(await run("inspectHnc8"), [
    { format: "hn", pages: 1, bookmarks: 2, warnings: 0 },
    { format: "hn", pages: 1, bookmarks: 1, warnings: 1 },
    { format: "c8", pages: 1, bookmarks: null, warnings: 0 },
    { format: "hn", pages: 1, bookmarks: null, warnings: 0 },
  ]);
});


test("Chromium: CAJ recovery retains a later malformed-object error", options, async () => {
  for (const mode of ["reject", "rejectSpooled"]) {
    const result = await run(mode, "broken-recovery.caj");
    assert.equal(result.error?.code, "MALFORMED_PDF");
    assert.equal(result.written, 0);
    if (mode === "rejectSpooled") {
      assert.deepEqual(result.after, []);
      assert.equal(result.unlocked, true);
    }
  }
});

test("Chromium: later-copy CAJ and stream replay run in a Worker", options, async (t) => {
  const result = await run("cajRecoveryInWorker");
  assert.equal(result.positive.report.pagesConverted, 2);
  assert.equal(result.positive.report.bookmarksWritten, 1);
  await validatePdf(t, decode(result.positive.output), 2);
  assert.equal(result.negative.error?.code, "MALFORMED_PDF");
  assert.equal(result.negative.written, 0);
  assert.equal(result.ascii85.report.pagesConverted, 2);
  assert.deepEqual(result.ascii85.output, result.cleanAscii85.output);
  assert.deepEqual(result.keywordCut.output, result.cleanAscii85.output);
  assert.deepEqual(result.referenceCut.output, result.cleanAscii85.output);
  await validatePdf(t, decode(result.ascii85.output), 2);
  assert.deepEqual(result.scalarReplay.output, result.scalarClean.output);
  await validatePdf(t, decode(result.scalarReplay.output), 2);
  // Payloads are not decoded: a corrupt checksum or a Length that also
  // covers the end-of-line byte still frames, and the replay still applies.
  for (const [recovered, clean] of [[result.brokenAscii85, result.brokenAscii85Clean], [result.scalarBroken, result.scalarBrokenClean], [result.deferredBroken, result.deferredBrokenClean]]) {
    assert.equal(recovered.report.pagesConverted, 2);
    assert.deepEqual(recovered.output, clean.output);
  }
  for (const [recovered, clean] of [[result.deferredReplay, result.deferredClean], [result.adjacentFlate, result.adjacentClean], [result.arrayReplay, result.arrayClean]]) {
    assert.deepEqual(recovered.output, clean.output);
    await validatePdf(t, decode(recovered.output), 2);
  }
});

test("Chromium: damaged CAJ mode returns explicit blank-page diagnostics", options, async (t) => {
  const result = await run("convertDamaged", "damaged.caj");
  assert.deepEqual(result.omittedPages.map((page) => page.pageIndex), [0]);
  assert.match(result.omittedPages[0].offset, /^\d+$/);
  await validatePdf(t, decode(result.output), 2);
});

test("Chromium: abort between the cancellation check and ACK wait closes OPFS", options, async () => {
  const result = await run("abortBeforeAckWait", "input.caj");
  assert.equal(result.error?.name, "AbortError", JSON.stringify(result));
  assert.equal(result.pauses, 1);
  assert.deepEqual(result.waits, ["not-equal"], "a pre-wait cancellation cannot lose its wakeup");
  assert.deepEqual(result.after, [], "the Worker closes its input before spool removal");
  assert.equal(result.unlocked, true);
});

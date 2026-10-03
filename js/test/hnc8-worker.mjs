// SPDX-License-Identifier: MIT

import { blobSource, convert, loadModule, spoolToOpfs, syncAccessHandleScratch } from "../browser.mjs";
import { qmStates, syntheticNativeC8, syntheticNativeHnb, syntheticNativeHnbMixed, syntheticNativeHnbAxes, syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

const root = await navigator.storage.getDirectory();
const names = [];
const handles = [];
const fontSpools = [];
let result;
try {
  const scratch = [];
  for (let i = 0; i < 4; i++) {
    const name = `caj2pdf-hn-test-${crypto.randomUUID()}`;
    const file = await root.getFileHandle(name, { create: true }); names.push(name);
    const handle = await file.createSyncAccessHandle(); handles.push(handle);
    scratch.push(syncAccessHandleScratch(handle, { maxBytes: 1024n }));
  }
  const module = await loadModule();
  const parts = [];
  const report = await convert(module, blobSource(new Blob([syntheticHn(true, true)])), {
    async writeChunk(bytes) { parts.push(...bytes); return bytes.length; }, async flush() {},
  }, { chunkSize: 3, hnc8: { qmStates, scratch } });
  const standardPdf = [];
  const standard = await convert(module, blobSource(new Blob([syntheticHn()])), {
    async writeChunk(bytes) { standardPdf.push(...bytes); return bytes.length; }, async flush() {},
  }, { chunkSize: 3, hnc8: { scratch } });
  const type1Pdf = [];
  const type1 = await convert(module, blobSource(new Blob([syntheticType1Hn().bytes])), {
    async writeChunk(bytes) { type1Pdf.push(...bytes); return bytes.length; }, async flush() {},
  }, { chunkSize: 3, hnc8: { scratch } });
  for (const markers of [false, true]) {
    const prefixedPdf = [];
    await convert(module, blobSource(new Blob([syntheticPrefixedHn(markers)])), {
      async writeChunk(bytes) { prefixedPdf.push(...bytes); return bytes.length; }, async flush() {},
    }, { chunkSize: 3, hnc8: { qmStates, scratch } });
    if (prefixedPdf.length !== parts.length || prefixedPdf.some((byte, i) => byte !== parts[i])) {
      throw new Error("paired raw prefix or image markers changed the mixed-image PDF");
    }
  }
  const fontBlob = await (await fetch("/fixtures/geometric.ttf")).blob();
  try {
    await spoolToOpfs(fontBlob.stream(), { maxBytes: BigInt(fontBlob.size) - 1n });
    throw new Error("font spool limit was not enforced");
  } catch (error) { if (error.code !== "LIMIT_EXCEEDED") throw error; }
  const fontSpool = await spoolToOpfs(fontBlob.stream(), { maxBytes: BigInt(fontBlob.size) });
  fontSpools.push(fontSpool);
  const rangedFont = fontSpool.source;
  let fontMaxRead = 0;
  const font = { size: rangedFont.size, async readAt(offset, length, signal) {
    fontMaxRead = Math.max(fontMaxRead, length);
    return rangedFont.readAt(offset, Math.min(length, 3), signal);
  } };
  const symbolBlob = await (await fetch("/fixtures/symbols.ttf")).blob();
  const symbolSpool = await spoolToOpfs(symbolBlob.stream(), { maxBytes: BigInt(symbolBlob.size) });
  fontSpools.push(symbolSpool);
  const symbols = { size: symbolSpool.source.size, async readAt(offset, length, signal) {
    fontMaxRead = Math.max(fontMaxRead, length);
    return symbolSpool.source.readAt(offset, Math.min(length, 3), signal);
  } };
  const nativePdfs = [];
  for (const [input, pages, hasSymbols, hasState3, latinState] of [[syntheticNativeC8(), 1], [syntheticNativeC8(true), 1], [syntheticNativeHnb(), 2], [syntheticNativeHnb(0), 2, true], [syntheticNativeHnbMixed(), 1], [syntheticNativeHnb(2, true), 2, false, true], [syntheticNativeHnbAxes(), 2], ...[3, 28, 31].map(state => [syntheticNativeC8(false, state), 1, false, false, state])]) {
    const pdf = [];
    const native = await convert(module, blobSource(new Blob([input])), {
      async writeChunk(bytes) { pdf.push(...bytes); return bytes.length; }, async flush() {},
    }, { includeBookmarks: false, chunkSize: 32, hnc8: {
      fonts: { cjk: font, latin: font, alternateLatin: font, ...(hasSymbols ? { symbols } : {}), ...(hasState3 ? { latinState3: { ...font } } : {}), ...(latinState ? { [`latinState${latinState}`]: { ...font } } : {}) }, qmStates, scratch,
    } });
    if (native.pagesConverted !== pages) throw new Error("native C8/HN-B page count mismatch");
    nativePdfs.push(pdf);
  }
  const lateInput = syntheticNativeHnb();
  const lateView = new DataView(lateInput.buffer);
  lateView.setUint16(lateView.getUint32(228, true), 0x8099, true);
  const lateParts = [];
  try {
    await convert(module, blobSource(new Blob([lateInput])), {
      async writeChunk(bytes) { lateParts.push(...bytes); return bytes.length; }, async flush() {},
    }, { includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { cjk: font, latin: font, alternateLatin: font }, scratch } });
    throw new Error("late HN-B record unexpectedly succeeded");
  } catch (error) {
    if (error.code !== "HNC8" || !/page 2/.test(error.message)) throw error;
  }
  const latePdf = new TextDecoder().decode(new Uint8Array(lateParts));
  if (!latePdf.includes("<0041> Tj") || latePdf.includes("%%EOF")) throw new Error("late HN-B failure did not preserve unfinished first-page output");
  if (!scratch.every((store) => store.size === 0n)) throw new Error("late HN-B failure left scratch data");
  const fontFailures = [];
  for (const mode of ["missing-glyph", "read-error", "cancel"]) {
    const input = syntheticNativeC8();
    if (mode === "missing-glyph") new DataView(input.buffer).setUint16(110, 0xa0c2, true);
    const controller = new AbortController();
    const failure = new Error("caller Worker font read failed");
    const failingFont = { size: font.size, async readAt(offset, length, signal) {
      if (mode === "read-error") throw failure;
      if (mode === "cancel") controller.abort();
      return font.readAt(offset, length, signal);
    } };
    try {
      await convert(module, blobSource(new Blob([input])), {
        async writeChunk(bytes) { return bytes.length; }, async flush() {},
      }, { includeBookmarks: false, chunkSize: 32, signal: controller.signal, hnc8: {
        fonts: { cjk: failingFont, latin: failingFont, alternateLatin: failingFont }, scratch,
      } });
      throw new Error(`expected ${mode} to fail`);
    } catch (error) {
      if (mode === "missing-glyph" && error.code !== "HNC8") throw error;
      if (mode === "read-error" && error !== failure) throw error;
      if (mode === "cancel" && error.name !== "AbortError") throw error;
      fontFailures.push(mode);
    }
    if (!scratch.every((store) => store.size === 0n)) throw new Error("font failure left scratch data");
  }
  result = { fontFailures, nativePdfs, fontMaxRead, type1Pages: type1.pagesConverted, type1Pdf, standardPages: standard.pagesConverted, standardPdf, pages: report.pagesConverted, pdf: parts, cleared: scratch.every((store) => store.size === 0n) };
} catch (error) {
  result = { error: `${error.name}: ${error.message}` };
} finally {
  for (const spool of fontSpools) await spool.dispose();
  for (const handle of handles) handle.close();
  for (const name of names) await root.removeEntry(name);
}
result.remainingEntries = [];
for await (const [name] of root.entries()) result.remainingEntries.push(name);
postMessage(result);

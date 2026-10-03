// SPDX-License-Identifier: MIT

import { blobSource, convert, loadModule, syncAccessHandleScratch } from "../browser.mjs";
import { qmStates, syntheticNativeC8, syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

const root = await navigator.storage.getDirectory();
const names = [];
const handles = [];
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
  const rangedFont = blobSource(fontBlob);
  let fontMaxRead = 0;
  const font = { size: rangedFont.size, async readAt(offset, length, signal) {
    fontMaxRead = Math.max(fontMaxRead, length);
    return rangedFont.readAt(offset, Math.min(length, 3), signal);
  } };
  const nativePdfs = [];
  for (const mixed of [false, true]) {
    const pdf = [];
    const native = await convert(module, blobSource(new Blob([syntheticNativeC8(mixed)])), {
      async writeChunk(bytes) { pdf.push(...bytes); return bytes.length; }, async flush() {},
    }, { includeBookmarks: false, chunkSize: 32, hnc8: {
      fonts: { cjk: font, latin: font, alternateLatin: font }, qmStates, scratch,
    } });
    if (native.pagesConverted !== 1) throw new Error("native C8 page count mismatch");
    nativePdfs.push(pdf);
  }
  result = { nativePdfs, fontMaxRead, type1Pages: type1.pagesConverted, type1Pdf, standardPages: standard.pagesConverted, standardPdf, pages: report.pagesConverted, pdf: parts, cleared: scratch.every((store) => store.size === 0n) };
} catch (error) {
  result = { error: `${error.name}: ${error.message}` };
} finally {
  for (const handle of handles) handle.close();
  for (const name of names) await root.removeEntry(name);
}
postMessage(result);

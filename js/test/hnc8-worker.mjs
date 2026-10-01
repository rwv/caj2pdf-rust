// SPDX-License-Identifier: MIT

import { blobSource, convert, loadModule, syncAccessHandleScratch } from "../browser.mjs";
import { qmStates, syntheticHn, syntheticType1Hn } from "./hnc8-fixtures.mjs";

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
  result = { type1Pages: type1.pagesConverted, type1Pdf, standardPages: standard.pagesConverted, standardPdf, pages: report.pagesConverted, pdf: parts, cleared: scratch.every((store) => store.size === 0n) };
} catch (error) {
  result = { error: `${error.name}: ${error.message}` };
} finally {
  for (const handle of handles) handle.close();
  for (const name of names) await root.removeEntry(name);
}
postMessage(result);

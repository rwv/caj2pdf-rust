// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { test } from "node:test";
import { convert, inspect } from "../node.mjs";
import { validateMultiImageHn, validateType1Hn, wasmModule } from "./helpers.mjs";
import { syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

function source() { return new Blob([syntheticHn()]); }
function sink(parts = []) { return { async writeChunk(bytes) { parts.push(bytes.slice()); return bytes.length; }, async flush() {} }; }

test("real WASM composes multiple HN images and bookmarks in memory", async (t) => {
  const parts = [];
  const report = await convert(await wasmModule(), new Blob([syntheticHn(true, true)]), sink(parts), { chunkSize: 7 });
  assert.equal(report.pagesConverted, 1);
  assert.equal(report.format, "hn");
  await validateMultiImageHn(t, Buffer.concat(parts));
});

test("WASM HN converts with three-byte reads and writes", async () => {
  const parts = [];
  const report = await convert(await wasmModule(), source(), sink(parts), { chunkSize: 3 });
  assert.equal(report.pagesConverted, 1);
  assert.equal(report.outputBytesWritten, BigInt(Buffer.concat(parts).length));
  for (const options of [undefined, { hnc8: {} }]) {
    assert.equal((await convert(await wasmModule(), source(), sink(), options)).pagesConverted, 1);
  }
});

test("source and sink faults reach the caller and the module converts again", async () => {
  for (const mode of ["source", "sink"]) {
    const failure = new Error(mode);
    class Failing extends Blob {
      slice() {
        throw failure;
      }
    }
    const input = mode === "source" ? new Failing([syntheticHn()]) : source();
    const output = mode === "sink" ? { async writeChunk() { throw failure; }, async flush() {} } : sink();
    await assert.rejects(convert(await wasmModule(), input, output), (error) => error === failure);
    const report = await convert(await wasmModule(), source(), sink());
    assert.equal(report.pagesConverted, 1);
  }
});

test("HN stops at scheduled cancellation", async () => {
  const controller = new AbortController(); const reason = new Error("stop HN");
  let scheduled = false;
  const output = {
    async writeChunk(bytes) {
      if (!scheduled) { scheduled = true; setTimeout(() => controller.abort(reason), 0); }
      return bytes.length;
    },
    async flush() {},
  };
  await assert.rejects(convert(await wasmModule(), source(), output, { chunkSize: 1, signal: controller.signal }), (error) => error === reason);
  assert.ok(scheduled);
});

test("HN/C8 inspection validates metadata without image reads or runtime tables", async () => {
  const { unknownOutline } = await import("./hnc8-fixtures.mjs");
  for (const [bytes, format, count, boundary] of [
    [syntheticHn(), "hn", 0, 0x15c],
    [syntheticHn(true), "hn", 2, 0x15c + 616],
    [unknownOutline("c8"), "c8", null, 0x50],
    [unknownOutline("hn"), "hn", null, 0xd8],
  ]) {
    let reads = 0n;
    class Checked extends Blob {
      slice(start, end) {
        // C8 also probes its last 32 bytes for an application-info trailer.
        const trailer = format === "c8" && start >= bytes.length - 32;
        assert.ok(Math.min(end, bytes.length) <= boundary || trailer, "inspection must not request image/page payloads");
        const chunk = super.slice(start, end);
        reads += BigInt(chunk.size);
        return chunk;
      }
    }
    const input = new Checked([bytes]);
    const info = await inspect(await wasmModule(), input, { chunkSize: 3 });
    assert.equal(info.format, format);
    assert.equal(info.pageCount, 1);
    assert.equal(info.bookmarkCount, count);
    assert.equal(info.inputBytesRead, reads);
  }
});

test("C8 inspection reports the application-info package", async () => {
  const { deflateSync } = await import("node:zlib");
  const { unknownOutline } = await import("./hnc8-fixtures.mjs");
  // Invented values; the framing is [decoded][compressed][zlib]APPINFOSIGN <start>.
  const xml = Buffer.from("<Package><Note-Package><NoteItems><Item/><Item/></NoteItems></Note-Package>"
    + "<FileProperty-Package><DOI>INVENTED:1</DOI><DURL>http://example.invalid/x</DURL></FileProperty-Package></Package>");
  const withPackage = (decoded) => {
    const c8 = unknownOutline("c8");
    const stream = deflateSync(xml);
    const lengths = Buffer.alloc(8);
    lengths.writeUInt32LE(decoded, 0);
    lengths.writeUInt32LE(stream.length, 4);
    return Buffer.concat([c8, lengths, stream, Buffer.from(`APPINFOSIGN ${c8.length}`)]);
  };
  const info = await inspect(await wasmModule(), new Blob([withPackage(xml.length)]));
  assert.deepEqual(info.applicationInfo, { doi: "INVENTED:1", url: "http://example.invalid/x", noteCount: 2 });
  // A defective package is ignored, as in conversion; a plain C8 has none.
  for (const bytes of [withPackage(xml.length + 1), unknownOutline("c8")]) {
    assert.equal((await inspect(await wasmModule(), new Blob([bytes]))).applicationInfo, null);
  }
});

test("HN-A outline defects are counted warnings in inspection and conversion", async () => {
  const clean = syntheticHn(true);
  const cleanInfo = await inspect(await wasmModule(), new Blob([clean]));
  assert.equal(cleanInfo.outlineWarnings, 0);
  const cleanParts = [];
  const cleanReport = await convert(await wasmModule(), new Blob([clean]), sink(cleanParts));
  assert.deepEqual([cleanReport.bookmarksWritten, cleanReport.outlineWarnings], [2, 0]);
  // Level 4 under a level-1 root is clamped to level 2: one warning, same outline.
  const clamped = syntheticHn(true);
  new DataView(clamped.buffer).setUint32(0x15c + 308 + 304, 4, true);
  // Page 9 of a one-page document skips only that entry.
  const skipped = syntheticHn(true);
  skipped[0x15c + 308 + 280] = 57;
  for (const [bytes, written] of [[clamped, 2], [skipped, 1]]) {
    const info = await inspect(await wasmModule(), new Blob([bytes]));
    assert.deepEqual([info.bookmarkCount, info.outlineWarnings], [written, 1]);
    const parts = [];
    const report = await convert(await wasmModule(), new Blob([bytes]), sink(parts));
    assert.deepEqual([report.pagesConverted, report.bookmarksWritten, report.outlineWarnings], [1, written, 1]);
    if (bytes === clamped) assert.deepEqual(Buffer.concat(parts), Buffer.concat(cleanParts));
  }
});

test("HN inspection rejects unreadable outlines, limits and cancelled reads", async () => {
  await assert.rejects(inspect(await wasmModule(), new Blob([syntheticHn(true)]), { limits: { maxBookmarks: 1 } }), { code: "HNC8" });
  const controller = new AbortController(); const reason = new Error("stop inspection");
  class Aborting extends Blob {
    slice(start, end) {
      controller.abort(reason);
      return super.slice(start, end);
    }
  }
  const input = new Aborting([syntheticHn()]);
  await assert.rejects(inspect(await wasmModule(), input, { signal: controller.signal }), (error) => error === reason);
});

test("bilevel compression is deterministic and has an explicit allocation floor", async () => {
  let reference;
  for (let iteration = 0; iteration < 2; iteration++) {
    const parts = [];
    await convert(await wasmModule(), source(), sink(parts), {
      chunkSize: 7, limits: { maxAllocationBytes: 512n * 1024n },
    });
    const pdf = Buffer.concat(parts);
    assert.match(pdf.toString("latin1"), /\/Filter \/FlateDecode/);
    if (iteration === 0) reference = pdf;
    else assert.deepEqual(pdf, reference);
  }
  await assert.rejects(convert(await wasmModule(), source(), sink(), {
    chunkSize: 7, limits: { maxAllocationBytes: 512n * 1024n - 1n },
  }), (error) => error.code === "HNC8" && /allocation bytes/.test(error.message));
});

test("type-1 JPEG uses the bounded public path and preserves its encoded image", async (t) => {
  const { bytes, jpeg } = syntheticType1Hn();
  const outputs = [];
  for (const kind of [1, 2]) {
    new DataView(bytes.buffer).setUint32(0x190, kind, true);
    const parts = [];
    const report = await convert(await wasmModule(), new Blob([bytes]), sink(parts),
      { chunkSize: 3 });
    assert.equal(report.pagesConverted, 1);
    const pdf = Buffer.concat(parts);
    assert.ok(pdf.includes(jpeg));
    await validateType1Hn(t, pdf);
    outputs.push(pdf);
  }
  assert.deepEqual(outputs[0], outputs[1]);
  new DataView(bytes.buffer).setUint32(0x190, 1, true);
  bytes[0x19c] = 0; // The type tag cannot bypass JPEG validation.
  await assert.rejects(convert(await wasmModule(), new Blob([bytes]), sink(),
    { chunkSize: 1 }));
});

test("paired raw HN prefix preserves independently validated mixed-image output", async (t) => {
  const outputs = [];
  for (const bytes of [syntheticHn(true, true), syntheticPrefixedHn(), syntheticPrefixedHn(true)]) {
    const parts = [];
    const report = await convert(await wasmModule(), new Blob([bytes]), sink(parts), { chunkSize: 3 });
    assert.equal(report.pagesConverted, 1);
    outputs.push(Buffer.concat(parts));
  }
  assert.deepEqual(outputs[0], outputs[1]);
  assert.deepEqual(outputs[0], outputs[2]);
  await validateMultiImageHn(t, outputs[1]);
});

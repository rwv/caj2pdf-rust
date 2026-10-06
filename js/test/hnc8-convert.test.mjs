// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { open, rm } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { blobSource, convert, fileHandleScratch } from "../node.mjs";
import { newInstance, tempDirectory, validateMultiImageHn, validateType1Hn } from "./helpers.mjs";
import { syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

function memoryStore() {
  let bytes = new Uint8Array();
  return {
    get size() { return BigInt(bytes.length); },
    async resize(size) { assert.ok(size <= 1024n); const next = new Uint8Array(Number(size)); next.set(bytes.subarray(0, next.length)); bytes = next; },
    async readAt(offset, length) { return bytes.slice(Number(offset), Number(offset) + Math.min(1, length)); },
    async writeAt(offset, input) { bytes.set(input.subarray(0, 1), Number(offset)); return Math.min(1, input.length); },
    async flush() {},
  };
}

function source() { return blobSource(new Blob([syntheticHn()])); }
function sink(parts = []) { return { async writeChunk(bytes) { parts.push(bytes.slice()); return bytes.length; }, async flush() {} }; }
function stores() { return Array.from({ length: 4 }, memoryStore); }

test("real WASM composes multiple HN images and bookmarks through bounded Node scratch", async (t) => {
  const directory = await tempDirectory("hnc8");
  const handles = [];
  try {
    const scratch = [];
    for (let i = 0; i < 4; i++) {
      const handle = await open(join(directory, `${i}`), "wx+"); handles.push(handle);
      scratch.push(await fileHandleScratch(handle, { maxBytes: 1024n }));
    }
    const parts = [];
    const report = await convert(await newInstance(), blobSource(new Blob([syntheticHn(true, true)])), sink(parts), { chunkSize: 7, hnc8: { scratch } });
    assert.equal(report.pagesConverted, 1);
    assert.equal(report.format, "hn");
    const pdf = Buffer.concat(parts);
    await validateMultiImageHn(t, pdf);
    assert.deepEqual(scratch.map((store) => store.size), [0n, 0n, 0n, 0n]);
  } finally {
    await Promise.all(handles.map((handle) => handle.close()));
    await rm(directory, { recursive: true, force: true });
  }
});

test("WASM HN preserves short scratch I/O and resets on completion", async () => {
  const scratch = stores(); const parts = [];
  const report = await convert(await newInstance(), source(), sink(parts), { chunkSize: 3, hnc8: { scratch } });
  assert.equal(report.pagesConverted, 1);
  assert.equal(report.outputBytesWritten, BigInt(Buffer.concat(parts).length));
  assert.ok(scratch.every((store) => store.size === 0n));
});

test("HN type-0 images use the standard states and need no scratch", async () => {
  // Only type-3 images use the stores; type-0 rows stream straight to the PDF.
  for (const options of [undefined, { hnc8: {} }]) {
    assert.equal((await convert(await newInstance(), source(), sink(), options)).pagesConverted, 1);
  }
  const scratch = stores();
  const report = await convert(await newInstance(), source(), sink(), { hnc8: { scratch } });
  assert.equal(report.pagesConverted, 1);
  assert.ok(scratch.every((store) => store.size === 0n));
  await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8: { scratch: [memoryStore()] } }), TypeError);
  const one = memoryStore();
  await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8: { scratch: [one, one, one, one] } }), TypeError);
});

test("type-0 HN never uses scratch I/O; source and sink faults clear all stores and permit instance reuse", async () => {
  for (const mode of ["read", "write", "resize", "flush", "source", "sink"]) {
    const instance = await newInstance(); const scratch = stores(); const failure = new Error(mode);
    for (const store of scratch) {
      if (mode === "read") store.readAt = async () => { throw failure; };
      if (mode === "write") store.writeAt = async () => { throw failure; };
      if (mode === "resize") { const original = store.resize; store.resize = async (size) => { if (size !== 0n) throw failure; await original(size); }; }
      if (mode === "flush") store.flush = async () => { throw failure; };
    }
    const input = mode === "source" ? { size: 1024n, async readAt() { throw failure; } } : source();
    const output = mode === "sink" ? { async writeChunk() { throw failure; }, async flush() {} } : sink();
    const run = convert(instance, input, output, { hnc8: { scratch } });
    if (mode === "source" || mode === "sink") await assert.rejects(run);
    else assert.equal((await run).pagesConverted, 1);
    assert.ok(scratch.every((store) => store.size === 0n));
    const report = await convert(instance, source(), sink(), { hnc8: { scratch: stores() } });
    assert.equal(report.pagesConverted, 1);
  }
});

test("conversion and cleanup failures are both retained and every store is attempted", async () => {
  const scratch = stores(); const primary = new Error("output failed"); const cleanup = new Error("cleanup failed");
  const attempted = [];
  scratch.forEach((store, index) => { store.resize = async () => { attempted.push(index); if (index === 0) throw cleanup; }; });
  await assert.rejects(convert(await newInstance(), source(), { async writeChunk() { throw primary; }, async flush() {} }, { hnc8: { scratch } }), (error) => {
    assert.ok(error instanceof AggregateError); assert.deepEqual(error.errors, [primary, cleanup]); return true;
  });
  assert.deepEqual(attempted, [0, 1, 2, 3]);
});

test("HN yields to scheduled cancellation and clears every supplied store", async () => {
  const scratch = stores(); const controller = new AbortController(); const reason = new Error("stop HN");
  let scheduled = false;
  const output = {
    async writeChunk(bytes) {
      if (!scheduled) { scheduled = true; setTimeout(() => controller.abort(reason), 0); }
      return bytes.length;
    },
    async flush() {},
  };
  await assert.rejects(convert(await newInstance(), source(), output, { chunkSize: 1, signal: controller.signal, hnc8: { scratch } }), (error) => error === reason);
  assert.ok(scheduled);
  assert.ok(scratch.every((store) => store.size === 0n));
});

test("HN/C8 inspection validates metadata without image reads or runtime tables", async () => {
  const { inspect } = await import("../node.mjs");
  const { unknownOutline } = await import("./hnc8-fixtures.mjs");
  for (const [bytes, format, count, boundary] of [
    [syntheticHn(), "hn", 0, 0x15c],
    [syntheticHn(true), "hn", 2, 0x15c + 616],
    [unknownOutline("c8"), "c8", null, 0x50],
    [unknownOutline("hn"), "hn", null, 0xd8],
  ]) {
    let reads = 0n;
    const input = {
      size: BigInt(bytes.length),
      async readAt(offset, length) {
        // C8 also probes its last 32 bytes for an application-info trailer.
        const trailer = format === "c8" && offset >= BigInt(bytes.length - 32);
        assert.ok(offset + BigInt(length) <= BigInt(boundary) || trailer, "inspection must not request image/page payloads");
        const chunk = bytes.slice(Number(offset), Number(offset) + 1);
        reads += BigInt(chunk.length); return chunk;
      },
    };
    const info = await inspect(await newInstance(), input, { chunkSize: 3 });
    assert.equal(info.format, format);
    assert.equal(info.pageCount, 1);
    assert.equal(info.bookmarkCount, count);
    assert.equal(info.inputBytesRead, reads);
  }
});

test("C8 inspection reports the application-info package", async () => {
  const { inspect } = await import("../node.mjs");
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
  const info = await inspect(await newInstance(), blobSource(new Blob([withPackage(xml.length)])));
  assert.deepEqual(info.applicationInfo, { doi: "INVENTED:1", url: "http://example.invalid/x", noteCount: 2 });
  // A defective package is ignored, as in conversion; a plain C8 has none.
  for (const bytes of [withPackage(xml.length + 1), unknownOutline("c8")]) {
    assert.equal((await inspect(await newInstance(), blobSource(new Blob([bytes])))).applicationInfo, null);
  }
});

test("HN-A outline defects are counted warnings in inspection and conversion", async () => {
  const { inspect } = await import("../node.mjs");
  const clean = syntheticHn(true);
  const cleanInfo = await inspect(await newInstance(), blobSource(new Blob([clean])));
  assert.equal(cleanInfo.outlineWarnings, 0);
  const cleanParts = [];
  const cleanReport = await convert(await newInstance(), blobSource(new Blob([clean])), sink(cleanParts), { hnc8: { scratch: stores() } });
  assert.deepEqual([cleanReport.bookmarksWritten, cleanReport.outlineWarnings], [2, 0]);
  // Level 4 under a level-1 root is clamped to level 2: one warning, same outline.
  const clamped = syntheticHn(true);
  new DataView(clamped.buffer).setUint32(0x15c + 308 + 304, 4, true);
  // Page 9 of a one-page document skips only that entry.
  const skipped = syntheticHn(true);
  skipped[0x15c + 308 + 280] = 57;
  for (const [bytes, written] of [[clamped, 2], [skipped, 1]]) {
    const info = await inspect(await newInstance(), blobSource(new Blob([bytes])));
    assert.deepEqual([info.bookmarkCount, info.outlineWarnings], [written, 1]);
    const parts = [];
    const report = await convert(await newInstance(), blobSource(new Blob([bytes])), sink(parts), { hnc8: { scratch: stores() } });
    assert.deepEqual([report.pagesConverted, report.bookmarksWritten, report.outlineWarnings], [1, written, 1]);
    if (bytes === clamped) assert.deepEqual(Buffer.concat(parts), Buffer.concat(cleanParts));
  }
});

test("HN inspection rejects unreadable outlines, limits and cancelled reads", async () => {
  const { inspect } = await import("../node.mjs");
  await assert.rejects(inspect(await newInstance(), blobSource(new Blob([syntheticHn(true)])), { limits: { maxBookmarks: 1 } }), { code: "HNC8" });
  const controller = new AbortController(); const reason = new Error("stop inspection");
  const inner = source();
  const input = { size: inner.size, async readAt(...args) { const chunk = await inner.readAt(...args); controller.abort(reason); return chunk; } };
  await assert.rejects(inspect(await newInstance(), input, { signal: controller.signal }), (error) => error === reason);
});

test("bilevel compression has bounded reusable WASM memory and an explicit allocation floor", async () => {
  const instance = await newInstance();
  const before = instance.exports.memory.buffer.byteLength;
  let retained, reference;
  for (let iteration = 0; iteration < 3; iteration++) {
    const parts = [];
    await convert(instance, source(), sink(parts), {
      chunkSize: 7, limits: { maxAllocationBytes: 512n * 1024n }, hnc8: { scratch: stores() },
    });
    const pdf = Buffer.concat(parts);
    assert.match(pdf.toString("latin1"), /\/Filter \/FlateDecode/);
    const memory = instance.exports.memory.buffer.byteLength;
    assert.ok(memory - before <= 1024 * 1024, "small-image working memory must stay bounded");
    if (iteration === 0) { retained = memory; reference = pdf; }
    else { assert.equal(memory, retained); assert.deepEqual(pdf, reference); }
  }
  await assert.rejects(convert(instance, source(), sink(), {
    chunkSize: 7, limits: { maxAllocationBytes: 512n * 1024n - 1n }, hnc8: { scratch: stores() },
  }), (error) => error.code === "HNC8" && /allocation bytes/.test(error.message));
});


test("type-1 JPEG uses the bounded public path and preserves its encoded image", async (t) => {
  const { bytes, jpeg } = syntheticType1Hn();
  const outputs = [];
  for (const kind of [1, 2]) {
    new DataView(bytes.buffer).setUint32(0x190, kind, true);
    const parts = [], scratch = stores();
    const report = await convert(await newInstance(), blobSource(new Blob([bytes])), sink(parts),
      { chunkSize: 3, hnc8: { scratch } });
    assert.equal(report.pagesConverted, 1);
    assert.ok(scratch.every((store) => store.size === 0n));
    const pdf = Buffer.concat(parts);
    assert.ok(pdf.includes(jpeg));
    await validateType1Hn(t, pdf);
    outputs.push(pdf);
  }
  assert.deepEqual(outputs[0], outputs[1]);
  new DataView(bytes.buffer).setUint32(0x190, 1, true);
  bytes[0x19c] = 0; // The type tag cannot bypass JPEG validation.
  const scratch = stores();
  await assert.rejects(convert(await newInstance(), blobSource(new Blob([bytes])), sink(),
    { chunkSize: 1, hnc8: { scratch } }));
  assert.ok(scratch.every((store) => store.size === 0n));
});

test("paired raw HN prefix preserves independently validated mixed-image output", async (t) => {
  const outputs = [];
  for (const bytes of [syntheticHn(true, true), syntheticPrefixedHn(), syntheticPrefixedHn(true)]) {
    const scratch = stores(); const parts = [];
    const report = await convert(await newInstance(), blobSource(new Blob([bytes])), sink(parts), { chunkSize: 3, hnc8: { scratch } });
    assert.equal(report.pagesConverted, 1);
    assert.ok(scratch.every((store) => store.size === 0n));
    outputs.push(Buffer.concat(parts));
  }
  assert.deepEqual(outputs[0], outputs[1]);
  assert.deepEqual(outputs[0], outputs[2]);
  await validateMultiImageHn(t, outputs[1]);
});

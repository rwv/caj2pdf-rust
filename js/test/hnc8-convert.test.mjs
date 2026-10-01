// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { open, rm } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { blobSource, convert, fileHandleScratch } from "../node.mjs";
import { newInstance, tempDirectory, validateMultiImageHn, validateType1Hn } from "./helpers.mjs";
import { qmStates, syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

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
    const report = await convert(await newInstance(), blobSource(new Blob([syntheticHn(true, true)])), sink(parts), { chunkSize: 7, hnc8: { qmStates, scratch } });
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
  const report = await convert(await newInstance(), source(), sink(parts), { chunkSize: 3, hnc8: { qmStates, scratch } });
  assert.equal(report.pagesConverted, 1);
  assert.equal(report.outputBytesWritten, BigInt(Buffer.concat(parts).length));
  assert.ok(scratch.every((store) => store.size === 0n));
});

test("HN uses standard states by default and requires random-access scratch", async () => {
  await assert.rejects(convert(await newInstance(), source(), sink()), { code: "RANDOM_ACCESS_REQUIRED" });
  await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8: { qmStates } }), { code: "RANDOM_ACCESS_REQUIRED" });
  const scratch = stores();
  const report = await convert(await newInstance(), source(), sink(), { hnc8: { scratch } });
  assert.equal(report.pagesConverted, 1);
  assert.ok(scratch.every((store) => store.size === 0n));
  for (const hnc8 of [
    { qmStates: [] }, { mqStates: [] }, { scratch: [memoryStore()] },
    { qmStates: qmStates.map(() => ({ qe: 0, nextLps: 0, nextMps: 0, switchMps: false })) },
    { qmStates: qmStates.map(() => ({ qe: 1, nextLps: 113, nextMps: 0, switchMps: false })) },
    { qmStates: qmStates.map(() => ({ qe: 1, nextLps: 0, nextMps: 0, switchMps: 0 })) },
  ]) await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8 }));
  const one = memoryStore();
  await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8: { scratch: [one, one, one, one] } }), TypeError);
});

test("scratch faults reject output, clear all stores and permit instance reuse", async () => {
  for (const mode of ["read", "write", "resize", "flush", "source", "sink", "host-state"]) {
    const instance = await newInstance(); const scratch = stores(); const failure = new Error(mode);
    if (mode === "read") scratch[0].readAt = async (_offset, length) => new Uint8Array(length + 1);
    if (mode === "write") scratch[0].writeAt = async (_offset, bytes) => bytes.length + 1;
    if (mode === "resize") { const original = scratch[0].resize; scratch[0].resize = async (size) => { if (size !== 0n) throw failure; await original(size); }; }
    if (mode === "flush") scratch[0].flush = async () => { throw failure; };
    const input = mode === "source" ? { size: 1024n, async readAt() { throw failure; } } : source();
    const output = mode === "sink" ? { async writeChunk() { throw failure; }, async flush() {} } : sink();
    const wasm = mode === "host-state" ? { ...instance.exports, caj2pdf_hnc8_add_state: () => 0 } : instance;
    await assert.rejects(convert(wasm, input, output, { hnc8: { qmStates, scratch } }));
    assert.ok(scratch.every((store) => store.size === 0n));
    const report = await convert(instance, source(), sink(), { hnc8: { qmStates, scratch: stores() } });
    assert.equal(report.pagesConverted, 1);
  }
});

test("conversion and cleanup failures are both retained and every store is attempted", async () => {
  const scratch = stores(); const primary = new Error("output failed"); const cleanup = new Error("cleanup failed");
  const attempted = [];
  scratch.forEach((store, index) => { store.resize = async () => { attempted.push(index); if (index === 0) throw cleanup; }; });
  await assert.rejects(convert(await newInstance(), source(), { async writeChunk() { throw primary; }, async flush() {} }, { hnc8: { qmStates, scratch } }), (error) => {
    assert.ok(error instanceof AggregateError); assert.deepEqual(error.errors, [primary, cleanup]); return true;
  });
  assert.deepEqual(attempted, [0, 1, 2, 3]);
});

test("HN yields to scheduled cancellation and clears pending workspace contents", async () => {
  const scratch = stores(); const controller = new AbortController(); const reason = new Error("stop HN");
  const write = scratch[0].writeAt;
  let scheduled = false;
  scratch[0].writeAt = async (...args) => {
    const count = await write(...args);
    if (!scheduled) { scheduled = true; setTimeout(() => controller.abort(reason), 0); }
    return count;
  };
  await assert.rejects(convert(await newInstance(), source(), sink(), { chunkSize: 1, signal: controller.signal, hnc8: { qmStates, scratch } }), (error) => error === reason);
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
        assert.ok(offset + BigInt(length) <= BigInt(boundary), "inspection must not request image/page payloads");
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

test("HN inspection rejects malformed outlines, limits and cancelled reads", async () => {
  const { inspect } = await import("../node.mjs");
  const malformed = syntheticHn(true);
  new DataView(malformed.buffer).setUint32(0x15c + 308 + 304, 4, true);
  await assert.rejects(inspect(await newInstance(), blobSource(new Blob([malformed]))), (error) => error.code === "HNC8" && /outline level/.test(error.message));
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
  for (const bytes of [syntheticHn(true, true), syntheticPrefixedHn()]) {
    const scratch = stores(); const parts = [];
    const report = await convert(await newInstance(), blobSource(new Blob([bytes])), sink(parts), { chunkSize: 3, hnc8: { qmStates, scratch } });
    assert.equal(report.pagesConverted, 1);
    assert.ok(scratch.every((store) => store.size === 0n));
    outputs.push(Buffer.concat(parts));
  }
  assert.deepEqual(outputs[0], outputs[1]);
  await validateMultiImageHn(t, outputs[1]);
});

// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { open, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { blobSource, convert, fileHandleScratch } from "../node.mjs";
import { newInstance, tempDirectory, validatePdf } from "./helpers.mjs";
import { qmStates, syntheticHn } from "./hnc8-fixtures.mjs";

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

test("real WASM composes asymmetric HN pixels through bounded Node scratch", async (t) => {
  const directory = await tempDirectory("hnc8");
  const handles = [];
  try {
    const scratch = [];
    for (let i = 0; i < 4; i++) {
      const handle = await open(join(directory, `${i}`), "wx+"); handles.push(handle);
      scratch.push(await fileHandleScratch(handle, { maxBytes: 1024n }));
    }
    const parts = [];
    const report = await convert(await newInstance(), source(), sink(parts), { chunkSize: 7, hnc8: { qmStates, scratch } });
    assert.equal(report.pagesConverted, 1);
    assert.equal(report.format, "hn");
    const pdf = Buffer.concat(parts);
    await validatePdf(t, pdf, 1);
    const path = join(directory, "output.pdf"); await writeFile(path, pdf);
    const info = JSON.parse(execFileSync("qpdf", ["--json", "--json-key=pages", path], { encoding: "utf8" }));
    const image = info.pages[0].images[0];
    assert.equal(image.width, 32); assert.equal(image.height, 2);
    const packed = execFileSync("qpdf", [`--show-object=${image.object.split(" ")[0]}`, "--filtered-stream-data", path]);
    assert.deepEqual([...packed], [0x40, 0, 0, 0, 0xa0, 0, 0, 0]);
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

test("HN requires codec states and random-access scratch explicitly", async () => {
  await assert.rejects(convert(await newInstance(), source(), sink()), { code: "HNC8" });
  await assert.rejects(convert(await newInstance(), source(), sink(), { hnc8: { qmStates } }), { code: "RANDOM_ACCESS_REQUIRED" });
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

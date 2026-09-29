// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { mkdtemp, open, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileHandleScratch } from "../node.mjs";
import { syncAccessHandleScratch } from "../browser.mjs";
import { MAX_IO_CHUNK } from "../io.mjs";

async function boundaries(store) {
  await store.resize(8n);
  await assert.rejects(store.resize(33n), RangeError);
  assert.equal(store.size, 8n);
  for (const offset of [-1n, 7n, 9n, (1n << 64n) - 1n, 0]) {
    await assert.rejects(store.readAt(offset, 2), RangeError);
    await assert.rejects(store.writeAt(offset, new Uint8Array(2)), RangeError);
  }
  await assert.rejects(store.readAt(0n, MAX_IO_CHUNK + 1), RangeError);
  await assert.rejects(store.writeAt(0n, new Uint8Array(MAX_IO_CHUNK + 1)), RangeError);
  await assert.rejects(store.writeAt(0n, [1]), TypeError);
  assert.equal((await store.readAt(8n, 0)).length, 0);
  assert.equal(await store.writeAt(8n, new Uint8Array()), 0);
  const signal = AbortSignal.abort(new Error("stop scratch"));
  for (const operation of [
    () => store.resize(16n, signal),
    () => store.readAt(0n, 1, signal),
    () => store.writeAt(0n, new Uint8Array([9]), signal),
    () => store.flush(signal),
  ]) await assert.rejects(operation(), /stop scratch/);
  assert.equal(store.size, 8n);
  assert.deepEqual(await store.readAt(0n, 8), new Uint8Array(8));
}

test("Node scratch positions, bounds and reuses a real caller-owned file", async () => {
  const directory = await mkdtemp(join(tmpdir(), "caj2pdf-scratch-"));
  const path = join(directory, "rows");
  const file = await open(path, "wx+");
  try {
    const store = await fileHandleScratch(file, { maxBytes: 32n });
    await boundaries(store);
    assert.equal(await store.writeAt(4n, new Uint8Array([7, 8])), 2);
    assert.equal(await store.writeAt(0n, new Uint8Array([1, 2])), 2);
    await store.flush();
    assert.deepEqual(await store.readAt(0n, 8), new Uint8Array([1, 2, 0, 0, 7, 8, 0, 0]));
    await store.resize(0n);
    await store.resize(16n);
    assert.deepEqual(await store.readAt(0n, 16), new Uint8Array(16));
    assert.equal((await file.stat()).size, 16, "adapter leaves handle open");
    await assert.rejects(fileHandleScratch(file, { maxBytes: 15n }), RangeError);
  } finally {
    await file.close();
    await rm(directory, { recursive: true });
  }
});

function nodeHandle(overrides = {}) {
  return {
    stat: async () => ({ size: 8n, isFile: () => true }),
    truncate: async () => {},
    read: async ({ buffer }) => { buffer[0] = 42; return { bytesRead: 1 }; },
    write: async () => ({ bytesWritten: 1 }),
    ...overrides,
  };
}

function opfsHandle(overrides = {}) {
  let bytes = new Uint8Array(0);
  return {
    getSize: () => bytes.length,
    truncate(length) { const next = new Uint8Array(length); next.set(bytes.subarray(0, length)); bytes = next; },
    read(output, { at }) { const part = bytes.subarray(at, at + output.length); output.set(part); return part.length; },
    write(input, { at }) { bytes.set(input, at); return input.length; },
    flush() {},
    ...overrides,
  };
}

test("OPFS scratch bounds and immediate read-after-write without file snapshots", async () => {
  const store = syncAccessHandleScratch(opfsHandle(), { maxBytes: 32n });
  await boundaries(store);
  await store.writeAt(1n, new Uint8Array([4, 5]));
  assert.deepEqual(await store.readAt(0n, 4), new Uint8Array([0, 4, 5, 0]));
  await store.resize(0n);
  await store.resize(16n);
  assert.deepEqual(await store.readAt(0n, 16), new Uint8Array(16));
  await store.flush();
});

test("scratch constructors reject unusable handles and inexact storage sizes", async () => {
  await assert.rejects(fileHandleScratch(null, { maxBytes: 32n }), TypeError);
  assert.throws(() => syncAccessHandleScratch(null, { maxBytes: 32n }), TypeError);
  for (const maxBytes of [undefined, -1n, 32, 1n << 53n]) {
    await assert.rejects(fileHandleScratch(nodeHandle(), { maxBytes }), RangeError);
    assert.throws(() => syncAccessHandleScratch(opfsHandle(), { maxBytes }), RangeError);
  }
  await assert.rejects(fileHandleScratch(nodeHandle({ stat: async () => ({ size: 0n, isFile: () => false }) }), { maxBytes: 32n }), TypeError);
  for (const size of [-1, NaN, 2 ** 53, 33]) {
    assert.throws(() => syncAccessHandleScratch(opfsHandle({ getSize: () => size }), { maxBytes: 32n }), RangeError);
  }
});

test("scratch preserves short results and rejects invalid host counts", async () => {
  const partial = await fileHandleScratch(nodeHandle(), { maxBytes: 32n });
  assert.deepEqual(await partial.readAt(0n, 4), new Uint8Array([42]));
  assert.equal(await partial.writeAt(0n, new Uint8Array(4)), 1);
  for (const count of [-1, NaN, undefined, 5]) {
    const node = await fileHandleScratch(nodeHandle({
      read: async () => ({ bytesRead: count }), write: async () => ({ bytesWritten: count }),
    }), { maxBytes: 32n });
    const browser = syncAccessHandleScratch(opfsHandle({ getSize: () => 8, read: () => count, write: () => count }), { maxBytes: 32n });
    for (const store of [node, browser]) {
      await assert.rejects(store.readAt(0n, 4), RangeError);
      await assert.rejects(store.writeAt(0n, new Uint8Array(4)), RangeError);
    }
  }
  const zero = syncAccessHandleScratch(opfsHandle({ getSize: () => 8, read: () => 0, write: () => 0 }), { maxBytes: 32n });
  assert.equal((await zero.readAt(0n, 4)).length, 0);
  assert.equal(await zero.writeAt(0n, new Uint8Array(4)), 0);
});

test("Node cancellation waits for writes and resize; completed extent stays accurate", async () => {
  for (const method of ["write", "truncate", "read"]) {
    let finish;
    const pending = new Promise((resolve) => { finish = resolve; });
    const store = await fileHandleScratch(nodeHandle({ [method]: () => pending }), { maxBytes: 32n });
    const controller = new AbortController();
    let settled = false;
    const operation = method === "write" ? store.writeAt(0n, new Uint8Array([1]), controller.signal)
      : method === "read" ? store.readAt(0n, 1, controller.signal)
      : store.resize(16n, controller.signal);
    const rejected = assert.rejects(operation.finally(() => { settled = true; }), /cancelled/);
    controller.abort(new Error("cancelled"));
    await Promise.resolve();
    assert.equal(settled, false, "cleanup must not race outstanding file I/O");
    finish({ bytesWritten: 1, bytesRead: 1 });
    await rejected;
    assert.equal(store.size, method === "truncate" ? 16n : 8n);
  }
});

test("scratch preserves host failures without claiming a successful resize", async () => {
  const failure = new Error("storage failure");
  const fail = () => { throw failure; };
  const node = await fileHandleScratch(nodeHandle({ truncate: fail, read: fail, write: fail }), { maxBytes: 32n });
  const browser = syncAccessHandleScratch(opfsHandle({ getSize: () => 8, truncate: fail, read: fail, write: fail, flush: fail }), { maxBytes: 32n });
  for (const store of [node, browser]) {
    await assert.rejects(store.resize(16n), (error) => error === failure);
    await assert.rejects(store.readAt(0n, 1), (error) => error === failure);
    await assert.rejects(store.writeAt(0n, new Uint8Array([1])), (error) => error === failure);
    assert.equal(store.size, 8n);
  }
  await assert.rejects(browser.flush(), (error) => error === failure);
});

test("scratch offsets remain exact beyond 32-bit positions", async () => {
  const maxBytes = BigInt(Number.MAX_SAFE_INTEGER);
  const offset = maxBytes - 4n;
  const positions = [];
  const node = await fileHandleScratch(nodeHandle({
    stat: async () => ({ size: maxBytes, isFile: () => true }),
    read: async ({ position }) => { positions.push(position); return { bytesRead: 4 }; },
    write: async (_bytes, _start, _length, position) => { positions.push(position); return { bytesWritten: 4 }; },
  }), { maxBytes });
  const browser = syncAccessHandleScratch(opfsHandle({
    getSize: () => Number.MAX_SAFE_INTEGER,
    read: (_bytes, { at }) => { positions.push(at); return 4; },
    write: (_bytes, { at }) => { positions.push(at); return 4; },
  }), { maxBytes });
  for (const store of [node, browser]) {
    assert.equal((await store.readAt(offset, 4)).length, 4);
    assert.equal(await store.writeAt(offset, new Uint8Array(4)), 4);
    await assert.rejects(store.readAt(offset + 1n, 4), RangeError);
  }
  assert.deepEqual(positions, Array(4).fill(Number(offset)));
});

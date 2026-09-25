// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { open, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Writable } from "node:stream";
import { test } from "node:test";
import {
  blobSource,
  MAX_IO_CHUNK,
  webWritableSink,
} from "../io.mjs";
import { fileHandleSource, nodeWritableSink } from "../node.mjs";

test("Blob source reads a slice and never calls whole-Blob arrayBuffer", async () => {
  const content = new Blob([Uint8Array.from([1, 2, 3, 4, 5])]);
  const ranges = [];
  const tracked = {
    size: content.size,
    slice(start, end) {
      ranges.push([start, end]);
      return content.slice(start, end);
    },
    arrayBuffer() {
      throw new Error("whole Blob read is forbidden");
    },
  };
  const source = blobSource(tracked);
  assert.equal(source.size, 5n);
  assert.deepEqual([...await source.readAt(2n, 2)], [3, 4]);
  assert.deepEqual(ranges, [[2, 4]]);
  await assert.rejects(source.readAt(0n, MAX_IO_CHUNK + 1), RangeError);
  await assert.rejects(source.readAt(4n, 2), RangeError);
});

test("Blob source rejects unsafe numeric sizes and cancellation", async () => {
  assert.throws(() => blobSource({ size: Number.MAX_SAFE_INTEGER + 1, slice() {} }), TypeError);
  const source = blobSource(new Blob([Uint8Array.of(1)]));
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(source.readAt(0n, 1, controller.signal), { name: "AbortError" });
});

test("Blob source reports a truncated slice with a typed error", async () => {
  const source = blobSource({
    size: 2,
    slice() { return new Blob([Uint8Array.of(1)]); },
  });
  await assert.rejects(source.readAt(0n, 2), { code: "TRUNCATED_INPUT" });
});

test("Node source uses BigInt positioned reads beyond Number.MAX_SAFE_INTEGER", async () => {
  const positions = [];
  const high = BigInt(Number.MAX_SAFE_INTEGER) + 7n;
  const handle = {
    async stat(options) {
      assert.deepEqual(options, { bigint: true });
      return { size: high + 4n };
    },
    async read({ buffer, offset, length, position }) {
      assert.equal(typeof position, "bigint");
      positions.push(position);
      const count = Math.min(length, 2);
      buffer.set([10 + offset, 11 + offset].slice(0, count), offset);
      return { bytesRead: count };
    },
  };
  const source = await fileHandleSource(handle);
  assert.equal(source.size, high + 4n);
  assert.deepEqual([...await source.readAt(high, 4)], [10, 11, 12, 13]);
  assert.deepEqual(positions, [high, high + 2n]);
});

test("Node source leaves a real FileHandle open and preserves its cursor", async () => {
  const directory = await mkdtemp(join(tmpdir(), "caj2pdf-node-source-"));
  const path = join(directory, "input.bin");
  await writeFile(path, Uint8Array.from([1, 2, 3, 4]));
  const handle = await open(path, "r");
  try {
    const source = await fileHandleSource(handle);
    assert.deepEqual([...await source.readAt(2n, 2)], [3, 4]);
    const next = new Uint8Array(2);
    const { bytesRead } = await handle.read({ buffer: next, offset: 0, length: 2, position: 0n });
    assert.equal(bytesRead, 2);
    assert.deepEqual([...next], [1, 2]);
  } finally {
    await handle.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("Node source reports a file shortened after its size snapshot", async () => {
  const source = await fileHandleSource({
    async stat() { return { size: 3n }; },
    async read() { return { bytesRead: 0 }; },
  });
  await assert.rejects(source.readAt(0n, 3), { code: "TRUNCATED_INPUT" });
});

test("Node Writable sink waits for the callback even when write returns false", async () => {
  const writable = new EventEmitter();
  let complete;
  let received;
  writable.write = (bytes, callback) => {
    received = bytes;
    complete = callback;
    return false;
  };
  const sink = nodeWritableSink(writable);
  let settled = false;
  const pending = sink.writeChunk(Uint8Array.from([7, 8])).then((value) => {
    settled = true;
    return value;
  });
  await Promise.resolve();
  assert.equal(settled, false);
  assert.deepEqual([...received], [7, 8]);
  complete();
  assert.equal(await pending, 2);
  assert.equal(settled, true);
  await sink.flush();
});

test("Node Writable sink catches a callback error and the later error event", async () => {
  const writable = new Writable({
    write(_bytes, _encoding, callback) {
      callback(new Error("sink failed"));
    },
  });
  await assert.rejects(nodeWritableSink(writable).writeChunk(Uint8Array.of(1)), /sink failed/);
  await new Promise((resolve) => setImmediate(resolve));
});

test("Web Writable sink awaits writer.write and leaves the writer open", async () => {
  let release;
  let received;
  const writer = {
    ready: Promise.resolve(),
    write(bytes) {
      received = bytes;
      return new Promise((resolve) => { release = resolve; });
    },
  };
  const sink = webWritableSink(writer);
  let settled = false;
  const input = Uint8Array.from([4, 5]);
  const pending = sink.writeChunk(input).then((value) => {
    settled = true;
    return value;
  });
  input[0] = 99;
  await Promise.resolve();
  assert.equal(settled, false);
  assert.deepEqual([...received], [4, 5]);
  release();
  assert.equal(await pending, 2);
  await sink.flush();
});

test("an abort interrupts stalled Node sink writes and file reads", async () => {
  const controller = new AbortController();
  const writable = new EventEmitter();
  writable.write = () => false;
  const pending = nodeWritableSink(writable).writeChunk(Uint8Array.of(1), controller.signal);
  const source = await fileHandleSource({
    async stat() { return { size: 1n }; },
    read: () => new Promise(() => {}),
  });
  const read = source.readAt(0n, 1, controller.signal);
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
  await assert.rejects(read, { name: "AbortError" });
});

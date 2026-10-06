// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Writable } from "node:stream";
import { test } from "node:test";
import { webWritableSink } from "../io.mjs";
import { convert, inspect, nodeWritableSink } from "../node.mjs";
import { discard, syntheticCaj, tempDirectory, wasmModule } from "./helpers.mjs";

test("Node inputs must be regular files, descriptors, or Blobs", async () => {
  const directory = await tempDirectory("node-input");
  try {
    await mkdir(join(directory, "folder"));
    for (const input of [join(directory, "folder"), -1, 1.5, {}, null, new Uint8Array(4)]) {
      await assert.rejects(convert(await wasmModule(), input, discard), TypeError, String(input));
    }
    await assert.rejects(inspect(await wasmModule(), join(directory, "missing")), { code: "ENOENT" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("a Blob that returns short or empty slices ends with a typed truncation", async () => {
  const bytes = syntheticCaj();
  class ShortBlob extends Blob {
    slice(start, end) {
      // One byte per read, then nothing at all past the first kilobyte.
      return start >= 1024 ? new Blob([]) : super.slice(start, Math.min(end, start + 1));
    }
  }
  await assert.rejects(convert(await wasmModule(), new ShortBlob([bytes]), discard), {
    code: "TRUNCATED_INPUT",
  });
});

test("a file shortened after its size snapshot ends with a typed truncation", async () => {
  const directory = await tempDirectory("node-truncated");
  const path = join(directory, "input.caj");
  try {
    await writeFile(path, syntheticCaj());
    const pending = convert(await wasmModule(), path, discard, { chunkSize: 1, progress() {} });
    await writeFile(path, syntheticCaj().subarray(0, 8));
    await assert.rejects(pending, (error) => ["TRUNCATED_INPUT", "MALFORMED_CAJ", "UNSUPPORTED_FORMAT"].includes(error.code));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
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

test("Web Writable sink awaits writer.write, hands over the bytes, and leaves the writer open", async () => {
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
  await Promise.resolve();
  assert.equal(settled, false);
  // Each chunk is a fresh copy from the Worker, so the writer may keep it.
  assert.equal(received, input);
  release();
  assert.equal(await pending, 2);
  await sink.flush();
});

test("an abort interrupts a stalled Node sink write", async () => {
  const controller = new AbortController();
  const writable = new EventEmitter();
  writable.write = () => false;
  const pending = nodeWritableSink(writable).writeChunk(Uint8Array.of(1), controller.signal);
  controller.abort();
  await assert.rejects(pending, { name: "AbortError" });
});

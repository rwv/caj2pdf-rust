// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { open, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Writable } from "node:stream";
import { finished } from "node:stream/promises";
import { test } from "node:test";
import { blobSource, convertKdhProof, copyRangeProof, DEFAULT_IO_CHUNK, MAX_IO_CHUNK, webWritableSink } from "../io.mjs";
import { fileHandleSource, nodeWritableSink } from "../node.mjs";

const wasmPath = new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url);

async function newInstance() {
  const bytes = await readFile(wasmPath);
  return (await WebAssembly.instantiate(bytes)).instance;
}

async function syntheticKdh() {
  const pdf = new Uint8Array(await readFile(new URL("../../tests/fixtures/valid_out_of_order_objects.pdf", import.meta.url)));
  const wrapped = new Uint8Array(254 + pdf.length);
  wrapped.set(new TextEncoder().encode("KDH 2.00 Copyright(C) 2000 CAJCD"));
  wrapped.set([0, 0, 2, 0], 0x28);
  const key = new TextEncoder().encode("FZHMEI");
  for (let index = 0; index < pdf.length; index += 1) {
    wrapped[254 + index] = pdf[index] ^ key[index % key.length];
  }
  return { pdf, wrapped };
}

test("Blob proof awaits three bounded slices through the real WASM core future", async () => {
  const payload = Uint8Array.from(
    { length: 2 * DEFAULT_IO_CHUNK + 17 },
    (_, index) => index % 251,
  );
  const blob = new Blob([payload]);
  const ranges = [];
  const source = blobSource({
    size: blob.size,
    slice(start, end) {
      ranges.push([start, end]);
      assert.ok(end - start <= DEFAULT_IO_CHUNK);
      return blob.slice(start, end);
    },
    arrayBuffer() {
      throw new Error("whole Blob.arrayBuffer() is forbidden");
    },
  });
  const chunks = [];
  const writer = new WritableStream({
    async write(chunk) {
      assert.ok(chunk.byteLength <= DEFAULT_IO_CHUNK);
      chunks.push(chunk);
      await Promise.resolve();
    },
  }).getWriter();
  const report = await copyRangeProof(await newInstance(), source, webWritableSink(writer));
  writer.releaseLock();
  assert.deepEqual(ranges, [
    [0, DEFAULT_IO_CHUNK],
    [DEFAULT_IO_CHUNK, 2 * DEFAULT_IO_CHUNK],
    [2 * DEFAULT_IO_CHUNK, payload.length],
  ]);
  assert.deepEqual([...Buffer.concat(chunks)], [...payload]);
  assert.deepEqual(report, {
    inputBytesRead: BigInt(payload.length),
    outputBytesWritten: BigInt(payload.length),
    pagesConverted: 0,
    bookmarksWritten: 0,
  });
});

test("Node positioned source uses the same WASM contract", async () => {
  const directory = await mkdtemp(join(tmpdir(), "caj2pdf-wasm-node-"));
  const path = join(directory, "input.bin");
  const payload = Uint8Array.from({ length: 2 * 4096 + 19 }, (_, index) => index % 251);
  await writeFile(path, payload);
  const handle = await open(path, "r");
  try {
    const source = await fileHandleSource(handle);
    const chunks = [];
    const writable = new Writable({
      highWaterMark: 1024,
      write(chunk, _encoding, callback) {
        chunks.push(chunk);
        setImmediate(callback);
      },
    });
    const report = await copyRangeProof(await newInstance(), source, nodeWritableSink(writable), { chunkSize: 4096 });
    writable.end();
    await finished(writable);
    assert.deepEqual([...Buffer.concat(chunks)], [...payload]);
    assert.equal(report.outputBytesWritten, BigInt(payload.length));
    const again = await source.readAt(0n, 1);
    assert.equal(again[0], payload[0]);
  } finally {
    await handle.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the real WASM core converts a synthetic KDH through bounded browser I/O", async () => {
  const { pdf, wrapped } = await syntheticKdh();
  const chunks = [];
  const writer = new WritableStream({
    async write(bytes) {
      chunks.push(bytes);
    },
  }).getWriter();
  const report = await convertKdhProof(
    await newInstance(),
    blobSource(new Blob([wrapped])),
    webWritableSink(writer),
    { chunkSize: 4096 },
  );
  writer.releaseLock();
  assert.deepEqual(new Uint8Array(Buffer.concat(chunks)), pdf);
  assert.equal(report.pagesConverted, 2);
  assert.equal(report.outputBytesWritten, BigInt(pdf.length));
});

test("the same WASM KDH core converts through positioned Node file I/O", async () => {
  const { pdf, wrapped } = await syntheticKdh();
  const directory = await mkdtemp(join(tmpdir(), "caj2pdf-wasm-kdh-"));
  const path = join(directory, "input.caj");
  await writeFile(path, wrapped);
  const handle = await open(path, "r");
  try {
    const chunks = [];
    const writable = new Writable({
      write(chunk, _encoding, callback) {
        chunks.push(chunk);
        callback();
      },
    });
    const report = await convertKdhProof(
      await newInstance(),
      await fileHandleSource(handle),
      nodeWritableSink(writable),
      { chunkSize: 4096 },
    );
    writable.end();
    await finished(writable);
    assert.deepEqual(new Uint8Array(Buffer.concat(chunks)), pdf);
    assert.equal(report.pagesConverted, 2);
  } finally {
    await handle.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("the real WASM KDH entrypoint reports a typed malformed wrapper", async () => {
  await assert.rejects(
    convertKdhProof(
      await newInstance(),
      blobSource(new Blob([new Uint8Array(254)])),
      { async writeChunk(bytes) { return bytes.length; }, async flush() {} },
    ),
    { code: "MALFORMED_KDH" },
  );
});

test("WASM does not request a second read while a write is pending", async () => {
  const reads = [];
  let release;
  const source = {
    size: 8n,
    async readAt(offset, length) {
      reads.push([offset, length]);
      return Uint8Array.from({ length }, (_, index) => Number(offset) + index);
    },
  };
  const sink = {
    async writeChunk(bytes) {
      if (release == null) {
        await new Promise((resolve) => { release = resolve; });
      }
      return bytes.byteLength;
    },
    async flush() {},
  };
  const pending = copyRangeProof(await newInstance(), source, sink, { chunkSize: 4 });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(reads, [[0n, 4]]);
  assert.equal(typeof release, "function");
  release();
  await pending;
  assert.deepEqual(reads, [[0n, 4], [4n, 4]]);
});

test("a busy WASM instance rejects a second proof without cancelling the first", async () => {
  const instance = await newInstance();
  let release;
  const source = {
    size: 2n,
    async readAt() { return Uint8Array.of(1, 2); },
  };
  const sink = {
    async writeChunk(bytes) {
      await new Promise((resolve) => { release = resolve; });
      return bytes.byteLength;
    },
    async flush() {},
  };
  const first = copyRangeProof(instance, source, sink);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(typeof release, "function");
  await assert.rejects(copyRangeProof(instance, source, sink), /already has an active/);
  release();
  assert.equal((await first).outputBytesWritten, 2n);
});

test("WASM handles short source reads and short sink writes", async () => {
  const source = {
    size: 5n,
    async readAt(offset, length) {
      return Uint8Array.of(Number(offset) + 1).subarray(0, length);
    },
  };
  const output = [];
  const report = await copyRangeProof(await newInstance(), source, {
    async writeChunk(bytes) {
      output.push(bytes[0]);
      return 1;
    },
    async flush() {},
  }, { chunkSize: 3 });
  assert.deepEqual(output, [1, 2, 3, 4, 5]);
  assert.equal(report.inputBytesRead, 5n);
  assert.equal(report.outputBytesWritten, 5n);
});

test("WASM returns typed resource and truncation errors without panicking", async () => {
  const sink = { async writeChunk(bytes) { return bytes.length; }, async flush() {} };
  await assert.rejects(
    copyRangeProof(await newInstance(), { size: 8n * 1024n ** 3n + 1n, async readAt() {} }, sink),
    { code: "LIMIT_EXCEEDED" },
  );
  await assert.rejects(
    copyRangeProof(await newInstance(), { size: 2n, async readAt() { return new Uint8Array(); } }, sink),
    { code: "TRUNCATED_INPUT" },
  );
  await assert.rejects(
    copyRangeProof(await newInstance(), blobSource(new Blob([Uint8Array.of(1)])), sink, { chunkSize: MAX_IO_CHUNK + 1 }),
    RangeError,
  );
});

test("the JS bridge names the KDH error category", async () => {
  const wasm = { exports: {
    memory: new WebAssembly.Memory({ initial: 1 }),
    caj2pdf_io_start: () => 0,
    caj2pdf_io_poll: () => 5,
    caj2pdf_io_error_kind: () => 15,
    caj2pdf_io_cancel: () => {},
    caj2pdf_io_reset: () => {},
  } };
  await assert.rejects(
    copyRangeProof(
      wasm,
      { size: 0n, async readAt() { throw new Error("unused"); } },
      { async writeChunk() { throw new Error("unused"); }, async flush() {} },
    ),
    { code: "MALFORMED_KDH" },
  );
});

test("raw WASM ABI rejects oversized completions without corrupting the future", async () => {
  const { exports } = await newInstance();
  assert.equal(exports.caj2pdf_io_start(4n, 0n, 4n, 2), 0);
  try {
    assert.equal(exports.caj2pdf_io_poll(), 1);
    assert.equal(exports.caj2pdf_io_request_length(), 2);
    assert.equal(exports.caj2pdf_io_complete_read(3), 0);
    assert.equal(exports.caj2pdf_io_poll(), 1);
    new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), 2).set([1, 2]);
    assert.equal(exports.caj2pdf_io_complete_read(2), 1);
    assert.equal(exports.caj2pdf_io_poll(), 2);
    assert.equal(exports.caj2pdf_io_complete_write(3), 0);
    assert.equal(exports.caj2pdf_io_poll(), 2);
  } finally {
    exports.caj2pdf_io_reset();
  }
});

test("WASM proof rejects cancellation after an awaited source read", async () => {
  const controller = new AbortController();
  const source = {
    size: 1n,
    async readAt() {
      controller.abort();
      return Uint8Array.of(1);
    },
  };
  await assert.rejects(
    copyRangeProof(await newInstance(), source, {
      async writeChunk() { throw new Error("must not write"); },
      async flush() {},
    }, { signal: controller.signal }),
    { name: "AbortError" },
  );
});

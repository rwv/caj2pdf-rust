// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { test } from "node:test";
import { blobSource, convert, DEFAULT_LIMITS, MAX_IO_CHUNK } from "../io.mjs";
import { fixture, newInstance } from "./helpers.mjs";

const PDF = "valid_out_of_order_objects.pdf";

function bytesSource(bytes, reads = []) {
  return {
    size: BigInt(bytes.length),
    async readAt(offset, length) {
      reads.push([offset, length]);
      return bytes.subarray(Number(offset), Number(offset) + length);
    },
  };
}

test("WASM does not request another read while a write is pending", async () => {
  const pdf = await fixture(PDF);
  const reads = [];
  let release;
  let readsAtWrite;
  const sink = {
    async writeChunk(bytes) {
      if (release == null) {
        readsAtWrite = reads.length;
        await new Promise((resolve) => { release = resolve; });
      }
      return bytes.byteLength;
    },
    async flush() {},
  };
  const pending = convert(await newInstance(), bytesSource(pdf, reads), sink, { chunkSize: 64 });
  while (release == null) await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(reads.length, readsAtWrite);
  release();
  assert.equal((await pending).outputBytesWritten, BigInt(pdf.length));
  assert.ok(reads.length > readsAtWrite);
});

test("a busy WASM instance rejects a second operation without cancelling the first", async () => {
  const pdf = await fixture(PDF);
  const instance = await newInstance();
  let release;
  const sink = {
    async writeChunk(bytes) {
      if (release == null) await new Promise((resolve) => { release = resolve; });
      return bytes.byteLength;
    },
    async flush() {},
  };
  const first = convert(instance, bytesSource(pdf), sink);
  while (release == null) await new Promise((resolve) => setImmediate(resolve));
  await assert.rejects(convert(instance, bytesSource(pdf), sink), /already has an active/);
  release();
  assert.equal((await first).outputBytesWritten, BigInt(pdf.length));
});

test("WASM handles short source reads and short sink writes", async () => {
  const pdf = await fixture(PDF);
  const source = {
    size: BigInt(pdf.length),
    async readAt(offset, length) {
      return pdf.subarray(Number(offset), Number(offset) + Math.min(length, 1));
    },
  };
  const output = [];
  const report = await convert(await newInstance(), source, {
    async writeChunk(bytes) {
      output.push(bytes[0]);
      return 1;
    },
    async flush() {},
  }, { chunkSize: 3 });
  assert.deepEqual(output, [...pdf]);
  assert.equal(report.outputBytesWritten, BigInt(pdf.length));
});

test("WASM returns typed resource and truncation errors without panicking", async () => {
  const sink = { async writeChunk(bytes) { return bytes.length; }, async flush() {} };
  await assert.rejects(
    convert(await newInstance(), { size: DEFAULT_LIMITS.maxInputBytes + 1n, async readAt() {} }, sink),
    { code: "LIMIT_EXCEEDED" },
  );
  await assert.rejects(
    convert(await newInstance(), { size: 2n, async readAt() { return new Uint8Array(); } }, sink),
    { code: "TRUNCATED_INPUT" },
  );
  await assert.rejects(
    convert(await newInstance(), blobSource(new Blob([Uint8Array.of(1)])), sink, { chunkSize: MAX_IO_CHUNK + 1 }),
    RangeError,
  );
});

test("the JS bridge names an error category without a Rust message", async () => {
  const wasm = { exports: {
    memory: new WebAssembly.Memory({ initial: 1 }),
    caj2pdf_start: () => 0,
    caj2pdf_io_poll: () => 5,
    caj2pdf_io_error_kind: () => 15,
    caj2pdf_io_message_ptr: () => 0,
    caj2pdf_io_message_len: () => 0,
    caj2pdf_io_cancel: () => {},
    caj2pdf_io_reset: () => {},
  } };
  await assert.rejects(
    convert(
      wasm,
      { size: 0n, async readAt() { throw new Error("unused"); } },
      { async writeChunk() { throw new Error("unused"); }, async flush() {} },
    ),
    { code: "MALFORMED_KDH", message: "conversion failed: MALFORMED_KDH" },
  );
});

test("raw WASM ABI rejects oversized completions without corrupting the future", async () => {
  const { exports } = await newInstance();
  const limits = [
    DEFAULT_LIMITS.maxInputBytes,
    DEFAULT_LIMITS.maxOutputBytes,
    DEFAULT_LIMITS.maxAllocationBytes,
    DEFAULT_LIMITS.maxPages,
    DEFAULT_LIMITS.maxBookmarks,
  ];
  assert.equal(exports.caj2pdf_start(1, 4n, 2, 0, 1, ...limits), 0);
  try {
    assert.equal(exports.caj2pdf_io_poll(), 1);
    assert.equal(exports.caj2pdf_io_request_length(), 2);
    assert.equal(exports.caj2pdf_io_complete_read(3), 0);
    assert.equal(exports.caj2pdf_io_complete_write(2), 0);
    assert.equal(exports.caj2pdf_io_poll(), 1);
    new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), 2).set([0x25, 0x50]);
    assert.equal(exports.caj2pdf_io_complete_read(2), 1);
    assert.equal(exports.caj2pdf_io_complete_read(2), 0);
    assert.equal(exports.caj2pdf_io_poll(), 1);
  } finally {
    exports.caj2pdf_io_reset();
  }
});

test("WASM conversion rejects cancellation after an awaited source read", async () => {
  const controller = new AbortController();
  const source = {
    size: 1n,
    async readAt() {
      controller.abort();
      return Uint8Array.of(1);
    },
  };
  await assert.rejects(
    convert(await newInstance(), source, {
      async writeChunk() { throw new Error("must not write"); },
      async flush() {},
    }, { signal: controller.signal }),
    { name: "AbortError" },
  );
});

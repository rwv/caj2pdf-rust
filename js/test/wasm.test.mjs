// SPDX-License-Identifier: MIT

// The raw WASM ABI, called directly on this thread with plain imports. The
// public API makes the same calls from its Worker.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DEFAULT_LIMITS, MAX_IO_CHUNK } from "../io.mjs";
import { fixture, wasmModule } from "./helpers.mjs";

const PDF = "valid_out_of_order_objects.pdf";
const LIMITS = [
  DEFAULT_LIMITS.maxInputBytes,
  DEFAULT_LIMITS.maxOutputBytes,
  DEFAULT_LIMITS.maxAllocationBytes,
  DEFAULT_LIMITS.maxPages,
  DEFAULT_LIMITS.maxBookmarks,
];

/** Instantiate the module with imports serving `bytes`; `hooks` override them. */
async function rawInstance(bytes, hooks = {}) {
  const state = { output: [], flushes: 0, progress: [] };
  let memory;
  const imports = {
    caj2pdf_read(resource, offset, pointer, length) {
      const chunk = bytes.subarray(offset, offset + length);
      new Uint8Array(memory.buffer, pointer, length).set(chunk);
      return chunk.length;
    },
    caj2pdf_write(pointer, length) {
      state.output.push(...new Uint8Array(memory.buffer, pointer, length));
      return length;
    },
    caj2pdf_flush() {
      state.flushes += 1;
      return 0;
    },
    caj2pdf_progress(done, total) {
      state.progress.push([done, total]);
    },
    caj2pdf_cancelled() {
      return 0;
    },
  };
  const wrapped = {};
  for (const [name, original] of Object.entries(imports)) {
    wrapped[name] = (...args) => (hooks[name] ?? original)(...args, { original });
  }
  const { exports } = await WebAssembly.instantiate(await wasmModule(), { caj2pdf: wrapped });
  memory = exports.memory;
  return { exports, state };
}

function message(exports) {
  return new TextDecoder().decode(
    new Uint8Array(exports.memory.buffer, exports.caj2pdf_message_ptr(), exports.caj2pdf_message_len()),
  );
}

test("raw ABI converts with short reads and short writes, then reports", async () => {
  const pdf = await fixture(PDF);
  const { exports, state } = await rawInstance(pdf, {
    caj2pdf_read: (resource, offset, pointer, length, { original }) => original(resource, offset, pointer, Math.min(length, 1)),
    caj2pdf_write: (pointer, length, { original }) => original(pointer, Math.min(length, 1)),
  });
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 3, 0, 1, ...LIMITS), 0);
  assert.deepEqual(state.output, [...pdf]);
  assert.equal(state.flushes, 1);
  assert.equal(exports.caj2pdf_format(), 1);
  assert.equal(exports.caj2pdf_output_bytes_written(), BigInt(pdf.length));
  assert.ok(exports.caj2pdf_input_bytes_read() >= BigInt(pdf.length));
  // Progress counts thousandths of the document read and ends complete.
  for (const [index, [done, total]] of state.progress.entries()) {
    assert.equal(total, 1000);
    assert.ok(done <= 1000 && (index === 0 || done > state.progress[index - 1][0]));
  }
  assert.deepEqual(state.progress.at(-1), [1000, 1000]);
  // The result stays readable until a reset frees the session.
  assert.equal(exports.caj2pdf_inspect(BigInt(pdf.length), 64, 0, ...LIMITS), 3);
  exports.caj2pdf_reset();
  assert.equal(exports.caj2pdf_inspect(BigInt(pdf.length), 64, 0, ...LIMITS), 0);
  assert.equal(exports.caj2pdf_info_page_count(), 2);
});

test("raw ABI types host failures, overlong counts and cancellation", async () => {
  const pdf = await fixture(PDF);
  const cases = [
    ["read failure", { caj2pdf_read: () => -1 }, 5, /host read failed/],
    ["overlong read", { caj2pdf_read: (resource, offset, pointer, length) => length + 1 }, 2, /more bytes than requested/],
    ["write failure", { caj2pdf_write: () => -1 }, 5, /host write failed/],
    ["overlong write", { caj2pdf_write: (pointer, length) => length + 1 }, 5, /accepted more bytes than offered/],
    ["flush failure", { caj2pdf_flush: () => -1 }, 5, /./],
    ["cancellation", { caj2pdf_cancelled: () => 1 }, 6, /cancel/i],
  ];
  for (const [name, hooks, kind, pattern] of cases) {
    const { exports } = await rawInstance(pdf, hooks);
    assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 64, 0, 1, ...LIMITS), 1, name);
    assert.equal(exports.caj2pdf_error_kind(), kind, name);
    assert.match(message(exports), pattern, name);
  }
});

test("raw ABI refuses re-entry, invalid configuration and oversized inputs", async () => {
  const pdf = await fixture(PDF);
  const nested = [];
  let exports;
  ({ exports } = await rawInstance(pdf, {
    caj2pdf_write(pointer, length, { original }) {
      nested.push([exports.caj2pdf_convert(BigInt(pdf.length), 64, 0, 1, ...LIMITS), exports.caj2pdf_c8_add_font(1n, 0)]);
      return original(pointer, length);
    },
  }));
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 64, 0, 1, ...LIMITS), 0);
  assert.ok(nested.length > 0);
  assert.ok(nested.every(([status, font]) => status === 3 && font === 0));
  exports.caj2pdf_reset();
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 64, 99, 1, ...LIMITS), 2);
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 0, 0, 1, ...LIMITS), 2);
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), MAX_IO_CHUNK + 1, 0, 1, ...LIMITS), 2);
  assert.equal(exports.caj2pdf_convert(BigInt(pdf.length), 64, 0, 1, DEFAULT_LIMITS.maxInputBytes, DEFAULT_LIMITS.maxOutputBytes, 1n << 40n, DEFAULT_LIMITS.maxPages, DEFAULT_LIMITS.maxBookmarks), 2);
  assert.equal(exports.caj2pdf_convert(DEFAULT_LIMITS.maxInputBytes + 1n, 64, 0, 1, ...LIMITS), 1);
  assert.equal(exports.caj2pdf_error_kind(), 4);
  exports.caj2pdf_reset();
  assert.equal(exports.caj2pdf_error_kind(), 0);
});

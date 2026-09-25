// SPDX-License-Identifier: MIT

// Integration tests for the public conversion API with the real WASM build.
// Browser adapters (Blob, WritableStream) run on Node's implementations of
// those Web APIs here; browser.test.mjs runs them in headless Chromium.
import assert from "node:assert/strict";
import { open, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { Writable } from "node:stream";
import { finished } from "node:stream/promises";
import { test } from "node:test";
import {
  blobSource,
  Caj2PdfError,
  convert,
  DEFAULT_IO_CHUNK,
  fileHandleSource,
  inspect,
  loadModule,
  MAX_ALLOCATION_LIMIT,
  nodeWritableSink,
  UnsupportedFormatError,
  webWritableSink,
} from "../node.mjs";
import {
  collectingWriter,
  discard,
  fixture,
  largePdfBlob,
  newInstance,
  syntheticCaj,
  syntheticKdh,
  tempDirectory,
  trackedBlob,
  trackedSink,
  validatePdf,
  wasmModule,
  wasmUrl,
} from "./helpers.mjs";

async function inputs() {
  const { wrapped } = await syntheticKdh();
  return [
    { name: "CAJ", format: "caj", bytes: syntheticCaj(), pages: 2, bookmarks: 1 },
    { name: "KDH", format: "kdh", bytes: wrapped, pages: 2, bookmarks: 0 },
    { name: "PDF", format: "pdf", bytes: await fixture("valid_nested_outline.pdf"), pages: 2, bookmarks: 0 },
  ];
}

const CHUNK = 4096;

test("Blob sources and Web WritableStream sinks convert CAJ, KDH, and PDF", async (t) => {
  for (const input of await inputs()) {
    await t.test(input.name, async (t) => {
      const record = {};
      const { writer, bytes } = collectingWriter();
      const report = await convert(
        await wasmModule(),
        blobSource(trackedBlob(new Blob([input.bytes]), record)),
        trackedSink(webWritableSink(writer), record),
        { chunkSize: CHUNK },
      );
      await writer.close();
      const output = bytes();
      assert.equal(report.format, input.format);
      assert.equal(report.pagesConverted, input.pages);
      assert.equal(report.bookmarksWritten, input.bookmarks);
      assert.equal(report.outputBytesWritten, BigInt(output.length));
      assert.ok(report.inputBytesRead > 0n);
      assert.ok(record.maxRead <= CHUNK && record.maxWrite <= CHUNK, JSON.stringify(record));
      await validatePdf(t, output, input.pages);
    });
  }
});

test("FileHandle sources and Node Writable sinks convert CAJ, KDH, and PDF", async (t) => {
  const directory = await tempDirectory("convert");
  try {
    for (const input of await inputs()) {
      await t.test(input.name, async (t) => {
        const inputPath = join(directory, `input.${input.format}`);
        const outputPath = join(directory, `output-${input.format}.pdf`);
        await writeFile(inputPath, input.bytes);
        const inputHandle = await open(inputPath, "r");
        const output = (await open(outputPath, "wx")).createWriteStream();
        const record = {};
        try {
          const source = await fileHandleSource(inputHandle);
          const tracked = {
            size: source.size,
            readAt(offset, length, signal) {
              record.maxRead = Math.max(record.maxRead ?? 0, length);
              return source.readAt(offset, length, signal);
            },
          };
          const report = await convert(
            await newInstance(),
            tracked,
            trackedSink(nodeWritableSink(output), record),
            { chunkSize: CHUNK },
          );
          output.end();
          await finished(output);
          assert.equal(report.format, input.format);
          assert.equal(report.pagesConverted, input.pages);
          assert.ok(record.maxRead <= CHUNK && record.maxWrite <= CHUNK, JSON.stringify(record));
          await validatePdf(t, new Uint8Array(await readFile(outputPath)), input.pages);
        } finally {
          output.destroy();
          await inputHandle.close();
        }
      });
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("CAJ bookmarks can be omitted and the format can be forced", async (t) => {
  const { writer, bytes } = collectingWriter();
  const report = await convert(await wasmModule(), blobSource(new Blob([syntheticCaj()])), webWritableSink(writer), {
    format: "caj",
    includeBookmarks: false,
  });
  assert.equal(report.bookmarksWritten, 0);
  await validatePdf(t, bytes(), 2);
});

test("KDH output is the decoded PDF byte for byte", async () => {
  const { pdf, wrapped } = await syntheticKdh();
  const { writer, bytes } = collectingWriter();
  await convert(await wasmModule(), blobSource(new Blob([wrapped])), webWritableSink(writer));
  assert.deepEqual(bytes(), pdf);
});

test("inspect reports format, pages, and CAJ bookmarks without output", async () => {
  const expected = { caj: 1, kdh: null, pdf: null };
  for (const input of await inputs()) {
    const info = await inspect(await wasmModule(), blobSource(new Blob([input.bytes])));
    assert.equal(info.format, input.format);
    assert.equal(info.pageCount, input.pages);
    assert.equal(info.bookmarkCount, expected[input.format]);
    assert.ok(info.inputBytesRead > 0n);
  }
});

test("HN, C8, and TEB inputs are recognized and rejected as unsupported", async () => {
  const cases = [
    ["truncated_hn.hn", "hn"],
    ["truncated_c8.c8", "c8"],
    ["truncated_teb.teb", "teb"],
  ];
  const sink = { async writeChunk() { throw new Error("must not write"); }, async flush() {} };
  for (const [name, format] of cases) {
    const source = blobSource(new Blob([await fixture(name)]));
    await assert.rejects(convert(await wasmModule(), source, sink), (error) => {
      assert.ok(error instanceof UnsupportedFormatError);
      assert.ok(error instanceof Caj2PdfError);
      assert.equal(error.code, "UNSUPPORTED_FORMAT");
      assert.equal(error.format, format);
      assert.match(error.message, /not supported yet/);
      return true;
    });
    await assert.rejects(inspect(await wasmModule(), source), { code: "UNSUPPORTED_FORMAT", format });
  }
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob([syntheticCaj()])), sink, { format: "hn" }),
    { name: "UnsupportedFormatError", format: "hn" },
  );
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob(["plain text"])), sink),
    { name: "UnsupportedFormatError", format: null },
  );
});

test("malformed inputs reject with typed codes and located messages", async () => {
  const sink = discard;
  const kdh = new Uint8Array(254);
  kdh.set(new TextEncoder().encode("KDH"));
  await assert.rejects(convert(await wasmModule(), blobSource(new Blob([kdh])), sink), {
    code: "MALFORMED_KDH",
    message: /KDH signature is invalid/,
  });
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob([await fixture("truncated_caj.caj")])), sink),
    { code: "MALFORMED_CAJ", message: /extends beyond source/ },
  );
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob([await fixture("truncated_kdh.kdh")])), sink),
    { name: "TruncatedInputError", code: "TRUNCATED_INPUT", message: /truncated input/ },
  );
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob([await fixture("invalid_xref_offset.pdf")])), sink),
    { code: "MALFORMED_PDF" },
  );
});

test("configured limits reach the Rust engine and are validated first", async () => {
  const sink = discard;
  const source = blobSource(new Blob([syntheticCaj()]));
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxPages: 1 } }), {
    code: "CAJ_LIMIT_EXCEEDED",
  });
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxInputBytes: 10n } }), {
    code: "LIMIT_EXCEEDED",
  });
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxOutputBytes: 100 } }), {
    code: "PDF_LIMIT_EXCEEDED",
  });
  for (const limits of [
    { maxAllocationBytes: MAX_ALLOCATION_LIMIT + 1n },
    { maxAllocationBytes: 16 },
    { maxPages: -1 },
    { maxInputBytes: -1n },
    null,
  ]) {
    await assert.rejects(convert(await wasmModule(), source, sink, { limits }), /limits/);
  }
  await assert.rejects(convert(await wasmModule(), source, sink, { format: "docx" }), RangeError);
  await assert.rejects(convert(await wasmModule(), source, sink, { chunkSize: 0 }), RangeError);
  await assert.rejects(convert({}, source, sink), TypeError);
  await assert.rejects(convert(await wasmModule(), {}, sink), TypeError);
  await assert.rejects(convert(await wasmModule(), source, {}), TypeError);
  // An explicit undefined keeps the default instead of failing validation.
  const report = await convert(await wasmModule(), source, sink, { limits: { maxPages: undefined } });
  assert.equal(report.pagesConverted, 2);
});

test("aborting stops conversion between awaited writes and resets the instance", async () => {
  const instance = await newInstance();
  const controller = new AbortController();
  let writes = 0;
  const sink = {
    async writeChunk(bytes) {
      writes += 1;
      if (writes === 2) controller.abort();
      return bytes.length;
    },
    async flush() {},
  };
  await assert.rejects(
    convert(instance, blobSource(largePdfBlob(64 * 1024)), sink, { chunkSize: 4096, signal: controller.signal }),
    { name: "AbortError" },
  );
  assert.equal(writes, 2, "no write follows the abort");
  // The engine was reset, so the same instance accepts a new operation.
  const { writer } = collectingWriter();
  const report = await convert(instance, blobSource(new Blob([syntheticCaj()])), webWritableSink(writer));
  assert.equal(report.pagesConverted, 2);
});

test("an abort interrupts a stalled Web sink write", async () => {
  const controller = new AbortController();
  const writer = new WritableStream({ write: () => new Promise(() => {}) }).getWriter();
  const pending = convert(await wasmModule(), blobSource(new Blob([syntheticCaj()])), webWritableSink(writer), {
    signal: controller.signal,
  });
  setTimeout(() => controller.abort(new Error("user cancelled")), 10);
  await assert.rejects(pending, /user cancelled/);
});

test("an already aborted signal rejects before reading", async () => {
  const controller = new AbortController();
  controller.abort();
  const source = { size: 1n, async readAt() { throw new Error("must not read"); } };
  await assert.rejects(
    convert(await wasmModule(), source, { async writeChunk() {}, async flush() {} }, { signal: controller.signal }),
    { name: "AbortError" },
  );
});

test("sink and source errors stop conversion with the original error", async () => {
  let writes = 0;
  const failing = {
    async writeChunk() {
      writes += 1;
      throw new Error("disk full");
    },
    async flush() {},
  };
  await assert.rejects(convert(await wasmModule(), blobSource(new Blob([syntheticCaj()])), failing), /disk full/);
  assert.equal(writes, 1);

  const writable = new Writable({ write(_chunk, _encoding, callback) { callback(new Error("pipe closed")); } });
  await assert.rejects(
    convert(await wasmModule(), blobSource(new Blob([syntheticCaj()])), nodeWritableSink(writable)),
    /pipe closed/,
  );

  const source = { size: 10n, async readAt() { throw new Error("network lost"); } };
  await assert.rejects(convert(await wasmModule(), source, failing), /network lost/);
});

test("a larger PDF converts with bounded chunks and bounded WASM memory", async (t) => {
  const streamBytes = 24 * 1024 * 1024;
  const blob = largePdfBlob(streamBytes);
  const instance = await newInstance();
  const memory = instance.exports.memory;
  const before = memory.buffer.byteLength;
  const record = {};
  let total = 0;
  const sink = trackedSink({
    async writeChunk(bytes) {
      total += bytes.byteLength;
      return bytes.byteLength;
    },
    async flush() {},
  }, record);
  const report = await convert(instance, blobSource(trackedBlob(blob, record)), sink);
  const after = memory.buffer.byteLength;
  assert.equal(report.outputBytesWritten, BigInt(blob.size));
  assert.equal(total, blob.size);
  assert.ok(record.maxRead <= DEFAULT_IO_CHUNK && record.maxWrite <= DEFAULT_IO_CHUNK);
  t.diagnostic(
    `input ${blob.size} B; WASM memory ${before} B before, ${after} B after (peak); ` +
      `max read ${record.maxRead} B, max write ${record.maxWrite} B, ${record.writes} writes`,
  );
  // The memory must not scale with the 24 MiB document.
  assert.ok(after - before < 4 * 1024 * 1024, `WASM memory grew by ${after - before} bytes`);
});

test("loadModule compiles a WASM file for reuse across conversions", async () => {
  const module = await loadModule(wasmUrl);
  assert.ok(module instanceof WebAssembly.Module);
  const pending = [1, 2].map(async () => {
    const { writer, bytes } = collectingWriter();
    await convert(module, blobSource(new Blob([syntheticCaj()])), webWritableSink(writer));
    return bytes().length;
  });
  const [first, second] = await Promise.all(pending);
  assert.equal(first, second, "concurrent conversions use separate instances");
});

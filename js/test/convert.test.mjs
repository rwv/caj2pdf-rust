// SPDX-License-Identifier: MIT

// Integration tests for the public conversion API with the real WASM build,
// run in a Node Worker. Blob inputs and WritableStream sinks use Node's
// implementations of those Web APIs here; browser.test.mjs runs the browser
// entry point in headless Chromium.
import assert from "node:assert/strict";
import { open, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { Writable } from "node:stream";
import { finished } from "node:stream/promises";
import { test } from "node:test";
import {
  Caj2PdfError,
  convert,
  DEFAULT_IO_CHUNK,
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
  syntheticCaj,
  syntheticAscii85Caj,
  syntheticFlateReplayCaj,
  syntheticRecoveredCaj,
  syntheticLaterCopyCaj,
  syntheticKdh,
  tempDirectory,
  trackedSink,
  validatePdf,
  wasmModule,
  wasmUrl,
} from "./helpers.mjs";

async function inputs() {
  const { wrapped } = await syntheticKdh();
  const pdf = await fixture("valid_out_of_order_objects.pdf");
  const footer = new TextEncoder().encode("WebFastLoad\uFEFF<FileProperty><Doi /><FileName>original-test</FileName><TableName>TEST</TableName><Type>1</Type></FileProperty>");
  return [
    { name: "PDF download footer", format: "pdf", bytes: new Uint8Array([...pdf, ...footer]), expected: pdf, pages: 2, bookmarks: 0 },
    { name: "adjacent Flate CAJ", format: "caj", bytes: syntheticFlateReplayCaj({ anchor: null }), pages: 2, bookmarks: 1 },
    { name: "array-replay CAJ", format: "caj", bytes: syntheticFlateReplayCaj({ anchor: "array", padding: "\n" }), pages: 2, bookmarks: 1 },
    { name: "scalar-replay CAJ", format: "caj", bytes: syntheticFlateReplayCaj(), pages: 2, bookmarks: 1 },
    { name: "ASCII85 CAJ", format: "caj", bytes: syntheticAscii85Caj(), pages: 2, bookmarks: 1 },
    { name: "CAJ", format: "caj", bytes: syntheticCaj(), pages: 2, bookmarks: 1 },
    { name: "later-copy CAJ", format: "caj", bytes: syntheticLaterCopyCaj(), pages: 2, bookmarks: 1 },
    { name: "recovered CAJ", format: "caj", bytes: syntheticRecoveredCaj(), pages: 2, bookmarks: 1 },
    { name: "KDH", format: "kdh", bytes: wrapped, pages: 2, bookmarks: 0 },
    { name: "KDH profile 1", format: "kdh", bytes: (await syntheticKdh([1, 0, 0, 0])).wrapped, expected: pdf, pages: 2, bookmarks: 0 },
    { name: "PDF", format: "pdf", bytes: await fixture("valid_nested_outline.pdf"), pages: 2, bookmarks: 0 },
  ];
}

const CHUNK = 4096;

test("Blob sources and Web WritableStream sinks convert CAJ, KDH, and PDF", async (t) => {
  for (const input of await inputs()) {
    await t.test(input.name, async (t) => {
      const record = {};
      const progress = [];
      const { writer, bytes } = collectingWriter();
      const report = await convert(
        await wasmModule(),
        new Blob([input.bytes]),
        trackedSink(webWritableSink(writer), record),
        { chunkSize: CHUNK, progress: (fraction) => progress.push(fraction) },
      );
      await writer.close();
      const output = bytes();
      if (input.expected) assert.deepEqual(output, input.expected);
      assert.equal(report.format, input.format);
      assert.equal(report.pagesConverted, input.pages);
      assert.equal(report.bookmarksWritten, input.bookmarks);
      assert.equal(report.outputBytesWritten, BigInt(output.length));
      assert.ok(report.inputBytesRead > 0n);
      assert.ok(record.maxWrite <= CHUNK, JSON.stringify(record));
      // Progress is the share of the input read, increasing to the end.
      assert.ok(progress.every((value, index) => value > 0 && value <= 1 && (index === 0 || value > progress[index - 1])));
      assert.equal(progress.at(-1), 1);
      await validatePdf(t, output, input.pages);
    });
  }
});

test("file paths and descriptors with Node Writable sinks convert CAJ, KDH, and PDF", async (t) => {
  const directory = await tempDirectory("convert");
  try {
    for (const input of await inputs()) {
      await t.test(input.name, async (t) => {
        const inputPath = join(directory, `input-${input.name}.${input.format}`);
        await writeFile(inputPath, input.bytes);
        const inputHandle = await open(inputPath, "r");
        try {
          for (const [kind, source] of [["path", inputPath], ["URL", pathToFileURL(inputPath)], ["descriptor", inputHandle.fd]]) {
            const outputPath = join(directory, `output-${input.name}-${kind}.pdf`);
            const output = (await open(outputPath, "wx")).createWriteStream();
            const record = {};
            try {
              const report = await convert(
                await wasmModule(),
                source,
                trackedSink(nodeWritableSink(output), record),
                { chunkSize: CHUNK },
              );
              output.end();
              await finished(output);
              assert.equal(report.format, input.format);
              assert.equal(report.pagesConverted, input.pages);
              assert.ok(record.maxWrite <= CHUNK, JSON.stringify(record));
              await validatePdf(t, new Uint8Array(await readFile(outputPath)), input.pages);
            } finally {
              output.destroy();
            }
          }
          // The caller's descriptor stays open and usable.
          const { bytesRead } = await inputHandle.read(new Uint8Array(4), 0, 4, 0);
          assert.equal(bytesRead, 4);
        } finally {
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
  const report = await convert(await wasmModule(), new Blob([syntheticCaj()]), webWritableSink(writer), {
    format: "caj",
    includeBookmarks: false,
  });
  assert.equal(report.bookmarksWritten, 0);
  await validatePdf(t, bytes(), 2);
});

test("KDH output is the decoded PDF byte for byte", async () => {
  const { pdf, wrapped } = await syntheticKdh();
  const { writer, bytes } = collectingWriter();
  await convert(await wasmModule(), new Blob([wrapped]), webWritableSink(writer));
  assert.deepEqual(bytes(), pdf);
});

test("a PDF header after a byte-order mark is detected and the mark dropped", async (t) => {
  const pdf = await fixture("valid_nested_outline.pdf");
  const input = new Blob([new Uint8Array([0xef, 0xbb, 0xbf]), pdf]);
  const { writer, bytes } = collectingWriter();
  const report = await convert(await wasmModule(), input, webWritableSink(writer));
  await writer.close();
  assert.equal(report.format, "pdf");
  assert.deepEqual(bytes(), pdf);
  await validatePdf(t, bytes(), 2);
  const info = await inspect(await wasmModule(), input);
  assert.equal(info.format, "pdf");
  assert.equal(info.pageCount, 2);
});

test("inspect reports format, pages, and CAJ bookmarks without output", async () => {
  const expected = { caj: 1, kdh: null, pdf: null };
  for (const input of await inputs()) {
    const info = await inspect(await wasmModule(), new Blob([input.bytes]));
    assert.equal(info.format, input.format);
    assert.equal(info.pageCount, input.pages);
    assert.equal(info.bookmarkCount, expected[input.format]);
    assert.ok(info.inputBytesRead > 0n);
  }
});

test("CAA inspection reports unknown counts and conversion refuses without writing", async () => {
  const bytes = await fixture("target_descriptor.caa");
  const source = new Blob([bytes]);
  const module = await wasmModule();
  const info = await inspect(module, source, { chunkSize: 3 });
  assert.equal(info.format, "caa");
  assert.equal(info.pageCount, null);
  assert.equal(info.bookmarkCount, null);
  assert.equal(info.inputBytesRead, BigInt(bytes.length));
  const sink = { async writeChunk() { assert.fail("CAA must not write PDF bytes"); }, async flush() {} };
  for (const options of [{}, { format: "caa" }]) {
    await assert.rejects(convert(module, source, sink, options), (error) => {
      assert.ok(error instanceof UnsupportedFormatError);
      assert.equal(error.code, "UNSUPPORTED_FORMAT");
      assert.equal(error.format, "caa");
      assert.match(error.message, /obtain the referenced document/);
      return true;
    });
  }
  await assert.rejects(inspect(module, new Blob([bytes.subarray(0, -1)])), {
    name: "UnsupportedFormatError", format: null,
  });
});

test("malformed HN/C8 and unsupported TEB failures are distinguished", async () => {
  const cases = [
    ["truncated_hn.hn", "hn"],
    ["truncated_c8.c8", "c8"],
    ["truncated_teb.teb", "teb"],
  ];
  const sink = { async writeChunk() { throw new Error("must not write"); }, async flush() {} };
  for (const [name, format] of cases) {
    const source = new Blob([await fixture(name)]);
    await assert.rejects(convert(await wasmModule(), source, sink), (error) => {
      assert.ok(error instanceof Caj2PdfError);
      if (format === "teb") {
        assert.ok(error instanceof UnsupportedFormatError);
        assert.equal(error.code, "UNSUPPORTED_FORMAT");
        assert.equal(error.format, format);
        assert.match(error.message, /DRM-encrypted CNKI container/);
      } else {
        assert.equal(error.code, "HNC8");
        assert.match(error.message, /HN\/C8/);
      }
      return true;
    });
    await assert.rejects(inspect(await wasmModule(), source), format === "teb" ? { code: "UNSUPPORTED_FORMAT", format } : { code: "HNC8" });
  }
  await assert.rejects(
    convert(await wasmModule(), new Blob([syntheticCaj()]), sink, { format: "hn" }),
    { name: "Caj2PdfError", code: "HNC8" },
  );
  await assert.rejects(
    convert(await wasmModule(), new Blob(["plain text"]), sink),
    { name: "UnsupportedFormatError", format: null },
  );
});

test("malformed inputs reject with typed codes and located messages", async () => {
  const sink = discard;
  const kdh = new Uint8Array(254);
  kdh.set(new TextEncoder().encode("KDH"));
  await assert.rejects(convert(await wasmModule(), new Blob([kdh]), sink), {
    code: "MALFORMED_KDH",
    message: /KDH signature is invalid/,
  });
  await assert.rejects(
    convert(await wasmModule(), new Blob([await fixture("truncated_caj.caj")]), sink),
    { code: "MALFORMED_CAJ", message: /extends beyond source/ },
  );
  await assert.rejects(
    convert(await wasmModule(), new Blob([await fixture("truncated_kdh.kdh")]), sink),
    { name: "TruncatedInputError", code: "TRUNCATED_INPUT", message: /truncated input/ },
  );
  await assert.rejects(
    convert(await wasmModule(), new Blob([await fixture("invalid_xref_offset.pdf")]), sink),
    { code: "MALFORMED_PDF" },
  );
});

test("configured limits reach the Rust engine and are validated first", async () => {
  const sink = discard;
  const source = new Blob([syntheticCaj()]);
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxPages: 1 } }), {
    code: "CAJ_LIMIT_EXCEEDED",
  });
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxInputBytes: 10n } }), {
    code: "LIMIT_EXCEEDED",
  });
  await assert.rejects(convert(await wasmModule(), source, sink, { limits: { maxOutputBytes: 100 } }), {
    code: "LIMIT_EXCEEDED",
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
  await assert.rejects(convert((await WebAssembly.instantiate(await wasmModule(), {
    caj2pdf: { caj2pdf_read() {}, caj2pdf_write() {}, caj2pdf_flush() {}, caj2pdf_progress() {}, caj2pdf_cancelled() {} },
  })).exports, source, sink), TypeError);
  await assert.rejects(convert(await wasmModule(), {}, sink), TypeError);
  await assert.rejects(convert(await wasmModule(), -1, sink), TypeError);
  await assert.rejects(convert(await wasmModule(), source, {}), TypeError);
  // An explicit undefined keeps the default instead of failing validation.
  const report = await convert(await wasmModule(), source, sink, { limits: { maxPages: undefined } });
  assert.equal(report.pagesConverted, 2);
});

test("aborting stops conversion between sink writes and the module converts again", async () => {
  const instance = await wasmModule();
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
    convert(instance, largePdfBlob(64 * 1024), sink, { chunkSize: 4096, signal: controller.signal }),
    { name: "AbortError" },
  );
  assert.equal(writes, 2, "no write follows the abort");
  // Each operation has its own Worker, so the module converts again.
  const { writer } = collectingWriter();
  const report = await convert(instance, new Blob([syntheticCaj()]), webWritableSink(writer));
  assert.equal(report.pagesConverted, 2);
});

test("an abort interrupts a stalled Web sink write", async () => {
  const controller = new AbortController();
  const writer = new WritableStream({ write: () => new Promise(() => {}) }).getWriter();
  const pending = convert(await wasmModule(), new Blob([syntheticCaj()]), webWritableSink(writer), {
    signal: controller.signal,
  });
  setTimeout(() => controller.abort(new Error("user cancelled")), 10);
  await assert.rejects(pending, /user cancelled/);
});

test("an already aborted signal rejects before reading", async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(
    convert(await wasmModule(), "/nonexistent/caj2pdf-input", { async writeChunk() {}, async flush() {} }, { signal: controller.signal }),
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
  await assert.rejects(convert(await wasmModule(), new Blob([syntheticCaj()]), failing), /disk full/);
  assert.equal(writes, 1);

  const writable = new Writable({ write(_chunk, _encoding, callback) { callback(new Error("pipe closed")); } });
  await assert.rejects(
    convert(await wasmModule(), new Blob([syntheticCaj()]), nodeWritableSink(writable)),
    /pipe closed/,
  );

  // A Blob read on the calling thread keeps the caller's error.
  const lost = new Error("network lost");
  class LostBlob extends Blob {
    slice() {
      throw lost;
    }
  }
  await assert.rejects(convert(await wasmModule(), new LostBlob([syntheticCaj()]), failing), (error) => error === lost);
  // A file that cannot be opened is refused before the Worker starts.
  await assert.rejects(convert(await wasmModule(), "/nonexistent/caj2pdf-input", failing), { code: "ENOENT" });
});

test("a larger PDF converts with bounded chunks", async (t) => {
  const streamBytes = 24 * 1024 * 1024;
  const blob = largePdfBlob(streamBytes);
  const record = {};
  let total = 0;
  const sink = trackedSink({
    async writeChunk(bytes) {
      total += bytes.byteLength;
      return bytes.byteLength;
    },
    async flush() {},
  }, record);
  const report = await convert(await wasmModule(), blob, sink);
  assert.equal(report.outputBytesWritten, BigInt(blob.size));
  assert.equal(total, blob.size);
  assert.ok(record.maxWrite <= DEFAULT_IO_CHUNK);
  t.diagnostic(`input ${blob.size} B; max write ${record.maxWrite} B, ${record.writes} writes`);
});

test("loadModule compiles a WASM file for reuse across conversions", async () => {
  const module = await loadModule(wasmUrl);
  assert.ok(module instanceof WebAssembly.Module);
  const pending = [1, 2].map(async () => {
    const { writer, bytes } = collectingWriter();
    await convert(module, new Blob([syntheticCaj()]), webWritableSink(writer));
    return bytes().length;
  });
  const [first, second] = await Promise.all(pending);
  assert.equal(first, second, "concurrent conversions use separate instances");
});

test("CAJ recovery rejects a later malformed object before publishing output", async () => {
  const { writer, bytes } = collectingWriter();
  await assert.rejects(
    convert(await wasmModule(), new Blob([syntheticRecoveredCaj(true)]), webWritableSink(writer)),
    { code: "MALFORMED_PDF" },
  );
  assert.equal(bytes().length, 0);
  await writer.close();
});

test("CAJ later-copy recovery rejects changed prefixes without output", async () => {
  const { writer, bytes } = collectingWriter();
  await assert.rejects(convert(await wasmModule(), new Blob([syntheticLaterCopyCaj(true)]), webWritableSink(writer)), { code: "MALFORMED_PDF" });
  assert.equal(bytes().length, 0);
  await writer.close();
});

test("ASCII85 replay preserves complete output without decoding the payload", async () => {
  for (const broken of [false, true]) {
    const outputs = [];
    for (const options of [{ interrupted: false }, {}, { cut: "keyword" }, { cut: "reference" }]) {
      const { writer, bytes } = collectingWriter();
      await convert(await wasmModule(), new Blob([syntheticAscii85Caj({ ...options, broken })]),
        webWritableSink(writer), { chunkSize: 1 });
      await writer.close();
      outputs.push(bytes());
    }
    assert.deepEqual(outputs[1], outputs[0]);
    assert.deepEqual(outputs[2], outputs[0]);
    assert.deepEqual(outputs[3], outputs[0]);
  }
});

test("Flate replay preserves output without decoding the payload", async () => {
  for (const broken of [false, true]) {
    for (const anchor of [null, "scalar", "array", "deferred"]) {
      const outputs = [];
      for (const interrupted of [false, true]) {
        const { writer, bytes } = collectingWriter();
        await convert(await wasmModule(), new Blob([syntheticFlateReplayCaj({ interrupted, broken, anchor, padding: anchor === "deferred" ? "" : "\n" })]),
          webWritableSink(writer), { chunkSize: 1 });
        await writer.close();
        outputs.push(bytes());
      }
      assert.deepEqual(outputs[1], outputs[0]);
    }
  }
});

test("explicit damaged mode reports blank pages through the public WASM API", async (t) => {
  const { syntheticDamagedCaj } = await import("./helpers.mjs");
  const bytes = syntheticDamagedCaj();
  const source = () => new Blob([bytes]);
  await assert.rejects(convert(await wasmModule(), source(), discard));
  const sink = collectingWriter();
  const report = await convert(await wasmModule(), source(), webWritableSink(sink.writer), { allowDamaged: true, chunkSize: 17 });
  await sink.writer.close();
  assert.equal(report.pagesConverted, 2);
  assert.equal(report.bookmarksWritten, 1);
  assert.deepEqual(report.omittedPages.map((page) => page.pageIndex), [0]);
  assert.equal(typeof report.omittedPages[0].offset, "bigint");
  await validatePdf(t, sink.bytes(), 2);
});

test("partial conversion requires an explicit boolean opt-in", async () => {
  for (const allowDamaged of ["false", "true", 1, null, {}]) {
    await assert.rejects(convert(await wasmModule(), new Blob([syntheticCaj()]), discard, { allowDamaged }), /allowDamaged must be a boolean/);
  }
});

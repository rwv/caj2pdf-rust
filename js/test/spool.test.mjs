// SPDX-License-Identifier: MIT

// Forward-only stream spooling: Node temporary files and a browser OPFS
// spool. The OPFS tests here use an in-memory test double of the OPFS API
// and stop before conversion, which needs a browser Worker; browser.test.mjs
// converts through the real OPFS in headless Chromium.
import assert from "node:assert/strict";
import { readFile, readdir, rm } from "node:fs/promises";
import { join } from "node:path";
import { Readable } from "node:stream";
import { test } from "node:test";
import { convertReadableStream, spoolToOpfs } from "../browser.mjs";
import { convertReadable, convertSpooled, spoolToTempFile, webWritableSink } from "../node.mjs";
import {
  collectingWriter,
  discard,
  fixture,
  syntheticCaj,
  syntheticKdh,
  tempDirectory,
  validatePdf,
  wasmModule,
} from "./helpers.mjs";

function pieces(bytes, size = 100) {
  const parts = [];
  for (let offset = 0; offset < bytes.length; offset += size) {
    parts.push(bytes.subarray(offset, offset + size));
  }
  return parts;
}

async function withTempRoot(action) {
  const directory = await tempDirectory("spool");
  try {
    await action(directory);
    assert.deepEqual(await readdir(directory), [], "spool directory was removed");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

test("a Node Readable spools to a private temp file that dispose removes", async () => {
  await withTempRoot(async (directory) => {
    const bytes = syntheticCaj();
    const spooled = await spoolToTempFile(Readable.from(pieces(bytes)), { maxBytes: 1n << 20n, directory });
    // The spooled copy is a file path that convert() reads directly.
    assert.deepEqual(new Uint8Array(await readFile(spooled.source)), bytes);
    assert.equal(spooled.source, join(spooled.path, "input"));
    assert.deepEqual(await readdir(spooled.path), ["input"]);
    await spooled.dispose();
  });
});

test("convertReadable converts Node and Web streams and removes the spool", async (t) => {
  const { wrapped } = await syntheticKdh();
  const streams = [
    ["Node Readable KDH", () => Readable.from(pieces(wrapped)), 2],
    ["Web ReadableStream CAJ", () => new Blob([syntheticCaj()]).stream(), 2],
  ];
  for (const [name, stream, pages] of streams) {
    await t.test(name, async (t) => {
      await withTempRoot(async (tempDirectory) => {
        const { writer, bytes } = collectingWriter();
        const report = await convertReadable(await wasmModule(), stream(), webWritableSink(writer), {
          tempDirectory,
          chunkSize: 4096,
        });
        assert.equal(report.pagesConverted, pages);
        await validatePdf(t, bytes(), pages);
      });
    });
  }
});

test("the spool rejects input beyond its bound and destroys the stream", async () => {
  await withTempRoot(async (tempDirectory) => {
    const stream = Readable.from(pieces(syntheticCaj()));
    await assert.rejects(
      convertReadable(await wasmModule(), stream, discard, { tempDirectory, maxSpoolBytes: 500 }),
      { code: "LIMIT_EXCEEDED", message: /spool limit of 500 bytes/ },
    );
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(stream.destroyed, true);
  });
  await withTempRoot(async (tempDirectory) => {
    // limits.maxInputBytes is the default spool bound.
    await assert.rejects(
      convertReadable(await wasmModule(), Readable.from(pieces(syntheticCaj())), discard, {
        tempDirectory,
        limits: { maxInputBytes: 64n },
      }),
      { code: "LIMIT_EXCEEDED" },
    );
  });
});

test("invalid options are rejected before the stream is spooled", async () => {
  let pulled = false;
  const stream = new ReadableStream({
    pull(controller) {
      pulled = true;
      controller.enqueue(syntheticCaj());
      controller.close();
    },
  }, { highWaterMark: 0 });
  const spool = () => assert.fail("spool must not run");
  for (const [wasm, sink, options, error] of [
    [await wasmModule(), discard, { format: "docx" }, RangeError],
    [await wasmModule(), discard, { chunkSize: 0 }, RangeError],
    [await wasmModule(), discard, { limits: { maxPages: -1 } }, RangeError],
    [await wasmModule(), {}, {}, TypeError],
    [{}, discard, {}, TypeError],
  ]) {
    await assert.rejects(convertSpooled(spool, wasm, stream, sink, options), error);
  }
  assert.equal(pulled, false);
});

test("aborting a stalled stream or a running conversion removes the spool", async () => {
  await withTempRoot(async (tempDirectory) => {
    const controller = new AbortController();
    const stalled = new ReadableStream({
      start(streamController) {
        streamController.enqueue(Uint8Array.of(1, 2, 3));
      },
    });
    setTimeout(() => controller.abort(), 10);
    await assert.rejects(
      convertReadable(await wasmModule(), stalled, discard, { tempDirectory, signal: controller.signal }),
      { name: "AbortError" },
    );
  });
  await withTempRoot(async (tempDirectory) => {
    const controller = new AbortController();
    const sink = {
      async writeChunk(bytes) {
        controller.abort();
        return bytes.length;
      },
      async flush() {},
    };
    await assert.rejects(
      convertReadable(await wasmModule(), Readable.from([syntheticCaj()]), sink, {
        tempDirectory,
        signal: controller.signal,
      }),
      { name: "AbortError" },
    );
  });
});

test("conversion and stream failures remove the spool", async () => {
  await withTempRoot(async (tempDirectory) => {
    await assert.rejects(
      convertReadable(await wasmModule(), Readable.from([await fixture("truncated_hn.hn")]), discard, { tempDirectory }),
      { name: "Caj2PdfError", code: "HNC8" },
    );
  });
  await withTempRoot(async (tempDirectory) => {
    const failing = new Readable({
      read() {
        this.destroy(new Error("upstream reset"));
      },
    });
    await assert.rejects(convertReadable(await wasmModule(), failing, discard, { tempDirectory }), /upstream reset/);
  });
  await withTempRoot(async (tempDirectory) => {
    await assert.rejects(
      convertReadable(await wasmModule(), Readable.from(["text"]), discard, { tempDirectory }),
      /must be Uint8Array/,
    );
  });
  await withTempRoot(async (tempDirectory) => {
    // The conversion error wins over a failing cleanup.
    const failingDispose = async (stream, options) => {
      const spooled = await spoolToTempFile(stream, { ...options, directory: tempDirectory });
      return {
        source: spooled.source,
        async dispose() {
          await spooled.dispose();
          throw new Error("cleanup failed");
        },
      };
    };
    await assert.rejects(
      convertSpooled(failingDispose, await wasmModule(), Readable.from([Uint8Array.of(0)]), discard),
      { name: "UnsupportedFormatError" },
    );
  });
  await assert.rejects(spoolToTempFile({}, { maxBytes: 1n }), TypeError);
  await assert.rejects(spoolToTempFile(Readable.from([]), {}), RangeError);
});

/** An in-memory test double of the OPFS directory API. */
function fakeStorage({ writable = true, abortError } = {}) {
  const files = new Map();
  const events = [];
  const root = {
    async getFileHandle(name, { create }) {
      assert.equal(create, true);
      files.set(name, []);
      const handle = {
        kind: "file",
        async getFile() {
          return new File(files.get(name), name);
        },
      };
      if (writable) {
        handle.createWritable = async () => ({
          async write(chunk) {
            files.get(name).push(chunk.slice());
          },
          async close() {
            events.push("close");
          },
          abort() {
            events.push("abort");
            if (abortError !== undefined) throw abortError;
            return Promise.resolve();
          },
        });
      }
      return handle;
    },
    async removeEntry(name) {
      files.delete(name);
      events.push("remove");
    },
  };
  return { files, events, storage: { getDirectory: async () => root } };
}

test("the browser spool writes an OPFS file handle that dispose removes", async () => {
  const { files, events, storage } = fakeStorage();
  const bytes = syntheticCaj();
  const spooled = await spoolToOpfs(new Blob([bytes]).stream(), { maxBytes: 1n << 20n, storage });
  // The spooled copy is the OPFS file handle that convert() opens in its Worker.
  assert.equal(spooled.source.kind, "file");
  assert.deepEqual(new Uint8Array(await (await spooled.source.getFile()).arrayBuffer()), bytes);
  await spooled.dispose();
  assert.equal(files.size, 0);
  assert.deepEqual(events, ["close", "remove"]);
});

test("the browser spool aborts and removes its file on failure", async () => {
  const bounded = fakeStorage();
  await assert.rejects(
    spoolToOpfs(new Blob([syntheticCaj()]).stream(), { maxBytes: 10n, storage: bounded.storage }),
    { code: "LIMIT_EXCEEDED" },
  );
  assert.equal(bounded.files.size, 0);
  assert.deepEqual(bounded.events, ["abort", "remove"]);
});

test("the browser spool fails explicitly without durable storage", async () => {
  const stream = () => new Blob([syntheticCaj()]).stream();
  await assert.rejects(spoolToOpfs(stream(), { maxBytes: 1n << 20n, storage: undefined }), {
    code: "RANDOM_ACCESS_REQUIRED",
    message: /OPFS/,
  });
  await assert.rejects(convertReadableStream(await wasmModule(), stream(), discard, { storage: {} }), {
    code: "RANDOM_ACCESS_REQUIRED",
  });
  const readOnly = fakeStorage({ writable: false });
  await assert.rejects(spoolToOpfs(stream(), { maxBytes: 1n << 20n, storage: readOnly.storage }), {
    code: "RANDOM_ACCESS_REQUIRED",
    message: /createWritable|cannot write/,
  });
  assert.equal(readOnly.files.size, 0);
});


test("OPFS failure cleanup retries only transient locks and reports persistent failures", async (t) => {
  for (const [name, failures, expectedAttempts] of [
    ["NoModificationAllowedError", 1, 2],
    ["NoModificationAllowedError", 3, 3],
    ["NotAllowedError", 1, 1],
  ]) {
    await t.test(`${name}, ${failures} failures`, async () => {
      const { storage, files } = fakeStorage();
      const root = await storage.getDirectory();
      const remove = root.removeEntry.bind(root);
      let attempts = 0;
      const cleanupError = new DOMException("injected cleanup failure", name);
      root.removeEntry = async (...args) => {
        if (++attempts <= failures) throw cleanupError;
        return remove(...args);
      };
      await assert.rejects(
        spoolToOpfs(new Blob([syntheticCaj()]).stream(), { maxBytes: 10n, storage }),
        (error) => {
          if (failures < expectedAttempts) {
            assert.equal(error.code, "LIMIT_EXCEEDED");
            assert.equal(files.size, 0);
          } else {
            assert.ok(error instanceof AggregateError);
            assert.equal(error.cause.code, "LIMIT_EXCEEDED");
            assert.equal(error.errors[0], error.cause);
            assert.equal(error.errors[1], cleanupError);
            assert.equal(files.size, 1);
          }
          return true;
        },
      );
      assert.equal(attempts, expectedAttempts);
    });
  }
});


test("a synchronously failing OPFS writer abort still removes the spool", async () => {
  const { storage, files, events } = fakeStorage({ abortError: new Error("writer abort failed") });
  await assert.rejects(
    spoolToOpfs(new Blob([syntheticCaj()]).stream(), { maxBytes: 10n, storage }),
    { code: "LIMIT_EXCEEDED" },
  );
  assert.equal(files.size, 0);
  assert.deepEqual(events, ["abort", "remove"]);
});

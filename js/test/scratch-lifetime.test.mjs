// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readdir, rm } from "node:fs/promises";
import { test } from "node:test";
import { withHnc8Scratch as withNodeScratch } from "../node.mjs";
import { withHnc8Scratch as withBrowserScratch } from "../browser.mjs";
import { tempDirectory } from "./helpers.mjs";

test("Node scratch scope cleans successful, failed and cancelled operations", async () => {
  const directory = await tempDirectory("scratch-scope");
  try {
    for (const error of [null, new Error("conversion failed"), new DOMException("Cancelled", "AbortError")]) {
      let stores;
      const operation = withNodeScratch(async (scratch) => {
        stores = scratch;
        assert.equal(scratch.length, 4);
        await scratch[0].resize(8n);
        assert.equal(await scratch[0].writeAt(0n, new Uint8Array([7])), 1);
        await assert.rejects(scratch[1].resize(9n), RangeError);
        if (error) throw error;
        return 42;
      }, { maxBytes: 8n, directory });
      if (error) await assert.rejects(operation, (actual) => actual === error);
      else assert.equal(await operation, 42);
      assert.deepEqual(await readdir(directory), []);
      await assert.rejects(stores[0].readAt(0n, 1));
    }
    await assert.rejects(withNodeScratch(() => assert.fail(), { maxBytes: -1n, directory }));
    assert.deepEqual(await readdir(directory), []);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("OPFS scope closes acquired handles and removes its directory after partial setup failure", async () => {
  for (const failureIndex of [0, 2, 4]) {
    const closed = [];
    let opened = 0;
    let removed = false;
    const failure = new Error("OPFS setup or conversion failed");
    const root = {
      async getDirectoryHandle() {
        return {
          async getFileHandle() {
            return {
              async createSyncAccessHandle() {
                if (opened === failureIndex) throw failure;
                const id = opened++;
                return { getSize: () => 0, truncate() {}, read() {}, write() {}, flush() {}, close() { closed.push(id); } };
              },
            };
          },
        };
      },
      async removeEntry(name, options) {
        assert.match(name, /^caj2pdf-hnc8-/);
        assert.equal(options.recursive, true);
        removed = true;
      },
    };
    await assert.rejects(withBrowserScratch(async () => { throw failure; }, {
      maxBytes: 8n, storage: { async getDirectory() { return root; } },
    }), (actual) => actual === failure);
    assert.equal(removed, true);
    assert.equal(closed.length, failureIndex);
  }
});

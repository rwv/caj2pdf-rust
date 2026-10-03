// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { blobSource, convert, withHnc8Scratch } from "../node.mjs";
import { newInstance, validatePdf } from "./helpers.mjs";
import { syntheticNativeC8, qmStates } from "./hnc8-fixtures.mjs";

const fontBytes = await readFile(new URL("../../tests/fonts/geometric.ttf", import.meta.url));
const source = (bytes) => blobSource(new Blob([bytes]));
const sink = (parts = []) => ({ async writeChunk(bytes) { parts.push(bytes.slice()); return bytes.length; }, async flush() {} });
const roles = (font) => ({ cjk: font, latin: font, alternateLatin: font });

test("native C8 public Node path reuses ranged fonts for text and mixed pages", async (t) => {
  for (const mixed of [false, true]) {
    let maxRead = 0;
    const input = source(fontBytes);
    const font = { size: input.size, async readAt(offset, length, signal) { maxRead = Math.max(maxRead, length); return input.readAt(offset, Math.min(length, 3), signal); } };
    const parts = [];
    await withHnc8Scratch(async (scratch) => {
      const result = await convert(await newInstance(), source(syntheticNativeC8(mixed)), sink(parts), {
        includeBookmarks: false, chunkSize: 32, hnc8: { fonts: roles(font), scratch, qmStates },
      });
      assert.equal(result.pagesConverted, 1);
      assert.ok(scratch.every((store) => store.size === 0n));
    });
    assert.ok(maxRead > 0 && maxRead <= 32);
    const pdf = Buffer.concat(parts);
    assert.equal(pdf.toString("latin1").match(/\/FontFile2 /g).length, 1);
    assert.equal(pdf.toString("latin1").match(/<0041> Tj/g).length, mixed ? 2 : 1);
    await validatePdf(t, pdf, 1);
  }
});

test("native C8 font validation and failed reads preserve caller errors", async () => {
  const font = source(fontBytes);
  for (const fonts of [{}, { ...roles(font), decoration: { source: font, character: "ab" } }, { ...roles(font), decoration: { source: font, character: "\ud800" } }]) {
    await assert.rejects(convert(await newInstance(), source(syntheticNativeC8()), sink(), { hnc8: { fonts } }), TypeError);
  }
  const failure = new Error("caller font read failed");
  const broken = { size: font.size, async readAt() { throw failure; } };
  await assert.rejects(convert(await newInstance(), source(syntheticNativeC8()), sink(), {
    includeBookmarks: false, hnc8: { fonts: roles(broken) },
  }), (error) => error === failure);
  for (const bytes of [new Uint8Array(16), fontBytes]) {
    const input = syntheticNativeC8();
    if (bytes === fontBytes) new DataView(input.buffer).setUint16(110, 0xa0c2, true);
    await assert.rejects(convert(await newInstance(), source(input), sink(), {
      includeBookmarks: false, hnc8: { fonts: roles(source(bytes)) },
    }), { code: "HNC8" });
  }
});

test("cancellation during a font read resets the instance and preserves resource ownership", async () => {
  const wasm = await newInstance();
  const controller = new AbortController();
  const input = source(fontBytes);
  let reads = 0;
  const font = { size: input.size, async readAt(offset, length) {
    reads++;
    controller.abort();
    return input.readAt(offset, length);
  } };
  await assert.rejects(convert(wasm, source(syntheticNativeC8()), sink(), {
    signal: controller.signal, includeBookmarks: false, hnc8: { fonts: roles(font) },
  }), { name: "AbortError" });
  assert.equal(reads, 1);
  const result = await convert(wasm, source(syntheticNativeC8()), sink(), {
    includeBookmarks: false, hnc8: { fonts: roles(input) },
  });
  assert.equal(result.pagesConverted, 1);
});

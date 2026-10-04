// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFile, readdir, rm } from "node:fs/promises";
import { test } from "node:test";
import { blobSource, convert, spoolToTempFile, withHnc8Scratch } from "../node.mjs";
import { newInstance, tempDirectory, validatePdf } from "./helpers.mjs";
import { syntheticNativeC8, syntheticNativeHnb, syntheticNativeHnbMixed, syntheticNativeHnbAxes, syntheticType1Hn, qmStates } from "./hnc8-fixtures.mjs";

const fontBytes = await readFile(new URL("../../tests/fonts/geometric.ttf", import.meta.url));
const symbolBytes = await readFile(new URL("../../tests/fonts/symbols.ttf", import.meta.url));
const source = (bytes) => blobSource(new Blob([bytes]));
const sink = (parts = []) => ({ async writeChunk(bytes) { parts.push(bytes.slice()); return bytes.length; }, async flush() {} });
const roles = (font) => ({ cjk: font, latin: font, alternateLatin: font });
const required = (font) => ({ cjk: font, latin: font });

test("native C8/HN-B public Node path reuses ranged fonts and preserves pages", async (t) => {
  for (const [inputBytes, pages, glyphs, hasSymbols, hasJpeg, hasState3, latinState] of [[syntheticNativeC8(), 1, 1], [syntheticNativeC8(true), 1, 2], [syntheticNativeHnb(), 2, 2], [syntheticNativeHnb(0), 2, 2, true], [syntheticNativeHnbMixed(), 1, 2, false, true], [syntheticNativeHnb(2, true), 2, 2, false, false, true], [syntheticNativeHnbAxes(), 2, 2], ...[3, 28, 31].map(state => [syntheticNativeC8(false, state), 1, 1, false, false, false, state])]) {
    let maxRead = 0;
    const input = source(fontBytes);
    const font = { size: input.size, async readAt(offset, length, signal) { maxRead = Math.max(maxRead, length); return input.readAt(offset, Math.min(length, 3), signal); } };
    const parts = [];
    await withHnc8Scratch(async (scratch) => {
      const result = await convert(await newInstance(), source(inputBytes), sink(parts), {
        includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { ...roles(font), ...(hasSymbols ? { symbols: source(symbolBytes) } : {}), ...(hasState3 ? { latinState3: { ...font } } : {}), ...(latinState ? { [`latinState${latinState}`]: { ...font } } : {}) }, scratch, qmStates },
      });
      assert.equal(result.pagesConverted, pages);
      assert.ok(scratch.every((store) => store.size === 0n));
    });
    assert.ok(maxRead > 0 && maxRead <= 32);
    const pdf = Buffer.concat(parts);
    assert.equal(pdf.toString("latin1").match(/\/FontFile2 /g).length, hasSymbols || hasState3 || latinState ? 2 : 1);
    assert.equal(pdf.toString("latin1").match(/<0041> Tj/g).length, glyphs);
    if (hasSymbols) {
      assert.equal(pdf.toString("latin1").match(/<0020> Tj/g).length, pages);
      assert.equal(pdf.toString("latin1").match(/<FF1A> Tj/g).length, pages);
    }
    if (hasJpeg) {
      assert.ok(pdf.includes(syntheticType1Hn().jpeg));
      assert.ok(pdf.indexOf("/Im0 Do") >= 0 && pdf.indexOf("/Im0 Do") < pdf.indexOf("<0041> Tj"));
    }
    await validatePdf(t, pdf, pages);
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


test("forward-only fonts enforce spool limits and dispose after conversion outcomes", async () => {
  const directory = await tempDirectory("c8-font-spool");
  const options = { directory, maxBytes: BigInt(fontBytes.length) };
  try {
    await assert.rejects(spoolToTempFile(new Blob([fontBytes]).stream(), { ...options, maxBytes: options.maxBytes - 1n }), { code: "LIMIT_EXCEEDED" });
    assert.deepEqual(await readdir(directory), []);
    for (const mode of ["success", "missing-glyph", "cancel"]) {
      const spool = await spoolToTempFile(new Blob([fontBytes]).stream(), options);
      try {
        const input = syntheticNativeC8();
        if (mode === "missing-glyph") new DataView(input.buffer).setUint16(110, 0xa0c2, true);
        const controller = new AbortController();
        const font = mode === "cancel" ? { size: spool.source.size, async readAt(offset, length) {
          controller.abort(); return spool.source.readAt(offset, length);
        } } : spool.source;
        const operation = convert(await newInstance(), source(input), sink(), {
          includeBookmarks: false, signal: controller.signal, hnc8: { fonts: roles(font) },
        });
        if (mode === "success") assert.equal((await operation).pagesConverted, 1);
        else await assert.rejects(operation, mode === "cancel" ? { name: "AbortError" } : { code: "HNC8" });
      } finally { await spool.dispose(); }
      assert.deepEqual(await readdir(directory), []);
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

// Without a symbol role, the visible space falls back to the Latin font,
// which lacks it; the located missing-glyph error is preserved.
test("HN-B symbols missing from the fallback font fail and clean scratch", async () => {
  const wasm = await newInstance();
  const font = source(fontBytes);
  for (const symbols of [undefined, font]) {
    await withHnc8Scratch(async (scratch) => {
      await assert.rejects(convert(wasm, source(syntheticNativeHnb(0)), sink(), {
        includeBookmarks: false, hnc8: { fonts: { ...roles(font), symbols }, scratch },
      }), { code: "HNC8" });
      assert.ok(scratch.every((store) => store.size === 0n));
    });
  }
});

test("HN-B image after text fails explicitly and releases scratch", async () => {
  const bytes = syntheticNativeHnbMixed();
  const image = bytes.slice(236, 264);
  bytes.copyWithin(236, 264, 276);
  bytes.set(image, 248);
  await withHnc8Scratch(async (scratch) => {
    await assert.rejects(convert(await newInstance(), source(bytes), sink(), {
      includeBookmarks: false, hnc8: { fonts: roles(source(fontBytes)), scratch, qmStates },
    }), (error) => error.code === "HNC8" && /image after text or drawing/.test(error.message));
    assert.ok(scratch.every((store) => store.size === 0n));
  });
});

// Absent optional roles use the core CJK/Latin fallback; only the two
// required roles are needed. One font then serves every role.
test("absent optional Latin roles fall back to the required fonts", async (t) => {
  for (const [input, pages] of [[syntheticNativeHnb(2, true), 2], ...[3, 28, 31].map((state) => [syntheticNativeC8(false, state), 1])]) {
    const parts = [];
    await withHnc8Scratch(async (scratch) => {
      const result = await convert(await newInstance(), source(input), sink(parts), {
        includeBookmarks: false, hnc8: { fonts: required(source(fontBytes)), scratch },
      });
      assert.equal(result.pagesConverted, pages);
      assert.ok(scratch.every((store) => store.size === 0n));
    });
    const pdf = Buffer.concat(parts);
    assert.equal(pdf.toString("latin1").match(/\/FontFile2 /g).length, 1);
    await validatePdf(t, pdf, pages);
  }
});


test("HN-B late unknown record leaves unfinished output and clears scratch", async () => {
  const bytes = syntheticNativeHnb();
  const view = new DataView(bytes.buffer);
  view.setUint16(view.getUint32(228, true), 0x8099, true);
  const parts = [];
  await withHnc8Scratch(async (scratch) => {
    await assert.rejects(convert(await newInstance(), source(bytes), sink(parts), {
      includeBookmarks: false, hnc8: { fonts: roles(source(fontBytes)), scratch },
    }), (error) => error.code === "HNC8" && /page 2/.test(error.message));
    assert.ok(scratch.every((store) => store.size === 0n));
  });
  const pdf = Buffer.concat(parts).toString("latin1");
  assert.ok(pdf.includes("<0041> Tj"), "first page must have been written");
  assert.ok(!pdf.includes("%%EOF"), "failure must not finalize the PDF");
});

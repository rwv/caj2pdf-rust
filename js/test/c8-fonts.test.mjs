// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFile, readdir, rm } from "node:fs/promises";
import { test } from "node:test";
import { convert, spoolToTempFile } from "../node.mjs";
import { pageText, tempDirectory, trackedBlob, validatePdf, wasmModule } from "./helpers.mjs";
import { syntheticNativeHnbProfile, privateAliasFont, syntheticNativeC8Profiles, syntheticC8, syntheticHn, syntheticNativeC8, syntheticNativeHnb, syntheticNativeHnbMixed, syntheticNativeHnbAxes, syntheticType1Hn } from "./hnc8-fixtures.mjs";

const fontBytes = await readFile(new URL("../../tests/fonts/geometric.ttf", import.meta.url));
const symbolBytes = await readFile(new URL("../../tests/fonts/symbols.ttf", import.meta.url));
const collectionBytes = await readFile(new URL("../../tests/fonts/collection.ttc", import.meta.url));
const cffBytes = await readFile(new URL("../../tests/fonts/geometric.otf", import.meta.url));
const source = (bytes) => new Blob([bytes]);
const sink = (parts = []) => ({ async writeChunk(bytes) { parts.push(bytes.slice()); return bytes.length; }, async flush() {} });
const roles = (font) => ({ cjk: font, latin: font, alternateLatin: font });
const required = (font) => ({ cjk: font, latin: font });

test("native C8/HN-B public Node path reads fonts in bounded ranges and preserves pages", async (t) => {
  for (const [inputBytes, pages, glyphs, hasSymbols, hasJpeg, hasState3, latinState] of [[syntheticNativeC8(), 1, 1], [syntheticNativeC8(true), 1, 2], [syntheticNativeHnb(), 2, 2], [syntheticNativeHnb(0), 2, 2, true], [syntheticNativeHnbMixed(), 1, 2, false, true], [syntheticNativeHnb(2, true), 2, 2, false, false, true], [syntheticNativeHnbAxes(), 2, 2], ...[3, 28, 31].map(state => [syntheticNativeC8(false, state), 1, 1, false, false, false, state])]) {
    const record = {};
    // A second Blob of the same bytes is a distinct font resource.
    const font = trackedBlob(new Blob([fontBytes]), record);
    const other = () => trackedBlob(new Blob([fontBytes]), record);
    const parts = [];
    const result = await convert(await wasmModule(), source(inputBytes), sink(parts), {
      includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { ...roles(font), ...(hasSymbols ? { symbols: source(symbolBytes) } : {}), ...(hasState3 ? { latinState3: other() } : {}), ...(latinState ? { [`latinState${latinState}`]: other() } : {}) } },
    });
    assert.equal(result.pagesConverted, pages);
    assert.ok(record.maxRead > 0 && record.maxRead <= 32);
    const pdf = Buffer.concat(parts);
    const text = pageText(pdf);
    assert.equal(text.match(/\/FontFile2 /g).length, hasSymbols || hasState3 || latinState ? 2 : 1);
    assert.equal(text.match(/<0041> Tj/g).length, glyphs);
    if (hasSymbols) {
      assert.equal(text.match(/<0020> Tj/g).length, pages);
      assert.equal(text.match(/<FF1A> Tj/g).length, pages);
    }
    if (hasJpeg) {
      assert.ok(pdf.includes(syntheticType1Hn().jpeg));
      assert.ok(text.indexOf("/Im0 Do") >= 0 && text.indexOf("/Im0 Do") < text.indexOf("<0041> Tj"));
    }
    await validatePdf(t, pdf, pages);
  }
});

test("image HN-A and compressed-text C8 inputs ignore supplied fonts", async (t) => {
  for (const inputBytes of [syntheticHn(), syntheticC8()]) {
    const outputs = [];
    for (const fonts of [undefined, roles(source(fontBytes))]) {
      const parts = [];
      const result = await convert(await wasmModule(), source(inputBytes), sink(parts), {
        includeBookmarks: false, ...(fonts ? { hnc8: { fonts } } : {}),
      });
      assert.equal(result.pagesConverted, 1);
      outputs.push(Buffer.concat(parts));
    }
    assert.deepEqual(outputs[1], outputs[0]);
    assert.ok(!outputs[0].toString("latin1").includes("/FontFile2 "));
    await validatePdf(t, outputs[0], 1);
  }
});

test("requested C8 bookmarks are omitted and reported, not a failure", async () => {
  const outputs = [];
  for (const includeBookmarks of [false, true]) {
    const parts = [];
    const result = await convert(await wasmModule(), source(syntheticNativeC8()), sink(parts), {
      includeBookmarks, hnc8: { fonts: roles(source(fontBytes)) },
    });
    assert.equal(result.outlineOmitted, includeBookmarks);
    assert.equal(result.bookmarksWritten, 0);
    outputs.push(Buffer.concat(parts));
  }
  assert.deepEqual(outputs[0], outputs[1]);
});

test("native C8 font validation and failed reads preserve caller errors", async () => {
  const font = source(fontBytes);
  for (const fonts of [{}, { ...roles(font), decoration: { source: font, character: "ab" } }, { ...roles(font), decoration: { source: font, character: "\ud800" } }]) {
    await assert.rejects(convert(await wasmModule(), source(syntheticNativeC8()), sink(), { hnc8: { fonts } }), TypeError);
  }
  const failure = new Error("caller font read failed");
  class Broken extends Blob {
    slice() {
      throw failure;
    }
  }
  const broken = new Broken([fontBytes]);
  await assert.rejects(convert(await wasmModule(), source(syntheticNativeC8()), sink(), {
    includeBookmarks: false, hnc8: { fonts: roles(broken) },
  }), (error) => error === failure);
  for (const bytes of [new Uint8Array(16), fontBytes]) {
    const input = syntheticNativeC8();
    if (bytes === fontBytes) new DataView(input.buffer).setUint16(110, 0xa0c2, true);
    await assert.rejects(convert(await wasmModule(), source(input), sink(), {
      includeBookmarks: false, hnc8: { fonts: roles(source(bytes)) },
    }), { code: "HNC8" });
  }
});

test("cancellation during a font read stops further reads and the module converts again", async () => {
  const wasm = await wasmModule();
  const controller = new AbortController();
  const input = source(fontBytes);
  let reads = 0;
  class Aborting extends Blob {
    slice(start, end) {
      reads++;
      controller.abort();
      return super.slice(start, end);
    }
  }
  const font = new Aborting([fontBytes]);
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
        const operation = convert(await wasmModule(), source(input), sink(), {
          includeBookmarks: false,
          signal: controller.signal,
          ...(mode === "cancel" ? { progress: () => controller.abort() } : {}),
          hnc8: { fonts: roles(spool.source) },
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
test("HN-B symbols missing from the fallback font fail", async () => {
  const wasm = await wasmModule();
  const font = source(fontBytes);
  for (const symbols of [undefined, font]) {
    await assert.rejects(convert(wasm, source(syntheticNativeHnb(0)), sink(), {
      includeBookmarks: false, hnc8: { fonts: { ...roles(font), symbols } },
    }), { code: "HNC8" });
  }
});

test("HN-B JPEG after text remains explicitly unsupported", async () => {
  const bytes = syntheticNativeHnbMixed();
  const image = bytes.slice(236, 264);
  bytes.copyWithin(236, 264, 276);
  bytes.set(image, 248);
  await assert.rejects(convert(await wasmModule(), source(bytes), sink(), {
    includeBookmarks: false, hnc8: { fonts: roles(source(fontBytes)) },
  }), (error) => error.code === "HNC8" && /image after text or drawing/.test(error.message));
});

// Absent optional roles use the core CJK/Latin fallback; only the two
// required roles are needed. One font then serves every role.
test("absent optional Latin roles fall back to the required fonts", async (t) => {
  for (const [input, pages] of [[syntheticNativeHnb(2, true), 2], ...[3, 28, 31].map((state) => [syntheticNativeC8(false, state), 1])]) {
    const parts = [];
    const result = await convert(await wasmModule(), source(input), sink(parts), {
      includeBookmarks: false, hnc8: { fonts: required(source(fontBytes)) },
    });
    assert.equal(result.pagesConverted, pages);
    const pdf = Buffer.concat(parts);
    const text = pageText(pdf);
    assert.equal(text.match(/\/FontFile2 /g).length, 1);
    await validatePdf(t, pdf, pages);
  }
});

test("HN-B late unknown record leaves unfinished output", async () => {
  const bytes = syntheticNativeHnb();
  const view = new DataView(bytes.buffer);
  view.setUint16(view.getUint32(228, true), 0x8099, true);
  const parts = [];
  await assert.rejects(convert(await wasmModule(), source(bytes), sink(parts), {
    includeBookmarks: false, hnc8: { fonts: roles(source(fontBytes)) },
  }), (error) => error.code === "HNC8" && /page 2/.test(error.message));
  const pdf = pageText(Buffer.concat(parts));
  assert.ok(pdf.includes("<0041> Tj"), "first page must have been written");
  assert.ok(!pdf.includes("%%EOF"), "failure must not finalize the PDF");
});

test("collection faces are selected per role and embedded as distinct fonts", async (t) => {
  const collection = source(collectionBytes);
  const parts = [];
  const result = await convert(await wasmModule(), source(syntheticNativeHnb(0)), sink(parts), {
    includeBookmarks: false,
    hnc8: { fonts: { cjk: { source: collection }, latin: { source: collection, face: 0 }, alternateLatin: collection, symbols: { source: collection, face: 1 } } },
  });
  assert.equal(result.pagesConverted, 2);
  const pdf = Buffer.concat(parts);
  const text = pageText(pdf);
  // Face 0 is shared by three roles; face 1 is a second embedded font.
  assert.equal(text.match(/\/FontFile2 /g).length, 2);
  assert.equal(text.match(/<FF1A> Tj/g).length, 2);
  await validatePdf(t, pdf, 2);
  await assert.rejects(convert(await wasmModule(), source(syntheticNativeC8()), sink([]), {
    includeBookmarks: false, hnc8: { fonts: { cjk: { source: collection, face: 2 }, latin: collection } },
  }), (error) => error.code === "HNC8" && /face index is out of range/.test(error.message));
  await assert.rejects(convert(await wasmModule(), source(syntheticNativeC8()), sink([]), {
    hnc8: { fonts: { cjk: { source: collection, face: -1 }, latin: collection } },
  }), RangeError);
});

test("CFF-flavoured OpenType fonts embed CID-keyed subsets", async (t) => {
  const parts = [];
  const result = await convert(await wasmModule(), source(syntheticNativeC8(true)), sink(parts), {
    includeBookmarks: false, hnc8: { fonts: roles(source(cffBytes)) },
  });
  assert.equal(result.pagesConverted, 1);
  const pdf = Buffer.concat(parts);
  const text = pageText(pdf);
  assert.equal(text.match(/\/FontFile3 /g).length, 1);
  assert.ok(text.includes("/Subtype /CIDFontType0 "));
  assert.ok(!text.includes("/CIDToGIDMap"));
  assert.equal(text.match(/<0041> Tj/g).length, 2);
  await validatePdf(t, pdf, 1);
});

test("native C8 new profiles retain six glyphs and the aligned-name JPEG", async (t) => {
  let baseline;
  for (const padded of [false, true]) {
    const record = {};
    const font = trackedBlob(new Blob([fontBytes]), record);
    const parts = [];
    const result = await convert(await wasmModule(), source(syntheticNativeC8Profiles(padded)), sink(parts), {
      includeBookmarks: false, chunkSize: 32, hnc8: { fonts: roles(font) },
    });
    assert.equal(result.pagesConverted, 1);
    assert.ok(record.maxRead > 0 && record.maxRead <= 32);
    const pdf = Buffer.concat(parts);
    assert.equal(pageText(pdf).match(/<0041> Tj/g).length, 6);
    assert.ok(pdf.includes(syntheticType1Hn().jpeg));
    if (baseline) assert.deepEqual(pdf, baseline);
    baseline = pdf;
    await validatePdf(t, pdf, 1);
  }
});


test("HN-B profile retains private codes and reports visual substitution", async (t) => {
  for (const [code, substitutions, shown] of [[0x0403, 1n, "0403"], [0xe6c7, 0n, "E6C7"]]) {
    const parts = [];
    const font = source(privateAliasFont(fontBytes, code));
    const result = await convert(await wasmModule(), source(syntheticNativeHnbProfile()), sink(parts), {
      includeBookmarks: false, chunkSize: 32, hnc8: { fonts: required(font) },
    });
    assert.equal(result.substitutedGlyphs, substitutions);
    assert.equal(result.pagesConverted, 1);
    const pdf = Buffer.concat(parts), text = pageText(pdf);
    assert.equal(text.match(/<0041> Tj/g).length, 2);
    assert.ok(text.includes(`<${shown}> Tj`));
    assert.equal(text.includes("/ActualText <FEFFE6C7>"), substitutions === 1n);
    await validatePdf(t, pdf, 1);
  }
});

// symbols.ttf labels a rectangle U+0020 and a triangle U+FF1A. Swapping them
// for the mode-0 space and colon codes keeps the decoded text.
test("HN-B mode-0 symbol glyphs select source shapes independently of text", async (t) => {
  const font = source(fontBytes);
  const parts = [];
  const result = await convert(await wasmModule(), source(syntheticNativeHnb(0)), sink(parts), {
    includeBookmarks: false,
    hnc8: { fonts: { ...roles(font), symbols: source(symbolBytes), symbolGlyphs: [{ code: 0xa1a1, glyph: "：" }, { code: 0xa3ba, glyph: " " }] } },
  });
  assert.equal(result.pagesConverted, 2);
  const pdf = Buffer.concat(parts);
  const text = pageText(pdf);
  assert.ok(!/<0020> Tj|<FF1A> Tj/.test(text));
  assert.equal(text.match(/<0001> Tj/g).length, 2);
  assert.equal(text.match(/<0002> Tj/g).length, 2);
  assert.ok(text.includes("/CajMappedUnicode"));
  assert.ok(text.includes("<0001> <0020>\n<0002> <FF1A>"));
  await validatePdf(t, pdf, 2);
});

test("HN-B mode-0 symbol glyph configuration is validated", async () => {
  const wasm = await wasmModule();
  const font = source(fontBytes);
  const fonts = (symbolGlyphs) => ({ ...roles(font), symbols: source(symbolBytes), symbolGlyphs });
  const run = (value) => convert(wasm, source(syntheticNativeHnb(0)), sink(), { includeBookmarks: false, hnc8: { fonts: value } });
  await assert.rejects(run({ ...roles(font), symbolGlyphs: [{ code: 0xa1a1, glyph: " " }] }), TypeError);
  await assert.rejects(run(fonts([{ code: 0x10000, glyph: " " }])), RangeError);
  for (const glyph of ["ab", "\ud800", "😀", 32]) {
    await assert.rejects(run(fonts([{ code: 0xa1a1, glyph }])), TypeError);
  }
  for (const symbolGlyphs of [[{ code: 0xa3c1, glyph: " " }], [{ code: 0xa1a1, glyph: " " }, { code: 0xa1a1, glyph: "：" }]]) {
    await assert.rejects(run(fonts(symbolGlyphs)), (error) => /symbol glyph/.test(error.message));
  }
  // The symbols font must map every selected glyph; nothing falls back.
  await assert.rejects(run(fonts([{ code: 0xa1a1, glyph: "A" }])), { code: "HNC8" });
});

// symbols.ttf: PostScript name CajFixture, head checkSumAdjustment 0x79f42dc3.
test("HN-B symbol glyph maps bound to a font identity refuse other fonts", async () => {
  const wasm = await wasmModule();
  const font = source(fontBytes);
  const symbolGlyphs = [{ code: 0xa1a1, glyph: "：" }, { code: 0xa3ba, glyph: " " }];
  const run = async (symbolFontIdentity) => {
    const parts = [];
    await convert(wasm, source(syntheticNativeHnb(0)), sink(parts), {
      includeBookmarks: false, hnc8: { fonts: { ...roles(font), symbols: source(symbolBytes), symbolGlyphs, symbolFontIdentity } },
    });
    return Buffer.concat(parts);
  };
  assert.deepEqual(await run({ postscriptName: "CajFixture", checksumAdjustment: 0x79f42dc3 }), await run(undefined));
  for (const identity of [{ postscriptName: "CajFixture", checksumAdjustment: 0x79f42dc4 }, { postscriptName: "Other", checksumAdjustment: 0x79f42dc3 }]) {
    await assert.rejects(run(identity), (error) => error.code === "HNC8" && /expected identity/.test(error.message));
  }
  for (const identity of [{ postscriptName: "", checksumAdjustment: 1 }, { postscriptName: "a b", checksumAdjustment: 1 }, { postscriptName: "x".repeat(64), checksumAdjustment: 1 }]) {
    await assert.rejects(run(identity), TypeError);
  }
  await assert.rejects(run({ postscriptName: "CajFixture", checksumAdjustment: 2 ** 32 }), RangeError);
  await assert.rejects(convert(wasm, source(syntheticNativeHnb(0)), sink(), {
    includeBookmarks: false, hnc8: { fonts: { ...roles(font), symbols: source(symbolBytes), symbolFontIdentity: { postscriptName: "CajFixture", checksumAdjustment: 1 } } },
  }), TypeError);
});

// SPDX-License-Identifier: MIT

// HN/C8 conversion called from the caller's own Dedicated Worker, with fonts
// from OPFS file handles and Blobs. Each convert() starts a nested Worker.
import { convert, loadModule, spoolToOpfs } from "../browser.mjs";
import { syntheticNativeHnbProfile, privateAliasFont, syntheticNativeC8Profiles, syntheticNativeC8, syntheticNativeHnb, syntheticNativeHnbMixed, syntheticNativeHnbAxes, syntheticHn, syntheticType1Hn, syntheticPrefixedHn } from "./hnc8-fixtures.mjs";

const root = await navigator.storage.getDirectory();
const fontSpools = [];
let result;

function collect(parts) {
  return { async writeChunk(bytes) { parts.push(...bytes); return bytes.length; }, async flush() {} };
}

try {
  const module = await loadModule();
  const parts = [];
  const report = await convert(module, new Blob([syntheticHn(true, true)]), collect(parts), { chunkSize: 3 });
  const standardPdf = [];
  const standard = await convert(module, new Blob([syntheticHn()]), collect(standardPdf), { chunkSize: 3 });
  const type1Pdf = [];
  const type1 = await convert(module, new Blob([syntheticType1Hn().bytes]), collect(type1Pdf), { chunkSize: 3 });
  for (const markers of [false, true]) {
    const prefixedPdf = [];
    await convert(module, new Blob([syntheticPrefixedHn(markers)]), collect(prefixedPdf), { chunkSize: 3 });
    if (prefixedPdf.length !== parts.length || prefixedPdf.some((byte, i) => byte !== parts[i])) {
      throw new Error("paired raw prefix or image markers changed the mixed-image PDF");
    }
  }
  const fontBlob = await (await fetch("/fixtures/geometric.ttf")).blob();
  try {
    await spoolToOpfs(fontBlob.stream(), { maxBytes: BigInt(fontBlob.size) - 1n });
    throw new Error("font spool limit was not enforced");
  } catch (error) { if (error.code !== "LIMIT_EXCEEDED") throw error; }
  // The spooled font is an OPFS file handle; the conversion Worker opens it.
  const fontSpool = await spoolToOpfs(fontBlob.stream(), { maxBytes: BigInt(fontBlob.size) });
  fontSpools.push(fontSpool);
  const font = fontSpool.source;
  const symbolBlob = await (await fetch("/fixtures/symbols.ttf")).blob();
  const symbolSpool = await spoolToOpfs(symbolBlob.stream(), { maxBytes: BigInt(symbolBlob.size) });
  fontSpools.push(symbolSpool);
  const symbols = symbolSpool.source;
  const nativePdfs = [];
  for (const [input, pages, hasSymbols, hasState3, latinState] of [[syntheticNativeC8(), 1], [syntheticNativeC8(true), 1], [syntheticNativeHnb(), 2], [syntheticNativeHnb(0), 2, true], [syntheticNativeHnbMixed(), 1], [syntheticNativeHnb(2, true), 2, false, true], [syntheticNativeHnbAxes(), 2], ...[3, 28, 31].map(state => [syntheticNativeC8(false, state), 1, false, false, state])]) {
    const pdf = [];
    // The Blob copy of the font is a second, distinct font resource.
    const native = await convert(module, new Blob([input]), collect(pdf), { includeBookmarks: false, chunkSize: 32, hnc8: {
      fonts: { cjk: font, latin: font, alternateLatin: font, ...(hasSymbols ? { symbols } : {}), ...(hasState3 ? { latinState3: fontBlob } : {}), ...(latinState ? { [`latinState${latinState}`]: fontBlob } : {}) },
    } });
    if (native.pagesConverted !== pages) throw new Error("native C8/HN-B page count mismatch");
    nativePdfs.push(pdf);
  }
  // Swapped symbol glyphs keep the decoded space and colon as text.
  const symbolGlyphPdf = [];
  await convert(module, new Blob([syntheticNativeHnb(0)]), collect(symbolGlyphPdf), { includeBookmarks: false, chunkSize: 32, hnc8: {
    fonts: { cjk: font, latin: font, alternateLatin: font, symbols, symbolGlyphs: [{ code: 0xa1a1, glyph: "\uff1a" }, { code: 0xa3ba, glyph: " " }] },
  } });
  let profilePdf;
  for (const padded of [false, true]) {
    const pdf = [];
    const report = await convert(module, new Blob([syntheticNativeC8Profiles(padded)]), collect(pdf), {
      includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { cjk: font, latin: font } },
    });
    if (report.pagesConverted !== 1) throw new Error("new C8 profile lost its page");
    if (profilePdf && (profilePdf.length !== pdf.length || pdf.some((byte, i) => byte !== profilePdf[i]))) {
      throw new Error("optional aligned-name padding changed C8 output");
    }
    profilePdf = pdf;
  }
  const hnbProfilePdf = [];
  const aliasFont = new Blob([privateAliasFont(await fontBlob.arrayBuffer())]);
  const hnbProfile = await convert(module, new Blob([syntheticNativeHnbProfile()]), collect(hnbProfilePdf), {
    includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { cjk: aliasFont, latin: aliasFont } },
  });
  if (hnbProfile.substitutedGlyphs !== 1n || hnbProfile.pagesConverted !== 1) {
    throw new Error("HN-B private-use substitution was not reported");
  }
  const lateInput = syntheticNativeHnb();
  const lateView = new DataView(lateInput.buffer);
  lateView.setUint16(lateView.getUint32(228, true), 0x8099, true);
  const lateParts = [];
  try {
    await convert(module, new Blob([lateInput]), collect(lateParts), { includeBookmarks: false, chunkSize: 32, hnc8: { fonts: { cjk: font, latin: font, alternateLatin: font } } });
    throw new Error("late HN-B record unexpectedly succeeded");
  } catch (error) {
    if (error.code !== "HNC8" || !/page 2/.test(error.message)) throw error;
  }
  const fontFailures = [];
  for (const mode of ["missing-glyph", "read-error", "cancel"]) {
    const input = syntheticNativeC8();
    if (mode === "missing-glyph") new DataView(input.buffer).setUint16(110, 0xa0c2, true);
    const controller = new AbortController();
    let failingFont = font;
    if (mode === "read-error") {
      // A handle whose file is gone cannot be read.
      const name = `caj2pdf-font-test-${crypto.randomUUID()}`;
      failingFont = await root.getFileHandle(name, { create: true });
      await root.removeEntry(name);
    }
    try {
      await convert(module, new Blob([input]), collect([]), {
        includeBookmarks: false,
        chunkSize: 32,
        signal: controller.signal,
        progress: mode === "cancel" ? () => controller.abort() : undefined,
        hnc8: { fonts: { cjk: failingFont, latin: failingFont, alternateLatin: failingFont } },
      });
      throw new Error(`expected ${mode} to fail`);
    } catch (error) {
      if (mode === "missing-glyph" && error.code !== "HNC8") throw error;
      if (mode === "read-error" && error.name !== "NotFoundError") throw error;
      if (mode === "cancel" && error.name !== "AbortError") throw error;
      fontFailures.push(mode);
    }
  }
  // An image-only HN-A input ignores supplied fonts: same bytes as without.
  const imageWithFonts = [];
  await convert(module, new Blob([syntheticHn()]), collect(imageWithFonts), { chunkSize: 3, hnc8: { fonts: { cjk: font, latin: font } } });
  if (imageWithFonts.length !== standardPdf.length || imageWithFonts.some((byte, i) => byte !== standardPdf[i])) {
    throw new Error("supplied fonts changed the image-only HN-A PDF");
  }
  result = { hnbProfilePdf, profilePdf, symbolGlyphPdf, fontFailures, latePdf: lateParts, nativePdfs, type1Pages: type1.pagesConverted, type1Pdf, standardPages: standard.pagesConverted, standardPdf, pages: report.pagesConverted, pdf: parts };
} catch (error) {
  result = { error: `${error.name}: ${error.message}` };
} finally {
  for (const spool of fontSpools) await spool.dispose();
}
result.remainingEntries = [];
for await (const [name] of root.entries()) result.remainingEntries.push(name);
postMessage(result);

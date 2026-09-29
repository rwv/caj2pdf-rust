// SPDX-License-Identifier: MIT

/** Invented constant arithmetic states, not the normative QM probability data. */
export const qmStates = Array.from({ length: 113 }, () => ({
  qe: 0x4000, nextLps: 0, nextMps: 0, switchMps: false,
}));

/** One raw HN-A page containing one or two original 3 x 2 type-0 images. */
export function syntheticHn(withBookmarks = false, twoImages = false) {
  const count = withBookmarks ? 2 : 0;
  const index = 0x15c + count * 308;
  const text = index + 20;
  const images = twoImages ? 2 : 1;
  const textLength = images * 28 + 4;
  const descriptor = text + textLength;
  const bytes = new Uint8Array(descriptor + images * (12 + 49));
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  const u16 = (at, value) => view.setUint16(at, value, true);
  bytes.set([72, 78, 0, 0, 0x90, 1, 0, 0]);
  u32(0x90, 1); u32(0x158, count);
  for (let number = 0; number < count; number++) {
    const at = 0x15c + number * 308;
    bytes.set(new TextEncoder().encode(number === 0 ? "Root" : "Leaf"), at);
    bytes[at + 280] = 49; u32(at + 304, number + 1);
  }
  u32(index, text); u32(index + 4, textLength); u16(index + 8, images);
  for (let image = 0; image < images; image++) {
    const record = text + image * 28;
    u16(record, 0x800a);
    // The supplement has a nonzero origin to detect dropped or reordered draws.
    u16(record + 4, image * 13); u16(record + 6, image);
    const entry = descriptor + image * (12 + 49);
    const dib = entry + 12;
    u32(entry, 0); u32(entry + 4, dib); u32(entry + 8, 49);
    u32(dib, 40); u32(dib + 4, 3); u32(dib + 8, 2);
    u16(dib + 12, 1); u16(dib + 14, 1); u32(dib + 32, 2);
    bytes.fill(255, dib + 40, dib + 43);
    // Alternating rows 101 / 010 under the invented constant state table.
    bytes[dib + 48] = 0x92;
  }
  u16(text + images * 28, 0x8004);
  return bytes;
}


/** Metadata-only controls: outline layout is unknown for these variants. */
export function unknownOutline(format) {
  const index = format === "c8" ? 0x50 : 0xd8;
  const bytes = new Uint8Array(index + 20);
  bytes.set(format === "c8" ? [0xc8, 0, 0, 0] : [72, 78, 0, 0, 0xc8, 0, 0, 0]);
  new DataView(bytes.buffer).setUint32(format === "c8" ? 8 : 0x90, 1, true);
  return bytes;
}

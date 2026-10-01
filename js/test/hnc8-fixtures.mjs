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
  u16(0xa8, 100); u16(0xaa, 200);
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
    u16(record + 8, 80 - image * 20); u16(record + 10, 40 + image * 10);
    const entry = descriptor + image * (12 + 49);
    const dib = entry + 12;
    u32(entry, 0); u32(entry + 4, dib); u32(entry + 8, 49);
    u32(dib, 40); u32(dib + 4, 3); u32(dib + 8, 2);
    u16(dib + 12, 1); u16(dib + 14, 1); u32(dib + 32, 2);
    bytes.fill(255, dib + 40, dib + 43);
    // Original rows: 101 / 010, then 110 / 001. Their constant-state
    // interval lower bounds are 0x9200 and 0xa100 (see core compose tests).
    bytes[dib + 48] = image === 0 ? 0x92 : 0xa1;
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

/** Original C8 wrapper with a stored zlib frame around raw text records. */
export function syntheticC8() {
  const hn = syntheticHn();
  const text = hn.slice(0x170, 0x190);
  // One uncompressed DEFLATE block inside a zlib frame; Adler-32 is computed
  // from our original text bytes, without a runtime compression dependency.
  let a = 1, b = 0;
  for (const byte of text) { a = (a + byte) % 65521; b = (b + a) % 65521; }
  const frame = new Uint8Array(2 + 5 + text.length + 4);
  frame.set([0x78, 0x01, 1, text.length, 0, 255 - text.length, 255]);
  frame.set(text, 7);
  new DataView(frame.buffer).setUint32(frame.length - 4, ((b << 16) | a) >>> 0);
  const textLength = 16 + frame.length;
  const descriptor = 0x64 + textLength;
  const bytes = new Uint8Array(descriptor + 61);
  const view = new DataView(bytes.buffer);
  bytes.set([0xc8, 0, 0, 0]);
  view.setUint32(8, 1, true);
  view.setUint16(32, 100, true); view.setUint16(34, 200, true);
  view.setUint32(0x50, 0x64, true); view.setUint32(0x54, textLength, true);
  view.setUint16(0x58, 1, true);
  bytes.set(new TextEncoder().encode("COMPRESSTEXT"), 0x64);
  view.setUint32(0x70, text.length, true);
  bytes.set(frame, 0x74);
  bytes.set(hn.subarray(0x190), descriptor);
  view.setUint32(descriptor + 4, descriptor + 12, true);
  return bytes;
}

/** Original 16 x 8 grayscale JPEG: a dark left block and light right block.
 * Uses the original core composition fixture's custom Huffman coding.
 */
export function syntheticType1Hn() {
  const jpeg = [0xff, 0xd8];
  const segment = (marker, body) => {
    const length = body.length + 2;
    jpeg.push(0xff, marker, length >> 8, length & 255, ...body);
  };
  segment(0xe0, [74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
  segment(0xdb, [0, ...Array(64).fill(1)]);
  segment(0xc0, [8, 0, 8, 0, 16, 1, 1, 0x11, 0]);
  segment(0xc4, [0, 0, 0, 0, 12, ...Array(12).fill(0), ...Array.from({ length: 12 }, (_, i) => i)]);
  segment(0xc4, [0x10, 1, ...Array(15).fill(0), 0]);
  segment(0xda, [1, 1, 0, 0, 63, 0]);
  let bits = "", previous = 0;
  for (const sample of [32, 224]) {
    const value = 8 * (sample - 128), difference = value - previous;
    previous = value;
    const category = 32 - Math.clz32(Math.abs(difference));
    const amplitude = difference < 0 ? difference + 2 ** category - 1 : difference;
    bits += category.toString(2).padStart(4, "0");
    if (category) bits += amplitude.toString(2).padStart(category, "0");
    bits += "0"; // EOB.
  }
  bits = bits.padEnd(Math.ceil(bits.length / 8) * 8, "1");
  for (let at = 0; at < bits.length; at += 8) {
    const byte = Number.parseInt(bits.slice(at, at + 8), 2);
    jpeg.push(byte);
    if (byte === 255) jpeg.push(0);
  }
  jpeg.push(0xff, 0xd9);
  const bytes = new Uint8Array(0x19c + jpeg.length);
  bytes.set(syntheticHn().subarray(0, 0x19c));
  const view = new DataView(bytes.buffer);
  view.setUint32(0x190, 1, true);
  view.setUint32(0x198, jpeg.length, true);
  bytes.set(jpeg, 0x19c);
  return { bytes, jpeg: new Uint8Array(jpeg) };
}

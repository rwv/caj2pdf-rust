// SPDX-License-Identifier: MIT

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
    // Original rows: 101 / 010, then 110 / 001, each a one-byte SCD under
    // the standard QM states.
    bytes[dib + 48] = image === 0 ? 0x39 : 0x0a;
  }
  u16(text + images * 28, 0x8004);
  return bytes;
}


/** Metadata-only controls: outline layout is unknown for these variants. */
export function unknownOutline(format) {
  const index = format === "c8" ? 0x50 : 0xd8;
  const bytes = new Uint8Array(index + 20);
  bytes.set(format === "c8" ? [0xc8, 0, 0, 0] : [72, 78, 0, 0, 0xc8, 0, 0, 0]);
  if (format !== "c8") new DataView(bytes.buffer).setUint32(0x88, 0xc8, true);
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

/** The same original mixed-image page with paired raw page-prefix records. */
export function syntheticPrefixedHn(markers = false) {
  const original = syntheticHn(true, true);
  const index = 0x15c + 2 * 308;
  const text = index + 20;
  const extra = 16;
  const bytes = new Uint8Array(original.length + extra);
  bytes.set(original.subarray(0, text));
  bytes.set(original.subarray(text), text + extra);
  const view = new DataView(bytes.buffer);
  for (const [i, tag, value] of [[0, 0x8003, 100], [4, 0x8003, 200], [8, 0x801c, 0], [12, 0x80ce, 0]]) {
    view.setUint16(text + i, tag, true);
    view.setUint16(text + i + 2, value, true);
  }
  const textLength = 60 + extra;
  view.setUint32(index + 4, textLength, true);
  for (let i = 0; i < 2; i++) {
    if (markers) {
      const record = text + extra + i * 28;
      view.setUint16(record + 2, 0xd300, true);
      for (const field of [4, 8]) {
        view.setUint16(record + field, view.getUint16(record + field, true) | 0xc000, true);
      }
    }
    const descriptor = text + textLength + i * 61;
    view.setUint32(descriptor + 4, descriptor + 12, true);
  }
  return bytes;
}

/** Original native C8 page: geometric-font A, optionally a type-0 image, then A. */
export function syntheticNativeC8(mixed = false, latinState) {
  const words = [[0x8001, 60], [0x8002, 0x1084], [30, 0xa0c1]];
  if (latinState !== undefined) words.splice(2, 0, [0x801d, latinState]);
  if (mixed) words.push([0x800a, 0xd300], [0xc014, 40], [0xc050, 40], [0xc050, 0xc033], [0xc037, 0xc000], [0xc06c, 0xc032], [0xc0f2, 0xc07a], [45, 0xa0c1]);
  words.push([0x8004, 39]);
  const end = 100 + words.length * 4;
  const bytes = new Uint8Array(end + (mixed ? 61 : 0));
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  bytes[0] = 0xc8; u32(8, 1); u32(12, 2);
  view.setUint16(32, 100, true); view.setUint16(34, 200, true);
  u32(80, 100); u32(84, words.length * 4); u32(88, Number(mixed));
  for (const [i, value] of words.flat().entries()) view.setUint16(100 + i * 2, value, true);
  if (mixed) { u32(end, 0); u32(end + 4, end + 12); u32(end + 8, 49); bytes.set(syntheticHn().slice(-49), end + 12); }
  u32(96, bytes.length);
  return bytes;
}

/** Original two-page compact HN-B text fixture using bare page ends. */
export function syntheticNativeHnb(mode = 2, latinState3 = false) {
  let text = syntheticNativeC8().slice(100, -2);
  if (mode === 0) {
    const extended = new Uint8Array(text.length + 8);
    extended.set(text.subarray(0, -2));
    const records = new DataView(extended.buffer);
    records.setUint16(10, 0xa3c1, true);
    for (const [index, word] of [40, 0xa1a1, 60, 0xa3ba, 0x8004].entries()) {
      records.setUint16(text.length - 2 + index * 2, word, true);
    }
    text = extended;
  }
  if (latinState3) {
    const extended = new Uint8Array(text.length + 4);
    extended.set(text.subarray(0, 8));
    new DataView(extended.buffer).setUint16(8, 0x801d, true);
    new DataView(extended.buffer).setUint16(10, 3, true);
    extended.set(text.subarray(8), 12);
    text = extended;
  }
  const bytes = new Uint8Array(240 + text.length * 2);
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  bytes.set([0x48, 0x4e]); u32(4, 200); u32(8, 136); u32(144, 2); u32(148, mode);
  view.setUint16(168, 100, true); view.setUint16(170, 200, true);
  for (let page = 0; page < 2; page++) {
    const offset = 240 + page * text.length;
    u32(216 + page * 12, offset); u32(220 + page * 12, text.length);
    bytes.set(text, offset);
  }
  return bytes;
}

/** Original ordinary-index HN-B page with one image followed by two glyphs. */
export function syntheticNativeHnbMixed() {
  const c8 = syntheticNativeC8(true);
  const old = new DataView(c8.buffer);
  const length = old.getUint32(84, true);
  const jpeg = syntheticType1Hn().jpeg;
  const bytes = new Uint8Array(236 + length + 12 + jpeg.length);
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  bytes.set([0x48, 0x4e]); u32(4, 200); u32(8, 136);
  u32(136, 0xc8); u32(144, 1); u32(148, 2);
  bytes.set(c8.subarray(28, 36), 164);
  u32(216, 236); u32(220, length); u32(224, 1); u32(232, bytes.length);
  // Move the original image ahead of both original glyphs.
  bytes.set(c8.subarray(112, 140), 236);
  bytes.set(c8.subarray(100, 112), 264);
  bytes.set(c8.subarray(140, 100 + length), 276);
  u32(236 + length, 2);
  u32(236 + length + 4, 236 + length + 12);
  u32(236 + length + 8, jpeg.length);
  bytes.set(jpeg, 236 + length + 12);
  return bytes;
}

/** Original compact HN-B with rectangular axes and no style record. */
export function syntheticNativeHnbAxes() {
  const base = syntheticNativeHnb();
  const bytes = new Uint8Array(276);
  bytes.set(base.subarray(0, 240));
  const view = new DataView(bytes.buffer);
  for (let page = 0; page < 2; page++) {
    const offset = 240 + page * 18;
    view.setUint32(216 + page * 12, offset, true);
    view.setUint32(220 + page * 12, 18, true);
    bytes.set(base.subarray(240, 244), offset);
    for (const [index, word] of [0x8070, 43, 0x8071, 28].entries()) {
      view.setUint16(offset + 4 + index * 2, word, true);
    }
    bytes.set(base.subarray(248, 254), offset + 12);
  }
  return bytes;
}

/** Original C8 controls for terminated strings, explicit sizes and an aligned image name. */
export function syntheticNativeC8Profiles(padded = false) {
  const words = [
    0x8001, 60, 0x8070, 38, 0x8071, 38, 30, 0xa0c1,
    0x80cc, 0x0104, 0xe041, 0xe000,
    0x8002, 0x094a, 40, 0xa0c1,
    0x8070, 22, 0x8071, 22, 50, 0xa0c1,
    0x8070, 34, 0x8071, 34, 60, 0xa0c1,
    0x801c, 2, 0x8070, 40, 0x8071, 40, 70, 0xa0c1,
    0x8002, 0xa4a5, 80, 0xa0c1,
    0x810a, 0xd300, 20, 40, 80, 40, 0, 4, 0x4241, 0x4443,
    ...(padded ? [0, 0] : []), 0x8004, 1,
  ];
  const jpeg = syntheticType1Hn().jpeg;
  const end = 100 + words.length * 2;
  const bytes = new Uint8Array(end + 12 + jpeg.length);
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  bytes[0] = 0xc8; u32(8, 1); u32(12, 2);
  view.setUint16(32, 600, true); view.setUint16(34, 600, true);
  u32(80, 100); u32(84, words.length * 2); u32(88, 1); u32(96, bytes.length);
  words.forEach((word, i) => view.setUint16(100 + i * 2, word, true));
  u32(end, 2); u32(end + 4, end + 12); u32(end + 8, jpeg.length);
  bytes.set(jpeg, end + 12);
  return bytes;
}

/** Original mode-2 HN-B title, metadata, zero-size and private-code controls. */
export function syntheticNativeHnbProfile() {
  const words = [
    0x8001, 100, 0x8002, 0x0929, 30, 0xa0c1,
    0x8067, 18, 0x8072, 0x8004, 0x8073, 278, 0x8074, 0xffff,
    0x8002, 0x1000, 50, 0xa0c1,
    0x8002, 0x1084, 70, 0xa661, 0x8004, 1,
  ];
  const bytes = new Uint8Array(228 + words.length * 2);
  bytes.set(syntheticNativeHnb().subarray(0, 216));
  const view = new DataView(bytes.buffer);
  view.setUint32(144, 1, true);
  view.setUint32(216, 228, true);
  view.setUint32(220, words.length * 2, true);
  words.forEach((word, i) => view.setUint16(228 + i * 2, word, true));
  return bytes;
}

/** Relabel the MIT geometric font's second shape; no external outline bytes. */
export function privateAliasFont(original, code = 0x0403) {
  const bytes = new Uint8Array(original);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < view.getUint16(4); i++) {
    const entry = 12 + i * 16;
    if (view.getUint32(entry) === 0x636d6170) {
      const cmap = view.getUint32(entry + 8);
      view.setUint32(cmap + 40, code);
      view.setUint32(cmap + 44, code);
      return bytes;
    }
  }
  throw new Error("original geometric font has no cmap");
}

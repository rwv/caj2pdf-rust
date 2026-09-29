// SPDX-License-Identifier: MIT

/** Invented constant arithmetic states, not the normative QM probability data. */
export const qmStates = Array.from({ length: 113 }, () => ({
  qe: 0x4000, nextLps: 0, nextMps: 0, switchMps: false,
}));

/** Original 3 x 2 asymmetric type-0 image in one raw image-first HN-A page. */
export function syntheticHn() {
  const index = 0x15c;
  const text = index + 20;
  const descriptor = text + 32;
  const payload = descriptor + 12;
  const bytes = new Uint8Array(payload + 49);
  const view = new DataView(bytes.buffer);
  const u32 = (at, value) => view.setUint32(at, value, true);
  const u16 = (at, value) => view.setUint16(at, value, true);
  bytes.set([72, 78, 0, 0, 0x90, 1, 0, 0]);
  u32(0x90, 1);
  u32(index, text); u32(index + 4, 32); u16(index + 8, 1);
  u16(text, 0x800a); u16(text + 28, 0x8004);
  u32(descriptor, 0); u32(descriptor + 4, payload); u32(descriptor + 8, 49);
  u32(payload, 40); u32(payload + 4, 3); u32(payload + 8, 2);
  u16(payload + 12, 1); u16(payload + 14, 1); u32(payload + 32, 2);
  bytes.fill(255, payload + 40, payload + 43);
  // Original alternating rows 101 / 010, each preceded by a per-pixel selector.
  // Their arithmetic interval under the invented constant table starts at 0x9200.
  bytes[payload + 48] = 0x92;
  return bytes;
}

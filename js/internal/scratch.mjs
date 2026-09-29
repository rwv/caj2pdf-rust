// SPDX-License-Identifier: MIT

import { requireU64 } from "../io.mjs";

/** File truncation and OPFS positions use exact JavaScript Numbers. */
export function scratchSize(size, maxBytes) {
  requireU64(maxBytes, "maxBytes");
  requireU64(size, "scratch size");
  if (maxBytes > BigInt(Number.MAX_SAFE_INTEGER) || size > maxBytes) {
    throw new RangeError("scratch size must fit maxBytes and the safe integer range");
  }
  return Number(size);
}

/** Preserve short I/O, but never accept an impossible host result. */
export function scratchCount(count, requested) {
  if (!Number.isSafeInteger(count) || count < 0 || count > requested) {
    throw new RangeError("scratch handle returned an invalid byte count");
  }
  return count;
}

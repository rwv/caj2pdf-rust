// SPDX-License-Identifier: MIT
import { convert, withHnc8Scratch, type WasmInput, type RangedSource, type SequentialSink } from '../../node.mjs';

export function useScopedHn(wasm: WasmInput, source: RangedSource, sink: SequentialSink) {
  return withHnc8Scratch((scratch) => convert(wasm, source, sink, { hnc8: { scratch } }), { maxBytes: 1024n });
}

// SPDX-License-Identifier: MIT

import type { ConvertOptions, ConversionReport, SequentialSink, Spooled, SpoolOptions, WasmInput } from "./io.mjs";

export * from "./io.mjs";

/** Fetch and compile the packaged `caj2pdf_wasm.wasm`, or the one at `url`. */
export declare function loadModule(url?: URL | string): Promise<WebAssembly.Module>;

/** The subset of `StorageManager` used for the OPFS spool. */
export interface SpoolStorage {
  getDirectory(): Promise<FileSystemDirectoryHandle>;
}

/**
 * Copy a stream into a uniquely named OPFS file bounded by `maxBytes`.
 * Rejects with `RANDOM_ACCESS_REQUIRED` when OPFS writes are unavailable.
 */
export declare function spoolToOpfs(
  stream: ReadableStream<Uint8Array>,
  options: { maxBytes: bigint; signal?: AbortSignal; storage?: SpoolStorage },
): Promise<Spooled>;

/** Spool to OPFS, convert, and always remove the OPFS file. */
export declare function convertReadableStream(
  wasm: WasmInput,
  stream: ReadableStream<Uint8Array>,
  sink: SequentialSink,
  options?: ConvertOptions & SpoolOptions & { storage?: SpoolStorage },
): Promise<ConversionReport>;

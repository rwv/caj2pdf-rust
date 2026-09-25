// SPDX-License-Identifier: MIT

import type { FileHandle } from "node:fs/promises";
import type { Writable } from "node:stream";
import type { ConvertOptions, ConversionReport, RangedSource, SequentialSink, Spooled, SpoolOptions, StreamInput, WasmInput } from "./io.mjs";

export * from "./io.mjs";

/** Compile the packaged `caj2pdf_wasm.wasm`, or the file at `url`. */
export declare function loadModule(url?: URL | string): Promise<WebAssembly.Module>;

/** Positioned BigInt reads over a caller-owned FileHandle; never closes it. */
export declare function fileHandleSource(handle: FileHandle): Promise<RangedSource>;

/** Awaits each write callback; never ends the stream. */
export declare function nodeWritableSink(writable: Writable): SequentialSink;

/** Copy a stream into a private temporary file bounded by `maxBytes`. */
export declare function spoolToTempFile(
  stream: StreamInput | NodeJS.ReadableStream,
  options: { maxBytes: bigint; signal?: AbortSignal; directory?: string },
): Promise<Spooled & { readonly path: string }>;

/** Spool to a temporary file, convert, and always remove the file. */
export declare function convertReadable(
  wasm: WasmInput,
  stream: StreamInput | NodeJS.ReadableStream,
  sink: SequentialSink,
  options?: ConvertOptions & SpoolOptions & { tempDirectory?: string },
): Promise<ConversionReport>;

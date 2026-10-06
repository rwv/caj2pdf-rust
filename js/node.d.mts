// SPDX-License-Identifier: MIT

import type { Writable } from "node:stream";
import type {
  ConversionReport,
  ConvertOptions,
  DocumentInfo,
  OperationOptions,
  SequentialSink,
  Spooled,
  SpoolOptions,
  StreamInput,
  WasmInput,
} from "./io.mjs";

export * from "./io.mjs";

/**
 * A document or font: a file path (string or `file:` URL), an open file
 * descriptor (for a `FileHandle`, its `fd`), or a Blob. The Worker reads a
 * path or descriptor directly; a Blob is read on the calling thread.
 */
export type NodeInput = string | URL | number | Blob;

/** Compile the packaged `caj2pdf_wasm.wasm`, or the file at `url`. */
export declare function loadModule(url?: URL | string): Promise<WebAssembly.Module>;

/** Convert PDF, CAJ, KDH or HN/C8 in a `node:worker_threads` Worker. */
export declare function convert(
  wasm: WasmInput,
  source: NodeInput,
  sink: SequentialSink,
  options?: ConvertOptions<NodeInput>,
): Promise<ConversionReport>;

/** Read pages and validated CAJ/HN-A bookmark counts without decoding images. */
export declare function inspect(
  wasm: WasmInput,
  source: NodeInput,
  options?: OperationOptions,
): Promise<DocumentInfo>;

/** Spool with `spool`, convert, and always dispose the spool. */
export declare function convertSpooled(
  spool: (stream: StreamInput, options: { maxBytes: bigint; signal?: AbortSignal }) => Promise<Spooled<NodeInput>>,
  wasm: WasmInput,
  stream: StreamInput,
  sink: SequentialSink,
  options?: ConvertOptions<NodeInput> & SpoolOptions,
): Promise<ConversionReport>;

/** Awaits each write callback; never ends the stream. */
export declare function nodeWritableSink(writable: Writable): SequentialSink;

/** Copy a stream into a private temporary file bounded by `maxBytes`; `source` is its path. */
export declare function spoolToTempFile(
  stream: StreamInput | NodeJS.ReadableStream,
  options: { maxBytes: bigint; signal?: AbortSignal; directory?: string },
): Promise<Spooled<string> & { readonly path: string }>;

/** Spool to a temporary file, convert, and always remove the file. */
export declare function convertReadable(
  wasm: WasmInput,
  stream: StreamInput | NodeJS.ReadableStream,
  sink: SequentialSink,
  options?: ConvertOptions<NodeInput> & SpoolOptions & { tempDirectory?: string },
): Promise<ConversionReport>;

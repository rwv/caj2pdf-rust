// SPDX-License-Identifier: MIT

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
 * A document or font: a Blob/File, read in the Worker with `FileReaderSync`,
 * or an OPFS `FileSystemFileHandle`, read through a synchronous access handle
 * the Worker opens and closes.
 */
export type BrowserInput = Blob | FileSystemFileHandle;

/** Fetch and compile the packaged `caj2pdf_wasm.wasm`, or the one at `url`. */
export declare function loadModule(url?: URL | string): Promise<WebAssembly.Module>;

/** Convert PDF, CAJ, KDH or HN/C8 in a module Worker. */
export declare function convert(
  wasm: WasmInput,
  source: BrowserInput,
  sink: SequentialSink,
  options?: ConvertOptions<BrowserInput>,
): Promise<ConversionReport>;

/** Read pages and validated CAJ/HN-A bookmark counts without decoding images. */
export declare function inspect(
  wasm: WasmInput,
  source: BrowserInput,
  options?: OperationOptions,
): Promise<DocumentInfo>;

/** Spool with `spool`, convert, and always dispose the spool. */
export declare function convertSpooled(
  spool: (stream: StreamInput, options: { maxBytes: bigint; signal?: AbortSignal }) => Promise<Spooled<BrowserInput>>,
  wasm: WasmInput,
  stream: StreamInput,
  sink: SequentialSink,
  options?: ConvertOptions<BrowserInput> & SpoolOptions,
): Promise<ConversionReport>;

/** The subset of `StorageManager` used for the OPFS spool. */
export interface SpoolStorage {
  getDirectory(): Promise<FileSystemDirectoryHandle>;
}

/**
 * Copy a stream into a uniquely named OPFS file bounded by `maxBytes`; `source`
 * is its file handle. Rejects with `RANDOM_ACCESS_REQUIRED` when OPFS writes
 * are unavailable.
 */
export declare function spoolToOpfs(
  stream: ReadableStream<Uint8Array>,
  options: { maxBytes: bigint; signal?: AbortSignal; storage?: SpoolStorage },
): Promise<Spooled<FileSystemFileHandle>>;

/** Spool to OPFS, convert, and always remove the OPFS file. */
export declare function convertReadableStream(
  wasm: WasmInput,
  stream: ReadableStream<Uint8Array>,
  sink: SequentialSink,
  options?: ConvertOptions<BrowserInput> & SpoolOptions & { storage?: SpoolStorage },
): Promise<ConversionReport>;

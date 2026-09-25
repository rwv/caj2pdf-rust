// SPDX-License-Identifier: MIT

/** Platform-neutral caj2pdf API shared by the Node.js and browser entry points. */

export declare const DEFAULT_IO_CHUNK: number;
export declare const MAX_IO_CHUNK: number;
export declare const MAX_U64: bigint;
export declare const MAX_ALLOCATION_LIMIT: bigint;
export declare const DEFAULT_LIMITS: Readonly<Required<Limits>>;

/** Input format names in WASM code order. Only `pdf`, `caj`, and `kdh` convert. */
export type Format = "auto" | "pdf" | "caj" | "kdh" | "hn" | "c8" | "teb" | "nh";
export type DetectedFormat = Exclude<Format, "auto">;
export declare const FORMATS: readonly Format[];

export type ErrorCode =
  | "UNKNOWN"
  | "UNSUPPORTED_FORMAT"
  | "INVALID_INPUT"
  | "TRUNCATED_INPUT"
  | "LIMIT_EXCEEDED"
  | "IO"
  | "CANCELLED"
  | "RANDOM_ACCESS_REQUIRED"
  | "MALFORMED_PDF"
  | "ENCRYPTED_PDF"
  | "UNSUPPORTED_PDF_FEATURE"
  | "AMBIGUOUS_PDF_REPAIR"
  | "PDF_LIMIT_EXCEEDED"
  | "MALFORMED_CAJ"
  | "CAJ_LIMIT_EXCEEDED"
  | "MALFORMED_KDH";

export declare class Caj2PdfError extends Error {
  constructor(message: string, code: ErrorCode);
  readonly code: ErrorCode;
}

/** `format` is the recognized format, or `null` for an unknown signature. */
export declare class UnsupportedFormatError extends Caj2PdfError {
  constructor(format: DetectedFormat | null);
  readonly code: "UNSUPPORTED_FORMAT";
  readonly format: DetectedFormat | null;
}

export declare class TruncatedInputError extends Caj2PdfError {
  constructor(message: string);
  readonly code: "TRUNCATED_INPUT";
}

/** A sized random-access input. Each request is at most `MAX_IO_CHUNK` bytes. */
export interface RangedSource {
  readonly size: bigint;
  /** Resolve with at most `length` bytes at `offset`; a short read is allowed. */
  readAt(offset: bigint, length: number, signal?: AbortSignal): Promise<Uint8Array>;
}

/** An ordered output. `bytes` is valid only until the returned Promise settles. */
export interface SequentialSink {
  /** Resolve with the number of leading bytes accepted (0..bytes.byteLength). */
  writeChunk(bytes: Uint8Array, signal?: AbortSignal): Promise<number>;
  flush(signal?: AbortSignal): Promise<void>;
}

/** Resource limits enforced in Rust. Byte limits accept BigInt or safe integers. */
export interface Limits {
  maxInputBytes?: bigint | number;
  maxOutputBytes?: bigint | number;
  /** At most `MAX_ALLOCATION_LIMIT` (256 MiB) and at least `chunkSize`. */
  maxAllocationBytes?: bigint | number;
  maxPages?: number;
  maxBookmarks?: number;
}

/** A compiled module (fresh instance per call), an instance, or its exports. */
export type WasmInput = WebAssembly.Module | WebAssembly.Instance | WebAssembly.Exports;

export interface OperationOptions {
  /** Defaults to `"auto"`: detect from the leading signature. */
  format?: Format;
  limits?: Limits;
  /** Bytes per read or write request, 1..MAX_IO_CHUNK. Default 256 KiB. */
  chunkSize?: number;
  signal?: AbortSignal;
}

export interface ConvertOptions extends OperationOptions {
  /** Write CAJ outline entries as PDF bookmarks. Default `true`. */
  includeBookmarks?: boolean;
}

export interface ConversionReport {
  format: DetectedFormat | null;
  inputBytesRead: bigint;
  outputBytesWritten: bigint;
  pagesConverted: number;
  bookmarksWritten: number;
}

export interface DocumentInfo {
  format: DetectedFormat;
  pageCount: number;
  /** Counted for CAJ; `null` when not counted (PDF, KDH). */
  bookmarkCount: number | null;
  inputBytesRead: bigint;
}

export interface SpoolOptions {
  /** Default: `limits.maxInputBytes`, else 8 GiB. */
  maxSpoolBytes?: bigint | number;
}

export interface Spooled {
  readonly source: RangedSource;
  /** Remove the temporary storage. */
  dispose(): Promise<void>;
}

export type StreamInput = ReadableStream<Uint8Array> | AsyncIterable<Uint8Array>;

/** Convert a PDF, CAJ, or KDH source to PDF with bounded, awaited I/O. */
export declare function convert(
  wasm: WasmInput,
  source: RangedSource,
  sink: SequentialSink,
  options?: ConvertOptions,
): Promise<ConversionReport>;

/** Read the format and page count without output. */
export declare function inspect(
  wasm: WasmInput,
  source: RangedSource,
  options?: OperationOptions,
): Promise<DocumentInfo>;

/** Bounded byte-range copy through the WASM bridge; a diagnostic, not conversion. */
export declare function copyRange(
  wasm: WasmInput,
  source: RangedSource,
  sink: SequentialSink,
  options?: { offset?: bigint; length?: bigint; chunkSize?: number; signal?: AbortSignal },
): Promise<ConversionReport>;

/** A source over a Blob or File using bounded `slice()` reads. */
export declare function blobSource(blob: Blob): RangedSource;

/** A sink over a caller-owned writer; it is never closed by this package. */
export declare function webWritableSink(writer: WritableStreamDefaultWriter<Uint8Array>): SequentialSink;

/** Spool with `spool`, convert, and always dispose the spool. */
export declare function convertSpooled(
  spool: (stream: StreamInput, options: { maxBytes: bigint; signal?: AbortSignal }) => Promise<Spooled>,
  wasm: WasmInput,
  stream: StreamInput,
  sink: SequentialSink,
  options?: ConvertOptions & SpoolOptions,
): Promise<ConversionReport>;

/** Await `consume` for each chunk; reject after more than `maxBytes`. */
export declare function pumpChunks(
  stream: StreamInput,
  consume: (chunk: Uint8Array) => Promise<unknown>,
  options: { maxBytes: bigint; signal?: AbortSignal },
): Promise<bigint>;

export declare function abortable<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T>;
export declare function checkAbort(signal?: AbortSignal): void;
export declare function checkRange(size: bigint, offset: bigint, length: bigint): void;
export declare function requireU64(value: unknown, name: string): bigint;
export declare function requireChunkLength(length: number, options?: { allowZero?: boolean }): number;
export declare function requireSinkChunk(bytes: unknown): void;

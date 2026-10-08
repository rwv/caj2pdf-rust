// SPDX-License-Identifier: MIT

/** Platform-neutral caj2pdf API shared by the Node.js and browser entry points. */

export declare const DEFAULT_IO_CHUNK: number;
export declare const MAX_IO_CHUNK: number;
export declare const MAX_U64: bigint;
export declare const MAX_ALLOCATION_LIMIT: bigint;
export declare const DEFAULT_LIMITS: Readonly<Required<Limits>>;

/** Input format names in WASM code order. Native C8/HN-B text needs caller fonts (`hnc8.fonts`). */
export type Format = "auto" | "pdf" | "caj" | "kdh" | "hn" | "c8" | "teb" | "nh" | "caa";
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
  | "MALFORMED_KDH"
  | "HNC8";

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

/** An ordered output on the calling thread. Each `bytes` is a fresh copy the sink may keep. */
export interface SequentialSink {
  /** Resolve with the number of leading bytes accepted (1..bytes.byteLength); the rest is offered again. */
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

/** The compiled module from `loadModule()`; each operation instantiates it in a fresh Worker. */
export type WasmInput = WebAssembly.Module;

export interface OperationOptions {
  /** Defaults to `"auto"`: detect from the leading signature. */
  format?: Format;
  limits?: Limits;
  /** Bytes per read or write request, 1..MAX_IO_CHUNK. Default 256 KiB. */
  chunkSize?: number;
  /**
   * Stops the operation. With cross-origin isolation (SharedArrayBuffer) the
   * Worker stops at its next checkpoint and closes its inputs before the
   * promise rejects; otherwise the Worker is terminated at once.
   */
  signal?: AbortSignal;
  /** Called with the fraction (0..1) of the input read so far, never decreasing. */
  progress?: (fraction: number) => void;
}

export interface ConvertOptions<Input> extends OperationOptions {
  /** Experimental native C8 fonts, given as inputs of the same kinds as the document. */
  hnc8?: Hnc8Options<Input>;
  /** Write supported CAJ/HN-A outlines. Default `true`; C8/HN-B require `false`. */
  includeBookmarks?: boolean;
  /** Explicitly replace damaged CAJ pages with blanks; inspect omittedPages. */
  allowDamaged?: boolean;
}

export interface ConversionReport {
  format: DetectedFormat | null;
  inputBytesRead: bigint;
  outputBytesWritten: bigint;
  pagesConverted: number;
  /** Private-use glyphs rendered with visual substitutes; codes retained in PDF ActualText. */
  substitutedGlyphs: bigint;
  bookmarksWritten: number;
  omittedPages: Array<{ pageIndex: number; offset: bigint }>;
  /** HN-A outline entries skipped or clamped instead of failing; zero otherwise. */
  outlineWarnings: number;
  /** Requested C8/HN-B bookmarks were not written because their layout is unverified. */
  outlineOmitted: boolean;
}

export interface DocumentInfo {
  format: DetectedFormat;
  /** `null` for a CAA target descriptor, which contains no document page count. */
  pageCount: number | null;
  /** Validated for CAJ/HN-A; `null` when unknown or not counted (C8, HN-B, PDF, KDH). */
  bookmarkCount: number | null;
  /** HN-A outline entries skipped or clamped instead of failing; zero otherwise. */
  outlineWarnings: number;
  /** The C8 application-info package; `null` when absent, defective or not C8. */
  applicationInfo: ApplicationInfo | null;
  inputBytesRead: bigint;
}

/** Values from a C8 application-info package. They are not verified. */
export interface ApplicationInfo {
  /** A CNKI identifier; not verified as a registered DOI. */
  doi: string | null;
  /** Never followed. */
  url: string | null;
  /** Annotation entries in the package. */
  noteCount: number;
}

export interface SpoolOptions {
  /** Default: `limits.maxInputBytes`, else 8 GiB. */
  maxSpoolBytes?: bigint | number;
}

export interface Spooled<Source> {
  readonly source: Source;
  /** Remove the temporary storage. */
  dispose(): Promise<void>;
}

export type StreamInput = ReadableStream<Uint8Array> | AsyncIterable<Uint8Array>;

/** A sink over a caller-owned writer; it is never closed by this package. */
export declare function webWritableSink(writer: WritableStreamDefaultWriter<Uint8Array>): SequentialSink;

/** Await `consume` for each chunk; reject after more than `maxBytes`. */
export declare function pumpChunks(
  stream: StreamInput,
  consume: (chunk: Uint8Array) => Promise<unknown>,
  options: { maxBytes: bigint; signal?: AbortSignal },
): Promise<bigint>;

export declare function abortable<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T>;
export declare function checkAbort(signal?: AbortSignal): void;
export declare function requireU64(value: unknown, name: string): bigint;
export declare function requireChunkLength(length: number, options?: { allowZero?: boolean }): number;
export declare function requireSinkChunk(bytes: unknown): void;

/** An OpenType font (TrueType or CFF outlines), or one face of a font
 * collection (`.ttc`). */
export type C8Font<Input> = Input | { source: Input; face?: number };

/** Explicit C8 resources. Reuse the same input across roles to embed once.
 * Inputs remain caller-owned and must stay unchanged until conversion settles.
 * Only `cjk` and `latin` are required. An absent optional role, or a role font
 * that does not map a character, falls back to `cjk` for CJK-coded characters
 * and to `latin` otherwise; a glyph missing from that font still fails. */
export interface C8Fonts<Input> {
  cjk: C8Font<Input>;
  latin: C8Font<Input>;
  alternateLatin?: C8Font<Input>;
  /** Semantic symbols/spaces required by the admitted HN-B mode-0 records. */
  symbols?: C8Font<Input>;
  /** Optional explicit font selected by HN-B/C8 state 801d/3. */
  latinState3?: C8Font<Input>;
  /** Distinct caller-supplied resources for verified C8 Latin states. */
  latinState28?: C8Font<Input>;
  latinState31?: C8Font<Input>;
  /** Nonsemantic decoration alias; must be one BMP Unicode scalar. */
  decoration?: { source: C8Font<Input>; character: string };
}

export interface Hnc8Options<Input> {
  /** Enables the admitted native C8 profile; currently requires includeBookmarks: false. */
  fonts?: C8Fonts<Input>;
}

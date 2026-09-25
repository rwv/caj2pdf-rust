// SPDX-License-Identifier: MIT

/**
 * Platform-neutral caj2pdf conversion API over the raw WASM ABI.
 *
 * A source is `{ size: bigint, readAt(offset, length, signal) }` and a sink
 * is `{ writeChunk(bytes, signal), flush(signal) }`. Only one bounded chunk
 * crosses the JavaScript/WASM boundary per awaited request.
 */
export const DEFAULT_IO_CHUNK = 256 * 1024;
export const MAX_IO_CHUNK = 1024 * 1024;
export const MAX_U64 = (1n << 64n) - 1n;
const MAX_U32 = 0xffff_ffff;
/** Largest `maxAllocationBytes` accepted inside 32-bit WASM memory. */
export const MAX_ALLOCATION_LIMIT = 256n * 1024n * 1024n;
/** Core defaults; `maxInputBytes` also bounds temporary spools by default. */
export const DEFAULT_LIMITS = Object.freeze({
  maxInputBytes: 8n * 1024n ** 3n,
  maxOutputBytes: 16n * 1024n ** 3n,
  maxAllocationBytes: 64n * 1024n * 1024n,
  maxPages: 100_000,
  maxBookmarks: 100_000,
});

/**
 * Format names in WASM code order. Only `pdf`, `caj`, and `kdh` convert;
 * `hn`, `c8`, `teb`, and `nh` are rejected with `UnsupportedFormatError`.
 */
export const FORMATS = Object.freeze(["auto", "pdf", "caj", "kdh", "hn", "c8", "teb", "nh"]);

const ERROR_CODES = [
  "UNKNOWN",
  "UNSUPPORTED_FORMAT",
  "INVALID_INPUT",
  "TRUNCATED_INPUT",
  "LIMIT_EXCEEDED",
  "IO",
  "CANCELLED",
  "RANDOM_ACCESS_REQUIRED",
  "MALFORMED_PDF",
  "ENCRYPTED_PDF",
  "UNSUPPORTED_PDF_FEATURE",
  "AMBIGUOUS_PDF_REPAIR",
  "PDF_LIMIT_EXCEEDED",
  "MALFORMED_CAJ",
  "CAJ_LIMIT_EXCEEDED",
  "MALFORMED_KDH",
];

/** A typed conversion failure. `code` is one of the stable error codes. */
export class Caj2PdfError extends Error {
  constructor(message, code) {
    super(message);
    this.name = "Caj2PdfError";
    this.code = code;
  }
}

/** A recognized-but-unconverted (`format` set) or unrecognized input. */
export class UnsupportedFormatError extends Caj2PdfError {
  constructor(format) {
    super(
      format == null
        ? "input signature is not a recognized PDF, CAJ, KDH, HN, C8, or TEB format"
        : `${format.toUpperCase()} input is recognized, but converting it is not supported yet`,
      "UNSUPPORTED_FORMAT",
    );
    this.name = "UnsupportedFormatError";
    this.format = format;
  }
}

export class TruncatedInputError extends Caj2PdfError {
  constructor(message) {
    super(message, "TRUNCATED_INPUT");
    this.name = "TruncatedInputError";
  }
}

export function requireU64(value, name) {
  if (typeof value !== "bigint" || value < 0n || value > MAX_U64) {
    throw new RangeError(`${name} must be a nonnegative unsigned 64-bit BigInt`);
  }
  return value;
}

function toU64(value, name) {
  return requireU64(Number.isSafeInteger(value) ? BigInt(value) : value, name);
}

function toU32(value, name) {
  if (!Number.isSafeInteger(value) || value < 0 || value > MAX_U32) {
    throw new RangeError(`${name} must be an integer 0..${MAX_U32}`);
  }
  return value;
}

export function requireChunkLength(length, { allowZero = false } = {}) {
  if (!Number.isSafeInteger(length) || length < (allowZero ? 0 : 1) || length > MAX_IO_CHUNK) {
    throw new RangeError(`chunk length must be ${allowZero ? "0" : "1"}..${MAX_IO_CHUNK}`);
  }
  return length;
}

function abortReason(signal) {
  return signal.reason ?? new DOMException("Operation cancelled", "AbortError");
}

export function checkAbort(signal) {
  if (signal?.aborted) {
    throw abortReason(signal);
  }
}

/**
 * Settle with `promise`, or reject as soon as `signal` aborts. An abandoned
 * operation may still finish, so it must only touch buffers it owns.
 */
export function abortable(promise, signal) {
  if (signal == null) return promise;
  checkAbort(signal);
  let onAbort;
  const aborted = new Promise((_, reject) => {
    onAbort = () => reject(abortReason(signal));
    signal.addEventListener("abort", onAbort, { once: true });
  });
  return Promise.race([promise, aborted]).finally(() => {
    signal.removeEventListener("abort", onAbort);
  });
}

export function checkRange(size, offset, length) {
  requireU64(size, "size");
  requireU64(offset, "offset");
  requireU64(length, "length");
  if (offset > size || length > size - offset) {
    throw new RangeError("requested range exceeds source size");
  }
}

/** A Blob/File source. Every read awaits only a slice of at most 1 MiB. */
export function blobSource(blob) {
  if (
    blob == null ||
    !Number.isSafeInteger(blob.size) ||
    blob.size < 0 ||
    typeof blob.slice !== "function"
  ) {
    throw new TypeError("a Blob with a safe integer size and slice() is required");
  }
  const size = BigInt(blob.size);
  return Object.freeze({
    size,
    async readAt(offset, length, signal) {
      requireChunkLength(length, { allowZero: true });
      checkRange(size, offset, BigInt(length));
      // The safe-size check above makes both Number conversions exact.
      const part = blob.slice(Number(offset), Number(offset + BigInt(length)));
      const arrayBuffer = await abortable(part.arrayBuffer(), signal);
      if (!(arrayBuffer instanceof ArrayBuffer) || arrayBuffer.byteLength !== length) {
        throw new TruncatedInputError("Blob range read returned a truncated or invalid chunk");
      }
      return new Uint8Array(arrayBuffer);
    },
  });
}

export function requireSinkChunk(bytes) {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError("writeChunk requires Uint8Array");
  }
  requireChunkLength(bytes.byteLength, { allowZero: true });
}

/** Adapt a caller-owned WritableStreamDefaultWriter without closing it. */
export function webWritableSink(writer) {
  if (writer == null || typeof writer.write !== "function") {
    throw new TypeError("a WritableStream writer is required");
  }
  return Object.freeze({
    async writeChunk(bytes, signal) {
      requireSinkChunk(bytes);
      // The copy keeps an abandoned or queued write away from reused WASM
      // staging memory.
      await abortable(writer.write(bytes.slice()), signal);
      return bytes.byteLength;
    },
    async flush(signal) {
      if (writer.ready != null) {
        await abortable(writer.ready, signal);
      }
    },
  });
}

/**
 * Feed a Web `ReadableStream`, Node `Readable`, or async iterable of
 * Uint8Array chunks to `consume`, awaiting each call. Rejects once more than
 * `maxBytes` arrive and cancels the stream on any failure.
 */
export async function pumpChunks(stream, consume, { maxBytes, signal } = {}) {
  requireU64(maxBytes, "maxBytes");
  let next;
  let stop;
  if (typeof stream?.getReader === "function") {
    const reader = stream.getReader();
    next = () => reader.read();
    stop = () => reader.cancel();
  } else if (typeof stream?.[Symbol.asyncIterator] === "function") {
    const iterator = stream[Symbol.asyncIterator]();
    next = () => iterator.next();
    stop = () => iterator.return?.();
  } else {
    throw new TypeError("a ReadableStream, Node Readable, or async iterable is required");
  }
  let total = 0n;
  try {
    for (;;) {
      const { done, value } = await abortable(next(), signal);
      if (done) return total;
      if (!(value instanceof Uint8Array)) {
        throw new TypeError("stream chunks must be Uint8Array or Buffer");
      }
      total += BigInt(value.byteLength);
      if (total > maxBytes) {
        throw new Caj2PdfError(`temporary spool limit of ${maxBytes} bytes exceeded`, "LIMIT_EXCEEDED");
      }
      await abortable(consume(value), signal);
    }
  } catch (error) {
    // Not awaited: a stalled producer must not delay cleanup.
    Promise.resolve().then(stop).catch(() => {});
    throw error;
  }
}

/**
 * Spool a forward-only stream with a platform `spool` function, convert the
 * spooled copy, and always dispose the temporary storage afterwards.
 */
export async function convertSpooled(spool, wasm, stream, sink, options = {}) {
  // Reject a bad configuration before spooling up to gigabytes of input.
  checkWasm(wasm);
  requireSink(sink);
  operationConfig(options);
  const maxBytes = toU64(
    options.maxSpoolBytes ?? options.limits?.maxInputBytes ?? DEFAULT_LIMITS.maxInputBytes,
    "maxSpoolBytes",
  );
  const spooled = await spool(stream, { maxBytes, signal: options.signal });
  let result;
  try {
    result = await convert(wasm, spooled.source, sink, options);
  } catch (error) {
    // A cleanup failure must not hide the conversion failure.
    await Promise.resolve().then(() => spooled.dispose()).catch(() => {});
    throw error;
  }
  await spooled.dispose();
  return result;
}

function resolveLimits(limits, chunkSize) {
  if (limits == null || typeof limits !== "object") {
    throw new TypeError("limits must be an object");
  }
  const merged = { ...DEFAULT_LIMITS };
  for (const [name, value] of Object.entries(limits)) {
    // An explicit `undefined` keeps the default, as for other options.
    if (value !== undefined) merged[name] = value;
  }
  const resolved = {
    maxInputBytes: toU64(merged.maxInputBytes, "limits.maxInputBytes"),
    maxOutputBytes: toU64(merged.maxOutputBytes, "limits.maxOutputBytes"),
    maxAllocationBytes: toU64(merged.maxAllocationBytes, "limits.maxAllocationBytes"),
    maxPages: toU32(merged.maxPages, "limits.maxPages"),
    maxBookmarks: toU32(merged.maxBookmarks, "limits.maxBookmarks"),
  };
  if (
    resolved.maxAllocationBytes > MAX_ALLOCATION_LIMIT ||
    resolved.maxAllocationBytes < BigInt(chunkSize)
  ) {
    throw new RangeError(`limits.maxAllocationBytes must be chunkSize..${MAX_ALLOCATION_LIMIT}`);
  }
  return resolved;
}

/**
 * Accept a `WebAssembly.Module` (a fresh instance per call), an `Instance`,
 * or its exports. An instance runs one operation at a time.
 */
function checkWasm(wasm) {
  if (wasm instanceof WebAssembly.Module) return wasm;
  const exports = wasm?.exports ?? wasm;
  if (!(exports?.memory instanceof WebAssembly.Memory) || typeof exports.caj2pdf_io_poll !== "function") {
    throw new TypeError("a caj2pdf WebAssembly.Module, Instance, or its exports is required");
  }
  return exports;
}

async function resolveExports(wasm) {
  const checked = checkWasm(wasm);
  return checked instanceof WebAssembly.Module
    ? (await WebAssembly.instantiate(checked, {})).exports
    : checked;
}

function requireSource(source) {
  if (source == null || typeof source.readAt !== "function") {
    throw new TypeError("source must expose size and readAt()");
  }
  requireU64(source.size, "source.size");
}

function requireSink(sink) {
  if (sink == null || typeof sink.writeChunk !== "function" || typeof sink.flush !== "function") {
    throw new TypeError("sink must expose writeChunk() and flush()");
  }
}

function formatCode(format) {
  const code = FORMATS.indexOf(format);
  if (code < 0) {
    throw new RangeError(`format must be one of: ${FORMATS.join(", ")}`);
  }
  return code;
}

function formatName(exports) {
  const code = exports.caj2pdf_io_format();
  return code === 0 ? null : FORMATS[code];
}

function engineError(exports) {
  const code = ERROR_CODES[exports.caj2pdf_io_error_kind()] ?? "UNKNOWN";
  if (code === "UNSUPPORTED_FORMAT") {
    return new UnsupportedFormatError(formatName(exports));
  }
  const bytes = new Uint8Array(
    exports.memory.buffer,
    exports.caj2pdf_io_message_ptr(),
    exports.caj2pdf_io_message_len(),
  );
  const message = new TextDecoder().decode(bytes) || `conversion failed: ${code}`;
  return code === "TRUNCATED_INPUT" ? new TruncatedInputError(message) : new Caj2PdfError(message, code);
}

function report(exports) {
  return {
    format: formatName(exports),
    inputBytesRead: exports.caj2pdf_io_input_bytes_read(),
    outputBytesWritten: exports.caj2pdf_io_output_bytes_written(),
    pagesConverted: exports.caj2pdf_io_pages_converted(),
    bookmarksWritten: exports.caj2pdf_io_bookmarks_written(),
  };
}

function inspection(exports) {
  const bookmarks = exports.caj2pdf_info_bookmark_count();
  return {
    format: formatName(exports),
    pageCount: exports.caj2pdf_info_page_count(),
    bookmarkCount: bookmarks < 0n ? null : Number(bookmarks),
    inputBytesRead: exports.caj2pdf_io_input_bytes_read(),
  };
}

/**
 * Drive one WASM operation to completion. Every read, write, and flush is
 * awaited before Rust resumes; the engine is always cancelled and reset.
 */
async function drive(exports, start, source, sink, chunkSize, signal, finish = report) {
  const started = start();
  if (started === 1) {
    throw new Error("WASM instance already has an active operation");
  }
  if (started !== 0) {
    throw new RangeError("invalid WASM operation configuration");
  }
  try {
    for (;;) {
      checkAbort(signal);
      const status = exports.caj2pdf_io_poll();
      if (status === 1) {
        const offset = exports.caj2pdf_io_request_offset();
        const length = exports.caj2pdf_io_request_length();
        if (length > chunkSize) {
          throw new Error("WASM requested more than the configured chunk size");
        }
        checkRange(source.size, offset, BigInt(length));
        const bytes = await source.readAt(offset, length, signal);
        checkAbort(signal);
        if (!(bytes instanceof Uint8Array) || bytes.byteLength > length) {
          throw new TypeError("source must return a bounded Uint8Array");
        }
        new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), bytes.byteLength).set(bytes);
        if (exports.caj2pdf_io_complete_read(bytes.byteLength) !== 1) {
          throw new Error("WASM rejected a read response");
        }
      } else if (status === 2 && sink != null) {
        const length = exports.caj2pdf_io_request_length();
        if (length > chunkSize) {
          throw new Error("WASM requested an oversized write");
        }
        // Stable until writeChunk settles; a sink that retains bytes longer
        // must copy them first (the supplied sinks do).
        const view = new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), length);
        const accepted = await sink.writeChunk(view, signal);
        checkAbort(signal);
        if (!Number.isSafeInteger(accepted) || accepted < 0 || accepted > length) {
          throw new RangeError("sink returned an invalid byte count");
        }
        if (exports.caj2pdf_io_complete_write(accepted) !== 1) {
          throw new Error("WASM rejected a write response");
        }
      } else if (status === 3 && sink != null) {
        await sink.flush(signal);
        checkAbort(signal);
        if (exports.caj2pdf_io_complete_flush() !== 1) {
          throw new Error("WASM rejected a flush response");
        }
      } else if (status === 4) {
        return finish(exports);
      } else if (status === 5) {
        throw engineError(exports);
      } else {
        throw new Error(`unexpected WASM I/O status: ${status}`);
      }
    }
  } finally {
    exports.caj2pdf_io_cancel();
    exports.caj2pdf_io_reset();
  }
}

const OPERATION_CONVERT = 1;
const OPERATION_INSPECT = 2;

/** Validate conversion and inspection options without side effects. */
function operationConfig(options) {
  const {
    format = "auto",
    limits = {},
    chunkSize = DEFAULT_IO_CHUNK,
    signal,
    includeBookmarks = true,
  } = options ?? {};
  requireChunkLength(chunkSize);
  return {
    code: formatCode(format),
    resolved: resolveLimits(limits, chunkSize),
    chunkSize,
    signal,
    includeBookmarks,
  };
}

async function run(operation, wasm, source, sink, options) {
  requireSource(source);
  if (operation === OPERATION_CONVERT) requireSink(sink);
  const { code, resolved, chunkSize, signal, includeBookmarks } = operationConfig(options);
  checkAbort(signal);
  const exports = await resolveExports(wasm);
  const start = () => exports.caj2pdf_start(
    operation,
    source.size,
    chunkSize,
    code,
    includeBookmarks ? 1 : 0,
    resolved.maxInputBytes,
    resolved.maxOutputBytes,
    resolved.maxAllocationBytes,
    resolved.maxPages,
    resolved.maxBookmarks,
  );
  return drive(exports, start, source, sink, chunkSize, signal, operation === OPERATION_INSPECT ? inspection : report);
}

/**
 * Convert a PDF, CAJ, or KDH source to PDF through bounded, awaited I/O.
 * The format is detected from the leading signature unless `format` is set.
 */
export function convert(wasm, source, sink, options = {}) {
  return run(OPERATION_CONVERT, wasm, source, sink, options);
}

/** Read the format, page count, and (for CAJ) bookmark count. No output. */
export function inspect(wasm, source, options = {}) {
  return run(OPERATION_INSPECT, wasm, source, null, options);
}

/**
 * Copy a byte range through the same bounded bridge. This diagnostic checks
 * a source/sink pair; it is not PDF conversion.
 */
export async function copyRange(wasm, source, sink, { offset = 0n, length, chunkSize = DEFAULT_IO_CHUNK, signal } = {}) {
  requireSource(source);
  requireSink(sink);
  requireU64(offset, "offset");
  length ??= source.size - offset;
  checkRange(source.size, offset, length);
  requireChunkLength(chunkSize);
  checkAbort(signal);
  const exports = await resolveExports(wasm);
  return drive(
    exports,
    () => exports.caj2pdf_io_start(source.size, offset, length, chunkSize),
    source,
    sink,
    chunkSize,
    signal,
  );
}

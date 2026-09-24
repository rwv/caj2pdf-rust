// SPDX-License-Identifier: MIT

/** Platform-neutral, bounded JavaScript I/O contract and WASM proof driver. */
export const DEFAULT_IO_CHUNK = 256 * 1024;
export const MAX_IO_CHUNK = 1024 * 1024;
export const MAX_U64 = (1n << 64n) - 1n;

export class TruncatedInputError extends Error {
  constructor(message) {
    super(message);
    this.name = "TruncatedInputError";
    this.code = "TRUNCATED_INPUT";
  }
}

export function requireU64(value, name) {
  if (typeof value !== "bigint" || value < 0n || value > MAX_U64) {
    throw new RangeError(`${name} must be a nonnegative unsigned 64-bit BigInt`);
  }
  return value;
}

export function requireChunkLength(length, { allowZero = false } = {}) {
  if (!Number.isSafeInteger(length) || length < (allowZero ? 0 : 1) || length > MAX_IO_CHUNK) {
    throw new RangeError(`chunk length must be ${allowZero ? "0" : "1"}..${MAX_IO_CHUNK}`);
  }
  return length;
}

export function checkAbort(signal) {
  if (signal?.aborted) {
    throw new DOMException("Operation cancelled", "AbortError");
  }
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
      checkAbort(signal);
      // The safe-size check above makes both Number conversions exact.
      const part = blob.slice(Number(offset), Number(offset + BigInt(length)));
      const arrayBuffer = await part.arrayBuffer();
      checkAbort(signal);
      if (!(arrayBuffer instanceof ArrayBuffer) || arrayBuffer.byteLength !== length) {
        throw new TruncatedInputError("Blob range read returned a truncated or invalid chunk");
      }
      return new Uint8Array(arrayBuffer);
    },
  });
}

/** Adapt a caller-owned WritableStreamDefaultWriter without closing it. */
export function webWritableSink(writer) {
  if (writer == null || typeof writer.write !== "function") {
    throw new TypeError("a WritableStream writer is required");
  }
  return Object.freeze({
    async writeChunk(bytes, signal) {
      if (!(bytes instanceof Uint8Array)) {
        throw new TypeError("writeChunk requires Uint8Array");
      }
      requireChunkLength(bytes.byteLength, { allowZero: true });
      checkAbort(signal);
      // The writer may retain its input after the WASM staging area is reused.
      await writer.write(bytes.slice());
      checkAbort(signal);
      return bytes.byteLength;
    },
    async flush(signal) {
      checkAbort(signal);
      if (writer.ready != null) {
        await writer.ready;
      }
      checkAbort(signal);
    },
  });
}

const ERROR_NAMES = [
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
];

function coreError(kind) {
  const code = ERROR_NAMES[kind] ?? "UNKNOWN";
  const error = new Error(`Rust I/O proof failed: ${code}`);
  error.code = code;
  return error;
}

/**
 * Drive the core Rust `copy_range` future through bounded JS range reads and
 * awaited writes. This copies bytes; it does not convert a document to PDF.
 * The raw WASM instance is single-operation. Use one instance per concurrent
 * operation until the handle-based production binding is implemented.
 */
export async function copyRangeProof(
  wasm,
  source,
  sink,
  { offset = 0n, length, chunkSize = DEFAULT_IO_CHUNK, signal } = {},
) {
  const exports = wasm?.exports ?? wasm;
  if (exports?.memory == null || typeof exports.caj2pdf_io_start !== "function") {
    throw new TypeError("a caj2pdf WASM instance or its exports is required");
  }
  if (source == null || typeof source.readAt !== "function") {
    throw new TypeError("source must expose size and readAt()");
  }
  if (sink == null || typeof sink.writeChunk !== "function" || typeof sink.flush !== "function") {
    throw new TypeError("sink must expose writeChunk() and flush()");
  }
  requireU64(source.size, "source.size");
  requireU64(offset, "offset");
  length ??= source.size - offset;
  requireU64(length, "length");
  checkRange(source.size, offset, length);
  requireChunkLength(chunkSize);
  checkAbort(signal);

  const started = exports.caj2pdf_io_start(source.size, offset, length, chunkSize);
  if (started === 1) {
    throw new Error("WASM instance already has an active I/O proof");
  }
  if (started !== 0) {
    throw new RangeError("invalid WASM I/O proof configuration");
  }

  try {
    for (;;) {
      checkAbort(signal);
      const status = exports.caj2pdf_io_poll();
      if (status === 1) {
        const requestedOffset = exports.caj2pdf_io_request_offset();
        const requestedLength = exports.caj2pdf_io_request_length();
        requireChunkLength(requestedLength);
        if (requestedLength > chunkSize) {
          throw new Error("WASM requested more than the configured chunk size");
        }
        checkRange(source.size, requestedOffset, BigInt(requestedLength));
        const bytes = await source.readAt(requestedOffset, requestedLength, signal);
        checkAbort(signal);
        if (!(bytes instanceof Uint8Array) || bytes.byteLength > requestedLength) {
          throw new TypeError("source must return a bounded Uint8Array");
        }
        new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), bytes.byteLength).set(bytes);
        if (exports.caj2pdf_io_complete_read(bytes.byteLength) !== 1) {
          throw new Error("WASM rejected a read response");
        }
      } else if (status === 2) {
        const requestedLength = exports.caj2pdf_io_request_length();
        requireChunkLength(requestedLength);
        if (requestedLength > chunkSize) {
          throw new Error("WASM requested an oversized write");
        }
        const view = new Uint8Array(exports.memory.buffer, exports.caj2pdf_io_buffer_ptr(), requestedLength);
        // Stable until this Promise resolves; retaining beyond that requires
        // the sink to copy before resolving (the supplied adapters do so).
        const accepted = await sink.writeChunk(view, signal);
        checkAbort(signal);
        if (!Number.isSafeInteger(accepted) || accepted < 0 || accepted > requestedLength) {
          throw new RangeError("sink returned an invalid byte count");
        }
        if (exports.caj2pdf_io_complete_write(accepted) !== 1) {
          throw new Error("WASM rejected a write response");
        }
      } else if (status === 3) {
        await sink.flush(signal);
        checkAbort(signal);
        if (exports.caj2pdf_io_complete_flush() !== 1) {
          throw new Error("WASM rejected a flush response");
        }
      } else if (status === 4) {
        return {
          inputBytesRead: exports.caj2pdf_io_input_bytes_read(),
          outputBytesWritten: exports.caj2pdf_io_output_bytes_written(),
          pagesConverted: 0,
          bookmarksWritten: 0,
        };
      } else if (status === 5) {
        throw coreError(exports.caj2pdf_io_error_kind());
      } else {
        throw new Error(`unexpected WASM I/O status: ${status}`);
      }
    }
  } finally {
    exports.caj2pdf_io_cancel();
    exports.caj2pdf_io_reset();
  }
}

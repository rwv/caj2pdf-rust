#!/usr/bin/env node
// SPDX-License-Identifier: MIT

// Usage: CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples node js/scripts/corpus.mjs
//          [--matrix PATH] [--wasm PATH] [--qpdf PATH]
//
// Optional external-corpus run of the JavaScript API over every entry of
// tests/conformance/matrix.json. Each entry's expectation comes from the
// matrix's `expected_outcome` (classified as scripts/conformance.py does) and
// the API's format contract (HN, C8, and TEB are rejected); see
// `expectationFor`. It never fetches the corpus. With CAJ2PDF_CORPUS_DIR
// unset or empty the report is NOT_RUN with zero counts; a requested but
// missing or changed corpus FAILs.
// Exit codes: 0 for PASS or NOT_RUN, 1 for FAIL, 2 for a setup error, and
// 130 when interrupted by SIGINT or SIGTERM (after temporary-file cleanup).
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { lstat, mkdtemp, open, readFile, realpath, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { finished } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { Caj2PdfError, convert, fileHandleSource, loadModule, nodeWritableSink, UnsupportedFormatError } from "../node.mjs";

export const DEFAULT_MATRIX = fileURLToPath(new URL("../../tests/conformance/matrix.json", import.meta.url));
export const DEFAULT_WASM = fileURLToPath(
  new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url),
);
/** Detected formats the JavaScript API rejects with `UnsupportedFormatError` (see io.mjs `FORMATS`). */
const API_UNSUPPORTED = new Set(["HN", "C8", "TEB"]);
const API_CONVERTED = new Set(["CAJ", "KDH", "PDF"]);
/** `expected_outcome` classes, as in scripts/conformance.py `audit_pdfs`. */
const REFERENCE_EXPECTATION = Object.freeze({
  success: "convert",
  error: "excluded",
  unsupported: "excluded",
  unknown: "not_run",
});
/** Typed errors that indicate a timeout, I/O, runaway output, or an internal failure, never an input verdict. */
const ENVIRONMENT_ERRORS = new Set(["CANCELLED", "IO", "LIMIT_EXCEEDED", "UNKNOWN"]);
const HASH_CHUNK = 1024 * 1024;
const MAX_MATRIX_BYTES = 16 * 1024 * 1024;
const MAX_REASON = 500;
const QPDF_OUTPUT_BYTES = 1024 * 1024;
const run = promisify(execFile);

class SetupError extends Error {
  name = "SetupError";
}

/**
 * What the runner requires of `row`:
 * - `unsupported`: the API must reject the detected format as unsupported.
 * - `convert`: the reference conversion succeeded; the output must pass
 *   `qpdf --check` and have the reference output page count.
 * - `excluded`: the reference conversion failed or was unsupported, and the
 *   repository records no Rust outcome; conformance.py reports it `EXCLUDED`.
 * - `not_run`: the reference outcome is unknown; conformance.py reports it
 *   `NOT_RUN`.
 * For `excluded` and `not_run` rows the conversion still runs; a typed input
 * rejection or a validated output is recorded as `observed`, never as a pass.
 */
export function expectationFor(row) {
  return API_UNSUPPORTED.has(row.detected_type) ? "unsupported" : REFERENCE_EXPECTATION[row.expected_outcome];
}

/** Load the committed matrix and check the fields this runner relies on. */
export async function loadMatrix(path = DEFAULT_MATRIX) {
  if ((await stat(path)).size > MAX_MATRIX_BYTES) throw new SetupError(`matrix exceeds ${MAX_MATRIX_BYTES} bytes`);
  const matrix = JSON.parse(await readFile(path, "utf8"));
  if (matrix?.schema_version !== 1 || !Array.isArray(matrix.samples) || matrix.samples.length === 0) {
    throw new SetupError("matrix must have schema_version 1 and a nonempty samples array");
  }
  const ids = new Set();
  const paths = new Set();
  for (const row of matrix.samples) {
    const id = row?.id;
    if (typeof id !== "string" || id === "" || ids.has(id)) throw new SetupError(`missing or duplicate sample ID: ${id}`);
    ids.add(id);
    const parts = typeof row.path === "string" ? row.path.split("/") : [];
    if (parts.length === 0 || parts.some((part) => part === "" || part === "." || part === "..")
        || /[\\\0:]/.test(row.path)) {
      throw new SetupError(`${id}: unsafe relative path`);
    }
    if (paths.has(row.path)) throw new SetupError(`${id}: duplicate path`);
    paths.add(row.path);
    if (!Number.isSafeInteger(row.size_bytes) || row.size_bytes < 0) throw new SetupError(`${id}: invalid size_bytes`);
    if (!/^[0-9a-f]{64}$/.test(row.sha256 ?? "")) throw new SetupError(`${id}: invalid sha256`);
    if (!/^[0-9a-f]{40}$/.test(row.git_blob_oid ?? "")) throw new SetupError(`${id}: invalid git_blob_oid`);
    if (!API_CONVERTED.has(row.detected_type) && !API_UNSUPPORTED.has(row.detected_type)) {
      throw new SetupError(`${id}: invalid detected_type`);
    }
    if (!Object.hasOwn(REFERENCE_EXPECTATION, row.expected_outcome)) {
      throw new SetupError(`${id}: invalid expected_outcome`);
    }
  }
  return matrix.samples;
}

/** Stream the open file once through SHA-256 and the Git blob SHA-1 with one fixed buffer. */
async function checkIdentity(handle, row) {
  const size = (await handle.stat()).size;
  if (size !== row.size_bytes) throw new Error(`size mismatch: expected ${row.size_bytes}, got ${size}`);
  const sha256 = createHash("sha256");
  const blob = createHash("sha1").update(`blob ${size}\0`);
  const buffer = new Uint8Array(HASH_CHUNK);
  let total = 0;
  for (;;) {
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, total);
    if (bytesRead === 0) break;
    const chunk = buffer.subarray(0, bytesRead);
    sha256.update(chunk);
    blob.update(chunk);
    total += bytesRead;
    if (total > size) break;
  }
  if (total !== size) throw new Error(`file changed while hashing: read ${total} of ${size} bytes`);
  const digest = sha256.digest("hex");
  if (digest !== row.sha256) throw new Error(`SHA-256 mismatch: ${digest}`);
  const oid = blob.digest("hex");
  if (oid !== row.git_blob_oid) throw new Error(`Git blob hash mismatch: ${oid}`);
}

/**
 * Open `row` inside the resolved corpus root and verify its identity. Every
 * path component is checked with `lstat`, so neither the file nor a parent
 * directory may be a symbolic link, and matrix paths have no `..`
 * components. The file is opened without following a final link and must be
 * the inode that was checked. The caller closes the returned handle.
 */
async function openVerified(root, row) {
  let path = root;
  let info;
  for (const part of row.path.split("/")) {
    path = join(path, part);
    info = await lstat(path);
    if (info.isSymbolicLink()) throw new Error(`symbolic link in corpus path: ${part}`);
  }
  if (!info.isFile()) throw new Error("not a regular file");
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = await handle.stat();
    if (opened.dev !== info.dev || opened.ino !== info.ino) throw new Error("file was replaced while opening");
    await checkIdentity(handle, row);
    return handle;
  } catch (error) {
    await handle.close();
    throw error;
  }
}

async function qpdfVersion(qpdf, timeoutMs) {
  if (!qpdf) return null;
  try {
    return (await run(qpdf, ["--version"], { timeout: timeoutMs })).stdout.split("\n")[0].trim();
  } catch {
    return null;
  }
}

function warningLine(text) {
  return /^.*\bwarning\s*:.*$/im.exec(text)?.[0].trim();
}

/**
 * Validate a PDF with qpdf and return the page count it reports. Like
 * `check_qpdf_log` in scripts/jbig2_oracle.py and the Rust KDH corpus test,
 * a qpdf warning fails: `qpdf --check` must exit 0 (exit 3 means warnings)
 * and print no `WARNING:` line.
 */
async function qpdfPages(context, path, signal) {
  const options = { timeout: context.timeoutMs, maxBuffer: QPDF_OUTPUT_BYTES, signal };
  const invoke = async (args) => {
    try {
      return await run(context.qpdf, args, options);
    } catch (error) {
      signal?.throwIfAborted();
      const output = `${error.stdout ?? ""}\n${error.stderr ?? ""}`;
      const detail = warningLine(output) ?? output.trim().split("\n")[0];
      const status = error.killed || error.signal ? `killed by ${error.signal ?? "timeout"}` : `exit ${error.code}`;
      throw new Error(`qpdf ${args[0]} ${error.code === 3 ? "warned" : "failed"} (${status})${detail ? `: ${detail}` : ""}`);
    }
  };
  const checked = await invoke(["--check", path]);
  const warning = warningLine(`${checked.stdout}\n${checked.stderr}`);
  if (warning) throw new Error(`qpdf --check warned (exit 0): ${warning}`);
  if (!checked.stdout.includes("No syntax or stream encoding errors found")) {
    throw new Error(`qpdf --check: ${checked.stdout.trim().split("\n")[0]}`);
  }
  return Number((await invoke(["--show-npages", path])).stdout.trim());
}

/** The outcome for a typed rejection that `expectation` allows, or `undefined` for a failure. */
export function allowedRejection(error, row, expectation) {
  if (expectation === "unsupported") {
    const format = row.detected_type.toLowerCase();
    return error instanceof UnsupportedFormatError && error.format === format ? { outcome: "unsupported" } : undefined;
  }
  if (expectation !== "convert" && error instanceof Caj2PdfError && !ENVIRONMENT_ERRORS.has(error.code)) {
    return { outcome: expectation, observed: `rejected: ${describe(error)}` };
  }
  return undefined;
}

/**
 * Convert one verified source handle to a private temporary PDF, validate
 * it, and return its outcome fields. Throws on a failure. The temporary
 * directory is removed on every path, including an abort.
 */
async function convertSample(handle, row, expectation, context, signal) {
  const directory = await mkdtemp(join(context.tempDirectory, "caj2pdf-corpus-"));
  const outputPath = join(directory, "output.pdf");
  let output;
  try {
    output = (await open(outputPath, "wx", 0o600)).createWriteStream();
    const timeout = AbortSignal.timeout(context.timeoutMs);
    let report;
    try {
      report = await convert(context.wasm, await fileHandleSource(handle), nodeWritableSink(output), {
        signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
        limits: { maxOutputBytes: context.maxOutputBytes },
      });
    } catch (error) {
      signal?.throwIfAborted();
      const allowed = allowedRejection(error, row, expectation);
      if (allowed) return allowed;
      throw error;
    }
    output.end();
    await finished(output);
    if (expectation === "unsupported") throw new Error(`${row.detected_type} input converted instead of being rejected`);
    const format = row.detected_type.toLowerCase();
    if (report.format !== format) throw new Error(`detected ${report.format}, expected ${format}`);
    const pages = row.expected_pdf?.page_count ?? row.page_count;
    if (expectation === "convert" && Number.isSafeInteger(pages) && report.pagesConverted !== pages) {
      throw new Error(`converter reported ${report.pagesConverted} pages, expected ${pages}`);
    }
    let observed = `converted ${report.pagesConverted} pages`;
    if (context.qpdf == null) {
      if (expectation === "convert") return { outcome: "not_run", reason: "qpdf unavailable; output not validated" };
      return { outcome: expectation, observed: `${observed}; not validated (qpdf unavailable)` };
    }
    const counted = await qpdfPages(context, outputPath, signal);
    if (counted !== report.pagesConverted) throw new Error(`qpdf counts ${counted} pages, expected ${report.pagesConverted}`);
    observed += "; qpdf --check clean";
    return { outcome: expectation === "convert" ? "passed" : expectation, observed };
  } finally {
    output?.destroy();
    await rm(directory, { recursive: true, force: true });
  }
}

function expectationReason(row, expectation) {
  const kind = row.python_reference?.error_kind;
  const reference = `reference ${row.expected_outcome}${kind ? ` (${kind})` : ""}`;
  if (expectation === "unsupported") return `the JavaScript API rejects ${row.detected_type}; ${reference}`;
  return `${reference}; no recorded Rust outcome`;
}

function describe(error) {
  const message = error?.message ?? String(error);
  const label = typeof error?.code === "string" ? error.code : error?.name ?? "Error";
  const text = message.startsWith(`${label}:`) ? message : `${label}: ${message}`;
  return text.length > MAX_REASON ? `${text.slice(0, MAX_REASON)}...` : text;
}

/**
 * Run the optional corpus check and resolve with the JSON report.
 *
 * Options: `corpusDir` (unset or "" reports NOT_RUN), `matrixPath`, `wasm`
 * (a compiled module; loaded from `wasmPath` only when the corpus is set),
 * `qpdf` (executable, or `null` to skip validation), `tempDirectory`,
 * `timeoutMs` per conversion and per qpdf call, `maxOutputBytes`, `signal`
 * (an abort stops the run after cleaning up and rejects with its reason),
 * and `onSample(result)`, called after each sample's conversion and before
 * the final identity re-check.
 */
export async function runCorpus({
  corpusDir,
  matrixPath = DEFAULT_MATRIX,
  wasm,
  wasmPath = DEFAULT_WASM,
  qpdf = "qpdf",
  tempDirectory = tmpdir(),
  timeoutMs = 10 * 60 * 1000,
  maxOutputBytes = 4n * 1024n * 1024n * 1024n,
  signal,
  onSample,
} = {}) {
  const samples = await loadMatrix(matrixPath);
  const report = {
    status: "NOT_RUN",
    reason: null,
    sample_count: samples.length,
    qpdf: null,
    checked: 0,
    passed: 0,
    failed: 0,
    unsupported: 0,
    excluded: 0,
    not_run: 0,
    failures: [],
    results: [],
  };
  if (!corpusDir) {
    report.reason = "CAJ2PDF_CORPUS_DIR is unset";
    return report;
  }
  let root;
  try {
    root = await realpath(resolve(corpusDir));
    if (!(await stat(root)).isDirectory()) throw new Error("not a directory");
  } catch (error) {
    return { ...report, status: "FAIL", reason: `corpus directory ${corpusDir}: ${describe(error)}` };
  }
  report.qpdf = await qpdfVersion(qpdf, timeoutMs);
  const context = {
    wasm: wasm ?? await loadModule(wasmPath),
    qpdf: report.qpdf == null ? null : qpdf,
    tempDirectory,
    timeoutMs,
    maxOutputBytes,
  };

  const verified = [];
  for (const row of samples) {
    signal?.throwIfAborted();
    const expectation = expectationFor(row);
    const result = {
      id: row.id,
      format: row.detected_type,
      reference: row.expected_outcome,
      expectation,
      outcome: "failed",
      stage: "before",
      reason: null,
      observed: null,
    };
    report.results.push(result);
    let handle;
    try {
      handle = await openVerified(root, row);
      verified.push({ row, result });
      result.stage = "convert";
      Object.assign(result, await convertSample(handle, row, expectation, context, signal));
      result.stage = null;
      if (result.outcome !== "passed") result.reason ??= expectationReason(row, expectation);
    } catch (error) {
      signal?.throwIfAborted();
      result.reason = describe(error);
    } finally {
      await handle?.close();
    }
    await onSample?.({ ...result });
  }
  // Every source verified before conversion must still match afterwards.
  for (const { row, result } of verified) {
    signal?.throwIfAborted();
    try {
      await (await openVerified(root, row)).close();
    } catch (error) {
      Object.assign(result, { outcome: "failed", stage: "after", reason: describe(error) });
    }
  }

  for (const { id, format, outcome, stage, reason } of report.results) {
    report[outcome] += 1;
    if (outcome === "failed") report.failures.push({ id, format, stage, reason });
  }
  report.checked = report.results.length;
  // As in conformance.py: unsupported and excluded rows neither pass nor
  // block a PASS; any failure fails; a not-run row or no pass is NOT_RUN.
  if (report.failed > 0) {
    report.status = "FAIL";
  } else if (report.not_run > 0) {
    report.reason = `${report.not_run} sample(s) not run: unknown reference outcome or qpdf unavailable`;
  } else if (report.passed === 0) {
    report.reason = "no sample with a successful reference conversion passed";
  } else {
    report.status = "PASS";
  }
  return report;
}

async function main(argv, signal) {
  const options = { corpusDir: process.env.CAJ2PDF_CORPUS_DIR, signal };
  const flags = { "--matrix": "matrixPath", "--wasm": "wasmPath", "--qpdf": "qpdf" };
  for (let index = 0; index < argv.length; index += 2) {
    const [flag, value] = [argv[index], argv[index + 1]];
    if (!Object.hasOwn(flags, flag) || !value) {
      throw new SetupError("usage: node js/scripts/corpus.mjs [--matrix PATH] [--wasm PATH] [--qpdf PATH]");
    }
    options[flags[flag]] = value;
  }
  options.onSample = ({ id, outcome, reason }) => {
    process.stderr.write(`${outcome.toUpperCase()} ${id}${reason ? `: ${reason}` : ""}\n`);
  };
  const report = await runCorpus(options);
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  return report.status === "FAIL" ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const controller = new AbortController();
  for (const name of ["SIGINT", "SIGTERM"]) {
    // A second signal gets the default handler and exits at once.
    process.once(name, () => controller.abort(new Error(`interrupted by ${name}`)));
  }
  try {
    process.exitCode = await main(process.argv.slice(2), controller.signal);
  } catch (error) {
    if (controller.signal.aborted) {
      process.stderr.write(`Corpus run ${controller.signal.reason.message}; temporary files removed\n`);
      process.exitCode = 130;
    } else {
      process.stderr.write(`Corpus setup error: ${describe(error)}\n`);
      process.exitCode = 2;
    }
  }
}

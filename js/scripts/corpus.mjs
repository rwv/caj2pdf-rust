#!/usr/bin/env node
// SPDX-License-Identifier: MIT

// Usage: CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples node js/scripts/corpus.mjs [--matrix PATH] [--wasm PATH]
//
// Optional external-corpus run of the JavaScript API. It converts every
// CAJ, KDH, and PDF entry of tests/conformance/matrix.json from a local
// corpus checkout through `fileHandleSource` and `nodeWritableSink`, and
// expects HN, C8, and TEB entries to be rejected as unsupported. It never
// fetches the corpus. With CAJ2PDF_CORPUS_DIR unset or empty the report is
// NOT_RUN with zero counts; a requested but missing or changed corpus FAILs.
// Exit codes: 0 for PASS or NOT_RUN, 1 for FAIL, 2 for a setup error.
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { lstat, mkdtemp, open, readFile, realpath, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import { finished } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { convert, fileHandleSource, loadModule, nodeWritableSink, UnsupportedFormatError } from "../node.mjs";

export const DEFAULT_MATRIX = fileURLToPath(new URL("../../tests/conformance/matrix.json", import.meta.url));
export const DEFAULT_WASM = fileURLToPath(
  new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url),
);
const CONVERTED = new Set(["CAJ", "KDH", "PDF"]);
const UNSUPPORTED = new Set(["HN", "C8", "TEB"]);
const HASH_CHUNK = 1024 * 1024;
const MAX_MATRIX_BYTES = 16 * 1024 * 1024;
const MAX_REASON = 500;
const run = promisify(execFile);

class SetupError extends Error {
  name = "SetupError";
}

/** Load the committed matrix and check the fields this runner relies on. */
export async function loadMatrix(path = DEFAULT_MATRIX) {
  if ((await stat(path)).size > MAX_MATRIX_BYTES) throw new SetupError(`matrix exceeds ${MAX_MATRIX_BYTES} bytes`);
  const matrix = JSON.parse(await readFile(path, "utf8"));
  if (matrix?.schema_version !== 1 || !Array.isArray(matrix.samples) || matrix.samples.length === 0) {
    throw new SetupError("matrix must have schema_version 1 and a nonempty samples array");
  }
  const ids = new Set();
  for (const row of matrix.samples) {
    const id = row?.id;
    if (typeof id !== "string" || id === "" || ids.has(id)) throw new SetupError(`missing or duplicate sample ID: ${id}`);
    ids.add(id);
    const parts = typeof row.path === "string" ? row.path.split("/") : [];
    if (parts.length === 0 || parts.some((part) => part === "" || part === "." || part === "..")
        || /[\\\0:]/.test(row.path)) {
      throw new SetupError(`${id}: unsafe relative path`);
    }
    if (!Number.isSafeInteger(row.size_bytes) || row.size_bytes < 0) throw new SetupError(`${id}: invalid size_bytes`);
    if (!/^[0-9a-f]{64}$/.test(row.sha256 ?? "")) throw new SetupError(`${id}: invalid sha256`);
    if (!/^[0-9a-f]{40}$/.test(row.git_blob_oid ?? "")) throw new SetupError(`${id}: invalid git_blob_oid`);
    if (!CONVERTED.has(row.detected_type) && !UNSUPPORTED.has(row.detected_type)) {
      throw new SetupError(`${id}: invalid detected_type`);
    }
  }
  return matrix.samples;
}

/** The regular, non-symlink file for `row` inside the resolved corpus root. */
async function corpusFile(root, row) {
  const candidate = join(root, ...row.path.split("/"));
  if ((await lstat(candidate)).isSymbolicLink()) throw new Error("canonical file is a symbolic link");
  const resolved = await realpath(candidate);
  const inside = relative(root, resolved);
  if (inside === "" || inside.startsWith(`..${sep}`) || inside === ".." || isAbsolute(inside)) {
    throw new Error("path escapes the corpus directory");
  }
  if (!(await stat(resolved)).isFile()) throw new Error("not a regular file");
  return resolved;
}

/** Stream the file once through SHA-256 and the Git blob SHA-1 with one fixed buffer. */
async function checkIdentity(path, row) {
  const handle = await open(path, "r");
  try {
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
  } finally {
    await handle.close();
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

/**
 * Convert one corpus file to a private temporary PDF, validate it, and
 * return "passed", "unsupported", or "not_run" (with a reason). Throws on a
 * failure. The temporary directory is removed on every path.
 */
async function convertSample(path, row, context) {
  const directory = await mkdtemp(join(context.tempDirectory, "caj2pdf-corpus-"));
  const outputPath = join(directory, "output.pdf");
  let input;
  let output;
  try {
    input = await open(path, "r");
    output = (await open(outputPath, "wx", 0o600)).createWriteStream();
    const expectUnsupported = UNSUPPORTED.has(row.detected_type);
    let report;
    try {
      report = await convert(context.wasm, await fileHandleSource(input), nodeWritableSink(output), {
        signal: AbortSignal.timeout(context.timeoutMs),
        limits: { maxOutputBytes: context.maxOutputBytes },
      });
    } catch (error) {
      if (expectUnsupported && error instanceof UnsupportedFormatError
          && error.format === row.detected_type.toLowerCase()) {
        return { outcome: "unsupported" };
      }
      throw error;
    }
    output.end();
    await finished(output);
    if (expectUnsupported) throw new Error(`${row.detected_type} input converted instead of being rejected`);
    if (report.format !== row.detected_type.toLowerCase()) {
      throw new Error(`detected ${report.format}, expected ${row.detected_type.toLowerCase()}`);
    }
    const pages = row.expected_pdf?.page_count ?? row.page_count;
    if (Number.isSafeInteger(pages) && report.pagesConverted !== pages) {
      throw new Error(`converter reported ${report.pagesConverted} pages, expected ${pages}`);
    }
    if (context.qpdf == null) return { outcome: "not_run", reason: "qpdf unavailable; output not validated" };
    const options = { timeout: context.timeoutMs, maxBuffer: 1024 * 1024 };
    const checked = await run(context.qpdf, ["--check", outputPath], options);
    if (!checked.stdout.includes("No syntax or stream encoding errors found")) {
      throw new Error(`qpdf --check: ${checked.stdout.trim()}`);
    }
    const counted = Number((await run(context.qpdf, ["--show-npages", outputPath], options)).stdout.trim());
    if (counted !== report.pagesConverted) throw new Error(`qpdf counts ${counted} pages, expected ${report.pagesConverted}`);
    return { outcome: "passed" };
  } finally {
    output?.destroy();
    await input?.close();
    await rm(directory, { recursive: true, force: true });
  }
}

function describe(error) {
  const message = error?.message ?? String(error);
  const label = error?.code ?? error?.name ?? "Error";
  const text = message.startsWith(`${label}:`) ? message : `${label}: ${message}`;
  return text.length > MAX_REASON ? `${text.slice(0, MAX_REASON)}...` : text;
}

/**
 * Run the optional corpus check and resolve with the JSON report.
 *
 * Options: `corpusDir` (unset or "" reports NOT_RUN), `matrixPath`, `wasm`
 * (a compiled module; loaded from `wasmPath` only when the corpus is set),
 * `qpdf` (executable, or `null` to skip validation), `tempDirectory`,
 * `timeoutMs` per conversion and per qpdf call, `maxOutputBytes`, and
 * `onSample(result)`, called after each sample's conversion and before the
 * final identity re-check.
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
    not_run: 0,
    failures: [],
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

  const results = [];
  const verified = [];
  for (const row of samples) {
    const result = { id: row.id, format: row.detected_type, outcome: "failed", stage: "before", reason: null };
    results.push(result);
    try {
      const path = await corpusFile(root, row);
      await checkIdentity(path, row);
      verified.push({ path, row, result });
      result.stage = "convert";
      Object.assign(result, await convertSample(path, row, context));
      result.stage = null;
    } catch (error) {
      result.reason = describe(error);
    }
    await onSample?.({ ...result });
  }
  // Every source verified before conversion must still match afterwards.
  for (const { path, row, result } of verified) {
    try {
      await checkIdentity(path, row);
    } catch (error) {
      Object.assign(result, { outcome: "failed", stage: "after", reason: describe(error) });
    }
  }

  for (const { id, format, outcome, stage, reason } of results) {
    report[outcome] += 1;
    if (outcome === "failed") report.failures.push({ id, format, stage, reason });
  }
  report.checked = results.length;
  if (report.failed > 0) {
    report.status = "FAIL";
  } else if (report.not_run > 0 || report.passed === 0) {
    report.reason = report.not_run > 0 ? "qpdf unavailable; converted outputs were not validated" : "no sample converted";
  } else {
    report.status = "PASS";
  }
  return report;
}

async function main(argv) {
  const options = { corpusDir: process.env.CAJ2PDF_CORPUS_DIR };
  for (let index = 0; index < argv.length; index += 2) {
    const [flag, value] = [argv[index], argv[index + 1]];
    if (flag === "--matrix" && value) options.matrixPath = value;
    else if (flag === "--wasm" && value) options.wasmPath = value;
    else throw new SetupError("usage: node js/scripts/corpus.mjs [--matrix PATH] [--wasm PATH]");
  }
  options.onSample = ({ id, outcome, reason }) => {
    process.stderr.write(`${outcome.toUpperCase()} ${id}${reason ? `: ${reason}` : ""}\n`);
  };
  const report = await runCorpus(options);
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  return report.status === "FAIL" ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = await main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`Corpus setup error: ${describe(error)}\n`);
    process.exitCode = 2;
  }
}

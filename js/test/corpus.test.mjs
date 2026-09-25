// SPDX-License-Identifier: MIT

/**
 * The optional external-corpus runner (js/scripts/corpus.mjs), exercised
 * against a synthetic corpus and matrix built at test time. No external
 * corpus file is read here.
 */
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, appendFile, chmod, mkdir, readdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";
import { Caj2PdfError, UnsupportedFormatError } from "../node.mjs";
import { allowedRejection, expectationFor, runCorpus } from "../scripts/corpus.mjs";
import { fixture, hasQpdf, syntheticCaj, syntheticKdh, tempDirectory, wasmModule } from "./helpers.mjs";

const run = promisify(execFile);
const script = fileURLToPath(new URL("../scripts/corpus.mjs", import.meta.url));
const posix = process.platform !== "win32";

function entry(id, type, bytes, pages, outcome) {
  return {
    id,
    path: id,
    size_bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    git_blob_oid: createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex"),
    detected_type: type,
    variant: type,
    expected_outcome: outcome,
    page_count: pages,
    python_reference: { convert_status: outcome, error_kind: outcome === "error" ? "synthetic_error" : null },
  };
}

/** A temporary corpus directory, its matrix, and an empty temp-output directory. */
async function syntheticCorpus(t, { pageOverride, extra = [] } = {}) {
  const root = await tempDirectory("corpus");
  t.after(() => rm(root, { recursive: true, force: true }));
  const corpusDir = join(root, "corpus");
  const tempDir = join(root, "tmp");
  await mkdir(join(corpusDir, "sub"), { recursive: true });
  await mkdir(tempDir);
  const pdf = await fixture("valid_out_of_order_objects.pdf");
  const inputs = [
    ["sub/a.caj", "CAJ", syntheticCaj(), 2, "success"],
    ["b.kdh", "KDH", (await syntheticKdh()).wrapped, 2, "success"],
    ["c.caj", "PDF", pdf, 2, "success"],
    ["d.caj", "HN", await fixture("truncated_hn.hn"), null, "unknown"],
    ...extra,
  ];
  const samples = [];
  for (const [id, type, bytes, pages, outcome] of inputs) {
    await writeFile(join(corpusDir, id), bytes);
    samples.push(entry(id, type, bytes, pageOverride?.[id] ?? pages, outcome));
  }
  const matrixPath = join(root, "matrix.json");
  await writeFile(matrixPath, JSON.stringify({ schema_version: 1, samples }));
  return { root, corpusDir, tempDir, matrixPath, samples };
}

async function runSynthetic(t, corpus, options = {}) {
  return runCorpus({
    corpusDir: corpus.corpusDir,
    matrixPath: corpus.matrixPath,
    tempDirectory: corpus.tempDir,
    wasm: await wasmModule(),
    ...options,
  });
}

/**
 * A stand-in qpdf: `--version` succeeds; `--check` either warns with exit 3,
 * warns with exit 0, or writes `marker` and hangs until killed.
 */
async function fakeQpdf(corpus, mode) {
  const path = join(corpus.root, `qpdf-${mode}.mjs`);
  await writeFile(path, `#!${process.execPath}
import { writeFileSync } from "node:fs";
const [flag] = process.argv.slice(2);
if (flag === "--version") { console.log("qpdf version fake"); process.exit(0); }
if (${JSON.stringify(mode)} === "warn3") { console.log("WARNING: output.pdf: synthetic"); process.exit(3); }
if (${JSON.stringify(mode)} === "warn0") {
  console.log("WARNING: output.pdf: synthetic\\nNo syntax or stream encoding errors found"); process.exit(0);
}
writeFileSync(${JSON.stringify(join(corpus.root, "marker"))}, "");
setInterval(() => {}, 1000);
`);
  await chmod(path, 0o755);
  return path;
}

test("each matrix row's expectation mirrors the conformance.py outcome classes", () => {
  const rows = [
    [{ detected_type: "CAJ", expected_outcome: "success" }, "convert"],
    [{ detected_type: "CAJ", expected_outcome: "error" }, "excluded"],
    [{ detected_type: "PDF", expected_outcome: "unsupported" }, "excluded"],
    [{ detected_type: "KDH", expected_outcome: "unknown" }, "not_run"],
    // The API rejects these formats whatever the Python reference did.
    [{ detected_type: "HN", expected_outcome: "success" }, "unsupported"],
    [{ detected_type: "C8", expected_outcome: "unknown" }, "unsupported"],
    [{ detected_type: "TEB", expected_outcome: "unsupported" }, "unsupported"],
  ];
  for (const [row, expected] of rows) assert.equal(expectationFor(row), expected, JSON.stringify(row));
});

test("an unset or empty corpus directory reports NOT_RUN with zero counts", async () => {
  for (const corpusDir of [undefined, ""]) {
    const report = await runCorpus({ corpusDir });
    assert.equal(report.status, "NOT_RUN");
    assert.deepEqual(
      [report.checked, report.passed, report.failed, report.unsupported, report.excluded, report.not_run],
      [0, 0, 0, 0, 0, 0],
    );
    assert.deepEqual([report.failures, report.results], [[], []]);
    assert.ok(report.sample_count > 0, "the committed matrix is still loaded and checked");
  }
  const { stdout } = await run(process.execPath, [script], { env: { ...process.env, CAJ2PDF_CORPUS_DIR: "" } });
  const report = JSON.parse(stdout);
  assert.equal(report.status, "NOT_RUN");
  assert.equal(report.checked, 0);
  assert.equal(report.passed, 0);
});

test("a requested but missing corpus fails with exit 1 and bad arguments exit 2", async (t) => {
  const corpus = await syntheticCorpus(t);
  const missing = join(corpus.corpusDir, "missing");
  const report = await runSynthetic(t, { ...corpus, corpusDir: missing });
  assert.equal(report.status, "FAIL");
  assert.equal(report.passed, 0);
  assert.match(report.reason, /missing/);
  const env = { ...process.env, CAJ2PDF_CORPUS_DIR: missing };
  await assert.rejects(
    run(process.execPath, [script, "--matrix", corpus.matrixPath], { env }),
    (error) => error.code === 1 && JSON.parse(error.stdout).status === "FAIL",
  );
  await assert.rejects(
    run(process.execPath, [script, "--bogus", "x"], { env }),
    (error) => error.code === 2 && /usage/.test(error.stderr),
  );
});

test("synthetic CAJ, KDH, and PDF samples pass and HN is counted as unsupported", async (t) => {
  const corpus = await syntheticCorpus(t);
  const seen = [];
  const report = await runSynthetic(t, corpus, {
    async onSample(result) {
      seen.push(result.id);
      // Each conversion's temporary directory is gone before the next sample.
      assert.deepEqual(await readdir(corpus.tempDir), []);
    },
  });
  assert.deepEqual(seen, corpus.samples.map((row) => row.id));
  assert.deepEqual(report.failures, []);
  assert.equal(report.checked, 4);
  assert.equal(report.unsupported, 1, "HN is rejected as unsupported");
  assert.deepEqual(
    report.results.map(({ id, reference, expectation }) => [id, reference, expectation]),
    [
      ["sub/a.caj", "success", "convert"],
      ["b.kdh", "success", "convert"],
      ["c.caj", "success", "convert"],
      ["d.caj", "unknown", "unsupported"],
    ],
  );
  assert.match(report.results[3].reason, /API rejects HN; reference unknown/);
  if (await hasQpdf()) {
    assert.equal(report.status, "PASS");
    assert.equal(report.passed, 3, "the unsupported HN sample is not a pass");
    assert.equal(report.not_run, 0);
  } else {
    assert.ok(!process.env.CI, "qpdf is required in CI");
    t.diagnostic("qpdf is not installed; the runner reports NOT_RUN without validation");
    assert.equal(report.status, "NOT_RUN");
    assert.equal(report.passed, 0);
  }
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

test("reference errors are excluded and unknown outcomes are not run, never passed or failed", async (t) => {
  const truncated = syntheticCaj().subarray(0, 0x420);
  const corpus = await syntheticCorpus(t, {
    extra: [
      ["e.caj", "CAJ", truncated, 9, "error"],
      ["f.caj", "CAJ", syntheticCaj(), 9, "error"],
    ],
  });
  const report = await runSynthetic(t, corpus);
  assert.deepEqual(report.failures, []);
  assert.equal(report.excluded, 2);
  const [rejected, converted] = report.results.slice(4);
  assert.equal(rejected.outcome, "excluded");
  assert.match(rejected.observed, /^rejected: [A-Z_]+: /);
  assert.match(rejected.reason, /reference error \(synthetic_error\); no recorded Rust outcome/);
  // No output expectation exists, so the matrix's page count is not applied.
  assert.equal(converted.outcome, "excluded");
  assert.match(converted.observed, /^converted 2 pages/);
  if (await hasQpdf()) {
    assert.equal(report.status, "PASS", "excluded rows do not block a pass, as in conformance.py");
    assert.equal(report.passed, 3);
  }

  const unknown = await syntheticCorpus(t, { extra: [["g.kdh", "KDH", (await syntheticKdh()).wrapped, 2, "unknown"]] });
  const pending = await runSynthetic(t, unknown);
  assert.equal(pending.status, "NOT_RUN");
  assert.equal(pending.not_run, 1);
  assert.equal(pending.results[4].outcome, "not_run");
  assert.match(pending.results[4].reason, /reference unknown; no recorded Rust outcome/);
  assert.match(pending.reason, /1 sample\(s\) not run/);
});

test("only a typed input rejection is allowed, and only where the expectation permits it", () => {
  const caj = { detected_type: "CAJ" };
  const hn = { detected_type: "HN" };
  const malformed = new Caj2PdfError("bad table", "MALFORMED_CAJ");
  assert.deepEqual(allowedRejection(malformed, caj, "excluded"), {
    outcome: "excluded",
    observed: "rejected: MALFORMED_CAJ: bad table",
  });
  assert.equal(allowedRejection(malformed, caj, "not_run").outcome, "not_run");
  assert.equal(allowedRejection(malformed, caj, "convert"), undefined, "a reference success must convert");
  // Timeouts, I/O, runaway limits, internal errors, and WASM traps fail every row.
  for (const code of ["CANCELLED", "IO", "LIMIT_EXCEEDED", "UNKNOWN"]) {
    assert.equal(allowedRejection(new Caj2PdfError("x", code), caj, "excluded"), undefined, code);
  }
  assert.equal(allowedRejection(new WebAssembly.RuntimeError("unreachable"), caj, "excluded"), undefined);
  assert.equal(allowedRejection(new DOMException("timeout", "TimeoutError"), caj, "not_run"), undefined);
  // An unsupported format must be rejected as exactly that format.
  assert.deepEqual(allowedRejection(new UnsupportedFormatError("hn"), hn, "unsupported"), { outcome: "unsupported" });
  assert.equal(allowedRejection(new UnsupportedFormatError("c8"), hn, "unsupported"), undefined);
  assert.equal(allowedRejection(malformed, hn, "unsupported"), undefined);
});

test("without qpdf converted samples are NOT_RUN, never passed", async (t) => {
  const corpus = await syntheticCorpus(t);
  const report = await runSynthetic(t, corpus, { qpdf: null });
  assert.equal(report.status, "NOT_RUN");
  assert.equal(report.passed, 0);
  assert.equal(report.not_run, 3);
  assert.equal(report.unsupported, 1);
  assert.match(report.reason, /qpdf unavailable/);
});

test("a qpdf warning, with exit 3 or exit 0, or a qpdf timeout fails", { skip: !posix }, async (t) => {
  const corpus = await syntheticCorpus(t);
  const warned = await runSynthetic(t, corpus, { qpdf: await fakeQpdf(corpus, "warn3") });
  assert.equal(warned.status, "FAIL");
  assert.equal(warned.failed, 3);
  assert.match(warned.failures[0].reason, /qpdf --check warned \(exit 3\): WARNING: output\.pdf: synthetic/);

  const quiet = await runSynthetic(t, corpus, { qpdf: await fakeQpdf(corpus, "warn0") });
  assert.equal(quiet.failed, 3);
  assert.match(quiet.failures[0].reason, /qpdf --check warned \(exit 0\)/);

  const hung = await runSynthetic(t, corpus, { qpdf: await fakeQpdf(corpus, "hang"), timeoutMs: 300 });
  assert.equal(hung.failed, 3);
  assert.match(hung.failures[0].reason, /qpdf --check failed \(killed by SIGTERM\)/);
  assert.equal(hung.passed, 0);
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

test("a source whose hash differs before conversion fails without converting", async (t) => {
  const corpus = await syntheticCorpus(t);
  await appendFile(join(corpus.corpusDir, "b.kdh"), "x");
  await rm(join(corpus.corpusDir, "c.caj"));
  const report = await runSynthetic(t, corpus);
  assert.equal(report.status, "FAIL");
  assert.equal(report.failed, 2);
  assert.deepEqual(report.failures.map(({ id, stage }) => [id, stage]), [["b.kdh", "before"], ["c.caj", "before"]]);
  assert.match(report.failures[0].reason, /size mismatch/);
  assert.match(report.failures[1].reason, /ENOENT/);

  // Same size, different bytes: the SHA-256 check catches it.
  const same = await syntheticCorpus(t);
  const path = join(same.corpusDir, "sub/a.caj");
  const bytes = syntheticCaj();
  bytes[bytes.length - 2] ^= 1;
  await writeFile(path, bytes);
  const changed = await runSynthetic(t, same);
  assert.equal(changed.status, "FAIL");
  assert.deepEqual(changed.failures.map(({ id, stage }) => [id, stage]), [["sub/a.caj", "before"]]);
  assert.match(changed.failures[0].reason, /SHA-256 mismatch/);
});

test("a source changed after its conversion is detected by the final re-check", async (t) => {
  const corpus = await syntheticCorpus(t);
  const report = await runSynthetic(t, corpus, {
    async onSample(result) {
      if (result.id === "sub/a.caj") await appendFile(join(corpus.corpusDir, "sub/a.caj"), "x");
    },
  });
  assert.equal(report.status, "FAIL");
  assert.deepEqual(report.failures.map(({ id, stage }) => [id, stage]), [["sub/a.caj", "after"]]);
  assert.ok(report.passed <= 2, "the changed sample is no longer counted as passed");
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

test("a page count mismatch or conversion error fails and cleans up", async (t) => {
  const truncated = syntheticCaj().subarray(0, 0x420);
  const corpus = await syntheticCorpus(t, {
    pageOverride: { "sub/a.caj": 3 },
    extra: [["e.caj", "CAJ", truncated, 2, "success"]],
  });
  const report = await runSynthetic(t, corpus);
  assert.equal(report.status, "FAIL");
  assert.deepEqual(report.failures.map(({ id, stage }) => [id, stage]), [["sub/a.caj", "convert"], ["e.caj", "convert"]]);
  assert.match(report.failures[0].reason, /2 pages, expected 3/);
  assert.match(report.failures[1].reason, /^[A-Z_]+: /);
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

test("a symbolic link as the file or a parent directory is refused", { skip: !posix }, async (t) => {
  const corpus = await syntheticCorpus(t);
  const kdh = join(corpus.corpusDir, "b.kdh");
  const moved = join(corpus.root, "outside.kdh");
  await rename(kdh, moved);
  await symlink(moved, kdh);
  // The parent of sub/a.caj becomes a link to a real directory elsewhere.
  const sub = join(corpus.corpusDir, "sub");
  await rename(sub, join(corpus.root, "sub"));
  await symlink(join(corpus.root, "sub"), sub);
  const report = await runSynthetic(t, corpus);
  assert.equal(report.status, "FAIL");
  assert.deepEqual(
    report.failures.map(({ id, stage, reason }) => [id, stage, reason]),
    [
      ["sub/a.caj", "before", "Error: symbolic link in corpus path: sub"],
      ["b.kdh", "before", "Error: symbolic link in corpus path: b.kdh"],
    ],
  );
});

test("SIGINT stops the script with exit 130 and removes its temporary output", { skip: !posix }, async (t) => {
  const corpus = await syntheticCorpus(t);
  const qpdf = await fakeQpdf(corpus, "hang");
  const child = spawn(process.execPath, [script, "--matrix", corpus.matrixPath, "--qpdf", qpdf], {
    env: { ...process.env, CAJ2PDF_CORPUS_DIR: corpus.corpusDir, TMPDIR: corpus.tempDir },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  const exited = new Promise((resolve) => child.on("exit", (code) => resolve(code)));
  // The fake qpdf writes its marker while the first output PDF exists.
  const marker = join(corpus.root, "marker");
  for (let attempt = 0; ; attempt += 1) {
    if (await access(marker).then(() => true, () => false)) break;
    assert.ok(attempt < 1000, "the fake qpdf started");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  assert.equal((await readdir(corpus.tempDir)).length, 1, "a conversion directory exists");
  child.kill("SIGINT");
  assert.equal(await exited, 130);
  assert.match(stderr, /interrupted by SIGINT; temporary files removed/);
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

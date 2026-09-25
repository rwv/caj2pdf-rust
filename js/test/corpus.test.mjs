// SPDX-License-Identifier: MIT

/**
 * The optional external-corpus runner (js/scripts/corpus.mjs), exercised
 * against a synthetic corpus and matrix built at test time. No external
 * corpus file is read here.
 */
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFile, mkdir, readdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";
import { runCorpus } from "../scripts/corpus.mjs";
import { fixture, syntheticCaj, syntheticKdh, tempDirectory, wasmModule } from "./helpers.mjs";

const run = promisify(execFile);
const script = fileURLToPath(new URL("../scripts/corpus.mjs", import.meta.url));

let qpdfInstalled;
async function hasQpdf() {
  if (qpdfInstalled === undefined) {
    qpdfInstalled = await run("qpdf", ["--version"]).then(() => true, () => false);
  }
  return qpdfInstalled;
}

function entry(id, type, bytes, pages) {
  return {
    id,
    path: id,
    size_bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    git_blob_oid: createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex"),
    detected_type: type,
    variant: type,
    page_count: pages,
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
    ["sub/a.caj", "CAJ", syntheticCaj(), 2],
    ["b.kdh", "KDH", (await syntheticKdh()).wrapped, 2],
    ["c.caj", "PDF", pdf, 2],
    ["d.caj", "HN", await fixture("truncated_hn.hn"), null],
    ...extra,
  ];
  const samples = [];
  for (const [id, type, bytes, pages] of inputs) {
    await writeFile(join(corpusDir, id), bytes);
    samples.push(entry(id, type, bytes, pageOverride?.[id] ?? pages));
  }
  const matrixPath = join(root, "matrix.json");
  await writeFile(matrixPath, JSON.stringify({ schema_version: 1, samples }));
  return { corpusDir, tempDir, matrixPath, samples };
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

test("an unset or empty corpus directory reports NOT_RUN with zero counts", async () => {
  for (const corpusDir of [undefined, ""]) {
    const report = await runCorpus({ corpusDir });
    assert.equal(report.status, "NOT_RUN");
    assert.deepEqual(
      [report.checked, report.passed, report.failed, report.unsupported, report.not_run, report.failures.length],
      [0, 0, 0, 0, 0, 0],
    );
    assert.ok(report.sample_count > 0, "the committed matrix is still loaded and checked");
  }
  const { stdout } = await run(process.execPath, [script], { env: { ...process.env, CAJ2PDF_CORPUS_DIR: "" } });
  const report = JSON.parse(stdout);
  assert.equal(report.status, "NOT_RUN");
  assert.equal(report.checked, 0);
  assert.equal(report.passed, 0);
});

test("a requested but missing corpus directory fails with exit code 1", async (t) => {
  const corpus = await syntheticCorpus(t);
  const missing = join(corpus.corpusDir, "missing");
  const report = await runSynthetic(t, { ...corpus, corpusDir: missing });
  assert.equal(report.status, "FAIL");
  assert.equal(report.passed, 0);
  assert.match(report.reason, /missing/);
  await assert.rejects(
    run(process.execPath, [script, "--matrix", corpus.matrixPath], {
      env: { ...process.env, CAJ2PDF_CORPUS_DIR: missing },
    }),
    (error) => error.code === 1 && JSON.parse(error.stdout).status === "FAIL",
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

test("without qpdf converted samples are NOT_RUN, never passed", async (t) => {
  const corpus = await syntheticCorpus(t);
  const report = await runSynthetic(t, corpus, { qpdf: null });
  assert.equal(report.status, "NOT_RUN");
  assert.equal(report.passed, 0);
  assert.equal(report.not_run, 3);
  assert.equal(report.unsupported, 1);
  assert.match(report.reason, /qpdf unavailable/);
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
    extra: [["e.caj", "CAJ", truncated, 2]],
  });
  const report = await runSynthetic(t, corpus);
  assert.equal(report.status, "FAIL");
  assert.deepEqual(report.failures.map(({ id, stage }) => [id, stage]), [["sub/a.caj", "convert"], ["e.caj", "convert"]]);
  assert.match(report.failures[0].reason, /2 pages, expected 3/);
  assert.match(report.failures[1].reason, /^[A-Z_]+: /);
  assert.deepEqual(await readdir(corpus.tempDir), []);
});

test("a symbolic link or path outside the corpus is refused", async (t) => {
  const corpus = await syntheticCorpus(t);
  const kdh = join(corpus.corpusDir, "b.kdh");
  const moved = join(corpus.tempDir, "..", "outside.kdh");
  await rename(kdh, moved);
  await symlink(moved, kdh);
  const report = await runSynthetic(t, corpus);
  assert.equal(report.status, "FAIL");
  assert.match(report.failures[0].reason, /symbolic link/);
});

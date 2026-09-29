// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { fixture, syntheticCaj, syntheticKdh, tempDirectory, validatePdf } from "./helpers.mjs";

const example = fileURLToPath(new URL("../examples/node.mjs", import.meta.url));

function run(args, stdin) {
  return new Promise((resolve) => {
    const child = execFile(process.execPath, [example, ...args], { timeout: 30_000 }, (error, stdout, stderr) => {
      resolve({ code: error?.code ?? 0, stdout, stderr });
    });
    child.stdin.end(stdin);
  });
}

test("Node example converts CAJ, KDH and PDF files and spooled stdin", async (t) => {
  const directory = await tempDirectory("example");
  try {
    for (const [format, bytes] of [
      ["CAJ", syntheticCaj()],
      ["KDH", (await syntheticKdh()).wrapped],
      ["PDF", await fixture("valid_nested_outline.pdf")],
    ]) {
      const input = join(directory, `input.${format}`);
      await writeFile(input, bytes);
      for (const fromStdin of [false, true]) {
        const output = join(directory, `${format}-${fromStdin}.pdf`);
        const result = await run([fromStdin ? "-" : input, output], fromStdin ? bytes : undefined);
        assert.equal(result.code, 0, result.stderr);
        assert.match(result.stdout, new RegExp(`Converted ${format}: 2 pages`));
        await validatePdf(t, await readFile(output), 2);
      }
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Node example leaves no output after missing or malformed input", async () => {
  const directory = await tempDirectory("example-error");
  try {
    const output = join(directory, "output.pdf");
    const missing = await run([join(directory, "missing.caj"), output]);
    assert.equal(missing.code, 1);
    assert.match(missing.stderr, /^ENOENT:/);
    assert.deepEqual(await readdir(directory), []);
    const invalid = await run(["-", output], "invalid input");
    assert.equal(invalid.code, 1);
    assert.match(invalid.stderr, /^UNSUPPORTED_FORMAT:/);
    assert.deepEqual(await readdir(directory), []);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Node example preserves existing output and rejects extra arguments", async () => {
  const directory = await tempDirectory("example-existing");
  try {
    const input = join(directory, "input.pdf");
    const output = join(directory, "output.pdf");
    await writeFile(input, await fixture("valid_nested_outline.pdf"));
    await writeFile(output, "keep me");
    const existing = await run([input, output]);
    assert.equal(existing.code, 1);
    assert.match(existing.stderr, /^EEXIST:/);
    assert.equal(await readFile(output, "utf8"), "keep me");
    const extra = await run([input, output, "ignored-before"]);
    assert.equal(extra.code, 2);
    assert.match(extra.stderr, /^Usage:/);
    assert.equal(await readFile(output, "utf8"), "keep me");
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

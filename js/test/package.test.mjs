// SPDX-License-Identifier: MIT

// Packaging: the npm tarball carries the WASM build, entry points,
// declarations, license, and README, and nothing else. The package is
// copied to a temporary directory so the checkout is never modified.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { cp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { tempDirectory, wasmUrl } from "./helpers.mjs";

const run = promisify(execFile);
const packageDirectory = fileURLToPath(new URL("..", import.meta.url));

/** Copy the package without any built WASM, and dry-run `npm pack` there. */
async function packCopy(directory, prepare = async () => {}) {
  await cp(packageDirectory, directory, {
    recursive: true,
    filter: (source) => !source.endsWith("caj2pdf_wasm.wasm") && !source.includes("node_modules"),
  });
  await prepare();
  const env = { ...process.env, npm_config_update_notifier: "false", npm_config_cache: join(directory, ".npm") };
  const { stdout } = await run("npm", ["pack", "--dry-run", "--json", "--offline"], { cwd: directory, env });
  return JSON.parse(stdout)[0];
}

test("npm pack includes the WASM build, entry points, declarations, LICENSE, and README only", async () => {
  const directory = await tempDirectory("pack");
  try {
    // The same copy step `npm run build:wasm` uses.
    const packed = await packCopy(directory, () =>
      run(process.execPath, [join(directory, "scripts", "copy-wasm.mjs"), fileURLToPath(wasmUrl)]),
    );
    const files = new Map(packed.files.map((file) => [file.path, file]));
    assert.deepEqual([...files.keys()].sort(), [
      "LICENSE",
      "README.md",
      "browser.d.mts",
      "browser.mjs",
      "caj2pdf_wasm.wasm",
      "io.d.mts",
      "io.mjs",
      "node.d.mts",
      "node.mjs",
      "package.json",
    ]);
    assert.equal(files.get("caj2pdf_wasm.wasm").size, (await readFile(wasmUrl)).length);
    assert.equal(files.get("caj2pdf_wasm.wasm").mode & 0o777, 0o644);
    assert.equal(packed.name, "caj2pdf-rust");
    // The Node entry point finds the packaged module by default.
    const { loadModule } = await import(pathToFileURL(join(directory, "node.mjs")).href);
    assert.ok(WebAssembly.Module.exports(await loadModule()).some(({ name }) => name === "caj2pdf_io_poll"));
    const manifest = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
    assert.equal(manifest.license, "MIT");
    assert.equal(manifest.exports["./caj2pdf_wasm.wasm"], "./caj2pdf_wasm.wasm");
    for (const field of ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]) {
      assert.equal(manifest[field], undefined, `${field} must stay empty`);
    }
    for (const hook of ["preinstall", "install", "postinstall"]) {
      assert.equal(manifest.scripts[hook], undefined, `no ${hook} script`);
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("npm pack refuses to package without the WASM build", async () => {
  const directory = await tempDirectory("pack-missing");
  try {
    await assert.rejects(packCopy(directory), /caj2pdf_wasm\.wasm/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

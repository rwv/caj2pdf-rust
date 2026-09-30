// SPDX-License-Identifier: MIT

// Packaging: the npm tarball carries the WASM build, entry points,
// declarations, license, and README, and nothing else. The package is
// copied to a temporary directory so the checkout is never modified.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { syntheticCaj, tempDirectory, validatePdf, wasmUrl } from "./helpers.mjs";
import { findChrome, launchChrome, openPage, startServer } from "./browser-harness.mjs";

import { syntheticC8 } from "./hnc8-fixtures.mjs";

const run = promisify(execFile);
const packageDirectory = fileURLToPath(new URL("..", import.meta.url));

/** Copy the package without any built WASM, and run `npm pack` there. */
async function packCopy(directory, prepare = async () => {}) {
  await cp(packageDirectory, directory, {
    recursive: true,
    filter: (source) => !source.endsWith("caj2pdf_wasm.wasm") && !source.includes("node_modules"),
  });
  await prepare();
  const env = { ...process.env, npm_config_update_notifier: "false", npm_config_cache: join(directory, ".npm") };
  const { stdout } = await run("npm", ["pack", "--json", "--offline"], { cwd: directory, env });
  return JSON.parse(stdout)[0];
}

test("npm pack includes the WASM build, entry points, declarations, LICENSE, and README only", async (t) => {
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
      "internal/scratch.mjs",
      "internal/spool-write.mjs",
      "io.d.mts",
      "io.mjs",
      "node.d.mts",
      "node.mjs",
      "package.json",
    ]);
    assert.equal(files.get("caj2pdf_wasm.wasm").size, (await readFile(wasmUrl)).length);
    assert.equal(files.get("caj2pdf_wasm.wasm").mode & 0o777, 0o644);
    assert.equal(packed.name, "caj2pdf-rust");
    // Extract the actual artifact into a fresh consumer. Imports must resolve
    // through its package exports without access to omitted checkout files.
    const consumer = join(directory, "consumer");
    const installed = join(consumer, "node_modules", "caj2pdf-rust");
    await mkdir(installed, { recursive: true });
    await run("tar", ["-xzf", join(directory, packed.filename), "--strip-components=1", "-C", installed]);
    await writeFile(join(consumer, "input.caj"), syntheticCaj());
    await writeFile(join(consumer, "input.c8"), syntheticC8());
    const { stdout } = await run(process.execPath, ["--input-type=module", "--eval", `
      import assert from "node:assert/strict";
      import { open } from "node:fs/promises";
      import { finished } from "node:stream/promises";
      import * as root from "caj2pdf-rust";
      import * as node from "caj2pdf-rust/node";
      import * as browser from "caj2pdf-rust/browser";
      assert.equal(root.loadModule, node.loadModule);
      assert.equal(root.writeSpoolChunk, undefined);
      assert.equal(browser.convert, node.convert);
      const module = await root.loadModule();
      assert.ok(WebAssembly.Module.exports(module).some(({name}) => name === "caj2pdf_io_poll"));
      for (const [name, target, pages] of [["input.caj", "output.pdf", 2], ["input.c8", "c8.pdf", 1]]) {
        const input = await open(name, "r");
        const output = (await open(target, "wx")).createWriteStream();
        try {
          const report = await root.withHnc8Scratch(async (scratch) => root.convert(
            module, await root.fileHandleSource(input), root.nodeWritableSink(output),
            { includeBookmarks: name === "input.caj", hnc8: { scratch } },
          ));
          output.end();
          await finished(output);
          assert.equal(report.pagesConverted, pages);
        } finally {
          output.destroy();
          await finished(output).catch(() => {});
          await input.close();
        }
      }
      console.log("packed exports and Node conversion passed");
    `], { cwd: consumer });
    assert.match(stdout, /packed exports and Node conversion passed/);
    await validatePdf(t, await readFile(join(consumer, "output.pdf")), 2);
    await validatePdf(t, await readFile(join(consumer, "c8.pdf")), 1);

    // The public Node example must work beside the unpacked package, without
    // a workspace target/ directory or a fallback to a stale build.
    await mkdir(join(installed, "examples"));
    await cp(new URL("../examples/node.mjs", import.meta.url), join(installed, "examples", "node.mjs"));
    await run(process.execPath, [join(installed, "examples", "node.mjs"), "input.c8", "example.pdf", "--no-bookmarks"], { cwd: consumer });
    assert.deepEqual(await readFile(join(consumer, "example.pdf")), await readFile(join(consumer, "c8.pdf")));

    await t.test("packed browser entry converts in Chromium with its default WASM URL", { timeout: 60_000 }, async (t) => {
      const chrome = findChrome();
      if (!chrome) {
        assert.ok(!process.env.CI, "Chromium is required in CI");
        t.skip("no Chromium found; packed browser conversion NOT_RUN");
        return;
      }
      const server = await startServer(installed, {
        "/index.html": "<!doctype html><title>packed package</title>",
        "/input.caj": syntheticCaj(),
        "/input.c8": syntheticC8(),
        "/artifact-worker.mjs": `
          import * as api from "/browser.mjs";
          try {
            const output = await api.withHnc8Scratch(async (scratch) => {
              const input = await (await fetch("/input.c8")).blob();
              const chunks = [];
              const report = await api.convert(await api.loadModule(), api.blobSource(input), {
                async writeChunk(bytes) { chunks.push(bytes.slice()); return bytes.length; },
                async flush() {},
              }, { includeBookmarks: false, hnc8: { scratch } });
              return { pages: report.pagesConverted, bytes: Array.from(new Uint8Array(await new Blob(chunks).arrayBuffer())) };
            });
            const root = await navigator.storage.getDirectory();
            postMessage({ ...output, remaining: await Array.fromAsync(root.keys()) });
          } catch (error) { postMessage({ error: String(error) }); }
          self.close();
        `,
      });
      let browser;
      try {
        browser = await launchChrome(chrome);
        const page = await openPage(browser.cdp, `${server.origin}/index.html`);
        const output = await page.evaluate(`(async () => {
          const api = await import("/browser.mjs");
          const module = await api.loadModule();
          const input = await (await fetch("/input.caj")).blob();
          const chunks = [];
          const writer = new WritableStream({ write(bytes) { chunks.push(bytes); } }).getWriter();
          const report = await api.convert(module, api.blobSource(input), api.webWritableSink(writer));
          await writer.close();
          return { pages: report.pagesConverted, bytes: Array.from(new Uint8Array(await new Blob(chunks).arrayBuffer())) };
        })()`);
        assert.equal(output.pages, 2);
        await validatePdf(t, new Uint8Array(output.bytes), 2);
        const c8 = await page.evaluate(`new Promise((resolve, reject) => {
          const worker = new Worker("/artifact-worker.mjs", { type: "module" });
          worker.onmessage = ({ data }) => resolve(data);
          worker.onerror = (error) => reject(new Error(error.message));
        })`);
        assert.equal(c8.error, undefined);
        assert.equal(c8.pages, 1);
        assert.deepEqual(Buffer.from(c8.bytes), await readFile(join(consumer, "c8.pdf")));
        assert.deepEqual(c8.remaining, []);
        assert.match(Buffer.from(c8.bytes).toString("latin1"), /\/Filter \/FlateDecode/);
        await validatePdf(t, new Uint8Array(c8.bytes), 1);
        assert.deepEqual(page.errors, []);
      } finally {
        await browser?.close();
        await server.close();
      }
    });
    const manifest = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
    assert.equal(manifest.license, "MIT");
    assert.equal(manifest.exports["./caj2pdf_wasm.wasm"], "./caj2pdf_wasm.wasm");
    assert.equal(manifest.exports["./internal/spool-write.mjs"], undefined);
    for (const field of ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]) {
      assert.equal(manifest[field], undefined, `${field} must stay empty`);
    }
    for (const hook of ["preinstall", "install", "postinstall"]) {
      assert.equal(manifest.scripts[hook], undefined, `no ${hook} script`);
    }
    // Release CI retains this exact tested tarball, without repacking.
    if (process.env.CAJ2PDF_PACKAGE_OUTPUT) {
      await cp(join(directory, packed.filename), process.env.CAJ2PDF_PACKAGE_OUTPUT);
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("npm pack refuses to package a missing or non-WASM caj2pdf_wasm.wasm", async () => {
  const directory = await tempDirectory("pack-missing");
  try {
    await assert.rejects(packCopy(directory), /caj2pdf_wasm\.wasm/);
    await rm(directory, { recursive: true, force: true });
    await assert.rejects(
      packCopy(directory, () => writeFile(join(directory, "caj2pdf_wasm.wasm"), "not a module\n")),
      /is not a WebAssembly module/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

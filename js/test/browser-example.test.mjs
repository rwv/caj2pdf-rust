// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { findChrome, launchChrome, openPage, startServer } from "./browser-harness.mjs";
import { syntheticCaj, validatePdf, wasmUrl } from "./helpers.mjs";

import { syntheticHn, syntheticC8 } from "./hnc8-fixtures.mjs";

const chrome = findChrome();
if (!chrome && process.env.CI) throw new Error("Chromium is required in CI");
const options = { skip: !chrome && "Chromium unavailable; browser example NOT_RUN", timeout: 60_000 };
let server;
let browser;
let page;

before(async () => {
  if (!chrome) return;
  server = await startServer(fileURLToPath(new URL("..", import.meta.url)), {
    "/caj2pdf_wasm.wasm": await readFile(wasmUrl),
    "/input.caj": syntheticCaj(),
    "/input.hn": syntheticHn(),
    "/input.c8": syntheticC8(),
    "/invalid.caj": "invalid input",
  });
  browser = await launchChrome(chrome);
  page = await openPage(browser.cdp, `${server.origin}/examples/browser.html`);
  await page.evaluate("window.showSaveFilePicker = undefined");
});

after(async () => {
  await browser?.close();
  await server?.close();
});

async function convert(name, cancel = false) {
  return page.evaluate(`(async () => {
    const blob = await (await fetch(${JSON.stringify(`/${name}`)})).blob();
    const files = new DataTransfer();
    files.items.add(new File([blob], "input.caj"));
    document.querySelector("#file").files = files.files;
    const run = document.querySelector("#run");
    await new Promise((resolve) => {
      const observer = new MutationObserver(() => {
        if (!run.disabled) { observer.disconnect(); resolve(); }
      });
      observer.observe(run, { attributes: true });
      run.click();
      if (!run.disabled) throw new Error("conversion must disable the run button");
      // A second physical click must not start another conversion.
      run.click();
      if (${cancel}) document.querySelector("#cancel").click();
    });
    const root = await navigator.storage.getDirectory();
    return {
      text: document.querySelector("#result").textContent,
      files: await Array.fromAsync(root.keys()),
      href: document.querySelector("#result a")?.href,
      cancelDisabled: document.querySelector("#cancel").disabled,
    };
  })()`);
}

test("browser example removes failed and cancelled OPFS output", options, async () => {
  const failed = await convert("invalid.caj");
  assert.match(failed.text, /^UNSUPPORTED_FORMAT:/);
  assert.deepEqual(failed.files, []);
  assert.equal(failed.cancelDisabled, true);
  const cancelled = await convert("input.caj", true);
  assert.equal(cancelled.text, "Conversion cancelled.");
  assert.deepEqual(cancelled.files, []);
  assert.equal(cancelled.cancelDisabled, true);
});

test("browser example cleans up when opening its OPFS writer fails", options, async () => {
  await page.evaluate(`
    window.originalCreateWritable = FileSystemFileHandle.prototype.createWritable;
    FileSystemFileHandle.prototype.createWritable = async () => { throw new Error("writer unavailable"); };
  `);
  try {
    const result = await convert("input.caj");
    assert.equal(result.text, "Error: writer unavailable");
    assert.deepEqual(result.files, []);
  } finally {
    await page.evaluate("FileSystemFileHandle.prototype.createWritable = window.originalCreateWritable");
  }
});

test("browser example replaces and discards downloadable OPFS output", options, async (t) => {
  const first = await convert("input.caj");
  assert.match(first.text, /^Converted CAJ: 2 pages/);
  assert.equal(first.files.length, 1);
  const bytes = await page.evaluate(`fetch(${JSON.stringify(first.href)}).then(r => r.arrayBuffer()).then(b => Array.from(new Uint8Array(b)))`);
  await validatePdf(t, new Uint8Array(bytes), 2);
  const second = await convert("input.caj");
  assert.equal(second.files.length, 1);
  assert.notEqual(second.files[0], first.files[0]);
  const result = await page.evaluate(`(async () => {
    const discard = document.querySelector("#discard");
    await new Promise((resolve) => {
      const observer = new MutationObserver(() => {
        if (discard.hidden) { observer.disconnect(); resolve(); }
      });
      observer.observe(discard, { attributes: true });
      discard.click();
    });
    const root = await navigator.storage.getDirectory();
    return {
      files: await Array.fromAsync(root.keys()),
      links: document.querySelectorAll("#result a").length,
      oldUrlReadable: await fetch(${JSON.stringify(first.href)}).then(() => true, () => false),
      newUrlReadable: await fetch(${JSON.stringify(second.href)}).then(() => true, () => false),
    };
  })()`);
  assert.deepEqual(result, { files: [], links: 0, oldUrlReadable: false, newUrlReadable: false });
  assert.deepEqual(page.errors, []);
});


test("browser worker example converts HN/C8 with standard tables and removes scratch", options, async () => {
  for (const format of ["hn", "c8"]) {
    await page.evaluate(`document.querySelector('#bookmarks').checked = ${format !== "c8"}`);
    const result = await convert(`input.${format}`);
    assert.match(result.text, new RegExp(`^Converted ${format.toUpperCase()}: 1 pages`));
    assert.equal(result.files.length, 1); // Only the downloadable output remains.
    assert.ok(result.files[0].startsWith("output-"));
    await page.evaluate("document.querySelector('#discard').click()");
    await page.evaluate(`(async () => {
      while (document.querySelector('#discard').disabled) await new Promise(resolve => setTimeout(resolve, 10));
    })()`);
    assert.deepEqual(await page.evaluate("(async () => Array.fromAsync((await navigator.storage.getDirectory()).keys()))()"), []);
  }
});

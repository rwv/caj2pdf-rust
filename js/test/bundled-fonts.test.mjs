// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { test } from "node:test";
import { convert } from "../node.mjs";
import { FONT_PACK } from "../internal/font-pack.mjs";
import { syntheticNativeC8, syntheticNativeHnb, syntheticC8 } from "./hnc8-fixtures.mjs";
import { wasmModule, wasmUrl, validatePdf } from "./helpers.mjs";
import { findChrome, launchChrome, openPage, startServer } from "./browser-harness.mjs";

const originals = [syntheticNativeC8(), syntheticNativeHnb(0)];
async function output(input, options) {
  const parts = [];
  const result = await convert(await wasmModule(), new Blob([input]), {
    async writeChunk(bytes) { assert.ok(bytes.length <= 65536); parts.push(bytes.slice()); return bytes.length; },
    async flush() {},
  }, options);
  return { bytes: Buffer.concat(parts), result };
}

test("bundled native presets convert both modes without user font options", async (t) => {
  for (const input of originals) {
    const { bytes, result } = await output(input);
    assert.ok(result.pagesConverted > 0);
    assert.ok(bytes.includes(Buffer.from("HGBZ_CNKI")));
    await validatePdf(t, bytes, result.pagesConverted);
    await assert.rejects(output(input, { hnc8: { fonts: false } }));
  }
  const automatic = await output(syntheticC8());
  const disabled = await output(syntheticC8(), { hnc8: { fonts: false } });
  assert.deepEqual(automatic.bytes, disabled.bytes);
});

test("browser automatically caches fonts, recovers corrupt cache, and reuses it offline", async (t) => {
  const chrome = findChrome();
  if (!chrome) { if (process.env.CI) throw new Error("Chromium required"); t.skip("Chromium unavailable"); return; }
  const routes = { "/index.html": "<!doctype html><title>bundled fonts</title>", "/caj2pdf_wasm.wasm": await readFile(wasmUrl),
    "/native.c8": originals[0], "/native.hn": originals[1], "/image.c8": syntheticC8() };
  const server = await startServer(fileURLToPath(new URL("..", import.meta.url)), routes, { isolated: true });
  const browser = await launchChrome(chrome);
  try {
    const page = await openPage(browser.cdp, server.origin + "/index.html");
    await page.evaluate(`(async () => {
      const api = await import('/browser.mjs');
      const module = await api.loadModule();
      const inputs = await Promise.all(['/native.c8', '/native.hn', '/image.c8'].map(async u => (await fetch(u)).blob()));
      globalThis.fontCase = async (index, options) => {
        const parts = []; let maxWrite = 0;
        const report = await api.convert(module, inputs[index], {
          async writeChunk(b) { maxWrite = Math.max(maxWrite,b.length); parts.push(b.slice()); return b.length; }, async flush() {}
        }, options);
        const bytes = await new Blob(parts).arrayBuffer();
        const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))].map(b => b.toString(16).padStart(2,'0')).join('');
        return { hash, maxWrite, pages: report.pagesConverted };
      };
    })()`);
    // No font directory is created by image conversion or inspection.
    await page.evaluate("fontCase(2)");
    assert.equal(await page.evaluate(`(async () => { const root=await navigator.storage.getDirectory(); try { await root.getDirectoryHandle('caj2pdf-fonts'); return true; } catch(e) { if(e.name==='NotFoundError') return false; throw e; } })()`), false);
    // Same-size corrupt bytes must fail verification and be replaced automatically.
    await page.evaluate(`(async () => { const root=await navigator.storage.getDirectory(); const dir=await root.getDirectoryHandle('caj2pdf-fonts',{create:true}); const file=await dir.getFileHandle('${FONT_PACK.sha256}',{create:true}); const w=await file.createWritable(); await w.truncate(${FONT_PACK.size}); await w.close(); })()`);
    const first = await page.evaluate("fontCase(0)");
    assert.ok(first.maxWrite <= 65536);
    assert.equal(first.hash, createHash("sha256").update((await output(originals[0])).bytes).digest("hex"));
    // Break the downloadable resource while retaining the OPFS cache. Both
    // concurrent conversions must finish using the already verified bytes.
    routes["/caj2pdf-fonts-v1.bin"] = "offline";
    const repeated = await page.evaluate("Promise.all([fontCase(0), fontCase(1)])");
    assert.deepEqual(repeated[0], first);
    assert.equal(repeated[1].hash, createHash("sha256").update((await output(originals[1])).bytes).digest("hex"));
    // A truncated new download fails and leaves no reusable partial cache.
    await page.evaluate("navigator.storage.getDirectory().then(r=>r.removeEntry('caj2pdf-fonts',{recursive:true}))");
    await assert.rejects(page.evaluate("fontCase(0)"), /truncated/);
    assert.equal(await page.evaluate(`(async () => { const root=await navigator.storage.getDirectory(); const dir=await root.getDirectoryHandle('caj2pdf-fonts'); return (await Array.fromAsync(dir.keys())).length; })()`), 0);
    // Cancellation while waiting for the shared cache lock must settle.
    await page.evaluate(`(async () => {
      globalThis.releaseFontLock = null;
      navigator.locks.request('caj2pdf-fonts-${FONT_PACK.sha256}',()=>new Promise(resolve=>{globalThis.releaseFontLock=resolve;}));
      while (!globalThis.releaseFontLock) await new Promise(r=>setTimeout(r,1));
    })()`);
    const cancelled = await page.evaluate(`(async () => { const c=new AbortController(); setTimeout(()=>c.abort(),100); try { await fontCase(0,{signal:c.signal}); return false; } catch(e) { return e.name==='AbortError'; } finally { releaseFontLock(); } })()`);
    assert.equal(cancelled, true);
  } finally { await browser.close(); await server.close(); }
});

// SPDX-License-Identifier: MIT

import { FONT_PACK } from "./font-pack.mjs";

const packUrl = new URL("../caj2pdf-fonts-v1.bin", import.meta.url);

/** Open and verify the package resource. Browser downloads stream to OPFS;
 * a hash-named cache is committed only after a complete, verified download.
 * `accept` verifies through the WASM ranged reader, without a whole-pack buffer.
 */
export async function loadFontPack(accept, cancelled) {
  if (typeof process !== "undefined" && process.versions?.node != null) {
    const { fileURLToPath } = await import("node:url");
    const { stat } = await import("node:fs/promises");
    const path = fileURLToPath(packUrl);
    if ((await stat(path)).size !== FONT_PACK.size || !await accept({ kind: "path", path, size: FONT_PACK.size })) {
      throw new Error("bundled font pack checksum mismatch; reinstall the package");
    }
    return;
  }
  if (!navigator.storage?.getDirectory || !navigator.locks) {
    throw new Error("automatic fonts require OPFS in a secure browser context; provide hnc8.fonts for a custom source");
  }
  const controller = new AbortController();
  const check = () => { if (cancelled()) controller.abort(); controller.signal.throwIfAborted(); };
  const timer = setInterval(() => { if (cancelled()) controller.abort(); }, 50);
  try {
    check();
    await navigator.locks.request(`caj2pdf-fonts-${FONT_PACK.sha256}`, { signal: controller.signal }, async () => {
      check();
      const root = await navigator.storage.getDirectory();
      const directory = await root.getDirectoryHandle("caj2pdf-fonts", { create: true });
      const name = FONT_PACK.sha256;
      let handle;
      try { handle = await directory.getFileHandle(name); } catch (error) { if (error.name !== "NotFoundError") throw error; }
      if (handle) {
        const blob = await handle.getFile();
        if (blob.size === FONT_PACK.size && await accept({ kind: "blob", blob, size: blob.size })) return;
        check();
        await directory.removeEntry(name);
      }
      const response = await fetch(packUrl, { signal: controller.signal });
      if (!response.ok || !response.body) throw new Error(`font pack download failed (${response.status})`);
      handle = await directory.getFileHandle(name, { create: true });
      const writer = await handle.createWritable();
      const reader = response.body.getReader();
      let size = 0;
      try {
        for (;;) {
          check();
          const { done, value } = await reader.read();
          if (done) break;
          size += value.byteLength;
          if (size > FONT_PACK.size) throw new Error("font pack download exceeds its pinned size");
          for (let offset = 0; offset < value.byteLength; offset += 65536) {
            check();
            await writer.write(value.subarray(offset, offset + 65536));
          }
        }
        if (size !== FONT_PACK.size) throw new Error("font pack download is truncated");
        await writer.close();
        const blob = await handle.getFile();
        if (!await accept({ kind: "blob", blob, size })) throw new Error("font pack download checksum mismatch");
      } catch (error) {
        await reader.cancel().catch(() => {});
        await writer.abort().catch(() => {});
        await directory.removeEntry(name).catch(() => {});
        throw error;
      } finally { reader.releaseLock(); }
    });
  } finally { clearInterval(timer); controller.abort(); }
}

// SPDX-License-Identifier: MIT

// Stage or verify the immutable resource using bounded reads/writes.
import { createReadStream, createWriteStream } from "node:fs";
import { copyFile, rename, rm, stat } from "node:fs/promises";
import { createHash } from "node:crypto";
import { pipeline } from "node:stream/promises";
import { FONT_PACK } from "../internal/font-pack.mjs";
const target = new URL("../caj2pdf-fonts-v1.bin", import.meta.url);

export async function verify(path = target) {
  if ((await stat(path)).size !== FONT_PACK.size) throw new Error("font pack size mismatch");
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  if (hash.digest("hex") !== FONT_PACK.sha256) throw new Error("font pack checksum mismatch");
}

export async function stage() {
  if (process.env.CAJ2PDF_FONT_PACK) {
    await copyFile(process.env.CAJ2PDF_FONT_PACK, target);
    await verify();
    return;
  }
  try { await verify(); return; } catch {}
  const temporary = new URL("../caj2pdf-fonts-v1.bin.download", import.meta.url);
  try {
    const response = await fetch(FONT_PACK.url);
    if (!response.ok || !response.body) throw new Error(`font download failed (${response.status})`);
    let size = 0;
    await pipeline(response.body, async function* (stream) {
      for await (const bytes of stream) {
        size += bytes.byteLength;
        if (size > FONT_PACK.size) throw new Error("font download exceeds pinned size");
        yield bytes;
      }
    }, createWriteStream(temporary, { mode: 0o644 }));
    await verify(temporary);
    await rename(temporary, target);
  } finally { await rm(temporary, { force: true }); }
}

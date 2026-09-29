// SPDX-License-Identifier: MIT

import { syncAccessHandleScratch } from "../browser.mjs";

async function run() {
  const root = await navigator.storage.getDirectory();
  const name = `caj2pdf-scratch-test-${crypto.randomUUID()}`;
  let handle;
  let reopened;
  try {
    const file = await root.getFileHandle(name, { create: true });
    handle = await file.createSyncAccessHandle();
    const store = syncAccessHandleScratch(handle, { maxBytes: 32n });
    await store.resize(8n);
    await store.writeAt(4n, new Uint8Array([7, 8]));
    await store.writeAt(0n, new Uint8Array([1, 2]));
    const immediate = [...await store.readAt(0n, 8)];
    let rejected = false;
    try { await store.resize(33n); } catch (error) { rejected = error instanceof RangeError; }
    await store.flush();
    const visible = [...new Uint8Array(await (await file.getFile()).arrayBuffer())];
    await store.resize(0n);
    await store.resize(16n);
    const reused = [...await store.readAt(0n, 16)];
    const size = String(store.size);
    handle.close();
    handle = undefined;
    reopened = await file.createSyncAccessHandle();
    const afterClose = reopened.getSize();
    reopened.close();
    reopened = undefined;
    return { immediate, visible, reused, size, afterClose, rejected };
  } finally {
    try { handle?.close(); } finally {
      try { reopened?.close(); } finally { await root.removeEntry(name); }
    }
  }
}

try {
  const before = [];
  const root = await navigator.storage.getDirectory();
  for await (const name of root.keys()) before.push(name);
  const result = await run();
  const after = [];
  for await (const name of root.keys()) after.push(name);
  postMessage({ ...result, cleaned: JSON.stringify(before.sort()) === JSON.stringify(after.sort()) });
} catch (error) {
  postMessage({ error: `${error.name}: ${error.message}` });
}

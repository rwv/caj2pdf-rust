// SPDX-License-Identifier: MIT
// Compile against actual Dedicated Worker platform declarations.
import { syncAccessHandleScratch } from '../../browser.mjs';

export async function useWorkerScratch(file: FileSystemFileHandle) {
  const handle = await file.createSyncAccessHandle();
  try {
    const scratch = syncAccessHandleScratch(handle, { maxBytes: 1024n });
    await scratch.resize(8n);
    await scratch.writeAt(0n, new Uint8Array([1, 2]));
    await scratch.flush();
    return await scratch.readAt(0n, 2);
  } finally {
    handle.close();
  }
}

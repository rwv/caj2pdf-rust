// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { blobSource, convert, convertReadableStream, loadModule, spoolToOpfs, syncAccessHandleScratch, webWritableSink } from '../../browser.mjs';

export async function useBrowser(file: File, destination: WritableStream<Uint8Array>, signal: AbortSignal) {
  const module = await loadModule();
  const writer = destination.getWriter();
  const sink = webWritableSink(writer);
  const report = await convert(module, blobSource(file), sink, { signal, limits: { maxInputBytes: file.size } });
  const count: bigint = report.outputBytesWritten;
  await convertReadableStream(module, file.stream(), sink, { signal, maxSpoolBytes: BigInt(file.size) });
  const spool = await spoolToOpfs(file.stream(), { maxBytes: BigInt(file.size), signal });
  await spool.dispose();
  await writer.close();
  return count;
}

export async function useBrowserScratch(handle: import('../../browser.mjs').ScratchAccessHandle) {
  const scratch = syncAccessHandleScratch(handle, { maxBytes: 1024n });
  await scratch.resize(8n);
  const written: number = await scratch.writeAt(0n, new Uint8Array([1]));
  const read: Uint8Array = await scratch.readAt(0n, written);
  await scratch.flush();
  return { size: scratch.size, read };
}

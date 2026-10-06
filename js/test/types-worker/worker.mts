// SPDX-License-Identifier: MIT
// Compile against actual Dedicated Worker platform declarations: the API
// also runs from a caller's own Worker.
import { convert, inspect, loadModule, webWritableSink } from '../../browser.mjs';

export async function useFromWorker(file: FileSystemFileHandle, blob: Blob, destination: WritableStream<Uint8Array>) {
  const module = await loadModule();
  const writer = destination.getWriter();
  const info = await inspect(module, file);
  const report = await convert(module, blob, webWritableSink(writer), { includeBookmarks: false });
  await writer.close();
  return { pages: info.pageCount, bytes: report.outputBytesWritten };
}

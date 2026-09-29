// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { blobSource, convert, convertReadableStream, loadModule, spoolToOpfs, webWritableSink } from '../../browser.mjs';

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

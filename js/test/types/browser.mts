// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { convert, convertReadableStream, inspect, loadModule, spoolToOpfs, webWritableSink } from '../../browser.mjs';

export async function useBrowser(file: File, opfs: FileSystemFileHandle, destination: WritableStream<Uint8Array>, signal: AbortSignal) {
  const module = await loadModule();
  const writer = destination.getWriter();
  const sink = webWritableSink(writer);
  const report = await convert(module, file, sink, {
    signal,
    limits: { maxInputBytes: file.size },
    progress: (fraction: number) => void fraction,
  });
  const count: bigint = report.outputBytesWritten;
  const info = await inspect(module, opfs, { signal });
  const pages: number | null = info.pageCount;
  await convertReadableStream(module, file.stream(), sink, { signal, maxSpoolBytes: BigInt(file.size) });
  const spool = await spoolToOpfs(file.stream(), { maxBytes: BigInt(file.size), signal });
  const spooled: FileSystemFileHandle = spool.source;
  await convert(module, spooled, sink);
  await spool.dispose();
  await writer.close();
  return { count, pages };
}

export function nativeC8Fonts(
  wasm: import('../../browser.mjs').WasmInput,
  source: import('../../browser.mjs').BrowserInput,
  sink: import('../../browser.mjs').SequentialSink,
  font: Blob,
) {
  return convert(wasm, source, sink, { includeBookmarks: false, hnc8: {
    fonts: { cjk: font, latin: { source: font, face: 1 }, decoration: { source: font, character: 'A' } },
  } });
}

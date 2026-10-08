// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { openAsBlob } from 'node:fs';
import { open as openHandle } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { convert, convertReadable, inspect, loadModule, nodeWritableSink, spoolToTempFile } from '../../node.mjs';

export async function useNode(inputPath: string, outputPath: string) {
  const module = await loadModule();
  const output = (await openHandle(outputPath, 'wx')).createWriteStream();
  const sink = nodeWritableSink(output);
  const report = await convert(module, inputPath, sink, { progress: (fraction) => void fraction });
  const count: bigint = report.outputBytesWritten;
  const skippedBookmarks: number = report.outlineWarnings;
  const omittedOutline: boolean = report.outlineOmitted;
  const substitutions: bigint = report.substitutedGlyphs;
  void skippedBookmarks;
  void omittedOutline;
  const input = await openHandle(inputPath, 'r');
  const info = await inspect(module, input.fd);
  const pages: number | null = info.pageCount;
  await convert(module, await openAsBlob(inputPath), sink);
  await convert(module, new URL(`file://${inputPath}`), sink);
  await convertReadable(module, Readable.from([]), sink, { maxSpoolBytes: 1024n });
  const spool = await spoolToTempFile(Readable.from([]), { maxBytes: 1024n });
  const path: string = spool.source;
  await spool.dispose();
  return { count, pages, path };
}

export function nativeC8Fonts(
  wasm: import('../../node.mjs').WasmInput,
  source: import('../../node.mjs').NodeInput,
  sink: import('../../node.mjs').SequentialSink,
  font: string,
) {
  return convert(wasm, source, sink, { includeBookmarks: false, hnc8: {
    fonts: { cjk: font, latin: { source: font, face: 1 }, alternateLatin: font, symbols: font, latinState3: font, latinState28: font, latinState31: font, decoration: { source: font, character: 'A' } },
  } });
}

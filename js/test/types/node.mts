// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { open } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { convert, convertReadable, fileHandleSource, fileHandleScratch, loadModule, nodeWritableSink, spoolToTempFile } from '../../node.mjs';

export async function useNode(inputPath: string, outputPath: string) {
  const module = await loadModule();
  const input = await open(inputPath, 'r');
  const output = (await open(outputPath, 'wx')).createWriteStream();
  const sink = nodeWritableSink(output);
  const report = await convert(module, await fileHandleSource(input), sink);
  const count: bigint = report.outputBytesWritten;
  await convertReadable(module, Readable.from([]), sink, { maxSpoolBytes: 1024n });
  const spool = await spoolToTempFile(Readable.from([]), { maxBytes: 1024n });
  await spool.dispose();
  return count;
}

export async function useNodeScratch(handle: import('node:fs/promises').FileHandle) {
  const scratch = await fileHandleScratch(handle, { maxBytes: 1024n });
  await scratch.resize(8n);
  const written: number = await scratch.writeAt(0n, new Uint8Array([1]));
  const read: Uint8Array = await scratch.readAt(0n, written);
  await scratch.flush();
  return { size: scratch.size, read };
}

export function useHn(
  wasm: import('../../node.mjs').WasmInput,
  source: import('../../node.mjs').RangedSource,
  sink: import('../../node.mjs').SequentialSink,
  hnc8: import('../../node.mjs').Hnc8Options,
) {
  return convert(wasm, source, sink, { hnc8, includeBookmarks: false });
}

export function nativeC8Fonts(
  wasm: import('../../node.mjs').WasmInput,
  source: import('../../node.mjs').RangedSource,
  sink: import('../../node.mjs').SequentialSink,
  font: import('../../node.mjs').RangedSource,
) {
  return convert(wasm, source, sink, { includeBookmarks: false, hnc8: {
    fonts: { cjk: font, latin: font, alternateLatin: font, symbols: font, decoration: { source: font, character: 'A' } },
  } });
}

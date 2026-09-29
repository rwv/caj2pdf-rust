// SPDX-License-Identifier: MIT
// Compile-only consumer check; this function is never executed.
import { open } from 'node:fs/promises';
import { Readable } from 'node:stream';
import { convert, convertReadable, fileHandleSource, loadModule, nodeWritableSink, spoolToTempFile } from '../../node.mjs';

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

// SPDX-License-Identifier: MIT

// Usage: node js/examples/node.mjs INPUT OUTPUT_COPY
// This is an I/O proof that copies bytes. It does not convert to PDF.
import { createWriteStream } from "node:fs";
import { open, readFile } from "node:fs/promises";
import { finished } from "node:stream/promises";
import { copyRangeProof } from "../io.mjs";
import { fileHandleSource, nodeWritableSink } from "../node.mjs";

const [inputPath, outputPath] = process.argv.slice(2);
if (!inputPath || !outputPath) {
  process.stderr.write("Usage: node js/examples/node.mjs INPUT OUTPUT_COPY\n");
  process.exitCode = 2;
} else {
  const handle = await open(inputPath, "r");
  const output = createWriteStream(outputPath, { flags: "wx" });
  try {
    const wasmUrl = new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url);
    const wasmBytes = await readFile(wasmUrl);
    const { instance } = await WebAssembly.instantiate(wasmBytes);
    const report = await copyRangeProof(
      instance,
      await fileHandleSource(handle),
      nodeWritableSink(output),
    );
    output.end();
    await finished(output);
    process.stdout.write(`Copied ${report.outputBytesWritten} bytes through bounded I/O.\n`);
  } finally {
    output.destroy();
    await handle.close();
  }
}

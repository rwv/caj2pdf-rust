// SPDX-License-Identifier: MIT

// Usage: node js/examples/node.mjs INPUT|- OUTPUT.pdf
// Converts a PDF, CAJ, or KDH file (or standard input, spooled to a bounded
// temporary file) to a new PDF. Build the WASM module first; see js/README.md.
import { open, rm } from "node:fs/promises";
import { finished } from "node:stream/promises";
import { convert, convertReadable, fileHandleSource, loadModule, nodeWritableSink } from "../node.mjs";

const [inputPath, outputPath] = process.argv.slice(2);
if (!inputPath || !outputPath) {
  process.stderr.write("Usage: node js/examples/node.mjs INPUT|- OUTPUT.pdf\n");
  process.exitCode = 2;
} else {
  const module = await loadModule(
    new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url),
  );
  const controller = new AbortController();
  process.once("SIGINT", () => controller.abort());
  const options = { signal: controller.signal };
  // "wx" never replaces an existing file; a failed conversion removes it.
  const output = (await open(outputPath, "wx")).createWriteStream();
  const input = inputPath === "-" ? null : await open(inputPath, "r");
  try {
    const sink = nodeWritableSink(output);
    const report = input == null
      ? await convertReadable(module, process.stdin, sink, options)
      : await convert(module, await fileHandleSource(input), sink, options);
    output.end();
    await finished(output);
    process.stdout.write(
      `Converted ${report.format.toUpperCase()}: ${report.pagesConverted} pages, ` +
        `${report.outputBytesWritten} bytes.\n`,
    );
  } catch (error) {
    output.destroy();
    await rm(outputPath, { force: true });
    process.stderr.write(`${error.code ?? error.name}: ${error.message}\n`);
    process.exitCode = 1;
  } finally {
    await input?.close();
  }
}

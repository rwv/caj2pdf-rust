// SPDX-License-Identifier: MIT

// Usage: node js/examples/node.mjs INPUT|- OUTPUT.pdf [--no-bookmarks]
// Converts a PDF, CAJ, KDH, HN, or C8 file (or standard input, spooled to a bounded
// temporary file) to a new PDF. Build the WASM module first; see js/README.md.
import { open, rm } from "node:fs/promises";
import { finished } from "node:stream/promises";
import { convert, convertReadable, fileHandleSource, loadModule, nodeWritableSink, withHnc8Scratch } from "../node.mjs";

const [inputPath, outputPath, flag, extra] = process.argv.slice(2);
if (!inputPath || !outputPath || extra !== undefined || (flag !== undefined && flag !== "--no-bookmarks")) {
  process.stderr.write("Usage: node js/examples/node.mjs INPUT|- OUTPUT.pdf [--no-bookmarks]\n");
  process.exitCode = 2;
} else {
  const controller = new AbortController();
  const abort = () => controller.abort();
  process.once("SIGINT", abort);
  const options = { signal: controller.signal, includeBookmarks: flag !== "--no-bookmarks" };
  let input;
  let output;
  try {
    const module = await loadModule(
      new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url),
    );
    input = inputPath === "-" ? null : await open(inputPath, "r");
    // "wx" never replaces an existing file. Only an output we successfully
    // opened belongs to this invocation and may be removed after failure.
    output = (await open(outputPath, "wx")).createWriteStream();
    const sink = nodeWritableSink(output);
    const report = await withHnc8Scratch(async (scratch) => {
      const conversionOptions = { ...options, hnc8: { scratch } };
      return input == null
        ? convertReadable(module, process.stdin, sink, conversionOptions)
        : convert(module, await fileHandleSource(input), sink, conversionOptions);
    });
    output.end();
    await finished(output);
    process.stdout.write(
      `Converted ${report.format.toUpperCase()}: ${report.pagesConverted} pages, ` +
        `${report.outputBytesWritten} bytes.\n`,
    );
  } catch (error) {
    if (output) {
      output.destroy();
      await finished(output).catch(() => {});
      await rm(outputPath, { force: true });
    }
    process.stderr.write(`${error.code ?? error.name}: ${error.message}\n`);
    process.exitCode = 1;
  } finally {
    process.off("SIGINT", abort);
    await input?.close();
  }
}

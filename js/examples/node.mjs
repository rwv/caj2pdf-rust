// SPDX-License-Identifier: MIT

// Usage: node js/examples/node.mjs INPUT|- OUTPUT.pdf [--no-bookmarks]
// Converts a PDF, CAJ, KDH, HN, or C8 file (or standard input, spooled to a bounded
// temporary file) to a new PDF. Build the WASM module first; see js/README.md.
import { open, rm } from "node:fs/promises";
import { finished } from "node:stream/promises";
import { convert, convertReadable, loadModule, nodeWritableSink } from "../node.mjs";

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
    const module = await loadModule();
    // Open the input first so a missing file never creates the output.
    input = inputPath === "-" ? null : await open(inputPath, "r");
    // "wx" never replaces an existing file. Only an output we successfully
    // opened belongs to this invocation and may be removed after failure.
    output = (await open(outputPath, "wx")).createWriteStream();
    const sink = nodeWritableSink(output);
    // The conversion runs in a Worker that reads the descriptor directly.
    const report = input == null
      ? await convertReadable(module, process.stdin, sink, options)
      : await convert(module, input.fd, sink, options);
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

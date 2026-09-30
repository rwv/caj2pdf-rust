// SPDX-License-Identifier: MIT

import { blobSource, convert, loadModule, webWritableSink, withHnc8Scratch } from "../browser.mjs";

const controller = new AbortController();
self.onmessage = async ({ data }) => {
  if (data.cancel) {
    controller.abort();
    return;
  }
  const writer = data.output.getWriter();
  try {
    const module = await loadModule(
      new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url),
    );
    const report = await withHnc8Scratch((scratch) =>
      convert(module, blobSource(data.file), webWritableSink(writer), {
        signal: controller.signal,
        includeBookmarks: data.includeBookmarks,
        hnc8: { scratch },
      }),
    );
    await writer.close();
    self.postMessage({ report });
  } catch (error) {
    await writer.abort().catch(() => {});
    self.postMessage({ error: { name: error.name, code: error.code, message: error.message } });
  } finally {
    writer.releaseLock();
    self.close();
  }
};

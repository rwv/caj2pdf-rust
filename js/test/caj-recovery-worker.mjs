// SPDX-License-Identifier: MIT

import { convertFile, reject } from "./browser-cases.mjs";

try {
  const positive = await convertFile("later-copy.caj");
  const negative = await reject("broken-later-copy.caj");
  const ascii85 = await convertFile("ascii85.caj");
  const cleanAscii85 = await convertFile("ascii85-clean.caj");
  const brokenAscii85 = await reject("ascii85-broken.caj");
  self.postMessage({ positive, negative, ascii85, cleanAscii85, brokenAscii85 });
} catch (error) {
  self.postMessage({ error: String(error?.stack ?? error) });
}

// SPDX-License-Identifier: MIT

import { convertFile, reject } from "./browser-cases.mjs";

try {
  const positive = await convertFile("later-copy.caj");
  const negative = await reject("broken-later-copy.caj");
  self.postMessage({ positive, negative });
} catch (error) {
  self.postMessage({ error: String(error?.stack ?? error) });
}

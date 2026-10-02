// SPDX-License-Identifier: MIT

import { convertFile, reject } from "./browser-cases.mjs";

try {
  const positive = await convertFile("later-copy.caj");
  const negative = await reject("broken-later-copy.caj");
  const ascii85 = await convertFile("ascii85.caj");
  const keywordCut = await convertFile("keyword-cut.caj");
  const cleanAscii85 = await convertFile("ascii85-clean.caj");
  const brokenAscii85 = await reject("ascii85-broken.caj");
  const scalarReplay = await convertFile("scalar-replay.caj");
  const scalarClean = await convertFile("scalar-clean.caj");
  const scalarBroken = await reject("scalar-broken.caj");
  self.postMessage({ scalarReplay, scalarClean, scalarBroken, positive, negative, ascii85, keywordCut, cleanAscii85, brokenAscii85 });
} catch (error) {
  const scalarReplay = await convertFile("scalar-replay.caj");
  const scalarClean = await convertFile("scalar-clean.caj");
  const scalarBroken = await reject("scalar-broken.caj");
  self.postMessage({ scalarReplay, scalarClean, scalarBroken, error: String(error?.stack ?? error) });
}

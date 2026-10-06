// SPDX-License-Identifier: MIT

import { convertFile, reject } from "./browser-cases.mjs";

try {
  const positive = await convertFile("later-copy.caj");
  const negative = await reject("broken-later-copy.caj");
  const ascii85 = await convertFile("ascii85.caj");
  const referenceCut = await convertFile("reference-cut.caj");
  const keywordCut = await convertFile("keyword-cut.caj");
  const cleanAscii85 = await convertFile("ascii85-clean.caj");
  const brokenAscii85 = await convertFile("ascii85-broken.caj");
  const brokenAscii85Clean = await convertFile("ascii85-broken-clean.caj");
  const scalarReplay = await convertFile("scalar-replay.caj");
  const scalarClean = await convertFile("scalar-clean.caj");
  const scalarBroken = await convertFile("scalar-broken.caj");
  const scalarBrokenClean = await convertFile("scalar-broken-clean.caj");
  const adjacentFlate = await convertFile("adjacent-flate.caj");
  const adjacentClean = await convertFile("adjacent-flate-clean.caj");
  const arrayReplay = await convertFile("array-replay.caj");
  const arrayClean = await convertFile("array-clean.caj");
  const deferredReplay = await convertFile("deferred-replay.caj");
  const deferredClean = await convertFile("deferred-clean.caj");
  const deferredBroken = await convertFile("deferred-broken.caj");
  const deferredBrokenClean = await convertFile("deferred-broken-clean.caj");
  self.postMessage({
    deferredReplay, deferredClean, deferredBroken, deferredBrokenClean, adjacentFlate, adjacentClean,
    arrayReplay, arrayClean, scalarReplay, scalarClean, scalarBroken, scalarBrokenClean, positive,
    negative, ascii85, keywordCut, referenceCut, cleanAscii85, brokenAscii85, brokenAscii85Clean,
  });
} catch (error) {
  self.postMessage({ error: String(error?.stack ?? error) });
}

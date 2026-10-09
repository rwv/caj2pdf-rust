// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";
import { test } from "node:test";

test("Worker dependency graph loads synchronously and handles a queued first message", { timeout: 10_000 }, async (t) => {
  // require(esm) refuses top-level await anywhere in the static module graph.
  // This guards IIFE bundling without adding a parser or bundler dependency.
  const gate = new Int32Array(new SharedArrayBuffer(4));
  const worker = new Worker(`
    const { workerData } = require("node:worker_threads");
    Atomics.wait(new Int32Array(workerData.gate), 0, 0);
    require(workerData.path);
  `, {
    eval: true,
    workerData: { path: fileURLToPath(new URL("../internal/worker.mjs", import.meta.url)), gate: gate.buffer },
    execArgv: ["--experimental-require-module"],
  });
  try {
    const response = once(worker, "message", { signal: t.signal });
    // Queue before the Worker has loaded or attached its one-shot listener.
    // An invalid input kind exercises the normal failure response without WASM.
    worker.postMessage({ inputs: [{ kind: "invalid-startup-control" }] });
    Atomics.store(gate, 0, 1);
    Atomics.notify(gate, 0);
    const [message] = await response;
    assert.deepEqual(message, {
      type: "done",
      failure: { name: "TypeError", message: "unsupported input kind: invalid-startup-control" },
    });
  } finally {
    await worker.terminate();
  }
});

// SPDX-License-Identifier: MIT

// Install the production module's message handler, then force the scheduling
// boundary between its CANCEL check and its first ACKS wait. Test-only Worker.
import "../internal/worker.mjs";

const wait = Atomics.wait;
let armed = true;
Atomics.wait = (control, index, expected, ...options) => {
  if (!armed || index !== 1) return wait(control, index, expected, ...options);
  armed = false;
  const gate = new Int32Array(new SharedArrayBuffer(4));
  self.postMessage({ type: "test-before-ack-wait", gate: gate.buffer });
  wait(gate, 0, 0, 5000);
  const result = wait(control, index, expected, ...options);
  self.postMessage({ type: "test-after-ack-wait", result });
  return result;
};

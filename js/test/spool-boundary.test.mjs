// SPDX-License-Identifier: MIT

// Original small stream/write controls. No WASM, filesystem fault, or corpus
// input is needed to exercise these owned spooling boundaries.
import assert from "node:assert/strict";
import { test } from "node:test";
import { pumpChunks } from "../io.mjs";
import { writeSpoolChunk } from "../internal/spool-write.mjs";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function assertUnlocked(stream) {
  assert.equal(stream.locked, false);
  const reader = stream.getReader();
  // An errored stream rejects this owned reader's closed promise.
  reader.closed.catch(() => {});
  reader.releaseLock();
  assert.equal(stream.locked, false, "a subsequent reader can be acquired and released");
}

test("owned Web readers are released on every pump outcome", async (t) => {
  await t.test("EOF", async () => {
    let cancelled = false;
    const stream = new ReadableStream({
      start(controller) {
        controller.enqueue(Uint8Array.of(1, 2, 3));
        controller.close();
      },
      cancel() { cancelled = true; },
    });
    const chunks = [];
    assert.equal(await pumpChunks(stream, async (chunk) => chunks.push([...chunk]), { maxBytes: 3n }), 3n);
    assert.deepEqual(chunks, [[1, 2, 3]]);
    assert.equal(cancelled, false, "successful EOF does not cancel the producer");
    assertUnlocked(stream);
  });

  await t.test("producer error", async () => {
    const failure = new Error("original producer failure");
    const stream = new ReadableStream({ pull() { throw failure; } });
    await assert.rejects(pumpChunks(stream, async () => assert.fail("must not consume"), { maxBytes: 3n }),
      (error) => error === failure);
    assertUnlocked(stream);
  });

  await t.test("sink error", async () => {
    const failure = new Error("original sink failure");
    const stream = new ReadableStream({ start(controller) { controller.enqueue(Uint8Array.of(1)); } });
    await assert.rejects(pumpChunks(stream, async () => { throw failure; }, { maxBytes: 3n }),
      (error) => error === failure);
    assertUnlocked(stream);
  });

  await t.test("byte limit", async () => {
    const stream = new ReadableStream({ start(controller) { controller.enqueue(Uint8Array.of(1, 2)); } });
    await assert.rejects(pumpChunks(stream, async () => assert.fail("oversized chunk must not reach the sink"),
      { maxBytes: 1n }), { code: "LIMIT_EXCEEDED" });
    assertUnlocked(stream);
  });

  await t.test("abort during a pending read", async () => {
    const reading = deferred();
    const controller = new AbortController();
    const reason = new Error("original read abort");
    const stream = new ReadableStream({ pull() { reading.resolve(); } });
    const pending = pumpChunks(stream, async () => assert.fail("must not consume"),
      { maxBytes: 3n, signal: controller.signal });
    await reading.promise;
    controller.abort(reason);
    await assert.rejects(pending, (error) => error === reason);
    assertUnlocked(stream);
  });
});

test("rejected or stalled cancellation starts before release and preserves the primary failure", { timeout: 5000 }, async (t) => {
  for (const outcome of ["rejected", "stalled"]) {
    await t.test(outcome, async () => {
      const cancellation = deferred();
      const failure = new Error("primary consume failure");
      const events = [];
      let stream;
      stream = new ReadableStream({
        start(controller) { controller.enqueue(Uint8Array.of(1)); },
        cancel() {
          events.push({ operation: "cancel", locked: stream.locked });
          if (outcome === "rejected") cancellation.reject(new Error("secondary cancellation failure"));
          return cancellation.promise;
        },
      });
      await assert.rejects(pumpChunks(stream, async () => { throw failure; }, { maxBytes: 1n }),
        (error) => error === failure);
      assert.deepEqual(events, [{ operation: "cancel", locked: true }]);
      assertUnlocked(stream);
      // A stalled cancel has still not settled when pumping has rejected and
      // a new reader has been acquired. Finish it only after those assertions.
      if (outcome === "stalled") cancellation.resolve();
    });
  }
});

test("temporary-file partial writes stay awaited and ordered", async () => {
  const first = deferred();
  const entered = deferred();
  const calls = [];
  const chunk = Uint8Array.of(10, 11, 12, 13);
  const handle = {
    write(bytes, offset, length, position) {
      assert.equal(bytes, chunk);
      calls.push({ offset, length, position });
      if (calls.length === 1) {
        entered.resolve();
        return first.promise;
      }
      return Promise.resolve({ bytesWritten: 1 });
    },
  };
  const pending = writeSpoolChunk(handle, chunk, 7);
  await entered.promise;
  assert.equal(calls.length, 1, "no next write while the first is pending");
  first.resolve({ bytesWritten: 2 });
  assert.equal(await pending, 11);
  assert.deepEqual(calls, [
    { offset: 0, length: 4, position: 7 },
    { offset: 2, length: 2, position: 9 },
    { offset: 3, length: 1, position: 10 },
  ]);
});

test("temporary-file invalid write progress rejects without another write", async (t) => {
  for (const [name, result] of [
    ["zero", { bytesWritten: 0 }],
    ["negative", { bytesWritten: -1 }],
    ["fractional", { bytesWritten: 0.5 }],
    ["missing count", {}],
    ["missing result", undefined],
    ["unsafe integer", { bytesWritten: Number.MAX_SAFE_INTEGER + 1 }],
    ["overreported", { bytesWritten: 5 }],
  ]) {
    await t.test(name, async () => {
      let calls = 0;
      const handle = { async write() { calls += 1; return result; } };
      await assert.rejects(writeSpoolChunk(handle, Uint8Array.of(1, 2, 3, 4), 0),
        { name: "RangeError", message: "FileHandle returned an invalid write count" });
      assert.equal(calls, 1);
    });
  }
});

test("temporary-file abort stops writes and preserves its reason after either settlement", async (t) => {
  await t.test("already aborted", async () => {
    const controller = new AbortController();
    const reason = new Error("original early abort");
    controller.abort(reason);
    const handle = { write() { assert.fail("must not initiate a write"); } };
    await assert.rejects(writeSpoolChunk(handle, Uint8Array.of(1), 0, controller.signal),
      (error) => error === reason);
  });
  for (const outcome of ["partial write", "write error"]) {
    await t.test(outcome, async () => {
      const response = deferred();
      const controller = new AbortController();
      const reason = new Error("original pending-write abort");
      let calls = 0;
      const handle = { write() { calls += 1; return response.promise; } };
      const pending = writeSpoolChunk(handle, Uint8Array.of(1, 2), 0, controller.signal);
      assert.equal(calls, 1);
      controller.abort(reason);
      if (outcome === "partial write") response.resolve({ bytesWritten: 1 });
      else response.reject(new Error("secondary write failure"));
      await assert.rejects(pending, (error) => error === reason);
      assert.equal(calls, 1, "no following write after the pending operation settles");
    });
  }
});

test("temporary-file write errors preserve the original error without an abort", async () => {
  const failure = new Error("original file write failure");
  const handle = { async write() { throw failure; } };
  await assert.rejects(writeSpoolChunk(handle, Uint8Array.of(1), 0), (error) => error === failure);
});

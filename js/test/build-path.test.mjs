// SPDX-License-Identifier: MIT

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { test } from "node:test";

const run = promisify(execFile);

test("WASM copy follows Cargo target-directory settings instead of stale default artifacts", async () => {
  const root = await mkdtemp(join(tmpdir(), "caj2pdf-build-path-"));
  try {
    await mkdir(join(root, "js", "scripts"), { recursive: true });
    await mkdir(join(root, ".cargo"));
    await writeFile(join(root, "Cargo.toml"), '[workspace]\n[package]\nname="build-path-test"\nversion="0.0.0"\n[lib]\npath="lib.rs"\n');
    await writeFile(join(root, "lib.rs"), "");
    await cp(new URL("../scripts/copy-wasm.mjs", import.meta.url), join(root, "js", "scripts", "copy-wasm.mjs"));
    const stale = Buffer.from([0, 97, 115, 109, 1, 0, 0, 0]);
    const current = Buffer.concat([stale, Buffer.from([0, 1, 0])]);
    for (const [directory, bytes] of [["target", stale], ["custom-target", current]]) {
      const release = join(root, directory, "wasm32-unknown-unknown", "release");
      await mkdir(release, { recursive: true });
      await writeFile(join(release, "caj2pdf_wasm.wasm"), bytes);
    }
    const env = { ...process.env, CARGO_TARGET_DIR: join(root, "custom-target") };
    const script = join(root, "js", "scripts", "copy-wasm.mjs");
    await run(process.execPath, [script], { cwd: root, env });
    assert.deepEqual(await readFile(join(root, "js", "caj2pdf_wasm.wasm")), current);
    delete env.CARGO_TARGET_DIR;
    await writeFile(join(root, ".cargo", "config.toml"), '[build]\ntarget-dir="custom-target"\n');
    await run(process.execPath, [script], { cwd: root, env });
    assert.deepEqual(await readFile(join(root, "js", "caj2pdf_wasm.wasm")), current);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

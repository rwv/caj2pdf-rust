// SPDX-License-Identifier: MIT

// Copy the release WASM build next to the package entry points, where
// `loadModule()` looks by default. Usage: node scripts/copy-wasm.mjs [SOURCE]
import { chmodSync, copyFileSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const source = process.argv[2] ??
  join(JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version=1"], {
    cwd: fileURLToPath(new URL("..", import.meta.url)),
    encoding: "utf8",
  })).target_directory, "wasm32-unknown-unknown", "release", "caj2pdf_wasm.wasm");
const target = fileURLToPath(new URL("../caj2pdf_wasm.wasm", import.meta.url));
const magic = readFileSync(source).subarray(0, 4);
if (!magic.equals(Buffer.from([0, 0x61, 0x73, 0x6d]))) {
  throw new Error(`${source} is not a WebAssembly module`);
}
copyFileSync(source, target);
chmodSync(target, 0o644);
console.log(`copied ${source} -> ${target}`);

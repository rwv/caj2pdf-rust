// SPDX-License-Identifier: MIT

/** Shared synthetic inputs and PDF checks for the JavaScript tests. */
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { constants as zlibConstants, deflateSync, inflateSync } from "node:zlib";

export const wasmUrl = new URL("../../target/wasm32-unknown-unknown/release/caj2pdf_wasm.wasm", import.meta.url);

let compiled;
/** One compiled module; each `convert(module, ...)` gets a fresh instance. */
export async function wasmModule() {
  compiled ??= WebAssembly.compile(await readFile(wasmUrl));
  return compiled;
}

export async function fixture(name) {
  return new Uint8Array(await readFile(new URL(`../../tests/fixtures/${name}`, import.meta.url)));
}

/** A KDH wrapper (docs/research/kdh-format.md) around a repository-owned PDF fixture. */
export async function syntheticKdh(versionField = [0, 0, 2, 0]) {
  const pdf = await fixture("valid_out_of_order_objects.pdf");
  const wrapped = new Uint8Array(254 + pdf.length);
  wrapped.set(new TextEncoder().encode("KDH 2.00 Copyright(C) 2000 CAJCD"));
  wrapped.set(versionField, 0x28);
  const key = new TextEncoder().encode("FZHMEI");
  for (let index = 0; index < pdf.length; index += 1) {
    wrapped[254 + index] = pdf[index] ^ key[index % key.length];
  }
  return { pdf, wrapped };
}

/**
 * A two-page, one-bookmark CAJ (docs/research/caj-format.md). The page-tree root is
 * absent from the body, so the core must reconstruct it.
 */
export function syntheticCaj() {
  const text = new TextEncoder();
  const body = text.encode([
    "3 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 100] /Resources << >> >>\nendobj\n",
    "4 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 300 150] /Resources << >> >>\nendobj\n",
    "5 0 obj\n<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>\nendobj\n",
  ].join(""));
  const table = 0x400;
  const bodyStart = table + 2 * 12;
  const bytes = new Uint8Array(bodyStart + body.length);
  const view = new DataView(bytes.buffer);
  bytes.set(text.encode("CAJ\0"));
  view.setUint32(0x10, 2, true);
  view.setUint32(0x14, table, true);
  view.setUint32(0x110, 1, true);
  bytes.set(text.encode("Intro"), 0x114);
  bytes[0x114 + 280] = "1".charCodeAt(0);
  view.setUint32(0x114 + 304, 1, true);
  view.setUint32(table, bodyStart, true);
  view.setUint32(table + 4, body.length, true);
  view.setUint32(table + 8, 3, true);
  view.setUint32(table + 12, bodyStart + body.length, true);
  view.setUint32(table + 20, 4, true);
  bytes.set(body, bodyStart);
  return bytes;
}

/** Original visible-content control for length-derived ASCII85 replay. */
export function syntheticAscii85Caj({ interrupted = true, broken = false, cut = "payload" } = {}) {
  const base = syntheticCaj();
  const start = 0x400 + 24;
  const original = new TextDecoder().decode(base.subarray(start))
    .replace("/Resources << >> >>", "/Resources << >> /Contents 6 0 R >>");
  // ASCII85 for the authored blue rectangle: q 0 0 1 rg 10 20 30 40 re f Q.
  const payload = "E?HqX0H`(mEb?LL0H`,)+>Y\\o1b^%mAKYS-;$m~>";
  const header = "6 0 obj << /Length 7 0 R /Filter /ASCII85Decode >> stream\n";
  let prefix = header + payload.slice(0, 11) + "\n";
  if (cut === "reference") prefix = header.slice(0, header.indexOf(" R")) + "\n";
  if (cut === "keyword") prefix = header.slice(0, -3) + "\n";
  if (!interrupted) prefix = "";
  const body = new TextEncoder().encode(original + prefix + header + payload +
    `\nendstream\nendobj\n7 0 obj ${payload.length + (broken ? 1 : 0)} endobj\n`);
  const bytes = new Uint8Array(start + body.length);
  bytes.set(base.subarray(0, start));
  bytes.set(body, start);
  const view = new DataView(bytes.buffer);
  view.setUint32(0x404, body.length, true);
  view.setUint32(0x40c, bytes.length, true);
  return bytes;
}

/** Original direct-Length Flate replay, with an optional known-object anchor. */
export function syntheticFlateReplayCaj({ interrupted = true, broken = false, anchor = "scalar", padding = "" } = {}) {
  const base = syntheticCaj();
  const start = 0x400 + 24;
  const pageObjects = new TextDecoder().decode(base.subarray(start))
    .replace("/Resources << >> >>", "/Resources << >> /Contents 6 0 R >>");
  const payload = deflateSync("q 0 0 1 rg 10 20 30 40 re f Q\n%" + " ".repeat(1024) + "\n", { level: 0 });
  if (broken) payload[payload.length - 1] ^= 1;
  const prior = anchor === "deferred" ? `7 0 obj ${payload.length} endobj\n`
    : anchor === "scalar" ? "7 0 obj 91 endobj\n"
    : anchor === "array" ? "7 0 obj [13 29 47] endobj\n" : "";
  const length = anchor === "deferred" ? "7 0 R" : payload.length + padding.length;
  const header = `6 0 obj << /Length ${length} /Filter /FlateDecode >> stream\n`;
  const parts = [pageObjects, prior];
  if (interrupted) parts.push(header, payload.subarray(0, 12), "\n", prior);
  if (anchor === "deferred") {
    parts.push("8 0 obj [13 29 47] endobj\n");
    if (interrupted) parts.push("8 0 obj [13\n");
    parts.push("9 0 obj 42 endobj\n");
  }
  parts.push(header, payload, padding, "\nendstream\nendobj\n");
  const body = Buffer.concat(parts.map((part) => Buffer.from(part)));
  const bytes = new Uint8Array(start + body.length);
  bytes.set(base.subarray(0, start));
  bytes.set(body, start);
  const view = new DataView(bytes.buffer);
  view.setUint32(0x404, body.length, true);
  view.setUint32(0x40c, bytes.length, true);
  return bytes;
}

/** Original controls for known dictionary/array cuts and adjacent headers. */
export function syntheticRecoveredCaj(broken = false) {
  const base = syntheticCaj();
  const suffix = new TextEncoder().encode(
    "3 0 obj\n<< /Type /Page /Parent\n" +
    "10 0 obj<< /Box [1 3 9] >>endobj\n" +
    "10 0 obj<< /Box [\n11 0 obj null endobj\n" +
    "12 0 \r\n12 0 obj null endobj\n" +
    (broken ? "13 0 obj<< /Fail @ >>endobj\n" : ""),
  );
  const bytes = new Uint8Array(base.length + suffix.length);
  bytes.set(base);
  bytes.set(suffix, base.length);
  const view = new DataView(bytes.buffer);
  const table = view.getUint32(0x14, true);
  view.setUint32(table + 4, view.getUint32(table + 4, true) + suffix.length, true);
  view.setUint32(table + 12, bytes.length, true);
  return bytes;
}

/** Original later-copy control rooted in the second CAJ page-table span. */
export function syntheticLaterCopyCaj(broken = false) {
  const base = syntheticCaj();
  const text = new TextEncoder();
  const table = 0x400;
  const start = table + 24;
  const original = new TextDecoder().decode(base.subarray(start));
  const split = original.indexOf("4 0 obj");
  const first = text.encode(original.slice(0, split) + `7 0 obj\n<< /Type /Ex${broken ? "b" : "a"}\n` +
    "8 0 obj\n<< /Length 600 >>\nstream\nZZZZZ\n");
  const second = text.encode(original.slice(split) + "7 0 obj\n<< /Type /Example /Values [3 9] >>\nendobj\n" +
    `8 0 obj\n<< /Length 600 >>\nstream\n${"Z".repeat(600)}\nendstream\nendobj\n`);
  const bytes = new Uint8Array(start + first.length + second.length);
  bytes.set(base.subarray(0, start));
  bytes.set(first, start);
  bytes.set(second, start + first.length);
  const view = new DataView(bytes.buffer);
  view.setUint32(table + 4, first.length, true);
  view.setUint32(table + 12, start + first.length, true);
  view.setUint32(table + 16, second.length, true);
  return bytes;
}

/**
 * A valid one-page PDF whose content stream is about `streamBytes` long, as
 * Blob parts that repeat one small chunk (the test never builds one array).
 */
export function largePdfBlob(streamBytes) {
  const text = new TextEncoder();
  const line = text.encode("0 0 0 rg 10 10 40 20 re f\n".repeat(2048));
  const repeats = Math.ceil(streamBytes / line.length);
  const length = repeats * line.length;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>",
  ];
  const parts = [text.encode("%PDF-1.7\n%âãÏÓ\n")];
  let offset = parts[0].length;
  const offsets = [];
  for (const [index, dictionary] of objects.entries()) {
    offsets.push(offset);
    const part = text.encode(`${index + 1} 0 obj\n${dictionary}\nendobj\n`);
    parts.push(part);
    offset += part.length;
  }
  offsets.push(offset);
  const streamHead = text.encode(`4 0 obj\n<< /Length ${length} >>\nstream\n`);
  const streamTail = text.encode("\nendstream\nendobj\n");
  parts.push(streamHead);
  for (let index = 0; index < repeats; index += 1) parts.push(line);
  parts.push(streamTail);
  offset += streamHead.length + length + streamTail.length;
  const xref = ["xref\n0 5\n0000000000 65535 f \n"]
    .concat(offsets.map((value) => `${String(value).padStart(10, "0")} 00000 n \n`))
    .join("");
  parts.push(text.encode(`${xref}trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${offset}\n%%EOF\n`));
  return new Blob(parts);
}

/**
 * A Blob that records the largest range sliced from it. Node serves Blob
 * reads on the calling thread, so every Worker read goes through `slice`.
 */
export function trackedBlob(blob, record) {
  class Tracked extends Blob {
    slice(start, end) {
      record.maxRead = Math.max(record.maxRead ?? 0, end - start);
      return super.slice(start, end);
    }
  }
  return new Tracked([blob]);
}

/** Wrap a sink, recording the largest chunk it is offered. */
export function trackedSink(sink, record) {
  return {
    writeChunk(bytes, signal) {
      record.maxWrite = Math.max(record.maxWrite ?? 0, bytes.byteLength);
      record.writes = (record.writes ?? 0) + 1;
      return sink.writeChunk(bytes, signal);
    },
    flush(signal) {
      return sink.flush(signal);
    },
  };
}

/** A WritableStream writer that collects its chunks for inspection. */
export function collectingWriter() {
  const chunks = [];
  const writer = new WritableStream({
    write(chunk) {
      chunks.push(chunk);
    },
  }).getWriter();
  return { writer, bytes: () => new Uint8Array(Buffer.concat(chunks)) };
}

/** A sink that accepts and drops every chunk. */
export const discard = Object.freeze({
  async writeChunk(bytes) {
    return bytes.byteLength;
  },
  async flush() {},
});

export async function tempDirectory(prefix) {
  return mkdtemp(join(tmpdir(), `caj2pdf-js-${prefix}-`));
}

const run = promisify(execFile);
let qpdfAvailable;

export async function hasQpdf() {
  if (qpdfAvailable === undefined) {
    try {
      await run("qpdf", ["--version"]);
      qpdfAvailable = true;
    } catch {
      qpdfAvailable = false;
    }
  }
  return qpdfAvailable;
}

/**
 * Check the PDF framing, and when `qpdf` is installed, run `qpdf --check`
 * and compare the page count. Without qpdf that validation is reported as
 * skipped through the test's diagnostics.
 */
export async function validatePdf(t, bytes, pages) {
  const text = Buffer.from(bytes).toString("latin1");
  assert.ok(text.startsWith("%PDF-"), "output starts with a PDF header");
  assert.match(text.trimEnd(), /%%EOF$/, "output ends with %%EOF");
  if (!(await hasQpdf())) {
    // CI installs qpdf, so a missing validator there is a failure, not a skip.
    assert.ok(!process.env.CI, "qpdf is required in CI");
    t.diagnostic("qpdf is not installed; independent PDF validation skipped");
    return;
  }
  const directory = await tempDirectory("qpdf");
  try {
    const path = join(directory, "output.pdf");
    await writeFile(path, bytes);
    const checked = await run("qpdf", ["--check", path]);
    assert.match(checked.stdout, /No syntax or stream encoding errors found/);
    const counted = await run("qpdf", ["--show-npages", path]);
    assert.equal(Number(counted.stdout.trim()), pages);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** Independently decode the original asymmetric type-1 fixture's PDF image. */
export async function validateType1Hn(t, bytes) {
  await validatePdf(t, bytes, 1);
  const directory = await tempDirectory("hn-type1");
  try {
    const path = join(directory, "output.pdf");
    await writeFile(path, bytes);
    await run("pdfimages", ["-j", path, join(directory, "image")]);
    const { stdout } = await run("djpeg", [join(directory, "image-000.jpg")], { encoding: "buffer" });
    const pixels = Array.from({ length: 128 }, (_, i) => i % 16 < 8 ? 32 : 224);
    assert.deepEqual(stdout, Buffer.concat([Buffer.from("P5\n16 8\n255\n"), Buffer.from(pixels)]));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** Check the original two-image HN fixture, including draw order and geometry. */
export async function validateMultiImageHn(t, bytes) {
  await validatePdf(t, bytes, 1);
  const directory = await tempDirectory("hn-images");
  try {
    const path = join(directory, "output.pdf");
    await writeFile(path, bytes);
    const info = JSON.parse((await run("qpdf", ["--json", "--json-key=pages", "--json-key=outlines", path])).stdout);
    const page = info.pages[0];
    const pageObject = (await run("qpdf", [`--show-object=${page.object.split(" ")[0]}`, path])).stdout;
    const box = pageObject.match(/\/MediaBox\s*\[([^\]]+)\]/);
    assert.ok(box, "page has a MediaBox");
    const bounds = box[1].trim().split(/\s+/).map(Number);
    assert.equal(bounds.length, 4);
    [0, 0, 100 * 240 / 2473, 200 * 240 / 2473].forEach((value, axis) =>
      assert.ok(Math.abs(bounds[axis] - value) < 0.000001, `page boundary ${axis}`));
    assert.equal(page.images.length, 2);
    for (const [index, image] of page.images.entries()) {
      assert.equal(image.name, `/Im${index}`);
      assert.equal(image.width, 3); assert.equal(image.height, 2);
      const { stdout } = await run("qpdf", [`--show-object=${image.object.split(" ")[0]}`, "--filtered-stream-data", path], { encoding: "buffer" });
      // Rows stay top-first: 101 / 010 and 110 / 001.
      assert.deepEqual([...stdout], index === 0
        ? [0xa0, 0x40]
        : [0xc0, 0x20]);
    }
    const content = (await run("qpdf", [`--show-object=${page.contents[0].split(" ")[0]}`, "--filtered-stream-data", path])).stdout;
    const draws = [...content.matchAll(/([\d.e+\- ]+) cm\s+\/Im(\d+) Do/g)];
    assert.equal(draws.length, 2);
    const scale = 240 / 2473;
    for (let i = 0; i < 2; i++) {
      assert.equal(Number(draws[i][2]), i);
      const matrix = draws[i][1].trim().split(/\s+/).map(Number);
      // A positive height with the origin at the image's bottom edge.
      const height = (40 + i * 10) * scale;
      const expected = [(80 - i * 20) * scale, 0, 0, height, i * 13 * scale, 200 * scale - i * scale - height];
      assert.equal(matrix.length, 6);
      matrix.forEach((value, axis) => assert.ok(Math.abs(value - expected[axis]) < 0.000001, `image ${i}, matrix axis ${axis}`));
    }
    assert.equal(info.outlines.length, 1);
    const root = info.outlines[0];
    assert.equal(root.title, "Root"); assert.equal(root.destpageposfrom1, 1);
    assert.equal(root.kids.length, 1);
    const leaf = root.kids[0];
    assert.equal(leaf.title, "Leaf"); assert.equal(leaf.destpageposfrom1, 1);
    assert.deepEqual(leaf.kids, []);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** Original malformed object after valid page dictionaries; no external bytes. */
export function syntheticDamagedCaj() {
  const base = syntheticCaj();
  const tail = new TextEncoder().encode("99 0 obj << /Bad @ >> endobj\n");
  const bytes = new Uint8Array(base.length + tail.length);
  bytes.set(base);
  bytes.set(tail, base.length);
  const view = new DataView(bytes.buffer);
  view.setUint32(0x404, bytes.length - (0x400 + 24), true);
  view.setUint32(0x40c, bytes.length, true);
  return bytes;
}

// Latin-1 text of a generated PDF with each Flate stream inflated, so tests
// can search page operators. A stream cut short by a failed conversion
// yields the prefix that inflates.
export function pageText(pdf) {
  const bytes = Buffer.from(pdf);
  const marker = Buffer.from("/Filter /FlateDecode\n>>\nstream\n");
  const parts = [];
  let from = 0;
  for (let at = bytes.indexOf(marker, from); at >= 0; at = bytes.indexOf(marker, from)) {
    const data = at + marker.length;
    const end = bytes.indexOf("\nendstream", data);
    const stream = bytes.subarray(data, end < 0 ? bytes.length : end);
    parts.push(bytes.subarray(from, data), inflateSync(stream, { finishFlush: zlibConstants.Z_SYNC_FLUSH }));
    from = end < 0 ? bytes.length : end;
  }
  parts.push(bytes.subarray(from));
  return Buffer.concat(parts).toString("latin1");
}

// Original length-framed metadata; no external document or annotation bytes.
export function syntheticFramedFooter(pdf) {
  const metadata = Buffer.from("<Package>" + "original note ".repeat(1000) + "</Package>");
  const encoded = deflateSync(metadata);
  const lengths = Buffer.alloc(8);
  lengths.writeUInt32LE(metadata.length, 0);
  lengths.writeUInt32LE(encoded.length, 4);
  return new Uint8Array(Buffer.concat([pdf, Buffer.from("WebFastLoad"), lengths,
    encoded, Buffer.from(`APPINFOSIGN ${pdf.length + 11}`)]));
}

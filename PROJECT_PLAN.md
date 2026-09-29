# caj2pdf-rust project plan

## Goal

Build a Rust CAJ-to-PDF converter with a memory-conscious core, a Linux CLI,
and a JavaScript package for browsers and Node.js. English is the language of
the repository, API, CLI, documentation, and diagnostics.

## Repository, license, and release model

- Use the public repository `rwv/caj2pdf-rust`. Keep the existing private
  `caj2pdf-rs` prototype as a reference.
- Keep `main` as the integration branch. Develop each issue on a short-lived
  branch such as `codex/4-streaming-core` and merge through a reviewed pull
  request; no shared long-lived release branch is required.
- All source code committed to this repository must be MIT-licensed. Do not
  copy code from the Python or Go projects or their FreeType/LGPL-derived
  decoders. Existing private Rust modules may be reused only after a per-file
  provenance review confirms that they are original and MIT-eligible.
  Reimplement HN parsing and the CAJ-specific JBIG decoder from format
  descriptions and independently constructed tests.
  Prefer external dependencies that permit MIT use; audit native and WASM
  dependency trees. Do not vendor code without an explicit MIT license and
  preserved attribution.
- Do not copy CAJ sample documents into the repository. Use an optional external
  corpus path for compatibility testing.
- Use Conventional Commits. A breaking change uses `!` or a `BREAKING CHANGE:`
  footer. Releases remain `v0.x.y` until the API and output contract stabilize;
  breaking changes are allowed during this period and must be documented.

## v0.1.0 compatibility target

- Match the Python project's working conversions for CAJ, HN, C8, KDH, and PDF,
  including PDF bookmarks where the source provides them.
- Preserve user capabilities for conversion, metadata inspection, and adding
  CAJ bookmarks to an existing PDF. Design the CLI independently of the Python
  command names and flags.
- Detect TEB and report that conversion is unsupported, matching the current
  reference behavior. Pure-text HN and searchable HN text are outside the
  reference converter's successful behavior and must not be advertised as
  supported in version 1.
- Use Python and Go as behavioral references. Treat the private Rust
  prototype as a migration candidate only after per-file provenance review;
  exclude its FreeType-derived JBIG/HN implementations. Record format
  observations and build or migrate only MIT-eligible code.

## Vendor validation baseline

Use the existing pinned CAJViewer container and capture recipe for a few
representative complete pages per working format. Reproducible manual capture
is sufficient. Compare ordinary copied text only when available and promised;
OCR is outside v0.1.0. Keep source/page/settings hashes in existing manifests
and vendor binaries, documents and captures outside Git. Preserve raw
mismatches and distinguish viewer observations from Python regression results.
See [the fixture plan](docs/cajviewer-fixtures.md) and
[existing CAJ comparisons](docs/cajviewer-page-boxes.md).

## Architecture and I/O

- Use a Cargo workspace with a platform-neutral conversion core, a CLI crate,
  and a WASM/JavaScript crate plus thin browser and Node.js adapters.
- Expose a native conversion API accepting seekable input and a sequential
  output sink, conceptually `Read + Seek` and `Write`. CAJ records contain
  offsets, so a forward-only input may need bounded temporary storage.
- Expose an asynchronous JavaScript API accepting a sized source with
  `readAt(offset, length)` and a backpressure-aware output sink. Implement
  browser `Blob`/`File` and Node.js file adapters. Support a plain
  `ReadableStream` by spooling when random access is required.
- Avoid whole-file input and output buffers. Process payloads in chunks;
  retain only required object/page indexes, bookmarks, and the current image
  or decoding strip. Measure peak memory with representative files.
- Replace the full-document PDF construction path with a sequential writer
  where feasible. Keep the output sink forward-only, including stdout and
  JavaScript `WritableStream` targets.
- Keep format parsing, PDF writing, and platform adapters separate. Return
  structured conversion errors rather than panicking on malformed input.

## CLI behavior

Proposed v0.1.0 interface:

```text
caj2pdf INPUT [-o OUTPUT] [--force]
caj2pdf inspect INPUT [--json] [--bookmarks]
caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]
```

- Conversion is the default operation. A named `paper.caj` produces a sibling
  `paper.pdf` unless `-o` is provided. A PDF input requires an explicit output
  path to avoid deriving its own input path.
- `-` represents stdin or stdout. With stdin and no `-o`, output goes to stdout;
  refuse binary output when stdout is a terminal. Spool non-seekable stdin to
  a temporary file when random access is needed.
- `inspect` prints human-readable metadata or a documented JSON schema;
  `--bookmarks` includes the outline tree. `add-bookmarks` imports the CAJ
  outline into an existing PDF and requires a distinct explicit output path.
- Write only PDF bytes to stdout. Send diagnostics and interactive progress
  to stderr. Disable progress when stderr is not a terminal or when requested
  by a quiet flag.
- Refuse existing output paths by default; `--force` permits overwriting the
  output but never reading and writing the same path. Write path outputs to a
  temporary sibling and commit them only after successful conversion.
- Return exit status 0 on success, 2 for invalid arguments, and 1 for I/O,
  unsupported format, or conversion failures. Provide `--help` and `--version`.

## Delivery status and remaining sequence

The [parent issue #1](https://github.com/rwv/caj2pdf-rust/issues/1) and its
native sub-issue/blocking relationships are authoritative.

1. **Integration acceptance completed (#10 / #182).** HN/C8 composition, native
   and JS scratch adapters, CLI/browser/Node conversion, and HN-A metadata
   inspection are implemented. The complete multi-image document produces
   identical PDFs on all three interfaces; page order, placement, bookmarks and
   cleanup are checked. Original two-image controls run in normal CI. See
   [the results and limitations](docs/js-validation.md#complete-multi-image-hn-a-public-interface-check).
2. **Representative viewer checks completed (#123 / #184).** Source page/image
   extents and storage-padding handling are corrected. HN-A/C8 selected frame
   sizes match, while exact pixels still differ; these remain explicit
   experimental-profile limitations. Complete corrected CLI/Node/browser outputs
   match. See [results and scope](docs/cajviewer-hnc8-kdh.md#results-after-the-source-geometry-correction).
3. **Finish codec distribution (#8 → #30, #9 → #44).** Resolve the exact missing
   rights evidence and wire approved state data into the implemented decoders.
   Record an unresolved decision precisely; do not restart broad research.
4. **Audit and release (#14).** Maintain one support matrix, reuse valid memory
   measurements, check packages/examples and run existing CI. Geometry must be
   fixed or explicitly limited; distribution decisions must be resolved before
   shipping the affected data. OCR and legacy Python ordering remain deferred.

Keep remaining work in these issues, with no new issue hierarchy or framework.
Missing optional corpus is NOT_RUN; unavailable checks do not count as passes.

Use short implementation PRs, focused unit tests, final-head review and
simplification, and the existing native/WASM/license/coverage gates. Add an
abstraction only for a concrete need. Detailed historical results remain in
linked issues and format notes; no extra approval or inventory project is needed.

## Reference material

- Vendor fixture plan: [CAJViewer fixtures](docs/cajviewer-fixtures.md) and
  [epic #123](https://github.com/rwv/caj2pdf-rust/issues/123)
- Python converter: https://github.com/rwv/caj2pdf
- Go prototype: https://github.com/rwv/caj2pdf-go
- Public sample corpus (optional, not vendored):
  https://github.com/caj2pdf/CAJSamples
- Rust standard I/O: https://doc.rust-lang.org/std/io/
- Rust CLI guidelines: https://rust-cli.github.io/book/
- Conventional Commits: https://www.conventionalcommits.org/en/v1.0.0/
- Semantic Versioning: https://semver.org/spec/v2.0.0.html
- Browser Blob ranges: https://developer.mozilla.org/en-US/docs/Web/API/Blob/slice
- Node.js positioned reads: https://nodejs.org/api/fs.html#filehandlereadbuffer-offset-length-position

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

- Add a separate pinned Linux CAJViewer baseline for complete-page images and
  local standard-copy text, as described in
  [the vendor fixture protocol](docs/cajviewer-fixtures.md). The initial GUI
  capture and ordinary-copy pilot is complete; see [the snapshot](docs/cajviewer-fixture-snapshot.md).
  The #129 native PDF pilot matched two pages exactly. Formal baseline approval,
  remaining formats and browser/Node comparisons are incomplete.
- Verify capabilities with original public canaries before private fixture
  acquisition. Keep native page export, complete-page viewer capture and
  print/export-derived images distinct. Compare complete bounded grids and
  preserve raw copy behavior, verified fresh clipboard transactions and page
  mapping.
- Keep the vendor application/runtime, private corpus and acquired images/text
  external. All committed generators, adapters and diff code remain original
  MIT. OCR/enhanced-copy/repair observations use separate modes; this work does
  not expand v0.1.0 to searchable HN, pure-text HN conversion or an OCR engine.
- Retain the Python regression baseline and its known limitations. Publish
  vendor/Python/native disagreements as version-scoped evidence rather than
  replacing existing expectations silently.

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

## Verification and milestones

The actionable v0.1.0 hierarchy and native blocking relationships start at
the [parent issue](https://github.com/rwv/caj2pdf-rust/issues/1). The list below is a summary;
issue acceptance criteria are authoritative for each task.

1. Establish the workspace, dependency license inventory, API contracts, CLI skeleton,
   browser/Node adapters, and generated small test fixtures.
2. Implement CAJ/PDF conversion and bookmarks with ranged input and chunked
   output end to end on native, browser, and Node.js targets.
3. Add clean-room HN, C8, and KDH parity, including an original MIT
   implementation of the required CAJ-specific JBIG decoder.
4. Compare page counts, bookmarks, and rendered output against the Python
   converter on its successful corpus cases. Keep known Python failures and
   unsupported formats classified separately.
   HN-A [field validation](docs/hnc8-outline-fields.md) now compares 159 entries
   from two additional documents and records fifteen targeted controls. #119
   can implement the original bounded visitor and emitted-page mapping for
   that profile, including explicit nullable `/XYZ` destinations. Malformed
   truncation/retargeting behavior is documented rather than silently adopted.
   C8/HN-B applicability and omitted-row policy remain unknown. Historical
   [Stage A results](docs/hnc8-outline-stage-a-results.md) remain unchanged.
5. Complete [the vendor fixture epic #123](https://github.com/rwv/caj2pdf-rust/issues/123)
   using the [simplified fixture plan](docs/cajviewer-fixtures.md).
   The [#124 capture pilot](docs/cajviewer-capture-pilot.md) established a
   practical recipe with reopen pixel differences. The
   [image/text snapshot](docs/cajviewer-fixture-snapshot.md) completes #126/#127;
   the wider #129 matrix remains incomplete. Reproducible manual capture is
   sufficient; no separate baseline-approval project is required.
   Manual initial acquisition is acceptable; Docker is a reproducibility aid.
   Reuse completed manifests (#125). Use [the decoded comparator](docs/vendor-fixture-diff.md) (#128)
   with original controls now and acquired fixtures when available. Begin rollout
   (#129) with implemented native formats; finish browser/Node and remaining
   format coverage after #10/#13. Text unavailability does not block images.
   The standalone source-loading framework (#153) is cancelled, not implemented.
   Fix concrete import/layout bugs with normal packaging and focused tests.
   Keep hashes, timeouts, page/output limits and cleanup; do not add another
   inventory/attestation project. V14 inventory success and twelve historical
   launch observations remain in the [historical report](docs/cajviewer-runtime-view-v14.md).
   Selected native PDF pages 1/11 and CAJ pages 1/75 now match the viewer
   exactly (#158/#168). See [the CAJ comparison](docs/cajviewer-page-boxes.md).
   HN/C8/KDH and browser/Node vendor comparisons remain `NOT_RUN`.

6. Record peak memory, throughput, and output validity on representative
   documents; make these release gates rather than assumptions.
   Public spooling hardening [#141](https://github.com/rwv/caj2pdf-rust/issues/141)
   releases owned Web readers and validates cancellable ordered Node writes,
   with original fault controls and package-import checks. Review the final
   published head and require hosted gates. The common JS package (#13) now
   has runnable example, real-tarball, type and memory checks; see
   [JavaScript validation](docs/js-validation.md). HN/C8 integration on both
   JS targets and its codec provenance gaps remain #10 work and block #14.

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

## Immediate implementation sequence

- #159 is complete: issue-77 converts to 75 valid, source-ordered pages;
  selected pages 1/75 match CAJViewer (#168). This is selected-page evidence,
  not whole-format parity.
- HN-A outline fields (#137) are validated. PR #162 implements the bounded
  reader and PDF output; 159 outline-only matches do not prove page content.
  Issue-29 passes 48 pages, 48 outlines and 96 full-page comparisons (#163).
  The uncompressed HN-A reader also passes issue-69's 81 pages, 111 outlines
  and 162 comparisons against an explicitly corrected Gray reference. See
  [the evidence and legacy deviation](docs/hnc8-uncompressed-text.md).
  C8/HN-B outlines and general HN coverage remain parent #10 work.
- Type-3/mixed-image composition now reuses the existing decoder and writer
  with bounded scratch storage and original tests. Issue-58's direct compressed
  text records now parse; its four complete pages match MuPDF exactly, with
  separately explained Poppler orientation-rounding differences. Next in #118:
  verify an actual mixed page and its source-to-image coordinate mapping. Connect
  complete HN/C8 conversion to CLI and JS in #10 using #13's adapters.
- Keep complete HN/C8 conversion (#10), codec rights, vendor comparisons
  (#123/#129) and the release matrix (#14) as explicit remaining requirements.
- Prefer a concrete failing sample or focused original test over another generic
  proof framework. Each PR still requires review, simplification and green CI.

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
  [the vendor fixture protocol](docs/cajviewer-fixtures.md). Current vendor
  validation is `NOT_RUN`; a supported Linux export CLI has not been proven.
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
   HN/C8 outline discovery is now recorded in the
   [closed Stage A report](docs/hnc8-outline-stage-a-results.md): two HN-A
   references agree on 52 complete entries, and a finite GB18030 candidate
   correlates with all 52 titles. Compatibility remains UNVERIFIED. Exact
   title field/codec, hierarchy and destination rules still need the held-out
   and one-field validation in native child/blocker
   [#137](https://github.com/rwv/caj2pdf-rust/issues/137) before
   [#119](https://github.com/rwv/caj2pdf-rust/issues/119) can implement its
   original bounded visitor and emitted-page mapping. Preserve `/XYZ` null
   parameters explicitly; C8/HN-B applicability and omitted-row policy remain
   unknown. No converter/native/render/vendor call occurred in this discovery;
   the first FAIL and all six unmet #119 criteria remain recorded.
   The original [Stage B design proposal](docs/hnc8-outline-stage-b-proposal.md)
   separates positive baseline and exact-control freezes. It remains DRAFT:
   unresolved grammar, runtime identities and calculated phase ceilings keep
   execution disabled and satisfy no #137 acceptance criterion.
5. Complete [the vendor-oracle epic #123](https://github.com/rwv/caj2pdf-rust/issues/123),
   a direct child of #1 and a blocker for the #14 release gate:
   [manifest #125](https://github.com/rwv/caj2pdf-rust/issues/125) is complete:
   [immutable bounded validation and regeneration](docs/vendor-fixture-manifest.md)
   merged in [PR #131](https://github.com/rwv/caj2pdf-rust/pull/131).
   [Capability #124](https://github.com/rwv/caj2pdf-rust/issues/124) remains open:
   public tmpfs controls pass and the measured `libxslt.so.1` provider is added,
   and an original app-zero shared-memory control isolates the Xvfb limit and
   verifies a bounded allowance. The fifth pair keeps its display/launcher
   alive but fails an unverified filename-based window predicate. Identical
   viewport captures support manual original-PDF opening observation only.
   The ten earlier failures remain recorded. The sixth frozen pair adds one
   helper failure and one observed owned window after PID/process-group
   checks; document identity stays unverified. All twelve attempts are
   retained and the pair remains FAIL. Its helper stderr was not retained,
   so [bounded public diagnostics/controls #133](https://github.com/rwv/caj2pdf-rust/issues/133)
   provide the completed diagnostic prerequisite as a native child/blocker of #124,
   merged in [PR #136](https://github.com/rwv/caj2pdf-rust/pull/136).
   Their original mandatory controls retain terminal captured-byte diagnostics,
   preserve primary failures without a raster, and enforce the existing receipt
   budget. They cannot recover historical stderr or establish a new viewer
   success. The
   [startup records and original child-limit controls](docs/cajviewer-linux-startup.md)
   require a new exact profile and reviewed finite budget before further apps.
   The [closed diagnostics-overlay preparation](docs/cajviewer-startup-diagnostics-v7.md)
   completed nine Docker clients with zero app or inventory calls. Its image
   preserves the full parent Config and six layers, adding one original script
   layer; all 108 public audit rows passed. Separately frozen inventory controls
   passed all eight groups and 47 variants in one original runner, with zero
   actual child, Docker, runtime or application actions. The first operational
   P2 phase closed FAIL after five Docker clients and one incomplete inventory
   attempt. Its complete envelope records `FileNotFoundError` without a loading
   stage or missing path; the specific cause remains UNKNOWN. Closing audits
   and owned-container cleanup passed; all 2,731 runtime comparisons remain
   NOT_RUN. Original source-load accounting and fault controls in
   native child/blocker [#143](https://github.com/rwv/caj2pdf-rust/issues/143)
   now preserve bounded ordered read/pin/compile/exec records for the two
   whitelisted MIT source paths. One reviewed original runner passed all
   25 methods (19 mandatory, four actual inline and two actual host controls),
   with ten mocked helper callbacks and zero actual child, Docker, runtime,
   viewer or private actions. All 24 source audit rows and the closing plan
   identity passed; invented fixtures were removed. The
   [source-load contract and evidence](docs/cajviewer-source-loading.md)
   retain cleanup-refusal controls and limited application/virtual I/O scopes.
   [PR #145](https://github.com/rwv/caj2pdf-rust/pull/145) merged after exact-head
   reviews and hosted native/WASM/MIT/coverage gates; #143 is closed.
   Four separately frozen original execute-escape controls also passed, with
   zero actual child/Docker/runtime/app work. The sole separately frozen v11
   inventory phase then closed FAIL: five Docker clients, one admitted but
   incomplete inventory, zero app/vendor passes and all 2,731 comparisons
   NOT_RUN. Its first original public module loaded; the second failed at
   `read` of `/opt/canary/inventory.py`. The specific missing filesystem
   component and the older v10 cause remain UNKNOWN. Both actual closing
   reviews, all 258 public audit rows and owned-container cleanup passed.
   [The closed v11 report](docs/cajviewer-runtime-view-v11.md) preserves the
   original captures and distinguishes operational failure from closing success.
   Historical v9 supplied the public inventory module through a pinned
   read-only bind; its inventory describes that mounted runtime view, not bare
   image membership. The separately frozen transport amendment's sole v12
   phase is now CLOSED_FAIL: both original source loads passed, but the
   fc-list helper's exit-0 result with 48 stderr bytes failed validation with
   ValueError. The exact message and cause remain UNKNOWN; inventory did not
   complete, all 2,731 comparisons are NOT_RUN and app/vendor counts are zero.
   Both actual closing reviews passed preservation while operational status
   remained FAIL. Independent native child/blocker
   [#148: inventory-helper diagnostics](https://github.com/rwv/caj2pdf-rust/issues/148)
   tracks the bounded original diagnostics, with #125/#133/#143 resolved.
   Further diagnostics and startup require separate exact freezes and reviews.
   The twelve historical launcher attempts and unknown helper-stderr cause
   are preserved; no new capability passed.
   New native child/blocker
   [#146: original capability protocol](https://github.com/rwv/caj2pdf-rust/issues/146)
   tracks implementation of separate document/page identity, complete-page and fresh
   standard-copy gates, with a strict collector and finite GUI discovery.
   Code completion is separate from #124's actual two-session evidence.
   The [capability implementation](docs/cajviewer-capability-protocol.md)
   contains the refused-default host/session, raw desktop/page/clipboard gates,
   exact collector and original controls. The sole reviewed carrier passed
   57 methods and 133 subtests with zero errors, failures or skips; all 112
   ordered source audit rows and both actual closing reviews passed, exit 0.
   One carrier parent and one controlled Python child ran, with zero additional
   candidate forbidden effects. Code acceptance requires exact committed-head
   reviews and the four hosted gates; app observations are NOT_RUN. Neither v11 nor v12
   FAIL satisfies the runtime-view gate, and no operational bindings are invented.
   After successful reviewed declared runtime-view integrity, a fresh frozen
   capability profile may propose two launches (cumulative maximum fourteen),
   ten physical-page observations and four ordinary-copy attempts. No issue
   creation or diagnostic approval grants those launches. Whole-page boundaries,
   repeated decoded grids, fresh clipboard transactions and actual available
   build/render/settings observations must be established before private acquisition.
   [Images #126](https://github.com/rwv/caj2pdf-rust/issues/126)
   and [text #127](https://github.com/rwv/caj2pdf-rust/issues/127) require
   #124 and #125;
   [diffs #128](https://github.com/rwv/caj2pdf-rust/issues/128) follow the manifest
   and acquisition contracts. [Rollout #129](https://github.com/rwv/caj2pdf-rust/issues/129)
   requires all five plus [production HN/C8 #10](https://github.com/rwv/caj2pdf-rust/issues/10)
   and [JavaScript #13](https://github.com/rwv/caj2pdf-rust/issues/13).
   Implementation follows the native GitHub dependency graph.
   Freeze inputs, modes, complete-stage resource caps and receipts before
   private execution. `NOT_RUN`, skipped or unsupported work is not a vendor
   compatibility pass.

6. Record peak memory, throughput, and output validity on representative
   documents; make these release gates rather than assumptions.
   Public spooling hardening [#141](https://github.com/rwv/caj2pdf-rust/issues/141)
   releases owned Web readers and validates cancellable ordered Node writes,
   with original fault controls and package-import checks. Exact-head independent
   reviews and hosted gates remain required. Parent #13 still requires supported
   HN/C8 conversion through both targets and its unresolved provenance gates.

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

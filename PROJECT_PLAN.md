# caj2pdf-rust project plan

## Goal

Convert CAJ-family documents (CAJ, KDH, HN, C8, and PDF) to PDF with a
memory-conscious Rust core, a native CLI, and a JavaScript package for
browsers and Node.js. Detect TEB and report it as unsupported. English is the
language of the repository, API, CLI, documentation, and diagnostics.

## Rules that do not change

- All source code is MIT-licensed and original. Do not copy code from the
  Python or Go converters or their FreeType/LGPL-derived decoders; migrate a
  private Rust module only after per-file provenance review
  ([provenance](docs/provenance.md)).
- Input is ranged or seekable and is never read whole; output is sequential.
  Buffers are capped by `Limits`: one image payload, symbol dictionary or page
  bitmap may live in memory. Forward-only input is spooled by the platform
  adapter ([I/O architecture](docs/io-architecture.md)).
- Keep format parsing, PDF writing, and platform adapters separate. Return
  structured errors instead of panicking on malformed input.
- Sample documents stay outside Git. A missing optional corpus is
  `NOT_RUN`, never a pass.
- Conventional Commits; `v0.x.y` releases may carry documented breaking
  changes ([release policy](docs/release-policy.md)).

## Current status

The [support matrix](docs/conformance.md#current-support-and-release-status)
is authoritative. CAJ, KDH and PDF are supported. HN-A converts as image
pages with bookmarks. C8 and HN-B convert image pages and admitted native-text
profiles with caller fonts, and omit their unverified outlines with a warning.
The CLI interface is documented in the [CLI reference](docs/cli.md).

## Remaining sequence

1. **C8/HN-B bookmarks (#303).** Blocked on a sample whose viewer shows a
   non-empty outline.
2. **Release v0.4.0 (#287).** Run the full release matrix from current main.
3. **Registry publication (#293).** Needs crates.io and npm accounts.
4. **Partial CAJ output decision (#297).** Waiting on a maintainer decision.

Use short PRs with focused tests and the existing native, WASM, license,
link and coverage gates. Add an abstraction only for a concrete need.

## History

The original v0.1.0 plan (milestones, proposed CLI, vendor validation
baseline and the v0.3.0 platform expansion) is preserved at
[this revision](https://github.com/rwv/caj2pdf-rust/blob/f21fa98c600d8aa3113f9daa0e7c08dc6a6ce4a3/PROJECT_PLAN.md).
Delivered changes are in the [changelog](CHANGELOG.md) and the
[release notes](docs/releases/). The format investigations are indexed in
[docs/research](docs/research/README.md) and live, with the research tooling,
in [caj2pdf-samples `research/`](https://github.com/rwv/caj2pdf-samples/tree/main/research/README.md).

# caj2pdf-rust

An MIT-licensed Rust project for converting CAJ-family documents to PDF from
the command line and JavaScript. v0.x is unstable; HN/C8 support is experimental.
English is the project's public language for code, documentation, APIs, CLI
output, and releases.

## Downloads

[GitHub Releases](https://github.com/rwv/caj2pdf-rust/releases) provide native CLI archives, the browser/Node JS tarball, standalone WASM,
container OCI archive and SHA256SUMS. See the [platform matrix](docs/platforms.md)
and [Docker usage](docs/docker.md). See the
[v0.3.0 notes](docs/releases/v0.3.0.md) for platform requirements and limitations.
After downloading, extract the CLI archive or install the JS tarball with
`npm install ./caj2pdf-rust-0.3.0.tgz`. Registry publication is separate.

## Goals

- Convert the formats that the Python caj2pdf project handles successfully:
  CAJ, HN, C8, KDH, and PDF. Detect TEB and report it as unsupported.
- Use a seekable or ranged input source and a sequential output sink so that
  conversion does not require loading a whole document into memory. A
  forward-only input may be spooled to temporary storage.
- Provide a Linux CLI and a WASM-backed JavaScript API for browsers and Node.js.
- Implement HN parsing and the CAJ-specific JBIG decoder as new MIT code.

The proposed architecture, CLI, acceptance milestones, and reference links are
in [PROJECT_PLAN.md](PROJECT_PLAN.md). Start with the
[v0.1.0 parent issue](https://github.com/rwv/caj2pdf-rust/issues/1) and its
native sub-issues and blocked-by relationships. All initial tasks belong to
the [v0.1.0 milestone](https://github.com/rwv/caj2pdf-rust/milestone/1).
See the [provenance and dependency inventory](docs/provenance.md) for format
references, test corpus rules, and the MIT-only source review process.
The [bounded I/O architecture](docs/io-architecture.md) records the native and
JavaScript source/sink contract for the converter.
The [conformance baseline](docs/conformance.md) documents the optional
external corpus runner and independently generated MIT test fixtures.
The [PDF input profile](docs/pdf-input.md) records supported syntax, repair
rules, and the existing-outline policy.
The [forward-only PDF writer](docs/pdf-writer.md) provides streamed reusable
image objects and ordered affine placements, with bounded page work and
same-document handle validation.

## Command-line usage

The `caj2pdf` Linux command converts CAJ, KDH and PDF inputs, and supports
experimental HN/C8 image-page conversion. HN/C8 arithmetic images use
built-in standard QM/MQ states; optional state files override those defaults. The same core converter is available through
[Node and browser WASM](js/README.md), using caller-owned bounded scratch stores.
See the [support matrix and release status](docs/conformance.md#v01-support-and-release-status)
for verified profiles and remaining HN/C8 rendering differences.
TEB is recognized and unsupported. Build with `cargo build --release -p caj2pdf-cli`.

For C8/HN-B, explicitly disable bookmark import until those outline semantics
are verified. Unsupported profiles, including image-less HN-B source rows,
fail rather than silently losing pages. See the [CLI reference](docs/cli.md)
for runtime state-file syntax and limitations. The existing
[page composer](docs/hnc8-page-composition.md),
[repeated-group rules](docs/hnc8-repeated-groups.md), and
[JS validation](docs/js-validation.md) document implementation evidence.

```sh
caj2pdf paper.caj                  # writes paper.pdf next to the input
caj2pdf paper.caj -o out.pdf       # explicit output; --force replaces a file
caj2pdf - < paper.caj > paper.pdf  # standard input and output
caj2pdf paper.c8 --no-bookmarks -o out.pdf
caj2pdf inspect paper.caj --json --bookmarks
caj2pdf add-bookmarks paper.caj scan.pdf -o scan-with-outline.pdf
```

Existing outputs are kept unless `--force` is given, an input is never
overwritten, and a path output is moved into place only after conversion
succeeds. Exit status is 0 on success, 2 for invalid arguments, and 1 for
other failures. The [CLI reference](docs/cli.md) documents every rule and the
`inspect` JSON schema.

## Versioning and development

Releases use `v0.x.y` during initial development. APIs, CLI behavior, and
output may change between `0.x` releases; breaking changes are documented in
[the release notes](CHANGELOG.md). Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/),
including `!` or a `BREAKING CHANGE:` footer for breaking changes. See
[CONTRIBUTING.md](CONTRIBUTING.md) and the [release policy](docs/release-policy.md).

## License and test data

Source code in this repository is licensed under [MIT](LICENSE). Contributors
must not copy code from the Python or Go converters or third-party decoders
with different licenses. Code from the earlier private Rust prototype may be
used only after per-file provenance review confirms MIT eligibility; its
FreeType-derived JBIG/HN code must be reimplemented. The separate
[CAJSamples](https://github.com/caj2pdf/CAJSamples) collection may be used as
an optional external compatibility corpus; sample documents are not included
in this repository.

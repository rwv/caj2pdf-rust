# caj2pdf-rust

An MIT-licensed Rust project for converting CAJ-family documents to PDF from
the command line and JavaScript. The project is in its initial development
phase; no converter release is available yet. English is the project's public
language for code, documentation, APIs, CLI output, and releases.

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

## Versioning and development

Releases use `v0.x.y` during initial development. APIs, CLI behavior, and
output may change between `0.x` releases; breaking changes are documented in
the release notes. Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/),
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

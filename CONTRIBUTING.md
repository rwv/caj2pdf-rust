# Contributing

## Before starting

Read the issue you intend to work on, including its acceptance criteria,
sub-issues, and blocked-by relationships. Keep a pull request focused on one
issue or a closely related set of issues. Use English for code, documentation,
CLI text, issue updates, and pull requests.

Create a short-lived branch from `main` for each issue, for example
`feat/4-streaming-core` or `chore/2-workspace`. Reference the issue in the pull
request and merge only after its acceptance criteria and tests are reviewed.

## Licensing and provenance

All source code committed here must be MIT-licensed. Write original code from
format specifications, documented observations, and independently authored
tests. Do not copy or transliterate the Python or Go projects or any
FreeType/LGPL-derived decoder or HN implementation. A private Rust module may
be migrated only after a per-file provenance review confirms original
ownership and MIT eligibility; its JBIG/HN code must be reimplemented.
Document the source of format facts in code comments or pull requests. Do not
vendor sample documents or third-party code unless its MIT license and
attribution have been verified. Audit native and WASM dependency trees before
release; a package manifest alone is not proof of source provenance. Record
the review in the [provenance inventory](docs/provenance.md).

## Documentation layout

User documentation lives in `docs/` (CLI, support matrix, platforms, Docker,
I/O architecture, PDF input, provenance and release policy). Format
investigations, oracles and validation runs live in `docs/research/`; add a
new note there and list it in [its index](docs/research/README.md). Notes whose
bytes are hash-pinned by a script must not be edited.

## Commit and release policy

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
feat(core): add bounded range reader
fix(cli): reject output paths matching the input
docs: explain HN support limits
feat(wasm)!: rename the JavaScript source interface
```

Mark breaking changes with `!` or a `BREAKING CHANGE:` footer, describe the
migration in the pull request, and include it in release notes. Version `v0.x.y`
is unstable: breaking changes are allowed and compatibility is not promised.
Do not silently change public APIs or output behavior.
The [release policy](docs/release-policy.md) defines version bumps, review,
lockfiles, and release-note requirements.

## Pull request evidence

State which acceptance criteria the change satisfies and provide the command
and result for relevant tests. Native, browser, and Node.js paths need separate
evidence when affected. Tests that skip because an optional external corpus is
absent do not count as corpus validation. Record memory measurements for changes
to buffering, decoding, or PDF output. Keep new test fixtures synthetic or
otherwise demonstrably redistributable under MIT.
Write meaningful unit tests for success, malformed input, and error paths.
The required native line-coverage gate is 100% for every source file in its
LCOV report. Exercise real behavior and error propagation; do not add
assertions that only mirror the implementation or hide uncovered lines.

## Fuzzing

`fuzz/` holds two cargo-fuzz targets that drive the public core entry points
on arbitrary bytes: `convert` (PDF, CAJ, KDH and HN/C8 image pages through the
same routes as the CLI) and `inspect` (CAJ metadata, HN/C8 page rows and HN-A
outlines). A target fails only by panicking, aborting, exceeding its limits or
timing out. CI checks that they compile; the weekly `fuzz.yml` workflow runs
them. Locally:

```sh
cd fuzz
cargo +nightly fuzz run convert -- -max_total_time=300 -max_len=65536 -timeout=60
```

Seed from `tests/fixtures`, never the external corpus. Turn every reproduced
crash into an ordinary regression test with synthetic bytes and a located
error. `fuzz/` is not part of the product workspace, published crates or the
coverage gate.

## Dependency updates

Dependabot proposes weekly grouped Cargo and GitHub Actions updates. Review them
like any other change: the MIT/provenance audit in `deny.toml` and
[docs/provenance.md](docs/provenance.md) applies, and the JavaScript package
keeps zero runtime dependencies. Validation tools pinned in workflows (qpdf,
TypeScript, Node types) are updated by hand.

## Quality gates

CI requires `cargo fmt --check`, Clippy with `-D warnings`, rustdoc with
`-D warnings`, locked native tests, the WASM build and JavaScript adapter
tests, the MIT license/source/advisory audit with a relative Markdown link
check (`python3 scripts/check-doc-links.py`), and the line-coverage gate in
`scripts/check-coverage.sh`. The gate requires every unique instrumented Rust
source line in the native LCOV report to be covered, both in total and in each
reported file; it fails on any uncovered line even when a rounded percentage
displays 100%. Run `bash scripts/check-coverage.sh` locally (it needs
`cargo-llvm-cov` and the PDF validators listed in
[the PDF writer notes](docs/research/pdf-writer.md)).

The merge gate is these `ci.yml` jobs plus the Linux x86_64/ARM64 glibc and
musl `native` jobs of `platforms.yml` (see the
[release policy](docs/release-policy.md)). On a pull request, `platforms.yml`
runs its other release-platform jobs (BSD/illumos VMs, Android, QEMU, Docker)
only when packaging, toolchain or lockfile inputs change; they always run on
`main`, before each release and on demand. A red matrix on `main` must be
fixed before the next release is tagged.

Coverage is measured per source file, so inline `#[cfg(test)]` modules count
toward their file's figure. Prefer a sibling `tests.rs` module (as
`pdf/input` and `jbig1` do) for new unit tests so the per-file figure reflects
production code.

## Optional CAJViewer setup

Use the [pinned binary mirror setup](docs/cajviewer-setup.md) to acquire the
external Linux viewer installer. Normal tests do not download vendor binaries;
the manual installer-integrity workflow does not count as viewer compatibility.

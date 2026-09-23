# Contributing

## Before starting

Read the issue you intend to work on, including its acceptance criteria,
sub-issues, and blocked-by relationships. Keep a pull request focused on one
issue or a closely related set of issues. Use English for code, documentation,
CLI text, issue updates, and pull requests.

## Licensing and provenance

All source code committed here must be MIT-licensed. Write original code from
format specifications, documented observations, and independently authored
tests. Do not copy or transliterate the Python or Go projects, the previous private
Rust prototype, or any FreeType/LGPL-derived decoder or HN implementation.
Document the source of format facts in code comments or pull requests. Do not
vendor sample documents or third-party code unless its MIT license and
attribution have been verified. Audit native and WASM dependency trees before
release; a package manifest alone is not proof of source provenance.

## Commit and release policy

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
feat(core): add bounded range reader
fix(cli): reject output paths matching the input
docs: explain HN support limits
feat(wasm)!: rename the JavaScript source interface
```

Mark breaking changes with `!` or a `BREAKING CHANGE:` footer, describe the
migration in the pull request, and include it in release notes. Version `0.x.y`
is unstable: breaking changes are allowed and compatibility is not promised.
Do not silently change public APIs or output behavior.

## Pull request evidence

State which acceptance criteria the change satisfies and provide the command
and result for relevant tests. Native, browser, and Node.js paths need separate
evidence when affected. Tests that skip because an optional external corpus is
absent do not count as corpus validation. Record memory measurements for changes
to buffering, decoding, or PDF output. Keep new test fixtures synthetic or
otherwise demonstrably redistributable under MIT.

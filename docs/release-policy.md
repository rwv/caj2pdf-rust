# Commit and release policy

English is the public language of commits, APIs, CLI output, documentation,
pull requests, and release notes. The project follows
[Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/).
All project-owned source committed here must be MIT-licensed; follow the
[provenance review](provenance.md) before adding any external code.

## Commits and pull requests

Use `type(scope): summary`, for example `feat(core): add bounded range
reads`, `fix(cli): reject duplicate input and output paths`, or
`test(wasm): cover malformed source errors`. Keep one logical change per
commit. Common types are `feat`, `fix`, `docs`, `test`, `refactor`, `perf`,
`build`, `ci`, and `chore`. A scope is optional.

Mark every intentional breaking change with `!` immediately before the colon
or a `BREAKING CHANGE:` footer; both are valid. For example:

```text
feat(wasm)!: rename the JavaScript source interface

BREAKING CHANGE: callers must provide readAt(offset, length).
```

A pull request must link its issue, enumerate satisfied and unmet acceptance
criteria, show relevant test and CI results, and explain any effect on memory
use, native API, CLI, JavaScript API, output PDF, or supported formats. Review
and simplify the final diff before merging: remove needless abstraction and
duplication, then rerun affected tests. A reviewer checks the issue
criteria, provenance and license record, tests, and release-note impact.
Required quality gates must pass; skipped optional-corpus tests are reported
separately and never counted as compatibility passes.

## `v0.x.y` versions

Releases remain unstable until `v1.0.0`. During `v0.x.y`, the native API, CLI,
JavaScript API, supported input behavior, and PDF output may change. A
breaking change is permitted only when its commit and release notes identify
it explicitly and give users a migration path. This is a project policy for
communicating changes, not a compatibility guarantee.

| Change | Default next release |
| --- | --- |
| Breaking public behavior or API, or a compatible feature | Increment `x`, reset `y` to 0. |
| Compatible fix or internal maintenance | Increment `y`. |
| First implementation milestone | `v0.1.0`. |

Release notes must list each breaking change with the old and new behavior,
affected native/CLI/browser/Node.js surfaces, and a short migration example.
They also state format support limits, known failures, and which optional
corpus cases were actually run. Tag and publish only after the locked native
and WASM build, tests, license/provenance review, and artifact inspection pass.
The workspace commits its root `Cargo.lock`; CI and release builds use it in
locked mode. If a JavaScript package later adds an npm dependency graph, commit
its package-manager lockfile and use frozen installs for release builds.

The npm package in `js/` stays `"private": true` in the repository. To
publish it, build and copy the WASM with `npm run build:wasm` inside `js/`
(its `prepack` script refuses to pack a missing or non-WASM
`caj2pdf_wasm.wasm`), inspect
`npm pack --dry-run` against the file list asserted by
`js/test/package.test.mjs`, remove `private` in the release commit, and
publish that tarball. The copied `.wasm` is a build product and is never
committed.

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

The npm package in `js/` stays `"private": true` until the release commit.
To publish it, remove `private` in that commit, then build and copy the WASM
with `npm run build:wasm` inside `js/`. Its `prepack` script refuses a
missing or non-WASM `caj2pdf_wasm.wasm`. Inspect the actual `npm pack`
tarball against the file list asserted by `js/test/package.test.mjs`, run
its Node and Chromium artifact smoke tests, and publish the verified
tarball. The copied `.wasm` is a build product and is never committed.

## GitHub release assets

After reviewing and merging release changes, push `v<package-version>` to run
`.github/workflows/release.yml`. It reruns the four existing quality gates,
checks crate/JS/tag versions, builds Linux x86_64 and WASM artifacts, tests the
actual npm tarball, and uploads assets plus SHA256SUMS. Publication starts as a
draft and becomes an unstable prerelease only after asset upload succeeds.
The release job also runs without publication on relevant pull requests.
GitHub-only releases may retain npm `private` and Cargo `publish = false`;
registry publication remains a separate operation. See `docs/releases/` for
versioned notes and platform limits. Never reuse the historical local candidate
hashes for CI-built artifacts; the uploaded SHA256SUMS is authoritative.

From v0.2.0, the release also calls `platforms.yml` and requires every target in
`docs/platform-targets.json`. It aggregates native archives, the tested JS/WASM
package and tested OCI container archive, then writes complete SHA256SUMS.
Windows and macOS assets are unsigned. GHCR uses the workflow's scoped
`packages: write` permission; first publication defaults to private visibility
on GitHub, so the package owner must make the new package public before claiming
anonymous pull support. The downloadable OCI archive remains available through
GitHub Releases. Never replace assets of an already published version.

From v0.3.0, the stable `Native platform matrix` status aggregates every required
native/container job and fails on failure, cancellation or skipped jobs. It is
required alongside the four existing quality statuses before merging to main.
Optional candidate probes are separate and never substitute for release gates.

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

<a id="github-release-assets"></a>

## Current release procedure

1. Merge a reviewed version/notes change. Keep all three crate versions, the
   CLI's core dependency, both Cargo lockfiles and `js/package.json` aligned.
   Update `CHANGELOG.md` and `docs/releases/v<version>.md`. Core and CLI publish
   to crates.io; the WASM crate stays `publish = false` and ships through npm
   and the standalone WASM asset. npm is already public.
2. Pass locked native/WASM builds, tests, license/provenance and packaged-source
   checks. Inspect the actual npm tarball and run its Node/Chromium tests.
   Generated WASM and font assets are build products, never committed source.
3. Push `v<package-version>` at the merged release commit. The
   [release workflow](../.github/workflows/release.yml) reruns quality gates,
   checks version agreement, and builds/tests every native and container target
   in [the native inventory](platform-targets.json) and
   [the container inventory](container-platforms.json).
4. The workflow publishes the tested OCI archive to GHCR, aggregates the exact
   file inventory and `SHA256SUMS`, and verifies attestations for every file and
   the exact container digest against the expected workflow, tag and commit.
   Only then does it publish the complete draft as an unstable GitHub prerelease.
5. With `REGISTRY_TRUSTED_PUBLISHING=true`, the same workflow then publishes
   core followed by CLI to crates.io and the exact tested tarball to npm using
   trusted-publisher OIDC. Verify registry visibility, clean installations,
   npm integrity/provenance and anonymous GHCR access before reporting release
   completion. GitHub publication alone does not prove registry publication.

The tagged run, uploaded checksums and verified attestations identify released
bytes; local candidate hashes do not identify CI-built assets. Never replace
published tags/assets/packages or retroactively attest an older release as a
new build. Windows and macOS binaries remain unsigned. See
[build provenance](build-provenance.md) for consumer verification and the
aggregation-job trust boundary. Keep long-lived registry tokens out of CI.

## Merge and publication gates

PR merging requires four quality statuses: `Native quality gates`, `WASM build`,
`MIT dependency and source audit`, and `Line coverage (90% required)`. It also
requires Linux x86_64/ARM64 native jobs for both glibc and musl: eight required
checks total. Keep status names and branch protection aligned; the coverage
floor is enforced by [check-coverage.sh](../scripts/check-coverage.sh).

The full `Native platform matrix`, including every native/container job, is a
publication gate. Rare platforms may finish after merge; inspect their results
and fix failures in focused follow-ups. Failure, cancellation, pending and
skipped jobs are not passing evidence. Optional probes never substitute for
required jobs. Branches need not rerun solely because unrelated main advanced;
resolve conflicts and test affected integration changes before merging. This
does not waive the complete inventory or attestation gates at publication.

## First registry publication (#293)

Historical [first-publication instructions](releases/registry-bootstrap.md#first-registry-publication-293).

## v0.5.0 registry bootstrap and subsequent OIDC releases

Historical [trust setup](releases/registry-bootstrap.md#v050-registry-bootstrap-and-subsequent-oidc-releases).
Current tags follow the procedure above.

### v0.5.0 bootstrap receipt (2026-10-08)

Preserved [v0.5.0 receipt](releases/registry-bootstrap.md#v050-bootstrap-receipt-2026-10-08).

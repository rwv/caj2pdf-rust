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
a release gate. PR merging prioritizes Linux: require the four quality statuses,
the Linux x86_64/ARM64 glibc and musl native jobs. The full Docker matrix
depends on rare musl/QEMU targets and remains a release gate. Slow rare-platform
and full-container jobs may finish after merge; inspect their results and fix failures in focused
follow-ups. Pending or skipped jobs are not passing platform evidence. Branches
need not rerun solely because an unrelated PR advanced main; resolve conflicts
and test affected integration changes before merging. This merge policy does
not waive the complete target inventory or attestations required for publication.
Optional candidate probes are separate and never substitute for release gates.

From v0.3.1, the tag-only publisher signs the same run's complete release file
set and exact GHCR digest using GitHub artifact attestations. Verification of
all subjects against the expected workflow, tag and commit must pass before
GitHub release publication. Downloadable Sigstore bundles are added after
checksumming/signing; they are verified cryptographically, not self-hashed.
See [build provenance](build-provenance.md) for consumer commands and the
aggregation-job trust boundary. Never retroactively attest old releases as
outputs of a new build run.

## First registry publication (#293)

The core and CLI manifests are prepared for crates.io; CI runs
`cargo publish --dry-run --locked -p caj2pdf-core -p caj2pdf-cli`, including
building the packaged sources. This does not upload or reserve either name.
The WASM crate remains `publish = false`: its distribution is the tested npm
package and standalone WASM, not a standalone Rust registry crate.

As of the 2026-10-05 audit, `caj2pdf-core`, `caj2pdf-cli` and npm
`caj2pdf-rust` returned registry 404 responses. Names are not reserved.
The registry owner must bootstrap publication and configure trust for this
repository before enabling automated uploads. The
[crates.io guidance](https://blog.rust-lang.org/2025/07/11/crates-io-development-update-2025-07/)
requires a first manual release before trusted publishing can be configured.
[npm trusted publishing](https://docs.npmjs.com/trusted-publishers/) is configured
in the package settings. Do not add long-lived registry credentials to CI.

GitHub artifact publication does not imply registry publication. Keep the npm
package private and installation instructions pointing to tested release
artifacts until registry publication and clean-machine installs are verified.

## v0.5.0 registry bootstrap and subsequent OIDC releases

The v0.5.0 release commit removes npm `private`, sets explicit public registry
publication, and bumps the Rust/JS packages together. Publish the first core,
then CLI from the reviewed release commit using owner authentication, and the
exact tested GitHub npm tarball. Never store owner tokens in GitHub secrets,
issues, chat, or committed files. Run clean registry installs and the pinned
two-page fixture before changing README installation instructions.

After those packages exist, configure a trusted publisher on each package:
GitHub owner `rwv`, repository `caj2pdf-rust`, workflow `release.yml`, with no
GitHub environment (the job does not declare one). For npm explicitly allow
`npm publish`; the stage-only default does not authorize direct publication.
Then set repository variable `REGISTRY_TRUSTED_PUBLISHING=true`. Subsequent
version tags publish only after the complete GitHub release job succeeds.
The crates.io auth action mints a scoped short-lived token and revokes it in
its post step; npm obtains OIDC credentials and provenance directly. No
standalone Rust WASM crate is uploaded. New npm trust configurations must be
used within the registry's validation window (currently two days), so bind
trust when the next publication is ready rather than long in advance.

The first manual uploads are bootstrap evidence, not proof of a tagged OIDC
publication. Keep #293's tagged-publication criterion open until an actual
trusted-publisher release and clean registry installations are verified.

### v0.5.0 bootstrap receipt (2026-10-08)

Core and CLI 0.5.0 were uploaded from release commit
`aa11e81205e1c03b1ea52ddf00e1f94e3a126a7a`. npm 0.5.0 uses the unmodified
`release-assets` tarball from tag run
[37737401209](https://github.com/rwv/caj2pdf-rust/actions/runs/37737401209),
SHA-256 `35ee02276c3f4d17897064c628bf9f3e20efa330c06e0ca00de40092442d459a`.
The npm registry integrity matches those bytes and `latest` points to 0.5.0.

Fresh `cargo install caj2pdf-cli --version 0.5.0 --locked --root ...` and
`npm install --ignore-scripts --save-exact caj2pdf-rust@0.5.0` installations
both converted the two-page `js/test/helpers.mjs::syntheticCaj` fixture
(SHA-256 `0d9ff4d560b2ec21ed0e0667def23d2adf0cb2311d0f8077f39ffe1c0dca6dc1`).
Qpdf accepted both two-page PDFs and their bytes matched. This is synthetic
registry-install evidence, not an external-corpus compatibility run.

All three trusted-publisher configurations were saved for
`rwv/caj2pdf-rust` / `release.yml`. The first uploads used owner authentication,
so no successful registry OIDC publication or npm OIDC provenance is claimed.
The temporary crates.io bootstrap token was revoked and removed locally.
The npm binding remains pending its first OIDC publish, with the initial
validation deadline 2026-10-10 06:30 UTC; recreate it if it expires before
another release. Do not republish or modify immutable v0.5.0 packages to test it.


### Recovering a missing npm upload

An already published GitHub release and crates.io version are immutable. If npm
publication alone fails, fix the workflow on main, then manually dispatch
`release.yml` on main with `npm_recovery_tag` set to the existing version tag.
The recovery job downloads the original public GitHub tarball, checksum manifest
and Sigstore bundle; verifies the tag, source commit, workflow and artifact
attestations; and uploads only that exact npm package through OIDC. It neither
rebuilds artifacts nor republishes Rust crates. An existing npm version is never
overwritten. Always spell the local tarball path with `./` or an absolute path;
otherwise npm can interpret `dist/name.tgz` as a GitHub repository shorthand.

npm recovery provenance identifies the manual publishing workflow run on main.
The original release's GitHub artifact attestation independently identifies the
build tag and source commit; do not describe the recovery run as the original
build. Record both runs and verify the registry integrity and installation.

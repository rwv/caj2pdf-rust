# Registry bootstrap history

These are the original October 2026 setup instructions and v0.5.0 receipt,
retained as historical evidence. Statements about private packages, unclaimed
names, pending trust and deadlines describe that checkpoint, not current
requirements. Follow the [current release policy](../release-policy.md).

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

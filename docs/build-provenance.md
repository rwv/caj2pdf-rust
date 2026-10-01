# Verifying release build provenance

Starting with v0.3.1, tagged releases include GitHub artifact attestations
using Sigstore and the SLSA provenance v1 predicate. The signed subjects include
all native archives, the JS tarball, standalone WASM, tested OCI archive,
release documents, container digest file and `SHA256SUMS`. A separate
attestation identifies the GHCR multi-platform image by its immutable digest.

## Verify a downloaded file

Install a current [GitHub CLI](https://cli.github.com/) and authenticate with
`gh auth login`. Download the desired release asset and
`RELEASE-PROVENANCE.sigstore.json` from the same release. Replace the example
version, filename and **independently trusted full release commit SHA** below:

```sh
version=v0.3.1
commit=FULL_RELEASE_COMMIT_SHA
asset=caj2pdf-v0.3.1-x86_64-unknown-linux-gnu.tar.gz
gh attestation verify "$asset" \
  --bundle RELEASE-PROVENANCE.sigstore.json \
  --repo rwv/caj2pdf-rust \
  --signer-workflow rwv/caj2pdf-rust/.github/workflows/release.yml \
  --source-ref "refs/tags/$version" \
  --source-digest "$commit" --signer-digest "$commit" \
  --deny-self-hosted-runners
```

Omit `--bundle` to retrieve attestations from GitHub instead. A local bundle
avoids fetching the attestation from the API; full offline verification also
requires a prepared trusted root (see the CLI documentation).

Use the same command with `asset=SHA256SUMS`, then run `sha256sum -c SHA256SUMS`
in a directory containing all covered assets. For a partial download, verify
that individual artifact directly instead. The two `.sigstore.json` bundles
are added after signing and are not in `SHA256SUMS`; their integrity is checked
by signature verification, avoiding circular hashes. GitHub's automatically
generated source archives are not signed release subjects.

## Verify a container

Verify `CONTAINER-DIGEST.txt` with the file command above first. Use its digest
in place of `sha256:...`, with the same version and commit policy:

```sh
gh attestation verify oci://ghcr.io/rwv/caj2pdf-rust@sha256:... \
  --bundle-from-oci \
  --repo rwv/caj2pdf-rust \
  --signer-workflow rwv/caj2pdf-rust/.github/workflows/release.yml \
  --source-ref "refs/tags/$version" \
  --source-digest "$commit" --signer-digest "$commit" \
  --deny-self-hosted-runners
```

The registry may require authentication (`gh auth token | docker login
ghcr.io -u YOUR_GITHUB_USER --password-stdin`). Alternatively use
`--bundle CONTAINER-PROVENANCE.sigstore.json` instead of `--bundle-from-oci`.
Always pull by the verified digest when immutability matters.

## Scope and trust boundary

The tag-only publisher downloads this same workflow run's required, tested
native/JS/WASM/container outputs. It validates the complete inventory, signs
all files, and verifies every signed file plus the registry attestation against
its repository, workflow, source commit and tag before publishing the GitHub
release. A deliberately modified file must fail verification. The container
and its attestation may already exist in GHCR if a later publication step fails.

The signature authenticates the release workflow and artifact bytes. Signing
occurs in the aggregation job, not within each compiler job. This is not a
claim of an independently isolated builder, SLSA level 3, reproducible builds,
malware scanning, Apple notarization or Windows Authenticode signing. The
workflow and its dependencies remain part of the trust boundary. Historical
releases are not retroactively attested or replaced.

The action is pinned to a full commit SHA. Only the tag-only publisher receives
OIDC and attestation write permissions; PR jobs neither sign nor publish.
No persistent signing secret is needed. OCI attestations are registry referrers,
so the existing ten-platform image index and tested layers remain unchanged.

References: [GitHub artifact attestations](https://docs.github.com/en/actions/concepts/security/artifact-attestations),
[official attest action](https://github.com/actions/attest), and
[GitHub CLI verification policy](https://cli.github.com/manual/gh_attestation_verify).

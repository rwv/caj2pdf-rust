<!-- SPDX-License-Identifier: MIT -->

# Fetching CAJViewer for optional tests

The installer mirror is [rwv/cajviewer-binaries](https://github.com/rwv/cajviewer-binaries).
It hosts original vendor installers in GitHub Releases. This repository keeps
only original MIT test tools and references; no vendor binaries or corpus
files belong in its Git history or release assets.

## Linux x86_64

From the repository root, explicitly fetch the pinned installer into external
storage (Python 3.10+):

```sh
python3 scripts/fetch_cajviewer.py "$HOME/.cache/caj2pdf-viewer/cajviewer.deb"
```

The command selects `linux-9.0.0-24093` and the fixed
`cajviewer-9.0.0-24093-linux-x86_64.deb` asset. It verifies the existing
235,087,704-byte size and SHA-256
`3142c633d74dcf34ebaca9b7653f88ad3619f0b7a6cb689487b6cc583ec926d3`.
There is no `latest` lookup or silent fallback to the vendor CDN. Downloads use
64 KiB buffers, an exact size ceiling, a 30-second socket timeout and a temporary
file in the destination directory. Only verified bytes are published, without
overwriting an existing destination. Interrupted/invalid downloads are removed.
Existing cached files are verified on every invocation; a corrupt cache fails
explicitly. Remove the corrupt file yourself before retrying. On mirror/network
failure, retry later or obtain the exact same installer from the original URL
and verify it with the existing canary.

The official provenance URL remains
`https://download.cnki.net/cajviewer_9.0_amd64.deb`. For new capture receipts,
record the actual acquisition URL in `runtime.application.installer_url` and
retain the original vendor URL in accompanying provenance. Historical receipts
remain unchanged. Mirror hashes establish byte identity, not vendor signatures.

The fetched file is accepted by the existing tools:

```sh
python3 scripts/cajviewer_canary.py --installer "$HOME/.cache/caj2pdf-viewer/cajviewer.deb"
python3 tools/cajviewer/prepare.py --help
```

Follow the [Linux preparation protocol](research/cajviewer-linux-startup.md)
for external extraction and isolated execution. Fetching does not execute the
installer, accept agreements, open documents, or produce reference fixtures.
The existing offline viewer/container boundary remains in place.

## Tests and CI

Normal conformance tests exercise fetching with original synthetic data and
mocked network I/O. They do not acquire proprietary software:

```sh
python3 -m unittest discover -s tests/conformance -p 'test_cajviewer_fetch.py' -v
```

The **CAJViewer installer integrity (manual)** Actions workflow is opt-in via
`workflow_dispatch`; it is not a PR/release gate. It uses a hash-keyed cache,
verifies both cache hits and new downloads, and has a 15-minute job timeout.
A corrupt cache must be deleted from Actions caches before retrying. No vendor
package is uploaded as a main-project artifact. A successful run means only
installer integrity: viewer rendering/text compatibility is NOT_RUN, with
zero vendor passes. Actual rendering tests still require the existing fixture
protocol and their own measured outcomes.

## Other platforms and licensing

The mirror catalog distinguishes OS, architecture, version, format and variant.
Windows/macOS installers are tracked in
[cajviewer-binaries#1](https://github.com/rwv/cajviewer-binaries/issues/1).
This fetcher and execution profile currently support only the pinned Linux
x86_64 installer. Adding mirror records does not automatically establish viewer
support on those platforms; add explicit pins, setup and platform tests when
an actual installer is available. Avoid a generic platform framework until then.

CAJViewer remains proprietary; our MIT license does not cover it. The mirror's
[third-party notices](https://github.com/rwv/cajviewer-binaries/blob/main/THIRD_PARTY_NOTICES.md)
record the owner-directed redistribution assumption and the absence of an
independently verified grant. Preserve bundled vendor terms and notices.

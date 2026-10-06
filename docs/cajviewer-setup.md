<!-- SPDX-License-Identifier: MIT -->

# Fetching CAJViewer for optional tests

The CAJViewer tooling is no longer part of this repository. The installer
fetcher (`fetch_cajviewer.py`), the canary, the Docker/X11 automation that
used to be `tools/cajviewer/`, its tests and the manual installer-integrity
workflow moved to caj2pdf-samples in
[#360](https://github.com/rwv/caj2pdf-rust/issues/360):

- [`research/cajviewer/`](https://github.com/rwv/caj2pdf-samples/tree/main/research/cajviewer)
  holds the automation, the archived workflow and the former version of this
  guide as [`SETUP.md`](https://github.com/rwv/caj2pdf-samples/tree/main/research/cajviewer/SETUP.md);
- [`research/README.md`](https://github.com/rwv/caj2pdf-samples/tree/main/research/README.md)
  explains how to run the scripts against a caj2pdf-rust checkout.

The pinned vendor installers are still mirrored by
[rwv/cajviewer-binaries](https://github.com/rwv/cajviewer-binaries) in GitHub
Releases. Vendor binaries, captures and corpus files never belong in this
repository's Git history or release assets, and no converter test here needs
the viewer.

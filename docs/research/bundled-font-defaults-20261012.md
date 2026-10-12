# Automatic font defaults verification (2026-10-12)

Issue: #547, under #406. [Machine-readable receipt](bundled-font-defaults-20261012.json).

All 13 retained documents / 77 pages were converted with no font, mode,
alias, symbol, ornament or embedding arguments. CLI, Node and Chromium each
produced byte-identical PDFs to the final explicit original-font configuration
validated in the [Wine ornament report](wine-ornament-bands-20261011.md).
This preserves the prior semantic, resource, page-box and link evidence;
no new Wine/native-Windows or universal raster-parity claim is made.

The three paths used the version-1 pack of unchanged font programs. CLI reads
its embedded bytes offline; Node uses the npm-relative file; Chromium loads
and caches the npm-relative resource in OPFS. Maximum observed JavaScript
output writes were 262,144 bytes, within the existing default 256 KiB I/O
bound. Pack verification and browser download writes are at most 64 KiB.

Additional original controls cover both rendering modes, explicit overrides,
image-document defaults, disabled automatic fonts, cache corruption and
replacement, concurrent/offline cache reuse, truncated downloads, cancellation
while waiting for a cache lock, and actual packed Node/Chromium consumers.
Native installed-font discovery remains separately tested with bundled defaults
disabled. Optional external-corpus tests are excluded from compatibility passes.

A first adapter candidate emitted duplicate progress events during the font
probe; the existing monotonic-progress test caught it and the worker now
suppresses probe progress. A first audit script incorrectly assumed the
explicit reference run's 64 KiB output setting was the public default; the
actual existing default is 256 KiB. The corrected bound and all PDF hash
comparisons pass. Neither failed audit is counted as passing evidence.

All new implementation source is original MIT code. The per-file observation
migration review, immutable pack hashes, original font notices and reproduction
command are recorded in [bundled fonts](../bundled-fonts.md). No document corpus
or third-party converter implementation is included.

# Bundled native-text fonts

The CLI and JavaScript package automatically select the measured CAJViewer
7.3.151 Simple font preset for native C8/HN-B mode 2 and HN-B mode 0. Run
`caj2pdf paper.caj -o paper.pdf`, or call JavaScript `convert()` with its usual
source and sink. No font directory, rendering-mode selection or glyph arguments
are needed. Inspection and documents routed to image/PDF conversion leave the
font data unread.

The default preserves complete original font programs, glyph aliases, optional
CJK and Latin styles, mode-0 symbol identities and mode-2 ornament identities.
Semantic text is unchanged. Complete font programs can make small documents'
PDFs substantially larger than subset output. The defaults reproduce the
retained Wine reference configuration; they do not imply universal pixel parity
or support for previously refused document profiles.

## Assets and builds

Font-data version 1 is a 61,493,200-byte concatenation of ten unchanged font
files: nine CAJViewer resources and the measured SimSun collection. Latin and
decoration roles share stored bytes but retain distinct resource identities.
The SHA-256 is
`8d828782065b3a405736cb51e47a747d0848a61ca47958550e26915de39d68c0`.
[The manifest](../crates/caj2pdf-cli/font-pack.json) records each range and
original-file digest. [Font notices](../crates/caj2pdf-cli/FONT-NOTICES.txt)
retain the original embedded attribution; these data assets are separate from
the MIT-licensed converter source.

Native CLI binaries embed the pack and work offline, including in the scratch
container. Core and standalone WASM do not embed fonts or perform network I/O.
Registry source archives remain small. Building the native CLI needs `curl` to
retrieve the immutable
[font-data release](https://github.com/rwv/caj2pdf-rust/releases/tag/fonts-v1).
For an offline build, set `CAJ2PDF_FONT_PACK` to a local copy of that `.bin`.
Both paths verify size and SHA-256 before embedding. `cargo --offline` only
controls Cargo's registry access; use this override for a fully offline build.
The same override works for `node js/scripts/copy-wasm.mjs`.

npm includes the pack alongside the WASM module; Node opens ranged readers on
that file. Browser bundlers must serve the package's `caj2pdf-fonts-v1.bin`
resource, referenced through `new URL(..., import.meta.url)`. A native-text
conversion streams it to OPFS under a cross-worker lock, verifies the hash
through bounded WASM reads, and reuses the cache. A damaged cache is downloaded
again. Secure-context OPFS and Web Locks are required for automatic browser
fonts. Font writes and verification reads are at most 64 KiB; font data is never
copied wholesale into the WASM heap. First use requires network access; later
conversions can reuse the verified cache offline. Regular document conversion
still follows the existing cancellation and output-backpressure contract.

## Custom fonts and migration

Explicit CLI font options or `hnc8.fonts` objects override the entire default
preset and retain their prior subset/mapping behavior. CLI
`--no-bundled-fonts` restores installed-font discovery;
`--no-bundled-fonts --no-system-fonts` disables all automatic fonts.
JavaScript `hnc8: { fonts: false }` disables automatic font loading.
`--no-system-fonts` alone only disables installed-font discovery.

This changes default CLI/JavaScript native-text output and package sizes.
The Rust core still requires caller-supplied font resources. Integrators can
use `bundled_fonts::configure` with the documented resource order; raw WASM
hosts may use the new probe/preset exports, while existing explicit-font
exports remain supported.

## Provenance of the preset

The following JSON observation files were reviewed per file before migrating
only numeric correspondence data. They were originally authored for the Wine
query controls in #541, under the owner's MIT source policy. They contain no
HN/JBIG parser, decoder or third-party converter source. Preparation code was
not copied. The migrated tables are in
[`bundled_fonts/data.rs`](../crates/caj2pdf-core/src/bundled_fonts/data.rs).

| Observation | SHA-256 | Migrated facts |
| --- | --- | --- |
| mode0.json | `63b81dab416afd824216d8596d21cdb72af11046826ddfae52214bdc1c6a3001` | 12 Latin aliases |
| mode2.json | `947007decac43d596f8d79a241e30a6dd90eb8e9ad3a46fe46879c0bc26a601d` | 441 aliases over five Latin roles |
| mode0-symbols.json | `b116afc878f68cebc0000872a0a751961541d6d3a5fc228749ab8550457c1caa` | 21 symbol mappings |
| mode2-ornaments.json | `79bcf7bf4916d1a93b1b5a474c8684d40c060f5b4cd0733fee1c0be14e07c5d2` | Four ornament mappings and identity |

The pack reproduces the final mode-0 and mode-2 original-font manifests used
by the [Wine font report](research/wine-ornament-bands-20261011.md). SimSun
SHA-256 is `2e3cc9dbd787c827f73496f01633bba430e15c5db4d4f6ed9938386ea01f0171`;
its unchanged collection uses face 0. No external document corpus is included.

To reproduce the pack from the ten original files named by the manifest, run
`python3 scripts/prepare-bundled-font-pack.py SOURCE_DIRECTORY OUTPUT.bin`.
It uses bounded streaming copies, deduplicates the shared Latin/decoration
bytes, verifies every original file and the complete output, and refuses to
overwrite existing files. No font compiler or font table rewriting is involved.

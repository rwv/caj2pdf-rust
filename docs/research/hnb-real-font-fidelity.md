<!-- SPDX-License-Identifier: MIT -->

# HN-B real-font checkpoint (#277)

## Scope and result

The three admitted HN-B documents convert with real, explicitly supplied
substitute fonts through CLI, packaged Node and a Chromium Worker. All 14
pages retain the previous draw traces: page boxes, Unicode, positions,
transforms, image operations, paths and their order. Selected fresh vendor
captures confirm the major page sections and image placement. They do not
establish source-font identity or exact pixel parity.

No new parser defect was demonstrated. Font weight, bearings, character
widths and crowded punctuation remain visibly different. In particular,
the English references and small header text are not typographically
equivalent to the viewer. Do not advertise these profiles as visually exact.
No font fallback, font engine, parser change or new framework is introduced.

## Inputs and implementation

| External corpus input | SHA256 | Pages | Detailed source comparison |
| --- | --- | ---: | --- |
| issue-100 | `3f3b9b57d6925df811247dced47fd7fb74cf0f678ab9bfda0827c827258ab39b` | 4 | 1, 4 |
| issue-63 | `63870d12f2069d3d6c663dc2813d04a38a632c4c319f20b6f8dd1b8b8bc589c2` | 4 | 1, 4 |
| issue-65 | `e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49` | 6 | 1, 6 |

Baseline main is `ac6651b`. The CLI binary from core `4910da9` has SHA256
`d3eebe3e80e6ac87efc570f1584b3826f95087a33fd73a8ff38bb0b8ecdfd07d`.
Only Rust tests changed between that core and this baseline; production
conversion code is identical. Existing framing, geometry and image-payload
evidence in [HN-B controls](hnb-compact-index.md) remains applicable.

The actual npm tarball is from Release preflight run `37172633688`, head
`2362e936`, merged by the baseline. Tarball SHA256:
`63fa438b1f107c27819f86e9608d5e4b7b9aff910b827f8ce24876c46fdd5ece`;
packaged WASM SHA256:
`6764e7435f76428196d53fdedbf1ad7eb8b490584b5460f14af2d58e4b26781c`.
Its package version is still 0.3.1: this is a CI preflight, not a new release
or replacement for published assets.

## Explicit font resources

All ordinary roles use external caller-owned YaHei, SHA256
`1a2cc12d0faf71e488be59eb657aa08abd308a5d57ab3a117e4dd51ef66f4928`,
except issue-65's Latin role, which uses an external MS Gothic face extracted
from its TTC into standalone TrueType without editing outlines. Its SHA256 is
`3e9b1acba43b825942f01080c00551bd8615be7907057ccea70da39640f7d38b`;
source collection and face index are in `latin-font-selection.json`.
The two mode-0 cases also supply YaHei for the symbols and state-3 roles.
Neither resource is bundled, committed, or identified as the source font.

The role names do not guarantee a Latin-only character set. Issue-65's Latin
role needs U+2217 as well as CJK punctuation/fullwidth forms. YaHei and the
tested WenQuanYi face lack U+2217; DejaVu covers it but lacks other required
characters. These failed attempts are resource failures, not parser failures.
Check the complete observed role repertoire before choosing a substitute.

Reuse one source object in JS, or the same font path in CLI, for shared roles.
The existing API then embeds the font once. Creating independent sources for
the same file initially produced a 60,293,521-byte issue-100 PDF; reusing the
source reduces it to 20,583,151 bytes. No deduplication feature is needed.

## Runtime and whole-document checks

| Input | Output bytes | Identical CLI / Node / Worker SHA256 |
| --- | ---: | --- |
| issue-100 | 20,583,151 | `5bd909a4d1900e4e982bdc4c08cbe611da5cd4efe232dae9cdaa18ac9462eebe` |
| issue-63 | 20,811,281 | `31a1fe1656908b0de3edf55ca831c15961893cef8c76a468eb18d541caf8d41a` |
| issue-65 | 30,719,784 | `cf88dd7dd71d12c5d2e6000852b32fc9fa1ba965b5a72be317bc3926719ee65f` |

All three PDFs pass qpdf 12.2.0. All 14 pages render in MuPDF at 96 DPI and were
visually inspected. Their MuPDF 1.25.1 draw traces match the prior marker-font
outputs after removing only font name, glyph ID and glyph advance attributes.
Unicode, per-glyph origins, text matrices, page boxes and other drawing
operations remain in the comparison. This proves no transport/geometry
regression relative to that baseline, not independent source correctness.

Node uses ranged files and sequential file output. Worker uses OPFS spools
and sequential output; all completed scratch stores clear and all temporary
files are removed. Source/font/output requests are at most 262,144 bytes.
Post-conversion WASM capacity is 2,162,688 bytes in each case; this is not a
high-water or process-RSS measurement. The browser harness loads fixtures and
hashes finished PDFs outside conversion; those whole-file diagnostic buffers
are not a production streaming claim. Existing memory evidence remains in
[conformance](../conformance.md).

An initial browser attempt correctly rejected the harness's 16 MiB font
spool cap and cleaned up. A 32 MiB per-font cap admits the selected resource.
The failure receipt remains distinct from the successful retry. All runs
explicitly omit unverified bookmarks; #221 remains unresolved.

## Independent viewer observations

Fresh captures use the existing offline Linux viewer image
`sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`,
Xvfb 1600 × 1200 at 96 DPI, with its existing Noto CJK resource and no injected
marker fonts. Viewer zoom is 62% / 57% / 56% for issue-100 / 63 / 65.
The input hashes match the conversion sources. Earlier marker-font viewer
captures are useful for geometry only and are not reused as readable-font
visual references.

The selected page pairs cover titles, one/two-column body text, references,
English abstracts, the issue-65 leading image and final image page. Major
sections and ordering agree on all six inspected pages (PASS for this scoped
manual structural check). Exact appearance does not match on native-text
pages (FAIL for pixel parity); source-font attribution and quantitative
same-scale pixel comparison are NOT_RUN. Font outline/metric differences
and renderer differences have not been independently isolated from each
other. No numerical tolerance is introduced to turn this into a pixel pass.

This is not a fresh source transcription of every page or a searchable-text
guarantee. Preserve [Unicode and reading-order limits](hnc8-text-fidelity.md).
Unknown image modes remain errors; no new source profile is promoted.

External scripts, fonts, PDFs, traces and captures are under
`caj2pdf-hnb-real-font-277/`. `shared-runtime-results.json`,
`shared-cli-results.json`, `draw-trace-comparison.json`, `selected-pages.json`
and `checkpoint-manifest.json` record the exact scope and identities.
No external document, font, copied text or screenshot is added to Git.

# Wine native rule contrast

The native converter now draws admitted horizontal and vertical segments as
gray-127 device hairlines. Diagonal segments retain their black treatment.
This corrects visibly heavy header, table, fraction and identifier rules in
the 13-document / 77-page Wine cohort (#544, parent #406, related #541).
It is an empirical appearance approximation, not evidence that the source
stores an explicit gray pen. Coordinates and admission restrictions are
unchanged; vertical coverage and overlapping segments can still differ.

The user's acceptance criterion is normal-reading visual agreement with Wine
CAJViewer, including its original fonts. Harmless subpixel and antialiasing
differences are acceptable. Exact RGB counts below are diagnostics, not a
zero-difference completion gate. This criterion supersedes the stricter
pixel-equality implications in earlier research notes.

## Independent controls

The original MIT generator
[`wine_rule_controls.py`](../../scripts/research/wine_rule_controls.py)
creates three documents without external documents or font data:

- `rules.caj`: two pages, nine admitted tag/style combinations and three
  orientations. The second page selects black text before the same lines.
- `rules-phased.caj`: independent coordinate phases and reversed endpoints.
- `rules-mode0.caj`: the admitted legacy tag/style, with varied phases and
  reversed endpoints.

Native horizontal/vertical interiors are gray 127 at 75% and 150%, while
diagonals retain black antialiasing. The text-color switch does not change
the lines. The phase/reversal and mode-0 controls reproduce the distinction
at 95% and 125%. Five independently repeated native visits are exact.

Six PDF diagnostics distinguish possible corrections. Finite widths of two
and four source units and either PDF stroke-adjust setting retain baseline
pixels. An extremely small finite width loses lines and is rejected.
Gray-127 axis-aligned hairlines improve appearance without coordinate fitting.
The production implementation uses the existing bounded path writer and
preserves the existing diagonal writer. No target zoom or document identity
enters conversion.

## Actual pages and invariants

All 77 baseline source/PDF pairs were reviewed at native display scale.
Fonts, spacing, wrapping, columns and image composition broadly agree;
the excessive rule weight is visibly distinct. After the correction, all
77 output pages were observed again in Wine. Every changed region was
reviewed in 19 source/baseline/candidate atlases, without scaling or alignment,
with six-pixel row context. The region inventory covers every changed pixel;
14 unchanged pages retain their separately reviewed baseline appearance.

| Measure | Result |
| --- | ---: |
| Actual documents / pages | 13 / 77 |
| Changed axis-aligned rules | 347 |
| Pages with fewer differing pixels | 63 |
| Pages with identical previous rasters | 14 |
| Pages with more differing pixels | 0 |
| Differing pixels before / after | 2,874,038 / 2,803,270 |

Header/table rules and short code-identifier marks are visibly closer. No
new missing object, font or layout defect was observed. The short marks on
`90e7b47716c3` pages 2/3/5 are segment operations, not a remaining glyph bug.
Vertical coverage and overlapping lines, including `166d00147923` page 5,
retain raster differences; the correction is not a universal renderer model.

For each of the 13 PDFs, undoing only the new axis-rule command encoding
restores every previous page content stream byte-for-byte. Page boxes,
resource dictionaries, non-content stream object IDs, dictionaries and raw
payloads are exact. Original font programs, images, semantic text, coordinates
and all other drawing commands are retained. Independent source-order text
checks retain all 113,418 characters, and qpdf passes for every actual PDF.

The [metadata receipt](wine-visible-rules-20261011.json) records 55 cold
sessions, 54 admitted sessions and 140 admitted visits across controls,
diagnostics, actual pages and repeats. One six-page production startup never
reached a stable numeric layout; it is retained as a failure and independently
retried successfully. All sessions preserve source hashes, finish without OOM
and remove their containers. The reference is pinned Wine 8.0 32-bit /
CAJViewer 7.3.151 Simple at 1600 × 1200, 96 DPI; actual pages use 75% zoom.

The small original 75% control page is below the actual-corpus observer's
200-pixel threshold. Its separate observer requires one isolated paper
component exceeding 100 pixels, visible margins and stable complete frames.
Actual-corpus admission is unchanged. Initial generator mistakes and a raw
stream verification mistake were rejected before successful observation and
remain in the external record.

## Runtime verification and reproduction

The 13 actual inputs and three original controls produce byte-identical PDFs
in the native CLI, Node and real Chromium: 16 Node and 16 browser comparisons,
with output chunks at most 65,536 bytes. Three initial browser font-fetch
attempts failed; a long workspace scratch path then exceeded a Unix socket
path limit. A short workspace scratch path passed all 16 browser cases.
The nearly full `/tmp` is a plausible contributor, not an established cause
of the font-fetch failures. Conversion code did not change for the retry.

Locked workspace tests pass: 1,452 passed, seven ignored and excluded from
compatibility evidence. JavaScript tests pass: 192 passed, none skipped.
Locked native/WASM release builds, all-target/all-feature clippy with warnings
denied, rustfmt and whitespace checks pass. The original regression checks
both modes, reversed endpoints, diagonal preservation, black-text state and
subsequent semantic text using the existing partial-I/O fixtures.

Generate the original controls into a new directory:

```sh
python3 scripts/research/wine_rule_controls.py /tmp/wine-rule-controls
```

The three documents and manifest reproduce the observed bytes exactly. Convert
them with the same explicit caller-supplied mode-0/mode-2 font configuration
used for the [original-font work](wine-original-font-production-20261011.md), then open
source and PDF separately at the declared zooms in disposable copies of the
pinned profile. Confirm numeric page/zoom fields and two complete stable
frames. The receipt pins source/PDF/CLI/code/capture hashes and per-page
geometry; document, font and screenshot bodies remain external.

No API, dependency or input-buffering change is introduced. Two stack endpoints
feed the existing sequential path writer. Existing ornament tile seams and
missing native metadata/reference dashed underlines remain separate under
#541. The bundled help describes knowledge-link underlines, but the documented
toggle did not remove these particular marks; an overlay explanation is not
yet established. No complete fidelity or native-Windows equivalence is claimed.

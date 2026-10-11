# Original font programs through production conversion

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
The [metadata receipt](wine-original-font-production-20261011.json) records
all thirteen native-text documents / 77 pages, including the four-page HN-B
mode-0 case, with unchanged caller font programs and external character aliases.

The converter now supports this path in native Rust, CLI, raw WASM, Node and
browser APIs. CLI uses `--font-embedding full` and repeatable
`--font-glyph ROLE:CHAR=GLYPH`; Node/browser use `fonts.embedding` and
`fonts.glyphAliases`. Aliases select original cmap entries while ToUnicode or
ActualText preserves decoded text. Explicit symbol-code and ornament maps
keep their own selections. Used standalone TrueType programs are byte-exact;
TTC faces retain their original tables and glyph IDs with a rebuilt directory,
omitted DSIG and updated checksum. Defaults still subset fonts with no aliases.
See [CLI usage](../cli.md) and [host configuration](../io-architecture.md).

## Wine result

The reference remains CAJViewer 7.3.151 Simple under the pinned Wine profile,
1600 × 1200 at 96 dpi, single-page 75% zoom. The earlier bootstrap route is
retained for the four-page C8 PDF that needs it. No capture is resized or
aligned to manufacture a match. Admission requires numeric page/zoom checks,
two equal captures, complete page bounds, unchanged sources and successful
container cleanup without OOM.

Eighteen sessions include two unadmitted startups and seven cold repeated
views across three documents. All seven repeat frames agree exactly. The two
initial failures remain recorded: one lost its numeric fields during layout
transition; another left a black viewport with viewer crash-report/termination
messages. Fresh sessions succeeded after waiting for stable numeric layout
before editing zoom; the final admission criteria are unchanged. This does
not establish that all future viewer startups will succeed.

| Comparison against refined-scale subsets | Pages |
| --- | ---: |
| Fewer differing pixels | 67 |
| Unchanged image-only pages | 10 |
| More differing pixels | 0 |
| Matching source/PDF page extents | 77 |

There are **1,371,646 fewer differing pixels** in aggregate. Every actual page
still differs from its Wine source view. All 73 mode-2 page frames are exactly
the same as the preceding [original-program diagnostic](wine-original-font-inventory-20261011.json),
including the four pages that had regressed with prepared cmaps. Thus original
PostScript names, production CID assignment and TTC serialization retain that
observed improvement. The four mode-0 pages improve more modestly:

| Page | Subset differing pixels | Original full-program differing pixels |
| --- | ---: | ---: |
| 1 | 82,841 | 81,934 |
| 2 | 85,118 | 84,280 |
| 3 | 90,317 | 89,658 |
| 4 | 32,366 | 31,573 |

## Independent checks

All thirteen outputs pass qpdf and preserve **113,418 semantic characters**
in independently extracted source order through explicit ToUnicode/ActualText.
An unused mapped resource can have an empty ToUnicode map; every painted CID
still requires a corresponding mapping. Node produces identical PDF bytes for
all thirteen documents with bounded 64 KiB writes. Synthetic real-Chromium
OPFS tests also agree with Node for complete fonts and independent glyph/text
identities; the actual corpus has not been run in browser UI sessions.

The original MIT checker
[`verify_original_font_output.py`](../../scripts/research/verify_original_font_output.py)
compares **113,630 glyph draws / 10,638 unique resource-document glyph pairs**.
It resolves each PDF CID, checks decomposed outlines, advances, bearings,
units-per-em and PDF widths, and compares all other drawing operators and
page boxes. Every used standalone program matches the supplied original;
collection tables match except the documented DSIG/checksum changes.
Semantic source-order and Wine raster comparisons remain separate checks.

```sh
python3 scripts/research/verify_original_font_output.py \
  subset.pdf original-full.pdf --fonts /external/font-pack > verification.json
```

The external pack's `manifest.json` lists `fonts[].output` and optional
zero-based `face` indices. This whole-file diagnostic uses fontTools and
pikepdf; it is separate from production's bounded font reads. The metadata
receipt includes font checksums, aliases, output hashes and capture identities.
It contains no font binaries, corpus documents, PDFs or screenshots.

Complete programs increase these PDFs to 21,732,714–32,029,986 bytes. Used CFF
full embedding remains explicitly unsupported. The observed source is Wine;
native Windows and other zoom/DPI combinations are not inferred. Recoverable
placement and raster differences keep #541 and the parent fidelity work open.

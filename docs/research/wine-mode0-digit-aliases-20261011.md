# Wine mode-0 digit and parenthesis aliases

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
Original-font controls identify a configuration omission: mode-0 letters use
ordinary Latin cmap slots, but its ten digits and two parentheses use private
HGBZ slots. The earlier recipe supplied only mode-2 Latin aliases and mode-0
symbol aliases. This follow-up corrects that recipe through the existing
external glyph-alias API. Converter code, geometry and archived font programs
remain unchanged.

The [metadata receipt](wine-mode0-digit-aliases-20261011.json) records 28 fresh
Wine sessions and 31 admitted page visits, including three exact cold repeats.
The reference remains CAJViewer 7.3.151 Simple in the pinned Wine profile at
1600 × 1200 / 96 dpi. Earlier actual-source and query views are explicitly
reused, rather than counted as fresh observations.

## Measured mapping and cause

The [resource query study](wine-font-resources-20261011.md) already recorded
these slots in separate low/high-byte controls and reversed glyph order.
Its statement that all mode-0 Latin glyphs use ordinary slots was too broad.
The correction is specific to these measured HGBZ mappings:

| Semantic character | Raw code | Observed HGBZ cmap slot |
| --- | --- | --- |
| `0` | A3B0 | U+656F |
| `1` | A3B1 | U+6571 |
| `2` | A3B2 | U+6573 |
| `3` | A3B3 | U+6575 |
| `4` | A3B4 | U+6576 |
| `5` | A3B5 | U+6578 |
| `6` | A3B6 | U+6579 |
| `7` | A3B7 | U+657A |
| `8` | A3B8 | U+657B |
| `9` | A3B9 | U+657C |
| U+FF08 | A3A8 | U+84FA |
| U+FF09 | A3A9 | U+8502 |

For example, HGBZ's ASCII `1` and U+6571 have distinct glyph IDs, 18 and 2578.
Their font-unit bounds are `[216, 0, 792, 1368]` and `[664, 288, 1428, 1672]`,
with advances 1024 and 2048. Moving the ASCII glyph cannot recover these shape
differences. Missing parenthesis aliases also caused a CJK-font fallback;
the corrected configuration selects the measured HGBZ resource. Text remains
ASCII digits and fullwidth parentheses through the existing semantic mapping.

Supply the aliases with the unchanged measured HGBZ program. For example:

```sh
caj2pdf input.caj -o output.pdf --no-system-fonts --fonts /external/mode0-pack \
  --font-embedding full --font-glyph latin:U+0031=U+6571
```

This abbreviated example supplies only `1`. The complete recipe needs all
twelve entries and the existing 21 mode-0 symbol mappings. Mode 2 retains its
separate 441 Unicode/font pairs. The API does not infer slots from font names.

## Independent controls

Four initial original documents isolate six characters in six styles, in both
native modes. The second page in each mode reverses glyph/style order and
shifts raw positions by +13/+7. Baselines use 75% and 150%; reversed controls
are held out at 100%. Both modes have exact cold source repeats at 150%.
Source and PDF use unchanged original font programs.

Before correction, each mode-0 frame has 20/36 identical tight glyph bitmaps,
compared with 24/36 in mode 2. Afterwards, mode 0 also has 24/36. All twelve
remaining shape differences in each frame belong to the unequal-axis rows.
Tight crops remove translation solely to diagnose shape, not page fidelity.

Two further original mode-0 pages contain all twelve mapped characters plus
Latin/Han anchors across six styles and an explicit 36-by-36 profile: 98 glyphs
per page. The held-out page also changes `801d/0` to `801d/4`. At each zoom,
the corrected shape diagnostic improves from 10/98 to 70/98 exact bitmaps;
all 28 remaining shape differences are in unequal-axis rows. Anchor cells
retain identical before/after pixels. A third cold source repeat is exact.

Fidelity comparisons preserve complete page bounds and strict RGB differences,
without registration, scaling or tolerance:

| Control | Before, changed pixels | After, changed pixels |
| --- | ---: | ---: |
| Six-character grid, 75% | 1,124 | 1,091 |
| Six-character grid, 150% | 3,060 | 3,002 |
| Reversed six-character grid, 100% | 1,756 | 1,672 |
| Full digit/parenthesis grid, 75% | 2,841 | 2,214 |
| Full digit/parenthesis grid, 150% | 7,971 | 5,770 |
| Reversed full grid, 100% | 4,309 | 2,849 |

Five full-grid cells have slightly worse strict scores: digit `3` in the two
unequal-axis rows and digit `6` at size zero, at 75%; digits `0` and `1` at
size zero in the held-out 100% page. These are retained, not fitted away.
Every whole frame improves. Correct glyph identity still leaves positioning
and axis-rasterization differences discussed in the [rounding study](wine-mode0-font-rounding-20261011.md).

## Actual document and validation

All four pages of the inventoried mode-0 document improve at 75%:

| Page | Before, changed pixels | After, changed pixels |
| --- | ---: | ---: |
| 1 | 81,934 | 81,176 |
| 2 | 84,280 | 83,559 |
| 3 | 89,658 | 89,080 |
| 4 | 31,573 | 31,110 |

The total reduction is 2,520 pixels. Extents match; every page still differs
from Wine. The other 73 inventoried mode-2 pages retain their prior observations:
their recipe and converter are unchanged, and their views are not rerun or
counted as fresh compatibility tests here.

The prepared original mode-0 pack reproduces all five observed alias PDFs
byte-for-byte. Independent checks preserve semantic order, glyph matrices,
gray values and page boxes. Used TTF programs remain byte-exact; a used TTC
face preserves its original tables except the established standalone checksum
and DSIG handling. Parenthesis resource changes are admitted explicitly.
Mode-2 preparation preserves all 441 aliases and ten supplied resource programs.
The optional remapped mode-0 recipe changes only HGBZ's `cmap` and `head`;
it is not the original-program Wine comparison configuration.

Initial parity harness attempts omitted optional font roles, yielding different
PDF object inventories. Preliminary structural assertions incorrectly included
unused fallback programs, assumed parenthesis resources would remain unchanged,
and treated a TTC face as a byte-identical standalone TTF. Corrected checks
distinguish intended resource changes and collection handling from geometry
changes. These diagnostic failures do not establish converter regressions.

CLI, Node and real Chromium produce byte-identical PDFs for both complete
control grids and the four-page actual document, with output chunks bounded
to 65,536 bytes. Initial Chromium font fetches failed while the system tmpfs
was 97% full. Using a short workspace-disk temporary directory succeeded;
an intermediate long path exceeded the Unix socket limit. These failures
remain recorded separately from successful conversion and Wine evidence.

## Reproduction and limits

The original MIT, standard-library generator reproduces all six source controls
and their fixed cell coordinates byte-for-byte:

```sh
python3 scripts/research/wine_original_glyphs.py /tmp/new-original-glyph-controls
```

The receipt includes source/capture hashes, repeats, prior-query mappings,
per-cell scores and validation. Fonts, documents, PDFs and screenshots remain
external. No vendor implementation or differently licensed converter code was
inspected or copied. This correction does not complete #541's placement/fidelity
criteria or establish native Windows equivalence.

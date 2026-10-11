# Wine CAJViewer native font resources (2026-10-11)

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under
[#406](https://github.com/rwv/caj2pdf-rust/issues/406), tracks font and geometry
fidelity against Windows CAJViewer 7.3.151 Simple running under Wine. The
[metadata receipt](wine-font-resources-20261011.json) records the original
controls, font-query counts, input/output identities and page comparisons.
This is Wine evidence; it does not establish native-Windows equivalence.

## Font selection

Original numbered rectangle fonts distinguish resource selection from glyph
shape. For Chinese controls, retaining localized family names is necessary:
adding a Windows BMP cmap alone did not make the initial HGHT control load.
This resolves the earlier unconfirmed Wine Han marker in #531.

Thirty-two admitted CJK control sessions decode 283 markers, with two stable
frames per visit, confirmed navigation, unchanged inputs, no OOM and complete
container cleanup. The two authored Han characters select:

| Admitted raw style | Observed resource | Explicit converter role |
| --- | --- | --- |
| High bits `0x1000` | `HGSS_CNKI` (Huaguang Song) | `cjk` |
| High bits `0x0800` | `HGHT_CNKI` (Huaguang Hei) | `cjk_sans` |
| High bits `0x0c00` | `HGKT_CNKI` (Huaguang Kai) | `cjk_kai` |
| Measured `0x0400` / `0x1400` styles | `SimSun`, face 0 | `cjk_system` |
| Style-zero explicit axes and measured `6084`, `64c6`, `9c84`, `a4a5`, `b94c`, `e58c` | `SimSun`, face 0 | `cjk_system` |

The receipt lists each exact observed style/profile. C8 Han selection agrees
in character modes `80ce/0` and `80ce/1`, and across Latin states 0, 3, 4, 28
and 31. HN-B modes 0 and 2 use the same observed Han families. The font must be
selected from the original raw style: geometry aliases such as C8 `0508` to
`1108`, or HN-B `04xx` to `10xx`, otherwise erase the system-font distinction.
The converter still validates geometry separately and does not admit new
record/style profiles through these font options.

The first eleven system-font experiments supplied CJK-only generated fonts
and failed canonical UI startup; they produced no admitted page observations.
The revised system controls preserve the surrounding system-font bytes and
change only the two control outlines in disposable local copies. No such
font copies are included here. Marker IDs agree at thresholds 162 and 192;
162 is the midpoint of the measured foreground/background values. This
threshold decodes authored holes; it is not a page-comparison tolerance.

## Latin and symbol queries

The 310 controlled letter/digit mappings select HGBZ, HGBX, HGHZ, HGB1 and
HGB1X at states 0, 3, 4, 28 and 31. Forward, reverse and isolated controls
confirm those finite mappings. Merely supplying these font names at ordinary
Unicode slots does not reproduce mode-2 resource glyphs.

The original [query-font generator](../../scripts/research/wine_font_query.py)
encodes the queried BMP cmap argument as two bytes in separate visits, plus
the resource identity. Eighteen generated fonts reproduce the observed bytes
exactly with fontTools 4.62.1. For example:

```sh
python3 scripts/research/wine_font_query.py \
  --family HGBZ_CNKI --resource 1 --shift 0 /tmp/HGBZ-query-low.ttf
```

A full-em rectangle has twelve possible holes: for bit `b`, the hole begins
at `(100 + (b % 4) * 220, 100 + (b // 4) * 300)` in a 1000-unit em, and has
size 120 by 180. The encoded value is `resource * 256 + queried_byte`.
These controls contain only original geometric outlines. They are not text
fonts and must be used in disposable viewer profiles.

The complete corpus inventory has 520 non-CJK raw-code/role/profile
combinations: 425 mode-2 and 95 mode-0. Twenty initial query sessions observe
all mode-2 combinations but leave 21 mode-0 symbols unmarked. Four additional
sessions identify all 95 mode-0 combinations after adding HGFX to the resource
set. Reversing the complete inventory in twenty further sessions reproduces
all 520 queries, plus 24 known sentinels. Original font cmaps contain the
observed slots; queries are not accepted solely because the control fonts
provide extra coverage.

Mode 0 queries ordinary Latin Unicode slots, while mode 2 uses resource
aliases. A single globally remapped Latin pack is therefore incorrect.
Combining the controlled ASCII mappings and actual mode-2 observations yields
441 Unicode/font pairs without conflicts within mode 2. Experimental derived
fonts change only `cmap` and the `head` checksum; glyph IDs, outlines and
metrics remain unchanged. All 21 mode-0 symbols select HGFX; the existing
explicit symbol-glyph API keeps each decoded character independent of its
visible glyph. The converter does not bundle these fonts or infer these
Latin/symbol mappings.

## Real documents and limits

The explicit mode-specific font packs convert all 13 documents and 77 pages.
Qpdf accepts every output. An independent bounded record/ToUnicode check
confirms all 113,418 source characters in order, including mapped symbol CIDs,
ActualText and nested nonsemantic artifacts. An intentionally wrong symbol
ToUnicode entry fails the check. Ten of the 77 pages contain only images;
their raster differences are not attributed to font selection.

Twenty-six primary Wine sessions request every original/output page at 75%.
All original pages and 73 output pages have complete, stable independent page
crops. One four-page output (`3f3b9b57d692…`) fails canonical startup, and a
cold repeat retains a viewer stack-overflow diagnostic. Two initial bootstrap
attempts open the unchanged PDF but reset the requested zoom to 176%, so they
are not accepted. Reapplying single-page layout and 75% **after** opening the
PDF confirms all four pages in two further cold sessions, with identical
crops. The original failures remain in the receipt. This is a viewer-route
workaround, not a conversion or PDF-byte change.

Across the 77 admitted source/output pairs, 35 have different page extents
and 42 have different pixels at equal extents. None is claimed pixel-exact.
No registration, resampling, column removal or fitted tolerance is used.
The new Han roles correct visible font-family substitutions, but character
positions, punctuation, decorations and device rasterization remain separate
obligations. The existing decoration alias is measured for ornament 46 only.
Non-Han characters retain their previous resource-selection rules in this
change. #541 remains open for broader resource and geometry fidelity.

With the new roles absent, all twelve mode-2 documents (73 pages) produce
bytes identical to the unmodified current-main CLI with the same eight font
inputs. The new roles retain bounded ranged reads, sequential output and
shared-source deduplication. Native tests, Node and real Chromium cover the
new role selection, both C8 character modes, HN-B modes 0/2, fallback and
unchanged character identities. Optional external-corpus tests not configured
for the general test suite are not counted as compatibility passes.

All added converter, fixture and query-generator source is original MIT work.
Vendor programs remain opaque. No foreign converter or private HN/JBIG
implementation was inspected or copied. External documents, font binaries,
PDFs and screenshots remain outside this repository.

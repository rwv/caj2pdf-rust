# Wine font programs: full fonts and retained metadata

Embedding the complete prepared caller font programs reduces source/PDF
pixel differences on four measured page views. Restoring only the omitted
`cmap` recovers part of this improvement; `gasp`, `OS/2`, `name` and `post`
alone do not change any measured pixels. Full fonts still do better than
restoring all five tables together. This identifies another recoverable
font-fidelity problem for [#541](https://github.com/rwv/caj2pdf-rust/issues/541),
without establishing the PDF renderer's internal cause.

These are external diagnostic PDF variants, not a production subsetting
change. The [receipt](wine-font-programs-20261011.json) records 18 new Wine
sessions: 14 initial variants, two cold repeats and two corrected metadata
variants. All retain the old `240/2473`
coordinate scale to isolate font effects. The separate
[coordinate correction](wine-page-extents-20261011.md) is not combined with
full fonts in this experiment. No font bytes or derived PDFs are committed.

## Controls and observations

Two inputs cover pages 1, 2 and 5 of actual document `d6a23a9cfd0f…` at 75%,
and the original 64-glyph `04c6` CJK-mode alias control at 200%. Each variant
is opened in a fresh pinned Wine CAJViewer 7.3 session. Numeric page/zoom
confirmation, two stable captures, complete independent page bounds,
unchanged input, no OOM and container cleanup are required. Strict RGB
comparison uses equal page extents with no resizing or fitted placement.

| Page/control | Current subset | Subset + cmap | All five tables | Complete program |
| --- | ---: | ---: | ---: | ---: |
| `d6a…`, page 1 | 43,297 | 37,276 | 37,276 | 32,985 |
| `d6a…`, page 2 | 90,169 | 82,885 | 82,885 | 78,566 |
| `d6a…`, page 5 | 50,305 | 41,256 | 41,256 | 37,886 |
| Original `04c6` | 9,078 | 7,158 | 7,158 | 6,720 |

Values are differing pixels against the same admitted source reference.
Every complete-font view improves but remains nonzero. The two fresh
complete-font sessions reproduce all four page views pixel-for-pixel.
`gasp`, `OS/2`, `name` and `post` restored individually are each pixel-identical
to the original subset output on these views. Restoring all five tables is
pixel-identical to restoring `cmap` alone. The receipt retains source/PDF and
baseline/variant comparisons separately.

Each ordinary TTF complete-program variant embeds the exact prepared caller
font bytes. TTC input is serialized as a standalone first-face TrueType font.
The CIDToGIDMap changes to original glyph IDs; used CIDs, content, PDF widths,
page boxes and ToUnicode do not change. Empty unused font resources remain
unchanged. Qpdf accepts all 14 variants, and extracted text is identical to
the corresponding baseline.

An independent fontTools decomposed-outline comparison verifies 951 unique
used glyph pairs in the actual PDF and 64 in the original control. Outlines,
advances, bearings and units-per-em agree between original subsets and full
programs. All page content, boxes, PDF widths and ToUnicode streams also
agree. This check establishes equivalent used glyph geometry; it does not
assert equivalence of every font table or renderer decision.

For the metadata-only variants, all existing subset tables except the
checksum-bearing `head` remain byte-identical. The corrected cmap preserves every
source cmap alias whose original glyph is retained, remapped to subset IDs;
it does not add unused outlines. The tested sources have cmap formats 0/4.
Original subset GIDs and CIDToGIDMap remain unchanged. Production already
retains TrueType hint-program tables; the observation does not justify
claiming that a specific hinting or auto-hinting mechanism is responsible.
Unused glyphs, other tables and renderer classification remain hypotheses.

## Reproduction and next implementation work

The original MIT research script accepts explicit external PDF/font inputs:

```sh
python3 scripts/research/wine_font_programs.py baseline.pdf /tmp/font-variants \
  --fonts resources.json
```

`resources.json` maps each PDF font resource, for example `"/F0"`, to the
same prepared TTF/TTC originally supplied for that resource. Relative paths
resolve against the JSON file. The baseline must use the converter's ordinary
Unicode-CID TrueType mapping; this is not a general PDF font-replacement tool.
Use fontTools 4.62.1, pikepdf 10.5.1, qpdf and pdftotext. The script writes
seven variants plus hashes and validation metadata in a new directory. Twelve initial PDF hashes reproduce byte-for-byte; the two corrected variants
have new hashes and fresh Wine observations.
Fonts and documents remain caller-provided; no resources are downloaded.

The research script processes fonts/PDFs in memory outside the converter.
A production implementation must preserve ranged font access, bounded
allocation and sequential PDF output on native, CLI, Node and browser. Full
programs can substantially increase output size: one observed CJK subset is
300,112 bytes versus a 13,739,804-byte complete program; the observed system
font face is 18,204,236 bytes. Avoiding that size by discarding font data needs
independent proof that the resulting output retains the measured fidelity.

Reproduction exposed two prototype state bugs. Its first cmap-only variant
copied a lazy format-0 subtable before decoding it, retaining raw original
glyph IDs instead of remapped IDs in that subtable. The second input's
OS/2-only variant reused metadata mutated while compiling the first input.
The published script explicitly decodes cmap subtables before copying and
isolates copied metadata. The old hashes and observations remain in the
receipt; corrected variants are pixel-identical to their prototypes in all
four fresh page views, leaving the table above unchanged. These
prototype problems never entered production font code.

The generator and verification are original MIT work using public font/PDF
library APIs. No vendor renderer, private HN/JBIG implementation or foreign
converter source was inspected or copied. Only source, documentation and
metadata enter Git. The sample is four page views across two inputs; no
all-corpus or native-Windows font-program conclusion is claimed. #541 remains
open while complete-font preservation and the remaining rendering differences
are unresolved.

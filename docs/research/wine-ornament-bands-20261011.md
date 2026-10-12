# Continuous ornament bands from original fonts

The visible double-rule seam on source `b9a64bf99e4b`, page 1, is removed by
painting its original ornament glyph once across the existing clip. Font
programs, vertical size/origin, page geometry, semantic text and link
appearances remain unchanged. This is a narrow shape-based correction for
#541 / draft #542; general patterned ornaments retain their existing repeats.
The [metadata receipt](wine-ornament-bands-20261011.json) records the rejected
first candidate, independent controls, final comparisons and runtime checks.

## Shape and bounded implementation

The selected resource is `HGBZ_CNKI`, ornament 117 / U+5D95. Its original
2,048-unit glyph contains two unhinted rectangular contours spanning
x=-16..2048; y spans are 520..648 and 1152..1696. It is not the separate
ornament-1 glyph. Independent fractional PDF glyph draws introduce visible
internal seams even though the vector bands cover the nominal em.

The original MIT `pdf/font/bands.rs` probe follows the OpenType
[`glyf`](https://learn.microsoft.com/en-us/typography/opentype/spec/glyf) and
[`loca`](https://learn.microsoft.com/en-us/typography/opentype/spec/loca)
formats. Only explicitly mapped native decoration resources are probed:
no more than the default alias plus four mapped aliases. Each probe reads two
location entries and at most 512 glyph bytes through existing ranged I/O.
It accepts at most eight separated, full-em rectangular contours with four
on-curve corners each, matching bounds/side bearing, and no glyph instructions.
Curves, inset/notched shapes, overlapping contours, composites and CFF outlines
retain ordinary placement. Source I/O failures and cancellation propagate.
No font name, vendor alias or actual-document identity is hardcoded in the
predicate. No font program is rewritten and no vector line is guessed.

For qualifying glyphs, one nonsemantic draw retains its original y transform
and clip and uses twice the clip width as its horizontal em. The excess is
clipped. Full-width horizontal bands have unchanged vector coverage under
this overdraw. The extra width prevents the endpoint contraction observed
with the first candidate in Wine's PDF renderer. It is a coverage guard, not
a new source unit, target-DPI pitch or per-document offset. The exact internal
cause of the renderer's contraction is not established.

## Independent controls and rejected candidate

[`wine_ornament_bands.py`](../../scripts/research/wine_ornament_bands.py)
uses the project's original nine-row C8 geometry control. Six styles, a second
ornament value and two explicit axes are exercised at 75% and 125%. The fonts
are independently authored pairs of rectangles: full-width, inset, and
full-width with a harmless glyph instruction. No vendor outlines or document
text enter the generator. With fontTools 4.62.1, source/font hashes reproduce
exactly. In a disposable pinned Wine profile, replace only the ornament
resource with the corresponding original control font.

The first candidate used exactly one clip width. It fixed the actual header
at 75% and 100%, but **failed the independent full-width controls**: several
rows ended visibly early. Those views and successful runtime checks are
retained as rejected-candidate evidence, not final compatibility passes.
Extending the same glyph beyond the unchanged clip restores all nine row
lengths at both zooms. The inset and instructed controls remain byte-identical
to baseline output, with 306 glyph draws each; the full-width control changes
306 draws to nine. The negative controls still exhibit the previously
recorded source/PDF pitch and seam differences: they establish selectivity,
not general pattern fidelity. Small band-weight and vertical pixel-phase
differences also remain in the accepted positive controls.

Generate controls outside the source tree:

```sh
python3 scripts/research/wine_ornament_bands.py /tmp/original-band-controls
caj2pdf /tmp/original-band-controls/bands.caj -o /tmp/bands.pdf \
  --no-bookmarks --no-system-fonts --font-embedding full \
  --font-cjk /tmp/original-band-controls/full.ttf \
  --font-latin /tmp/original-band-controls/full.ttf \
  --font-decoration /tmp/original-band-controls/full.ttf \
  --decoration-char '嵁' --decoration-glyph 1=U+5D04 \
  --decoration-glyph 2=U+5D05 --decoration-glyph 46=U+5D41 \
  --decoration-glyph 117=U+5D95
```

The receipt's observed conversions retain the existing external text resources
and replace only the decoration font; the standalone command above needs
only the generated font because this control contains no ordinary text.
It is a portable reproduction of the ornament behavior, not a promise of the
same PDF resource graph or file hash as that observed setup.

## Production verification and limits

All 13 actual documents / 77 pages are reconverted. Only `b9a64bf99e4b` page 1
changes: 47 artifact draws become one. The remaining page commands, all page
boxes, resolved font/image resource streams, 133 link annotations and 113,418
semantic characters remain exact. The other 12 documents are byte-identical.
The three other actual ornament-bearing pages keep their original output.
Qpdf accepts every actual and controlled PDF. Fresh production Wine views
agree exactly with the visually inspected overdraw diagnostic at both zooms.

The actual header now appears continuous like the native source at normal
reading size. Whole-page RGB differences versus Wine remain: 12,351 pixels
at 75% and 24,830 at 100%, versus 12,505 and 24,874 for repeated glyphs. These
counts are diagnostics, not the acceptance gate. All four affected actual/
positive-control pairs and four negative-control pairs were visually reviewed.
Earlier complete 77-page visual review remains applicable to the unchanged
content; this step does not claim 77 newly captured GUI pages.

Native CLI, Node and Chromium verification covers the 13 actual documents and
three independent font controls, with identical bytes and writes bounded to
65,536 bytes. Workspace tests pass: 1,466, with seven ignored optional tests
excluded. Final ornament regressions, strict all-target/all-feature clippy,
release CLI/WASM builds, 192 JavaScript tests and source/static checks pass.
Original tests cover short reads, location formats, cancellation/source faults,
truncation, packed flags, bounds, overlapping contours and semantic isolation.

No universal pattern/zoom/renderer equivalence is claimed. General repeated
ornament cadence, link dash cadence, small image-edge and antialiasing
variations remain documented. External fonts, documents, screenshots and PDF
artifacts stay outside this repository. No vendor/private converter source is
read or migrated. Merge/release review remains separate.

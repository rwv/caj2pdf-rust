<!-- SPDX-License-Identifier: MIT -->

# C8 real-font checkpoint (#278)

## Result and limitations

The admitted six/four/five-page native profiles complete with real caller
fonts in CLI, packaged Node and a Chromium Worker. All 15 output pages were
rendered and inspected. Page structure, images and native drawing transport
remain intact. **The substitutes are not a source-faithful typography preset:**
English headings, references, abstracts, punctuation and some formulas are
crowded or overlapping, especially in the four/five-page profiles. Character
coverage alone does not establish usable appearance or compatible font metrics.

Seven selected fresh viewer comparisons confirm the major content sections
and image placement. Exact appearance fails; same-scale quantitative pixel
comparison and source-font identity are NOT_RUN. The evidence does not isolate
font outline/metric effects from all renderer effects or establish a new
parser defect. Keep native C8 experimental; do not change source positions or
add per-document width corrections to make these substitute fonts look closer.

## Reproducible inputs and resources

The production core is unchanged from the [HN-B checkpoint](hnb-real-font-fidelity.md).
Baseline main is `8d44a3d`; the same pinned CLI and actual npm tarball from
Release preflight run `37172633688` are used. Their exact SHA256 identities
and the preflight-versus-published-release distinction are recorded there.

| Input | Source SHA256 | Pages | Selected viewer pages |
| --- | --- | ---: | --- |
| issue-66 | `90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6` | 6 | 1, 2, 6 |
| issue-90 / 4-[21] | `1b9aa912e0bc9dcce4dfbefa07e08a24665d6db624a02f9a30a8c404df5a9ef7` | 4 | 1, 2 |
| issue-90 / 4-[24] | `03770ea1cdebc9856d1443438b296860fa11f3e283717d256ea0af57ecae1c89` | 5 | 1, 2 |

Explicit MS Gothic supplies the ordinary Latin role; YaHei supplies CJK,
alternate Latin and states 3/28/31. These are the same external resources,
with the same hashes, as the HN-B checkpoint. Font cmap coverage is checked
against each observed role: MS Gothic covers the required U+2217 and fullwidth
punctuation that the tested YaHei/DejaVu alternatives cannot cover together.
The state-role fonts are explicit substitutes, not inferred source families.
Shared roles reuse the same source objects/paths to avoid repeated embedding.

The six-page document uses an explicit MS Gothic U+25BA decoration alias.
It is a nonsemantic single triangle, visibly different from the viewer's
two-arrow ornament. Preserve the established [decoration classification](c8-native-records.md#six-page-comparison-and-decoration-classification):
neither doubling the repeat count nor altering offsets is justified by this
substitution. No fonts or document-derived assets are distributed.

## Runtime and regression results

| Input | Output bytes | Identical CLI / Node / Worker SHA256 |
| --- | ---: | --- |
| Six-page | 29,745,646 | `a2fb46e11ed6f007d2013ee5734011ac23205d23aae2a7ecaf5acdc8c2032bdb` |
| Four-page | 29,885,086 | `c25d61926c404f484e1a47a68c80aae5b410f2cc864467af94ae9124f7e959d1` |
| Five-page | 30,282,307 | `1add6dda59c8f18f606e7e21637649dc4b071a034a12a82a8ca1155d982db246` |

All three pass qpdf 12.2.0 and render with MuPDF 1.25.1 at 96 DPI. Their draw
traces preserve every page box, Unicode character, glyph origin, text matrix,
image operation, path and ordering from the previous marker-font outputs,
after excluding only font names, glyph IDs and advances. One deliberate
exception is the six-page final divider: 45 glyph aliases change from `A`
to U+25BA inside empty ActualText spans. Their count, positions, transforms
and nonsemantic classification match. The unmodified trace comparison first
reports that difference; a separate check verifies this exact exception.
This is transport regression evidence, not independent source-text validation.

Node uses ranged files; Worker uses OPFS spools. Both write sequentially,
clear scratch and leave no temporary files. Maximum font/output requests are
262,144 bytes; source requests peak at 2,453 / 78,936 / 139,607 bytes.
Final WASM capacity is 2,293,760 / 2,490,368 / 2,490,368 bytes, respectively;
these are post-conversion capacities, not peak live heap or process RSS.
The diagnostic server buffers fixtures and hashes completed output separately.
Existing converter memory limits and the earlier peak-memory evidence apply;
this run adds no whole-process memory claim. Bookmarks are explicitly omitted
and remain owned by #221.

## Independent selected-page observations

Use the same offline viewer image/Xvfb recipe as the HN-B checkpoint, without
marker-font injection. Fresh source hashes match the converter inputs.
Displayed zoom is 57% for the six-page document and 56% for the others.

| Pages | Scoped manual structure check | Residual appearance differences |
| --- | --- | --- |
| Six-page 1, 2 | PASS: title, figure, prose/code blocks and drawing segments remain represented. | Monospaced-looking Latin substitutions, crowded punctuation and different weight; code is not guaranteed copyable. |
| Six-page 6 | PASS: code tail, both reference/abstract blocks and divider are present in order. | Different English spacing and single-triangle decoration; exact appearance FAIL. |
| Four-page 1, 2 | PASS: two-column structure, formulas, figures and captions occupy corresponding sections. | Header/caption/formula text overlaps or has different metrics; exact appearance FAIL. |
| Five-page 1, 2 | PASS: headings, formulas and three diagram groups retain their page structure. | Latin and mathematical glyph shapes/spacing differ and overlap; exact appearance FAIL. |

Other pages have full output-render and draw-trace inspection, not fresh
independent viewer comparison. The final English abstracts also show severe
substitute-font crowding. No pixel tolerance or whitespace normalization is
used to label that a fidelity pass. Existing character-mapping controls and
[Unicode limits](hnc8-text-fidelity.md) remain separate evidence. No new
source profile, OCR, font discovery or reading-order behavior is added.

External evidence is in `caj2pdf-c8-real-font-278/`: `selection.json`,
`font-coverage.json`, `cli-results.json`, `trace-and-runtime-comparison.json`,
`decoration-comparison.json`, `checkpoint-manifest.json`, runtime receipts,
scripts, captures and renders. This report records a bounded verification
with unresolved appearance limits, not unrestricted native C8 fidelity.


## Same-resource control follow-up

To distinguish a converter scaling error from incompatible font outlines,
reuse original `alphabet-0-1-upper.caj` from `alphabet_documents()` in
`tools/cajviewer/c8_native_control_fixture.py`. It contains 26 fullwidth
uppercase letters and two Chinese anchors at style `10a5`. Supply exactly
one renamed copy of the external YaHei font to both the converter and the
viewer's HGHT resource path. Only name identifiers are intentionally edited; cmap, glyph outlines and
metric tables are byte-identical. Diagnostic font SHA256:
`36141ff37507092ec1b5befa56064e6ae32528fcecd96b7169bf41414610f637`.
The font remains external and is not a distributable project asset.

The ordinary control retains a 170-source-unit column step. A second original
control changes only that step to 60 units. Both PDFs pass qpdf. With the
same font, both the viewer and PDF show the dense letters and final Chinese
anchors overlapping. The well-spaced control has 28 isolated glyphs in each
render; fixed-page-normalized component bounds differ by at most 3.244 viewer
pixels at displayed 364% zoom versus a 192-DPI PDF render. This is a measured
bound, not an exact pixel pass or a threshold adjusted to accept the result.
Different raster scales, integer viewer sizing and hinting remain unisolated.
An initial equal-cell calculation split glyphs and is excluded; the recorded
component calculation checks all 28 complete glyphs without registration.

The four-page final abstract's low-page trace includes 648 CJK-resource glyphs
at the body matrix `10.4651169 0 0 10.4651169`, plus its smaller text and other
roles. Its English appearance therefore depends heavily on the CJK font's
fullwidth Latin outlines. Fresh default-resource and renamed-YaHei viewer
captures retain the source layout but show different letter appearance;
substitution also produces crowding in the viewer. The first default capture
was taken before the requested document opened and is excluded; the confirmed
capture visibly identifies the correct source and page 4/4.

These controls demonstrate that tight source positions can overlap with this
font in both implementations. They do not justify a universal scale correction,
ASCII normalization, moving source positions or an implicit font fallback.
They also do not prove every remaining symbol/style difference is font-only.
The current bounded verification is complete with its recorded limits; native
C8 remains experimental and exact source-font typography remains unverified.

Receipts, original controls, scripts, name-only font copy, captures and hashes
are external under `caj2pdf-c8-real-font-278/same-font/`. In particular use
`alphabet-component-bounds.json` and `checkpoint-manifest.json`; do not cite
the excluded equal-cell measurement as a layout result.

<!-- SPDX-License-Identifier: MIT -->

# Observed C8 native records

Implementation tracking: #232 (parser), #233 (rendering), parent #229.
This note records source observations, not an accepted rendering profile.
The production converter must keep rejecting required unknown semantics.

## Six-page inventory

The issue-66 source has SHA-256
`90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6`.
Each indexed text span ends with a four-byte `8004` record. There are no
remaining bytes inside those spans after that record. The file still has
1,449 bytes after the last indexed text span; they are the application-info
block classified below.

| Page | Text offset | Text bytes | Images | Drawing starts `8006` | Additional unresolved high words |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 200 | 4,756 | 1 | 2 | `8072` |
| 2 | 7,421 | 7,944 | 0 | 73 | `8072`, `8073`, `8074` |
| 3 | 15,365 | 3,924 | 2 | 6 | `8072`, `8073`, `8074` |
| 4 | 21,969 | 5,336 | 0 | 2 | `c054`, `8072`, `8073`, `8074` |
| 5 | 27,305 | 4,584 | 0 | 25 | `c054`, `8072`, `8073`, `8074` |
| 6 | 31,889 | 7,536 | 0 | 1 | `8010`, `c053`, `8072`, `8073`, `8074` |

These are aligned-word observations with the three measured 28-byte image
records excluded from the four-byte survey. They do not prove all record
boundaries: drawing points and unknown payloads must not be counted as glyphs.
Each page also contains `8001`, `8002`, `801d`, and `8067` words.

The measured `8006/a381` and `8006/a38b` sequences have two coordinate pairs
followed by `ffff/0005`. The `8006/a383` sequence instead has two pairs followed
immediately by a state-setting record; a parser must not require or scan for a
universal `ffff` drawing terminator. Page 6 contains an `8010/0001` sequence
with two pairs and `ffff/0005`. Style, stroke and drawing semantics remain
unverified. The `c053`/`c054` words occur among text-like records; treating every
high word as a drawing opcode or every following pair as a glyph is unsafe.

## Additional controlled observations

Four source copies each change one little-endian 16-bit word. A whole-file
comparison verifies that no byte outside that word differs. All four control
captures and both source navigation states repeat identically in their own
state, using the pinned offline CAJViewer 9.0.0 at 96 DPI and 100% zoom.

| Word offset | Mutation | Observed effect on page 2 |
| ---: | --- | --- |
| 28 | 4652 → 4752 | Content shifts left approximately 13 pixels; page frame remains fixed. |
| 30 | 4274 → 4374 | Content shifts up approximately 13 pixels; page frame remains fixed. |
| 7,451 | `a0c4` → `a0da` | The selected Latin D becomes Z. This tests the selected pair, not all A0 codes. |
| 7,465 | 5335 → 5435 | The first short horizontal drawing changes, supporting a drawing-point interpretation. Stroke semantics are unresolved. |

Control file SHA-256 values, in the same order:

- `052813daadb19321e67c085d35667b0695722b6456f8f4725702430b94f3e5ee`
- `0f1f3821ff94e93f1fd3c3b15f2ba9f4ace3ca84cd3a5fa973c5205c3cc48469`
- `69a46ec552544bba86a8e168e190ba610705dfed4c05783b12cce0af159b858b`
- `8ca5ef6bdb6b1ac7f6a11bb20e627b28c808d6c7eb2a5e152f906efcc29df50f`

These observations support subtractive origin words and selected native field
interpretations. They do not establish physical units or exact PDF transforms.
Returning to the source tab and navigating again changed 23,719 pixels within
the unchanged 661×967 page crop. Some control differences also extend beyond
the intentionally changed glyph/segment. Retain both navigation phases;
these are field-semantics controls, not an approved pixel-parity baseline.
No alignment, resizing or tolerance was used to hide those differences.

External evidence directory: `caj2pdf-c8-fields-20261001`, including
`controls.json`, `integrity.json`, `aligned-word-inventory.json`, action logs,
original/repeated captures and both comparison phases. Source documents,
mutants, extracted text and captures are deliberately not committed.

## Implementation boundary

Reuse the existing bounded reader and character helpers. Establish explicit
record framing before exposing glyph/vector events. Track raw style words
without assigning font names or silently ignoring style changes. Verify the
remaining Latin map, image placement, header/footer controls, special text
words and font resources before admitting complete conversion. Unknown
required records remain located errors. Parser-only success does not complete
#229 or #233; all six pages must eventually preserve their visible content.

## Rust parser boundary

`Hnc8Reader::visit_native_records` visits the current page using existing
`TextBudget`, range reads and cursor poisoning. It retains one fixed 28-byte
record buffer and run state, allocates no parser-owned heap storage, and awaits
each visitor before reading the next record. Reads respect `Limits`; the largest
record-payload request is 24 bytes. This low-level API is deliberately raw:
`NativeRecord::Glyph::code` is not a Unicode scalar, and `Control` values are
never silently discarded. No CLI/JavaScript conversion route is enabled yet.
The renderer must resolve or reject unknown style/character semantics.

The admitted framing is `8001`, `8002`, observed `801d` values 0/4, observed
`8067` values 5/6/8/9, raw controls `8072..8074` and `c053/c054`, the three
measured `8006` forms, the `8010/1` coordinate form, the 28-byte `800a/d300`
image record, and an exact-end `8004`. Drawings and images are atomic visitor
events, including marker-looking payload bytes. Unknown tag/value pairs stop.
The current page's declared image count must agree before end-of-page success.

The initial limited parser probe of all six source pages gave the following
**incomplete prefixes**, not successful extraction or conversion:

| Page | Visited records | Raw glyph records | Drawings | Images | First unsupported byte/tag |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 1,064 | 929 | 1 | 1 | 4,492 / `8072` |
| 2 | 1,729 | 1,317 | 72 | 0 | 15,201 / `8072` |
| 3 | 909 | 810 | 5 | 2 | 19,109 / `8072` |
| 4 | 1,201 | 890 | 0 | 0 | 26,773 / `c054` |
| 5 | 167 | 130 | 4 | 0 | 28,021 / `c054` |
| 6 | 902 | 793 | 0 | 0 | 35,497 / `8010` |

The source hash was rechecked after the probe. The external harness and report
live in `caj2pdf-c8-native-probe`; neither extracted glyph codes nor source text
are included in the report. Original synthetic tests cover state changes,
asymmetric glyph order, marker-like image/drawing payloads, both drawing end
forms, every record truncation boundary, span/count/working limits, unknown
controls, short reads, source/visitor errors, cancellation and an abandoned
suspended visitor. Raw non-ASCII, Latin and invalid codes are preserved, not
misrepresented as decoded characters. The character helper below now covers the verified alphanumeric/GB18030 subset.
The still-required special characters, controls and rendering remain #229/#233.

## Character mapping and ordinary-copy controls

`decode_native_character` reuses the existing factual GB18030 table without
allocating a String per glyph. It returns the standard two-byte character for
ordinary codes, and ASCII letters/digits for the verified A0-prefixed codes.
The 62 alphanumeric codes are `a0b0..a0b9`, `a0c1..a0da`, and `a0e1..a0fa`.
The three explicitly verified symbols listed below are also mapped. Other A0
codes, private-use mappings and invalid sequences return `None`.
A visitor that requires complete text can reject `None`; the reader retains
the failing source-record offset and poisons the cursor. Raw glyph events still
preserve the original code. The helper is not a full-page text extractor.

Evidence was acquired with the same pinned offline viewer, at 100% zoom and
96 DPI, using the visibly labeled ordinary Copy menu, **not enhanced copy or
OCR**. A fresh distinct clipboard sentinel was installed before each copy.
The page-2 selection rectangle was `(548,218)` to `(1137,1060)`; it excludes
the final body paragraphs and page furniture. Original bytes remain external.

- Source selection: 2,459 UTF-8 bytes / 1,523 code points, SHA-256
  `b5bd244865f329c798b5dd92a371c16f1171832e45d7865eb721d586e87ea240`.
- Original uppercase/lowercase/digit alphabet control: 62 non-space glyph-code
  words replaced with the corresponding A0 codes; all other bytes unchanged.
  Control SHA-256 `a214562e8474b6e7579c67b8c3efa0e2cc1dc0be2fa97c49c5d84ec6313a4549`.
- Control ordinary copy: 2,417 bytes / 1,542 code points,
  SHA-256 `7def702a95f42df4291c190a344bcd8804e478bac83254786af677864d2b3137`. Its first 62 non-whitespace
  characters exactly equal `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789`.
  Whitespace removal was used only to align this invented alphabet; retained
  raw clipboard bytes were not normalized or substituted as a text baseline.

The unchanged source selection independently contains 40 distinct A0 letter
codes that agree with that rule. Ordinary copy is not a universal Unicode
oracle: it changes some fullwidth digits to ASCII, produces U+0082 for the
standard GB18030 fullwidth comma code, and maps special A0 ampersand/AA symbol
codes differently from an ordinary GB18030 decode. The helper does not copy
those comma/digit transformations or guess unknown special-code semantics.
The three symbol exceptions have since received separate original controls,
as described below; other unverified codes still fail explicitly.
Source record order has only been compared for this selected body region;
full-document reading order is not established.

The external evidence directory `caj2pdf-c8-copy-20261001` retains the raw
clipboard payloads, visible menu/selection captures, mutation manifest, full-file
integrity checks and comparison report. Only format facts and hashes are
recorded here. No extracted document text, source document or font is committed.

## Style-word follow-up (not yet a font contract)

Four more original controls change only the style word at source offset 210,
from `0884` to `0885`, `08a4`, `0c84`, or `1084`. Each source/control capture
repeats identically with the same page frame. The first two changes affect only
the selected five-glyph run (589 and 466 changed pixels, respectively); the
other two produce zero changed page pixels. In the first control, the low-field
increment visibly increases glyph height. The control file labels `width` and
`height` were initial hypotheses, not established field names.

All observed source style words have equal low five-bit and next five-bit
fields. The asymmetric controls support investigating independent glyph-size
fields, but do not establish size units, font selection or an accepted style
bit layout. Zero visible difference does not authorize ignoring high bits.
Runtime diagnostic font requests include Fangzheng/CNKI names; the viewer has
its own font resources. Those log observations do not identify the effective
font for each source style or license any font for redistribution. Keep style
words raw until the rendering work validates a reproducible font contract.

## Application-info tail and source coverage

The remaining interval at 39,425 begins with two little-endian u32 lengths:
10,400 decoded bytes and 1,424 compressed bytes. A bounded zlib decode consumes
exactly those 1,424 bytes, produces exactly 10,400 bytes, and leaves the 17-byte
ASCII trailer `APPINFOSIGN 39425`. The compressed/decoded SHA-256 values are:

- `33de306020c1aa748b10094d04ebb6928b4057a99b2dae2dd285387884981f17`
- `fd2920fa45820856482b89577239587e2a9a5c4ee60339fc4f9c4e11bbecd4b5`

The decoded XML is an application `Package` with a `Note-Package` and
`FileProperty-Package`. There are 23 Link entries, each containing one rectangle
and one UrlLink. Their page counts are 13 on page 1, one each on pages 2–4, and
seven on page 6. This explains the 13 entries in the viewer's page-1 annotation
panel; it is not a table of contents or embedded font resource. No URL was
followed. XML and link/text contents remain external.

An independent interval check accounts for every source byte without gaps or
overlaps: the 200-byte header/index, six indexed text spans, three image
descriptors/payloads, and this application-info block total 40,874 bytes.
No separate font resource interval is observed. Font identifiers within text
records and the effective viewer font selection are still unresolved; this
accounting does not authorize a guessed font or omission of source controls.
It also does not infer bookmark absence for every C8 variant (#221).

The external `application-info-summary.json`, `application-info-structure.json`
and `source-span-coverage.json` retain the bounded decode and interval checks.
The ordinary-copy/runtime smoke evidence supplements the frozen #223 report;
it does not retroactively make that report's intentionally limited prototype a
complete extractor.

## Original fixed-position style controls

The style subset of `tools/cajviewer/c8_style_fixture.py` builds nine original 392-byte documents
without reading an external source. Each has one text-only page, eight rows
and the test characters `中文AM1`. The observed header identifier and structural
constants are retained as format facts; their necessity is not established.
The grid varies the fields by row; the other documents repeat one field set
at identical positions, permitting direct comparisons without alignment.

```sh
python3 tools/cajviewer/c8_style_fixture.py /tmp/c8-style-controls
```

The output directory must be new. The manifest contains input hashes and
raw per-row values. Open the files with the existing pinned offline viewer
recipe, close the annotation sidebar and explicitly set `100%` zoom after
opening each tab. The initial fit-to-width zoom differs and is not a valid
comparison. All generated documents were byte-identical to the observed
inputs. The grid SHA-256 is
`5cdcadbd6559d0c50c21a16e1fa59eb0e729e383e9a0cf937bf6ec584d795d7c`.

At 96 DPI, the fixed page crop was `(494,178)` to `(1155,1145)`; each capture
repeated identically. Compared with `style=1084, 801d=0, 8067=6`:

| Changed field | Changed page pixels | Observation |
| --- | ---: | --- |
| style `1085` | 3,524 | Increased glyph height. |
| style `10a4` | 3,218 | Increased glyph width. |
| `801d=4` | 1,580 | Changes confined to the Latin/digit columns; Chinese columns unchanged. |
| `8067=5`, `8`, or `9` | 0 each | No visible difference for these glyphs and state. |
| style `0884` | 0 | No visible difference for these glyphs and state. |

There was no resizing, registration or pixel tolerance. These independent
controls corroborate the selected source mutations, but do not determine the
size lookup, font identities, baseline metrics or all state interactions.
In particular, `801d=4` is not established as a universal bold flag, and a
zero pixel difference is not permission to discard a control. Keep the raw
parser/rendering boundary until those required semantics are resolved.

External receipts are in `caj2pdf-c8-grid-20261001`: `variants.json`,
`comparison.json`, action logs and paired captures. Viewer images/fonts are
not bundled. Successful display of these original controls does not satisfy
the six-page source conversion requirement in #233.


## Remaining record framing and full-span traversal

The same original generator now also emits seven control variants and three
coordinate-record variants. Controls are inserted after each row's context
and before its glyphs: `8072/0`, `8073/38`, `8074/0`, `c053/5200`,
`c054/5200`, `c053/5700`, and `c054/5700`. Each is four bytes. All seven
preserve every following glyph and produce zero changed page pixels against
the baseline, with identical repeat captures. Combined with the aligned source
inventory, this establishes the admitted framing, not their rendering meaning.
The visitor now delivers these controls and their full u16 payload unchanged.

The coordinate variants insert, before each row, a 16-byte sequence consisting
of `8010/1`, `(5200, 4800 + row*500)`, `(6300, 4850 + row*500)`, and
`ffff/5`. A second control changes the first x to 5400. Neither produces a
visible line. Replacing only the opening pair with `8006/a381` produces eight
sloped lines (3,566 changed page pixels). All captures repeat identically.
Thus `8010/1` uses the observed two-pair/end framing but must **not** be
rendered automatically as an `8006` stroke. `NativeRecord::Drawing` preserves
its tag, style and points without promising a visible drawing. Other `8010`
values remain unsupported; terminator mismatch and truncation remain errors.

The extended file-backed parser now consumes each indexed source span exactly:

| Page | Records | Glyphs | Coordinate records | Images | Largest read request |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1,178 | 1,015 | 2 | 1 | 24 |
| 2 | 1,768 | 1,338 | 73 | 0 | 20 |
| 3 | 952 | 840 | 6 | 2 | 24 |
| 4 | 1,329 | 986 | 2 | 0 | 20 |
| 5 | 1,072 | 801 | 25 | 0 | 20 |
| 6 | 1,879 | 1,658 | 2 | 0 | 20 |

The original source hash is unchanged. `caj2pdf-c8-full-record-probe/result.txt`
is the external receipt; control/drawing manifests and comparisons are in
`caj2pdf-c8-grid-20261001`. The generator reproduces all 19 viewer input files
byte for byte. Original Rust regressions check retained marker-looking control
payloads, atomic coordinate payloads, short reads and all new truncation
boundaries. No parser buffer or output API is added.

This completes traversal of the sample's records, not conversion, Unicode
coverage or rendering semantics. The public route remains disabled pending
#233. A renderer must explicitly interpret or reject each required control;
none is silently dropped by the parser.


## Three verified symbol exceptions

A complete raw-glyph inventory across the six source pages found only three
codes rejected by the character helper: `a0a6` (8 occurrences on page 2),
`aab3` (2/17/5/12 on pages 1/2/4/5), and `aca3` (2 on page 3).
Counts are decimal. The 6,638 total glyph records include spacing characters;
this is not a reading-order or text-layout claim.

The original generator's `symbols` fixture places `aab3 a0a6 aca3 a3a6 a3aa`
in five columns and repeats them on eight rows. `symbols-permuted` places
`aca3 a3aa a0a6 a3a6 aab3` and changes `801d` to 4. In the pinned offline
viewer at 100% and 96 DPI, the first three symbols visibly appear as an
asterisk operator, ampersand and filled right-pointing triangle. The standard
GB18030 fullwidth ampersand/asterisk columns serve as separate controls.

Fresh distinct clipboard sentinels preceded each selection. The visibly
labeled ordinary **Copy** menu (not enhanced copy/OCR) produced exactly eight
rows of the expected symbol order, allowing whitespace only between symbols:

| Native code | Unicode mapping |
| --- | --- |
| `a0a6` | U+FF06 FULLWIDTH AMPERSAND |
| `aab3` | U+2217 ASTERISK OPERATOR |
| `aca3` | U+25BA BLACK RIGHT-POINTING POINTER |

The earlier real-source ordinary-copy observations independently agree for
`a0a6` and `aab3`. The reordered original control confirms the association for
all three and rules out a stale clipboard result. Font appearance and Unicode
identity are separate: do not derive glyph width from the word “FULLWIDTH”,
and do not extend this exception to arbitrary A0 punctuation or private-use
codes. Tests retain rejection of neighboring unverified codes.

Fixture SHA-256 values:

- `symbols`: `e9c413d7b70494fa3ec699baf350bcf281946cf47a6c50d85bc6974f46cefdcf`
- `symbols-permuted`: `c825648f82dba851333c081b31ec8126434ff98bf423132dc81e426566d993c2`

Raw ordinary-copy SHA-256 values, respectively:

- `d4dc44574a9e80fd3abc52af8f62733ff27ff613d1ec56e6598605e849e5e824`
- `4f5b050cf0d83b3830be642a850f87e2558bbd41c55a5a4010556f0fdf5fa846`

The helper now maps these three explicit exceptions without allocations.
The same six-page source probe reports zero unmapped glyph codes; this does
not prove complete visual conversion, correct font resources or reading order.
The generator reproduces all 21 original inputs byte for byte. Captures,
clipboard bytes and probe outputs remain external under the existing evidence
directories.


## Runtime font-call observations (2026-10-01)

An original external MIT `LD_PRELOAD` shim forwards the public FreeType
size/transform calls unchanged and logs only face family/style, units per em,
size arguments and matrices. It reads no viewer implementation or glyph
outlines. The pinned offline viewer opened the original baseline, vertical,
horizontal, weight and size-square controls. At 100% zoom, each captured page
rectangle `(494,178,1155,1145)` is pixel-identical to its earlier capture without
the shim. This checks the observed rendering, not arbitrary instrumentation
transparency.

For the baseline's `中文AM1` glyphs, the observed CNKI faces are `HGHT_CNKI`
(256 units/em) and `HGBZ_CNKI` (2048 units/em). With only `801d` changed from
0 to 4, calls instead include `HGHT_CNKI` and `HGHZ_CNKI`. This agrees with
the previously measured change confined to the Latin/digit columns, but does
not establish a universal bold flag or the font mapping for every character.
Neither face names nor units/em identify a redistributable font resource.

The vertical control produces nonuniform horizontal matrices while changing
the requested pixel height; the horizontal control changes the horizontal
matrix while retaining the corresponding baseline pixel heights. The final
recorded CNKI pixel-height requests are 11 for baseline, 13 for vertical, 11
for horizontal and 11 for weight. The final vertical/horizontal matrix xx
values are 55453 and 77451 respectively (yy is 65536). These describe this
viewer execution; they are not a physical point-size lookup table. Opening
and resizing also produces thumbnail/intermediate-scale calls, so the full
trace must not be treated as a list of document font sizes.

FreeType documents pixel sizing and 16.16 transform matrices in its
[sizing and scaling reference](https://freetype.org/freetype2/docs/reference/ft2-sizing_and_scaling.html).
The next rendering step still requires associating each admitted raw style
with reproducible font resources, source-space dimensions and baseline
placement. Do not infer those values solely from hinted raster bounds.
External receipts are in `caj2pdf-c8-fonttrace-20261001`: original shim,
per-control traces, captures and action log. No proprietary font, source
content or runtime trace is committed.


The generator also reproduces the original `size-squares` control: eight rows
with both five-bit size fields set to 3 through 10, fixed high bits `0x1000`,
and the original string `■□田国中`. Its SHA-256 is
`7d89555d34b6a5e4d3d0aee357b4c03db6197354b4690029a3394fa4391a860f`.
Generation was checked byte-for-byte against the externally observed control.
The manifest records every raw row style and character code. This is a size
comparison fixture, not an accepted physical-size mapping.

A bounded zoom follow-up on the baseline records CNKI pixel-height requests
12, 14, 17, 20 and 23 at requested zooms 110%, 125%, 150%, 175% and 200%.
Requests at 400% and 800% were not accepted: the visible zoom field remained
200%; they provide no high-zoom size evidence. Empty traces at previously
rendered sizes also do not establish absence of glyph rendering. Derive a
source-space size rule from independent controls before adopting a PDF font
size; do not use the final pixel request as a zoom-independent point size.


### Viewer font character aliases

A follow-up original forwarding shim also observes public `FT_Get_Char_Index`
and `FT_Load_Glyph` arguments. The original baseline at 100% remains
pixel-identical to the earlier uninstrumented capture. For its Chinese glyphs,
observed HGHT lookups include Unicode U+4E2D and U+6587. For its visible Latin
A, however, HGBZ is queried with U+7CA4 (31908), yielding glyph ID 4851.

The independent `letter-a` fixture contains only eight occurrences of raw code
A0C1 at fixed positions. It confirms the HGBZ U+7CA4-to-4851 lookup and actual
loads of glyph 4851 at several pixel sizes. The existing generator reproduces
this 264-byte control; SHA-256 `930089a6d884f58e5b4365b6e1d1d8073b676822ce83bd5498589739a9298b1d`.

This does not change A0C1's independently verified textual meaning, Unicode A.
It demonstrates that the vendor font's character slots cannot be assumed to
match document Unicode. The shared PDF writer's standard Unicode font contract
is still valid for explicitly supplied Unicode fonts; passing a viewer font
unchanged is not established as faithful rendering. No generic font remapper
or proprietary font data is added. Font choice/substitution must stay explicit,
and baseline/size rules remain unverified. External traces, shim and captures
remain in `caj2pdf-c8-glyphtrace-20261001`; no outlines or font programs are copied
into the repository.


An independent `digit-one` control (eight raw A0B1 codes) confirms actual HGBZ
glyph-2578 loads. Its SHA-256 is `71368e44e4cae4133f6d61cac050c9a37963670cebec70a7debde708f90e8402`; the generator reproduces it exactly.
Metadata-only reads of the SFNT directory, loca, ten-byte glyph headers and
horizontal advances reveal a material distinction: glyph 2578 has advance
2048 and bounds `[664,288,1428,1672]`, while ordinary Unicode 1 (glyph 18) has
advance 1024 and bounds `[216,0,792,1368]`, in 2048 units/em. No outline
coordinates or font programs are exported. A/ M's observed aliases have the
same bounds and advances as their ordinary slots; this does not prove outline
identity. Consequently neither universal alias equivalence nor one uniform
compensation factor is justified. The raw character-to-Unicode mapping remains
unchanged. Measurements are external in `actual-glyph-metrics.json` and
`digit-one-trace.tsv` under the glyph-trace directory.

## Size ladder and first text-page preview

The original `size-ladder` fixture repeats `中文AM1` on twelve fixed rows,
using equal size fields 1 through 12 and high bits `0x1000`. It is 536 bytes,
SHA-256 `648a2074f743d3c588e48058f5847a9cc1775aba0c320fdbd9c3244987d04f7e`.
The generator reproduces the observed input exactly. The pinned viewer's
100% captures repeat identically. A metadata-only extension of the public
FreeType forwarding shim records bitmap dimensions/bearings after loading;
its baseline page is pixel-identical to the prior uninstrumented baseline.
No glyph bitmap or outline is exported by the shim.

A fresh six-page source inventory narrows the required equal size fields to
**2, 3, 4, 5, 6 and 8**. Page 2 uses field 5 for all 1,338 glyph records. This
bounds the first renderer's required size investigation; it does not establish
all 32 possible field values or their physical units.

An external Rust preview now exercises the existing record visitor and shared
PDF writer on actual page 2, which has no source images. It emits all 1,338
glyphs and the 73 observed `8006` coordinate records. The experiment explicitly
substitutes caller-provided WenQuanYi Zen Hei and Latin/symbol fonts. It tests
candidate source-space sizes `[60,70,80,90,105,120,140,160,180,210,240,280]`,
subtractive header origins, and baseline `raw_y + candidate_height`, using the
existing empirical coordinate factor. It models the coordinate records as
thin segments. **Those size/baseline/stroke choices remain hypotheses, not an
admitted production rendering profile.**

The preview PDF passes qpdf validation and text extraction, and renders to
661×967 pixels at 96 DPI. The independently captured viewer page frame is
`(494,156,1155,1123)` after navigating to page 2; it is not the synthetic
single-page frame at y=178. All four edges are visible and the page capture
repeats identically. The first comparison reveals visible font-shape, spacing
and stroke differences. Switching the explicit Latin substitute from DejaVu
Serif to Liberation Serif does not eliminate those differences. No alignment,
resizing or tolerance is used to turn this preview into a fidelity claim.

External receipts are in `caj2pdf-c8-metrics-20261001` (original controls,
public-call metadata and captures), `caj2pdf-c8-full-record-probe` (style
inventory), and `caj2pdf-c8-render-preview-20261001` (original experimental
harness, explicit font resources, PDF/raster/text and hashes). These artifacts
stay outside Git. The preview is restricted to an actual image-free page and
refuses any source image, so it cannot silently omit diagrams. It is not a
six-page conversion, a public CLI/JS route, or a successful completion of
#233. Next compare the candidate glyph geometry against the original controls
before admitting rules, then integrate the existing image emitters in source
draw order for pages 1 and 3.


## Original geometric-font control (#240)

`tools/cajviewer/c8_geometric_font.py OUTPUT` generates three original fonts
with 1000 units per em, an empty missing glyph, a square and two half-square
outlines. It reads no font input. It requires the external MIT fontTools tool
(observed version 4.62.1), not a converter runtime dependency. Resource-slot
family names and observed character aliases select the controlled glyphs;
these files contain no vendor outlines. Generate outside the repository and
mount over the corresponding viewer resources read-only in the isolated
viewer container. Never replace installed host fonts.

The style generator's `size-profile` has six rows, equal size fields
2, 3, 4, 5, 6 and 8, two glyphs per row, row spacing 350 and page height 3200.
Its SHA256 is `ff55ebd71ec505dc0ec26d971571126308544445878f642858ef3fc7dcd95db0`.
This shorter original page keeps all edges visible at 200% zoom. It and all
three generated fonts reproduce the external experiment byte for byte.

In the pinned offline viewer, repeated captures at each zoom were identical.
The original square's measured ink widths, rounded to integer pixels, were:

| Zoom | Field 2 | 3 | 4 | 5 | 6 | 8 |
| --- | --- | --- | --- | --- | --- | --- |
| 193% | 17 | 19 | 22 | 26 | 30 | 40 |
| 194% | 18 | 19 | 22 | 27 | 30 | 40 |
| 197% | 18 | 20 | 22 | 27 | 31 | 41 |
| 198–200% | 18 | 20 | 23 | 27 | 31 | 41 |

These observations contradict interpreting the preview's candidate table as
`floor(size * 320 / 2473 * zoom / 100)`. For example, size 105 at 194%
predicts 26 pixels, while field 5 measured 27. They do **not** establish a
replacement size table or an unmodified vendor-font rendering rule. Font
hinting, viewer size policy and page rasterization must remain distinct.
The public font-call trace contains intermediate scales and cached glyphs;
its order is not sufficient to associate every call with a final-page row.

Short pages are vertically centered: the observed outer frame at 200% is
(164, 247, 1485, 1076), unlike the earlier tall-page frame. Always measure
all four page edges again. No content alignment or fitted baseline offset
was applied. External captures, traces and measurement receipts remain in
`caj2pdf-c8-square-font-20261001`; none are bundled as passing fidelity tests.
The isolated transition and coordinate controls below supersede this earlier
missing observation.
Production font size, baseline and segment rules remain unapproved.


### Isolated coordinates and segment controls (#240)

External receipt `caj2pdf-c8-isolated-size-20261001` uses the same original
geometric fonts. A single field-5 glyph requests/renders 26 pixels at 193%
and 27 at 194%; before/after trace snapshots isolate those calls. Revisiting
each zoom reproduces the screenshot exactly. Changing page width from 5105
to 4000 preserves both requests. The transition is not a page-width effect.

Automatic fit-to-width accepts much higher magnifications than the earlier
manual zoom attempt. Six original 300 × 230 pages at displayed 2896% use
origin (4652,4274) and one square glyph at (4672,4294). For fields 2,3,4,5,6,8,
the final bitmap sizes are 269,298,336,404,461,606 pixels. All frames are
(449,231,1574,1093); all dark glyph bounds start at (599,250). The UI zoom is
rounded and dark bounds are not exact fractional outline bounds; these
measurements alone still do not define a point-size table.

With field 5 fixed, adding 20 to glyph x moves the dark left edge from 599
to 674; adding 20 to glyph y moves the top from 250 to 325. Adding 20 to
header x origin moves the left edge to 524. Adding 20 to both origins and
both glyph coordinates preserves all glyph bounds. The isolated positive
y-origin control clips the glyph at the page edge and is not a full-height
measurement. These establish subtractive origins and translation direction
without fitting document-specific offsets.

A segment at the same raw starting point as the square begins at the same
horizontal position but a different vertical position. Text baseline and
segment placement must therefore not share an unverified y correction.

Three original diagonal segments, tag 8006 with styles a381/a383/a38b, were
rendered on one page. Each has dx=100 and dy=20, with starts at x=4672 and
y=4304,4354,4404. The a383 record has no ffff/0005 terminator; the other two
do. All three remain thin solid segments at both displayed 2896% and 200%.
At an interior column, integrated darkness relative to black is approximately
1.04 pixels at both magnifications (1.047 for all three at 200%). Thus the
preview's fixed positive source-space stroke width is contradicted: it would
grow with zoom. PDF's existing zero-width hairline operation is the candidate
for this observed profile; endpoint, color and independent PDF rendering
comparison remain to be completed before admitting a production rule.
The style generator now includes `draw06compact` and `draw06alternate` with
their correct record framing for further independent controls. No screenshots,
external font data or source-document text are committed.

### Core origin metadata and first segment PDF

`Hnc8Reader::header().native_origin` now exposes the two unsigned C8 words
at offsets 28/30 using one bounded four-byte read. HN-A/HN-B return `None`:
their corresponding bytes have not been established as native origins.
Subtract in signed or floating-point arithmetic; a coordinate below the
origin is not unsigned overflow. The field does not apply a viewer margin,
font baseline correction or an image transform. Explicit `Header` literals
must include the new field in this unstable API.

An original external Rust diagnostic passes the three-segment control through
the native-record visitor and existing PDF writer. It uses the candidate
20-unit x/y margin and zero-width hairlines, with the existing empirical page
scale. qpdf validates the resulting PDF. Reading the origin through the core
instead of the diagnostic's ad hoc header read produces identical bytes.

Opened in the same pinned viewer, source and PDF both have the complete frame
(449,231,1574,1093). At x=800, their three ink-weighted y positions are
458.232/458.162, 645.368/645.462 and 832.989/832.736 respectively. Integrated
black-equivalent width is approximately 1.04 pixels for both. These are
unregistered measurements, not an exact pixel match: rasterization and
subpixel placement still differ. This supports continued hairline comparison
but does not complete segment fidelity or native page conversion. Diagnostic
source is under `caj2pdf-c8-render-preview-20261001/src/bin/segments.rs` and
captures/receipts remain in `caj2pdf-c8-isolated-size-20261001`, outside Git.


### Horizontal decoration and complete diagnostic preview

The earlier diagonal `8010/1` control did not establish that this record was
ignorable. The actual page-6 record has equal endpoint y coordinates. Removing
only this record removes a visible repeated chevron separator; repeated source
and modified captures are individually identical. The changed page-relative
region is (19,600,641,606), with full frame (494,202,1155,1169).
The original `draw10horizontal` fixture reproduces visible decoration without
source-document content. Its geometry manifest explicitly sets `drawing_dy=0`.
Do not silently discard this required record or assume its diagonal behavior.

The PDF content writer now supplies a bounded black filled polygon primitive
(three to eight vertices), streaming coordinates through fixed scratch. Tests
cover concavity, closure, short writes, invalid coordinates, cancellation and
failed-output poisoning. This primitive does not establish C8 decoration shape
or spacing; those remain profile-specific work in #240.

Original geometric font controls now offer `extended-metrics` and
`shifted-outline` variants. Changing ascent/descent alone preserved the tested
CJK and Latin page crops. Moving original outlines up by 250/1000 em moved the
field-5 glyph up by 101 pixels for a 404-pixel em. This supports ordinary outline
placement relative to a baseline, but does not resolve the observed class-specific
Latin placement or the exact size table. No vendor outlines are copied.

An external six-page diagnostic now emits all 6638 mapped glyphs, three decoded
images and the required separator through shared PDF primitives. qpdf accepts
its 14,845,250-byte PDF, and MuPDF renders all six pages. Images were decoded by
the existing Rust codecs into temporary PDFs and extracted losslessly into
external row sidecars. This is not the production streaming image integration.
The diagnostic still uses hypothetical size/baseline rules, explicit substitute
fonts and an original approximate chevron shape. Visible text-spacing differences
remain. It therefore does not pass #233/#240 fidelity or public-adapter acceptance.

External diagnostics and outputs remain under
`caj2pdf-c8-render-preview-20261001`; viewer controls are under
`caj2pdf-c8-nativefont-page6-20261001`,
`caj2pdf-c8-ascent-control-20261001` and
`caj2pdf-c8-outline-shift-20261001`. No external fonts, captures, extracted text,
image rows or preview PDFs are repository fixtures.


### Direct codec and content-page integration control

The original `decoded_images_share_a_content_page_with_glyphs_and_vectors`
control decodes type-0, JPEG and type-3 descriptors directly into a document,
then interleaves their image handles with original Latin/CJK glyphs, a segment
and a filled polygon. It uses the same private checked-image emitter as the
image-only composer, including its scratch accounting and cleanup. No emitted
PDF is parsed to supply the mixed page's images.

The control checks original decoded bilevel rows, drawing order, cleared scratch
and bounded requests with short source/output calls. qpdf validation and a
72-DPI MuPDF raster independently check interior pixels: JPEG covers the earlier
segment, and the final polygon covers the white portion of a type-3 image.
The portable core assertions and external raster check are separate tests;
targets without local validators explicitly filter only the latter.
This verifies codec/content-writer integration, not the still-unverified source
C8 size, baseline or decoration rules. The external six-page diagnostic has
not yet been switched from sidecars to this internal path.


### Independently controlled native image coordinates

`tools/cajviewer/c8_image_fixture.py` generates seven original controls with
one asymmetric 32 × 24 JPEG. They share a 300 × 230 source page and vary only
one placement field or the declared origin. The generated files reproduce the
external viewer inputs byte-for-byte. No external document image is used.

At displayed 2896%, all seven complete page frames are (449,231,1574,1093).
Each capture repeats identically. Bounds below are half-open screen ink bounds,
not fractional mathematical edges:

| Control | Ink bounds |
| --- | --- |
| Baseline (relative x=30, y=40, width=80, height=50) | (562,380,863,569) |
| x + 20 | (637,380,938,569) |
| y + 20 | (562,455,863,644) |
| width + 20 | (562,380,938,569) |
| height + 20 | (562,380,863,644) |
| Both header origins + 20 | (486,305,788,494) |
| Both origins and image x/y + 20 | (562,380,863,569) |

The last page crop is byte-identical to the baseline. These controls establish
subtractive origins, independent axes/extents and absence of a text-specific
20-unit margin for this image profile. One-pixel edge rounding is visible in
the origin-only control; do not infer fractional edges from threshold bounds.
They independently support the earlier actual-source image-x observation.

`decode_native_image_coordinate` admits the observed `d300` prefix, removes
`c000` high bits from x/width, and preserves unsigned y/height. Unknown prefixes
or zero extents return `None`; a renderer must report that unsupported profile
explicitly. Remaining payload words are still opaque. The helper allocates
nothing and does not infer fonts, units or complete-page support.

The viewer vertically reverses the original JPEG relative to its encoded rows
(black source top border appears below). Row orientation therefore remains a
codec/emitter concern, as in the existing image-only composer; one universal
positive-height image transform is not justified. The six-page diagnostic uses
already decoded sidecars, whose representation must be distinguished from raw
source JPEG/type-0 storage. This observation alone does not identify a defect
in its existing diagrams.

External receipts: `caj2pdf-c8-image-controls-20261001/manifest.json`,
`measurements.json`, original inputs and repeated captures. This advances
image placement under #233/#240; font size/baseline and separator fidelity
remain unresolved.


## Independent size fields and original glyph anchors (#240)

The style fixture generator now reproduces eight original `axis-*` controls:
CJK `中` and Latin `A`, each with horizontal/vertical fields `(3,3)`, `(3,5)`,
`(5,3)` and `(5,5)`. They use a 150×100 page, raw position `(4672,4294)`
and the existing asymmetric origin. Generated inputs match the observed
controls byte for byte; all prior generated fixtures remain unchanged.

The pinned viewer restarted in its default smaller window. Its inspected
page interior is `(648,537,1023,787)` (375×250 pixels), not the earlier
maximized-window frame. With original geometric fonts, thresholded ink
left/top positions relative to that interior are:

| Horizontal, vertical | CJK | Latin |
| --- | --- | --- |
| 3, 3 | 99, 12 | 124, 34 |
| 3, 5 | 99, 12 | 124, 27 |
| 5, 3 | 99, 12 | 132, 34 |
| 5, 5 | 99, 12 | 132, 27 |

All eight repeat captures match. The high five-bit field changes width and
Latin horizontal placement; the low field changes height and Latin vertical
placement. The original CJK square keeps its upper-left anchor. Larger squares
clip at page edges, so these measurements do not establish their full extent.

A separate font control doubles only `hmtx` advance widths. Table comparison
finds changes only in `hmtx` and the expected `head` checksum. For equal fields
3, 5 and 8, both CJK and Latin page crops are identical to the normal-advance
controls; repeated captures also match. Thus advance width does not explain
the observed anchor difference for these controls. Vertical-metric and outline
controls described above remain separate evidence.

These observations establish independent field effects, not an exact PDF
font-size or Latin baseline formula. A proposed `ceil(em/12)` vertical offset
is inconsistent with the observed size-dependent deltas and is not admitted.
Production rendering acceptance remains open. External receipts are in
`caj2pdf-c8-advance-control-20261001/{comparison,axis-comparison,axis-inputs}.json`;
no captures or font binaries are committed.


### Additional raw drawing value `8006/a385`

An original one-row control changes only the existing `8006/a381` record's
value to `a385`, retaining two asymmetric endpoints, the trailing `ffff/0005`
pair and five following glyphs. Both controls render the line and all five
geometric glyphs in pinned CAJViewer; page crops and their repeated captures
are identical at displayed 57% zoom. The inspected page frame is
`(647,386,1024,937)`. This low-zoom equality establishes record framing and
preservation of following content, not identical stroke semantics at every
scale. The generator's `draw06a385` reproduces the observed input exactly.

The initial C8 visitor treated this as one 16-byte `NativeRecord::Drawing`.
The independent in-run boundary controls below supersede that assumption:
the drawing is 12 bytes and `ffff/0005` is a separate raw control. Short-read tests
use marker-like payload words followed by a glyph; the existing truncation
sweep now covers every shortened length of this form. Adjacent unverified
value `a384` remains unsupported. No allocation or new rendering rule is added.

HN-B inventories in #241 also encounter `a385`; that observation alone does
not enable the C8 visitor for HN-B. HN-B variant semantics and complete-document
conversion remain open. External receipts are the `segment-a381` and
`segment-a385` controls/captures and `segment-inputs.json` in
`caj2pdf-c8-advance-control-20261001`.

## Corrected in-run drawing boundaries

Eleven original C8 controls test `8006/a381`, `a385` and `a38b` separately:
12-byte drawing, drawing followed by `ffff/5`, and drawing followed by
`8001/5000`, plus a no-drawing baseline and standalone `ffff/5`. For each
style, bare and footer variants have identical first-page pixels. A following
y record moves the five glyphs while preserving the visible segment.
Standalone `ffff/5` matches the baseline. All repeat crops match at 57%,
page interior `(648,387,1023,936)`. This independently confirms the boundary
without assuming that HN-B semantics apply to C8.

The visitor now emits each `8006` drawing as a 12-byte record and preserves
`ffff/5` as its own `Control`. It does not discard the control or infer that
it is always a no-op. The already admitted `a383` remains 12 bytes. The
separate `8010/1` form was subsequently verified below with its own controls.
Existing sources containing `ffff/5` therefore yield one additional raw event
per occurrence: exhaustive event/count consumers must accommodate it. This is
a documented unstable v0.x parser behavior correction, not a conversion claim.

The generator reproduces all eleven inputs. Tests cover immediate y/end
records, marker-like coordinates, short reads and truncated drawings. Earlier
16-byte boundary descriptions in this investigation are superseded for these
`8006` forms. Font size, baseline and stroke fidelity remain unresolved under
#240. External receipts are `c8-drawing-boundary-{inputs,comparison}.json` in
`caj2pdf-c8-advance-control-20261001`; external captures remain outside Git.

### 8010 boundary and resource-controlled replay

The `8006` conclusion was not automatically applied to `8010/1`. Initial original
horizontal controls with substituted geometric fonts showed no decoration.
Replaying the known-positive `draw10horizontal` input and three new long
horizontal controls in the same pinned offline image with its shipped font
resources restores the visible repeated ornament. Those font files remain in
the external viewer image; no font outlines or implementation are copied.

Bare 12-byte `8010/1` and the variant followed by `ffff/5` have identical page
pixels. A following `8001/5000` moves only the glyph row; the ornament remains.
All four replayed inputs match their own repeats at 57%, page interior
`(648,387,1023,936)`. The generator reproduces the three new controls exactly.
This establishes the 12-byte boundary and independent following control, allowing
the reader to use one fixed drawing read without a guessed mandatory footer.
Tests cover following y/end records, marker-like points and truncation.

The resource comparison establishes sensitivity to font replacement for this
control; it does not identify a redistributable glyph or approve a substitute
pattern. `8010/1` must still not be silently dropped or rendered as a plain
segment. Its rendering semantics remain in #240. External receipts are in
`caj2pdf-c8-decoration-default-20261001`, including `comparison.json` and the
recorded launch arguments. Earlier mandatory-footer descriptions are superseded.

### Original glyph control for the 8010 resource dependency

Public FreeType call observations for the original horizontal `8010/1` control
show a character query of decimal 23812 in HGBZ_CNKI, returning glyph 1862 in
the viewer's bundled font. These are API metadata, not extracted outlines.
The numeric query is a resource alias; it does not establish the ornament's
Unicode text meaning or a stable glyph ID across fonts.

The geometric font generator now offers `--variant decoration-alias`. It adds
only that character-map entry, pointing at the existing original upper-half
rectangle. Against otherwise identical generated baseline fonts, the long
horizontal control gains a solid visible strip; the baseline has no strip.
Both captures repeat exactly at 57% with page interior `(648,387,1023,936)`.
The changed region is `(17,34,335,39)` relative to that interior. This isolates
the alias as necessary for visible decoration in this control and provides an
original positive resource fixture. It does not establish the complete pattern
placement, scaling or repetition rule and does not authorize rendering the
source ornament as a plain line. Those remaining rules stay under #240.

The default generator and previous metric/outline variants retain their output.
Receipts and captures remain external in
`caj2pdf-c8-decoration-{alias,no-alias}-20261001`. No vendor font data is added.

### Decoration axes and repetition control

Five original `decoration-geometry-*` inputs vary only the two `8010/1`
coordinate pairs. With the original alias font, shortening the horizontal
span changes the decoration width; translating both points by 300 source x
units moves its visible region 22 pixels, and 500 source y units moves it
37 pixels at the same 57% page frame. A vertical span is visible. The sloped
control matches the no-decoration baseline; this is an observation, not
permission to silently discard arbitrary diagonal records. All repeat captures
match. Raw coordinate endpoints remain preserved by the parser.

`--variant decoration-narrow` narrows the original upper-half rectangle to a
quarter em while retaining its one-em advance and the same alias. The long
and short horizontal controls now show separated repeated marks (53 and 27
connected runs at desktop row 423, threshold 200), rather than one stretched
rectangle. Vertical repetition is also visible; all three captures repeat.
The generator reproduces the tested glyph/cmap/metric tables and all five
source controls. This establishes axis sensitivity and glyph repetition, but
not an exact source-space step, endpoint clipping or font-size formula. Do not
promote pixel-run counts into a document-independent repetition rule.

External receipts are `geometry-{inputs,comparison}.json` in
`caj2pdf-c8-decoration-alias-20261001` and `comparison.json` in
`caj2pdf-c8-decoration-narrow-20261001`. The next discriminator is the source
step and end clipping under an independently changed zoom/advance; reuse these
controls rather than introducing another renderer or screenshot framework.

### Advance and zoom discriminate decoration spacing

Doubling the original narrow glyph's horizontal advance, leaving its
outline and mapping unchanged, produces an identical whole page at 57%.
The generated font also updates `hhea.advanceWidthMax` to remain consistent.
A fresh viewer replay with both metrics at 2000 confirms the same result.
Reproduce with `--variant decoration-narrow --decoration-advance-multiplier 2`.
Thus normal font advance is not the spacing rule for this observed decoration.

At independently selected and visually confirmed 100% zoom, all page edges
remain visible (interior `(858,179,1518,1145)`). The same long control has 51
separated runs on desktop row 245, with 11-pixel spacing in its unobstructed
tail, versus 53 runs and 6-pixel spacing at 57%. Repeated captures match.
This disproves a zoom-invariant glyph count derived from the 57% screenshot.
The viewer applies raster-dependent spacing/rounding; exact pixel equality at
one zoom cannot establish a source-space repetition count for PDF output.
Nominal symbol size and end clipping still need an independent rule before
production admission. Font outline substitution remains explicit. Receipts
are in `caj2pdf-c8-decoration-advance-20261001/comparison.json`; the
consistent-metrics replay is in
`caj2pdf-c8-decoration-advance-valid-20261001/comparison.json`.

## Print-path limit for physical-size evidence (#240)

An isolated CUPS-PDF destination was attached by Unix socket to the pinned,
network-disabled viewer. The original `c8-axis-reference-5` control printed
successfully. Two explicitly selected actual-size jobs and one automatic-fit
job produced byte-identical 160,762-byte PJL-wrapped PostScript spools (SHA256
`2db49b940f3ae532f3e3c4f2207ec3b307d0bcfac53d860c520c4d46ee94e6f4`).
Each spool contains one 2310×3059 RGB raster placed using `28 28 translate`
and `555 736 scale`; it contains no `/PageSize` request. The resulting PDF
passes qpdf, has no font objects, and uses an A4 MediaBox despite the Letter
label in the application's dialog.

These controlled print settings do not establish native vector geometry or
physical font units. Do not infer a point-size table from the dialog labels,
use the backend's paper size as the source page size, or promote this output
as an exact page-fidelity baseline. The print path is usable as a separately
identified raster appearance reference only. This bounded experiment is
complete; repeated screen/print ratio fitting does not resolve the missing
source-space style rule.

External receipts: `caj2pdf-c8-print-size-20261001` (build/server configuration)
and `caj2pdf-c8-print-output-20261001` (settings, raw spools, converted PDF and
`repeat-print-result.json`). The print service is experiment infrastructure,
not a runtime dependency. Original controls only; no captures or PDFs are
committed.
### HN-B native image framing (issue #250)

`tools/cajviewer/hnb_image_fixture.py` generates 13 original one-page controls
outside the repository. The pinned offline CAJViewer accepts the 20-byte index,
28-byte `800a/d300` image record and chained type-2 descriptors. Changing x, y,
width or header origins independently changes the displayed image as expected;
two images consume consecutive descriptors. Text following the 28-byte record
remains visible. The visitor preserves all 13 raw words, source order and exact
image counts; this is framing support, not a new public conversion profile.
The compact 12-byte index still rejects nonzero third words.

The generated inputs reproduce the independently captured controls byte-for-byte.
External receipts are in `caj2pdf-hnb-image-controls-20261001`; every selected page
crop repeats identically at 971% zoom. No source documents or captures are bundled.

Mixed-page rendering is unresolved: a glyph followed by a green JPEG changes
`(68,68,68)` to `(0,4,0)`, consistent with bitwise AND against decoded `(1,180,0)`
and inconsistent with Multiply. However image A → glyph → image B renders B
opaquely, while glyph → A → B retains cumulative AND in the overlap. These
controls rule out a universal image blend. They do not yet establish the state
that selects the operation; do not implement a guessed global blend or claim
complete HN-B conversion from raw record admission.

The following HN-B `8006/a383` drawing is independently verified as 12 bytes:
original bare and `ffff/5`-suffixed controls render identically, and a following
y control changes the glyph row independently. The visible segment occupies
only the added drawing region. `8072/cdc1` bare/next-y controls preserve their
following glyphs; unchanged pixels do not establish that this control is a no-op.
Both records reuse the existing raw events. Seven controls in
`hnb_index_fixture.py` reproduce the external inputs byte-for-byte, with repeated
identical crops (`remaining-record-comparison.json`).

After these admissions, the pinned issue-65 page 6 traverses all four raw records.
Page 1 traverses 283 records (235 raw glyphs) before another unsupported control
at offset 1500. Other pages retain explicit style/control failures. No complete
page rendering or document-conversion acceptance is claimed.

Four further original bare/next-y controls verify raw `801d/0003` and
`8070/001c` framing. All repeat identically and preserve the following row;
changes are confined to the affected first-row glyphs. The generator reproduces
captured input bytes exactly (`style-inputs.json`, `style-comparison.json`).
Only the observed raw values are admitted; this does not establish physical
font units or admit the corresponding untested `8071` value. The issue-65 probe
then reaches page-1 offset 1532 and page-3/page-4 offsets 53366/57046 before the
next unsupported controls; implicit-style failures on pages 2/5 remain.

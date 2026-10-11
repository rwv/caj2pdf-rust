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

## Fullwidth C8 queries

A [supplemental receipt](wine-cjk-fullwidth-20261011.json) records 20 further
sessions and 590 observed queries: all 52 fullwidth Latin letters plus space,
ampersand, two Han characters, the two low-byte letters and an ordinary Latin
sentinel, under five styles in forward and reverse order. Every repeated
resource/slot pair agrees. Styles `1084`, `0884`, `0c84`, `04c6` and style-zero
34-by-34 explicit axes select HGSS, HGHT, HGKT, SimSun and SimSun respectively
for the fullwidth/Han controls. Fullwidth letters query their ordinary Unicode
slots. The system controls replace only these authored glyphs in disposable
SimSun/NSimSun copies, preserving their cmap and surrounding UI glyphs.

The separate 72 mixed-code queries sample both C8 character modes. They
confirm that raw `006c` and `0070` always use HGSS at ordinary Latin slots,
independently of style. Ordinary `a0` Latin codes in character mode 1 retain
their Latin resource aliases. Character mode 0 queries distinct CJK aliases
for those `a0` codes. The [760-query follow-up](wine-cjk-aliases-20261011.md)
now implements all 62 alphabet/digit aliases, styled resources and independent
ActualText. The combination is absent from the 13-document inventory. Existing
placement gates remain unchanged, including CJK-mode space/ampersand refusal.

The optional CJK roles now also select C8 fullwidth letters, ideographic
space and fullwidth ampersand from the original style. The actual inventory
contains 1,805 fullwidth Latin glyphs under non-base styles in three documents.
The space/ampersand correction also affects other C8 documents. Ordinary
Latin and low-byte letters keep their previous selection rules. HN-B symbols
retain their separately measured resource rules.

## Explicit family state

The initial Han/fullwidth implementation ignored `801c`. Real-page review
exposed the error: `d6a23a9cfd0f…` retained Hei in paragraphs that Wine draws
with Song, and its fullwidth-only candidate increased five page pixel counts.
That candidate is retained as a failed intermediate result, not an improvement.

An initial hypothesis that explicit width/height reset the family is rejected:
12 sessions and 384 byte queries preserve the selected family across single
and paired axes in C8 and HN-B modes 0/2. Four further sessions (114 queries)
vary nineteen controls under Song, Hei and Kai styles. `801c/2`, `/3` and `/4`
select HGHT, HGKT and HGSS respectively; the other tested controls preserve
those families. Eight confirmation sessions (228 queries) establish the C8
Han/fullwidth behavior in both character modes, unchanged ordinary Latin and
low-byte resources, space/ampersand selection, and HN-B mode-2 `801c/4`.
Forward/reverse observations agree. A following `8002` restores its own
family; axis controls preserve the override. No new HN-B mode-0 control is
admitted. See the [state-query receipt](wine-cjk-state-20261011.json).

The converter now retains this family state separately from the geometric
style. Family 4 uses the caller's base `cjk` role, even after a system-font
style or explicit axes. Family 2/3 use the optional sans/Kai roles with their
existing fallback. Original native, Node and Chromium fixtures cover both
the override and its reset while preserving extracted text.

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

The final family-state candidate is independently converted and checked for
all 13 inputs again. The fullwidth update changes nine PDFs (51 pages), and
the subsequent `801c` fix changes three of them (21 pages); each changed page
is revisited in Wine. Other pages reuse observations only after PDF-byte
identity is confirmed. Compared with the initial Han-style candidate, strict
changed-pixel counts decrease on nine equal-extent pages from 756,145 to
594,185 in total, with no increases on the other equal-extent pages. Two
additional changed frames still have unequal source/output extents and are
not scored by resizing. These counts describe this pinned cohort, not a
general accuracy percentage.

A further six-page PDF (`90e7b47716c3…`) has unstable Wine startup: direct open
and two bootstrap attempts fail, including visible fatal errors. One bootstrap
visit confirms all six pages with stable repeated frames, identical to the
older candidate's six crops. The final PDF bytes are unchanged from that
fullwidth candidate. Its one admitted visit is **not** a repeatable cold-start
success; the [supplemental receipt](wine-cjk-state-20261011.json) retains all
four attempts. This stability issue remains open alongside pixel fidelity.

Across the 77 admitted source/output pairs, 35 have different page extents
and 42 have different pixels at equal extents. None is claimed pixel-exact.
No registration, resampling, column removal or fitted tolerance is used.
The new CJK roles correct visible font-family substitutions, but character
positions, punctuation, decorations and device rasterization remain separate
obligations. The ornament follow-up below now selects four distinct patterns.
#541 remains open for broader resource and geometry fidelity.

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

## Per-value ornament follow-up

[Four original query sessions](wine-ornaments-20261011.json) provide 160
observations of C8 `8010` values 1, 2, 46 and 117. The slots are respectively
HGBZ U+5D04, U+5D05, U+5D41 and U+5D95 (GIDs 1862, 1863, 1909 and 1981).
Five Latin states crossed with four CJK family states and reversed order all
agree. Low/high-byte geometric markers identify the complete cmap argument;
rectangles have distinct boundaries so repeated decoration tiles cannot merge
into a misleading marker. Both page frames match in every session, navigation
is confirmed, sources are unchanged and all containers were removed.

Explicit per-value maps now transport these aliases through Rust, CLI, raw
WASM, Node and browser. Maps are limited to four unique admitted values, may
be bound to the font identity, and reject missing glyphs without fallback.
Existing placement/state restrictions and nonsemantic Artifact/empty-ActualText
marking stay intact. Unlisted values retain the previous global alias.
Original geometric fixtures cover mixed/repeated values, unchanged ordinary
text and drawing geometry, resource sharing, missing glyphs and wrong identities.

The complete 13-document/77-page rerun passes qpdf and independent text-order
verification. Only the three PDFs containing values 1, 2 and 117 change;
the other ten PDFs are byte-identical to the family-control candidate. Node
also matches the native bytes for all three changed documents (15 pages).
The focused Node/real-Chromium suite passes all 44 tests without skips.

Wine revisits all 15 changed-document pages; only the three ornament-bearing
page frames change. Equal-extent differences fall from 35,437 to 35,201 pixels
on `90e7…` page 6, and from 50,736 to 50,305 on `d6a2…` page 5. The third
page (`b9a6…` page 1) retains a 505-versus-506-pixel width mismatch and is not
resampled or scored. Its pattern now uses the observed two-line ornament,
but visible tile seams differ from the source's continuous rule. All 77
source/output pairs still differ: 35 in extent and 42 in equal-extent pixels.
This corrects glyph identity without claiming complete ornament geometry.

Independent cold repeats match every page of the four-page and five-page
PDFs exactly. The six-page `90e7…` PDF succeeds in one direct cold visit but
fails during startup in the repeat, with a logged stack overflow and no
admitted page frames. Its earlier startup failures are retained; the mapping
change does not establish a startup fix. All six sessions preserve source
hashes and clean up their containers without OOM. Twelve mode-2 documents
still match frozen-main PDF bytes with the new roles/maps absent.

The [ornament-spacing controls](wine-ornament-spacing-20261011.md) isolate the
remaining repetition difference with original rectangle glyphs. Forty-three
Wine sessions include four zooms, exact cold source repeats and doubled-advance
controls. All 36 source-row measurements match integer-floor device-pixel
spacing, while the PDF retains fractional origins. Coalesced text, equivalent
rectangle paths and 23 pattern sessions do not establish an improvement;
blank pattern results are explicitly disqualified. This study changes no
production geometry and does not close the actual-input fidelity obligations.


## Page-boundary follow-up

The [67-session coordinate study](wine-page-extents-20261011.md) supersedes
the earlier 35-extent/42-pixel inventory classification for its new candidate.
A single empirical scale, selected with independent synthetic boundaries and
validated at held-out dimensions/zooms, gives matching extents on all 77
actual pages. All pages still differ in pixels. On the 42 previously
comparable pages, 30 improve, ten are unchanged and two worsen slightly.
The historical measurements above remain records of their own candidates.

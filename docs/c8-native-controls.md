# Additional native C8 control framing

This parser increment belongs to #242 and follows the encoded-string prefix
work in #243. Rendering remains unsupported until required semantics are
implemented. Some of these tags are also observed in HN-B (#241); the native
visitor still accepts only C8, and this is not an HN-B support claim.

## Admitted records

| Tag | Admitted values | Total bytes | Event |
| --- | --- | --- | --- |
| `801c`, `8070`, `8071` | `0004` | 4 | `Control` |
| `801d` | adds `0003` to the existing `0000`, `0004` | 4 | `Control` |
| `80ce` | `0000`, `0001` | 4 | `Control` |
| `8024` | `2800`, `281d` | 4 | `Control` |
| `8021` | `2000` | 4 | `Control` |
| `80d0`, `80d2` | `0000` | 4 | `Control` |
| `80d1` | `0001` | 4 | `Control` |
| `81ff` | `0001`, `0002`, `0003` | 8 | `ExtendedControl` |
| `80cc` | `0204` | 8 | `ExtendedControl` |

The eight-byte forms preserve two uninterpreted payload words. Their payload
is atomic, even if a word resembles a record tag. In particular `80cc/0204`
is not an ASCII string and must not be processed as the `80cc/01xx` profile.
Other tag/value combinations continue to fail explicitly.

No rendering meaning is assigned by the parser. The `Glyph` event's `style`
is the last `8002` value; renderers must also process preceding controls and
reject unimplemented required state. Neither existing style fields nor
successful parsing authorize silently ignoring transformations.

## Original controls and observations

`tools/cajviewer/c8_native_control_fixture.py` generates a baseline and
16 single-control variants. Each has two fixed, asymmetric glyphs separated
by the control under test. The same original geometric font as #240 gives
a full square followed by an upper half-square. The generator reads no
external document or font and reproduces all 17 observed inputs byte for byte.

Pinned offline CAJViewer 9.0.0 rendered the second glyph after every control.
At displayed fit-width 2896%, the outer page frame was
`(449,231,1574,1093)`. All repeated page captures match. Thirteen variants
match the baseline page pixels; three have visible changes:

- `8024/281d` skews the following glyph; the right side is clipped by the page.
- `8070/0004` changes the following glyph's horizontal extent.
- `8071/0004` changes the following glyph's vertical extent.

These observations distinguish valid record boundaries from rendering
semantics. They do not establish the exact transform units, authorize
ignoring the other controls, or validate entire source documents. No screenshot
resizing, content registration or blanket pixel tolerance was applied.
External captures and manifests remain in `caj2pdf-c8-size-scale-20261001`
(`short-controls.json`, `short-control-comparison.json`).

The original Rust controls exercise short reads, raw values, atomic marker-like
payloads, all truncated eight-byte lengths and unsupported values. The existing
fixed buffer, indexed-span limits, record budgets, cancellation and poisoned
cursor behavior are reused. No allocation or separate parser framework is added.

## Remaining work

The two SHA-pinned additional C8 files also contain `810a/d300` records with
apparent image coordinates and a variable byte-string payload, plus further
controls. Their boundaries and image semantics require separate original
controls; do not coerce them into the existing 28-byte `800a` image form.
Remaining record/glyph/font interpretation, complete 4-/5-page conversion,
geometry and the public CLI/Node/browser path stay open under #242/#233.

Direct probes of the unchanged, hash-pinned additional C8 inputs after this
increment still stop on specific unadmitted data: `801d/001c` or `001f`,
`8024/281c`, `810a/d300`, and a drawing form that does not match the currently
required terminator. These are current-profile limitations, not proof that
the source is corrupt. All nine pages remain incomplete conversion cases.
The direct probe receipts are `4-[21]-direct-controls.txt` and
`4-[24]-direct-controls.txt` in the external native-profile inventory.

## Mixed-context resource and painting checks (#242)

The original generator now also reuses the existing mixed image/glyph/segment/
decoration fixture under ordinary and alternate Latin resources. Distinct
original marker fonts make resource changes visible. For `80ce/1`,
`8021/2000`, `80d0/0`, `80d1/1` and `80d2/0`, accepted page crops and their
repeats equal each same-state baseline exactly. The renderer admits these
specific values without changing state. `80ce/1` also retains its earlier
independently verified HN-B behavior; the other additions are C8-only.

The controls also disprove a blanket no-op interpretation: `80ce/0` changes
the Latin marker resource and position, and `81ff/1` and `81ff/2` change glyph
gray to black in these contexts. At that checkpoint these remained explicit errors pending their
state semantics; the exact black payload is now established below. The initial `81ff/3` mixed probe was followed by
viewer process exit; its black capture and all subsequent black frames are
excluded. The remaining controls were repeated in a verified fresh process.
`80cc/0204` with the original `(33,5)` payload preserves this baseline, but
that single payload does not establish the real documents' varying values;
its rendering remains unsupported here.

All 22 generated mixed inputs reproduce the captured originals. Twelve
admitted controls convert, pass qpdf and produce PDFs identical to their
same-state baseline; ten unadmitted controls fail without final output.
The existing mixed-page Rust test covers the added no-paint values, and
`80ce/0` remains an explicit negative regression. Fixed parser bounds and
raw visitor values are unchanged.

External receipts are `c8-state-comparison.json`,
`c8-extra-control-accepted.json` and `c8-extra-output/checks.json` under
`caj2pdf-hnb-rendering-20261003`. Accepted screenshots use the original page
crop `(648,380,1024,944)` at 486%; no registration/scaling or pixel tolerance
is used. The earlier raw comparison ledger includes excluded black frames;
only the accepted ledger supplies comparison evidence.

Both complete-document retries now reach `81ff/1` with payload `(0,200)`:
page-1 byte 236 in `4-[21].caj`, byte 248 in `4-[24].caj`. They still fail
explicitly without final output. The following source prologue includes
`81ff/2` and `81ff/3`; investigate their combined state rather than treating
the single-control probe's viewer exit as evidence that the source is corrupt.

## Verified black glyph state (#242)

Nine additional original controls distinguish the required `81ff/1..3`
payload `(0,200)` from other values. Combined and individual controls paint
subsequent glyphs black. Inserting each control once before style/resource
selection gives the same nonblank source crop as repeating the combined
controls. Style and resource changes therefore preserve this state within
the tested page. New pages initialize their own ordinary gray state.

The renderer stores one gray byte and admits only this exact C8 payload.
It does not infer general RGB encoding: the historical `first-red` and
`second-red` probes actually paint black, and `all-gray` paints a different
color. Those other payloads remain unsupported. A valid repeated isolated
`81ff/3` capture supersedes the earlier process-exit observation; excluded
black frames remain excluded.

All nine source crops are nonblank and repeat exactly. The six admitted
controls convert and pass qpdf. Independent MuPDF inspection reports glyph
color `0x444444` for the baseline and `0x000000` for all five black controls;
the three unadmitted payloads fail without a final PDF. Original Rust tests
cover all three selectors, persistence across style/resource changes,
variant isolation and nearby invalid payloads. No parser allocation or
whole-page buffering was added.

External evidence remains under `caj2pdf-hnb-rendering-20261003`:
`c8-color-comparison.json`, `c8-color-output/checks.json`, and
`c8-color-output/render-checks.json`. The fixture generator reproduces the
original inputs; probe names do not assert color semantics.

Full-document retries now stop on `80ce/0` at page-1 byte 284 in `4-[21].caj`
and byte 296 in `4-[24].caj`. Earlier mixed controls show a resource/placement
change, so it must not be ignored. Both documents still fail explicitly
without publishing output. Complete conversion and public runtime acceptance
remain open in #242.

## Persistent CJK resource/placement mode (#242)

Eight original `mode_documents()` controls establish `80ce/0` as a persistent
CJK resource/placement selection in the tested C8 mixed context. An initial
single zero survives subsequent style and ordinary/alternate Latin resource
controls. Repeating `1,0` produces the same source crop. Repeating `0,1`
restores the corresponding baseline exactly, including its Latin resource.
All crops are nonblank and their repeats agree; the earlier zero-mode mixed
captures agree with the new zero-mode controls.

The existing renderer now retains one boolean per page. Zero selects CJK
font/geometry for admitted CJK characters and ASCII alphanumerics; one
restores ordinary raw-code resource selection. Other zero-mode characters
remain explicit errors until independently established. This is C8-only;
HN-B zero mode is not inferred. Existing style checks and bounded I/O remain.

All eight original inputs reproduce their captured bytes and convert through
the CLI with qpdf validation. For each resource state, restored output equals
baseline PDF bytes, and initial-zero output equals repeated-zero output.
Independent MuPDF rendering preserves the observed marker-resource change.
At the viewer's 486% scale, measured filled marker edges differ by 0–3 pixels
from source; these residuals are reported, not hidden with image registration
or blanket pixel acceptance. Source interior RGB is `(68,68,68)` and this
MuPDF RGB rendering produces `(67,68,67)`. The measurement excludes thin
segments by requiring 15 interior pixels per row/column; it does not establish
full-page pixel equality or improve the existing segment rasterization.

External receipts: `c8-mode-comparison.json`, `c8-mode-output/checks.json`
and `c8-mode-output/marker-bounds.json` under the existing
`caj2pdf-hnb-rendering-20261003` evidence root. Original Rust regressions cover
persistence, restoration to both Latin resources, variant isolation and
unknown zero-mode glyphs.

Full four-/five-page conversion now stops at `80cc/0204`, page-1 byte 292/304,
with first payload `(420,5)` / `(342,5)` respectively. No final PDF is
published. Resolve that required behavior next; the earlier `(33,5)` probe
alone does not establish arbitrary values. Complete #242 acceptance remains
open.

## Extended metadata painting checks (#242)

Twelve original `extended_string_documents()` controls exercise `80cc/0204`
in both ordinary and CJK resource modes. Payloads include the observed
`(342,5)` and `(420,7)`, zero, both maximum words, and marker-like `(8004,1)`.
Each record is inserted before glyphs, all three admitted segment forms,
decoration and an image. Every nonblank source crop repeats exactly and equals
its same-mode baseline at the existing 486% crop, without registration or
pixel tolerances. This establishes the scoped painting behavior; it does not
assign meaning to the raw metadata or establish text-selection semantics.

The composer consumes the existing bounded `ExtendedControl` event without
changing painting state. The parser still exposes both raw words and treats
the eight-byte record atomically. Existing short-read, truncation and marker
payload tests are reused. The original mixed-page rendering test now covers
these payloads in both modes and rejects inferred HN-B behavior.

All twelve generated inputs reproduce the captured bytes. CLI outputs pass
qpdf and equal their respective baseline PDF bytes. Receipts remain outside
Git under `caj2pdf-hnb-rendering-20261003`:
`c8-extended-string-comparison.json` and `c8-extended-string-output/checks.json`.
The complete four-/five-page inputs now stop on `801d/28` at page-1 byte
644/656, followed by a required `a3ca` glyph. Neither publishes final output.
Investigate that resource state and its required mapping next; full #242
acceptance remains open.

## Extended font-state framing and resource distinction (#242)

Eight original `font_state_documents()` controls vary only `801d` among
0, 4, 28 and 31 and the Latin-slot raw code between `a0c1` and required
`a3ca`. CJK glyphs, three segment forms, decoration and the asymmetric image
retain their framing and placement. All nonblank page crops repeat exactly.
The generator reproduces the captured original bytes.

For `a0c1`, states 28/31 show the fourth marker resource instead of the
ordinary or alternate Latin marker. For `a3ca`, all four states show the
same CJK marker and CJK placement. These observations disprove treating the
new font states as no-ops and distinguish raw-code resource choice from
active Latin state. Because multiple external font filenames intentionally
share the fourth original marker, equality between 28 and 31 does NOT prove
that they select the same original font. Marker shape alone does not prove
Unicode mapping either.

The bounded C8 visitor now admits the four-byte `801d/28` and `/31` controls
and preserves their raw values. Original short-read tests verify following
glyph alignment; HN-B rejects the new C8 values. The renderer still rejects
both states explicitly rather than silently reusing a wrong font. No new
public font role or memory allocation is added in this framing increment.

External evidence is `c8-font-state-comparison.json` and
`c8-font-framing-output/checks.json` under
`caj2pdf-hnb-rendering-20261003`. Complete four-/five-page retries still stop
at page-1 byte 644/656, now with a rendering-state error instead of an unknown
record boundary. No final PDF is published. Next distinguish the required
font resources using independently identifiable substitutions, verify the
required character mapping, and use the existing bounded font transport.
Full #242 acceptance remains open; this is not conversion support.

## Independently identified resources and fullwidth J (#242)

The original geometric-font helper now provides seven-bit interior markers
while retaining each generated font's lookup names, metrics and character
aliases. A separate pinned offline viewer received 84 independently marked
original fonts. All 84 files reproduce byte-for-byte with
`identified_resource_font`; no vendor outlines are inputs.

Eight repeated ready-state controls identify the following resources on all
three glyph rows (the CJK reference glyph always selects `HGHT_CNKI`):

| `801d` state | `a0c1` resource | `a3ca` resource |
| --- | --- | --- |
| 0 | `HGBZ_CNKI` | `HGHT_CNKI` |
| 4 | `HGHZ_CNKI` | `HGHT_CNKI` |
| 28 | `HGB1_CNKI` | `HGHT_CNKI` |
| 31 | `HGB1X_CNKI` | `HGHT_CNKI` |

The resource names describe this pinned viewer, not fonts bundled by this
project. The identified states are distinct; do not alias 28/31 merely because
the older four-group marker test looked identical. Their existing glyph
positions remain ordinary Latin versus CJK as previously observed.

Ordinary Copy of the original required first row yields `中Ｊ\r\n`, with a
fresh sentinel-to-viewer clipboard transition and complete UTF-8 transfer.
The raw `a3ca` is therefore fullwidth `Ｊ` (U+FF2A), agreeing with the existing
GB18030 decoder. It uses CJK font/geometry independently of Latin selection.
The renderer now admits this verified C8 glyph; states 28/31 themselves still
require explicit font-role transport and remain renderer errors. Tests compare
the glyph matrix to a CJK reference and verify states 0/4 do not change its
resource. Both corresponding original mixed CLI controls pass qpdf; independent
MuPDF extraction finds three U+FF2A characters in each output.

External evidence under `caj2pdf-hnb-rendering-20261003`:
`identified-family-viewer/manifest.json`, `resource-identities.json`, the
`*-ready-{a,b}.png` captures, and `required-copy-result.json`; converted controls
are in `c8-required-j-output/checks.json`. Marker decoding uses the centers of
seven authored holes; thin vector pixels are excluded from glyph bounds. This
is resource identification, not pixel-perfect font fidelity. Ordinary Copy is
selected from its visible menu, not the enhanced-copy shortcut. The Qt
selection-owner name is checked in the isolated viewer, and freshness/complete
transfer are verified separately.

Excluded trials: the earlier `identified-font-viewer` renamed lookup families
and consequently omitted glyphs; these are not valid fidelity evidence. The
first `identified-family-viewer` ordinary-state-0 capture preceded document
opening and is superseded by its `ready` capture. Neither trial supports a
format or Unicode claim. Full-document acceptance remains pending the two
font roles and subsequent required content in #242.

## Extended Latin resources through public runtimes (#242)

The verified state-28 and state-31 resources now use distinct optional roles:
Rust `latin_state28` / `latin_state31`, JS `latinState28` / `latinState31`, and
CLI `--font-latin-state28` / `--font-latin-state31`. They reuse the existing
bounded source, shared-source embedding and temporary-spooling mechanisms.
The fixed resource capacity increases from six to eight. No discovery,
embedded-path opening, bundled vendor font or generic font framework is added.

The new WASM `caj2pdf_c8_set_latin_state(state,index)` accepts only verified
states 3, 28 and 31 after base registration and before polling. Existing base
and state-3 exports retain their signatures. Missing roles and out-of-range,
duplicate or late registration fail explicitly. State selection/restoration
and optional source aliasing have original core tests; Node and real Worker
controls exercise both distinct resources with short reads and scratch cleanup.
Rust literal initializers need the two documented optional fields.

All eight original font-state CLI controls pass qpdf using distinct identified
state-28 and state-31 marker inputs. The complete four-/five-page sources now
reach raw glyph `a3ef` at page-1 byte 676/688 respectively. Both fail explicitly
without publishing a PDF. The following run contains additional fullwidth
letters; verify the alphabet's mapping/resource/geometry as one original
control group rather than opening a ticket per letter. External CLI receipts
are `c8-font-roles-output/checks.json` under the existing evidence root. Full
conversion, independent fidelity and final public-runtime acceptance remain
open in #242.

Two additional original transition controls (`28→31→0` and `31→28→4`)
verify restoration in a single mixed page. Each repeated source glyph-row crop
exactly equals its independently captured state baseline. Both PDFs pass qpdf;
MuPDF identifies `HGB1_CNKI`, `HGB1X_CNKI`, then `HGBZ_CNKI` for the first
sequence and the reversed extended pair followed by `HGHZ_CNKI` for the second.
All ten font-state inputs reproduce their original bytes. Receipts:
`c8-font-roles-output/transition-checks.json` and `render-checks.json`.

## Fullwidth alphabet in C8 (#242)

Original `alphabet_documents()` controls cover all 26 uppercase and lowercase
letters plus two CJK anchors in a seven-row grid. States 0/4/28/31 in ordinary
mode and state 31 in CJK mode each have uppercase, lowercase and all-CJK
versions (15 files). All inputs reproduce the captured bytes. Repeated viewer
page crops are nonblank and equal the corresponding CJK baseline. The identified
marker resource is HGHT_CNKI, independently of Latin selection.

Explicit drag selection followed by the visible ordinary Copy menu returns
U+FF21..FF3A and U+FF41..FF5A, respectively. Both clipboard transfers pass the
independent freshness/complete-transfer validator. An earlier uppercase attempt
used whole-page selection without a confirmed menu-ready observation and
returned normalized ASCII; it is excluded from character-mapping evidence.
The successful drag/menu-ready transaction resolves that ambiguity.

The renderer admits C8 raw `a3c1..a3da` and `a3e1..a3fa` as CJK-resource glyphs,
including in CJK mode. Existing GB18030 decoding already supplies their Unicode;
HN-B's separate mode-dependent mapping is unchanged. Original core regressions
cover all 52 letters across the observed states/modes and compare placement to
a CJK reference. All 15 CLI controls pass qpdf and independent Unicode/resource
checks. All ten alphabet PDF renders equal their same-state CJK baseline with
the original geometric font; this is not original-font pixel fidelity.

External receipts under `caj2pdf-hnb-rendering-20261003`:
`c8-fullwidth-alphabet-comparison.json`,
`identified-family-viewer/alphabet-{upper-drag,lower}-copy{,-validation}.json`,
the corresponding menu-ready captures, and
`c8-fullwidth-alphabet-output/{checks,render-checks}.json`.

Complete-document retries now reach style `1484`, CJK mode 0, at page-1 byte
2024 in the four-page input and 2072 in the five-page input. Both still fail
explicitly with unverified style flags and publish no final PDF. Next verify
that style's geometry/painting against the existing original style controls;
do not infer it from its low bits alone. Full-document acceptance remains open.

## Additional field-4 style `1484` (#242)

Six original `field4_style_documents()` controls hold glyph positions constant
and compare `1084`, `1484` and the different vertical size `1085`. Each contains
CJK and ordinary Latin glyphs under resource states 0, 4 and 28; mode 0 and mode
1 are separate files. All six reproduce the captured input bytes. At 364%,
repeated nonblank page interiors `(648,521,1023,802)` match exactly for `1484`
and `1084` in both modes, while `1085` differs. This distinguishes measured
field-4 equivalence from an indiscriminate style-bit mask.

The existing glyph transform admits exactly `1484` with the established field-4
metrics. Unknown adjacent `1485` and decoration states remain errors. Both CJK
and Latin transforms have regression tests. All six CLI PDFs pass qpdf; the two
`1484` outputs are byte-identical to their respective `1084` outputs, and the
`1085` outputs differ. No new rendering state or allocation is introduced.

Receipts under `caj2pdf-hnb-rendering-20261003`:
`input/c8-style1484/manifest.json`, `c8-style1484-comparison.json`, repeated
`identified-family-viewer/style-*-mode-*-{a,b}.png` captures, and
`c8-style1484-output/checks.json`. The source resources are original marker
fonts, so equality does not establish original-font raster fidelity.

Complete four-/five-page retries now stop at `801c/4`, page-1 byte 2512/2560,
followed by `8070/4` and `8071/4`. Both publish no final PDF. Determine that
control's interaction with explicit axes using the existing original controls;
do not assume the payload is a literal four-unit size. Full-document acceptance
remains open in #242.

## C8 state control and four-unit explicit axes (#242)

`state_axis_documents()` generates 20 original state/axis/reset controls and
four enlarged size discriminators. All 24 reproduce the observed input bytes;
all repeated page interiors are nonblank and identical. Ordinary/alternate
Latin resources and styles `1084`/`10a5` distinguish resource and style state.
Eight same-context comparisons establish that `801c/4` preserves baseline,
explicit 4/36 axes, and later style-reset behavior. Its other semantics are not
inferred. `8002` continues to clear the existing two axis fields.

At displayed 1457%, four-unit CJK and Latin marker glyphs have 19x19-pixel ink
bounds; the size-3 and size-5 controls give 14x14 and 24x24. The CJK and Latin
source bounds relative to the independently located 375x375 page interior are
`(74,84,93,103)` and `(227,112,246,131)`. The existing em model with axes `(4,4)`
and a 15-coordinate-unit Latin baseline yields PDF bounds `(75,85,94,104)` and
`(228,113,247,132)`. Each edge is one pixel farther right/down. This recorded
raster/page-frame residual is not corrected by a source-specific offset and
is not a claim of pixel-perfect original-font fidelity. The detail frame is
`(648,474,1023,849)`; ordinary controls use `(648,521,1023,802)`.

The C8 visitor now accepts the observed state control and registers axis value
4 in its existing fields. The shared transform handles only the verified paired
`(4,4)` case with the measured baseline. Single-axis 4, mixed `(4,36)`, adjacent
sizes 3/5 and other state-control values remain explicit errors. HN-B admission
of explicit axis values is unchanged. No buffer or state structure is added.
Original core tests cover reset, state persistence, placement and failures.

Eighteen admitted CLI controls pass qpdf; all eight unchanged-state PDF pairs
are byte-identical. Six unadmitted controls fail without a final PDF. External
receipts under `caj2pdf-hnb-rendering-20261003` are
`input/c8-state-axis/{manifest,detail-manifest}.json`,
`c8-state-axis-{repeats,comparison}.json`, the repeated identified-family-viewer
captures and `c8-state-axis-output/{checks,detail-comparison}.json`.

Complete-document retries now stop at raw glyph `a3c0`: page-1 byte 4412 in the
four-page input and 4036 in the five-page input. Both fail with unverified glyph
resource/placement and publish no final PDF. Reuse HN-B's existing evidence as a
hypothesis, but verify the C8 resource/mapping before extending its admission.
Full-document acceptance remains open in #242.

## Fullwidth at sign in C8 (#242)

Eight original `at_sign_documents()` controls compare raw `a3c0` against the
already admitted `a3ac` comma at 26 fixed positions and two CJK anchors, under
Latin resource states 0/4/28/31. All reproduce their input bytes. Repeated
nonblank viewer page crops equal their same-state comma baseline. The existing
identified marker resources distinguish ordinary, alternate and both extended
Latin fonts; the at sign retains the current Latin resource and the comma's
CJK-class matrix with zero baseline fraction.

Explicit drag selection and the visible ordinary Copy menu return 26 U+FF20
characters plus the two CJK anchors (with viewer-inserted whitespace). The
independent clipboard validator confirms fresh ownership and complete transfer.
The existing GB18030 decoding already preserves U+FF20. The implementation
extends the existing contiguous symbol range to `a3c0`, removing its former
HN-B-only alternative without adding state or allocation. Core tests retain
HN-B coverage and add C8 ordinary/alternate/extended resource cases.

All eight CLI PDFs pass qpdf and independent Unicode checks; each same-state
at-sign/comma pair renders identically with the original marker fonts. This
establishes the tested resource/geometry behavior, not original-font pixel
parity. External receipts under `caj2pdf-hnb-rendering-20261003`:
`input/c8-at-sign/manifest.json`, `c8-at-sign-comparison.json`, repeated
identified-family-viewer `at-*-{a,b}.png` captures, the `at-copy-menu-ready.png`,
`at-copy{,-validation}.json` receipts, and `c8-at-sign-output/checks.json`.

Complete-document retries now stop at unsupported native tag/value `9002/0`,
page-1 byte 4564 in the four-page input and 4248 in the five-page input. Both
publish no final PDF. Verify this record's framing and painting-state effects
with original controls before admitting it; complete-document acceptance stays
open in #242.

## Four-byte `9002/0` record (#242)

Eight original `record_9002_documents()` controls insert the exact record before
CJK/Latin glyphs, segments, decoration and a JPEG image. Ordinary/alternate
resources and ordinary/CJK glyph-selection modes are independently paired;
black color is set before each insertion. All inputs reproduce their bytes.
Repeated nonblank viewer page interiors `(648,380,1024,944)` at 486% are equal
to their respective baselines, including the content following the record.
This establishes four-byte framing and preservation of the tested painting
state, without assigning speculative non-painting semantics.

The parser exposes only `9002/0` as an existing Control event; the writer
consumes it without changing state. HN-B's profile guard continues to reject
it, and other values remain unsupported. Original tests cover one-byte reads,
exact subsequent-glyph offsets, invalid values and all incomplete value-word
lengths. Existing mixed-page tests verify unchanged output in both modes and
HN-B rejection. No scanning, new event type, state or allocation is introduced.

All eight CLI PDFs pass qpdf, and four same-context PDF pairs are byte-identical.
External evidence under `caj2pdf-hnb-rendering-20261003`:
`input/c8-9002/manifest.json`, `c8-9002-comparison.json`, repeated
identified-family-viewer `record-*-{a,b}.png` captures and
`c8-9002-output/checks.json`. Original marker-font comparisons do not establish
original-font raster fidelity.

Complete-document retries now stop on style `14c6` at page-1 byte 4772 in the
four-page input and 4528 in the five-page input. The following glyph is raw
`aab3` under ordinary Latin resource and ordinary glyph-selection mode. Both
publish no final PDF. Verify the style against existing field-size controls
before admitting it; whole-document acceptance remains open in #242.

## Additional field-5/6 style flags (#242)

Thirteen original `additional_style_documents()` controls compare required
styles `14c6`, `04c6` and `14a5` with supported `10c6`/`10a5` and unequal-axis
`10c5`/`10a4` discriminators. Three resource rows (0/4/28) contain CJK, Latin
and the required `aab3` symbol; CJK-mode controls use a CJK anchor in the last
column instead of admitting an unverified symbol/mode combination.

All inputs reproduce their bytes. Repeated nonblank page interiors
`(648,531,1023,793)` at 291% agree for each required style and its same-context
baseline, while the unequal-axis controls differ. `14c6` is observed in ordinary
mode; `04c6` and `14a5` are additionally controlled in CJK mode. This justifies
specific field-size aliases, not every possible high-bit combination. The
shared transform reuses existing metrics; adjacent unverified flags and
unverified decoration styles remain errors. Both glyph classes retain core
placement regressions. No new state, allocation or renderer is added.

External receipts under `caj2pdf-hnb-rendering-20261003`:
`input/c8-style14c6/{manifest,additional-manifest,field5-manifest}.json`,
`c8-style{14c6,04c6,14a5}-comparison.json`, and the repeated
identified-family-viewer `field{5,6}-*-{a,b}.png` captures. The initial
`style-10c6-ready.png` still displayed the viewer's startup document and is
excluded; `field6-confirmed.png` confirms the correct input before accepted
captures. These original-marker comparisons do not establish original-font
raster fidelity.

All thirteen converted controls pass qpdf; five required-style/baseline PDF
pairs are byte-identical. Receipts: `c8-field56-output/checks.json`. Full-document
retries now diverge: the four-page source stops at `8024/281c`, page-1 byte
6272; the five-page source reaches `801d/3`, page-1 byte 19552. Neither publishes
a final PDF. Verify the required skew state in the four-page source first,
then the five-page font resource; existing HN-B/other-value observations are
hypotheses to test, not automatic admission. Full acceptance remains open.

## C8 `8024/281c` skew state (#242)

Seventeen original `skew_281c_documents()` controls reuse the established
400-by-400 canvas. Three unequal/large glyph sizes compare `2800`, `281c` and
`281d`; separate CJK and CJK/Latin pairs check activation, explicit reset and
style persistence. All inputs reproduce, and repeated nonblank page interiors
`(648,474,1024,850)` at 729% are identical. Reset matches the baseline; a style
change retains the active tilt. Latin resource identity and baseline remain
visible through the original resource markers.

The large marker's observed left-edge slope is approximately -0.22503 screen
pixels per row, versus -0.24196 for `281d`. Unequal axes show that displacement
scales with width, as in the existing model. The specific C8 control therefore
sets the existing shear field to 0.225; the glyph matrix uses
`[width, 0, width * 0.225, height, x, y]`. No inferred physical angle, extra
translation, horizontal scale correction or new state field is introduced.
`2800` clears the shear and `8002` retains it. HN-B still rejects `281c`.
Unverified values and drawing/image events in active skew remain errors.

Independent MuPDF renders retain measured edge residuals. For `1067`, the
PDF-minus-source top/bottom/left/right deltas are `(0,1,1,1)` pixels; for `10e3`,
`(0,1,1,4)`; for `e58c`, `(-2,1,1,2)`. The large-glyph PDF slope is -0.22501.
The ordinary CJK/Latin pair has deltas `(0,1,1,3)` / `(0,1,0,2)` respectively.
These are scoped original-marker comparisons, not original-font pixel parity;
no blanket tolerance or compensating offset hides the residuals.

All 17 CLI PDFs pass qpdf. Both CJK and Latin reset/style-preservation pairs
are byte-identical to their respective baselines. Existing matrix regressions
now cover this value, unequal axes, persistence and reset; bounded parser tests
verify following-record offsets and reject other values. External receipts in
`caj2pdf-hnb-rendering-20261003`:
`input/c8-skew281c/{manifest,latin-manifest}.json`,
`c8-skew281c-{repeats,state,latin-repeats}.json`, repeated `s28-*` captures, and
`c8-skew281c-output/{checks,render-comparison,latin-checks}.json`.

The full four-page input now stops at raw character `006c`, page-1 byte 6280,
immediately after the admitted skew control. The five-page input still stops
at C8 `801d/3`, page-1 byte 19552. Neither publishes a final PDF. Verify the
character's Unicode/resource/geometry independently before assuming it is an
ASCII scalar or admitting an entire low-byte range. Whole-document acceptance
remains open in #242.

## Observed C8 low-byte letter `006c` (#242)

Fresh ordinary Copy of the original grid returns 26 U+006C letters plus two CJK
anchors; the independent validator confirms ownership change and complete UTF-8
transfer. Resource markers show HGHT_CNKI, not the current Latin font. A CJK
reference moved down 15 source-coordinate units matches the observed geometry.
The behavior persists in ordinary/CJK selection modes and resource states 0/4.
The `a0ec` ordinary-Latin control differs, so equal Unicode does not imply equal
resource or placement.

Ten original `low_letter_documents()` inputs reproduce their captured bytes.
The explicit `281c` skew pair also matches its shifted-CJK reference exactly.
State-4 mode/reference comparisons are pixel-identical. State-0 comparisons
after reopening have identical glyph bounds but differ by at most one grayscale
level at 4,196 pixels; this residual is retained. The first state-0 repeat pair
still had the previous copy selection highlighted and is excluded; the clean
`low-0-low-clear-{a,b}` pair replaces it. No selected-page image is used as a
rendering baseline.

Only the C8 page writer maps this exact raw code to U+006C and selects the CJK
resource with the existing symbol baseline, before mode-dependent ordinary
classification. The public generic decoder and HN-B admission are unchanged;
adjacent unverified low-byte codes remain errors. No new font role or buffer is
needed. Core tests check Unicode, resource, both selection modes, explicit skew,
15-unit baseline equivalence and HN-B/adjacent-code rejection. Algebraically
equivalent matrix values use a 1e-10-point comparison because different floating
operation order can change the last decimal digit.

All ten CLI controls pass qpdf and independent Unicode extraction. Four ordinary
mode/reference PDF raster pairs and the skewed pair are identical using original
marker fonts. This does not establish original-font raster parity. External
receipts under `caj2pdf-hnb-rendering-20261003`:
`input/c8-low-code/{manifest,additional-manifest,skew-manifest}.json`,
`c8-low-code-accepted-comparison.json`, `c8-low-code-skew-comparison.json`,
identified-family-viewer `low-copy{,-validation}.json`, the visible menu-ready
capture and accepted repeated page captures, and `c8-low-code-output/checks.json`.

The full four-page source now reaches style `1021` (size field 1), page-1 byte
10048 at raw glyph `a3db`. The five-page source remains at C8 `801d/3`, page-1
byte 19552. Neither publishes a final PDF. Verify the smaller size and required
punctuation geometry with existing size controls before admitting it. Complete
four-/five-page acceptance remains open in #242.

## Observed field-1 glyph styles

Original `field1_documents()` controls isolate `1021`, `1022`, and `1041`
against field-2 and explicit 24/24 axes. On the pinned viewer, all five page
crops repeat exactly; `1021` equals the explicit-axis reference. The measured
field-1 em is 24 units and its Latin baseline offset is 10 coordinate units.
Unequal-axis controls isolate width and height. Admission is confined to these
three glyph styles; decorations and other field-1 combinations remain errors.

External evidence is under `caj2pdf-hnb-rendering-20261003`:
`input/c8-field1/manifest.json`, `c8-field1-observations.json`, and
`identified-family-viewer/field1-*-{a,b}.png`. At 1457% the field-1 CJK bounds
are (74,85)-(190,200), and Latin bounds are (240,104)-(355,219), relative to
the page crop. The field-2 Latin control is clipped by the right page boundary;
do not use its width to infer full glyph dimensions. Required bracket mapping
and complete-document conversion are separate, unfinished acceptance.

The four admitted controls pass qpdf and independent extraction (`中A`). At
1457%, independent PDF raster bounds retain 0–2 pixel edge residuals relative
to the viewer; `c8-field1-output/render-checks.json` records each edge, including
the clipped field-2/height-1 Latin comparison. No raster compensation is added.
A full-document retry exposed the old punctuation table's assumption that all
admitted fields are at least 2. The writer now rejects unverified punctuation
fields before lookup, with all eight affected raw punctuation codes covered
at the three new styles. The four-page input stops explicitly at byte 10048
rather than panicking or publishing a partial final PDF.

## Field-1 square brackets

Original `small_bracket_documents()` isolates both square brackets under states
0/4 for `1021`, `1022`, and `1041`, plus three shifted fullwidth-comma references.
All 15 source crops are nonblank and repeat identically. Bracket markers identify
the ordinary HGBZ_CNKI resource even under alternate state. The observed offsets
are (21,3), (21,1), and (24,3) source units respectively, applied to the existing
CJK-class transform; width and height controls distinguish the two axes.

At 1457%, bracket bounds relative to the page crop are (113,91)-(229,206),
(113,87)-(229,221), and (119,91)-(254,206). Shifted-comma references have matching
horizontal bounds but their vertical bounds are one pixel higher. Translating
those reference crops by one pixel does not make all raster pixels identical;
retain this residual rather than applying a renderer-specific correction.
The earlier two-glyph exploratory control has overlapping glyphs and is not
used for isolated bounds. No new font or generic punctuation rule is inferred.

External receipts: `input/c8-small-brackets-isolated/manifest.json`,
`c8-small-brackets-observations.json`, `c8-small-brackets-comparison.json`, and
`identified-family-viewer/small-bracket-*-{a,b}.png`, under the existing evidence
root. Unknown small-field punctuation and explicit-axis brackets still fail
before the ordinary size-offset table lookup.

All 15 generated inputs reproduce exactly and pass CLI conversion, qpdf and
independent Unicode extraction. `c8-small-brackets-output/checks.json` records
0–2 pixel PDF/viewer edge residuals without hiding the shifted-reference
mismatch. The full four-page input now reaches byte 19296 (`801d/3`); the
five-page input reaches the same state at byte 19552. Neither is a complete
conversion yet.

## C8 state-3 Latin resource

Eight original `state3_documents()` controls independently identify `801d/3`
as HGBX_CNKI (marker 22). In ordinary mode, Latin A and fullwidth opening
parenthesis select this resource; CJK and fullwidth s retain HGHT_CNKI (marker
61). Their outer bounds match state-0/state-4 controls, whose Latin resources
are markers 23/63. All eight captures repeat identically and are nonblank.

The transition sequence 3/4/3/0 gives markers 22/63/22/23. Mode 1/0/1 followed
by the same style gives 22/61/22/22: CJK mode temporarily changes glyph selection
without losing the Latin state, and a style record retains state 3. Existing
`latinState3` / `--font-latin-state3` transport is reused; no new API or automatic
font discovery is needed. Missing state-3 resources remain explicit errors.

Five admitted controls pass CLI/qpdf and independent Unicode extraction. Three
mode-0 punctuation controls remain explicitly unsupported (the existing CJK-mode
punctuation boundary); these are not counted as successful conversions. The
full-document retries now reach `a6c5` at byte 22428 (four-page) and `a6c8` at
byte 19840 (five-page), both on page 1. Neither publishes a complete PDF.

External receipts under the existing root: `input/c8-state3/manifest.json`,
`c8-state3-observations.json`, `c8-state3-transitions.json`,
`c8-state3-output/checks.json`, and `identified-family-viewer/c8-state*-{a,b}.png`.
The page crop is (648,604)-(1023,721), viewer fit 182%. Marker lookup preserves
font names and uses the existing 84 original fonts. This identifies the role
for these controls, not universal source-font fidelity.

Independent PDF rendering (`c8-state3-output/render-checks.json`) recovers the
same marker sequences, including all four transition/persistence glyphs. The
three ordinary-state PDFs have identical outer bounds; compared with the viewer
crop their edges differ by 1–2 pixels at 182%. These residuals remain explicit.
Node and real Worker tests include a distinct state-3 font source, bounded short
reads and cleanup, reusing the existing font transport tests.

## Required Greek letters

Three original `required_greek_documents()` controls establish raw `a6c5`
(U+03B5 epsilon) and `a6c8` (U+03B8 theta) in ordinary mode/style `10a5`.
Both follow the current Latin resource: markers 23, 22, 63 for states 0, 3, 4.
Their vertical bounds equal the fullwidth-comma symbol control rather than the
ordinary Latin A baseline. Repeated nonblank crops are identical. Only these
two raw codes are admitted in C8; adjacent Greek codes and HN-B are not inferred.

Fresh ordinary Copy, selected by its visible menu item, independently returns
U+03B5 and U+03B8. The same transaction returns U+0082 for the comma control;
that anomalous control value is retained, not treated as evidence of comma
Unicode or used to change the existing mapping. Selection-highlighted/menu
captures are not rendering baselines.

External receipts under the existing root: `input/c8-greek-required/manifest.json`,
`c8-greek-required-observations.json`, `identified-family-viewer/c8-greek-*-{a,b}.png`,
`greek-copy-menu-ready.png`, and `greek-copy{,-validation}.json` in that viewer
directory. The copied payload has a fresh selection-owner transition and full
bounded transfer; this scoped evidence is not universal searchable-text parity.

The three supported controls reproduce exactly and pass CLI/qpdf and independent
Unicode extraction. Independent PDF raster markers match all four source glyphs
in each state, with 1–2 pixel outer-edge residuals retained at 182%.
`c8-greek-required-output/checks.json` records these checks and complete-input
retries: four-page `8090/a3e6` at byte 22512, five-page `80d5/0` at byte 20808.
Both remain located failures without a published final PDF.

## Investigating `8090/a3e6` radical drawing (not admitted)

Six original `radical_record_documents()` controls establish that the observed
candidate 12-byte record paints a radical shape, not a ignorable state change.
At fixed coordinates, changing the last word 125 to 60 reduces its vertical
extent while preserving horizontal extent; changing the preceding low value
143 to 64 reduces horizontal extent while preserving vertical extent. Moving
the candidate position by (60,40) translates the shape. The following CJK/Latin
anchors remain available for state/placement comparison. Flagged (`c000` bits)
and unflagged coordinate/extent controls render identically in this experiment.
These observations do not establish every flag combination or final framing.

All six nonblank page crops repeat exactly. Relative to the page crop, the
baseline-subtracted bounds (including antialiasing) are: observed (1,53)-(133,139),
short (1,53)-(133,98), moved (39,78)-(171,142), narrow (1,53)-(84,139). The cropped
"top region" measurement clips the moved shape and is not its complete bounds;
use the baseline-subtracted measurements. The root's exact path vertices,
stroke width/join and coordinate anchoring remain unverified. Keep conversion
explicitly unsupported until these are established with asymmetric controls
and malformed/truncated framing tests in the existing parser/composer.

External evidence: `input/c8-record8090/manifest.json`,
`c8-record8090-observations.json`, and identified-family-viewer
`record8090-{base,record,short,moved,narrow,unflagged}-{a,b}.png` in the existing
root. Crop (648,505)-(1024,819), viewer fit 486%. The first `record8090-initial`
capture shows the startup document and is excluded; `record8090-confirmed`
shows the correct input. The viewer was restarted after confirmed exit 124.
No production behavior or complete-document support changes in this step.

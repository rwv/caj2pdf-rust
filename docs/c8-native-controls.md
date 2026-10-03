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

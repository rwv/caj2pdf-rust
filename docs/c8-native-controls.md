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
gray to black in these contexts. These remain explicit errors pending their
complete state semantics. The initial `81ff/3` mixed probe was followed by
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

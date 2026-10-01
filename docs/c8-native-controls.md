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

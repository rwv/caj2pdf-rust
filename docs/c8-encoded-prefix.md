# C8 encoded-string record framing

This is parser work for #242. It does not establish complete native C8
conversion or authorize ignoring unknown required resource semantics.

## Evidence

Both pinned additional C8 inputs in `tests/conformance/matrix.json` start
every page's indexed native span with `80cc/01xx`:

| Input | Pages | Value | Following encoded words |
| --- | --- | --- | --- |
| `issue-90/4-[21].caj` | 4 | `011e` | 28 |
| `issue-90/4-[24].caj` | 5 | `011a` | 24 |

All observed payload words have the form `e000 | printable ASCII`. The
following pair is `801d/value`. These observations are from bounded reads
of hash-verified source intervals, not another converter's implementation.
External strings and document content are not reproduced here.

`tools/cajviewer/c8_encoded_prefix_fixture.py` creates seven original controls:
a baseline, prefixes containing 0, 5, 28, 253 or 254 characters, and a
five-character record between run controls and the glyph. They use
one fixed-position glyph and the invented string `fixture` repeated to the
requested length. No external document or font data is needed to generate
these files. The observed format identifier is a format fact.

In pinned offline CAJViewer 9.0.0, with the original geometric font from
#240, the 0/5/28/253-character controls retain the baseline page pixels.
The 254-character probe (`value=0200`) displays a blank page instead.
Repeated captures are identical for these controls. After the bounded viewer
session expired, a fresh baseline and in-run control also matched exactly,
including the repeat; this checks preservation of active run context. Comparisons use the
same outer page frame `(449,231,1574,1093)` at displayed fit-width 2896%;
there is no content registration or scaling of screenshots. These are
framing controls, not full-document fidelity passes or proof that all
encoded strings are semantically ignorable.

The generator reproduces the observed controls byte for byte. Receipts are
outside Git under `caj2pdf-c8-size-scale-20261001`; the nine-page prefix
inventory is in `caj2pdf-native-profile-inventory-20261001/prefixes.json`.

## Admitted parser contract

- Tag `80cc`, value `0102..=01ff` only.
- The low byte counts **all 16-bit words**, including tag and value.
- Each of the remaining 0..=253 words must be `e020..=e07e`.
- Validate the entire payload within the indexed page span before delivering
  one `NativeRecord::EncodedString` event. Preserve value and source span;
  do not expose it as glyphs or interpret a pathname/resource role.
- Reuse the fixed 28-byte parser buffer. No string allocation, marker scan,
  new budget, or payload-sized retained object is introduced. The existing
  byte/record budgets, cancellation and cursor poisoning still apply.
- Other values/encodings fail explicitly. In particular, do not treat
  `0200` as a longer record or wrap its low byte into an empty record.

This does not resolve subsequent controls such as `80ce/0001`, `8024`,
`8021`, `81ff` or additional font/style semantics. Unknown required content
continues to stop conversion. HN-B framing is separately tracked by #241;
the C8 visitor is not automatically enabled for HN-B.

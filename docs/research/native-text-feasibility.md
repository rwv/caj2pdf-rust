<!-- SPDX-License-Identifier: MIT -->

# Native text feasibility (#223)

## Current status

The bounded study below is historical. Its visible native-text work subsequently
landed on main for the admitted HN-B/C8 profiles; see the
[current support summary](../conformance.md#current-support-and-release-status).
The [Unicode checkpoint](hnc8-text-fidelity.md) separates verified character
transport and selected source checks from unverified general reading order,
whitespace and searchable image content. No OCR or general extraction API was
added. Historical observations and receipts below remain unchanged.

## Historical decision

**Go for the measured C8 native-glyph work in #229; no-go for a general
searchable HN/C8 PDF feature at this point.** The source demonstrably contains
some Unicode-mappable characters and glyph positions. Complete record grammar,
font/style resolution, vector controls and reading order remain prerequisites.
No OCR, font framework or production extraction API is added by this study.
Visible native-text pages are required content, independently of adding an
invisible text layer to image-only documents.

## Three input controls

| Input / selected page | Classification | Evidence and limits |
| --- | --- | --- |
| C8 issue-66, pages 1–2 | Source-native text, with a diagram on page 1 | A two-byte character mutation changes only its visible glyph; position mutations move that glyph/run. Page 2 has visible text with zero image descriptors. Full text/font interpretation is not established. |
| HN-A issue-85 Zhouli, page 1 | Image-only page | Its 40-byte raw text span contains two page-prefix records, one image placement record and a terminator, with no glyph records. The cover content is in its JPEG. This says nothing about other pages' text. |
| CAJ issue-77, selected region of page 75 | Ordinary-copy Unicode available; source-native origin unknown | Reuses the pinned September 29 snapshot: 161 UTF-8 bytes / 83 code points. Copy alone does not establish whether the viewer used native text or internal OCR. |

The external copy artifact was rechecked against the manifest: SHA-256
`21e2ab5568d1536bfcb3ced444e734097a02b7347e62ad8a89012c42834de07c`.
The catalog hash remains
`c438c575a004727040a75cbac11055c040408d2cfbd0c33c9d5df0b0c8064a5e`.
This reuses a selected-region observation, not an approved rendering baseline:
[the snapshot](cajviewer-fixture-snapshot.md) retains its failed acquisition
receipt and missing telemetry. No new ordinary-copy result is claimed for C8
or HN-A. Raw text remains external and is neither normalized nor committed.

## C8 direct evidence

Source SHA-256:
`90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6`.
The same pinned offline Viewer 9.0.0 image, 96 DPI and 100% zoom were used for
three bounded mutations. Full-file comparison confirms that only the intended
word changed. Each original/control screenshot repeats identically.

- At offset 218, `c4 ce` → `e2 b2` changes `文` to `测`; 134 pixels change in
  a 12×12 glyph area. The little-endian payload word interpreted as big-endian
  GB18030 bytes agrees for those characters.
- At offset 216, position 7952 → 8052 moves only that glyph horizontally;
  the original/new glyph union spans 25×12 pixels, consistent with 13 pixels
  of movement at this scale.
- At offset 206, the preceding `0x8001` payload 4760 → 4860 moves its five
  glyphs down, ending at the next position-setting record. The changed area
  spans 53×26 pixels.

Those observations establish neither a complete character map nor physical
coordinate units. Header origin words, `0x8002` style/font words, `0x801d`
nonzero values, `0x8067` controls, the special A0xx Latin range, drawing records
and image fields remain partly or wholly unknown. Source record order is not
proven reading order. Font/glyph-outline availability and a portable font
strategy remain unverified; no font or proprietary glyph data is distributed.

## Small bounded prototype

`scripts/c8_text_probe.py` reuses the existing source-index reader, reads at
most four bytes per record, retains only one record plus position/style state,
and caps a text span at 1 MiB and records at 65,536. It emits provisional JSONL
records in source order. It does not accumulate a page or document in memory.

Only observed neutral controls and four-byte glyph records are traversed.
A GB18030 character candidate is emitted only when the existing Python codec
returns one assigned, non-control, non-private-use character. Other codes get
`null`; in particular the special A0xx range is **not** silently mapped to
private-use Unicode. No character is invented or normalized. Font words and
coordinates remain raw. Unknown/image/vector controls stop at their source
offset instead of having their payload reinterpreted as glyphs.

The real page-1 probe emits 239 provisional glyphs (208 mapped candidates,
31 unmapped), then stops at the nonzero `0x801d` control at offset 1356.
Page 2 emits six (three mapped, three unmapped), then stops at `0x8006` at
7461, before vector point data. Both exit nonzero and explicitly report
`complete_text: false`. These are measured incomplete prefixes, not successful
page extraction or compatibility passes. A terminator also never implies
complete Unicode or reading order.

Example (redirect only to an external evidence directory):

```sh
python3 scripts/c8_text_probe.py /external/corpus/input.caj \
  --sha256 90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6 \
  --page 1 > /external/evidence/page-1.jsonl
```

Original MIT tests cover non-ASCII mapping, source versus reading order,
position resets, unknown/vector controls, undecodable codes, short reads,
bounds, cancellation and partial/missing records. The Python diagnostic uses
stdlib decoding as a black box, never another converter's implementation.
It is not a new Python runtime requirement for Rust/JavaScript conversion.

## Subsequent implementation evidence

The later [native-record work](c8-native-records.md) verifies the 62 A0
alphanumeric codes, adds a bounded Rust visitor and character helper, and
classifies this sample's application-info tail. The prefix-only Python probe
and measurements above remain the original bounded feasibility result.

## Next implementation

#229 owns the observed visible C8 profile, including verified Unicode mapping,
font resolution/metrics, correctly positioned PDF glyphs and ToUnicode,
source-page completeness, bounded memory and CLI/Node/browser equivalence.
Unknown glyphs or vectors must not disappear. Use original synthetic controls
and external vendor comparisons for each admitted rule. Keep general invisible
text-layer/searchability work deferred until complete mappings and reading
order are established. Other HN-B/C8 samples require their own evidence; this
C8 study does not automatically explain every image-less row in #220.

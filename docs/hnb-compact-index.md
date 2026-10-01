<!-- SPDX-License-Identifier: MIT -->

# Compact HN-B page index

Tracking: #236, under page-completeness issue #220. This is container metadata
support, not native-text rendering or successful conversion of the source.

## Evidence and discriminator

The pinned issue-100 source has SHA-256
`3f3b9b57d6925df811247dced47fd7fb74cf0f678ab9bfda0827c827258ab39b`
and 26,450 bytes. Its HN-B outer marker is 200, its index starts at 216,
and its declared page count is four. Unlike the measured 20-byte HN-B inputs,
its little-endian u32 at offset 136 (`0x88`) is zero rather than `0xc8`.

| Page | Text offset | Text bytes | Third u32 |
| --- | ---: | ---: | ---: |
| 1 | 264 | 5,904 | 0 |
| 2 | 6,168 | 6,718 | 0 |
| 3 | 12,886 | 6,378 | 0 |
| 4 | 19,264 | 7,186 | 0 |

Four 12-byte rows end at 264. These spans are contiguous and end exactly at
EOF. The old 20-byte interpretation overlaps text and produces misleading
negative-count/out-of-range diagnostics. Those historical diagnostics are
not evidence that this source is corrupt. The pinned offline CAJViewer 9.0.0
reports four pages and displays visible text on the first page.

## Independent original controls

Generate four source-independent two-page controls into a new directory:

```sh
python3 tools/cajviewer/hnb_index_fixture.py /tmp/hnb-index-controls
```

Page 1 contains eight rows of `中文AM1`; page 2 contains four rows of `PAGE2`.
The text spans are 292 and 148 bytes. The controls vary only the index width,
its required absolute offsets, and the marker at 136. Ordinary viewer page
navigation establishes the following:

| Marker | Actual row width | Observed second-page content |
| --- | ---: | --- |
| `0xc8` | 20 | Correct four `PAGE2` rows |
| `0` | 12 | Correct four `PAGE2` rows |
| `0xc8` | 12 | Incorrect first-page content retained |
| `0` | 20 | Blank second page |

A page-count label or successful open alone is insufficient: the crossed
controls still open. The two correctly paired controls establish the explicit
marker rule; do not infer a width from the first text offset or retry a broken
layout under another width. The generator reproduces all four observed files
byte for byte. Original/repeated captures and manifests remain external in
`caj2pdf-c8-grid-20261001`.

## Reader contract and limits

For HN-B, the reader selects 12-byte rows for marker zero and 20-byte rows
for `0xc8`. Other marker values fail at byte 136. This check does not affect
HN-A or C8. Existing synthetic 20-byte HN-B fixtures now contain the explicit
`0xc8` marker rather than relying on an unverified zero-filled header.

Compact rows contain signed checked offset/length fields followed by a u32
whose only admitted value is zero. Its nonzero meaning is not established;
reject it instead of treating it as an image count. The admitted compact
profile reports zero images, so it cannot be converted by inserting blank
pages or emitting only images. Native text remains required under #220.

The checked index span, page probing and row offsets use the selected width.
A fixed 20-byte scratch array is reused, but compact reads request only 12
bytes, further limited by the caller's I/O budget. Compact text cannot overlap
the index. Existing signed bounds, page/span budgets, source errors,
cancellation and poisoned-cursor behavior remain in effect. Aliasing text
spans across page identities retains the existing reader contract.

`PageRecord::unknown` retains its fixed ten-byte representation: for compact
rows only its first two bytes correspond to source offsets +10/+11; its last
eight bytes are explicitly padding, not invented source fields. The admitted
zero third word makes all ten bytes zero. `Header::page_index.length` reflects
the actual index width multiplied by page count.

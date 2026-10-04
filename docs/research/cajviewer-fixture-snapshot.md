# CAJViewer fixture snapshot

Observed on 2026-09-29 for #126 and #127. This is a small external reference
snapshot, not a converter compatibility result or an approved release baseline.

## Captured scope

| External source | Viewer pages | Captured pages | RGB grid | Ordinary copy |
| --- | ---: | --- | --- | --- |
| CAJSamples issue 77, CAJ | 75 | 1, 75 | 651 × 843 | Page 75: 161 UTF-8 bytes, 83 code points, selected region |
| CAJSamples issue 33, test3.caj (PDF magic) | 11 | 1, 11 | 633 × 860 | Page 1: empty result, UNAVAILABLE |

These are explicit subsets. Page counts were observed in the viewer. No HN,
C8, KDH, converted PDF, browser or Node comparison was performed.

The known Unicode control returned `RUST 9876543210 中口一`. The image-only
control retained its clipboard sentinel and offered no ordinary Copy action.
The real CAJ selection replaced a different sentinel with nonempty text.
Raw bytes were saved unchanged; no OCR or normalization was requested. Ordinary
copy does not establish whether the viewer used embedded text internally.
Other unattempted text pages are NOT_RUN. Empty PDF copy is UNAVAILABLE,
not evidence of a text-free page or a successful comparison.

## Repeatable capture settings

Use the [pilot recipe](cajviewer-capture-pilot.md), with these refinements:

- CAJViewer 9.0.0, image
  `sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`.
- Xvfb 1600 × 1200, 24-bit, 96 DPI; Openbox; maximized viewer.
- Mount NotoSansCJK-Regular.ttc read-only alongside the image's fonts. The
  external manifest hashes the actual font and retains the font listing.
- Close the sidebar, select single-page view and 80% zoom. Confirm the visible
  page number and all four page edges before every capture.
- Extract the physical page rectangle from the retained desktop screenshot:
  CAJ `(499, 245, 651, 843)`, PDF `(508, 237, 633, 860)`, expressed as
  `(x, y, width, height)`. Do not resize or align by content.
- Save lossless PNG and its decoded RGB bytes one page at a time. The external
  assembly script creates both from the same cropped image.
- Select the ordinary `复制` menu label. Ctrl+C invokes enhanced copy in this
  version. The CAJ text selection was viewport rectangle `(590,307,1045,385)`
  expressed as corner coordinates; it is not whole-page text coverage.

Page navigation attempts intended for page 2 unexpectedly reached the last
page. The snapshot records the visibly observed pages 75 and 11, never page 2.
Automated whole-document navigation remains unproven. One exploratory file
named `b-page2-verified.png` is actually page 11; the manifest maps it to 11.

A fresh container captured CAJ page 1 again: the entire extracted RGB grid
was exactly equal, with zero changed pixels. This result uses the new font
and sidebar settings. It does not replace the earlier pilot's unequal reopen
captures or prove stability for every page.

## Bounds and evidence

Each viewer container had no network, a read-only root, a 1 GiB memory cap,
128 PID limit, 20-minute timeout, 64 MiB file limit and 128 MiB output tmpfs.
Individual recorded helper commands had a 15-second timeout. Both containers
were stopped and removed. Observed container memory peaks were 881,770,496
and 495,579,136 bytes; these include the viewer/display/tmpfs, exclude host
helpers, and are not Rust conversion measurements.

The external snapshot is `cajviewer-fixtures-20260929/snapshot-v1`, containing
the catalog and separate bundle/source/runtime roots. Its integrity check
passed: four image payloads, one text record, 38 file checks, zero comparisons.
The catalog SHA-256 is
`c438c575a004727040a75cbac11055c040408d2cfbd0c33c9d5df0b0c8064a5e`.

Review remains REVIEW_REQUIRED and the acquisition receipt remains FAIL:
there was no pre-acquisition audit, and aggregate elapsed time and disk peak
were not measured. Null retains those unknowns without inventing zeroes.
The journal describes issued helper commands, not a prospectively frozen
schedule or every host action. Integrity verification cannot retroactively
establish acquisition telemetry. Existing baseline promotion rejects this
receipt; do not treat this snapshot as an approved regression baseline.

## Next work

Use the four pages for a small exploratory native conversion comparison in
#129. Review page framing and any actual mismatch before expanding formats
or GUI automation. If an approved baseline is needed, reacquire only the
selected pages with the existing receipt requirements; do not add another
inventory or proof framework. Keep unavailable text independent of images.

Existing fixture/comparator tests cover missing artifacts, dimensions, page
order, changed characters and text bounds; clipboard controls cover freshness.
The added receipt test accepts unknown telemetry only for failed receipts.
All source documents, fonts, installer bytes, images, copied text and full
receipts stay outside Git. Normal CI uses original MIT controls.

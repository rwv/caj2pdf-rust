<!-- SPDX-License-Identifier: MIT -->

# HN/C8 and KDH viewer checks

Observed on 2026-09-29 for #123, using the conversion implementation at
`bb402cb` (#182). These are selected-page observations, not a claim of complete
CAJViewer compatibility. Source documents and captures remain external.

## Results

| Format / source | Pages checked | Source / output page count | Result |
| --- | --- | --- | --- |
| HN-A, issue 76 Ren document | 1, 23 (mixed images) | 163 / 163 | Different page-frame dimensions; not a pixel pass |
| C8, issue 58 Xie document | 1, 4 | 4 / 4 | Different page-frame dimensions; not a pixel pass |
| KDH, issue 48 `ZZXX200402047.caj` | 1 | 1 / 1 | Exact complete-page pixels, zero differences |

The HN/C8 source and output identities are recorded in
[public-interface validation](js-validation.md#complete-multi-image-hn-a-public-interface-check).
HN-A outline titles, hierarchy and destinations were checked separately in
#182. C8 conversion explicitly omitted unverified bookmarks. The KDH source
SHA-256 is `5f6f1af5b148af2b6756ff8878124c09797882505d0aca12ca983d9073d94507`;
output SHA-256 is `82fc33ce060e14acaf68fa52b592f15d0e481db57a95103c068925518287e6ca`.

All ten settled source/output views were captured twice without reopening;
each pair of full desktop RGB captures was identical. This establishes
same-view repeatability only. It does not replace the earlier pilot's unequal
reopen observations or establish fresh-process stability for these documents.

The KDH page frame is desktop rectangle `(529, 262, 591, 799)` as
`x, y, width, height`, including the thin page border. Both decoded RGB hashes
are `8b2a669f6e33d70814dbe247bb1327e34a40ce5ed947d1f788a50a55d30025da`.
The entire document viewport `(65, 146, 1518, 1032)` also matched exactly,
so the page extraction did not hide a surrounding difference.

## Retained HN/C8 differences

Reviewed page-frame rectangles include the thin border and exclude outer
shadow. No content alignment, resampling or tolerance was applied.

| Source page | Source rectangle | Output rectangle |
| --- | --- | --- |
| HN 1 | `(536, 225, 578, 883)` | `(642, 385, 367, 564)` |
| HN 23 | `(536, 156, 578, 883)` | `(534, 156, 582, 881)` |
| C8 1 | `(519, 156, 611, 865)` | `(522, 156, 606, 863)` |
| C8 4 | `(519, 304, 611, 865)` | `(521, 306, 607, 863)` |

These unequal grids are `DIMENSION_MISMATCH`; a changed-pixel count between
unequal page grids would not be an exact page comparison. In particular,
the HN cover image is smaller in the output and lacks the source viewer's
additional right/bottom page space. Mixed-page and C8 boundaries also differ;
the HN mixed page has visibly different rasterization. Current HN/C8 geometry
matches the documented Python-derived empirical profile, but does not reproduce
these vendor page extents. Screenshots alone do not establish the correct
source geometry rule. #123 retains that investigation; #14 must carry this
limitation until it is resolved. No converter geometry was changed in this run.

## Environment and acquisition

Reuse the [existing capture recipe](cajviewer-capture-pilot.md): Linux Viewer
9.0.0, image `sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`,
Xvfb 1600 × 1200, 24-bit, 96 DPI, Openbox, NotoSansCJK-Regular.ttc mounted
read-only, maximized window, sidebar closed and displayed zoom 80%.
Raw `fc-list` SHA-256:
`a191f6fb6530c5a6c2399615cf0062f4883fad89a5815da8262ff2777265f41c`.

HN page 1 used single-page mode for both inputs. Other rows used continuous
mode, which exposes the complete selected page plus adjacent-page fragments
outside the recorded rectangle. Single-page navigation repeatedly landed on
page 163 instead of requested page 23; those captures are excluded. In
continuous mode, double-click the page field, use Home then Shift+End to select
the number, type the target and press Enter. Verify the visible page indicator
and all physical edges after navigation; filenames alone are not evidence.

The offline, read-only container retained the recipe's CPU, filesystem and
20-minute bounds. Opening the large PDF with the HN source already open hit
its original 1 GiB memory cap (`oom_kill=1`, peak 1,073,741,824 bytes).
A fresh viewer process opened it after the cap was raised to 2 GiB. A later
KDH open hit the 128-task cgroup limit (`pids.events: max 5`), with
`std::system_error: Resource temporarily unavailable`. A fresh process opened
KDH; raising the limit to 256 also allowed further capture commands. These
failures are retained, not compatibility failures or converter memory results.
The owned container was stopped and removed after capture.

The external `caj2pdf-hn-viewer-20260929` directory retains full desktop
captures, extracted page frames, hashes, comparison JSON, commands, logs and
resource records. No vendor program, document, derived image or codec state
data is committed. Ordinary copy was not attempted here: text is NOT_RUN.
This adds neither OCR nor a new baseline-management workflow.

## Cross-interface reuse

CLI, Node 24.13.0 and Chromium produced byte-identical PDFs for the actual
CAJ/PDF/KDH documents used by these and the previous viewer captures:

| Format | Pages / bookmarks | Output SHA-256 |
| --- | --- | --- |
| CAJ, issue 77 | 75 / 58 | `17af66b3201925945c16cbfbda3b587cc3cd2369eafe9ddfcd2f71f15f72c1cf` |
| PDF, issue 33 | 11 / 0 | `fcf88d82a59391b73c90c799c22c8002a0ab16ffd3617ced04c974635c101a75` |
| KDH, issue 48 | 1 / 0 | `82fc33ce060e14acaf68fa52b592f15d0e481db57a95103c068925518287e6ca` |

HN/C8 byte identity is already recorded in #182/#180. Thus each viewer result
can be shared across those three interfaces; this does not turn HN/C8
mismatches into passes. The small CAJ/PDF/KDH verification collected output
for hashing and made no memory-efficiency measurement. CI continues to use
original MIT comparator and conversion fixtures without the external corpus.

## Controlled geometry checks

A follow-up on 2026-09-29 changed six external source copies, reusing the same
viewer image, fonts, 80% zoom and display. Byte comparisons verified that each
copy changed only the declared little-endian fields; image payloads and page
indexes were unchanged. No reference-converter implementation was inspected.

| Input / field | Change | Observed viewer effect |
| --- | --- | --- |
| HN cover image record, absolute offset 33184 | 5327 → 2663 | Image width halves; image height and page frame stay unchanged |
| HN cover image record, offset 33186 | 8173 → 4086 | Image height halves; image width and page frame stay unchanged |
| HN document header, offset `0xa8` | 5579 → 2789 | Page width halves, clipping the unchanged image |
| HN document header, offset `0xaa` | 8528 → 4264 | Page height halves, clipping the unchanged image |
| HN mixed page 23, first image record, offset 1120742 | 5557 → 2778 | The first image compresses horizontally; the separate overlaid illustration and page frame remain unchanged |
| C8 document header, offsets `0x20` / `0x22` | 5901 / 8354 → 2950 / 4177 | Both page dimensions halve, clipping rather than scaling the content |

The image fields are at `+8/+10` from the raw `0x800a` record marker
(`+4/+6` from the coordinate-only tail representation). These interventions
separate declared image extents from decoded pixel dimensions, and document
page extents from first-image extents. They explain why a 300-DPI pixel-based
cover and first-image-based page can differ substantially from the viewer.
They do not independently establish the physical unit, signedness, all profile
variants or pixel-perfect rendering. Do not extrapolate the HN-A header check
to HN-B without evidence.

Nine settled view/repeat pairs were identical without reopening. Page 23 used
continuous mode; other observations used single-page mode with visible page
indicators checked. One early blank page-23 capture and an incorrectly entered
zoom capture were excluded. The external `caj2pdf-hn-geometry` directory retains
source hashes, byte-change checks, repeat hashes, commands and captures in
`changes.json`, `verification.json` and `actions.jsonl`. The offline container
used 2 GiB / 256 tasks, peaked at 1,401,978,880 bytes, reported no OOM or task-limit
hits, and was removed after capture. These are viewer measurements.

The #123 correction uses these declared page/image extents in the existing
bounded parser/composer. Original fixtures vary them independently of decoded
pixels and check that DIB padding is not painted. The original conversion hashes
and mismatches above describe the pre-correction build; fresh vendor comparison
for the corrected build remains pending. The physical unit remains empirical.

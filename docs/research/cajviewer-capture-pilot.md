<!-- SPDX-License-Identifier: MIT -->

# CAJViewer capture pilot — 2026-09-29

This completes the practical capability experiment in #124. It establishes
whole-page screenshot acquisition and ordinary-copy behavior, with an explicit
reopen stability limitation. It does not establish Rust conversion parity or
finish the fixture/comparator tasks #126–#129.

## Observations

| Input | Observation |
| --- | --- |
| Original `digital.pdf` | Opened; four-page control visible |
| Original `second-text.pdf` | All four colored edges/corner marks captured, 1125 × 843 RGB pixels |
| Original `image-only.pdf` | Complete marked page visible; no text selection/copy observed |
| External CAJSamples `issue-77` | Viewer reports 75 pages; physical page 1 captured, 651 × 843 RGB pixels |

Ordinary copy from the positive control returned exactly
`RUST 9876543210 中口一` through drag selection → right-click → Copy. No OCR or
enhanced copy was used. Ctrl+A/Ctrl+C attempts left the clipboard sentinel
unchanged; the context menu worked. The image-only control left its distinct
sentinel unchanged and did not offer Copy in the observed context menu.
This is a negative observation, not an empty-text comparison pass.

Reopening documents within the same application process gave equal page
sizes but **different pixels**: 30,518 changed pixels for the positive control
and 6,050 for the CAJ page. Two later captures of the same still-open CAJ page
were byte-identical. The reason for reopen differences is not established;
settings/history/rendering may need investigation in #126. No tolerance,
content alignment or reference replacement was applied. Fresh-process
stability and source-versus-converted-PDF equality remain untested.

The application UI has missing Chinese font glyphs in this environment.
Embedded control glyphs and the sample's page imagery are visible. The sample
page also has partially clipped header content at its physical top edge in
both single and continuous modes; this is retained as observed, not corrected.
Capture covers the physical page rectangle, not a claim that the viewer
renders all source content correctly.

## Environment and external evidence

- Viewer: Linux 9.0.0; offline, no network.
- Image: `sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`.
- Tool checkout: `fce1e4a92e9974aa614332bf5a867bf04873ab34`.
- Display: Xvfb `:99`, 1600 × 1200, depth 24, 96 DPI; maximized Viewer.
- Packages: Xvfb `2:21.1.7-3+deb12u9`, Openbox `3.6.1-10`,
  xdotool `1:3.20160805.1-5`, xclip `0.13-2`, fonts-dejavu-core `2.37-6`.
- Raw `fc-list` SHA-256:
  `bf8ebe5e39be4fb9308716f04e7a10157e6ff309ff43fe65d09b62f57d5a3c9b`.
- External sample SHA-256:
  `5d988d74a6e6a0c392eb58297e70d91ff2e1ad2c2887374a04adc67a253ac2ab`.

Artifacts remain in the operator's external `cajviewer-pilot-20260929`
directory: full desktop captures, page PNGs, raw copied control text,
`fonts.txt`, `packages.txt`, application logs and `result.json`. No sample,
vendor binary or rendered document is committed. The container was stopped
and removed after the experiment. Earlier failed attempts remain unchanged.

| Artifact | Decoded RGB SHA-256 |
| --- | --- |
| `control-page.png` | `8b47b1de1062b92d2ebccceb16ed57d33a956b12b960180d461c8aa61d0ac2b7` |
| `control-page-repeat.png` | `7574251907c7288825c3b9042102bbbd873dfc79994f8ae3581c6482b73e105a` |
| `sample-page-1.png` | `08ad3c3f82762051614650310737ec966cdbee3e52530d2b33227610695303e0` |
| `sample-page-1-repeat.png` | `948ea6f2f8a78e4f2390145bd1ffcca570da039e40b9807ef4f223d4b90d466b` |

## Repeat the experiment

Use the existing external image above; do not distribute it. Original controls
come from `scripts/cajviewer_canary_fixtures.py`. Mount them at `/input`, an
external output directory at `/output`, and this checkout's
`tools/cajviewer` at `/tools` (read-only). Copy the chosen external sample to
`/output/sample.caj`, recording its original identity/hash.

The successful run used Docker's `--network none --read-only --cap-drop ALL
--security-opt no-new-privileges --memory 1g --memory-swap 1g --cpus 2
--pids-limit 128 --shm-size 128m`, the image's UID/GID 1000, and these tmpfs
mounts: `/tmp` (128 MiB), `/home/canary` (128 MiB, owner 1000), `/runtime`
(16 MiB, owner 1000, mode 700). Set `XDG_CACHE_HOME=/tmp`; retain the image's
other environment, including `DISPLAY=:99` and its existing QtWebEngine
sandbox setting. The entry point was `timeout 900 sh /output/start.sh`.

`start.sh` starts `Xvfb :99 -screen 0 1600x1200x24 -dpi 96 -nolisten tcp
-noreset`, waits up to five seconds for `xdpyinfo`, starts `openbox --sm-disable`,
and launches `/opt/cajviewer/bin/start.sh /input/digital.pdf`. Redirect each
program's output to a separate external log and wait for the child processes.
The pilot used one application launch; document reopen is not app restart.

1. Maximize the Viewer by double-clicking its title bar. Use Ctrl+O, wait for
   the open dialog, type an absolute path, then press Enter. Sending text before
   the dialog is ready can lose the filename; check the actual UI.
2. Open `second-text.pdf`, then `image-only.pdf`. In this layout, both fit fully
   at the displayed 329% width-fit setting. Verify all colored edges and corner
   markers. Do not generalize these coordinates to different display layouts.
3. Open `sample.caj`. Enter `80` in the zoom box and select View → Single Page.
   Verify page `1/75`. Reopening resets to continuous mode/width fit, so apply
   these settings again each time.
4. Move the pointer outside the page, clear selection, and visually verify
   readiness, page identity and all four physical edges. Capture the desktop;
   retain it alongside the page rectangle so framing can be reviewed.
5. For the observed layout, control bounds were `(450, 241, 1125, 843)` and
   sample bounds `(686, 245, 651, 843)`, as `(x, y, width, height)`. These exclude
   application chrome and the sample's gray page border. Extract exactly that
   rectangle without scaling or content-based alignment.
6. Before copy, set a distinct clipboard sentinel with
   `printf PILOT_SENTINEL | xclip -selection clipboard -in`. Select text with
   the arrow selection tool, right-click the selection and choose the visibly labeled ordinary
   Copy (复制) item. Do not use Ctrl+C: the later font-enabled experiment
   identified that shortcut as enhanced copy. Menu positions can vary. Read with `timeout 3 xclip -selection clipboard -out`. Preserve
   raw bytes. Repeat with a new sentinel on the image-only page.
7. Close and reopen the document, reapply settings and capture again. Compare
   complete equal-sized decoded RGB payloads and report actual changed pixels.
   Keep differences; do not silently promote a visual resemblance to equality.
8. Stop and remove the owned container. Keep logs and captures externally.

The existing small X11 client can save a bounded desktop PPM without adding
an image library to the container. Run this inside the container:

```python
import sys
from pathlib import Path
sys.path.insert(0, "/tools")
from capability_x11 import X11

client = X11()
try:
    pixels = client.image((0, 0, 1600, 1200))
    Path("/output/desktop.ppm").write_bytes(b"P6\n1600 1200\n255\n" + pixels)
finally:
    client.close()
```

Run each capture command under a short host timeout (the pilot used ten
seconds). Each desktop RGB payload is 5,760,000 bytes; capture only the named
pages and process one image at a time. The pilot kept finite screenshots and
logs under the 15-minute session deadline; it did not enforce a total quota
on the output bind mount. Future unattended batches should add an output
quota/log-size limit. This capability experiment measured neither converter
memory nor general-purpose batch resource behavior.

## Next work

#126 should expand to 2–3 documents, write #125 manifests and assess fresh
session stability. #127 should capture actual external text if available.
#128 can implement exact comparisons against original generated fixtures now;
these observed reopen differences remain review-required, not passes. #129
must still compare real Rust/CLI/browser/Node output. No converter comparison
ran in this experiment.

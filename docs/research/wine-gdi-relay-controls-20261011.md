# Wine public font and bitmap API observations

Issue [#541](https://github.com/rwv/caj2pdf-rust/issues/541), under #406.
The [receipt](wine-gdi-relay-controls-20261011.json) records twelve cold Wine
sessions, independent API probes, retained-log hashes, filtering counts and
exact frame comparisons. Production and original font programs are unchanged.

## Result

Selected public GDI calls do not reveal the missing glyph-origin or axis rule.
They do provide a narrower observation boundary: the last bitmap submission
uses identical dimensions for the native source and PDF at every tested view.
The selected viewer traces contain no `GetGlyphOutlineA/W`, `GetGlyphIndicesW`
or `GetFontData` call. The independent positive program successfully calls and
logs the corresponding W APIs in every session. Retained font-creation messages
include UI faces, but no HG-prefixed content face.

All **twelve observed frames equal their unobserved reference frames exactly**.
Every source/PDF pair still differs. This is evidence for choosing the next
experiment, not a production correction or a claim that GDI never participates
in rendering. The default `RelayFromExclude` remains active, and this study does
not identify the internal font engine.

## Setup and failed retention

The inputs are three original axis controls from
[`wine_axis_rasters.py`](../../scripts/research/wine_axis_rasters.py) and their
original-font PDFs produced by `7f8b7e7`. They cover low forward 75%/150%, reversed
and shifted 100%, and high 600%. Resource font programs, viewer/profile,
single-page layout, 1600 × 1200 display and 96 DPI are held fixed.

The first four sessions cover low forward source/PDF at 75% and 150%. In a
fresh prefix copy, `RelayInclude` lists selected GDI functions and `RelayExclude`
is removed. Wine initialization runs without relay tracing. An independent
program runs before the viewer, with its own output, to verify that the selected
entry points are observable. The viewer then runs with `trace+relay`.

Wine's [developer discussion](https://www.winehq.org/pipermail/wine-devel/2008-August/068112.html)
identifies `HKCU\Software\Wine\Debug` for relay configuration, and its
[configuration documentation discussion](https://www.winehq.org/pipermail/wine-patches/2002-December/004652.html)
describes semicolon-separated function selectors. The actual registry values
and successful positive calls are retained as evidence for this Wine version.

Despite the function list, window-procedure and DLL lifecycle messages are
also emitted. All four first logs reach their **16 MiB retention cap**, dropping
60,438,858 / 61,927,289 / 61,033,938 / 62,628,281 bytes respectively. Their page
captures remain complete, stable and unchanged, but these logs cannot support
complete-trace or API-absence conclusions. They are retained as failed logging
experiments; no screenshot is retried to hide them.

Eight new sessions use a smaller function list, add `trace+font`, and filter
window-procedure, lifecycle and unrelated font diagnostics before applying the
same retention cap. The filter retains GDI call/return lines and the selected
font-creation/glyph-outline diagnostic lines, while recording filtered bytes
and selected bytes lost to the cap separately. All eight streams reach EOF with
**zero selected bytes dropped**. Retained sizes are 5,101,070–5,414,745 bytes.
The much larger raw streams are consumed incrementally and are not archived.

## Independent logging control

The original MIT [C probe](../../scripts/research/wine_gdi_relay_probe.c) creates
a private memory DC and bitmap, selects SimSun, queries its face/glyph/font data,
draws `A`, and copies a four-color 2 × 2 bitmap at explicit coordinates. It does
not alter the viewer, register fonts or change resource files. All twelve runs
return success and log each of nine expected calls and returns exactly once.

In particular, `GetGlyphOutlineW` reports a 64-byte monochrome bitmap requirement,
`GetFontData` reports 18,214,472 bytes, and both bitmap-copy APIs report two scan
lines. Only the required size is queried; no font program or glyph bitmap is
dumped. The public [GetGlyphOutlineW definition](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-getglyphoutlinew)
explains this size-query behavior. These controls validate the observer; they
do not establish that the viewer uses the same font-drawing route.

## Bitmap boundary and remaining differences

The final `SetDIBitsToDevice` call in each complete selected trace has destination
and source origins `(0,0)`, start scan zero, `DIB_RGB_COLORS`, and the following
sizes. Its return equals the declared scan-line count. Source/PDF pairs also
have equal selected API-call counts and equal font-creation-name counts.

| View | Source and PDF final width × height | Captured page including border | Source/PDF differing pixels |
| --- | --- | --- | ---: |
| Low forward 75% | 204 × 213 | 208 × 218 | 624 |
| Low forward 150% | 408 × 427 | 412 × 432 | 1,620 |
| Low reversed/shifted 100% | 272 × 285 | 276 × 290 | 1,314 |
| High forward 600% | 512 × 497 | 516 × 502 | 2,947 |

The dimensions agree with the observed page interior after accounting for the
existing four/five-pixel border. The public
[SetDIBitsToDevice definition](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-setdibitstodevice)
describes its rectangle and scan-line parameters. **Matching arguments are not
matching bitmap contents.** This study does not dereference the passed bitmap
header/pixel pointers, inspect private objects, reconstruct every DC transform,
or examine viewer implementation code. It therefore does not prove where
inside the upstream rendering path the pixels diverge.

The useful inference is to investigate glyph/image generation and quantization
before this observed submission boundary, rather than infer a fix from absent
GDI font messages or change the already matching final rectangle. Previously
tested equivalent PDF encodings remain ineffective; the original fonts remain
the reference. Independent image-geometry controls can target the large scanned
page residuals without fitting offsets to individual actual documents.

## Reproduction and admission

The public [registry configuration](../../scripts/research/wine-gdi-relay.reg)
and C probe are byte-identical to those used in the complete selected study.
The public [stream filter](../../scripts/research/wine_gdi_relay_filter.py) differs
only in formatting: its Python AST is identical to the observed helper's.
The filter expects a writable `/output` directory inside the isolated container.
It writes `relay-filter.json` there and the selected stream to standard output.

```sh
i686-w64-mingw32-gcc -Wall -Wextra -Werror -O2 \
  scripts/research/wine_gdi_relay_probe.c -o /external/relay-probe.exe -lgdi32
# Within a fresh copy of the fixed reference Wine prefix:
wine regedit /S Z:\\external\\wine-gdi-relay.reg
WINEDEBUG=+timestamp,+pid,err+all,trace+relay \
  wine Z:\\external\\relay-probe.exe > /output/probe.txt 2> /output/probe.log
```

Launch the viewer separately with `trace+relay,trace+font`, passing its combined
output through the public filter. Keep the probe log separate. The receipt pins
the local GUI driver, input hashes and exact reference captures; numeric page
and zoom confirmation and two stable captures remain mandatory. The original
source controls are reproducible from the linked generator. Viewer, font, PDF,
screenshot and executable binaries remain external.

An original stream independently exercises the public filter: 131,089 selected
128-byte records plus two unrelated records. It retains exactly 16 MiB, reports
2,176 selected bytes dropped, excludes both unrelated records and matches the
expected output hash. This is a logging-boundary check, not rendering coverage.

All twelve GUI visits pass numeric readiness, two-frame stability, complete page
bounds, input integrity and container cleanup; no session is OOM-killed. Exact
RGB comparisons use the entire admitted page including its border, without
alignment, resizing, tolerance or normalization. An initial ad-hoc analyzer
passed a `(metadata,array)` tuple instead of the pixel array and failed before
writing comparisons; corrected analysis uses the same captures.

No new actual-corpus view, native/Node/browser parity result or production fix is
claimed. The existing 77-page cohort still differs. #541 and draft PR #542 remain
open; no merge, release, dependency, API, I/O or font-archive change is included.

<!-- SPDX-License-Identifier: MIT -->

# CAJViewer vendor fixtures

## Current status and remaining work

Use CAJViewer as a version-specific behavior reference. Reuse the pinned
container, manual capture recipe and existing comparator; no new GUI or
fixture-management framework is needed.

| Format | Selected-page result | Evidence |
| --- | --- | --- |
| CAJ | Pages 1 and 75 match | [Page-box comparison](cajviewer-page-boxes.md) |
| PDF | Pages 1 and 11 matched in the existing native pilot | #158 |
| KDH | Page 1 matches exactly | [Current checks](cajviewer-hnc8-kdh.md) |
| HN/C8 | Corrected page-frame sizes match; four selected pages still differ in pixels | [Current checks and limits](cajviewer-hnc8-kdh.md) |

CLI, Node and browser output hashes match for these actual documents, so the
same viewer comparisons apply to all three. After #184, corrected complete
HN-A and C8 output identity is verified again. #123's selected-page work is
complete with the pixel limitations below; #14 owns final release acceptance.
#124–#128 supplied the working recipe,
fixtures and comparator; #129's remaining work was consolidated into #123.
Their older future-work text is historical, not a new approval requirement.

#184 corrected the observed source page/image extents and removed displayed
storage padding. Preserve the remaining exact-pixel differences as explicit
experimental HN/C8 limitations. No additional capture campaign is required to
claim the recorded selected-page scope. Ordinary-copy text is optional;
OCR/searchable HN remains outside v0.1. Missing work is NOT_RUN.

## First experiment

1. Reuse the pinned Linux runtime; record viewer/package version, container
   digest if used, fonts, display size and zoom. Docker/Xvfb helps reproduce
   the environment. Manual GUI operation is acceptable initially.
2. Open an original PDF with visible corner/page markers, known Unicode text
   and an image-only page. Check page identity and all four edges. Then open
   one external CAJ sample and capture a complete page.
3. Prefer an available whole-page export; otherwise use a page-fit screenshot
   without application chrome. Record dimensions. An embedded image or an
   incomplete viewport is not a full-page reference. No supported Linux
   headless/export CLI has yet been established by this project.
4. Clear the clipboard and try ordinary copy on text and image-only controls.
   Save raw text or record no-text/unavailable. Do not add OCR.
5. Reopen and repeat one nonblank page capture. Report exact pixel equality or
   the observed instability. If container automation fails, record the actual
   error and try the desktop route before adding infrastructure.

## Fixtures and comparisons

Begin with 2–3 named external documents, including a multipage sample.
Use [the existing manifests](vendor-fixture-manifest.md) to record source
hash, ordered page numbers, dimensions, capture method/settings and artifact
hashes. Keep vendor binaries, external documents and derived captures outside
Git and distribution artifacts. Do not overwrite an accepted reference to
make a candidate pass; fixture updates create a new reviewed version.

Prefer source and converted PDF rendered with the same viewer settings.
A print-to-PDF plus other-renderer fallback has a separate origin and pinned
settings. Compare decoded pixels, not PNG compression bytes. Report changed
pixels and optionally a diff image. Do not crop, align or resize by content.
Exact equality is the initial automated gate; rendering noise is reported for
review, without inventing tolerance thresholds before actual observations.

Preserve raw Unicode, whitespace, order and page boundaries in copied text.
Optional CRLF/NFC diagnostics are separate from raw equality. Ordinary copy
is not proof of embedded searchable text. Text comparison applies only where
the converter promises text preservation; this plan adds neither OCR nor
searchable HN output. Unsupported/no-text observations are not text passes.

Process one page at a time with a maximum page size and bounded text/context
buffers. Use practical command timeouts, output limits and child cleanup.
Reuse normal imports and pinned checkout/container versions. Fix actual
container-layout bugs with ordinary packaging and a smoke test. A custom
verified-byte loader or execution-origin receipt system is not required.

## Comparator

The [decoded fixture comparator](vendor-fixture-diff.md) implements #128 using
existing manifest validation, exact page pixels and raw text. It runs without
the viewer and keeps unavailable text distinct from equality. Acquisition
still supplies decoded payloads and their provenance; no rendering noise is
silently accepted.

## Tests and completion

Ordinary CI uses generated original MIT fixtures, without CAJViewer or the
external corpus. Test equal content, changed edge pixels, dimensions,
missing/reordered pages, Unicode/line-order changes and corrupt/oversized
artifacts. These checks already run in ordinary CI.
Existing native/WASM/license/coverage gates remain in place; review and
simplify each PR and test changed behavior.

External runs report actual sample/page/platform coverage, viewer and fixture
versions, differences and limitations. Missing optional corpus is `NOT_RUN`;
explicitly requested missing inputs are errors. Skipped/unavailable work never
counts as compatibility success. Keep Python regressions separate. Resolve
HN-B page mapping explicitly and retain independent bookmark and Rust/WASM
memory checks in release work.

## Status and historical evidence

The table above supersedes the earlier zero-comparison status. Unchecked
pages and text remain NOT_RUN; completed selected-page checks do not establish
whole-document or all-format pixel parity.
The V14 inventory and metadata consumer completed, but do not prove capture
or conversion compatibility. The twelve historical launch observations and
failures remain in [the V14 report](cajviewer-runtime-view-v14.md) and
[startup notes](cajviewer-linux-startup.md).

[The previous plan](https://github.com/rwv/caj2pdf-rust/blob/10134d1/docs/cajviewer-fixtures.md)
and its detailed protocols remain historical references. This revised plan
supersedes their future launch-approval, inventory and source-proof planning
requirements. It does not change historical results or existing tool behavior.
#153 is closed as not planned; its unmerged bootstrap draft is not an
implemented feature. Refactor an existing harness only when needed for the
practical capture task, and document any breaking tool change normally.

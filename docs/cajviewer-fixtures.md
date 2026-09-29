<!-- SPDX-License-Identifier: MIT -->

# CAJViewer vendor fixtures

## Current plan (2026-09-29)

Use CAJViewer as a version-specific behavior reference for full-page images
and ordinary-copy text. Start with a small working experiment and reuse the
existing manifest tooling. The public language is English; all committed
code and original controls are MIT.

| Issue | Deliverable | Prerequisites |
| --- | --- | --- |
| [#124](https://github.com/rwv/caj2pdf-rust/issues/124) | A repeatable capture recipe, tested on original controls and one CAJ sample | Resolved diagnostic tasks |
| [#125](https://github.com/rwv/caj2pdf-rust/issues/125) | External manifest validation (complete) | Resolved |
| [#126](https://github.com/rwv/caj2pdf-rust/issues/126) | Small complete-page image baseline | #124, #125 |
| [#127](https://github.com/rwv/caj2pdf-rust/issues/127) | Raw copied text or documented unavailability | #124, #125 |
| [#128](https://github.com/rwv/caj2pdf-rust/issues/128) | Bounded image/text comparisons and original CI fixtures | #125 only |
| [#129](https://github.com/rwv/caj2pdf-rust/issues/129) | Representative conversion results | #124–#128, #10 and #13 for final completion |

The six tasks remain children of [#123](https://github.com/rwv/caj2pdf-rust/issues/123).
#123 remains a release prerequisite. Start #129 native pilots when fixtures
and comparison are available; finish the platform matrix after conversion
support is implemented. Text unavailability must not block the image route.

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

## Tests and completion

Ordinary CI uses generated original MIT fixtures, without CAJViewer or the
external corpus. Test equal content, changed edge pixels, dimensions,
missing/reordered pages, Unicode/line-order changes and corrupt/oversized
artifacts. Develop these checks in #128 before GUI automation is complete.
Existing native/WASM/license/coverage gates remain in place; review and
simplify each PR and test changed behavior.

External runs report actual sample/page/platform coverage, viewer and fixture
versions, differences and limitations. Missing optional corpus is `NOT_RUN`;
explicitly requested missing inputs are errors. Skipped/unavailable work never
counts as compatibility success. Keep Python regressions separate. Resolve
HN-B page mapping explicitly and retain independent bookmark and Rust/WASM
memory checks in release work.

## Status and historical evidence

Complete-page/text compatibility is still `NOT_RUN`, with zero vendor passes.
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

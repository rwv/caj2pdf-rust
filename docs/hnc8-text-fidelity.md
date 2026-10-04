<!-- SPDX-License-Identifier: MIT -->

# Native-text fidelity checkpoint (#269)

## Result and scope

The selected HN-B and C8 outputs preserve the admitted Unicode glyph sequence.
Eight manually read source regions (120 characters, with English word spacing
excluded) agree with the decoded text. This supports the measured mappings;
it does not certify every character, semantic reading order, exact whitespace,
copyable source code, or general searchable HN/C8 conversion. No new parser,
OCR, extraction API or layout reconstruction is introduced.

The baseline is main `60c0671`. The external HN-B visitor uses core at
`37e59dc`, whose core tree is identical. The actual npm tarball is
`831691cdb597367b30a0789254dbef690ef9221f214c62f437ddcf1999ed0a0c`;
its WASM is `d6a1f066d8ef7dd6efac1a9b3f759d4701d9aabbee0c543cca398f935324bd99`.
These are preflight artifacts, not a new published release.

A later [real-font HN-B checkpoint](hnb-real-font-fidelity.md) repeats all
three admitted documents with explicit substitute fonts and readable viewer
captures. Its visual limits remain separate from these Unicode checks.

## HN-A pages carry no native text

HN-A output is image-only and cannot be made searchable from the source. In
the pinned 125-page `issue-7/a.caj`, every page's text span inflates to
placement and control records only (`0x8001`, `0x8004`, `0x800a`, `0x801c`,
`0x8070`, `0x8071`); none of the 125 pages contains a glyph-code record. The
738-page catalog document `issue-111/56.caj` converts through the same record
kinds, and the [#223 study](native-text-feasibility.md) found the same on its
selected HN-A page. The text is part of the scanned images. C8 and admitted
HN-B native pages are the only sources with real glyphs.

A searchable HN-A PDF therefore needs OCR. That is out of scope here because
it would mean bundling recognition models. Run an external OCR tool on the
converted PDF instead, for example `ocrmypdf --language chi_sim+eng in.pdf out.pdf`.

## Selected documents and interfaces

| Input | Source SHA256 | Pages checked |
| --- | --- | --- |
| HN-B issue-100 | `3f3b9b57d6925df811247dced47fd7fb74cf0f678ab9bfda0827c827258ab39b` | All four for transport; page 4 for manually read regions |
| C8 issue-66 | `90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6` | All six for transport; mixed image/text page 1 and image-free native-text page 2 for manually read regions |

HN-B uses the three original geometric marker fonts from `geometry-fonts`.
C8 uses six original marker resources from `identified-family-viewer/fonts`,
including states 3/28/31, and an explicit decoration alias. Exact font hashes
are in the external resource manifest. They are diagnostic caller resources,
not original source fonts. Neither their appearance nor their bearings establish
source-font fidelity. Bookmarks are explicitly omitted in these measured runs.

The previously verified HN-B CLI/packaged Node/Worker outputs share SHA256
`697a2c4252b71e512de67d372f47c319396ec7466c711d1bf65ed3e52b5a9a41`.
A fresh C8 run through the extracted npm package in Node and a real Chromium
Worker produces six pages, SHA256
`7b48415106dcca79f53ca708498556c4845ce9a67c2f54bb28021c60b7b29170`,
identical to the recorded CLI output. Both fresh runs clear scratch and leave
no files; source requests peak at 2,453 bytes and font requests at 1,160 bytes.
The harness loads assets and hashes completed output separately; these figures
are not process-memory measurements. See #222 for measured memory acceptance.

## Character evidence

Independent MuPDF 1.25.1 text tracing reads the PDF's emitted Unicode mappings.
The HN-B source-record visitor produces 1,302 / 1,555 / 1,470 / 1,642 characters
on pages 1–4; each PDF sequence matches exactly (5,969 total). This visitor
calls the existing mode-aware mapping helper, so equality proves transport,
not independent correctness of that helper.

The six-page C8 transport checkpoint already records 1,015 / 1,338 / 840 /
986 / 801 / 1,658 characters (6,638 total), plus separate nonsemantic decoration
marks. The fresh package output has the same hash. The two additional C8
profiles retain their separate 14,300-glyph transport evidence in
[c8-native-controls.md](c8-native-controls.md).

For independent spot checks, manually read the pinned, readable source captures:
HN-B page 4's reference heading, English title and author; C8 page 1's title
fragment and section heading; C8 page 2's two Chinese code comments and one
English library phrase. All eight regions match the emitted Unicode sequence.
English comparisons exclude visible word spacing; no compatibility folding or
punctuation normalization is used. The external receipt records exact text,
lengths, screenshot identities and SHA256 hashes. This is a 120-character
spot check, not a whole-document transcription.

Chinese, Latin/digits, punctuation and special symbols also retain the original
controlled mapping evidence in [native records](c8-native-records.md),
[native controls](c8-native-controls.md) and [HN-B controls](hnb-compact-index.md).
Visual similarity alone cannot distinguish fullwidth and ASCII code points or
lookalike mathematical symbols. The existing explicit mode-specific controls
remain necessary; this report does not replace them with visual guesses.

## Copy and ordering limitations

- Source draw order is preserved, not inferred reading order. HN-B page 4's
  footer digits occur in reverse visual order in the source visitor and PDF
  draw trace. This is not a lost or reordered conversion glyph; consumers may
  reconstruct spatial order differently.
- English words can be positioned without literal space characters. Word and
  line spacing in extracted text therefore depends on the PDF consumer.
- The C8 page-2 code sample visibly uses underscore-like drawing segments.
  They are not glyph records and do not appear as underscore characters in the
  Unicode trace. Rendering those strokes does not promise copyable source code.
- Image content remains image content. The page-1 diagram is not OCRed.
- The old HN-B ordinary-copy attempt left its clipboard sentinel unchanged and
  is NOT_RUN as a text reference. One completed C8 symbol-copy control returned
  an anomalous control character. Neither result justifies replacing an
  independently established mapping. Viewer copy is not an infallible oracle.

No new converter defect is established by these checks. General reading-order,
whitespace or vector-to-character reconstruction would need a separate concrete
requirement and evidence; it is not silently added to this conversion scope.

## Reproduction and existing regressions

All source text, captures, fonts and generated PDFs remain outside Git in
`caj2pdf-hnb-rendering-20261003`. This report stores only identities and results.
New receipts are under `text-fidelity-269`: `hnb-transport.json`,
`manual-regions.json`, `resource-manifest.json`, the bounded source visitor
inputs/outputs, and `package-recheck.log`. Fresh package scripts/receipts are
`c8-six-packaged269-{node,browser}.{mjs,json}`. The visitor is an external
acceptance diagnostic that retains a page's code points for comparison; it is
not the memory contract of the production conversion API.

For independent PDF tracing, use:

```sh
mutool draw -q -F trace -o /external/evidence/trace.xml /external/output.pdf
```

Compare `page`/`g` Unicode attributes with source visitor output. Keep that
transport comparison separate from manual source checks and rendered appearance.
Original regression coverage already includes `hnc8/native/tests.rs` mapping,
truncation and short-read cases, `hnc8/native_page/tests.rs` mode-specific
Unicode/resource cases, and `pdf/document` text/font tests for ToUnicode,
unsupported glyphs, cancellation and failed sequential output. Reuse those
checks and the existing final-head quality/platform gates; a documentation-only
acceptance report does not warrant another test framework or duplicate fixtures.

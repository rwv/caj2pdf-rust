# Incremental native-text PDF output

Status: shared-core groundwork for #233. This does **not** enable a new C8
conversion profile. Required source controls, font/style meanings and six-page
real-document fidelity remain unresolved.

## Resource and page contract

1. Supply a stable `RangedSource` to `TrueTypeFont::read`. The current profile
   requires a standalone static TrueType font with `glyf`/`loca`, character
   and metric tables and a Unicode PostScript name. TTC, CFF and variable
   fonts are explicit unsupported profiles. At most 128 table entries and
   1 MiB of combined metadata are accepted. At most 16 character-map subtables
   bound work per character independently of metadata size. Outline bytes
   stay in the source.
2. Call `PdfDocument::add_font`. It copies the complete font program through
   the existing bounded resource-copy path. It emits a Type 0/CIDFontType2
   resource with explicit Unicode-to-glyph mapping, widths and ToUnicode.
   No subsetting or document-wide character collection is needed. The returned
   document-owned handle retains only object identity and an 8 KiB BMP
   character bitmap. Font-file bytes are counted alongside copied images.
3. Begin a content page with borrowed font/image handles. There are at most
   128 fonts and 8192 images per page. Await `glyph`, `image` and `segment`
   calls in source draw order, then `finish` the page. No page display list
   or completed content stream is retained in memory. Existing page-tree,
   output, cancellation and allocation limits continue to apply.

`glyph` takes a BMP character and a six-component text matrix. Its font size
is one: `[12, 0, 0, 12, x, y]` draws at 12 points with baseline `(x, y)`.
The format handler supplies positions and draw order; this API performs no
shaping or reading-order inference. Non-BMP characters, missing glyphs and
foreign handles fail explicitly. `segment` strokes a black straight segment;
zero width uses PDF's device-dependent hairline. Source-specific vector
semantics must be established before choosing these operators.

A failed or abandoned page/font emission prevents completing the PDF.
Adapters must discard partial output using their existing cleanup path. An
empty content page is possible at this low level; it is not permission for a
converter to replace unknown source content with a blank page.

Font metadata checks do not sanitize every glyph outline. The caller provides
a valid, reproducible font and keeps it stable throughout reading/embedding.
Declared embedding restrictions are checked. No system-font discovery,
proprietary font bundling or silent substitution is performed. The initial
font descriptor uses the source ascender when cap height is unavailable and
zero for unknown vertical stem width; embedded outlines determine rendering.

## Original fixture verification

Run the portable unit tests normally with Cargo. To export their original
font and PDF for independent validators, choose a scratch directory outside
the checkout:

```sh
CAJ2PDF_FONT_TEST_OUTPUT=/tmp/caj2pdf-original-font-test \
  cargo test -p caj2pdf-core --lib embedded_font_and_ordered_mixed_page_reopen
qpdf --check /tmp/caj2pdf-original-font-test/mixed.pdf
pdffonts /tmp/caj2pdf-original-font-test/mixed.pdf
pdftotext -layout /tmp/caj2pdf-original-font-test/mixed.pdf -
pdftoppm -f 1 -singlefile -r 72 -aa no -aaVector no -png \
  /tmp/caj2pdf-original-font-test/mixed.pdf /tmp/caj2pdf-original-font-test/page1
```

The expected two pages extract `A 中` and `中`. The fixture font deliberately
uses geometric glyphs: a rectangle and triangle. At 72 DPI page 1 is 120×100;
interior pixels `(11,40)` and `(38,42)` are black, `(15,45)` is red where the
later image covers the rectangle, `(25,40)` and `(31,37)` are white, and
`(80,50)` lies on the black segment. These are original controls, not CAJViewer
baselines or a claim about real C8 font fidelity. Missing independent tools
must be reported as NOT_RUN, never as successful validation.

## Node and browser Worker core-runtime check

At core commit `2940104`, an external MIT test module reused
`embedded_font_and_ordered_mixed_page_reopen` and its exported original font,
including the glyph-support, width, source-order, page-count and short-output
assertions. It compiled the actual core for `wasm32-unknown-unknown`; every
registry dependency version/checksum matched this repository's lockfile.
Node v24.13.0 and a real Chromium Worker both executed the module. Each
produced the same 142,360-byte PDF as native Rust, SHA256
`bc9bd48bd72ccce51d15c73f2f83ac643a369224d2657f5500c1f167b814a9fd`.
qpdf passed, and Poppler extracted `A 中` / `中`.

The external harness reused `js/test/browser-harness.mjs` and disposed the
Worker, browser and local HTTP server. Its WASM SHA256 is
`94cd5e4ca9a83d029cc776196a662f8a369741f51cab9ea7f73c1e1051f4dfb2`.
Sources, build lockfile, commands, outputs and `results.json` are retained
under the external `caj2pdf-font-wasm-runtime-20261001` receipt directory.
The first browser attempt failed to resolve a relative fetch URL inside a
Blob Worker; using the local server's absolute URL fixed the harness.

This is a core font/PDF runtime check, not the production JS conversion API
or native C8 document acceptance. The tiny original font is embedded in the
test module and output is collected for byte comparison. It does not measure
streaming host callbacks, peak WASM memory, OPFS cleanup or cancellation
through JS; those remain in #233/#222. No test-only production export or
additional font framework was introduced.

The same external runtime check subsequently exercised four negative paths
in both Node and the Chromium Worker: missing glyph, injected output error,
cancellation during glyph emission, and abandoning an unfinished page.
Failed draws reject page completion; all four cases reject document
completion and emit no final `%%EOF` marker. Resetting the temporary failure
or cancellation flag does not revive a failed page. Sink requests remain
at most 31 bytes. The normal fixture remains byte-identical to native.
The expanded harness WASM SHA256 is
`84102a509b835409d10b62f4216af40bf6c2a1ca021af965cee1f01c791c847b`;
it supersedes the initial positive-only harness for subsequent reruns.
These are core cancellation/output invariants executed under WASM, not
JavaScript adapter cleanup or host-driven asynchronous cancellation tests.

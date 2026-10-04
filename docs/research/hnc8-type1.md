<!-- SPDX-License-Identifier: MIT -->

# HN/C8 type-1 JPEG support (#224)

The type-1 descriptor in the pinned issue-43 HN-A document contains ordinary
baseline JFIF JPEG data. Its six observed images are 8-bit, three-component
frames on pages 1, 16, 30, 64, 86 and 94. Their dimensions are respectively
1563×2174, 1363×939, 1626×298, 665×740, 1782×1045 and 1806×1059.
All six exact indexed payloads decoded with libjpeg-turbo without warnings.
This observation is specific to the admitted JPEG profile, not every possible
type-1 payload.

HN-A/C8 composition now routes types 1 and 2 through the same existing bounded
JPEG preflight and byte-preserving DCT emission. Baseline SOF0, exact marker
framing/EOI, grayscale or verified three-component JFIF interpretation, source
identity, work/allocation limits and cancellation checks are unchanged. Invalid
JPEG, progressive/arithmetic/CMYK and uncertain color profiles still fail.
HN-B composition remains restricted to its separately measured type-2 profile.
Historical `read_type2_jpeg_info` / `convert_type2_image_pdf` diagnostic names
are retained; these APIs now accept type 1 as well as type 2. No JPEG decoder,
whole-image buffer, new dependency or source-specific dispatch is added.

## Container compatibility selection

The issue-43 document also contains the previously measured `0xa40c` JBIG2
text-header anomaly on page 11 image 1. CLI and WASM HN/C8 adapters explicitly
select `HnC8UnusedRefinementTemplate` after entering the HN/C8 conversion path.
The existing composer verifies container/segment profiles and retains the typed
anomaly marker per image. See [the compatibility note](t88-text-header-compatibility.md).
General JBIG2 APIs and default low-level composition options remain strict;
other malformed flag combinations do not become accepted. This is a documented
container interoperability choice, not a filename or source-hash exception.

## Verification

The complete 109-page issue-43 input has SHA-256
`826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134`.
Native output passes qpdf without warnings and has SHA-256
`e334c2be876c21237a9e6aa4095ad228d8093948a320e9e8b50b210adef087a4`.
Independent source-descriptor/Poppler checks pass on all 109 pages and 114
images: exact JPEG payloads, plus decoded type-0/type-3 pixels against the
existing pinned oracles (including the separately recorded anomalous image).
The 42 output outlines also match independently read source titles, depths
and destinations. The initial strict-policy failure left no final output.
These checks establish page/image identity and order, not complete CAJViewer
page-pixel parity or physical coordinate units.

Original synthetic tests exercise mixed type-1/type-2 placement, duplicate
images, asymmetric grayscale/color pixels, malformed JPEG refusal before image
output, short reads, resource limits and cancellation. Node and real Chromium
Worker tests use a generated 16×8 left-dark/right-light JPEG, three-byte I/O,
real OPFS scratch in the browser, qpdf validation and independent `pdfimages` /
`djpeg` pixel checking. All external documents, extracted images, PDFs, raw logs
and copied text remain outside Git. Unavailable optional corpus is NOT_RUN.

## Public-interface full-document repeat

[Measured metadata and hashes](../../tests/conformance/type1_current.json) record
native, Node and Chromium Worker conversion of that same 109-page source.
All three produced the identical 10,690,852-byte PDF above, including 42
bookmarks. Native completed in 70.604 s, Node in 375.622 s and Chromium in
339.724 s on this host; the two JS runs overlapped, so these are observations,
not comparative performance benchmarks. Both JS runs used built-in codec states
and four 64 MiB-capped scratch stores, all empty after conversion. The Node
scratch helper removed its temporary files, and browser OPFS was empty after
source/output/scratch disposal.

Post-conversion WASM linear memory was 1,900,544 bytes in Node (64 KiB I/O)
and 1,769,472 bytes in Chromium (4 KiB I/O). These are linear-memory sizes,
not process peaks or total browser memory. Browser input was streamed into
OPFS and output was sequential; a whole-output allocation used solely for
SHA-256 verification occurred after conversion. The test server's source
buffer is also outside the converter memory measurement.

The prior strict-policy failure is retained externally. This repeat does not
replace the frozen v0.3.1 baseline or claim source-versus-viewer pixel parity.

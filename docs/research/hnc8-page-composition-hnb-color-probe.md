<!-- SPDX-License-Identifier: MIT -->

# Issue #117: HN-B JPEG color interpretation investigation

Status: FROZEN BEFORE PRIVATE OBSERVATION.
The original external probe was independently reviewed and its eleven
source-only synthetic tests passed before this status was frozen. Source:
`/home/hzc/.cache/issue117-hnb-color-probe.py`, 61899 bytes, SHA-256
`2263e604ac1e7e46af005912f6a674a2d2c70826fd17b4192df2fe06d51f6236`.
Commit this plan and bind its resulting hash before execution; no private
observations have occurred under this protocol while drafting or reviewing it.
This is the predeclared investigation for [child #122](https://github.com/rwv/caj2pdf-rust/issues/122),
which blocks the remaining HN-B complete-page criterion in [parent #117](https://github.com/rwv/caj2pdf-rust/issues/117).

## Preserved failure and hypotheses

The identity-parameter comparison remains FAIL at source commit `8041ee4`.
Its external report is
`/home/hzc/.cache/caj2pdf-issue117-validation-identity-params/composition-report.json`,
1696715 bytes, SHA-256
`104cd225c58876e38a1d5a04d58de164d42a049d3a823632cadea4b7ac939291`.
Its internal receipt is 27498 bytes, SHA-256
`e4410e0652e0eec077a379788d953c69ebc01540e47dfaf7d730ae60cb5de9b8`.
Preserve both earlier complete-comparison failures, the identity-parameter
dictionary probe, every artifact and each attempt's separate counts.

The controlled comparison completed three native conversions and zero Python
conversions. All 81 source rows, 77 page boxes, 127 ordered draws, 53 unchanged
JPEG streams and 74 complete Type0 arrays passed their declared comparisons.
HN-A/C8's 75 complete pages passed 150 renderer pairs. HN-B output page 1
failed its first MuPDF pair: 261584 of 316863 pixels differed, maximum channel
difference 255, mean absolute channel difference 169.6357542534155. Three
remaining HN-B renderer pairs were not attempted. The strict pixel gate stays
zero; none of these observations establish HN-B pixel compatibility.

Recorded metadata declares DeviceRGB for both reference HN-B JPEG XObjects
and DeviceGray for both native XObjects, with identical compressed streams,
dimensions, boxes and ordered transforms. The largest HN-B transform/box
residual is 5.684341886080802e-14 points. The earlier metadata gate did not
compare JPEG color dictionaries; its passing scope excludes this difference.

Original Rust source maps a one-component SOF frame to Gray, then JpegGray8
and DeviceGray. The competing hypotheses are a wrong native component/color
classification, an incompatible reference RGB declaration on grayscale DCT
data, or other reference/native Decode, DecodeParms or mask semantics. This
plan decides none of those hypotheses in advance.

The committed [type-2 inventory](../../tests/conformance/hnc8_type2_jpeg_inventory.tsv)
already labels these two payloads one-component/JFIF without APP14. These are
prior clues, not new independent header observations. The
[selected-image diagnostic](hnc8-type2-pdf.md) previously checked all 1,085
source JPEGs and this same HN-B grayscale canary against direct `djpeg` and
MuPDF, with pointwise agreement. Its Poppler evidence uses a separately
disclosed weaker local-sampling rule. That evidence concerns individual
selected images; it does not establish complete original-page parity.

## Frozen inputs and read scopes

Reference PDF:
`/tmp/caj2pdf-layout-reference-final/hnc8-layout-run-mi3kj_io/hn_b-run1-komdoko5/output.pdf`,
826645 bytes, SHA-256
`b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51`.

Native PDF:
`/home/hzc/.cache/caj2pdf-issue117-validation-identity-params/hnc8-composition-wo0rprwb/hn_b.pdf`,
826945 bytes, SHA-256
`785fb0fd76bc9f53eecdc9ded9c7a4af505221c498c8b38beea5c8e26ebf9a77`.

Original source:
`/tmp/caj2pdf-ranged-corpus/issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj`,
862235 bytes, SHA-256
`e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49`.
Source/image spans below come only from previously recorded public/report
metadata; no document bytes were examined while drafting this plan.

| Output/source page | Image | Descriptor offset | JPEG span | Dimensions | Encoded SHA-256 |
| --- | ---: | ---: | --- | --- | --- |
| 1/1 | 1 | 6408 | 6420 + 34458 | 2071 × 153 | `557242be2e183a9c278b48d6088d868c76493c1d63601b941c1e893836af477b` |
| 2/6 | 1 | 71094 | 71106 + 790923 | 2222 × 3276 | `4b489fe0981a1af9f629f31c58a091060b4c11803c3edbd768690bf08dfa2cf3` |

The unchanged native executable is
`/home/hzc/.cache/caj2pdf-issue117-build/release/examples/hnc8_page_composition`,
940504 bytes, SHA-256
`997d95046715942d025abcaf9845233648c5c65bbc9031b96fc2ff9c60ec2884`.
The Rust/Cargo fingerprint remains 139 files, SHA-256
`2b899b0a68e2ba3919914189b1483668fc7004fd7dfac4c47d30d6f9c6b189d1`.
Neither executable nor Rust changes, and neither is invoked by this probe.

## Four dictionary-only queries

Use `/usr/bin/qpdf`, SHA-256
`30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792`.
Execute exactly once each, in this order:

1. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=7 REFERENCE`
2. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=3 NATIVE`
3. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=9 REFERENCE`
4. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=10 NATIVE`

Require actual qpdf version-2 root/parameters framing, exactly the selected
entry and a stream object containing only `dict`. Refuse `data`, `datafile`,
nonselected entries and excessive structural depth/length. Type is optional;
if present it must be XObject. Subtype, dimensions and eight-bit depth must
match the recorded image identity. Object IDs are diagnostic inputs only.

Summarize bounded key names and numeric, boolean, name or reference fields
for Type, Subtype, Width, Height, BitsPerComponent, ColorSpace, Decode,
DecodeParms, ImageMask, Mask, SMask, Interpolate, Intent, Filter and Length. Hash/length
encoded strings; report unknown fields by name/type. Keep raw JSON external.
Page 2 corroborates the first observation; its ColorSpace metadata was already
seen, so it is not independent statistical evidence.

## Two bounded marker-only observations

After the complete source hash passes, parse only the two pinned JPEG headers
above, in source order. Traverse at most 65536 bytes from each span start to
the end of its first SOS header. Every exact field/header read is at most
4096 bytes and must stay inside the selected span. Use checked offsets to
skip opaque APP/table/comment payloads. Never prefetch across the first SOS
header into entropy; never decode an entropy symbol, sample or pixel.

Report SOI/SOF/SOS framing and offsets, precision, dimensions, component
count/IDs/sampling/quantization selectors, bounded SOS selector fields, JFIF
presence/version/density/thumbnail dimensions and Adobe APP14 presence/
version/flags/transform. APP14 presence or absence is scoped to the observed
SOI-through-first-SOS prefix; later markers are not scanned. Unknown APP
identifiers remain hash/length only.
Once the first SOS boundary is known, hash its complete preceding header
with 4096-byte reads; expose only offset/length/digest and I/O counts.
No DQT/DHT payload values or arbitrary strings enter the report.
Use an unbuffered FileIO source so exact logical field reads cannot cause a
Python BufferedReader to prefetch entropy. The parser recognizes bounded
SOF0/SOF1/SOF2 framing with one through four distinct components, valid
sampling/selectors and matching distinct SOS identities; other frame markers
fail this observation profile. This is not full JPEG/entropy validation.
Application byte counters exclude kernel/cache read-ahead. The opaque audits
may use ordinary buffered files with separately declared 65536-byte
application requests.

The complete source digest before and after pins all previously measured
selected-span bytes. The marker parser stops at the first SOS and performs
no entropy parsing, decoding or extraction. Separately metered 65536-byte
opaque full-file provenance hashes read every source/PDF byte, including
encoded entropy; they do not interpret it. Selected payload SHA values are
inherited pins and are not independently recomputed as spans. Marker and
opaque full-file requested/read bytes have distinct report counters. A source
mutation, malformed/truncated header, over-limit traversal or unsupported
framing is a failure, never a successful compatibility observation.

## Original public synthetic controls before private work

Generate asymmetric grayscale and RGB JPEGs at runtime with cjpeg. On the
same grayscale JPEG stream and transform, create an original valid DeviceGray
wrapper and an intentionally incompatible DeviceRGB wrapper. Query original
qpdf dictionaries and compare complete MuPDF and Poppler renders using the
same frozen 300-ppi RGB/no-antialias settings as #117. Record tool agreement,
warnings and full pixel identities; do not assume decoder independence.
The three-component control uses a correctly declared DeviceRGB wrapper and
explicit DCT ColorTransform 1. Record actual public cjpeg/dictionary/render
launch counts separately from the later eight private-phase tool attempts.
Tool agreement means both detect the mismatched declaration; raw cross-tool
pixel equality is neither expected nor used as a compatibility gate.
Check grayscale equal RGB channels, component counts, safe JSON framing,
optional Type and opaque-string redaction. Original header-only tests must
cover caps/truncation, malformed frames and a reader that refuses every
request at or beyond first entropy. All fixture files are runtime-only,
external and cleaned after tests; no private observation occurs in tests.

## Limits, immutable receipt and final audits

- Four dictionary-tool attempts and two marker observations, no retries.
  Up to four separately counted startup probes: ldd on qpdf/Python before
  and after. Zero native or Python conversions, decoded stream extractions,
  samples, private renders or input mutations.
- Each child: 45 seconds, 1 GiB virtual memory, stdout 65536 bytes, stderr
  32768 bytes, read requests at most 65536 bytes. Whole probe including final
  audits: 120 seconds. Kill the whole child group on failure; record actual
  wait4 RSS or explicit unavailability separately from sampled RSS.
- Fresh mode-0700 directory under
  `/home/hzc/.cache/caj2pdf-issue117-hnb-color-dictionary-probe`, owned files
  at most 1 MiB. Reserve raw JSON and final-report bytes before writes.
- Freeze reviewed source, committed plan, owned controller/scaffolding,
  original three protocol files, native/Rust, Python/qpdf/ldd, exact argv,
  source/span/PDF/report pins and effective environment in an immutable
  receipt before private audits. First complete the two startup-before
  queries and freeze their canonical library identities and attempts in that
  receipt. The failed report and its original immutable receipt, both earlier
  failure/probe metadata inputs, and all three original protocol files remain
  fixed audited inputs. The failed report has an explicit 2 MiB
  metadata cap and 65536-byte requests; parse the same bytes whose hash was
  checked; validate its source mapping, spans, object IDs and inherited stream
  lengths/hashes. Provenance full-file hash requests are separately at most
  65536, including source/PDF/report, Rust and canonical-library audits.
- Re-audit every declared source/PDF/report/code/plan/tool file in finally,
  including unsuccessful/unfinished attempts. Recheck effective environment,
  canonical startup-library identities and immutable receipt. A changed
  identity or exceeded whole deadline prevents PASS. Keep honest attempted,
  completed, passing, failing and skipped counts for each observation kind.
  Missing or unsupported work never becomes a compatibility pass.

## Interpretation and remaining acceptance

The original marker reader derives functional frame/scan fields from
[T.81 Annex B.2.2/B.2.3](https://www.w3.org/Graphics/JPEG/itu-t81.pdf), printed
pages 35–37 (zero-based PDF pages 38–40): Nf counts frame component specifications and
the frame/scan lengths delimit those specifications. No decoder source,
tables, figures, examples or arithmetic-coding annex data are copied.

[Adobe PDF Reference 1.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
§4.8.2 (printed page 264/zero-based PDF page 283) derives the image's component count
from ColorSpace. §3.3.7/Table 3.11 (printed page 60/zero-based PDF page 79) obtains the
DCT component count from encoded data and ignores ColorTransform for one or
two components. §4.8.4 (printed page 267/zero-based PDF page 286) identifies inconsistent
image/color-space entries as errors. Primary semantics, observed declarations and source
component facts must drive any next action. Do not misdeclare native
grayscale as RGB merely to match a potentially incompatible reference.

This probe establishes framing/metadata only. It cannot repair a reference,
choose production encoding, compare private pixels or close #117. If an
independent grayscale-reference blocker or intentional reference deviation
is needed, record it explicitly. Any corrected reference/control or new
private parity run needs a separate reviewed plan and receipt; preserve both
complete-comparison FAIL reports and all strict zero-difference gates.
HN-B full-page acceptance remains unmet; HN-A/C8 passes cannot replace it.

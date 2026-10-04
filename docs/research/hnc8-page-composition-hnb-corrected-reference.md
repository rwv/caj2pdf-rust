<!-- SPDX-License-Identifier: MIT -->

# Issues #117/#122: independent HN-B grayscale reference

Status: FROZEN BEFORE PRIVATE EXECUTION.

The reviewed original MIT external probe is
`/home/hzc/.cache/issue117-hnb-corrected-reference.py`, 77797 bytes, SHA-256
`59e174ac567be52401b1877068b1a2c7ec5b3edccb20e5a6afebe9fc06e56273`.
Root and the independent reviewer approved correctness, provenance and
simplification at that exact source and the pre-freeze draft SHA-256
`107959d1695c8ba3c380e33d4925fc958b25e7e730a5a94ee0e8929a0ab7d21d`.
Both independently passed all twelve original-only tests (root: 1.889 s;
reviewer: 1.923 s), with 138 public fixture launches and zero private, native
or converter calls. No-input NOT_RUN and zero work were also confirmed.
This final update changes only status and the source/review record; the
reviewed executable source remains byte-identical. Private execution requires
the committed plan and separately frozen runtime receipt.

This two-page phase addresses [#122](https://github.com/rwv/caj2pdf-rust/issues/122),
which blocks the HN-B pixel criterion of [#117](https://github.com/rwv/caj2pdf-rust/issues/117).
It reuses the unchanged native PDF and launches **zero converters**. Both
historical complete-comparison FAIL results remain unchanged. Passing this
phase means agreement with an explicitly corrected grayscale reference,
never exact legacy-reference compatibility or whole-family conversion.

## Observed basis and fixed inputs

The [framing-only plan](hnc8-page-composition-hnb-color-probe.md), SHA-256
`a26fb4e926e974942dc4c5df4d9fef7ed1d6f767a2e4924d7876fae6fc4f08ef`,
was committed before four dictionary queries and two bounded header reads.
Its PASS report is
`/home/hzc/.cache/caj2pdf-issue117-hnb-color-dictionary-probe/hnb-color-uma4ilxs/probe-report.json`, 74626 bytes,
SHA-256 `ebc1fe08e2121e5322ad2d29e3bfd821070e25580b666e0bd860be7ff401bb49`.
Its immutable receipt is 33225 bytes, SHA-256
`f8b9c097524c54874401195d4d0a29c350362cab2258c2e23e4ecd9d83c78e7d`.
Both source JPEGs have one eight-bit SOF0 component, JFIF, and a 649-byte
first-SOS prefix. APP14 was absent in those prefixes. Reference dictionaries
declare DeviceRGB, native dictionaries DeviceGray; neither has Decode,
DecodeParms or image masks. This is framing evidence, not pixel parity.

Retain and audit the failed comparison report
`/home/hzc/.cache/caj2pdf-issue117-validation-identity-params/composition-report.json`,
1696715 bytes, SHA-256
`104cd225c58876e38a1d5a04d58de164d42a049d3a823632cadea4b7ac939291`.
Its immutable receipt is 27498 bytes, SHA-256
`e4410e0652e0eec077a379788d953c69ebc01540e47dfaf7d730ae60cb5de9b8`.
The original three protocol documents and earlier failure/dictionary reports
remain separately pinned and audited; this plan replaces none of them.

| Input | Bytes | SHA-256 |
| --- | ---: | --- |
| HN-B source | 862235 | `e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49` |
| Retained legacy reference PDF | 826645 | `b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51` |
| Retained native PDF | 826945 | `785fb0fd76bc9f53eecdc9ded9c7a4af505221c498c8b38beea5c8e26ebf9a77` |
| Uninvoked native executable | 940504 | `997d95046715942d025abcaf9845233648c5c65bbc9031b96fc2ff9c60ec2884` |

Use exactly the source/PDF/native paths from the frozen framing plan. The
Rust/Cargo fingerprint stays 139 files, SHA-256
`2b899b0a68e2ba3919914189b1483668fc7004fd7dfac4c47d30d6f9c6b189d1`.
The independently reviewed main controller at commit
`35a7f9c584a4f0c068baa0e6520b7fa8d3ba3c4a` has SHA-256
`b02e7b73c0e4fdfc4b4f4aa679288d7f83bf5d4919de89264d27a3add03392f9`. The
unchanged owned framing helper is 61899 bytes, SHA-256
`2263e604ac1e7e46af005912f6a674a2d2c70826fd17b4192df2fe06d51f6236`.

| Output/source page | Legacy/native image object | JPEG offset + length | Dimensions | Encoded SHA-256 | Frozen CTM |
| --- | --- | --- | --- | --- | --- |
| 1/1 | 7/3 | 6420 + 34458 | 2071 × 153 | `557242be2e183a9c278b48d6088d868c76493c1d63601b941c1e893836af477b` | `[497.04,0,0,-36.72,0,36.72]` |
| 2/6 | 9/10 | 71106 + 790923 | 2222 × 3276 | `4b489fe0981a1af9f629f31c58a091060b4c11803c3edbd768690bf08dfa2cf3` | `[533.28,0,0,-786.24,0,786.24]` |

The preserved native source rows must remain `[1,2,3,4,5,6]`, output mapping
`[1,6]`, and explicit no-image rows `[2,3,4,5]`. Source page 6 is required;
first-page success cannot stand in for it.

## Sole semantic correction and preservation proof

1. Query complete legacy and native object metadata using
   `qpdf --json --json-key=qpdf --json-stream-data=none PDF`. Capture at most
   1 MiB per document, 64 generation-zero objects, 16 streams and 8192 JSON
   nodes. Reject encryption, object/xref streams, dangling references,
   duplicate keys, nonfinite values and excessive depth. Require canonical
   positive generation-zero references, rejecting other reference-shaped strings.
   Parse real numbers with standard-library Decimal to preserve exact values;
   scalar equality distinguishes booleans, integers and reals. Keep raw JSON external.
2. Build an original JSON update containing **only legacy image objects 7
   and 9**, complete original dictionaries, and no stream data/datafile.
   Change only `/ColorSpace /DeviceRGB` to `/DeviceGray` on those two images.
   All dimensions, DCT filters and other entries stay identical. Store/hash
   this exact update and freeze its identity before the sole write command.
   Make the update and both input inventories read-only, then independently
   rehash those generated inputs and the derived receipt immediately before use.
3. Once, invoke
   `qpdf --preserve-unreferenced --remove-unreferenced-resources=no --stream-data=preserve --object-streams=disable --normalize-content=n --static-id --update-from-json=UPDATE LEGACY CORRECTED`.
   The retained legacy PDF is never modified. The static ID is test-only;
   this corrected oracle is an external validation artifact, not a release.
4. Reopen the corrected PDF with the same bounded JSON query and `qpdf --check`.
   Require equal PDF version/object counts, a bounded paired graph bijection
   across original/corrected objects, and exact recursive equality after
   remapping references and applying precisely the two ColorSpace changes.
   Seed the paired walk from trailer Root/Info references, compare every
   dictionary/array/scalar, map references one-to-one, and follow each paired
   object once so cycles terminate. Every object participates. A reference-free
   unreferenced value can be proven only by exactly one identical unmatched
   counterpart; ambiguous or unreferenced reference/stream graphs fail. Resolve
   stream Length entries against opaque raw-stream lengths; direct/indirect
   Length representation may differ only if the exact length agrees.
5. Hash **every** original/corrected stream using bounded
   `qpdf --show-object=ID --raw-stream-data PDF` output. Every complete stream
   SHA-256 and length must agree; JPEG streams also match both inherited
   source-span pins. Thus page/content streams, filters and arbitrary metadata
   cannot change silently. No raw stream is retained by the preservation
   comparison. Extra/deleted objects, altered masks/Decode/DecodeParms,
   changed content, geometry or any unrelated semantic mutation fail.

[qpdf 12.2 JSON input](https://qpdf.readthedocs.io/en/12.2/json.html) documents
updates that omit stream data and retain existing bytes. Its writer can
renumber objects. The only permitted serialization bookkeeping is equivalent
object-number references, cross-reference offsets/serialization, recomputed
trailer Size, and the fixed test ID. Other trailer entries compare exactly.
The fixed ID is established by original runtime controls before private work.
The exact pinned qpdf 12.2 whole-document command above includes stream Length
entries on the original runtime fixtures; Length acceptance is specific to
that observed tool/command profile. Direct and indirect Length controls remain
required, without a general claim about every qpdf JSON mode.

## Independent samples, source mapping and complete pages

Independently reopen corrected/native PDFs with the existing owned metadata
helper, using only qpdf/mutool/pdfimages. Check two pages, two ordered JPEG
draws, eight-bit DeviceGray, exact DCT streams/dimensions and all six CTMs
against the unchanged source/native/public records. Box/CTM tolerance stays
0.00005 pt. An in-memory corrected comparison basis changes only those two
ColorSpace facts; the public legacy oracle remains unchanged. Record the
legacy ColorSpace mismatch as a rejected compatibility control, not a pass.

For each source JPEG, spool precisely its checked span through 4096-byte
requests, independently recompute its exact encoded SHA/length, and run
`djpeg -grayscale -pnm -outfile SOURCE.pgm SOURCE.jpg`. Use
`pdfimages -f PAGE -l PAGE PDF PREFIX` on both corrected/native PDFs. Require
exact width/height, eight-bit grayscale, complete top-row-first sample-array
SHA/length and pointwise equality against direct source JPEG decoding:
**two independent source decodes and four complete sample comparisons**.
Direct output must be P5 Gray8. Poppler may expand Gray samples to P6 RGB;
accept that representation only if all three channels are exactly equal at
every pixel, then compare the identical Gray value. This is a lossless
representation check over every byte, with no tolerance or color conversion.
Every sample participates; require nonblank/variable source content. Sample
extraction is explicitly authorized by this new phase, unlike the framing probe.
These comparisons run independently from Rust. `djpeg` and `pdfimages` may
share the installed libjpeg backend; two command names do not establish
decoder-implementation independence. Record canonical libraries and disclose
any shared backend. Within-renderer page comparisons remain separate.

For pages 1 and 2, render corrected/native independently with each renderer:

- `mutool draw -q -r 300 -A 0 -c rgb -F pnm -o - PDF PAGE`
- `pdftoppm -r 300 -singlefile -aa no -aaVector no -f PAGE -l PAGE PDF`

There are exactly eight renderer launches and four within-renderer page
comparisons. Check every RGB channel on the complete page, exact raster
dimensions, whole-file SHA/length, payload SHA and pointwise differences.
Both changed-pixel/channel counts, maximum/absolute/mean differences must be
zero; require nonwhite content. No interpolation envelope, cropping, interior
sampling, error tolerance or cross-renderer equality substitutes for this gate.

## Limits, original controls, receipts and counts

- Fresh 0700 session below `/home/hzc/.cache/caj2pdf-issue117-hnb-corrected-reference`;
  never Git or `/tmp`. Owned disk cap 128 MiB, individual raster/file cap
  64 MiB, PDF cap 2 MiB, source JPEG cap 1 MiB, JSON cap 1 MiB and preserved
  failed-report cap 2 MiB. Monitor file/session caps for the entire child
  lifetime and reserve program-written files before writes. Keep a 1 GiB
  free-space reserve. Delete successful sample/raster pairs immediately;
  retain the corrected PDF, safe report/receipts/JSON and first failed pair.
- At most 256 total tool launches, renders included; zero native/Python
  converter launches. Exactly one qpdf update, two source decodes and eight
  renders in a successful run. At most twelve separately counted startup
  queries: ldd for qpdf/Python/mutool/pdfimages/pdftoppm/djpeg before/after.
  Record exact argv, attempts, output sizes/digests, errors and nullable
  wait4/sample RSS. No retry after a failed phase.
- Child limit 90 s, 1 GiB virtual memory and 512 MiB RSS; whole phase including
  final audits 600 s. Child text stdout cap 1 MiB, stderr 32768 bytes, raster
  stdout cap 64 MiB. Hash/comparison I/O requests are at most 65536 bytes;
  source-span spooling requests at most 4096. Retained buffers are bounded;
  interpreter/validator RSS is measured separately.
- Bind reviewed source/plan/controller/helpers, unchanged native/Rust,
  public oracle/catalog, every preserved report/receipt/source/PDF, tool
  executables and canonical startup libraries, parent/effective environment,
  resolved argv/paths and all command templates in an immutable receipt
  **before any private audit or observation**. Freeze the derived update
  receipt before rewriting. Audit every input/code/tool/receipt in finally,
  including failed/interrupted attempts; a mutation or final deadline failure
  prevents PASS. Record opaque provenance hashes separately from spool,
  sample and pixel I/O. No-input invocation is NOT_RUN with zero work.
  Every generated inventory/update is also audited in finally against the
  exact bytes frozen before use, alongside the receipts and corrected PDF.
- Original runtime asymmetric grayscale JPEGs and a two-page nine-object
  PDF prove qpdf's exact-update/bijection/stream contract with actual tools.
  Include an unreferenced object and refusal controls for extra dictionary,
  content-stream, graph, trailer and Length mutations. Compare direct djpeg,
  pdfimages and complete MuPDF/Poppler pages on correct Gray wrappers; show
  the original RGB wrapper is incompatible. All original fixture bytes stay
  in temporary external files and are cleaned. Mandatory tools missing is
  a failure. Public fixture launches are distinct from private evidence.
- Every required group reports attempted/passing/failing/skipped/unsupported
  counts and retained progress on first failure. Unsupported is a subset of
  failure, never a compatibility pass. Dynamic preservation object/stream
  totals become planned only after their bounded inventory succeeds.

The original source-only suite has twelve tests. Its actual full-run fixture
uses 36 preparation launches and 73 runner launches, including twelve startup
queries and eight renders; the separate exact-update fixture uses 29 launches,
including ten renders. Thus the controls execute 138 public fixture launches
and zero private/native/converter calls. They prove four full sample pairs and
four complete page pairs in each applicable fixture, with exact preservation
of nine objects/four streams. Pure rejection controls cover precise real values,
boolean/integer/real distinctions, reference grammar/dangling references,
duplicate keys, mutated generated inputs, refused launch counts, located legacy
ColorSpace errors and truthful first-failure preservation/audit/I/O counts.

## Interpretation boundary

The [Adobe PDF 1.4 reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
§4.8.2 infers sample component count from ColorSpace; §3.3.7 obtains DCT
components from encoded data and ignores ColorTransform for one/two components;
§4.8.4 identifies inconsistent image/color-space entries as errors.
The independently observed single component supports DeviceGray. This phase
tests that precise interpretation; it does not alter the native converter or
construct an RGB misdeclaration to fit the legacy output. No converter/decoder
implementation, differently licensed code, normative tables or private bytes
are copied into Git. The probe and invented fixtures are original MIT code.

Any unexpected dictionary/object graph or failed sample/page check preserves
FAIL and blocks #122/#117 until a separately reviewed amendment. This phase
cannot replace HN-A/C8 evidence, unblock table rights, add type 3, or enable
released CLI/JS conversion. Any successful evidence is labeled corrected
reference agreement with an explicit intentional legacy deviation.

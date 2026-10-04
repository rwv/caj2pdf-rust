<!-- SPDX-License-Identifier: MIT -->

# Issue #117: explicit identity-parameter comparison amendment

Status: FROZEN BEFORE THIS CONTROLLED RERUN, 2026-09-28 UTC. This amendment
follows the separately preserved first attempt and dictionary-only probe.
No further source/PDF/sample observation, converter call or render has been
performed while drafting this amendment. Original runtime synthetic PDFs
are permitted to test the verifier before this plan is executed.

## Preserved observations

- First complete comparison report: 733091 bytes, SHA-256
  `7e9d0e6d43d1f0f3e428da636f82566f7fcac44e4868347093edd652735816e5`,
  external `/home/hzc/.cache/caj2pdf-issue117-validation/composition-report.json`.
  It remains **FAIL**: one native HN-A output completed; metadata gates passed
  for 68 source/output pages, 91 draws and 24 JPEG streams; the first full
  Type0 comparison failed at the reference dictionary guard. Two Poppler
  page-image extractions had run; no complete array or page-pixel comparison
  passed. C8 and HN-B were not attempted. All before/after integrity groups
  passed. Source commit: `42125b6b2a98169083868bfa19f2595f12eb311f`.
- Dictionary-only probe report: 21470 bytes, SHA-256
  `ec6a38adda35e156ad5b67857471414a1208953c9aa8c7d3d50e0675ca513fa4`,
  external `/home/hzc/.cache/caj2pdf-issue117-dictionary-probe/dictionary-srrp1_nc/probe-report.json`.
  Receipt: 4544 bytes, SHA-256
  `3431ff99f9bd3fcaaf1b4f88fad602377e6566126ebffd44d62e0c9f391a4a48`.
  The probe passed its framing checks with exactly two object queries and
  four startup-identity probes, zero conversion/sample/render work and all
  before/after identities equal. Probe source SHA-256:
  `9c55bde8bbbbe84e9805bae7018bf8cfea0839673ed844c29d99ba5c1c33b621`;
  committed plan SHA-256:
  `abb348a40495a5ff12e1f38f8dc7f568f3596dc49f9041c70e313a40398f08fb`.

The reference page-2/image-1 dictionary declares `/FlateDecode` and a direct
`/DecodeParms` containing exactly `/BitsPerComponent 1`, `/Colors 1`,
`/Columns 2304` (its declared sample width) and `/Predictor 1`. Its only key
from the old broad refusal guard is `/DecodeParms`; `/Mask`, `/SMask` and
`/ImageMask` are absent. The native object declares unfiltered one-bit
`/DeviceGray` and `/Decode [1 0]`, without DecodeParms or masks. These are
dictionary observations, not image or page compatibility passes.

## Narrow verifier correction

The [Adobe PDF Reference 1.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf),
Tables 3.7 and 3.8 (printed pages 49–51), identifies Predictor 1 as no
prediction. This parameter leaves the filtered sample sequence unchanged.
[qpdf 12.2 inspection documentation](https://qpdf.readthedocs.io/en/12.2/cli.html#option-filtered-stream-data)
specifies filtered stream output and errors on unsupported decoding filters.

Accept only the observed direct identity-parameter profile: FlateDecode,
exactly four unique integer fields, BitsPerComponent=1, Colors=1,
Columns=the checked display width, Predictor=1. Keep the existing measured
one-bit palette and explicit inverse-gray interpretation checks. Refuse
additional/duplicate/nested/indirect parameters, parameter arrays, predictors
other than 1, incorrect widths/components/depths, masks and unproven decode
or color dictionaries. Match `/Decode` as a complete name, so `/DecodeParms`
cannot be mistaken for the separate Decode key.

Prove this correction first on original asymmetric runtime PDFs. Independently
extract every bit with qpdf and Poppler, including all padded columns; compare
against the complete invented array. Reversed rows and a changed padding bit
must fail. These synthetic checks use original data and inspect no converter
implementation. They are verifier tests, not private compatibility passes.

Add explicit typed unsupported-verifier counters for requested profiles and
comparison groups. Unsupported required work remains a failure and prevents
PASS; successful final evidence requires zero unsupported required work.
No error-message substring is used to silently classify an operation as
successful. Clean-clone no-input output remains NOT_RUN with every actual
counter zero.

## Controlled rerun and limits

All other inputs, caller table, reference PDFs, source-derived geometry,
native options, renderer commands, sample normalization rules and resource
ceilings are those of the [original frozen protocol](hnc8-page-composition-protocol.md),
whose first-attempt SHA-256 is
`6b423115b903521ffc7d99f2ce592c45e1933649e941afd485fe6a51aa292c93`.
The Rust/Cargo fingerprint and native executable must equal the first attempt;
this correction changes only the original development verifier, tests and
English evidence notes.

Run the full three-profile protocol once after review and source freeze:
maximum three new native calls, zero new Python converter calls, 74 required
complete Type0 arrays, 53 JPEG streams, 81 source rows, 77 output pages,
127 ordered draws, 154 full-page/renderer comparisons and 308 render launches.
This is a separately declared revalidation after a known verifier refusal,
not a retry inside the failed attempt. Preserve the original actual one
native call and all failed/remaining counts; cumulative native calls across
the two comparison attempts can therefore reach four. Each attempt retains
its separate receipt, limits, commands and counts.

Use a fresh mode-0700 session beneath
`/home/hzc/.cache/caj2pdf-issue117-validation-identity-params`. Bind both
committed protocol files, final reviewed helper/tests/runner fingerprints,
source commit, unchanged native executable/Rust fingerprint, effective
environment, commands and startup-library identities before private work.
Include the original failed report and dictionary probe report as audited
metadata inputs. Re-audit all declared files, 27 corpus sources, six reference
PDFs, table, tools, environment and code in finally. Keep the 1800-second
whole deadline, 2048 aggregate tool ceiling, per-child limits and 512 MiB
owned-session disk ceiling from the original protocol. Preserve honest
failed/unsupported/remaining counters and first-failure artifacts.

Every padded bit and every complete-page RGB channel still requires exact
equality. Boxes/all six CTM components retain absolute 0.00005 pt tolerance.
No padding normalization, row/shift fitting, crop, renderer-setting change,
additional private document mutation or tolerance expansion is authorized.
Any new mismatch stops its profile and remains FAIL; another follow-up must
have its own independently reviewed predeclared plan. The dictionary probe
and prior metadata passes do not satisfy #117's remaining sample/page gates.

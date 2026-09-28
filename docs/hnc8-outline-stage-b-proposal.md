<!-- SPDX-License-Identifier: MIT -->

# Issue #137: Stage B outline validation proposal

Status: **DRAFT — STAGE B NOT_RUN; EXECUTION DISABLED.**

This design addresses the evidence prerequisite in [#137](https://github.com/rwv/caj2pdf-rust/issues/137),
which blocks native outline field rules and parity under #119. Its public source
basis is `ecc4d160b045599b8410819449ccf9d3f9f4af43`, byte-equivalent to the reviewed
`4293480` change for the files discussed below. The
[closed Stage A report](hnc8-outline-stage-a-results.md) remains historical
discovery, with compatibility **UNVERIFIED**. This proposal supplies no candidate
field rule, execution contract, token, new observation, or native implementation.

## 1. Inputs and unknowns

Only the following two held-out identities are proposed for semantic validation.
Their sizes and counts are inherited public declarations, not fresh audits or
positive baseline expectations. Other inventory entries remain OUT_OF_SCOPE,
NOT_RUN, and zero semantic passes. The later frozen plan must enumerate its entire
identity-audit set separately; semantic exclusion never waives a declared audit.

| Public label | Inherited SHA-256 | Bytes | Historical pages / records |
| --- | --- | ---: | ---: |
| issue-29 | `ede5eddb0e8ec1dea46c32a06e16ac12b874a141ca2d6669736495d2ac549261` | 10,155,625 | 48 / 48 |
| issue-69@57a3c60e1d86 | `57a3c60e1d8639955c625452398d2c8a32a46a51b815a44cc9a5e0351fcb8ef4` | 6,685,012 | 81 / 111 |

An externally supplied, independently reviewed candidate grammar must be frozen
before either held-out source is observed. It must declare:

- Title offset, encoded width, termination/padding, empty-title behavior, strict
  codec/version/provider, decoded byte ceiling, and two-byte/four-byte/non-BMP
  predictions. GB18030 correlation on issue-21 supplies none of these rules.
- Numeric offsets, widths, byte order, signedness and valid ranges; root origin,
  preorder/parent rules, count/order filtering, and exact invalid-case behavior.
- Physical source-page numbering, supported applicability, omitted-target policy
  within that profile, and the predicted destination kind/nullable parameters.
- Framing/checksum algorithm and covered spans, ownership of maintenance bytes,
  unchanged bytes, and a finite method to distinguish remaining equivalent rules.

Missing or ambiguous fields stay UNKNOWN. A fingerprint or filename may bind an
evidence input; it must never select a runtime parsing rule. C8/HN-B applicability
and omitted-row semantics remain unknown and outside this HN-A validation scope.
Original structural fixtures may later establish an implementation policy without
claiming HN-B format evidence.

## 2. Two separate freezes

**Baseline freeze.** An original MIT adapter and its original controls require
root and independent correctness/simplification review. Freeze exact candidate
grammar, schema, source/code/runtime pins, supported profile, predicted baseline
counts, commands, repetitions, exclusive output names and every ceiling. This
proposal recommends two unmodified conversion repetitions per held-out source:
four converter calls and eight outline-parser queries. These are planned counts;
actual Stage B counts remain zero. Exact converter/runtime identities, required
positive outline counts, predicted maps/map-verification methods, verification
calls and ceilings are currently unresolved, so even this first invocation cannot
be authorized yet. Actual generated output maps are future baseline results.

After an explicitly authorized future invocation, close and review its complete
baseline artifacts. Generated PDF hashes cannot be known in advance: freeze their
names, producer commands, required outcomes and size caps beforehand; create them
exclusively, then hash/seal them as results. Do not represent an unknown result hash
as an input pin. A missing, zero-entry, ambiguous, unsupported or failed required
baseline blocks controls. No replacement source, retry, grammar change or expanded
probe follows automatically.

**Control freeze.** Only after those closed baselines pass independent review may
a new plan bind their immutable results and enumerate exact controls. Each row
specifies source/repetition, record ordinal, field span, original/replacement byte
identities, complete bounded diff, checksum updates, predicted complete outline
effect, expected success/failure, commands, outputs and verification calls. Seal
all predictions before any mutation or control conversion. Changed hypotheses,
controls, pins or limits require a new independently reviewed freeze; retain the
failed or superseded attempt. Baseline approval supplies no control authorization.

For each invocation use an acyclic binding chain: committed frozen protocol and
reviewed original source -> external input/command manifest -> wrapper pinned to
that manifest -> separate binding/review record -> same-process effective-runtime
receipt. No file embeds its own hash. Review the actual environment receipt before
an explicit single-use authorization; an old receipt or elapsed approval wait is
insufficient. The manifest binds adapter/helper source, not the derived wrapper or
binding record; the separate binding pins that source, the manifest and wrapper
without pinning itself. A DRAFT, CLOSED/consumed, incomplete or unknown-schema plan
refuses before any private audit/read. This document deliberately has no executable
`execution-contract` block.

## 3. Bounded baseline and exact comparison

During a future authorized phase, hold no-follow regular input descriptors, verify
exact size/hash and stable inode/path identities, then range-read the HN-A marker,
count and checked intervals. The existing interval evidence is count at `0x158`,
records from `0x15c` in 308-byte windows, and index start `0x15c + 308*N` with
`20*P` bytes. Recheck multiplication/addition, containment and frozen caps before
reading. These framing facts do not establish the record contents.

Apply only the pre-frozen grammar, one record/title at a time, producing a capped
external prediction ledger before the converter sees that source. Store exact
UTF-8 identities without trimming, replacement characters or Unicode normalization.
Keep the preorder ancestor state bounded by declared depth; keep the separately
bounded physical-source/output map. Do not retain all decoded source titles or
pages. Use the frozen predicted map for advance source predictions, then compare
the independently measured actual map; never revise predictions to fit output.
PDF query JSON is a separate explicitly capped parser input.

For every required PDF use the existing independent qpdf outline JSON and MuPDF
`show -g` pages/root/selected-object path. The second query follows the capped qpdf
object set but independently validates links, titles, destination object references
and emitted page indices; disclose that selection dependency. Both parsers must
agree on the whole ordered list, not merely a count or matching subset. Refuse
cycles, parent gaps, omitted selected objects, indirect/unsupported title schemas,
unknown view forms, warnings, short reads, or cap exhaustion. Display-outline URIs
are not an exact oracle. New required schemas need reviewed original controls and
a new source/protocol pin, never a permissive fallback during observation.

Each entry uses the existing canonical schema: ordinal, zero-based depth, exact
UTF-8 title SHA-256/byte length, resolved zero-based PDF page index, destination
kind and nullable parameters. Hash the complete ordered canonical list, compare
entries as well as its hash, and require repetitions and source predictions to
agree. Canonicalization removes PDF object-number differences; it preserves title
whitespace and Unicode, destination kind, finite numeric value and every null.
In particular `/XYZ [null,null,null]` must remain XYZ with three nulls: no `/Fit`
substitution, null-to-zero conversion, or inference of source-view fields.

Resolve the actual source-to-output map under a separately frozen page-order/draw/
image-identity verification method and budget. Equal source/PDF counts alone do
not prove an identity map. Repeated/ambiguous page identities are blockers unless
the frozen independent linkage resolves them. Explicitly label physical source
pages as one-based and PDF indices as zero-based; resolve targets through the
checked map. The historical HN-B `[1,6] -> [0,1]` map grants no neighbor retargeting
for omitted `[2,3,4,5]`. Parser agreement on a PDF target alone does not establish
which source field or physical page produced it.

## 4. Controls and unchanged-data proof

The later control freeze must instantiate a finite list covering ASCII title,
claimed two-byte and four-byte/non-BMP title behavior, a distinct already-emitted
physical-page destination, and a legal hierarchy change. Each positive control
predicts the full resulting list, including descendants and unchanged entries.
Select distinguishing numeric controls before execution; indistinguishable
width/endian/signedness hypotheses remain ambiguous or narrow the supported rule.
Separately enumerate empty title, malformed/incomplete codec sequence, invalid
hierarchy and out-of-range destination controls with advance failure/deviation
criteria. A converter failure alone is not a successful negative compatibility
test. View controls need an independently identified applicable field.

Copy an original source sequentially into an exclusive external file, modifying
only the declared field and separately declared checksum-maintenance spans. Stream
an exact original/copy comparison: verify every unchanged span and record the
complete difference intervals, hashes, lengths and retained external bytes. A
checksum algorithm, its covered bytes and valid framing must be established
before mutation; arbitrary donor prefixes or damaged-container controls are
refused. Original sources stay read-only and are re-audited at closure.

For successful controls independently verify baseline page count/order, complete
page-stream/draw identity and pixels under the exact frozen verifier commands and
caps. Outline object/numbering differences must not mask changed page content.
Use bounded stream hashing and sequential/row or temporary-file pixel comparison;
freeze renderer/version, dimensions, color/sample convention and exact comparison
rule. Any tolerance must be separately justified before freezing. No full decoded
page list or unbounded extraction is allowed. Verification methods, object/page
sets, repetitions and output caps are currently unknown and block control freeze.

## 5. Resource calculation and closing audits

Stage A's 1 GiB opaque / 1 MiB logical / 16 MiB session / 256 MiB harness / 600 s /
24 children plus runner ceilings are immutable history, not Stage B defaults.
Fill the following ledger from frozen declarations before reviewing each phase:

| Ceiling | Required calculation |
| --- | --- |
| Children | Baseline: `4 conversions + 8 outline queries + B + F + V`; controls: `sum_j(r_j * (1 + 2 + v_j)) + B + F`. `B/F` are exact opening/closing runtime calls; `V/v_j` cover map/stream/pixel and any other verifier calls. Adapt explicitly for expected no-PDF negatives. Add one runner per invocation. |
| Opaque requested reads | `sum_f(a_f * (S_f + 1)) + H + E`: each complete exact-read hash pass charges size plus its EOF request; `H` includes all separately enumerated manifest/code/runtime/generated/query/result rehash passes. `E` reserves requested bytes for frozen failed/short-read handling, never an unbounded retry. Use approved maximum sizes for not-yet-generated files. |
| Logical requested reads | Sum every declared header/index/record, field/checksum scan and comparison request, including duplicate reads and each reserved failed/short-read allowance. A semantic checksum scan is not an opaque identity audit. |
| Disk/spool | Immutable retained copies + all generated PDF/query/diff/prediction/report caps + maximum live scratch/extraction slots + closing/failure reserves. Preassign file counts and exclusive names, reserve before creation, and enforce child per-file/aggregate limits. |
| Memory/work/time | Explicit input/record/page/depth/title/offset/argv/JSON caps; parent, each child and concurrent-process limits; allocation/buffer/spool capacities; requested work; per-child/group timeout plus opening/work/closing/persistence deadlines. |

For the two inherited sources alone, their total is **16,840,637 bytes**; two
complete before/after hash passes with one EOF byte per file request
**33,681,278 bytes** assuming exact reads. If inherited `P/N` counts are confirmed
and the existing 8-byte signature + 348-byte prefix reads are retained, one header/index/record
pass requests `sum(356 + 20*P + 308*N) = 52,264` logical bytes. These are partial
declaration-only calculations, not measured totals or a sufficient phase ceiling.
Code/runtime files, checksum spans, copies, output PDFs, verification data, failure
allowances and persistence are still missing. No numeric Stage B cap is adopted.

Charge requested bytes and attempts before each operation, including failed
launches, partial/EOF reads and failed output creation. Actual returned bytes,
per-child RSS, parent VmHWM and final retained bytes are separate measurements.
Adapter read counters do not measure a black-box child's internal I/O; explicitly
state their scope and freeze separate child file/output/time/resource enforcement.
Run children serially, drain capped stdout/stderr, kill/reap the entire process
group on failure/cancellation, and reserve closing slots/bytes/time before work.
Do not conceal descendant residency behind parent RSS or monitored disk peaks
behind a claim of hard write limits.

Before private work, seal and review actual interpreter/tool/codec-provider/library
pins, commands, loaded original modules, effective environment (including scratch),
inputs and immutable receipts. Before and after, audit every declared code/runtime/
environment/source input. Seal generated artifacts when created and verify them
again at closure, including on earlier failure. Audit failures cannot be hidden by
a prior exception. Persistence/sealing/deadline failure prevents overall PASS;
retain bounded sanitized FAIL metadata with honest remaining work.

Preserve the first Stage A FAIL/cause UNKNOWN and its closed CLI SHA
`e8660ffd526b06ddecf30c59e242d88ec5002f3fd8350e748815ffef221212f9`, plus the
closed selected-three identities in the public report. Cumulative historical
Stage A controlled process launches remain **13 + 25 = 38**. Report Stage B by
phase and cumulatively as `38 + every attempted Stage B child + every Stage B
runner`; keep public preparation launches separate. Classify converter, parser/
validator, native, render, vendor and viewer calls explicitly without double
counting the total. Required progress uses planned, attempted, completed, failed,
unsupported and not-attempted counts; a failed attempt is never a semantic pass.

## 6. Minimal later implementation and completion boundary

Add one original MIT evidence orchestrator, proposed
`scripts/hnc8_outline_validation.py`, and its original synthetic controls only
after this design is reviewed. Reuse the existing bounded JSON/PDF metadata and
no-follow identity primitives; give Stage B its own strict plan schema/controller
and limits rather than invoking the Stage A `run` contract or changing its frozen
constants. Factor a small helper only if differing limits require it. The candidate
grammar is validated declarative data, never imported executable converter code.
The externally retained predictions/diffs/receipts contain private bytes; only
reviewed identities, numeric facts and original examples may enter Git.

Current public integration facts constrain the eventual #119 design:

- `hnc8.rs:88,130,533` exposes header/index metadata and a record-count budget;
  it does not implement outline fields or retain an outline span in `Header`.
- `operations.rs:54,64` owns one `Bookmark` title and visits asynchronously.
  `pdf/document.rs:595,938` emits preorder with O(depth) active titles/links and
  memory preflight; `:1096` currently emits `/Fit`, so full XYZ parity needs a
  separately reviewed extension.
- `hnc8/compose.rs:96,808,828` emits borrowed map events with **one-based**
  output page numbers and finishes immediately after pages. Future integration
  must retain a bounded explicit map, subtract one only for an emitted page,
  and stream outlines before `finish`; `None` stays an omitted row.

Before native work, freeze exact encoded/decoded-title, record/depth/page/map,
allocation/offset/work limits and located cancellation/failure policy. One record
plus O(depth) ancestors and a separately bounded source/output map suffice for
preorder delivery; no all-title or decoded-page buffer is needed. No native visitor,
title decoder, HN/JBIG migration or PDF API change is implemented by this draft.

#137 can close only after both positive held-out baselines, distinguishable rules
within the declared supported profile, every frozen control/unchanged-data check,
and all closing audits/accounting pass independent review. Otherwise retain a
precise blocker. C8/HN-B unknowns remain explicit; full #119 native tests/emission,
CLI/JS behavior, hosted/MIT checks, exact per-file 100% Rust LCOV and parent/release
parity remain subsequent gates. This DRAFT satisfies none of those execution gates.

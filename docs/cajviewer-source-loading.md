# Safe public-module load diagnostics

This is original MIT, public-only preparation for
[#143](https://github.com/rwv/caj2pdf-rust/issues/143), a prerequisite of
[#124](https://github.com/rwv/caj2pdf-rust/issues/124). It adds bounded source
accounting and original controls. It does not run CAJViewer, Docker, X11,
an inventory, a converter or a document experiment.

## Preserved first failure

The first frozen P2 inventory phase remains **FAIL**. Its sealed receipt is
61,409 bytes, SHA-256
`7470c1f6c1c9177752dd7798efc8c1ae960b092c8f660ae83c1761533b8ed756`.
The complete 1,192-byte inline FAIL envelope records `FileNotFoundError`
without a missing path or load stage. The specific cause remains UNKNOWN;
the old Dockerfile or inventory does not independently identify the missing
file. The root closing record is 2,685 bytes, SHA-256
`88f765f3607f7c5884a04c102d25bceda739897d2727e5b5d6280fe345c684a5`.
Independent closing review confirmed the failure accounting, not compatibility.

There were five Docker client attempts, one admitted inventory attempt, zero
nested metadata actions, zero completed inventories and zero membership
comparisons; all 2,731 comparisons remain unverified. The independent cap,
environment and user closing observations, owned-container removal and final
absence passed, as did 153 public identity audits and three retained-output
audits. All frozen V4/V5 code, plans, receipts and the first FAIL remain
unchanged. The twelve previous launcher outcomes also remain unchanged.

## Two whitelisted original modules

The loader in [tools/cajviewer/run.py](../tools/cajviewer/run.py) accepts only
these ordered module names, roles and public paths:

| Ordinal | Module | Role | Path |
| --- | --- | --- | --- |
| 1 | `cajviewer_canary` | `process-helper` | `/opt/canary/cajviewer_canary.py` |
| 2 | `inventory` | `runtime-inventory` | `/opt/canary/inventory.py` |

The default whole-byte pins are the original module versions declared by the
consumed P2 plan and historical baseline: 10,992 bytes / `ba9abc3be6285cfb7d0af3840b88197fd5aa4ec22d7277ecee109ea0eae60c77`
and 2,656 bytes / `cb1cec2a74be10413ce628cf5320180c1d2d950212f59b5750bbad434a24d556`.
A separately frozen phase must audit its exact declared pins. Original controls
may supply invented bytes and their identities at those same two public paths;
the function does not accept another role, module name or path.

For each module, the loader appends a record before reading and marks each of
`read`, `pin`, `compile` and `exec` PENDING before the operation. Later stages
and the second module stay NOT_RUN until their prerequisites complete. The
first failure is fatal and re-raises the original exception, including
cancellation. It does not retry, bless changed pins or continue the chain.

The reader supplied by the frozen adapter must use a bounded regular-file read
through EOF, or raise. Its application read requests are at most 65,536 bytes;
the returned source is also capped at 65,536 bytes. No-follow/nonblocking
regular-file checks belong to that reader and the immutable read-only image
profile. A partial or raised read produces no actual whole-byte identity. A
complete read records its byte count and SHA-256 before the independent pin
comparison. These identities describe the bytes read, not a prefix, parser
interpretation or vendor result. The reader's existing request/return accounting
and every container/host closing audit remain separate.

Each record permits only the ordinal, fixed role/path, expected and actual
whole-byte identities, four stage states, status, failure stage, fixed reason
and a whitelisted error type. Unclassified types become `OTHER_ERROR_TYPE`.
Exception text, traceback, arbitrary paths, source bytes, title text and
environment values never enter source-load records. There are at most two
records. A failed read clears any incomplete read-stage identity.

## Independent host interpretation

`validate_source_loads` checks exact keys and scalar types, ordinal/path/order,
identity bounds and hashes, the stage dependency chain and fixed failure
classification. Boolean/float integers, extra fields and malformed hashes are
refused. A non-null actual identity with read PENDING/FAIL is contradictory;
pin PASS must match the expected identity. Module two cannot have a record
after module one FAIL/PENDING.

`source_loading_observation` processes only a complete, already bounded and
duplicate-safe metadata envelope. A complete valid FAIL may retain known
attempts and failures **before** the outer nonzero exit is refused. A complete
valid FAIL may also retain two loaded modules when a later inventory operation
failed. These are source-load observations, not completed inventory or vendor
passes. PASS is contradictory unless both modules complete all four stages.

TIMEOUT/OUTPUT_LIMIT captures are prefixes. They, missing data and malformed
records remain UNKNOWN with no successful source count. A valid PENDING record
can retain an attempt with zero completions and zero failures; its unobserved
stages and unstarted modules remain explicit. An empty valid FAIL ledger means
zero module attempts. None of these states can satisfy required phase success.

The observation helper validates source accounting only. The host still
requires the original child exit, stream completeness, empty stderr, ordered
nested actions, exact environment/user/cap facts, membership comparison,
owned-output integrity and final public audits. Every unclassified fault is
FAIL. Source failures do not bypass independent closing observations or change
cleanup ownership.

## Exact fragments and bounded deployment

The COMMON, LOADER and VALIDATOR blocks in the host runner have explicit byte
markers. `public_source_fragment` extracts only a complete bounded, separately
whole-file-pinned source. Missing, duplicate, reordered or oversized framing is
refused. It does not read, import or execute the source.

The proposed external inline entry embeds COMMON + LOADER byte-for-byte;
the proposed external host embeds COMMON + VALIDATOR byte-for-byte. The exact
whole-file and fragment identities must be frozen and audited before use.
There is **no third installed module**, image change, vendor copy or library
dependency. The proposed image and two module pins remain unchanged. Their
declarations alone do not prove installation or loading; the later v11
observation below records the first complete load and the second failed read.

At the #143 implementation checkpoint, the fresh v11 proposal was
DRAFT/REFUSED. The separately frozen phase is now closed FAIL, as recorded in
[the v11 runtime-view report](cajviewer-runtime-view-v11.md). It completed the
first source load and failed the second source's read. This diagnostic code
alone grants no execution; any later phase needs its own reviewed immutable
plan, actual same-PID preflight receipt and fresh exact token. The consumed
phase retained one inventory maximum, with the same
five client maximum, 5/10/90/15/5-second child limits, 512 MiB container limit,
CPU 2, PID 64, 8 MiB tmpfs, offline/read-only/non-root profile, 16 KiB inline
entry, 256 KiB host receipt and all existing closing/persistence guards.
No diagnostic preparation grants the two future launcher attempts or raises
the cumulative launcher maximum of fourteen.

## Original controls and remaining gates

[test_cajviewer_source_loading.py](../tests/conformance/test_cajviewer_source_loading.py)
is mandatory original-only discovery input. It invents module bytes and
receipts at runtime, executes the unchanged source fragments with private
module registries, and independently mutates type/order/path/identity/stage
records. It covers missing first/second sources, raised partial reads, pin,
compile and exec failures, cancellation, fixed error classification, valid
PENDING versus impossible PASS, captured-prefix UNKNOWN and fragment framing.
A process guard must remain uncalled. No proprietary implementation or corpus
bytes are fixtures. The nineteen methods formed part of the reviewed original
run below; no installed-runtime observation is inferred from them.

## Reviewed original-control evidence

Root invoked the separately frozen original-control plan once, using isolated
Python (`-I -B`), and it exited zero. All 25 fixed methods passed: nineteen
mandatory loader/validator methods, four controls of the actual inline closing
body and two controls of the actual host retention/refusal body. Ten mocked
helper callbacks occurred; forbidden process attempts and actual child, Docker,
ENV/tool probe, runtime inventory, application, vendor, private-input, native
and converter counts were each zero. These are public source/fault controls,
not vendor compatibility results or proof that either file exists in the image.

The complete nonzero FAIL control retained exact source stages before the real
`required-docker-helper` refusal. The TIMEOUT-prefix control retained UNKNOWN
before `incomplete-inventory-envelope`; neither completed an inventory or any
of the 2,731 membership comparisons. The inline controls retained first/second
source failures, known loads before a later invented failure, and the primary
source failure alongside independent closing-cap failure. A cleanup OSError
or interruption is caught by the actual finally helper: it keeps known method
records/counts and the first reason, sets FAIL with fixture state
UNKNOWN_OR_PARTIAL, then continues final accounting and publication.

The immutable external `control-report.json` is 7,112 bytes, SHA-256
`f91f1d1930fe511d903e208006d14fd4216cac4f1cd0f89eae26090bcd016fde`.
Its canonical/provisional files are mode0400 and share the same owned inode.
The consumed `module-load-controls-plan-frozen-v3.json` is 9,388 bytes,
SHA-256 `6bbe1d82ec5678fa44b5f93a409417a81c61563edaf38902b53fc8a3c1bbe178`.
The coordinator is 28,006 bytes, SHA-256
`89bfe4eb2414aa12bb5d1270197d689a1b42bd3481bad16a57c2fd39f2054a62`.
All 24 before/after source audit rows and the separate closing plan identity
passed, and invented fixtures were removed before publication. Root's closing
proof is 5,409 bytes, SHA-256
`6ed81d7b927813c3e605b4735f3cf37e9ad60cb5505459ae4df6aa0aa4c518a3`;
independent closing review agreed. Eight historical artifacts retained their
exact byte identities, modes and inodes. Old code/plans/first failure were not
rewritten or retried.

The report's pre-persistence application meter records 210 read calls,
12,678,534 requested and 863,202 returned bytes, plus 32 writes with
45,399 requested/returned bytes. Its virtual meter records 172 reads,
1,785,018 requested and 88,754 returned bytes. These cover explicit
coordinator/harness file operations and declared virtual fixture streams.
Interpreter/stdlib import I/O, filesystem metadata, kernel cache readahead
and RSS are not measured. The named pre-persistence snapshot excludes final
report writing; the returned CLI metadata separately records I/O after that
publication. The 60-second cooperative deadline covers final publication
refusal checks, without claiming a kernel-I/O watchdog or a process-memory cap.
The recorded pre-persistence elapsed time is 0.084079828 seconds, not the
whole interpreter lifetime or final elapsed time.

## Remaining gates

Root and independent source/provenance/simplification and actual-control
closing reviews passed. [PR #145](https://github.com/rwv/caj2pdf-rust/pull/145)
merged after its exact-head native/WASM/MIT/coverage gates passed; #143 is
closed. The actual Rust LCOV DA recount covered 54 unique files and all
27,413 deduplicated executable lines, with 100% total and per-file coverage;
raw LF/LH were separately 28,272/28,205. This describes the Rust files in that
report, not Python, JavaScript or whole-repository coverage. Optional external
corpus tests remained NOT_RUN and contributed zero compatibility passes.
Normal native/WASM CI is separate from the zero runtime-experiment/conversion
counts above. Parent #124 capability, complete-page image, fresh standard-copy
text, print and OCR criteria remain unmet or NOT_RUN. The separately consumed
v11 phase failed and its closing reviews passed; its source-load diagnostics
do not complete an inventory. Any later diagnostic phase must preserve both
v10/v11 failures and receive its own exact frozen profile and reviews.

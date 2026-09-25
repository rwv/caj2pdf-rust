# T.82 arithmetic core

This is the implementation note for [issue #26](https://github.com/rwv/caj2pdf-rust/issues/26).
It describes the original, bounded Rust decoder for the standard arithmetic
stripe coded data (SCD) algorithm. It does **not** specify a CAJ-family image
decoder. The separate [HN/C8 row-model candidate](jbig1-row-model.md)
matches the pinned type-0 images in local external-corpus tests but is not a
released decoder.
The [Table 24 provenance decision, issue #30](https://github.com/rwv/caj2pdf-rust/issues/30),
blocks completion of #26 and the integrated decoder in #28 while unresolved.

## Standard and source boundary

Use the English **ITU-T Recommendation T.82 (03/1993)**, which is published as
identical to ISO/IEC 11544:1993. The locally checked official PDF has SHA-256
`6d4280f4402ce285199b3835dda54e35372e8378e7352d2e88ab3ac420f46942`.
The relevant clauses are §6.2.5 (stripe boundaries, stuffing, and reset
markers), §6.8.2.3/Table 24 (113 probability states), §6.8.3.1–§6.8.3.9
(arithmetic decoding), and §7.1/Table 26 (the 256-event conformance trace).
This design targets that 1993 edition; any later corrigendum must be reviewed
separately before claiming its support. The ITU component list also shows
Technical Corrigendum 1 (03/1995) and Technical Corrigendum 2 (03/2001);
their effect on Table 24 remains to be checked under #30.

The implementation may use the standard's behavior and separately measured
input/output facts. Its Rust code, comments, tests, and synthetic fixtures must
be independently authored. Do not read or translate legacy Python, Go,
private Rust, or differently licensed decoder implementations. The official
Table 24 rows and §7.1 vector are **not** project-owned source or committed
fixtures. See [provenance](provenance.md) for the unresolved redistribution
question.

## Core contract

The arithmetic core receives one **already delimited, unstuffed SCD span**
and a caller-supplied context index for each requested symbol. A separate
container layer finds stripe boundaries, removes standard stuffing where
applicable, and decides which symbols require arithmetic decoding. The core
emits one decoded bit at a time; it neither constructs whole pages nor decides
HN/C8 pixel order.

Represent each QM state with `qe`, `next_lps`, `next_mps`, and `switch_mps`.
Borrow a validated table of exactly 113 states for the lifetime of a decoder;
do not embed or generate normative values in the crate. `QmTable::new` rejects
the wrong row count, zero or out-of-range `qe`, and out-of-range transition
indices. The external fixture parser rejects malformed values, including
numeric switch values other than 0 or 1.
Represent each context separately as its current state index and MPS bit.
The caller chooses a bounded context count; no symbol may reference an absent
context. Table and context values must be validated before use, so malformed
input produces a typed error rather than an index panic.

The decoder stores small arithmetic registers and an internal input buffer
of at most 256 bytes; it borrows the configured context bank. Its snapshot
distinguishes coded bytes consumed from bytes actually returned by source
reads, including bounded prefetch, and from virtual zero byte-input events.
It reads its
span through the project's `RangedSource` contract, handles short reads, and
requests no byte beyond the declared end. It must never read an entire CAJ
document or image into memory.
For `N` contexts and input chunk size `B`, working memory is `O(N + B)` and is
independent of the decoded page area; an eventual row assembler may retain a
bounded number of rows separately. Use checked `u64` offset and length
arithmetic. `ArithmeticError` records the next absolute source byte offset
and context when available; the read-only snapshot exposes symbol progress.
The offset is in the supplied `RangedSource` coordinate space. If that source
contains unstuffed or spooled SCD, it is not automatically an offset in the
original CAJ document. The #28 container adapter must retain any mapping
needed to report the original document location during row integration.

The standard's `BYTEIN` rule (§6.8.3.8) supplies virtual `0x00` bytes **after
the delimited SCD span is exhausted**. This is distinct from a physical source
EOF or I/O failure *inside* the declared span, which is a truncated-input or
I/O error. The three initialization reads (§6.8.3.9) follow the same rule.
Virtual padding must not silently redefine an arbitrary file truncation as a
valid shorter stripe. The caller supplies the expected symbol count and
verified stripe boundary; if those are unknown, the result is not a validated
T.82 stripe. Track real and virtual reads separately for diagnostics.

Each decoder has configured symbol and total-work limits, plus cancellation
checks at regular symbol/I/O boundaries. Real reads are bounded by the
declared SCD span; virtual padding reads consume the work budget. Exhausting
a limit reports its resource, limit, and attempted value, plus source offset
and context when available. These limits prevent a malformed span followed
by virtual zeros from causing unbounded work. Numeric shifts and conversions
must use explicit widths and checked operations.

`INITDEC` starts each stripe's arithmetic registers afresh. On the first
stripe for a bit-plane/resolution layer, or after a forced reset, initialize
all of that layer's context probability states and MPS bits to zero. Otherwise,
carry **only the appropriate previous stripe's context probability states**
into the next stripe of the same bit-plane and resolution layer (§6.2.5 and
§6.8.3.9). The API should
make `Reset` versus `Carry` explicit and reject `Carry` without prior state.
Other image-model state, such as adaptive template position, typical
prediction, and prior rows, belongs to the later image decoder and must obey
its own stripe rules. Do not infer those rules from an arithmetic-core pass.

## Verification plan

Commit only original tests and small synthetic data. A deliberately invented
113-row table can exercise register initialization, MPS/LPS exchanges,
renormalization, state switching, context reset/carry, and overflow/error
paths without claiming standard conformance. Test short ranged reads,
physical EOF inside the declared span, virtual padding after it, missing
bytes, invalid context/state/table indices, every numeric conversion,
cancellation, and each work limit.
A fixed-budget mutation smoke test and a CI-safe regression subset should use
the same bounded API. Native tests and `wasm32-unknown-unknown` compilation
must not need FFI, an external executable, or the official table.

The normative check is an **opt-in local test**, separate from clean-clone CI:

1. Obtain the English 03/1993 PDF from the [official ITU page](https://www.itu.int/rec/T-REC-T.82)
   and verify its SHA-256 above. Keep the PDF and any extraction under `/tmp`.
2. Supply a local line fixture, currently
   `/tmp/caj26-official-vector-with-checkpoints.txt` (SHA-256
   `11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`).
   Line 1 is `T82-1993`, line 2 is `113`, the next 113 lines each contain
   decimal `qe next_lps next_mps switch` fields in Table 24 order, then a
   line containing `3` and three decimal `symbols_decoded A C CT` lines
   transcribed from Table 26. The final three lines are hexadecimal SCD,
   context bits, and expected bits (§7.1), representing 25, 32, and 32
   bytes. The checkpoint counts 0, 2, and 7 mean before Table 26 events
   001, 003, and 008 respectively; `A` is the common column, while `C` and
   `CT` are decoder columns. All fixture values stay outside Git, packages,
   generated source, and WASM bundles.
3. The ignored [integration test](../crates/caj2pdf-core/tests/qm_official_external.rs)
   reads the fixture through `CAJ2PDF_T82_VECTOR_FILE`, reads at most 16 KiB
   plus one sentinel byte, rejects any fixture above 16 KiB, and verifies
   the pinned SHA-256 above **before parsing**. It then checks the fixture's
   edition and structure, decodes exactly 256 symbols, compares each bit,
   and compares the three `A`, `C`, and `CT` register checkpoints **before**
   the specified events. A missing fixture is an ignored `NOT_RUN` in an
   ordinary test run, never a `PASS`; requesting the test with a missing,
   oversized, changed, or malformed fixture fails. Run it explicitly from
   the repository root:

   ```sh
   CAJ2PDF_T82_VECTOR_FILE=/tmp/caj26-official-vector-with-checkpoints.txt \
     cargo test -p caj2pdf-core --test qm_official_external --locked -- --ignored --nocapture
   ```

   Record the fixture/PDF hashes and checkpoint event numbers in the local
   report. This command requires separately supplied official data and is
   not a clean-clone CI check. The 2026-09-24 local run passed all 256
   symbols and three register checkpoints; it does not resolve #30 or prove
   CAJ compatibility.
4. After standard conformance, compare CAJ output separately against the
   [#22 pinned pixel oracle](jbig1-oracle.md), with explicit HN and C8
   nonblank canaries. A standard-vector `PASS` with a CAJ hash mismatch
   refutes the particular CAJ framing/context hypothesis under test; it does
   not establish CAJ decoding support.

The existing independent `/tmp` prototype reproduced the §7.1 256-bit output
and tested a finite set of HN/C8 framing and context guesses. Those early
guesses did **not** match the nonblank canaries; blank matches were not
diagnostic. The [investigation](jbig1-bitstream-investigation.md) records the
tested cases and limits. The Rust core separately passed the pinned opt-in
§7.1 check locally on 2026-09-24.

The later, independently authored [HN/C8 row-model candidate](jbig1-row-model.md)
used the bounded Rust core with an externally supplied T.82 Table 24 fixture.
The local Rust runs matched both raw-stride and visible-bit SHA-256 hashes for
**1,400/1,400** pinned type-0 images, verified **27/27** source hashes, and
verified all 1,400 encoded-span hashes. The [hash-only result ledger](../tests/conformance/jbig1_row_model_results.md)
records the exact scope and opt-in reproduction command. The portable harness
was hardened after its full run to reuse the verified file handle and close
its temporary spool before removal; the full external corpus was not rerun
after those edits. Ordinary clean-clone CI ignores both the official-vector
and CAJ corpus tests, so each is **NOT_RUN** there. These local results do not
resolve the Table 24 rights question in #30 or establish a released native or
WASM image decoder.

Before closing #26, run formatting, Clippy with warnings denied, native
tests, the WASM target check, the repository MIT/source audit, the coverage
gate, and the fixed-budget mutation test. Report the opt-in standard-vector
status and CAJ corpus status independently; an optional check omitted from a
given run is `NOT_RUN` for that run, never a compatibility pass.

## Numeric table rights decision

The exact 113 Table 24 numeric states have no documented MIT redistribution
basis yet. The [provenance register](provenance.md) records the source review;
[issue #30](https://github.com/rwv/caj2pdf-rust/issues/30) holds the detailed
decision criteria and rights-resolution path. Keep the normative tuples and
mechanically generated equivalents out of source, binaries, npm, and WASM
while that decision is open. An external local table supports research but
does not make a standalone decoder distributable. Issue #26 remains open
until #30 resolves, regardless of the local vector result.

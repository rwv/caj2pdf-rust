# HN/C8 type-0 row-model candidate

This is an experimental result for [issue #27](https://github.com/rwv/caj2pdf-rust/issues/27),
not a claim that every CAJ-family image is supported. The starting image
inventory, source SHA-256 values, encoded-span hashes, and oracle raw/visible
hash conventions are pinned in the [#22 manifest](../tests/conformance/jbig1_oracle.json)
and [oracle note](jbig1-oracle.md). Corpus documents, coded spans, decoded
pixels, the external native oracle, and the official T.82 numeric probability
table remain outside this MIT repository. No decoder source or numeric table
was taken from the Python, Go, private Rust, GLWT, JBIGkit, or FreeType
implementations.

## Candidate rule and bounded implementation shape

For each measured type-0 DIB-plus-coded image span, the first 48 bytes are
the observed 40-byte DIB header and two palette entries. The candidate treats
the remaining bytes as one already delimited arithmetic SCD. It starts the
T.82 (03/1993) arithmetic decoder with at least 1,024 zero-initialized
contexts (the ten-bit range), no leading byte skip, no extra prologue symbol,
and no observed stripe restart. Its probability state table is supplied
**externally** in these experiments;
[issue #30](https://github.com/rwv/caj2pdf-rust/issues/30) blocks embedding
the exact Table 24 values in a released MIT decoder.

The candidate iterates rows in display order, using the DIB's visible width
for coding. Immediately before each row, it decodes one arithmetic symbol at
the lowest-resolution three-line pseudo-pixel context described in T.82
§6.5/Figure 11. In the observed CAJ-family candidate, a **one** means copy
the preceding display-order row (background for row zero); a **zero** means
decode every visible pixel in that row using the T.82 §6.7.1 three-line
context. In most-significant-to-least-significant context-bit order, the ten
pixel neighbors are current row `x−2`, `x−1`; previous row `x−2` through
`x+2`; and two rows back `x−1` through `x+1`. Missing neighbors are
background. The three-line pseudo-pixel context is `0b0111001001` (457).
The control bit is an **optional copy instruction**, not an exact
row-sameness indicator: a coded row can still equal its predecessor. This
direct bit meaning differs from
T.82 §6.5's cumulative `SLNTP`/`LNTP` transition rule. It is an observed
CAJ candidate, not a standard T.82 mode claim.

Pack only the visible pixels into each `stride`-byte 1 bpp row, leaving the
unused tail bits and DIB padding zero. The candidate emits display-order
rows; the pinned oracle's raw buffer is in the reverse row order. Reverse
the complete row sequence for **both** hash comparisons. The raw-stride hash
includes each reversed full-stride row; the visible-bit hash uses the visible
bytes of each reversed row with only the unused tail bits masked. This is the
#22 convention.

The independent PDF placement tests in the
[bitstream investigation](jbig1-bitstream-investigation.md) indicate that a
future PDF writer can use the appropriate page matrix rather than hold and
reverse a whole page in RAM.

A production implementation should use the bounded ranged-input core in
[`qm.rs`](../crates/caj2pdf-core/src/qm.rs), at most the current and two
previous pixel rows, one packed output row, and sequential PDF output. The
temporary Python experiment loads external sources into memory for research;
its memory behavior is **not** a production implementation or memory result.
The row rule still needs an explicit public mode boundary, integration-level
failure handling, and native/WASM tests before it can be public API.

## Finite hypothesis grid

A preregistered finite experiment tested 80 combinations **per canary**:

| Dimension | Values |
| --- | --- |
| Coded-byte skip | 0 or 1 |
| Context template | T.82 three-line or two-line |
| Coded row width | visible DIB width or aligned `stride × 8` |
| Extra initial context-0 symbol | 0 or 1 |
| Row handling | no pseudo symbol; decode but ignore one; copy on zero; copy on one; or normative cumulative T.82 `LNTP` |

The prefix limits were 25 display rows of nonblank C8 `issue-33/test1.caj`
page 1 and 150 rows of nonblank HN `issue-21` page 31. The first 5 rows of
C8 page 2 were also checked as a weak blank control. A candidate stopped
after its first raw and visible mismatch. Only skip 0, three-line template,
visible width, no prologue, and copy-on-one survived both discriminating
prefixes. The C8 prefix included 13 nonblank rows; the HN prefix included 10
nonblank rows. A match over blank rows alone was never counted as evidence.
The temporary original script `/tmp/caj27/finite_control_probe.py` has
SHA-256 `b52a739724ef688dea3190edee7d0a67698f885cb3c6c2f2e01e2ab6c5105a56`;
its hash-only report `/tmp/caj27/finite_control_results.jsonl` has SHA-256
`6347ad67c0577e8ffe32bf692aecd99b0d37a58a1986fa012b4d19b3b0b9247f`.
Both remain external to the repository.

## Complete-image canaries

The candidate subsequently decoded the following six **nonblank** image-1
canaries through their complete display-order heights. Every source and
encoded-span SHA-256 was checked against the #22 manifest before decoding.
Both resulting full-image hashes equal that manifest exactly. The entries
below pin the geometry and absolute DIB-plus-coded `(offset, length)`; the
manifest holds their source and encoded-span hashes.

| Kind, sample/page | Span | Width × height; stride | Raw-stride SHA-256 | Visible-bits SHA-256 | Virtual zero reads |
| --- | --- | --- | --- | --- | ---: |
| C8 `issue-33/test1.caj` 1 | `(14778, 79390)` | 2573 × 3285; 324 | `435d70126b94b59d1466b702b0f7e1fb4e2936b7e0d829c3cd968cbb3bfccf83` | `6223db89541e707bd48dc71bc46136082dbedec475040687b17d637914b36bd5` | 1 |
| C8 `issue-33/test1.caj` 2 | `(142826, 2630)` | 2573 × 3285; 324 | `1633c8c74f25dad52bec4419b291880c7acb4c93f7710bf301c39203ee459848` | `4951bdbededd2918509a363e27269242cf8e3ed2952a3aac160f0639eb461cdc` | 1 |
| HN `issue-21` 31 | `(2126707, 17512)` | 2275 × 3425; 288 | `b516cbe6586855d549b8bbc8eee6dcdda69bb488c96ba9416446db63861d2f70` | `8d346853ae11b0f356bf9ef4a3a1203819d0063593dbc263bd1be2c091d348fe` | 1 |
| HN `issue-21` 33 | `(2464776, 14148)` | 2275 × 3425; 288 | `4b76833763d397041323c98a2358df1b08521e2dd0bcabebbf1b3d77808e74d7` | `6af1937915af81ed42f50fba76c8aabfdd953572b597c1c0cd649bcef2b16a77` | 1 |
| HN `issue-21` 61 | `(5355036, 52063)` | 2275 × 3425; 288 | `5d2c05195711aaa177298f53975f526c0d80140960f86c4c5e193b7a620d97c7` | `0683caf5aa9074ea890010fedce56feff81a027ad98eb76715d5dcb23bbb7de5` | 2 |
| HN `issue-85` 4 | `(1214899, 39124)` | 2481 × 3508; 312 | `a64ccf5a5c28f38b8829947c5f54ed38531c67a6bff0326de0b9eb63ba115158` | `d088c66109c6ba1f015d010624e34609c216db6347c63fb4b832a51173af6173` | 2 |

The four-byte HN `issue-7/a.caj` pages 75/85 also match both full-image
hashes, but are nondiscriminating blank controls. Under this candidate,
each decodes one control-zero all-background row followed by 3,396
control-one copy rows. A longer, nonblank image can share those initial
arithmetic bytes and continue with later coded rows, so the common prefix
is not an empty-image marker. This is a candidate explanation of those two
observations, not a universal validity rule.

The six nonblank canaries include two C8 pages, an ordinary HN image, and
three HN first-byte families. Direct oracle-memory-order comparison fails;
the row-reversed comparison passes. In these six images a control-one bit
always copied the previous row, while at least one control-zero row also
equaled its predecessor. No success on skipped optional corpus entries is
inferred from these canaries.

## Stratified bounded check

A separate hash-only batch selected **45 of the 1,400** pinned images: one
median coded payload per source file with type-0 images, every observed
first-byte family, all five images on the page with the most images, named
HN/C8 canaries, and geometry/payload extremes. The selected set covers all
21 such source files, 33 HN and 12 C8 images, all seven observed first-byte
families, widths 666–2573, heights 172–3669, and coded lengths 4–79,342
bytes. Every selected source and encoded span was SHA-256 checked against
the manifest before decoding. Its result is **45 PASS, 0 mismatch, 0 error
or timeout, and 1,355 NOT_RUN**. PASS means both exact full-image raw-stride
and visible-bit SHA-256 match. The missing 1,355 are not compatibility passes.

The temporary Python batch ran in one process under a 256 MiB process-wide
address-space cap. Each image had a 30 s `SIGALRM` wall timer and explicit
ceilings of 1,000,000 coded bytes, 2,000,000 decoded bytes, and 20 million
pixels. All selected cases stayed within these limits. The original
temporary script `/tmp/caj27/batch_hash_probe.py` has SHA-256
`67c803120eaac07e9f12a4f3ed9c2999c63a6e8a72b819053c96ec3dfa66e8c6`;
the hash-only `/tmp/caj27/batch_stratified.jsonl` report has SHA-256
`b5f4df89a6458ba9e3fef743ba1bd639719d21720ea57e52f8653c92494fdcac`.
Both remain external. The 1,355 `NOT_RUN` statuses describe **this Python
batch only**; a subsequent independent Rust batch ran all 1,400 images.
The six named nonblank canaries consumed their entire declared coded spans
and some required T.82 virtual-zero `BYTEIN` afterward. That is a declared
SCD-end behavior, distinct from physical truncation inside the span.

## Full bounded Rust check

An independently authored temporary Rust probe used the repository's
[`ArithmeticDecoder`](../crates/caj2pdf-core/src/qm.rs) with the same
row-control candidate. It loaded the separately supplied T.82 Table 24
fixture only at runtime after checking its 16 KiB size cap and SHA-256
`11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`.
The fixture and standard vector remain outside the repository. The probe
stream-hashed all 27 corpus source files and every DIB-plus-coded image span
against the #22 manifest, decoded each image with the bounded core, and
compared **both** full-image hashes after reversing the output row sequence.
The batch result was **1,400 PASS, 0 FAIL, 0 NOT_RUN** across all 1,400
type-0 images and 27 source files. This includes HN and C8, multi-image
pages, all seven observed first-byte families, varying dimensions, and row
padding. The numbered, per-image [hash-only result ledger](../tests/conformance/jbig1_row_model_results.jsonl)
and [reproduction note](../tests/conformance/jbig1_row_model_results.md)
tie each PASS to an exact manifest image reference. The original full text
report is `/tmp/caj27/rust-batch-report.txt`, SHA-256
`fe5e2f6421d5af904ad28826067d8271c9183061762700dd69191e6b62bb914c`;
the temporary MIT probe source is `/tmp/caj27/rust-canary/src/main.rs`, SHA-256
`01e52ca11e6a764a242d68a1d6bb174c33bf6a6be4808242ebb22476dbe4904b`.
Those `/tmp` files are research artifacts, not required repository inputs.

For each image, the probe capped the encoded span at 64 MiB, decoded output
at 128 MiB, and arithmetic symbols at 12 million; the work budget was at
most `32 × max_symbols + 1024`. The core's input prefetch is at most 256
bytes; the row model used three stride-sized row buffers and a sequential
temporary output spool to reverse rows for hash comparison. These are
resource ceilings in the **probe**, not an end-to-end memory measurement of
the future converter. Each completed image called the arithmetic decoder's
`finish(actual_symbols)`; that checks the symbol count and does not prove
that every byte in the declared coded span was consumed.

## Integration contract and failure modes

For the initial #28 implementation, accept this **observed type-0 mode** only
when the HN/C8 container supplies a checked absolute image span and the first
48 bytes describe the measured one-bit DIB wrapper. In the pinned mode,
`biSize = 40`, `biPlanes = 1`, `biBitCount = 1`, `biCompression = 0`, the two
palette entries are white then black, `biClrUsed` is 0 or 2, and
`biSizeImage` is 0 or exactly `stride × height`. Compute the 32-bit-aligned
stride and total output length with checked arithmetic; compare DIB
dimensions and declared span against source bounds and caller limits before
arithmetic decoding. The remaining `length − 48` bytes are one SCD range.
Unknown image records or wrapper variants require an explicit unsupported-mode
error and new evidence rather than a guessed decoder setting.

Initialize 1,024 arithmetic contexts and the externally supplied valid
113-state table for each observed image; use reset stripe mode. Decode the
row-control symbol and then either copy the previous display row or decode
exactly `width` pixels with the context order above. Count at most
`height × (width + 1)` symbols, enforce a caller-selected work budget and
cancellation, and emit each completed packed row in display order to a
sequential sink. Propagate source short reads, sink failures, cancellation,
arithmetic state errors, and limit failures with source/page/image and byte
location where available. A PDF writer must handle image orientation and
palette explicitly; it should not reverse an entire image in memory.

No checksum or validated end marker has been identified for this CAJ SCD.
The arithmetic core permits virtual zero bytes **after** the declared SCD
end, as T.82 byte input requires, but a physical short read **inside** the
declared span is an error. A declared span cut short at a valid source offset
can still produce plausible pixels through virtual padding; thus a successful
symbol count is not proof that malformed coding was rejected. Production
callers must not present this corpus match as a general CAJ validity check.
The experimental implementation can reject unsupported modes, impossible
dimensions, out-of-range spans, exhausted budgets, and I/O failure; malformed
but in-range arithmetic data may require additional integrity evidence.

## Limits of the evidence

The official T.82 §7.1 arithmetic check and its Table 24 states were used
only through separately supplied local material. The temporary Python and
Rust row prototypes are original research code. All 1,400 **pinned type-0**
images are explained by this candidate, but their success cannot establish
the mode partition of other CAJ-family inputs, dynamic template changes,
deterministic prediction, stripe boundaries, or behavior on malformed coding.
The Rust converter does not yet have a shipped image pipeline, and the
candidate has no browser/WASM integration test. The
[#30 rights decision](https://github.com/rwv/caj2pdf-rust/issues/30)
still blocks distribution of the exact numeric table under MIT and the
released decoder in [#28](https://github.com/rwv/caj2pdf-rust/issues/28).

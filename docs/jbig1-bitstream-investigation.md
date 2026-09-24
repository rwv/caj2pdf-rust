# HN/C8 type-0 bitstream experiments

This note tracks [issue #27](https://github.com/rwv/caj2pdf-rust/issues/27).
It records independent measurements and black-box experiments against the
[pinned image manifest](../tests/conformance/jbig1_oracle.json). A later
[row-model candidate](jbig1-row-model.md) matches all 1,400 pinned type-0
images; support beyond that observed mode and a released decoder remain open.
The corpus documents, coded image bytes, decoded bitmaps, and all external
executable binaries stay outside this MIT
repository. The differently licensed reference decoder was only invoked as a
behavioral oracle; its source, tables, comments, and pseudocode were
not inspected or used.

## Inputs and controls

All 27 HN/C8 source hashes and all 1,400 image-span hashes were checked
against the [corpus matrix](../tests/conformance/matrix.json) and the image
manifest before these experiments. The corpus revision and DIB/stride/hash
definitions are in the [oracle note](jbig1-oracle.md). Pixel comparisons use
the manifest's canonical `raw_stride_sha256` and `visible_bits_sha256`, never a
visual judgment. Native oracle queries use the pinned binary SHA-256
`d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6`
as a black box. The #22 baseline and malformed synthetic probes ran in
separate, timed processes; valid-image suffix probes ran as guarded local
calls. Two output prefills (`00` and `a5`) and guard bytes must agree before
a result is used. The independent standard decoder
was the external Debian `jbgtopbm` binary SHA-256
`9e175f0711f9efcf264d5c968bae6734574a0f231a874df9681817799a748904`;
it was used as a command-line black box, not linked or redistributed.

The [ITU-T T.82 recommendation](https://www.itu.int/rec/T-REC-T.82), section
6.2.2, defines a 20-byte BIH with big-endian dimensions. None of the 1,400
CAJ payloads begins with a matching BIH. Section 6.2.5 describes protected
SCD byte stuffing and stripe markers. Across the measured payloads, 195,715
`ff` bytes have a successor, but only 755 are followed by `00`. No payload
ends in a standard `ff 02` or `ff 03` stripe marker. Therefore these spans
are not complete protected T.82 streams **as stored**; an unframed or
unprotected standard arithmetic core remains possible.

## Prefix and blank controls

The first coded byte is `4b` in 1,348 images and `4c` in 40, with five other
values across the remaining 12 images. Ten payloads share one four-byte
prefix, omitted here to keep corpus bytes out of the repository.
Only two of them, `issue-7/a.caj` pages 75 and 85, consist of exactly four
bytes and decode to all-zero buffers. The other eight are 5,066–39,076
bytes long and decode to nonblank images. In particular, `issue-85` page 4
shares that prefix and has 318,732 one-bits in its oracle output.
The prefix is **not** an empty-image flag.

An original, synthetic all-zero P4 bitmap with the issue-7 dimensions
(2349 × 3397) was encoded using the external standard `pbmtojbg` command
version 2.1, binary SHA-256
`69df355ad594f73461efb04b9a7ad5410de3d2313c196f953e4b6964daae2e18`,
with one stripe and explicit settings `-q -p 0 -m 0 -s 3397 -o 0`. The
synthetic P4 input SHA-256 is
`34ccf11e3ed6fdf3642db6fa3a8a42698da04a2b311e93da74bf8feb31ed3b15`.
The 24-byte BIE SHA-256 is
`93deee666987d09e26caa4c7b436437b9fd2689400e5ea4e69ada97782eae81c`;
its two SCD bytes before the `ff 02` terminator are `4b c6`, SHA-256
`672c8cf92ba64cfd32c0bf0aad185b379c1fda70e5223913e09385df51a09777`.
The same two SCD bytes arose with width 2348/2350 or height 3396/3398,
though those BIE hashes changed. T.82 section 6.8.2.10's FLUSH procedure
treats trailing zero bytes specially; the two additional zeros alone do not
establish a distinct arithmetic coder. Thus the four-byte CAJ blank payload
is consistent with standard arithmetic coding of that particular blank, but
this is a weak control: many constructed standard headers decode it to blank.
It does not establish the nonblank context template, predictor, or byte framing.

Separate four-byte *synthetic* queries show that the native oracle's `void`
ABI does not simply return blank on every short input. Each call ran in an
isolated process with a two-second timeout, distinct output prefills, a
4 KiB output guard, and independent `00`/`ff` bytes beyond the declared input
length; all reported results were stable under those controls.

| Probe input | One-bits at 7 × 1 | One-bits at 9 × 2 | One-bits at 33 × 2 |
| --- | ---: | ---: | ---: |
| Pinned four-byte blank canary; bytes omitted | 0 | 0 | 0 |
| Last coded byte incremented by one | 0 | 0 | 0 |
| Second coded byte incremented by one | 0 | 0 | 0 |
| Four synthetic zero bytes | 5 | 8 | 44 |
| Third byte of synthetic zeros set to one | 5 | 8 | 44 |
| Four synthetic one bytes | 0 | 4 | 13 |

These are **malformed-input behavior probes**, not known valid CAJ encodings:
the ABI has no success/failure return value. No implementation rule is based
on their pixels. The valid `issue-7` page 123 image has 31 coded bytes
and 12 one-bits over nine output rows, while valid `issue-85` page 9 shares
the blank canary's four-byte prefix and has 77,965 one-bits. Both match
the pinned oracle manifest.

## Finite standard-decoder probe

For each selected nonblank image, the [reproducible finite probe](../scripts/jbig1_standard_probe.py)
constructed a standard
single-plane, zero-layer BIH with the measured DIB width and height. It tried
`L0 ∈ {height, 128}`, order `∈ {0, 3}`, options `∈ {0, 8, 64, 72}`, raw
or `ff 00`-stuffed coded bytes, and either `ff 02` or `ff 03` terminator:
64 written combinations per image. A separate temporary experiment also
tested option `28` for the exceptional-prefix images. Inputs and decoder
outputs were kept under `/tmp`. The first experiment compared the PBM's
cropped, unused-bit-masked rows with the manifest visible hash. The committed
probe additionally compares direct and reversed row orders and checks both
visible-bit and raw-stride hashes. Its raw-stride candidates retain the PBM
row bytes and assume zero DIB padding; they are hypotheses, not an external
decoder's DIB output. A decoder exit, a parse error, or a wrong size is not
a pixel comparison; a visible-only match is reported separately and does not
establish full raw-stride equivalence.

Raw coded bytes were rejected by the standard decoder after an unknown
marker on the nonblank canaries below. Stuffing every `ff` allowed a standard
decode under some settings, but no tested setting matched the oracle visible
hash. The standard outputs also contained far more one-bits than the oracle.

The exact source samples, source SHA-256s, encoded-span SHA-256s, expected
pixel hashes, and image numbers are pinned in the manifest. These are the
selected DIB-plus-coded spans (`offset`, `length`) and dimensions used in the
finite standard-decoder comparison:

| Sample and page | Span | Width × height |
| --- | --- | ---: |
| `issue-7/a.caj`, 75 (blank control) | `(5534160, 52)` | 2349 × 3397 |
| `issue-85`, 4 | `(1214899, 39124)` | 2481 × 3508 |
| `issue-85`, 25 | `(7287107, 786)` | 2481 × 3508 |
| `issue-21`, 2 | `(342560, 16159)` | 2275 × 3425 |
| `issue-21`, 31 | `(2126707, 17512)` | 2275 × 3425 |
| `issue-33/test1.caj`, 1 | `(14778, 79390)` | 2573 × 3285 |

| Pinned canary | Coded bytes | Oracle one-bits | Standard outputs with accepted tested settings | Result |
| --- | ---: | ---: | ---: | --- |
| HN `issue-85`, page 4 | 39,076 | 318,732 | about 3.4–3.9 million | Hash mismatch |
| HN `issue-85`, page 25 | 738 | 7,558 | about 4.17–4.61 million | Hash mismatch |
| HN `issue-21`, page 2 | 16,111 | 309,770 | about 3.56–3.88 million | Hash mismatch |
| C8 `issue-33/test1.caj`, page 1 | 79,342 | 628,847 | about 3.96–4.71 million | Hash mismatch |
| HN `issue-21`, page 31 | 17,464 | 150,352 | about 3.48–4.18 million | Hash mismatch |

For the accepted standard outputs of these canaries, exhaustive simple
display transformations in the tested set—bitwise complement, row reversal,
per-byte bit reversal, and horizontal shifts from −7 to +7—also found no
exact visible-hash match. These results reject the **specified finite BIH,
stuffing, option, and display-transform grid**. They do not rule out every
T.82 mode or a standard arithmetic core with CAJ-specific contexts.

An independent, original arithmetic-decoder prototype used T.82's numeric
states and official section 7.1 vector **only from an external copy of the
standard under `/tmp`**; no state table or vector bytes enter this MIT
repository. The official PDF SHA-256 was
`6d4280f4402ce285199b3835dda54e35372e8378e7352d2e88ab3ac420f46942`;
the temporary original arithmetic prototype SHA-256 was
`19a38aa5a77148fa26d0fa75dc389ec14d9cd5a6e30de0586acdbba863778623`.
It reproduced all 256 expected vector bits, SHA-256
`62b5b36a3364f8d927cbc0e2a55c8308c2c1e82cb3dcf2488c95c998499394d6`,
and checked initial register trace values before testing CAJ data. A second
finite grid then tried four SCD starting offsets (0, 1, 2,
4), visible or 32-bit-aligned row width, and four context hypotheses
(constant, previous pixel, T.82 three-line, and T.82 two-line) with typical
prediction off: 32 candidates. Another 16 candidates used the two T.82
templates with typical prediction on. Direct, row-reversed, and complemented
outputs were compared with the oracle in bounded row checks. None matched
the nonblank C8 `issue-33/test1.caj` page 1; all 48 candidates differed
already in row 0.
The all-zero `issue-7` page 75 matched some offset-0 candidates, as expected
for a non-discriminating blank. Offset-0 three-line candidates for nonblank
HN `issue-85` page 4 first differed at row 341, byte 66. A fresh scan of the
pinned oracle's external raw buffer found its **first nonzero byte at exactly
that location**. The apparent match through row 340 covered only a blank
prefix; it provides no evidence that the tested context template is correct.
For C8 `issue-33/test1.caj` page 1, the first nonzero oracle byte is at row 0,
byte 303, where the tested offset-0 candidates first differ too. These are
raw-buffer row and byte coordinates, with no pixel bytes retained here.
Passing the official arithmetic vector and failing this limited CAJ grid are
compatible. At that stage, the private context/prediction model or framing
remained unknown. The grid did not include deterministic prediction,
adaptive-template movement, or stripe
restart. The nonblank canaries used bounded first-row/first-six-row checks
and extended representative survivors to their first difference; the
full-image transform comparisons above belong to the separate standard CLI
experiment.

The exceptional `issue-21` pages 33 and 61 (two unusual first-byte families) also
failed the tested raw framing and mismatched after byte stuffing under
options 0, 8, 64, and 72. Deleting one, two, or four leading coded bytes
from representative HN/C8 nonblank images changed the guarded oracle output
to much denser bitmaps; deleting one to three bytes from the four-byte blank
did so as well. This rules out treating those bytes as an ignorable fixed
prefix in these probes, without proving that each byte is arithmetic data.

## Row placement observation

For a valid 666 × 172 image (`issue-66`, page 3 image 2), calling the external
oracle with the original width and coded bytes but a shorter height produced
exactly the **last `h` raw-stride rows** of the full 172-row output for each
tested `h` in 1, 14, 15, 16, 20, 24, 32, 48, 64, 96, 128, 160, and 171.
A second 2481 × 3509 HN image (`issue-68`, page 6) showed the same suffix
relationship for `h` of 1, 128, 512, 2048, and 3508. The source bytes and
width were held constant; two output prefills and guard bytes agreed.
A C8 image (`issue-33/test1.caj`, page 1, 2573 × 3285) likewise produced
exactly the full image's last 512 and 3284 rows at those reduced heights.
One plausible explanation is top-down decoding into a bottom-up DIB buffer,
but the height experiments establish only the output-suffix relationship.

There is separate PDF evidence for the display direction. The pinned Python
reference PDFs for HN `issue-21` page 2 and C8 `issue-33/test1.caj` page 1
contain Flate-decoded 1 bpp image bytes exactly equal to their oracle raw
buffers. Both use an indexed palette with 0=white and 1=black, a PDF image
width of `stride × 8`, and a negative-Y page matrix. A 300 dpi MuPDF PBM
render of the HN page equals the oracle's padded rows **reversed vertically**,
byte for byte, with no bit inversion. Thus a Rust decoder that emits the
oracle's memory-order rows can stream those bytes into a PDF image with the
measured negative-Y transform. A decoder that naturally emits the opposite
traversal order can stream with a positive-Y placement instead: two original
2 × 2 synthetic PDFs, one with top-down bytes and a positive-Y matrix and
the other with reversed bytes and a negative-Y matrix, rendered to identical
MuPDF PBM bytes. This validates the placement equivalence for that small
geometry; the final Rust path still needs real HN/C8 render checks. A
page-sized row reversal buffer should not be assumed necessary.

## Valid-image pair and mutation controls

The earlier `issue-68` pages 6/7 and `issue-76` pages 2/3 pairs share 15 and
22 initial coded bytes, respectively. Their oracle raw buffers first differ
at memory-order row 217, byte 153 and row 106, byte 273. Those are exactly
the first nonzero rows of one member of each pair: `issue-68` page 6 first
has ink at row 217 (page 7 at 267), and `issue-76` page 3 at row 106 (page 2
at 517). Their shared output prefixes contain only blank rows and do not
establish a shared predictor or arithmetic state.

A stronger valid pair is `issue-68` pages 97/101, image 1, both 2481 × 3509
with a 312-byte raw stride. Their pinned DIB-plus-coded spans are
`(12791062, 19292)` and `(12952640, 22294)`, with encoded SHA-256 values
`1536368271cffd6ff4466f23e36a236c62791a5cedbd3c00454e73be74b52a5e`
and `53b1b57bc9b8feb34d5fcdb14ebaf6771650ad9c6b570fa646a9958e32a75d60`.
The first 1,214 coded bytes are equal. Their complete oracle buffers share
the final 380 memory-order rows (3129–3508), including 39 rows with at least
one visible foreground bit. This is a valid-image nonblank agreement; it is
stronger than the blank-only controls above, though it does not determine the
private context or stripe rule.

All source and encoded-span SHA-256 values were verified against the pinned
manifest before calling the native binary. Each valid or mutated coded span
was passed to the pinned black-box `jbigDecode` ABI in four isolated runs:
output prefill `00`/`a5` crossed with 4 KiB input tail `00`/`ff` beyond the
declared input length. An output buffer of exactly `stride × height` bytes
had 4 KiB guards on both sides. Each child had a 256 MiB address-space cap,
8 s CPU cap, and 20 s parent wall timeout. Guards and all four outputs had
to agree before comparing hashes; all six valid baselines matched both
manifest raw-stride and visible-bit SHA-256 values. The temporary original
orchestration scripts are `/tmp/caj27/pair_divergence.py` (SHA-256
`6e47a0be67c661dbde9ee20e975a31f8e64ab3f38fa7e5bf48b8cf0e72b9ccb7`)
and `/tmp/caj27/pair_p97_probe.py` (SHA-256
`7c2deb07f8f7f3e62a7191cb6822cf7984e07fc37ed48632c573e93a304e821d`).
They and their coded/raw outputs remain outside the repository; the protocol
uses the pinned manifest and external corpus, not embedded document bytes.

A one-byte XOR mutation at p97 coded offset 500 left 196 output suffix rows,
including 12 nonblank rows, matching **unmutated p97**. Mutating the first
differing coded byte at offset 1214 or its successor at 1215 left 397 or 388
suffix rows, respectively, matching unmutated p97, each with 39 nonblank rows.
These mutation comparisons are distinct from the 380 shared rows of the valid
p97/p101 pair. A last-nine-byte flip left p97's output unchanged. In the
blank-prefix pairs,
substituting the first differing coded byte made roughly 0.9–1.0 million raw
bytes differ from the corresponding valid output, beginning at row 0 byte 0.
These are reproducible black-box responses to **possibly invalid inputs**:
the native `void` ABI gives no success/failure signal. They are consistent
with sequential decoding into high-to-low memory rows but cannot prove a
valid CAJ context or restart rule.

## Still unresolved

No tested **self-contained standard wrapper** or simple display transform
reproduced a nonblank HN/C8 hash in the earlier grid. That negative result
does not refute the later [CAJ row-control candidate](jbig1-row-model.md),
which matches complete nonblank canaries with an externally supplied T.82
arithmetic state table. The observed 1,400 pinned type-0 images have since
matched the finite row-control candidate under that external table, as
recorded in the [row-model note](jbig1-row-model.md). This does not identify
the broader CAJ mode partition, dynamic template changes, deterministic
prediction, stripe behavior, or malformed-input handling. A standard-vector
pass,
a blank-image match, or a plausible-looking render alone does not establish
CAJ support. The valid-pair observations above constrain output traversal
but do not prove a context rule; mutated inputs have no validity signal.
The portable, ignored [Rust corpus harness](../crates/caj2pdf-core/tests/qm_caj_oracle_external.rs)
checks the pinned mode independently. Production integration in #28 and an
MIT Table 24 redistribution decision remain necessary before a decoder ships.

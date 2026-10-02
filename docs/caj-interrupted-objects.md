<!-- SPDX-License-Identifier: MIT -->

# Interrupted CAJ PDF objects

Tracking: #226. Issue-92 now converts all 58 pages and passes PDF structure,
page-order and rasterization checks. Viewer pages 1, 4 and 58 have now been
inspected; stable source/output pixel differences remain unclassified.
The other five baseline sources remain unsupported. Source files and diagnostic
mutations remain outside Git. Earlier sections retain the investigation history;
the latest result is recorded at the end.

## Measured prefixes

All three discovery inputs match the SHA-256 values in the conformance matrix.
Offsets below are absolute in the original source. Prefix comparison excludes
only trailing ASCII whitespace; the remaining bytes match exactly.

| Input | Partial object starts | Following header candidate | Matching same-reference header | Matching bytes |
| --- | ---: | ---: | ---: | ---: |
| issue-25 | 116653 | 116724 | 457263 (later) | 69 |
| issue-92 | 19103 | 19116 | 14482 (earlier) | 11 |
| issue-30 | 189169 | 189264 | 191653 (later) | 93 |

The first two failures interrupt dictionaries. The third interrupts a Flate
payload. Candidate locations alone are not sufficient to accept a repair.
An external experiment removing only those prefixes and adjusting the CAJ
page table encounters additional malformed objects in all three files;
there is no successful conversion result from that experiment.

## Admitted known-dictionary rule

On an `expected PDF name` or `invalid PDF value token` error within 256 bytes
of the current object start,
find exactly one already indexed object with the same reference. It must be
a complete non-stream dictionary. Compare its bytes with at most 256 bytes
at the current object start. The exact common prefix, followed only by ASCII
whitespace, defines one candidate boundary. The next byte must be a digit;
normal scanning must then parse an indirect object at that exact boundary.
All subsequent object/link/stream validation still applies before output.

This handles a cut between the two closing dictionary brackets, inside a
key or inside a dictionary array. It does not search for object markers or stream terminators. Changed
fields that do not leave a valid next object fail; unknown references,
conflicting complete objects, stream dictionaries, incomplete literals and
invalid next-object syntax retain errors. No objects are synthesized: the
complete previously indexed dictionary remains in the reconstruction plan.
The existing input, metadata, cancellation and output budgets remain active.

Original tests use invented nested dictionaries and a different following
object, including cuts after an opening dictionary, inside a key and between
closing brackets, plus cuts at and within an array. Negative controls cover altered data, unknown/overflowing
object IDs, streams, literals and non-object suffixes.

## Initial real-source result (historical)

The original issue-92 source passes its interrupted dictionary prefixes with
this rule, then stops at byte 288441, object 186, because stream Length has no
unique bounded repair. No final PDF is published. Issue-25 and issue-30 retain
their initial errors because their matching complete objects occur later;
this rule deliberately requires an already validated dictionary.

Remaining work in #226 includes full-source recovery where uniquely justified,
all six baseline classifications, independent page/content validation and the
three public interfaces. No recovered-complete-document claim is made here.
External receipts are in `caj2pdf-caj-failure-diagnostics-20261001`.


The issue-92 stream failure also has a measured repeated prefix: the object at
267914 shares 131 bytes with the same-reference object at 269578; after the
prefix and whitespace, the next header starts at 268047. Independently inflating
the later copy reaches zlib EOF with 39,784 decoded bytes, matching its declared
uncompressed font length. The 20,460-byte encoded extent contains one trailing
byte after zlib EOF. This is evidence for a later complete copy, not permission
to locate objects by searching binary payloads.

An external hypothesis copy omitting just that 133-byte interruption exposes
a subsequent known dictionary cut inside an array. The array extension uses
the same exact-prefix rule and has original positive/altered-array controls.
The hypothesis copy then fails on a partial object header at its offset 292989;
it still produces no final PDF and is not a successful corpus conversion.


## Adjacent unfinished object headers

A separate narrow case has no object body to recover: `number 0` followed
immediately by a complete object with the same reference. For an unexpected
keyword at that boundary, inspect at most 64 prefix bytes. Require exactly
two whitespace-separated fields, generation zero, and an exact byte prefix
of the successfully parsed following object. Normal scanning retains the
entire following object, including any stream. A different number/generation,
changed header spelling, intervening body, invalid following object or longer
prefix is refused. This uses the parser error position, not marker searching.

Original controls cover dictionary, array and stream bodies, mismatches,
truncation and the exact 64/65-byte boundary. The issue-92 external hypothesis
copy passes its adjacent header at 292989 with this rule, then stops at 346970
on another dictionary prefix whose full object occurs later. This remains a
hypothesis-copy result; the original file still stops at its earlier stream
failure and no final PDF is published.


## The other three baseline failures

The unmodified source hashes match the conformance matrix. An external
metadata-only scanner trace identifies the object start responsible for each
error; the reported failure offset can be well inside another apparent object
and must not be mistaken for the starting object. Temporary tracing was removed
from the production source and executable after measurement.

| Input | Failing object starts | Object | Current failure |
| --- | ---: | ---: | --- |
| issue-39 | 889064 | 320 | Unterminated indexed-color lookup literal; value error later at 898312. |
| issue-85 (Mingtang) | 512113 | 4 | Direct stream length has no unique nearby repair. |
| issue-90/4-[6] | 1314164 | 4474 | Independent zlib decoding also fails with invalid code lengths. |

For issue-39, the literal's apparent closing parenthesis is escaped, so parsing
continues into later bytes. The indexed DeviceCMYK declaration has high value
43, requiring 176 lookup bytes; the short visible prefix does not establish
those bytes. Only one object-320 header candidate was found. Do not guess a
palette or silently remove the referenced color resource. Whole-file textual
candidate searches here are external discovery, not a production repair rule.

For the Mingtang case, object 4 at 559123 shares the first 143 bytes with the
interrupted object. After that prefix and whitespace, another header starts at
512258. This is another later-copy case, not merely a nearby Length typo.

For issue-90, the sole object-4474 candidate's compressed payload shares 3276
bytes with a later *different-reference* image, object 4479. The mismatch is at
1317616, followed by an apparent header at 1317618. The later payload reaches
zlib EOF, but yields 3,466,638 bytes versus 3,433,011 bytes for its declared
1019-by-1123 RGB image. Reaching EOF therefore does not establish correct image
content. A textual reference search finds no reference to object 4474, but that
alone does not prove an unreferenced-object repair is safe. Do not substitute
object 4479 or scan compressed payloads for headers.

These observations classify the next work; they do not complete #226 or prove
any document irrecoverable. External receipts include per-case scanner offsets,
`remaining-three-prefixes.json`, and the independent Flate observations.

## Later copies reachable from page-table anchors

A diagnostic run starts the existing bounded fragment scanner at each nonempty
CAJ page-table span, without searching for headers. Three independently parsed
spans contain the later copies needed by the earlier failures:

| Source | Page-table row | Span start | Complete matching object |
| --- | ---: | ---: | --- |
| issue-25 | 3 | 455713 | 27 at 457263, 121 bytes |
| issue-30 | 2 | 191410 | 2 at 191653, 1058 bytes |
| issue-92 | 4 | 268684 | 186 at 269578, 20545 bytes |

Every successful diagnostic span contains its declared page object. Across
the six documents, successful/failed span scans are respectively 9/69, 53/5,
101/40, 78/2, 179/55 and 0/60 in issue-25/92/30/39/85/90 order. These are local
object-framing observations, not successfully converted pages or documents.
In particular, the Mingtang row containing object 4 fails later in that span;
the current scanner does not return a validated index for that failed span.

This evidence supports investigating a bounded later-copy index rooted in
container offsets. Any production rule must still verify that a candidate is
an actual object reached by the final complete scan, not bytes inside another
stream; reject conflicting candidates; and account for additional reads,
decoded work and metadata. A page-table offset alone does not certify an
object boundary. No such production recovery is admitted by this diagnostic.
The temporary Rust probe was removed after execution. Source hashes match the
matrix; receipts are `page-table-anchors.json`, `all-anchor-probe.log` and
`all-anchor-summary.json` in the existing external evidence directory.

### Candidate recovery implementation

The draft now retries a malformed whole-fragment scan using complete later
page-table span scans. It ignores unsuccessful spans and spans requiring Length
patches, requires the first object to match the declared page ID, and retains
only bounded object metadata. Cancellation, I/O and resource-limit failures
propagate; decompression work is counted across the original scan, anchored
attempts and final retry.

A unique same-reference later candidate may justify an exact interrupted
prefix of at most 256 bytes. Only the first differing byte and following
whitespace define a possible next-object boundary; that boundary must parse
normally. Before returning a successful scan, every used candidate must appear
at its exact range and reference in the complete forward parse. A candidate
inside another stream therefore fails, even if its local syntax is valid.
Existing duplicate-object conflict checks and final document validation still
apply. No source hash or external document byte is embedded in the rule.

Original controls cover dictionary and direct/indirect stream interruptions,
changed prefixes, conflicting candidates, a fake object inside an opaque
stream, short reads, metadata limits and shared decompression work. A two-page
public-API fixture produces the same PDF bytes as its unmodified counterpart;
changed or incorrectly anchored variants fail before writing output.

On the unchanged external sources, issue-25 now stops at 119435 (unfinished
header), issue-30 at 221220 (missing object terminator), and issue-92 at 347103
(another dictionary interruption). The other three retain their prior errors.
All six still fail without publishing a final PDF. These newer results supersede
the earlier stop offsets above, not the requirement for complete conversion.
Receipts are in `caj2pdf-caj-candidate-recovery-20261002`.

The current implementation passes the full Rust coverage gate (32,015/32,015
lines, 100% per file), workspace all-target/all-feature Clippy and a fresh
release WASM build. Node v24.13.0 and real Chromium tests pass 41 cases with
zero skipped, including a Dedicated Worker later-copy success and changed-prefix
rejection. Their original outputs pass the existing qpdf check. These results
validate the recovery mechanism and adapters; the six external failures above
remain open, and hosted platform gates are still required before merge.

## Deferred syntax proof and first complete source

A short interrupted dictionary, array or scalar can now be deferred until the
complete forward scan proves a later same-reference counterpart. The boundary
comes from the syntax error, or at most two immediately preceding lexical
tokens when parsing consumed the next object's numeric header. It never searches
ahead for object markers. Only reference/range metadata is retained; each prefix
is at most 256 bytes. Before acceptance, the prefix must match the complete
object byte for byte, excluding only trailing ASCII whitespace. Existing
duplicate checks reject conflicting complete copies. Stream-embedded fake
objects cannot satisfy this proof, and indirect Length verification still runs.

A repeated unfinished `number 0` header can also use a unique already indexed
object with an identical header prefix. A known dictionary prefix may belong to
a stream object only when the interruption remains inside its dictionary, before
payload bytes; the entire previously validated stream is preserved. These rules
are covered by original controls for changed bytes, absent counterparts, malformed
following syntax, stream-embedded decoys and propagated resource limits.

On the original issue-92 input (SHA256
`49be4cec9ac6334b02355b30785407b9c9048bf1cf392cf470e670349dc51f50`),
the implementation produces a 633,831-byte PDF with SHA256
`853491f2e51ce91da80e8498b3223dbc29e2ac98a0d39229bb92ffc99eb24892`.
`qpdf --check` succeeds; all 58 output page object IDs match the ordered source
page-table IDs, and MuPDF renders all 58 pages. This is complete conversion and
structural evidence, not a claim that every page has been visually verified.
CAJViewer comparison remains **NOT_RUN**. External receipts and renders are in
`caj2pdf-caj-candidate-recovery-20261002`, including
`issue-92-first-complete-check.json`.

Issue-25 still fails at 466247 (unfinished header); issue-30 at 463864
(dictionary syntax). Issue-39, Mingtang issue-85 and issue-90 retain their prior
failures. None is declared irrecoverable solely because this implementation
still rejects it. #226 remains open for those cases and independent content
validation. The 41 Node/Chromium tests pass without skips against freshly built
WASM, including original fixtures that require anchored stream recovery as well
as deferred dictionary proof. Hosted CI and the final reviewed commit remain
separate release gates.

### Split closing dictionary delimiter

Issue-30 object 66 begins at 463816 and stops after the first `>` of `>>`;
the parser reports 463864, and the next complete object begins at 463867.
The same-reference object at 888798 has an identical 49-byte prefix. The
deferred proof now retains that closing bracket and skips only immediately
following ASCII whitespace, within the same 256-byte bound, before attempting
normal object parsing. The final full-scan counterpart check remains mandatory.
Original controls cover this split, changed dictionary content and excessive
whitespace; the complete Rust gate covers 32,288/32,288 lines, 100% per file.

The original issue-30 now stops at 966008 on another unfinished header
(`7422` without its generation/keyword); no final PDF is created. A later
7422 header at 1131327 is an observation for subsequent investigation, not
yet an admitted recovery rule. External source hash, exact offsets and prefix
equality are recorded in `issue-30-split-bracket.json` beside the earlier
receipts. Issue-25's unfinished 430 header has no exact `430 0 obj` occurrence
in the independent byte inventory; this does not establish irrecoverability.

The fresh release WASM build and 41 Node/Chromium tests pass without skips.
Re-running issue-92 with this change produces the identical PDF SHA256 recorded
above and again passes qpdf, preserving the first complete-source result.

### Later complete headers and already indexed lengths

The deferred proof now accepts an unfinished header consisting solely of an
object number, or an object number plus generation zero. It keeps the existing
64-byte header budget, accepts only a normal next-object boundary within the
same three adjacent lexical positions, and requires the complete scan to find
an exact later same-reference object. Missing, conflicting or stream-embedded
counterparts remain errors. Arbitrary broken keywords are not admitted.

For an indirect Length whose filter cannot be independently framed, the scanner
can use a scalar object already parsed from the fragment. It still checks the
stream tail, rejects conflicting duplicate objects, and verifies the final
resolved Length. It does not scan opaque payloads for markers or implement a
new filter decoder. A future unresolved scalar retains an unsupported error.

These rules advance unchanged issue-30 past the unfinished 7422 header and
ASCII85 stream object 15417 (referencing already parsed Length 15420 = 986).
The next failure is byte 1206814: a 15431 header cut inside `obj`. A complete
15431 header exists at 1271891, but that observation is not yet an admitted
rule. No final PDF is produced. Evidence is retained externally in
`issue-30-header-and-length.json`. Original negative fixtures cover missing or
conflicting counterparts, nonzero generations, invalid keywords, hidden objects
in streams, bad lengths and unresolved future lengths.

### Interrupted `obj` keyword

The header rule also permits `number 0 o` and `number 0 ob`, with the same
64-byte bound and mandatory exact later counterpart. It consumes only the
fixed partial keyword and following whitespace, not arbitrary text. A malformed
adjacent-header candidate now lets the remaining proof rules run; I/O, resource
and cancellation errors still propagate immediately. Original controls reject
unknown keywords, nonzero generations and excessive whitespace, and confirm
that a candidate's syntax limit is propagated.

Issue-30 now stops at 1254859: scalar object 7461 is cut inside its `endobj`
terminator. That rule is not yet supported. Issue-25 reaches an unresolved
indirect Length at 565829 (object 373); deferred prefixes have not yet received
final whole-fragment verification, so this progress is not successful recovery.
Both fail without a final PDF. Issue-92 still converts to the identical SHA256
recorded above and passes qpdf. New external receipts are
`keyword-regression-results.json`, alongside the earlier source observations.

### Known integer terminator prefixes

The existing known-prefix comparison is shared by dictionaries and unsigned
integer objects. This covers a cut inside an integer object's `endobj` when a
unique, byte-identical complete same-reference object was already indexed. The
256-byte bound, exact common-prefix boundary, normal following-object parsing
and final conflict checks are unchanged. Original tests use all five nonempty
partial terminator spellings, changed scalar/keyword bytes, conflicting prior
copies and a new following object that must be retained. No integer value or
terminator is synthesized.

In issue-30, partial object 7461 at 1254841 matches its complete object at
1244614. Conversion now reaches 1455036, object 15446, where JPEG framing
fails. The two observed 15446 headers at 1454773 and 1603523 are diagnostic
candidates, not yet proof of a complete later object. No final PDF is produced.
Receipts remain external in `issue-30-known-integer.json`.

## Cross-row candidate collection

Anchored row collection now shares the same scanner with complete-fragment
validation, but returns only candidate object metadata. Locally framed stream
extents still require normal syntax/codec checks, and patched rows supply no
candidates. Deferred prefix and indirect Length references may leave the row;
those obligations are checked by the complete-fragment scan before conversion.
The public internal complete-scan entry point always enables those checks.
Every used candidate must still appear at its exact reference and byte range
in that complete forward scan; a locally plausible object inside an opaque
stream cannot justify a repair. No iterative repair scheduler is introduced.

An original three-span regression covers a truncated stream in the first span,
its complete copy in a second span, and a dictionary counterpart required by
the second span in the third. Removing that counterpart still fails final
validation. Separate controls cover unresolved/mismatched cross-row Length
references and exclusion of patched rows.

This admits the previously blocked issue-30 JPEG candidate at 1603523 and
advances conversion to byte 1819762, object 7566 (invalid Flate framing). It
does not complete that document. Issue-92 retains its identical 58-page PDF
and passing qpdf result (`issue-92-cross-row-regression.json`, external).
All other existing final validation and resource budgets remain active.

## Current issue-92 viewer observations

Pinned offline CAJViewer image
`sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`
opened the original CAJ and the unchanged 58-page PDF. Viewer font resources
were unmodified; the environment included the existing Noto CJK UI font.
Display was 1600×1200 at 96 DPI, 80% zoom, single-page mode. Page-number
controls and all physical page edges were inspected before comparison.

The cover (page 1), English abstract (page 4), and final acknowledgments
(page 58) show no obvious missing content or layout loss. This is a selected-page
observation, not full-document visual acceptance or an exact text comparison.

| Page | Changed RGB pixels | Threshold-128 mask differences | Source/output repeat differences |
| --- | ---: | ---: | ---: |
| 1 | 24,330 | 1,267 | 0 / 0 |
| 4 | 71,612 | 5,249 | 0 / 0 |
| 58 | 49,907 | 4,356 | 0 / 0 |

Each crop is 634×897 pixels. Crops use independently observed physical-page
bounds; no rescaling, content registration or tolerance was applied. Ink bounds
agree on page 1 and differ by one pixel at the left edge on pages 4/58. These
measurements do not classify the remaining differences as harmless. Other
pages remain visually unchecked. Earlier NOT_RUN entries above describe the
historical stage before this observation.

The external `caj2pdf-caj92-viewer-20261002` directory retains `launch.json`,
action receipts, full desktop/repeat captures and `comparison.json` with
source/output hashes and crop bounds. The container was stopped. Source pages,
text, screenshots and vendor assets are not committed.

## Remaining same-row dependency in issue-30

Object 7566 at 1819698 shares 78 bytes with its same-reference occurrence at
2403619. Both are within page-table row 46 (offset 1783324, length 690825);
there is no intervening page-table anchor to admit the latter through the
current collector. The first copy interrupts Flate data. A later textual
header occurrence alone is not a verified object boundary, so this observation
does not authorize skipping the damaged payload. Complete conversion remains
open; a different independently justified boundary source is needed.

## ASCII85 indirect stream lengths

The scanner now validates ASCII85's encoding-defined end marker, allowed
characters, zero-group abbreviation, full-group range and final partial group.
It retains only a group accumulator and counters, with cancellation and the
shared decoded-work budget. It does not search binary data for PDF object
markers. Referenced Length and the normal PDF stream tail still must agree.
Original controls include all final-group lengths, whitespace, overflow,
malformed terminators, zero abbreviations inside groups, missing terminators,
work exhaustion, cancellation and a wrong final Length object.

Issue-25 now reaches byte 565972, object 373: its interrupted ASCII85 payload
fails encoding validation. The complete same-reference occurrence follows
inside the same page span and is not admitted merely from a textual search.
The input remains unsupported and publishes no final PDF. This adds standard
framing support; it is not a complete-source recovery claim.

## ASCII85 adjacent replay derived from Length (#226 follow-up)

An additional original rule derives a restart boundary from the first ASCII85
`~>` terminator, a complete stream/object tail, and the immediately following
referenced unsigned Length object. Subtract Length and the current header
length from the encoded end; accept only a later start within 4096 bytes, an
identical header of at most 256 bytes and an exact interrupted prefix (apart from trailing
whitespace). Validate every ASCII85 group from the derived start, then resume
the normal complete scanner. No object header is searched for, no decoded
stream is buffered, and boundary scanning shares work limits and cancellation.
The replay check runs before measuring the apparent original stream: PDF header
bytes can themselves form valid ASCII85 groups, so malformed-group errors are
not a sufficient trigger. Original visible-rectangle controls exercise this case
through Rust, Node and a real browser Worker, comparing the entire recovered PDF
with its uninterrupted counterpart and rejecting a false Length without output.
A native CLI check also renders the authored first page with MuPDF at 72 dpi:
the 200×100 page contains exactly the expected 30×40 blue rectangle at PDF
coordinates (10,20), with white elsewhere. Both PDFs pass qpdf and are
byte-identical; the malformed input exits nonzero without a final PDF.
External receipts are in `caj2pdf-ascii85-public-20261002`.

For pinned issue-25, object 373 starts at 565829, its header occupies 57 bytes,
the first EOD ends at 566503, and immediate object 374 gives Length 508. This
derives object start 565938 and a 109-byte interruption. The complete encoded
stream independently decodes to 768 bytes. Original synthetic tests cover
wrong references/lengths, changed prefixes, invalid/truncated encodings,
excessive prefixes, cancellation and work-budget propagation.

### Interrupted tail keywords

The next issue-25 failure at byte 598735 cuts `stream` as `stre` after a
complete dictionary. The existing deferred syntax-prefix rule now recognizes
proper prefixes of `stream` and `endobj` followed by whitespace. It retains
the existing 256-byte bound and requires an exact complete counterpart from
the full object scan. Arbitrary tokens, missing or changed counterparts and
excessive whitespace remain errors; this does not search inside stream data.
Original one-byte-read tests cover every proper prefix of both keywords.
Node and browser Worker controls compare the recovered PDF with the complete
version of the same authored rectangle stream.

### Direct-Length Flate replay after an exact scalar repeat

The subsequent issue-25 failure at byte 710644 has a short interrupted direct-
Length stream followed by an exact repeat of the immediately preceding parsed
integer object, then a complete stream copy. Object 316 first starts at 709655;
scalar 315 repeats at 709741; the complete stream starts at 709765. Its 84-byte
trimmed interrupted prefix matches exactly. Independent zlib decoding confirms
938 encoded bytes and 2950 decoded bytes with a valid end marker/checksum.

The initial scalar-anchor implementation admitted this bounded combination only
after ordinary direct-Length tail validation and the existing small Length
repair failed. The preceding
object was required to be a uniquely indexed integer; the final review below
extends this exact-byte proof to other non-stream objects. Its exact bytes must
occur once within the interrupted object's first 256 bytes, after some payload bytes.
The immediately following object must have the same reference, header and
interrupted prefix; independent Flate framing must equal the declared Length,
and the complete stream/object tail must parse. A candidate is then reparsed
normally. This does not search later pages or arbitrary object-header markers.
Fixed prefix buffers and the existing shared codec-work budget bound the work.

Original one-byte-read controls reject absent/noninteger/duplicate/oversized
anchors, changed scalar values, multiple repeats, header/prefix mismatches,
invalid checksums and unequal codec extents. A small work budget propagates its
limit error. Native CLI, Node and a real browser Worker compare an authored
rectangle PDF against its uninterrupted version; wrong checksums publish no
output. Native qpdf and MuPDF checks confirm the same independently specified
rectangle bounds. Receipts are in `caj2pdf-scalar-replay-public-20261002`.

### Cut indirect-reference suffix

At byte 954402, object 268 contains `/Length 271 0` cut before `R`. The value
parser has accepted `271` as an integer and reports generation zero where the
next dictionary name should begin. The existing deferred-prefix path now
permits that single `0` followed by whitespace, within its original 256-byte
bound. The full scan must still prove the exact entire prefix against a complete
same-reference dictionary; it does not infer or insert a missing reference.
Original one-byte controls reject `0x`, `00`, nonzero generations, excessive
whitespace and changed/missing counterparts. Public Node/browser controls
compare a cut-reference stream with its uninterrupted PDF.

At this intermediate stage, issue-25 stopped at byte 1572389, object 145, in
another direct-Length stream interruption. The final review below supersedes
that stopping point; #226 remains open. External source spans and per-step
diagnostics stay in
`caj2pdf-caj-candidate-recovery-20261002`.

### Final replay review and current issue-25 limit

A second observed ASCII85 interruption has a 523-byte prefix. The same EOD and
immediate Length proof derives the complete start 1873341 from the encoded end
1874388 and Length 990; independent decoding produces 768 bytes. The prefix
bound is now one fixed 4 KiB window, with the existing 256-byte header bound.
This is still bounded copying of encoded prefixes, not a decoded stream buffer;
all reader allocation/work limits remain enforced. An original 520-byte payload
prefix succeeds with one-byte reads, and over-4-KiB prefixes remain rejected.

For adjacent direct-Length Flate copies, inspect only the 256 bytes after the
declared encoded end for a stream tail. Subtract the known Length and header
size to derive a restart. Require an exact interrupted prefix including the
already parsed header, independent zlib framing/checksum and a complete tail.
The previous-object anchor rule also applies to already parsed non-stream arrays
and dictionaries, not just integers: the proof depends on exact object bytes and
uniqueness. Stream anchors remain refused. Object 443 uses an exact repeat of
array object 442; no additional recovery framework is needed.

The encoded extent may include one LF, CR or CRLF after the zlib end. Such bytes
are preserved and checked explicitly; arbitrary padding and a mismatch remain
errors. This covers object 443's 2574-byte zlib stream with one counted LF in
Length 2575. Original controls cover all three line endings and invalid padding.

A new negative control also exposed an older Length-repair weakness: a short
repeated header could be incorrectly absorbed into a larger Flate Length. A
proposed Flate Length repair now requires independent codec validation, so this
cannot produce a nominally repaired but undecodable stream. Exact-prefix checks
already prove complete header equality; redundant separate header comparisons
and reparsing were removed. The final scanner still parses the recovered object.

Fresh CLI controls for adjacent and array-anchored replay (including counted LF)
produce byte-identical clean/recovered PDFs, pass qpdf and render the independently
authored blue rectangle exactly with MuPDF. Node and real browser Worker controls
use the same public paths. Receipts: `caj2pdf-framed-replay-public-20261002`.
The 58-page issue-92 regression still passes qpdf with unchanged output SHA256
`853491f2e51ce91da80e8498b3223dbc29e2ac98a0d39229bb92ffc99eb24892`.

Issue-25 now reaches final deferred-prefix validation. It refuses object 450 at
455462: the surviving prefix declares Length 3304 and cuts `/Type/Metada`, while
the original Catalog references it through `/Metadata 450 0 R`; no complete
object 450 exists in the scanned fragment. No output PDF is published. This is
a remaining missing-metadata target, not successful full-document conversion.
The implementation does not invent metadata, silently drop the unresolved
reference or claim full issue-25 support. Receipts remain outside Git in
`caj2pdf-caj-candidate-recovery-20261002/issue25-replay-reviewed.json`.


## Deferred stream prefix proved by a local Length replay

For issue-30, object 7566 at 1819698 contains a 64-byte header and an interrupted
Flate payload. The immediately preceding Length object 7565 at 1819675 repeats
exactly once at relative offset 80, within the existing 256-byte anchor window.
Its 22-byte object ends before the independently parsed object 14916 at 1819802.
The trimmed 78-byte interrupted prefix matches the complete object at 2403619;
independent zlib decoding validates 391 encoded bytes and 722 decoded bytes.
External receipt: `caj2pdf-caj-candidate-recovery-20261002/issue30-deferred-scalar-boundary.json`.

The scanner reuses the existing exact anchor search only after an indirect
Flate codec failure, with the immediately preceding object required to be its
known Length scalar. It retains the bounded prefix in the existing deferred
index and continues after the anchor. The final full scan must independently
reach and prove the complete same-reference object; a header hidden inside an
opaque stream cannot satisfy this check. Missing, changed, conflicting or
invalid-codec counterparts remain errors. Work budgets are not refunded.

Final validation now also accepts a unique already parsed complete counterpart.
This covers issue-30's 143-byte array prefix at 943121 (complete array at 439245)
and bare object-number prefix at 1265856 (complete object 7460 at 1244640).
Duplicate prior objects remain ambiguous during prefix admission. This reuses
one exact-prefix proof rather than adding array-specific recovery code.

The resulting native PDF has 141 pages, matching source metadata; qpdf passes
and MuPDF renders every page. Output SHA256 is
`01e138e904710100e75f39de16d2101a1b80fed345eaf5bceba1565a12e14efb`.
A separate read of the source's 12-byte page-table records confirms that all
141 page object IDs match qpdf's output page order. Independent qpdf extraction
also confirms source/output encoded identity for repaired stream 7566 (391
bytes) and image 7460 (10009 bytes); all 2741 ordered references in complete
source array 64 match the output array. These checks target the recovered
content, not every page's visual appearance.

On commit `867fb33`, Node and a real Chromium Worker both produce the same PDF
hash, 141 pages and 31 bookmarks. All four scratch stores finish at zero bytes;
browser input/output cleanup leaves OPFS empty. Post-conversion WASM memory is
4587520 bytes on both JS paths; this is not a peak-memory measurement. External
receipts and reused public-adapter runners are in
`caj2pdf-issue30-public-20261002` (`page-order.json`, `repaired-content.json`,
`node/result.json`, `browser/result.json`). The selected CAJViewer results below supplement these structural checks;
structural identity and cross-runtime agreement alone do not prove fidelity.
Original MIT rectangle controls produce byte-identical clean/recovered PDFs
through Node and a real Chromium Worker. Native qpdf/MuPDF checks reproduce
blue bounds `(10,40,40,80)` at 72 DPI on the authored 100-point page. Missing
counterparts, opaque-stream decoys, changed prefixes/checksums and ambiguous
anchors are covered by original one-byte-read controls. External native
receipt: `caj2pdf-caj-candidate-recovery-20261002/deferred-public-control.json`.


### Issue-30 selected viewer comparison

Pinned offline CAJViewer image
`sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`
opened the unchanged source and accepted native PDF, using its original font
resources plus the existing Noto CJK UI font. At 1600×1200, 96 DPI, single-page
mode and displayed 80% zoom, independently inspected physical interiors are
634×897 pixels at `(508,218,1142,1115)` (exclusive right/bottom).

| Page | Changed RGB pixels | Source/output repeat differences |
| --- | ---: | ---: |
| 1 | 277905 | 0 / 0 |
| 18 | 0 | 0 / 0 |
| 25 | 0 | 0 / 0 |
| 46 | 0 | 0 / 0 |
| 141 | 16236 | 0 / 0 |

Pages 18/25/46 cover the repaired array, image and Flate-stream page-table rows;
their complete physical page crops match exactly. Visual inspection of all five
selected pairs found no obvious missing content or layout loss. The cover and
last-page residuals remain unclassified; this is not whole-document pixel
parity or an independent text-diff claim. No resizing, content registration or
comparison tolerance was applied.

Several early navigation attempts retained page 141 or transient paint despite
the requested page number. They are excluded. The selected captures have
visually verified tab, page-number and zoom controls, plus identical repeats.
The external `caj2pdf-caj30-viewer-20261002/comparison.json` names each accepted
capture explicitly; `compare.py`, action receipts and diagnostic captures remain
outside Git. The viewer container was stopped after collection.


### Six-case regression review after deferred-prefix recovery

A fresh native build of `4348b5b` was run against all six original #226 sources;
each SHA256 matches `tests/conformance/matrix.json`. Results:

| Case | Result |
| --- | --- |
| issue-25 | Located rejection at 455462, object 450: missing complete prefix counterpart. |
| issue-30 | 141 pages, qpdf passes; unchanged accepted PDF hash above. |
| issue-39 | Located rejection at 898312: invalid PDF value token. |
| issue-85 Mingtang | Located rejection at 529945, object 4: no unique bounded Length repair. |
| issue-90 4-[6] | Located rejection at 1318436, object 4474: invalid Flate framing. |
| issue-92 | 58 pages, qpdf passes; unchanged `853491f2…24892` PDF hash recorded above. |

All four rejected inputs leave no final output PDF. Reaching a later diagnostic
is not a compatibility pass. The original issue-39 palette and issue-90 image
ambiguities remain unresolved; no guessed replacement was admitted. Mingtang's
later object-4 occurrence still needs an independently justified path through
the anchored scanner. Source hashes, executable hash, exact commands, exit
statuses and outputs are retained externally in
`caj259-source-review-20261002/results.json`, with its small `run.py` driver.
This review adds no new visual claim or whole-file conversion API.

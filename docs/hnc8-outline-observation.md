# HN/C8 outline investigation and implementation proposal

Status: **DRAFT — PUBLIC PREPARATION ONLY.** This selected-source amendment
proposes a finite Stage A for
[issue #119](https://github.com/rwv/caj2pdf-rust/issues/119), a child of #10.
Its prerequisites #112 and #117 are closed. The first broader Stage A ended
FAIL and is preserved below; the amended phase has not run. The adapter refuses
this DRAFT. The parent and independent reviewer must approve the exact amended
document and executable pins, and the parent must commit a frozen contract
before explicitly authorizing one invocation. Source mutation, converters,
vendor execution, Stage B and core outline-field implementation remain outside
this phase. No vendor outline oracle has run.

## 1. Evidence available now

The repository's original MIT code establishes HN-A's checked outline-like
interval, not its contents:

- A nonnegative little-endian i32 at absolute `0x158` contributes `308 * N`
  bytes between `0x15c` and the page index.
- The page-index start is `0x15c + 308 * N`; each source row occupies 20 bytes.
- `Hnc8Reader` checks the count against `Budget.max_outline_records` and
  bounds the resulting index. Its public `Header` does not retain an outline
  count or span. It is declared metadata, not an unforgeable capability.
- C8's index starts at `0x50`; HN-B's starts at `0xd8`. Neither fact proves
  that its source has no outlines elsewhere.

The committed matrix's top-level counts are historical Python `show`
observations. It has 27 HN/C8 rows, 19 with positive outline-like counts,
1,241 such records in total, and a maximum count of 111. It does not establish
title bytes, encoding, hierarchy, destinations, or successful outline output.
In that matrix HN-A/C8 conversion rows are skipped. Zero-count C8/HN-B
observations cannot establish a general absence rule.

The independently verified existing #107 reference report is 40,896 bytes,
SHA-256 `8cc9fce6c8c0f1f97f7d39453d0734d053fd543aa023a04ef835bf187918d959`.
It pins six deterministic PDFs (two copies of each of three profiles) and
unchanged before/after hashes for all 27 sources. It records page/draw facts,
**no outline count or title/hierarchy/destination fingerprint**. These PDFs
are candidates for an outline oracle, not already validated title oracles.
The completed #117 native composition report likewise leaves outlines
unverified. Its corrected HN-B image ColorSpace basis is irrelevant to title
semantics; preserve the original six reference PDF identities here.

Public metadata bindings:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `tests/conformance/matrix.json` | 237,750 | `af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9` |
| `tests/conformance/hnc8_layout_oracle.json` | 99,829 | `4b88befeecf9a68dd6eca4966c79ea8cdb130c43e3c6d92f4cb56fb34dfb665e` |
| `tests/conformance/jbig1_oracle.json` | 546,731 | `e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a` |

All labels below omit document titles and paths; the public matrix resolves
the complete source identity. No runtime algorithm may use these identities
as an outline-format lookup.

| Initial profile | Source label and SHA-256 | Source bytes / physical pages / historical show count | Reference PDF bytes / SHA-256, each of two copies |
| --- | --- | --- | --- |
| HN-A | issue-21; `33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4` | 5,753,314 / 68 / 52 | 8,239,244 / `833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40` |
| C8 | issue-33; `35951c3775790c230c84e4312e8db7a57ca2980a04d35ff03d328806df7d622c` | 2,625,902 / 7 / 0 | 2,759,609 / `acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885` |
| HN-B | issue-65; `e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49` | 862,235 / 6 / 0 | 826,645 / `b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51` |

The selected emitted mapping is HN-A source 1..68 to PDF indices 0..67;
C8 source 1..7 to indices 0..6; HN-B source `[1,6]` to indices `[0,1]`, with
source rows `[2,3,4,5]` omitted. Source count is not PDF count.

### Preserved first Stage A failure

The original protocol was committed at
`3e1860dec485d6d359c46c32393daec63e4caa8b`: 37,544 bytes, SHA-256
`ba5a53f3ec7a774fca52a53c9f18fe3873e7faff84919f05aa7fb774e8410dc1`.
Its adapter was 65,006 bytes, SHA-256
`0b967f10a707d308d748ebddf6793aa7bd48336d99e3d03b6c3336af9bd370ab`,
and its tests were 43,362 bytes, SHA-256
`9eab0def4f97d5f28b500230fc0fbfe8ec12f3cde9cfa9f32441a893c95964ea`.
Those files, public preparation/input manifests, preflight and closed external
artifacts remain unchanged in the original worktree.

One authorized invocation ended **FAIL**, with compatibility **UNVERIFIED**.
The closed mode-0400 CLI report is 41,723 bytes, SHA-256
`e8660ffd526b06ddecf30c59e242d88ec5002f3fd8350e748815ffef221212f9`.
It records two of 27 source inventory attempts: one completed, one failed,
25 unstarted. All six PDFs, 12 outline queries and discovery were **NOT_RUN**.
The second attempted source is public identity
`3f3b9b57d6925df811247dced47fd7fb74cf0f678ab9bfda0827c827258ab39b`.
Only the generic sanitized failure was retained, so its field/reason is
**UNKNOWN**. A prior historical error label or separately published invalid-row
fact cannot establish which guard failed in this invocation.

The invocation made 12 public version/startup child calls plus one runner
(aggregate 13), with zero converters, native calls, renders or vendor calls.
Before and after, all 93 declared file identities were verified unchanged
(93 + 93); closing runtime verification passed. It requested/read 2,300
logical field bytes in seven calls, with no title-record discovery. These are
the first run's actual counts, never amended-run success or expected-negative
evidence. Preserve FAIL/cause UNKNOWN; no retry or retroactive blessing occurred.

The narrower proposal below follows the already selected three pinned reference
profiles. It does not classify other corpus inputs as valid, invalid, supported
or incompatible. No historical skip/error label grants an exemption for a
required selected-source failure.

## 2. Stage A: a finite, zero-converter field/oracle observation

Stage A must first have an original MIT external adapter, original synthetic
positive/refusal controls, independent correctness/provenance and
simplification review, a committed frozen plan, and an immutable execution
receipt. Until then every count below is planned, not executed.

### Exact allowed input scope

1. All 27 sources pinned by the public matrix, opaquely hashed before/after.
   Their total size is 264,682,650 bytes; the largest is 73,763,774 bytes.
2. Header/index observations only for the exact **three source SHA-256
   identities in the profile table above**, including issue-21 discovery.
   After recognizing the already specified signature/marker, read the prefix
   `[0,0x15c)` for HN-A,
   `[0,0x50)` for C8, or `[0,0xd8)` for HN-B, and the checked declared
   page-index interval of `page_count * 20` bytes. Retain safe variant,
   count, span, per-row image-count/no-image metadata and interval hashes;
   unknown header fields stay unknown.
   The remaining 24 sources each have `inventory_status=OUT_OF_SCOPE`,
   `semantic_status=NOT_RUN` and `semantic_passes=0`. Their opaque identity
   before/after audits stay mandatory; a mismatch is fatal. Outcome/skip/error
   labels never determine this scope. Inventory progress plans three selected
   sources, while the receipt and report separately declare all 27 audit inputs
   and the 24 excluded identities. A failed selected header, row, count or span
   is fatal, with honest failed/unsupported/remaining counts and closing audits.
3. Interpret exactly issue-21's 52 outline-like records: zero-based record
   `r` is the 308-byte interval `[0x15c + 308*r, 0x15c + 308*(r+1))`.
   This is 16,016 bytes, ending at 16,364; its 68-row index ends at 17,724.
   Recheck the raw count, variant, offset arithmetic and source containment
   before reading. Do not inspect another source's title-bearing records in
   this discovery phase.
4. Query only the six already generated, pinned PDFs. No new Python/native
   conversion, source copy/mutation, image extraction, page-text observation,
   image/text-frame decoding, page rendering, or vendor invocation.

Application-level provenance digest reads are separate from field reads:
full-file 64 KiB opaque hashing necessarily reads encoded document/PDF bytes,
but does not parse or extract them. The field reader is unbuffered and requests
at most 308 bytes; page-index reads use at most 4,096 bytes. Record requested
bytes/calls before each read, including failures/short reads. Every exact
field window and its SHA belongs in the external report. Never print raw
private record bytes or decoded titles.

### Safe selected-source failure locations

Schema-v2 contract, receipt and report bind `three-pinned-layout-profiles-v1`,
the exact profile/source and discovery SHA-256 identities, and the preserved
first FAIL/report/counts above. The original schema-v1 contract cannot authorize
this changed adapter. The receipt is sealed before any private audit, including
all 27 sources; selecting field work never removes an identity audit.

`SourceObservationError` reports only fixed stage, field and reason enums,
the source hash, its one-based public inventory ordinal and selected-source
ordinal, an optional one-based physical page/record ordinal, and a declared
absolute schema offset/width. No observed raw value, private bytes/title/path,
traceback or arbitrary exception message enters this object. A window location
identifies the entire requested schema interval, not an inferred precise failing
byte within a short read. `reader_state` uses offset/width `[0,0]` as a metadata
anchor and does not claim that a particular encoded field failed.

| Fixed field | Declared source location |
| --- | --- |
| `signature`, `variant_marker` | `[0,8)` read; unknown signature `[0,4)` or HN marker `[4,8)` |
| `prefix` | Profile prefix interval listed above |
| `page_count` | C8 `[8,12)`; HN `[0x90,0x94)` |
| `outline_count` | HN-A `[0x158,0x15c)` |
| `page_index` | Checked index start and `20 * page_count` window |
| `text_offset`, `text_length`, `image_count` | Source row start +0/4 bytes, +4/4 bytes, +8/2 bytes |
| `text_span` | Row start +0/8 bytes declaring the offset/length pair |
| `outline_record` | `[0x15c + 308*r, 0x15c + 308*(r+1))`, zero-based `r` |

Reasons are the fixed enums `UNSUPPORTED_SIGNATURE`, `UNSUPPORTED_MARKER`,
`INVALID_RANGE`, `OUTSIDE_SOURCE`, `PINNED_COUNT_MISMATCH`,
`DISCOVERY_WINDOW_MISMATCH`, `IO_ERROR`, `CANCELLED`, `READ_REFUSED`,
`NO_PROGRESS`, `OVERREPORTED_READ`, `REQUEST_LIMIT`, `INPUT_CHANGED`,
`DEADLINE` and `RSS_LIMIT`. `READ_REFUSED` leaves a nonclassified reader/control
failure unknown rather than parsing its exception text. Unknown signatures or
markers also increment required unsupported/failure counts; they never pass.
Each selected physical page count and issue-21's discovery count/window must
agree with its pinned declaration before PDF queries or title-window discovery.

### Two independently run PDF parsers

Original runtime fixtures have established that `mutool show ... outline`
is lossy: `/XYZ null null 0` becomes a navigation URI with `zoom=100,nan,nan`,
and other view coordinates are transformed. It is **not** the comparison
oracle. The exact two-query protocol for each pinned PDF is instead:

```text
qpdf --json --json-key=outlines --json-stream-data=none <held-pdf>
mutool show -g <held-pdf> pages trailer/Root/Outlines <outline-object-ids...>
```

The first query discovers at most 512 unique, positive generation-zero outline
object IDs in preorder. The second uses that exact list once, with total argv
at most 16 KiB. `pages` independently discovers sequential emitted page IDs;
`trailer/Root/Outlines` independently discovers the root. It does not accept
qpdf's resolved page indices as MuPDF's page map. Selected objects must be
non-stream dictionaries; `-g` is mandatory, and the original controls prove a
stream is displayed only as a marker that is refused. An absent outline root
must appear as `null` with no discovered outline objects.

Run each query once on each of the six PDFs: exactly 12 planned outline
queries. Preserve capped raw stdout externally, never in Git or public logs.
Reject tool warnings (including page-tree repair), unexpected schema,
malformed Unicode, missing linked children/siblings, cyclic or repeated
objects, unresolved/non-page destinations and parser disagreement. There is
no extra private object query or retry when a schema fails.

qpdf's exact profile is JSON version 2, `parameters.decodelevel=generalized`,
and `outlines` nodes with exactly `dest`, `destpageposfrom1`, `kids`, `object`,
`open`, `title`. The original parser bounds JSON bytes/string/literal/nesting
before allocation, rejects duplicate keys and nonfinite numbers, and retains
finite reals as `Decimal`, never binary float. MuPDF's original small parser
accepts only the bounded single-line object-display value grammar needed by
these queries; it is not a general PDF file parser or stream decoder. It
validates `First/Last/Next/Prev/Parent`, complete selected-object coverage,
preorder, unique pages and exact referenced targets independently.

The observed title profile supports direct ASCII or BOM UTF-16BE PDF strings.
Unmeasured extended PDFDocEncoding or indirect title values are explicit
unsupported failures, not guessed Unicode. Source encoding remains unknown.
Direct explicit destinations support `Fit`, `XYZ`, `FitH`, `FitV`, `FitR`,
`FitB`, `FitBH`, `FitBV`, plus a direct action with exactly `S=GoTo` and `D`.
Named/indirect/remote actions and unresolved views fail. Null and exact numeric
parameters remain distinct; mathematical equal numeric spellings are
canonicalized without precision loss. No destination is changed into `/Fit`.

The common ordered fingerprint canonically hashes `{ordinal, depth,
title_utf8_sha256, title_utf8_bytes, resolved_pdf_page_index, destination_kind,
destination_parameters}`. Title whitespace, empty strings, tabs, line breaks,
CJK and supplementary Unicode remain exact; no normalization or trimming is
allowed. The fingerprint is a new explicit Stage A schema, not an assertion
that a different historical fingerprint format is identical. Repeat PDFs must
agree on the entire list. Their page counts must also match the hash-pinned
public layout maps (68/7/2), independently of source page counts.

Public summaries contain only lengths/hashes, depths, physical/resolved page
numbers, numeric destination facts and exact counters. Title strings, byte
strings and raw command output remain in the bounded external session.

### Predeclared field hypotheses; no CAJ offset assumption

The finite enumeration is `308-byte-numeric-and-terminated-title-v1`:

1. For each record, visit all starts 0..307, and for each start the fixed
   ordered codec list UTF-8, GB18030, UTF-16LE, UTF-16BE: exactly 1,232 attempts.
   UTF-8/GB18030 end at the first zero byte, UTF-16 at the first two-byte zero
   aligned relative to that start. A missing terminator ends at byte 308;
   odd/truncated or invalid sequences fail strict decoding. No BOM stripping,
   replacement, whitespace trim, NFKC or dropped record is allowed. ASCII is
   an ambiguous UTF-8 subset, not a fifth codec or a proved source encoding.
2. Inventory every 16/32-bit position, LE/BE, signed/unsigned: exactly
   `(307+305)*4 = 2,448` fixed candidates per record. Then visit maximal ASCII
   digit runs of 1..10 bytes, never sub-runs or whitespace/sign parsing. There
   are at most 308 additional attempts per record; longer runs are ignored.
3. Score title candidates by valid decoding and any reference-title hash
   equality. Only when the independently parsed count equals the record count
   is ordinal alignment an explicitly labeled hypothesis. Numeric candidates
   score source-page/depth ranges, and (only under that alignment hypothesis)
   one-based page, zero-based depth, or one-based level equality.
4. Sort title candidates by ordered matches descending, any matches descending,
   offset ascending, codec ascending. Sort numbers by maximum ordered page/
   depth/level matches descending, page-range count descending, then offset,
   width, order, signedness ascending. Publish at most the first 16 in each
   category, with full attempted/total counts. Each record publishes only
   offset/length/SHA/entropy/NUL counts and its first 16 any-title matches in
   enumeration order, plus the complete match count.

Issue-21 alone supplies 52 streamed records, 64,064 title attempts and 127,296
fixed numeric attempts plus the bounded ASCII runs. Only one raw 308-byte
record and its strict decoded scratch (at most 1,232 UTF-8 bytes) need survive
the next source read. Small bounded score dictionaries and metadata survive;
raw record/title arrays do not. CPython's public installed strict codecs are
initialized and their provider files hashed before private audit and bound in
the receipt; they are diagnostic hypotheses, not an adopted HN decoder.

No offset, encoding or numerical field is selected by this phase. Candidate
correlation is retrospective discovery, not independent validation. Field
length, terminator, padding, signedness, root/depth origin, parent IDs, order
and page numbering stay empirical questions. CAJ's different 256-byte title,
ASCII page and level fields provide no HN evidence.

If several encodings fit the observed sequence class, document the exact
ambiguity. GBK-compatible two-byte titles do not by themselves prove full
GB18030 or its four-byte/version semantics. If no outline-containing PDF or
unambiguous title/hierarchy/destination oracle exists, stop with a precise
#119/#10 blocker. Three zero-outline PDFs cannot satisfy positive title parity.

### Bounds and receipt contract

Implemented ceilings are refusal limits, not allocator or total-system RSS
guarantees:

| Resource | Stage A ceiling |
| --- | ---: |
| Source/PDF file | 128 MiB each; exact declared hashes/sizes also required |
| Shared opaque audit/hash reads | 1 GiB, requested bytes including failed calls |
| Logical header/index/record reads | 1 MiB total; requests 4,096/308 bytes as above |
| Declared page rows | 256 per source; overflow/negative/unknown profile refuses |
| Outline records | 512 per source; issue-21's observed count must stay 52 |
| Outline depth / raw title / decoded UTF-8 | 64 / 308 / 1,232 bytes |
| Individual query stdout / stderr | 1 MiB / 32 KiB |
| Sanitized report / receipt | 1 MiB each, same-read pinned JSON |
| Fresh external session | 16 MiB aggregate, 1 MiB per artifact |
| Child lifetime | 45 seconds, 1 GiB virtual address space, 256 MiB RSS |
| Adapter RSS / elapsed `run()` | 256 MiB / 600 seconds through final report persistence |
| Child launches / runner | 24 / 1, aggregate 25 maximum |

The depth limit is maximum zero-based reported node depth 64 (at most 65
active root-to-leaf nodes). Empty child traversal after that leaf is allowed;
a depth-65 node is refused by both independently traversed hierarchies.

The 24-child plan includes 12 outline queries, six before/after public version
commands and six before/after startup-library probes for Python/qpdf/MuPDF.
Reserve six closing slots; renders/converters/native/vendor counts stay zero.
A planned full-length-read multiplicity estimate for the preserved first
protocol and its approved runtime was 1,014,697,375 requested bytes.
It includes every setup/pre/post public/code/tool/codec/plan hash, four
startup-library hashes, two full source/six-PDF audits, three prior-report
hashes and a maximum-size receipt. It uses the inherited source/PDF sizes;
this preparation did not open or stat those files. The estimate assumes each
successful `pread` supplies its full requested length, plus the declared EOF
checks. It leaves 59,044,449 bytes below the 1 GiB ceiling. Successful short
reads can increase charged requests; this estimate is not an unconditional
upper bound. The strict actual requested-byte guard governs every attempt,
including failed reads. Exhaustion is FAIL with unverified remaining audits,
never a partial verification pass. New runtime/library/code/input identities
require a new calculation and review, never silently increased I/O. The
earlier focused-test estimate, including its additional loaded generator,
was 1,014,723,214 bytes under the same full-length-read assumption.
For this amended source/test pair, the original stat-only control includes
all loaded helpers, the extra original generator, a maximum-size 256 KiB
protocol and a maximum-size receipt. Full test discovery loads further original
helpers, so its estimate exceeds the focused invocation's ten-helper CLI
superset. Both estimates for the reviewed runtime fit a rounded planning
allowance of **1,017,000,000 requested bytes** (56,741,824 bytes below 1 GiB).
The test asserts the declared 1 GiB cap, not a different limit for other active
public-test runtimes. It opens/stats only public code, tools, codecs
and libraries; source/PDF sizes are inherited declarations. Final amended
runtime/protocol/code identities still need separate review and preflight
before execution; the old external manifest/preflight does not enforce new
pins automatically. The strict actual guard, including successful short reads,
remains decisive.
Do not count provenance probes as outline queries or duplicate renderer subsets.
Original pre-freeze self-tests are separate public work, not compatibility.

Create a fresh 0700 session under `/home/hzc/.cache`, never a heavy `/tmp`
artifact. Pin current code/protocol/argv/public metadata/reference report,
effective environment, tools, canonical startup libraries and expected input
identities in a pre-private-audit immutable receipt. Historical tool clues
from the existing report are qpdf12.2.0 SHA
`30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792`,
MuPDF1.25.1 SHA
`b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7`,
and Python3.13.5 SHA
`889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9`.
These exact executable pins and public version profiles have also passed the
original-only before/after controls. They do not imply a private query has run.
Missing or changed runtime pins require reviewed public preparation, never
silent fallback. The canonical `ldd` executable and discovered libraries are
frozen in the receipt; library and codec-provider identities are re-audited.

Enforce caps over the child's entire lifetime including closed stdout/stderr;
kill its process group on every failure and reap it. Preserve actual argv,
attempt/completion/failure, exit/timeout, stdout/stderr sizes/hashes, nullable
wait4 RSS and separately sampled RSS. Keep before/after source/PDF/code/tool/
environment/library audits even on first failure, with honest unverified
remaining work when a cap prevents an audit. No retries or erased failed
sessions. NOT_RUN without inputs means zero private queries/audits/passes;
an explicitly supplied missing input is FAIL.

## 3. Stage B: freeze a rule, validation and one-field controls separately

Stage A is retrospective discovery. It is not independent validation of a
chosen field rule. Before Stage B, publish the candidate offsets/widths,
record/title bounds, codec/version and expected semantics; freeze exact
copies, changed byte spans, full-source hashes, commands, counts and predicted
outline effects in a new reviewed protocol.

Held-out public source candidates are issue-29 (48 records; source SHA
`ede5eddb0e8ec1dea46c32a06e16ac12b874a141ca2d6669736495d2ac549261`,
10,155,625 bytes, 48 physical pages) and issue-69@57a3c60e1d86 (111 records;
SHA `57a3c60e1d8639955c625452398d2c8a32a46a51b815a44cc9a5e0351fcb8ef4`,
6,685,012 bytes, 81 pages). Their HN-A marker/count/span must be freshly
confirmed; historical `show` counts alone are not field proof. Obtain a
positive independently parsed baseline before claiming validation on either.
No unplanned new converter call is authorized by this draft.

Once exact fields are known, the smallest useful control set is:

1. One title-field-only original ASCII/Unicode replacement at the same record,
   including independently encoded two-byte and four-byte candidates; all
   hierarchy/destination fields, index, page/text/image spans remain identical.
2. One destination-field-only change to a distinct already emitted physical
   page; only that entry's resolved destination may change.
3. One hierarchy-field-only legal sibling/child change whose descendant impact
   is predicted explicitly; no title/destination changes.
4. A separately declared empty-title, parent-gap and out-of-range destination
   negative set; expected failures versus reference deviations declared in
   advance, never auto-blessed.
5. Omitted-destination behavior only on an independently identified applicable
   HN-B outline field (or a separately declared original structural fixture).
   Source `[1,6]` does not justify retargeting `[2,3,4,5]` to a neighbor. If
   applicability cannot be established, HN-B's source-outline behavior remains
   unsupported/unknown; fixture policy is not private-format proof.

Before mutation, establish any framing/checksum that covers the record.
Single-field means a valid logical field change with an exact bounded byte
diff, never truncation, arbitrary donor prefixes, hidden checksum damage or
unrelated padding. Repeated conversions and any original control outputs stay
external. Freeze a small initial batch, not a speculative sweep; each failed
attempt is retained and a changed plan requires a new receipt.

## 4. Smallest maintainable original native architecture

Implement only a proven measured profile; unknown C8/HN-B applicability must
not become a fabricated zero count. No source/PDF fingerprint enters parsing
or destination selection.

1. A child `hnc8/outline.rs` reads one fixed-width record and yields a borrowed
   source event such as `SourceOutlineEntry { depth, title: &str,
   source_page_number }` through a backpressure-aware async visitor. Source
   page numbering is explicitly one-based, never `Bookmark.page_index`.
   Keep outline count/span metadata separate and revalidate public Header/Span,
   actual signature/count, index arithmetic and source containment. Extend a
   located error with record ordinal/absolute encoded field offset; do not
   report a decoded Unicode index as an absolute byte offset.
2. Reuse original decoder logic only once the observations justify its exact
   encoding profile. The current CAJ parser retains all `Bookmark`s and its
   private GB18030 decoder has infallible String growth. Reuse neither behavior
   as the HN architecture. Factor a private crate-visible strict bounded decode
   primitive with fallible exact reservation or a checked output buffer; no
   external implementation/table copying and no unnecessary public codec API.
3. An explicit outline budget checks count, depth, raw title bytes, decoded
   UTF-8 capacity, retained stack capacity and total visit work. Validate
   preorder parent/depth transitions using only the bounded active ancestor
   stack. If the independently observed layout requires arbitrary unresolved
   parent IDs/order, refuse that profile or revisit the O(depth) contract
   openly rather than secretly allocating every title/parent.
4. The composer optionally allocates a fallible, separately bounded
   `source_page -> Option<zero_based_pdf_index>` array. Charge checked
   `page_count * size_of(element)` and actual capacity before allocation.
   Record every no-image row as None and every actual emitted page after
   `add_placed_page` succeeds. Do not silently reuse a preceding output index.
5. After successful page traversal and before its currently immediate
   `PdfDocument::finish`, stream the source outline into a small map adapter.
   It makes one bounded fallible owned title for the existing
   `BookmarkVisitor`/`PdfDocument::add_bookmark`. Keep source parser and PDF
   emission separate. `ComposeOptions` may gain an explicit optional outline
   mode with default disabled to preserve current image-only behavior; settle
   this small v0 API change after evidence and document any source-breaking
   option-field addition. Avoid duplicating the composition engine.
6. Initially proposed omitted policy is **reject** with a located error;
   its final semantics require Stage B evidence/review. Invalid source pages,
   empty titles, unsupported destination views and unresolved parents likewise
   require an explicit policy, not automatic correction. A partial outline/
   source/visitor/sink/cancellation failure invalidates the PDF; caller must
   discard it. Emit only after destination page objects exist.

`PdfDocument::add_bookmark` already checks emitted destinations, count,
parent gaps, fallible stack reservation and retained title capacities. It
streams closed siblings/ancestors and writes UTF-16BE hex through fixed 4,096
byte scratch. Its `/Fit` target, parent-gap policy, and proposed HN empty-title/depth policy
still need alignment with measured semantics; the HN reader must enforce its own title/depth caps.

O(depth) applies to retained outline titles/links, not the whole converter.
The explicit source-output map is O(source pages); the existing PDF writer
also retains O(output pages) page IDs and O(objects) xref state. Track each
managed buffer/index separately and measure RSS independently. No all-title
vector, whole-document Vec API, hidden forward-input buffering or platform
adapter dependency is required. Ranged source and sequential sink remain
compatible with browser/Node adapters and forward-input scratch spooling.

## 5. Implementation gates and present blockers

Before implementation: a positive independent outline oracle and exact HN-A
title/encoding/hierarchy/destination evidence; explicit C8/HN-B applicability
status; reviewed omitted-page and destination-view policy; no unplanned
conversion needed to compensate for an empty reference.

Original MIT controls must cover Unicode/empty/whitespace/supplementary titles,
nested/sibling/root transitions and repeated targets, parent gaps/cycles,
wrong source page vs PDF index, omitted destinations, malformed/truncated
encoding/record/count/range, overflow/fabricated metadata, short/zero/overread
and raised I/O, cancellation before/after reads/visits, title/depth/count/map/
allocation/visitor/sink refusal and finalization. Independent qpdf and MuPDF
checks on original PDFs must resolve hierarchy and actual page objects, not
merely compare serialized implementation fields.

Native integration must preserve all #117 streams/order/boxes/full page pixels
and explicit no-image mapping, and all selected-image/CLI/JS gates. New outline
support is not new HN OCR/searchable-text support or a CAJViewer compatibility
claim. Full-parent #10 and release #14, plus #30/#44 routing, remain gated.
Require independent correctness/provenance and simplification review, hosted
native/WASM/MIT gates and exact per-file 100% Rust source-line LCOV. Optional
corpus skips/unknowns/zero-title-only references never count as title parity.

**Current unmet criteria:** source title layout/encoding/hierarchy/destination
unknown; output outline oracle unobserved; C8/HN-B outline applicability and
omitted-destination semantics unknown. No private observation, core outline-field implementation, new converter,
or compatibility pass has occurred in this preparation task.

## Primary tool reference

The [qpdf12.2 JSON documentation](https://qpdf.readthedocs.io/en/12.2/json.html)
supports explicit stream omission, filtered JSON sections, strict Unicode vs
binary string distinction, one-based resolved page positions and the page-tree
repair caveat. It documents tool semantics, not HN fields. The adapter must
validate its pinned command on original fixtures before private execution.


## Adapter, preparation controls and execution gate

The original MIT adapter is
[`scripts/hnc8_outline_observation.py`](../scripts/hnc8_outline_observation.py);
its original runtime controls are
[`test_hnc8_outline_observation.py`](../tests/conformance/test_hnc8_outline_observation.py).
No Rust or production conversion API changes in this preparation.

The original tests deliberately use the active installed public tool profile;
only their stand-in orchestration patches those pins. Private execution still
requires the fixed approved tools and refuses a different runtime.

The tests generate two-page PDFs with Unicode/whitespace/empty nested titles,
several explicit destination kinds, nullable and exact decimal parameters,
and direct GoTo actions. Actual qpdf/MuPDF commands prove the common schema,
raw stream refusal, independent page-order disagreement and lossy display
behavior. Other controls prove missing child/sibling/link refusal, strict
source-window enumeration on invented non-CAJ field positions, no-follow held
input descriptors, real FIFO/socket/directory nonblocking refusal in a bounded
separate Python process, read/error/cancel accounting, stdout/timeout/closed-pipe
lifetime limits, exhaustive final audits and post-persistence invalidation.
Original max-depth 64/65 and actual indirect-title refusal controls also
prove the declared depth and unsupported counters. Full original orchestration
covers all six stand-in PDFs with real parser
queries, immutable receipt-before-source-read order, failure/remaining counts
and a changed-after-audit refusal. Those synthetic source counts and tool
configuration patches are explicitly public controls, never HN evidence.

Run original preparation only with:

```sh
python3 tests/conformance/test_hnc8_outline_observation.py -v
python3 scripts/hnc8_outline_observation.py
```

No arguments returns `NOT_RUN`, all launch and progress counts zero, no input
or output file work. Explicit incomplete inputs return `FAIL` before reads or
children. A complete invocation whose plan is still DRAFT reads only that
public plan and refuses before session creation or any private input. An
explicit missing private input under a frozen reviewed contract is `FAIL`,
with honest actual attempts, final audits and remaining work, never a skip.

The exact root invocation after approval will supply `--corpus-dir`,
`--reference-report`, `--artifact-root`, `--plan`, `--plan-sha256`, and both
`--hn-a-pdf-1/2`, `--c8-pdf-1/2`, `--hn-b-pdf-1/2`. These actual resolved paths,
expected source/report/PDF identities, tool paths, effective child environment,
startup versions/libraries and code pins enter a fresh mode-0400 receipt
before the first private-file audit. Full receipt/path/environment details
remain external. All input components are traversed via held POSIX directory
FDs with `O_NOFOLLOW`; final leaves also use `O_NONBLOCK`, so a FIFO cannot
block before the regular-file check. Only regular files qualify. Pre-audit source/PDF FDs stay
open, and children read them through the declared Linux `/proc/<parent>/fd/N`
alias. There is no weaker portable fallback or mutable-path reopen for a query.
Opaque hashes use the same metered stream with exact EOF and fstat checks;
JSON parsing uses that same bounded retained hashed read.

The deadline measures `run()` from its first monotonic timestamp through final
report write/sealing. Interpreter startup/argument parsing and public
preparation are outside that scope. On crossing during persistence, any stored
PASS is removed and a bounded FAIL summary is retained if storage permits.
A failed audit/cap still attempts every remaining declared audit within the
same global budgets; it cannot reset time/read/disk ceilings. Child process
groups are killed on failure and their leader is reaped; leader wait4 peak RSS
and separately sampled leader RSS are recorded honestly, not a claimed summed
RSS of descendants. The Python adapter's VmHWM is separate. Missing RSS is
nullable, not an invented zero. The query stdout/stderr buffers, retained
artifacts and all generated reports share the 16 MiB session cap.

A successful Stage A is only completed discovery with a positive independently
parsed HN-A outline. `compatibility_status` stays `UNVERIFIED`; a zero-positive
result is `BLOCKED`, not a parity pass. Full #119 remains open until separately
frozen validation/controls establish the fields and original native
implementation passes its acceptance gates. Required unsupported schema,
malformed/ambiguous hierarchy or parser disagreement is FAIL. Historical and
vendor `NOT_RUN` evidence remains separate.

### Completed public preparation and external invocation bindings

The original adapter and controls were independently reviewed by the parent
and simplification reviewer before preparation PR
[#132](https://github.com/rwv/caj2pdf-rust/pull/132) merged. Each ran all 36
focused original tests successfully. The full original conformance suite
passed 451 tests, and seven original fixture checks passed. One focused
invocation made 63 process-launch attempts: 62 spawned public children and
one expected nonexistent-executable refusal. These are public controls,
not private compatibility or native-outline evidence.

All four hosted jobs passed on exact preparation head
`7d23f2780c9eb2bb2ce92b97540ef68510d13bb6` in
[run 36402968249](https://github.com/rwv/caj2pdf-rust/actions/runs/36402968249).
The downloaded LCOV payload was independently checked using all actual `DA`
records: 54 source files, 27,413 lines, all 27,413 covered, each file 100%,
with no exclusions. The artifact ZIP is 374,225 bytes, SHA-256
`055b688424a90b4bf32603e3053a6dafdb4465e50bda5c39e04fe838dc440470`;
its LCOV payload is 6,311,083 bytes, SHA-256
`70c0559e836dbb8ce4454c0dc5864d1b1cca4c2d94ba7df545f1051c261b4048`.
This is existing Rust line coverage, not native HN-outline implementation
or private-title parity. Raw LLVM `LF`/`LH` counters are separate and are not
the actual source-line denominator used by the repository gate.

The preserved first frozen-protocol worktree started at merged main
`fc0c0652f0e35be954589780bd938299415cf2f1` and changed only its protocol.
This amendment is in a separate worktree from `3e1860dec485d6d359c46c32393daec63e4caa8b`.
Only the adapter, original controls, this document and provenance note change.
The eight shared helper pins remain unchanged; amended adapter/test pins are
listed below. Nothing modifies the first worktree or its external artifacts.

External `public-preparation.json` is 39,923 bytes, SHA-256
`f38d32045baaacab6dea088ecd682993c42ca5f98c71bd33243721b2a1423847`.
It binds the ten code identities, installed tool/version/codec identities,
canonical startup libraries, inherited 27 source/six PDF paths and identities,
reference-report identity and proposed CLI input paths. All private paths
remain outside Git. Its inherited source/PDF pins are explicitly
`INHERITED_PIN_NOT_REAUDITED`; no new source/PDF open, stat, field read or
query was performed. The passing preparation recovered those declarations
from the existing authorized reference JSON through one bounded same-read
hash check.

Public preparation has two preserved attempts. The first completed 12 public
version/startup-library probes, then failed while serializing a parent
environment without `TZ`; it is retained as a 367-byte failure record,
SHA-256 `e2c5568b56916dc17996d1e4b6816b5eae45c2ce7f55299b136fae4b0aa2f1ef`.
The corrected public-only metadata adapter then completed 12 probes and
wrote the passing metadata. Thus preparation made 24 successful public
child probes in total. They are outside the future phase's 24-child plan;
private source/PDF observations and converters/native/vendor calls stayed
zero. No failed private run was retried or erased.

The declared executable paths are `/usr/bin/python3.13`, `/usr/bin/qpdf`,
`/usr/bin/mutool` and `/usr/bin/ldd`. Public preparation independently pinned
`ldd` at 5,356 bytes, SHA-256
`7bcd61a279946cb376d36fd91f4938ab4f4aba48e793c652a9797319d23f8dd3`;
the other three fixed hashes appear in the bounds contract above. Its
before/after version text and startup-library identities agreed exactly.
Codec-provider identities were hashed once for public preparation. Their
identities were then verified unchanged in the closed first phase; no amended
runtime audit has run. The preserved external input/argv manifest bound those
observations to the first protocol hash. It has not been rewritten for this
amendment. No new public preflight or private file observation is part of this
amendment's preparation; a later frozen phase needs separately reviewed current
bindings.

Parent environment and preview child environment fingerprints are distinct.
The preview used a public preparation session's `TMPDIR`; it is not the
future private session's environment. Parent `TZ` may be absent, while the
child's declared `TZ` is `UTC`. The actual run inherits the invoking
environment and overrides `LC_ALL=C`, `TZ=UTC`, `PYTHONHASHSEED=0`,
`PYTHONDONTWRITEBYTECODE=1`, `PYTHONNOUSERSITE=1` and its fresh scratch
`TMPDIR`. The immutable runtime receipt binds the actual environment and
canonical libraries after public startup preparation and **before private
file audits**. The preview is not evidence that a future runtime stayed
unchanged; final runtime audits must establish that separately.

The parent must separately review this amended document, new executable pins
and proposed external input/runtime bindings before freezing and committing a
new protocol and authorizing one direct invocation. That invocation uses the original six
legacy reference PDFs, including the original HN-B PDF, never the corrected
Gray image reference. Stage A and all full #119 implementation/parity
criteria remain unmet. The first FAIL is preserved; amended observations and
all native title/encoding/hierarchy/destination implementation remain NOT_RUN.

### Amended executable contract (DRAFT; no execution authorized)

Public preparation for this amendment passed **42/42 focused original tests**,
**457/457 full conformance tests**, seven original fixture tests and the fixture
generator's `--check`. The focused invocation was independently instrumented
at the subprocess boundary: **75 launch attempts, 74 spawned original public
children and one expected nonexistent-executable refusal**. Its estimated
full-length-read opaque work was 1,014,796,600 bytes; full test discovery's
additional loaded helpers gave 1,016,430,265 bytes. Neither is a private audit
or a guarantee for short reads. These controls are separate from the preserved
first phase's actual 12 children + one runner and from the amended 24-child plan.

The controls verify all 27 original source identities while refusing any field
read on 24 intentionally malformed excluded originals. They also verify fatal
selected row/span/signature/count failures despite historical error labels,
fatal excluded identity mismatch, fixed reason/location metadata, read/error/
cancel accounting, record-ordinal discovery failure, closing audits and actual
no-input zero work. One earlier full-suite run failed an overly narrow planning
assertion because test discovery loaded more helpers; its external log is
preserved. The artificial assertion was removed, with the unchanged actual
1 GiB guard retained. No amended private source/PDF open/stat, preflight, query,
converter, native, vendor or application phase ran during this preparation.
Independent final review, commit/freeze, current runtime bindings and execution
authorization remain pending. Full native #119 criteria remain unmet.

The source pins below propose the amended complete CLI-loaded original
module/test list for independent review. The adapter checks this contract and
rejects a different source topology. The final frozen protocol must be committed and
execution explicitly authorized before root's one invocation. A hash or pin
change requires reviewed public preparation, never a silent retry. Schema v2
binds only the three selected source field profiles while retaining every
source/PDF integrity audit, plus the first FAIL's exact immutable identity and
counts. Selected observation failures remain fatal.

<!-- execution-contract -->
```json
{
  "code_sha256": {
    "scripts/hnc8_layout_pdf.py": "5f920335514872b0a1a618bbfef4bb3d0c830ccdf9621544951b608ee42871d2",
    "scripts/hnc8_layout_reference.py": "7c3725394d999d391e770fabb1a91949a59987bd29d2f53b5fb067ddb9ad17fd",
    "scripts/hnc8_layout_source.py": "f6e150fe905ca9bb7d0079fcda6fc65824eef7bbcc60ded71b3540d5040f31dc",
    "scripts/hnc8_outline_observation.py": "e3aca1d0d2756c886dbaf908ea71b8eee7b4f76d654d68d84a17fffbdd75bc96",
    "scripts/hnc8_page_composition.py": "c9ec94a1577a694a19449ce208c0c4facb6fd1179f523f698cb142d267a6208b",
    "scripts/hnc8_placement_analysis.py": "e6d03d78b26443749d96cf20a690ff87b3e7878e2b5d8aadd1322af201fcb94a",
    "scripts/hnc8_placement_probe.py": "f438ba3026a96a14cd88298c731fc89b7f5471d8dc5fde4e48fb8cdc36f4f50d",
    "scripts/hnc8_placement_rule.py": "419fe68d0547f2f6ff8bc55b26daaac7bec7843e6da51441c63afa92d402bf3b",
    "scripts/hnc8_text_frame.py": "63363fc6591bc699bda0bc97897d901b25a610f6a9bb2e059ca58d4ad33edac7",
    "tests/conformance/test_hnc8_outline_observation.py": "1622f2fb1887d8be23e67335793425c13d4a3776df8dbeda7abaac1bc6001644"
  },
  "enumeration": "308-byte-numeric-and-terminated-title-v1",
  "preserved_failure": {
    "cause_status": "UNKNOWN",
    "identity_audits": {
      "after_verified": 93,
      "before_verified": 93,
      "status": "PASS"
    },
    "launches": {
      "aggregate": 13,
      "converter": 0,
      "native": 0,
      "render": 0,
      "runner": 1,
      "validator_children": 12,
      "vendor": 0
    },
    "report": {
      "sha256": "e8660ffd526b06ddecf30c59e242d88ec5002f3fd8350e748815ffef221212f9",
      "size_bytes": 41723
    },
    "source_progress": {
      "attempted": 2,
      "completed": 1,
      "failed": 1,
      "planned": 27,
      "remaining": 25,
      "unsupported": 0
    },
    "status": "FAIL",
    "unstarted": {
      "discovery": 1,
      "pdfs": 6,
      "queries": 12
    }
  },
  "queries": "qpdf-outlines+mutool-g-objects",
  "schema_version": 2,
  "source_scope": {
    "audit_source_count": 27,
    "discovery_source_sha256": "33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4",
    "field_source_sha256": {
      "c8": "35951c3775790c230c84e4312e8db7a57ca2980a04d35ff03d328806df7d622c",
      "hn_a": "33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4",
      "hn_b": "e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49"
    },
    "mode": "three-pinned-layout-profiles-v1",
    "unselected_inventory_status": "OUT_OF_SCOPE",
    "unselected_semantic_status": "NOT_RUN"
  },
  "stage": "A"
}
```

The [MuPDF1.25 `mutool show` documentation](https://mupdf.readthedocs.io/en/1.25.0/mutool-show.html)
documents `-g`, object paths, page IDs and the display-outline command. These
public tool semantics and the original controls establish a finite observation
interface only; no external converter, PDF implementation or HN title decoder
was read or copied.

# PDF input and repair profile

Issue [#6](https://github.com/rwv/caj2pdf-rust/issues/6) adds reusable PDF
operations for the CAJ-family converters. The PDF reader accepts a stable
`PdfRange` in a `RangedSource`; the writer uses any `std::io::Write`. Platform
adapters own file handles, browser `Blob` slices, and any temporary spool for
forward-only input.
`Limits.max_input_bytes` applies to the selected PDF range or the sum of
fragment spans, rather than unrelated bytes in a containing CAJ file.

## Supported input

- PDF 1.7 syntax with ordinary indirect objects, classic cross-reference
  tables, and incremental revisions linked by `/Prev`. The bounded xref-stream
  subset accepts direct `/Size`, `/W`, `/Index`, and `/Length`, unfiltered or
  `/FlateDecode` data, and free, standalone or compressed in-use entries.
  The measured PNG Up profile accepts direct `/DecodeParms` with
  `/Predictor 12`, `/Colors 1`, `/BitsPerComponent 8`, and `/Columns` equal
  to the sum of `/W`. Each row must carry algorithm byte 2; prediction state
  continues across `/Index` subsections and resets for each xref stream. A
  later classic table may point back to an xref stream through `/Prev`.
  Linearized PDFs may link forward to the main xref section. Precedence follows
  the chain, not physical offsets; cycles, out-of-range links and chains longer
  than 64 sections fail. Only older trailers may omit `/Root`. A forward-linked
  input receives an ordinary incremental revision so stale linearization hints
  no longer describe the output; a validated Catalog copy suffices when no
  other object needs repair. Content streams are preserved.
- Direct or indirect stream `/Length`. In a complete PDF the xref resolves
  an indirect length, and `endstream` must follow the declared extent. A
  headerless CAJ fragment has no xref; its extent rule is described under
  [Stream extents in CAJ fragments](#stream-extents-in-caj-fragments).
  PDF-looking bytes inside a stream are payload once the extent is fixed;
  the reader does not re-parse them.
  Up to 64 PDF whitespace bytes may separate that fixed extent from the exact
  `endstream`/`endobj` tail (#409). This measured tolerance does not extend the
  payload, search past non-whitespace, or relax Length-repair width limits.
- Catalog and page-tree links, declared page counts, and bounded object and
  page indexes. Page geometry accepts direct rectangles or indirect scalar
  rectangle objects. Compressed metadata uses generation-zero, standalone
  `/ObjStm` containers with direct `/N`, `/First`, `/Length` and a single
  `/FlateDecode` filter. Member numbers, offsets, xref ordinals, syntax and
  live references are checked before structure traversal. The final xref
  revision determines membership, including later standalone replacements.
  Original stream bytes are retained. `PdfIndex::object_location` returns
  `UnsupportedFormat` for a compressed member, which has no standalone byte
  span; internal diagnostics identify the physical container.
- Object-stream encoded and decoded metadata each use the object-syntax cap
  `min(4 MiB, max_allocation_bytes / 32)` (plus one inflation sentinel);
  retained member bytes and the member index each have a separate
  `max_allocation_bytes / 8` cap. Reads, inflation and member walks observe
  cancellation. Input remains ranged and output sequential.
  Encrypted input, other prediction algorithms, filter arrays, object-stream
  `/DecodeParms`, `/Extends`, external streams, indirect object-stream lengths,
  compressed indirect stream-length values and unrecognized structural damage
  remain typed errors. This is the measured #402/#404 profile, not general
  object-stream or PNG-filter support.

Pass-through does not decode page content filters. The reader validates stream
framing and declared byte lengths, but it cannot establish that compressed
payloads decompress correctly. The independent validator checks the generated
and available corpus outputs exercised by tests; callers needing full payload
validation must use a PDF validator outside this conversion layer.

The reader recognizes narrowly observed repair cases: a complete PDF
followed by a recognized CAJ download footer, and identical duplicate
`/MediaBox` values in one `/Pages` dictionary. It verifies the original xref and object
graph before omitting the recognized footer. Unknown non-whitespace bytes
after EOF fail, including a truncated later incremental revision. It removes
the duplicate key by appending a replacement of that same indirect object
with one value. Conflicting values
or other duplicate keys are ambiguous and fail. The original page content
streams are not decoded or rewritten.

The measured length-framed `WebFastLoad` profile (#494) is also accepted:
two little-endian u32 lengths (decoded, encoded), one complete zlib stream,
and exact `APPINFOSIGN <offset>` with the absolute decimal position of the
length fields. The checksum and both lengths must agree. Metadata is discarded
through a 4 KiB scratch buffer with a 4 MiB decoded cap and cancellation checks;
it is not interpreted as PDF, and its URLs are not followed. Unknown framing
and arbitrary suffixes remain rejected.

Recognized footers include the existing `WebFastLoadP` and `WebFastLoadW`
profiles, exact `WebFastLoad`, and UTF-8 BOM plus `FileProperty` metadata,
optionally preceded by `WebFastLoad`. The metadata profile requires the four
plain-text leaves `Doi`, `FileName`, `TableName`, and `Type` in that order;
entities, declarations, nested fields and trailing data are not admitted.
All recognition uses the existing 64 KiB tail bound and retains the guard
against embedded PDF syntax. The encrypted `right-meta` samples and tails
containing HTML debris remain rejected. See #386 for the measured identities
and classification; accepting a footer does not waive PDF validation.

The KDH PDF-body profile also normalizes a `stream` keyword followed by one
carriage return. After validating the referenced object and its declared
stream length, the copy path changes that one CR byte to LF; byte offsets and
stream payloads are unchanged. A stale `/Parent` on a `/Page` can be replaced
only if the reference is the sole dangling reference in that page and the
validated page-tree `/Kids` traversal identifies a unique actual parent. The
replacement is an incremental object update. Short incomplete object prefixes
in one observed input are permitted in otherwise whitespace-only gaps only
when the named object is free or is the immediately following live object.
Their exact bytes are checked during copying and replaced by the same number
of spaces. A second measured profile (#410) permits a gap of at most 128 bytes
when its trimmed bytes are an exact proper prefix of a complete generation-zero
dictionary or unsigned integer selected by the validated xref, even when that
object is elsewhere in the file. The same bounded parser verifies the live
counterpart; stream objects, other scalars, missing/compressed targets, complete
objects and conflicting prefixes do not qualify. The earlier free/adjacent
orphan rule retains its 64-byte cap; both share the existing retained-patch
budget and copy-time source checks. Other gap content remains an error.
These repairs do not accept
unrelated dangling references, duplicate page-tree children, or cycles.

A Page's direct Resources/ExtGState dictionary may contain the measured single
pair of duplicate resource names when both references resolve through the live
xref to equivalent opacity-only dictionaries (#412). Both generation-zero target
objects must fit within 256 bytes and contain exactly direct CA/ca values in
[0, 1], using nonnegative decimal spellings (optional `+`, no minus/exponent,
at most 64 fractional places). Exact decimal comparison prevents rounded unequal
values from being treated as equivalent. The later resource pair is omitted in
an incremental Page revision; raw streams are unchanged. Additional duplicates,
other resource paths/fields, streams and unresolved/compressed targets stay
errors. Existing repair budgets and sequential copying apply.

Existing outlines accept direct `/Dest` arrays and direct local
`/A << /S /GoTo /D [...] >>` dictionaries, optionally with `/Type /Action`.
The destination must identify a page in the validated page tree. Actions are
preserved without execution. Byte-string destinations also accept the bounded
[name-tree profile](#named-local-destinations) below. Indirect action dictionaries,
other action types, extra/chained action fields and simultaneous `/Dest` and
`/A` remain unsupported or malformed. A missing sibling `/Prev` is derived
from the validated `/Next` chain. A `/Last` that points to the final descendant
instead of the final direct child is corrected after the subtree is checked.
Both links are combined in one incremental replacement per affected dictionary;
titles, ordering and destinations stay unchanged. Explicit contradictory links,
other `/Last` mismatches, cycles and repeated nodes fail. The bounded outline
walk retains references rather than all subtree dictionaries.

### Named local destinations

For a byte-string outline `/Dest` or local GoTo `/D`, the indexed reader resolves
an indirect Catalog `/Names` dictionary and indirect `/Dests` tree. The measured
profile has indirect dictionary nodes, direct `/Kids` reference arrays or direct
`/Names` key/value arrays, and indirect `/XYZ` destination arrays. Root nodes
contain only Kids or Names; other nodes additionally require two-string Limits.
Each XYZ array must identify a live page followed by exactly three finite numeric
or null arguments. Literal/hexadecimal keys are compared as decoded bytes, without
Unicode normalization. Other named destination representations and views remain
unsupported; existing direct outline-array behavior is unchanged.

The tree is checked once in lexical order. Duplicate keys, overlapping or reversed
child ranges, incorrect Limits, empty nodes, cycles/shared nodes and missing keys
fail. The walk uses the existing 64-level syntax-depth ceiling and one visited byte
per admitted xref slot. The destination index, traversal stack and cumulative
reserved name/Limit bytes each have an allocation-derived budget of
`max_allocation_bytes / 8`; cumulative loaded destination metadata is capped by
`max_allocation_bytes`. Existing object-syntax, xref and cancellation limits still
apply. Named outline lookups then use binary search. Metadata may reside in the
existing supported object-stream profile.

No name tree, action, page or destination is rewritten. A clean PDF remains
byte-identical under ranged input and sequential copying. As with the existing
reader, callers must keep the source stable throughout inspection and copying.

The KDH wrapper admits the measured `01 00 00 00` field at offset `0x28` as
well as `00 00 02 00`, retaining the signature, offset-254 XOR and complete PDF
framing checks. Stream-head reads request lookahead when a CR is the last byte
of a buffer, so a split CRLF does not shift the declared payload extent. See
#387, #393 and #394 for the external sample identities and validation scope.

CAJ fragment reconstruction also normalizes exactly two identical direct
`/MediaBox` values on a non-stream `/Page` or `/Pages` dictionary. The second
key/value pair becomes equal-length whitespace while the object is copied;
page identities, all other dictionary entries and stream payloads stay intact.
The fragment index retains only the bounded pair offsets. Conflicting or
invalid boxes, a third occurrence, other duplicate keys, indirect repeated
boxes and stream dictionaries keep their ambiguity errors. This extends the
measured complete-PDF geometry repair to the separate fragment path (#407).

CAJ files that contain indirect object fragments but lack a complete PDF
header/xref use `FragmentPlan`: the CAJ format handler supplies complete,
nonoverlapping object byte spans and explicit page order. Reconstruction
validates those spans before writing a new PDF header and xref. It synthesizes
a missing Catalog or single missing Pages root only when the page-parent links
make the result unambiguous. A fragment Catalog with an existing `/Outlines`
link is rejected before output because this reconstruction path does not
validate an existing outline tree. The format handler can import independently
parsed CAJ bookmarks with `PdfOutlineAppender` after reconstruction. Indirect
fragment `/MediaBox` objects are also unsupported until the fragment plan can
validate their geometry. CAJ header, page-table, and TOC extraction belong
to [issue #7](https://github.com/rwv/caj2pdf-rust/issues/7); no end-to-end CAJ
compatibility is inferred from this PDF-layer test alone.

The CAJ converter scans the fragment once (issue
[#359](https://github.com/rwv/caj2pdf-rust/issues/359)). Each object's
inspection (span, role, references, page parent and whether it has a
`/MediaBox`) is carried through page-tree synthesis, link repair and
reconstruction instead of parsing the object again. Link repair reads only
the objects it rewrites. Generated objects, such as a synthesized `/Pages`
node, a repaired link or a blank page, are inspected from their own bytes.
Reconstruction checks span overlap and duplicate object numbers in one
place. Its memory is bounded by the object, page and bookmark counts
(`MAX_PDF_OBJECTS`, `Limits::max_pages`, `Limits::max_bookmarks`) and by
`Limits::max_allocation_bytes` on each buffer, not by a summed byte budget.

CAJ link repair also handles one measured absent optional appearance (#417).
A complete non-stream Link with a direct destination to a retained page may
drop its `/AP << /N absent 0 R >>` pair only when that is its sole missing
reference, the reference occurs only there, and `/BS << /W 0 >>` is the complete
border-style dictionary. The destination and all other bytes survive. Live
appearances, named states, rollover/down entries, indirect appearance maps,
other border profiles, actions, nonzero generations and unrelated missing
references do not qualify. Ordinary indexed-PDF validation remains unchanged.
The existing bounded object reads, source recheck, retained-repair budget and
sequential CAJ reconstruction apply to this narrow normalization.

## Stream extents in CAJ fragments

The fragment scanner never decodes a stream payload to frame it:

1. A direct `/Length`, or an indirect one whose integer object was already
   scanned, is trusted when `endstream` and `endobj` follow it.
2. An indirect `/Length` whose integer object comes later is resolved by a
   forward search: the stream provisionally ends at the first `endstream`
   with a complete tail. The integer object must frame the stream at the
   same end; if it frames another end, the scan repeats with that length.
   If the scan fails while a provisional end is still unconfirmed, it
   repeats with the search moved past that `endstream`. At most 16 repeats
   are made; then the first error stands. A length whose integer object
   never appears is an error. Partial (`--allow-damaged`) and page-row
   candidate scans do not repeat.
3. Otherwise the stream extent is not confirmed, and recovery decides.

## Damaged-input recovery

All recovery rules sit behind one entry, `try_recover` in
`pdf/input/recovery.rs`, which the scanner calls when an object does not
parse or its stream extent is not confirmed. A rule either resumes at an
independently derived boundary, or defers the interrupted bytes until the
complete scan proves them an exact proper prefix of the indexed object with
the same number. A deferred prefix without that proof is an error.

- An interrupted stream followed by its complete replay: an `endstream`
  after the interrupted stream, less at most two end-of-line bytes and the
  declared or resolved `/Length`, fixes where the replay starts, at most
  64 KiB later. The replay must repeat the stream header, and the
  interrupted bytes must share at least one payload byte with it. Complete
  objects may sit between the two; exact duplicates are kept once.
- An understated direct `/Length`: the one complete terminator within 64
  bytes of the declared end is accepted, and the `/Length` digits are
  replaced in the copy by a same-width value. A final stream repaired this
  way must be the only terminator left in the scanned bytes.
- A syntax interruption: an exact prefix of an earlier object, of an integer
  replay, of a later copy offered by a page-table row, or a short cut at the
  parser's error boundary, under the bounds the rules document in code.

With `--allow-damaged` (below), a failure no rule recovers is recorded and
the scan resumes after an independently framed stream or at the page
table's next page dictionary.

### Nested duplicate empty Forms

The measured #439 CAJ profile contains a complete empty Form immediately at
another empty Form's declared data boundary. Both must use the same object
number, generation zero, direct Length zero, and exactly Type, Subtype, Length,
BBox and Matrix keys. Type/Subtype must be XObject/Form. All four BBox and six
Matrix numeric tokens must match exactly; floating-point normalization is not
used. Each header is limited to 256 bytes. Two consecutive complete
endstream/endobj tails are required, with at most 64 PDF whitespace bytes
before each keyword. No payload search or recursive recovery is performed.

The scanner keeps the complete inner object and resumes after the outer tail.
It rechecks both headers and tails before selection, then uses the existing
object index, duplicate checks, graph validation and sequential reconstruction.
Only redundant framing is omitted; no content stream is edited. Additional
keys, differing geometry or IDs, indirect/nonzero Lengths, nonempty content,
incomplete tails and nested repeats do not qualify. Header/tail probes use
bounded ranged reads and existing allocation, cancellation and I/O limits.
Indexed PDFs use only the separate profile below; this CAJ rule is unchanged.

### Indexed nested empty Forms

The measured #446 indexed-PDF profile has the same five-key empty-Form
dictionaries, generation-zero objects, immediate nesting, 256-byte header
bounds and two bounded complete tails. The inner ID must differ from the
outer ID and have its own generation-zero, standalone live xref elsewhere.
The ordinary sorted span validation rejects any live object overlapping the
outer frame. Geometry comparison strips only redundant trailing fractional
zeros and their decimal point; signs and integer digits remain exact. It
never rounds floating-point values or combines adjacent tokens.

Recovery appends an empty stream revision under the **outer** object ID,
retaining its dictionary and every reference. The actual live inner ID is
unmodified. The old nested serialization remains in the original PDF prefix;
existing stream-separator normalization may still apply. Both parsed headers
and tails are checked against a fresh read of the complete bounded frame,
then every retained byte is checked again during sequential copying, before
separator normalization. Proof bytes and their index are each limited to
64 KiB or one eighth of the caller's allocation limit, whichever is smaller;
replacement bodies share the existing repair budget. There is no payload
search, full-file buffer or per-candidate scan of the object index.

Same-ID nesting, nonempty/indirect Lengths, extra or duplicate keys, conflicting
geometry, missing/free/compressed/different-generation inner xrefs, recursive
nesting and incomplete tails remain errors. This does not establish a general
nested-object deletion rule. See [source evidence](provenance.md#indexed-nested-empty-form-446).

### Checksum-confirmed CAJ stream substitution

The measured #436 profile additionally recovers `ca a7 c2 e4` expanded from
`b5 f4` inside a damaged zlib frame. This is restricted to a complete CAJ
scan, an understated direct Length with the unique nearby terminator above,
and exactly `/Filter /FlateDecode` without DecodeParms or external-stream
keys. The measured extent must end in LF. Every candidate position lies
after the zlib header and before its checksum; removing two bytes per site
must restore the original Length, including LF. A checksum-valid original
stream, even with trailing bytes, forbids substitution. The candidate must
reach checksum-valid EOF at exactly the original frame length.

All candidate objects must survive the complete scan without unresolved
damage. Every original page-table object/offset must agree with the sparse
substituted view, and at least one page offset must move. Only then does the
converter rescan that view and perform the ordinary graph reconstruction.
It does not rewrite metadata, page tables, Lengths or checksums to manufacture
this evidence. Identical byte sequences outside admitted streams stay intact.
Failed admission retains the existing framing outcome; it does not promise
to reject every damaged codec payload.

The profile bounds each encoded extent to 256 KiB, each decoded probe to
4 MiB, and the document to 64 candidate streams; the existing 64-byte Length
repair bounds substitutions per stream. Decoder state requires a 64 KiB
allocation allowance, and decoded work also respects `max_allocation_bytes`.
Inflation and hashing use fixed 4 KiB buffers with reads clipped to the
caller's I/O chunk. Only positions and original SHA-256 digests are retained.
Original encoded ranges are rechecked after recognition, before conversion
and after successful emission; marker bytes are rechecked whenever read.
Cancellation and source errors propagate, and diagnostics use original input
offsets. As with other reconstruction, the source must stay stable throughout
the operation and callers must discard partial output on error.

This is an independently measured corruption profile, not a general byte
replacement, text encoding conversion, or validation of all content streams.
See [provenance](provenance.md#bounded-production-recovery) for the original
synthetic controls, external measurements and remaining viewer limitation.

## Redundant CAJ framing

The measured #434 profile additionally recognizes an obsolete classic
xref/trailer/startxref/EOF block followed by an encoded duplicate of the CAJ
header. Its syntax is capped at 4 KiB and the existing allocation-derived
syntax limit, with at most 128 ordered entries and exactly Size, Root, Info,
Prev and two 16-byte hexadecimal ID strings in the trailer. Old xref offsets
are never used as object boundaries. Every copied byte must equal the source
header from byte 144 after the measured FZHMEI XOR phase. At least 128 and
fewer than 65,536 bytes must match before CRLF and a complete generation-zero
integer that exactly confirms a previously pending stream Length. Header
fields, framing bytes and the complete duplicate are checked again.

Two narrowly measured unused interruptions can be omitted only after the
complete forward scan proves that no parsed object references their IDs and
no complete object has that ID. An unfinished generation-zero object followed
by a single `<`, CRLF and an adjacent complete non-stream object has no value
bytes to retain. An interrupted Image/Flate/RGB stream must have the measured
nine-field dictionary and a known indirect Length. Its dictionary, except
Length, must equal an independently anchored later image, and at least 256
but fewer than 65,536 payload bytes must match before CRLF and a pending
Length's exact integer. The complete image's declared extent and object
boundary must agree. Only one candidate may qualify, and the full scan must
actually reach it outside other stream payloads.

Image headers and adjacent opener objects are capped at 256 bytes; at most
64 eligible images and 64 unused interruptions are admitted. Prefix checks
use chunks of at most 256 bytes, respecting smaller I/O limits, short reads
and cancellation. A changed prefix/header, meaningful unmatched value,
incoming reference, conflicting counterpart or incomplete inspection fails.
Opaque object/xref streams cannot supply complete-reference proof; partial
scans that discard other damage cannot establish it either. This is not a
general policy of deleting unreferenced malformed objects. Complete stream
payloads are preserved, and no missing image data is reconstructed.

## Header offset

Issue [#300](https://github.com/rwv/caj2pdf-rust/issues/300): when no CAJ,
KDH, HN, C8, or TEB signature starts at byte 0, `detect_source` also selects
PDF if the whole five-byte `%PDF-` marker lies within the first 1,024 bytes
(`PDF_HEADER_SEARCH_BYTES`). Observed leading bytes are a newline, a UTF-8
byte-order mark, and a junk line. Only this search reads past the first
five bytes, in reads of at most `io_chunk_bytes`. The other families still
match only at byte 0.

For a header at offset H, the CLI and WASM engine read the PDF through
`PdfRange { offset: H, length: size - H }`. Cross-reference, `startxref`, and
`/Prev` offsets are therefore relative to the header, which is correct for a
complete PDF with bytes prepended; `qpdf --check` 11.9.0 accepts such files
without reconstruction warnings. Offsets counted from byte 0 are not tried
as a fallback, so a file written that way fails as malformed PDF. The leading
bytes are not copied: the output starts at `%PDF-` and equals converting the
input without them. `Limits.max_input_bytes` applies from H. A JavaScript
caller that sets `format: "pdf"` skips detection, so it still requires the
header at byte 0.

## Bookmarks and output

`copy_pdf` checks the PDF and copies its active bytes in bounded chunks. A
clean PDF is byte-identical at the destination. Recognized damage gets a
minimal incremental update. `PdfOutlineAppender` takes depth-first
`Bookmark` entries, checks each zero-based destination against the inspected
page order, and emits linked outline objects. It updates the existing Catalog
under the same indirect reference, preserving unrelated Catalog entries and
page streams. The output must be a distinct sink; a path-based caller should
stage output and rename it after `finish` succeeds.

If the input already has a nonempty outline tree, import preserves that tree
and reports zero imported bookmarks. A clean such input is copied byte for
byte. If recognized structural repair is needed, the repair still applies.
Unsupported encryption and PDFs with recognized certification or signature
indicators (`/Perms` or AcroForm `/SigFlags`) are rejected before the writer
claims success. The importer retains a bounded depth stack and emits
closed bookmark objects as entries arrive; it does not retain all titles.

On a source, sink, cancellation, or limit failure, the operation returns an
error and never a successful report. A sink may already contain a partial
PDF, so callers requiring an atomic path must use a temporary file. Required
tests reopen outputs with `qpdf` and MuPDF, compare page renders, and inject
sink failures. The optional external CAJSamples corpus is reported separately
and a missing corpus is never counted as a compatibility pass.

## Optional PDF-body corpus observation

On 2026-09-24, the two locally available PDF-body inputs at
`issue-33/test2.caj` and `issue-33/test3.caj` matched the SHA-256 values in
the [conformance matrix](../tests/conformance/matrix.json). With `qpdf`
12.2.0 and MuPDF 1.25.1, the Rust PDF layer normalized both outputs with a
clean `qpdf --check` result. MuPDF PNM renders at 36 dpi matched the input
page by page: 26/26 and 11/11. This measures only the PDF-body repair path;
the CAJ container conversion path is an issue #7 task. Neither the documents
nor the generated outputs are committed here.

## Optional KDH PDF-body check

The ignored `kdh_pdf_external` test is `NOT_RUN` in normal test runs. Supply
three independently decoded, trimmed PDF bodies named `issue-21.pdf`,
`issue-34.pdf`, and `issue-48.pdf` in one external directory, then run:

```sh
CAJ2PDF_KDH_PDF_DIR=/path/to/decoded-pdfs \
  cargo test --locked -p caj2pdf-core --test kdh_pdf_external -- --ignored --nocapture
```

At the pinned corpus revision used for issue #36, the local check returned
`qpdf --check` exit 0 for all three normalized outputs and matching 36-dpi
MuPDF PNM hashes on all 74 pages (6 + 67 + 1). This is PDF-body evidence;
the KDH wrapper and decryption path remain issue #11 work. The same outputs,
copied to the matrix-mapped paths outside Git, also returned inventory
`PASS` 3/3 and PDF `PASS` 3/3 from the corpus runner
`conformance.py --only-format KDH --pdf-dir ... --corpus-dir ...` (now in
[caj2pdf-samples](https://github.com/rwv/caj2pdf-samples/tree/main/research/scripts/conformance.py)):
all page counts, dimensions, outlines, and 74 rendered-page hashes matched
the pinned matrix. The first attempt used external symlinks and was rejected
by the harness's path-safety check; the reported pass used copied files.

## Explicit partial conversion of damaged CAJ inputs

`--allow-damaged` (JavaScript `allowDamaged: true`, Rust
`ConversionOptions::allow_damaged`) permits partial output for CAJ containers
with malformed embedded PDF objects. It is off by default. It does not add a
partial mode for PDF, KDH, HN, C8, or encrypted inputs.

After the existing bounded reconstruction fails, the partial path resumes at
an independently framed stream end or the page table's next page dictionary.
A table boundary can precede that dictionary by at most 64 bytes; its object
number and complete Page dictionary must agree with the table. This boundary
is used to discard content, never to claim lossless repair. No whole-document
buffer, marker search across the file, or external converter is used.

A dependency pass identifies pages affected by missing or malformed objects,
including shared fonts and images. Their contents, annotations, and resources
are replaced with an empty resource dictionary. Page object IDs, page order,
parentage, available page boxes, rotation, and bookmark targets are retained.
A shared damaged resource can require many blank pages; this mode does not
promise to recover every page a viewer can display. Missing or ambiguous page
geometry/boundaries, unsupported features, I/O errors, cancellation, and
resource-limit failures still abort conversion.

The CLI commits a valid PDF and returns **3** when it substituted any page.
Each warning identifies the one-based page and absolute input byte offset;
`--quiet` does not suppress these warnings. No substitutions means exit 0.
Rust's `omitted_pages` and JavaScript's `omittedPages` contain zero-based page
indices and absolute input offsets. JavaScript resolves with that report;
callers must check it before treating the output as complete.

Default corpus failures remain failures. An explicitly partial PDF is not a
successful lossless compatibility result.

### Pinned damaged-input observations

The five damaged CAJ entries in the pinned external corpus were checked on
2026-10-05. Each remained a strict-mode failure with no committed output.
Explicit partial mode returned 3, passed `qpdf --check`, retained every source
page object ID in table order, and had no Contents or Annots plus an empty
Resources dictionary on every reported blank page.

| Corpus case | Total pages | Blank substitutes |
| --- | ---: | ---: |
| issue-20 | 63 | 62 |
| issue-25 | 78 | 11 |
| issue-39 | 80 | 4 |
| issue-85 Mingtang | 234 | 15 |
| issue-90 `4-[6]` | 60 | 60 |

In particular, the current mode retains no page content for issue-90; a valid
PDF structure is not evidence of useful recovery. `pagesConverted` includes
blank substitutes. All omissions are reported even when every page is blank.
The other 12 pinned CAJ files retained their v0.4.0 output SHA-256 hashes under
both default options and `allowDamaged`. These are structural/hash checks,
not a new whole-document CAJViewer pixel comparison. Original corpus bytes
and resulting PDFs remain external to Git.

### CAJ QITE source-path strings

CAJ fragment recovery can hex-encode the measured unescaped source-file paths
in retained Pages' direct QITE_pageid/F metadata (#419). Every incoming target
reference must occur only once in such a Page. The scanner admits only a
complete generation-zero, single-line drive path ending in .pdf, with one
unescaped ASCII opening parenthesis and the observed GBK full-width closing
parenthesis. It does not reinterpret escapes or repair rendering strings.
A short prefix probe and a 640-byte object ceiling bound reads; at most 64
candidates can require complete graph validation, and retained bytes remain
under Limits. Every payload byte is preserved in the hexadecimal string.
Source rechecks, cancellation, sequential output and strict required references
remain in force. Ordinary indexed PDF strings retain their existing rules.

### Measured CAJ Pattern Matrix fallback

CAJ fragment recovery admits one measured malformed tiling-pattern Matrix
(#414): `[0.72 0 0 -0.719999 -5e-006 842]`. It writes explicit identity
`[1 0 0 1 0 0]` with equal-width padding to preserve the observed CAJViewer
and Poppler rendering. The pattern content and every stream byte are retained.
MuPDF handles the malformed original differently; neither arithmetic exponent
expansion nor replacing just the fifth value with zero matches the measured
source-viewer behavior. This is a documented recovery profile, not additional
PDF number syntax or general permission to discard malformed matrices.

The complete header must fit 512 bytes and contain exactly the eleven measured
keys: Type Pattern, PatternType/PaintType/TilingType 1, XStep/YStep 64,
BBox `[0 0 64 64]`, the measured Matrix, a generation-zero Resources reference,
Filter FlateDecode and direct Length 45. The generation-zero stream must have
its complete declared boundary; it cannot combine with an unproved stream
extent. Source bytes are rechecked and patched during ranged reads, with
bounded retained metadata and sequential output. Other malformed profiles
remain errors. Valid matrices and ordinary indexed PDFs remain strict.

See [provenance](provenance.md#malformed-tiling-pattern-matrices-414) and the
[scoped comparison](conformance.md#tiling-pattern-matrix-checkpoint-414).

### Proved interrupted copies across CAJ page rows

The measured #442 profile compares an interrupted object against one complete
same-ID generation-zero copy, either already framed or independently anchored
by a later page-table row. The comparison includes the complete stream header
and at least one payload byte. At most 64 KiB of strict proper prefix is
accepted; the first mismatch must lead through at most 64 non-NUL PDF spacing
bytes to a parsed adjacent indirect-object header. Both the prefix and boundary
are rechecked. No payload marker search or codec reinterpretation is added.
The two comparison buffers are at most 256 bytes each and respect smaller
I/O chunks and allocation limits.

Candidate rows are collected once each, from last to first, so later anchors
can supply proofs needed to frame earlier rows. Only starts inside the row
enter its candidate index. A distant counterpart is read by bounded ranges;
ordinary syntax/stream search windows are unchanged. The complete forward scan
must reach every used candidate at its exact range and must still reject
conflicting complete copies. Candidate-only scans do not establish validity of
the final document. Existing object/page/allocation and retry bounds apply.

Known metadata prefixes may stop inside a boolean or negative number. A
terminal repeated `number 0` header is accepted only when it exactly prefixes
one retained complete non-stream object and ends at actual source EOF.
Separately, the measured unfinished `Length <positive integer>` / `Filter
/FlateDecod` declaration followed by CRLF may be omitted only before an adjacent
generation-zero stream header, with its syntax bounded to 256 bytes. It has no
stream keyword or payload. Existing complete-graph validation must prove no
same-ID complete object or incoming reference, no damaged/uninspectable object
and no opaque metadata stream. Additional keys, content and unmeasured endings
remain errors; this is not general malformed-object deletion or parent repair.
Ordinary indexed PDFs keep their existing strict rules.

### Indirect lengths and distant same-row replays

The #444 profile resolves known indirect stream Length constraints before a
provisional stream span can establish a duplicate, anchored candidate or
missing Length error. One bounded batch updates sorted per-object hints; each
subsequent pass still requires the final integer objects to agree. Unknown
constraints remain unproved. The existing limit of 16 rescans is unchanged;
there is no retry for each independent known constraint.

When a Length is known, an `endstream` can propose a complete copy starting
within 1 MiB of an interrupted stream, including inside the same page-table
row. The proposed copy must repeat the header, have an exact declared tail,
and satisfy the existing 64 KiB proper-prefix and 64-byte adjacent-boundary
rules. The full forward scan must reach that exact copy range. Conflicting
anchored candidates and complete duplicates remain errors. The distance is a
search bound, never a buffer size: comparison uses the existing two bounded
256-byte buffers. Candidate-row scans, partial conversion, codecs and ordinary
indexed PDFs retain their existing scope. See [source evidence](provenance.md#indirect-length-ordering-and-same-row-replays-444).

## Interrupted metadata and missing parents

The measured CAJ profile in #452 admits an unused XML metadata stream that
ends after the exact `<?xpac` plus CRLF opener. Its generation-zero dictionary
contains only a direct positive Length, Type Metadata and Subtype XML, within
256 header bytes. The immediately following complete generation-zero integer
must resolve a previously framed stream's pending indirect Length. The full
scan verifies that constraint and proves the metadata ID absent and
unreferenced. This is not a general rule for dropping malformed XML or streams.

Two payload-free parent openers (`number 0` or `number 0 obj<<`, then CRLF)
may precede a complete Page object. Only absent IDs with incoming references
exclusively in leaf Page Parent fields qualify. Every affected page must
explicitly supply valid direct MediaBox/CropBox, a Resources dictionary or
reference, and direct Rotate 0/90/180/270. The existing converter checks table
membership and reconstructs parent groups in CAJ page order. No inherited
property is guessed and no dictionary key/value is discarded.

Both rules require a complete inspected graph without opaque ObjStm/XRef
metadata streams. Referenced metadata, other incoming parent edges, missing
page properties, ambiguous copies and unmeasured boundaries remain errors.
Parent openers are at most 64 bytes, with at most 64 pending parent records;
unused metadata uses the existing 64-record interruption bound. Existing
allocation limits, ranged reads, cancellation and sequential output apply.
Clean indexed PDFs and public APIs are unchanged.

## Retained catalog and incomplete page tree

The #456 CAJ profile has one retained catalog with exactly Type, Pages,
PageLabels, AcroForm and Metadata. Its old three-entry Pages root contains only
Type, Count and Kids; every listed child is absent and no complete page/tree
node points back to that root. The old root's Count must equal the complete
leaf inventory. Catalog and root have no other incoming edges. The converter
rebuilds the tree from complete leaves, existing validated intermediate nodes
and CAJ table order, then emits a new catalog.

The original two-object PageLabels tree remains connected to that catalog.
Only the measured single range at index zero with an indirect, one-entry
decimal style dictionary is admitted. AcroForm has no complete object or
interruption; Metadata has only the measured positive-Length declaration
ending inside `/Type/Metada`, followed by CRLF and a complete object. Neither
optional target may have another incoming reference. Undefined optional
references have null semantics under ISO 32000-1:2008 7.3.9–7.3.10; available
form/metadata content is never discarded by this rule. Every leaf must have
explicit MediaBox, CropBox, Rotate and Resources and no Annots entry.

Catalog, old root, both label objects and the metadata declaration are each
bounded to 256 bytes. The complete-graph/no-opaque-stream proof remains
mandatory. Extra catalogs, keys, incoming edges, live optional targets,
different label profiles and incomplete proofs remain errors.

Parent recovery additionally recognizes the measured bare header with one
space before CRLF. A Count/Kids interruption must end strictly inside the last
child's decimal number, with the count and all preceding child references
matching complete independent leaves in source order. A referenced complete
intermediate Pages node may have only Type, Parent, Count and Kids, with a
matching count and distinct independent leaf children. There is at most one
such level and 64 children; no inherited value is inferred. The existing
64-byte parent-prefix and 64-candidate bounds remain. Reads to the immediately
following complete page are bounded to 256 bytes; no new payload search or
whole-file buffer is used. Public APIs and ordinary indexed-PDF rules retain
their existing behavior.

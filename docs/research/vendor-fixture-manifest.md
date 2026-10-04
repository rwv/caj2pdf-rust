<!-- SPDX-License-Identifier: MIT -->
# External vendor fixture manifests

This is the version 1 integrity contract for
[issue #125](https://github.com/rwv/caj2pdf-rust/issues/125). It supports the
separate CAJViewer acquisition work in
[the vendor fixture plan](cajviewer-fixtures.md). The reader does not start
CAJViewer, decode an image, run OCR, convert a document or compare candidate
output. A small external acquisition is documented in [the fixture snapshot](cajviewer-fixture-snapshot.md). Converter compatibility remains **NOT_RUN**.

An integrity `PASS` means the supplied declarations, exact file identities,
page mapping and text encoding agree. It does not prove that a capture covers
a page, that an application observed those bytes, or that encoded PNG/TIFF
bytes produced the separately supplied pixels. The acquisition protocol and
its reviewed receipt must establish those relationships. A `PASS` with
`basis: original-synthetic` is an original test control only; vendor passes
and actual comparisons remain zero.

## Ownership and storage

The schema, reader, generator and runtime controls are original MIT code in
[scripts/vendor_fixtures.py](../../scripts/vendor_fixtures.py). They use Python
3.11 or later and only its standard library. Asset verification requires
POSIX directory descriptors, `O_DIRECTORY` and `O_NOFOLLOW`; it refuses an
unsupported platform rather than using a weaker path check. This developer
tool does not change the portable Rust, browser or Node.js conversion APIs.

Keep vendor installers, containers, source documents, acquired text, page
images, decoded pixel arrays, manifests and full receipts in external
storage. Do not commit them, their private paths, document titles or private
text. A reviewed public catalog contains safe synthetic source aliases,
counts, modes, geometry and hashes. It omits external paths, runtime strings,
source names and action commands. Catalog IDs and selected public metadata
must still be reviewed before publication; redaction is not anonymization.

Each version has a fresh directory:

```text
external-version/
  catalog.json                 # public, redacted identity/projection
  bundle/
    manifest.json              # exact external declarations
    receipt.json               # hash-bound acquisition receipt
    images/...                 # acquired encoded files
    pixels/...                 # separately acquired full raw arrays
    text/...                   # raw copy, strict UTF-8 and optional derivative
    history/...                # retained predecessor/diff/review/receipts
```

`source` and `runtime` are separately supplied immutable root directories.
They need not be copied into each version. The example generator creates
them inside its new control directory with invented non-executable files.
Source/runtime roots may contain undeclared unrelated files; this reader
checks every declared pin, while an acquisition protocol determines whether
its runtime inventory is complete. Bundle files must be exactly the declared
set; missing, extra, symlinked and non-regular entries fail.

## Types, ordering and schema version

All objects require exactly their documented keys. `schema` is
`cajviewer-fixtures/1`. Unknown schemas, acquisition modes, statuses and
fields fail. JSON duplicate keys, binary floating point numbers, non-finite
literals and integers outside signed 64-bit framing fail. Geometry is a
decimal string without exponents, with at most 48 bytes and absolute value
at most 2,147,483,647. Byte counts and indices are exact integers, never
booleans. SHA-256 is 64 lowercase hexadecimal digits; code commits are 40.

A file reference is `{scope, path, bytes, sha256}`. `scope` is `bundle`,
`source` or `runtime`; `path` is relative to that explicitly supplied root.
Absolute paths, empty components, `.`/`..`, backslashes, colons and more than
16 components fail. The final file must be regular. Every component is
opened relative to a held directory descriptor with no symlink following.
Root replacement is checked again during the final audit. The tool does not
claim that an attacker cannot change a file after the verification ends.

An observation is `{status, value}`. `status` is `OBSERVED`, `PINNED`,
`REQUESTED`, `UNAVAILABLE` or `NOT_RUN`. Unavailable/not-run observations
must have null values. Requested settings are declarations, not measured
facts. Physical page counts cannot be `REQUESTED`. An unknown installed
build, vendor page count, page box, rotation or device pixel ratio stays
null with the appropriate observation status; it is not inferred from the
source count, display dimensions or requested Qt scale.

The top-level external manifest has these keys:

| Field | Contract |
| --- | --- |
| `schema`, `profile`, `bundle_id`, `version` | Versioned schema/profile, safe bundle ID and positive immutable version number. |
| `basis` | `original-synthetic` or `vendor-observation`; these evidence bases never merge. |
| `creator` | `protocol_sha256`, declared `code_commit`, `code_sha256`. Acquisition must freeze these before private work. |
| `runtime` | Application, installer provenance, platform/container, libraries/plugins/fonts/tools, exact settings and environment digest. |
| `sources` | Canonical external source IDs, files, declared coverage, counts and ordered physical-page records. |
| `receipt` | Bundle file reference to the acquisition receipt. |
| `history` | Ordered `{kind, file}` entries preserving baseline manifests/receipts, failures, diffs, reviews and regeneration receipts. |

### Runtime pins

`runtime` has `application`, `platform`, `resources`, `settings` and
`environment_sha256`. The environment hash binds the acquisition protocol's
declared canonical child environment; this reader cannot reconstruct it from
a hash or prove that a child used it.

`application` has `installer` (runtime file reference), `installer_url`
(declared HTTPS provenance URL), `build` (observation), `aur_commit`,
`aur_sha256` and `license`. Official origin and the installed build require
the capability issue's independent observations; an AUR packaging version
is not an observed vendor build. No installer is fetched or executed here.

`platform` has `os`, `architecture`, `image_reference` and
`container_sha256`. `resources` has arrays `libraries`, `plugins`, `fonts`
and `tools`, each containing `{id, version, file}` with unique resource IDs.
All declared runtime files are hashed before and after verification.

`settings` has `locale`, `timezone`, `display`, `screen_width`,
`screen_height`, `dpi`, `qt_scale`, `render`, `print` and `clipboard`.
`dpi` and `qt_scale` are positive decimal observations; other settings are
bounded declared strings/integers. Actual capture DPI/zoom/DPR are separate
page observations. Acquisition must name complete modes and print settings
in its external receipt; no print/export/headless capability is inferred.

### Source and page coverage

A source record has `id`, `file`, `variant`, `source_pages`, `vendor_pages`,
`output_pages`, `coverage` and `pages`. Variants are `CAJ`, `HN-A`, `HN-B`,
`C8`, `KDH`, `PDF` or `TEB`; the reader validates the label, not the document
format. Counts are observations. `coverage` is
`{kind: complete|subset, source_pages: [...]}`. Indices are unique, positive
and increasing. Complete coverage must list every known physical source
page. A pilot subset must declare exactly its selected indices.

Each page has `source_page`, nullable `vendor_page` and `output_page`, `box`,
`rotation`, `capture`, `image` and `text`. `box` observes four ordered point
bounds; `rotation` observes 0/90/180/270. `capture` observes positive `dpi`,
`device_pixel_ratio` and `zoom`. Unknowns remain explicit null observations.
Acquired content requires a vendor-page mapping. Non-null vendor and output
indices must be increasing and unique; complete declared counts must be
fully represented. Thus six source rows mapping to outputs 1 and 2 retain
the four nullable rows. Equal page pixels never remove a physical row.

### Complete raw image contract

`image` has `status`, `origin`, `format`, `coverage`, `artifact`, `payload`
and `raster`. `PASS` records a claimed acquired complete page; other statuses
(`FAIL`, `UNAVAILABLE`, `NOT_RUN`) require all other fields null. A failed
acquisition can retain its bytes in an external failure session; it cannot
present them as a successful page in this schema.

Origins are exactly `viewer-native-page-image`,
`viewer-complete-page-capture` or `viewer-exported-pdf-render`. They remain
distinct. Formats are `png`, `tiff`, `ppm`, `pgm`, `pbm` or `raw`. Coverage
must be `complete-page`; region/viewport captures need a different profile.
`artifact` identifies the encoded file; `payload` independently identifies
the complete raw array. No file header, encoded image or pixel content is
decoded by this reader. Even a plausible PPM artifact does not establish
the relationship to its payload without the acquisition receipt.

`raster` has `width`, `height`, `depth`, `channels`, `alpha`, `icc_sha256`,
`background` and `row_order`. Version 1 defines all raw layouts:

- Channels are Gray (1), RGB (3) or RGBA (4), in that order; no BGR or planar
  representation is admitted. Depth is 8 or 16 bits per channel, with 16-bit
  samples in big-endian order.
- Bilevel is depth 1, channels 1, most significant bit first, 0 black and 1
  white. Each row rounds up to a byte; unused low bits must be zero and are
  checked even across read boundaries.
- Other rows are tightly packed with no padding. `row_order` is explicitly
  `top-to-bottom` or `bottom-to-top`; verification does not flip rows.
- Alpha is `none` for Gray/RGB or `straight`/`premultiplied` for RGBA.
  Background is a lowercase `#rrggbb` value or `transparent`; ICC is an
  exact optional SHA-256. No colorspace conversion or ICC application occurs.

The exact payload size is `ceil(width * depth * channels / 8) * height`,
subject to dimension and byte caps. Every byte participates in the checked
SHA, including all admitted rows, channels and bilevel padding. These
constraints establish integrity, not pixel agreement with a converter.

### Text modes, strict Unicode and freshness

`text` has `status`, `mode`, `selected_page`, `coverage`, `range`, `mime`,
`encoding`, `raw`, `unicode`, `unicode_contract`, `code_points`,
`normalization` and `clipboard`. `FAIL`, `UNAVAILABLE` and `NOT_RUN` require
all other fields null. `NO_TEXT` requires a freshly observed empty copy;
`PASS` requires nonempty text. Two empty outputs are not a positive text
compatibility result. Selection must name the mapped vendor page.

Modes are `existing-text-confirmed`, `standard-copy-origin-unverified`,
`enhanced-copy`, `ocr` or `repair`. These labels are not changed by successful
hashing. Standard copy is not automatically existing/native Unicode, and
enhanced/OCR/repair data does not become ordinary text evidence. Version 1
supports successful **clipboard acquisitions only**, including when an
explicit OCR/repair mode uses a clipboard. Native TXT or OCR file-export
observations require a future distinct profile, not a fabricated clipboard.

Coverage is `complete-page` with null range or `explicit-region` with
`{unit: page-points|viewport-pixels, bounds: [x0,y0,x1,y1]}`. Whole-document
fixtures use separate ordered complete-page records. Region text cannot
satisfy a complete-page gate. Raw MIME and encoding are retained. Encoding
is `utf-8`, `utf-16-le`, `utf-16-be` or `latin-1`; endian variants have no
implicit BOM stripping. `unicode_contract` is `utf-8-strict`.

The reader incrementally decodes raw bytes with strict errors and checks
their exact UTF-8 length, SHA and Unicode code-point count against `unicode`.
Truncated code units and invalid encodings fail; it never injects replacement
characters. A literal, expected U+FFFD is valid content. Raw UTF-8 and
canonical UTF-8 may have the same identity; their semantic roles remain
distinct fields. Only named optional `NFC` or `newline-lf` derivatives are
supported. The original Unicode identity is always checked first; whitespace
collapse, NFKC, case folding and implicit normalization are not performed.

`clipboard` has `sentinel_sha256`, `fresh: true` and `mechanism`. Acquisition
must establish a fresh transaction/content observation after installing a
sentinel; elapsed time alone is not proof. The sentinel cannot equal the raw
copy identity. A timeout or still-present sentinel is failure/unavailable,
not an empty page. This reader validates a bound declaration, not an X11 or
clipboard-manager event. Positive/negative public canaries establish that
mechanism in the acquisition issue.

## Acquisition receipts and audits

An acquisition receipt has `schema`, `bundle_id`, `observations_sha256`,
`status`, `caps`, `resources`, `actions`, `counts`, `before_audit` and
`after_audit`. Its file identity is bound by the manifest and public catalog.
`observations_sha256` is SHA-256 of the canonical JSON object containing
exact `creator`, `runtime` and `sources` from the manifest. Canonical JSON
uses sorted keys, ASCII escaping, compact separators and one final newline.
Receipt/history references are excluded to avoid a circular hash. Changed
runtime settings, source bytes or output metadata require a new receipt.

`caps` declares `wall_ms`, `child_wall_ms`, `memory_bytes`, `pids`,
`file_bytes`, `session_bytes`, `stdout_bytes` and `stderr_bytes`.
For a passing receipt, `file_bytes` must cover each known acquired-page
encoded artifact, raw pixel payload, raw text, strict Unicode and optional
normalization file. Source documents, installers/runtime pins, manifests,
receipts and historical packaging metadata are outside that acquisition-file
scope and have their own reader limits. A failed receipt can preserve an
honest cap violation. `pids` is a declared acquisition ceiling: version 1
has no observed task-count field, and this integrity reader does not measure
or enforce process counts. The acquisition supervisor/protocol remains
responsible for that bound and its reviewed evidence.
`resources` records `elapsed_ms`, `owned_disk_peak_bytes` and
`process_tree_memory: {status: MEASURED|UNAVAILABLE, peak_bytes, method}`.
Unavailable memory is null, never zero. Failed receipts may also use null
for unmeasured elapsed time and disk peak; passing receipts still require
measured values within their caps. A passing vendor acquisition must
have measured process-tree memory evidence. The reader does not operate a
process supervisor; acquisition must enforce whole-stage/process-tree caps
and account for UI, screenshots, clipboard, exports and finally cleanup.

Each sequential `action` records `number`, `kind` (`process` or `ui`), exact
external `command`/UI arguments, `completed`, `status`, nullable
`return_code`, `wall_ms`, nullable `rss_bytes`, and separate stdout/stderr
byte counts and hashes. Counts are `planned`, `attempted`, `completed`,
`failed` and `unstarted`, checked against actual entries. Missing/unstarted
actions cannot be passing acquisition evidence. Before/final audit states
are explicit. Failed receipts may honestly record a cap violation; the
reader retains the declared cap and observed excess rather than rewriting
the failure to fit it. No such failed candidate can be promoted.

Verification hashes every declared source/runtime/bundle file through the
same bounded read used for validation, checks its exact size and EOF, and
checks descriptor identity/timestamps before and after. It repeats checked
reads and root identity/bundle membership audits in `finally`, including
failure paths. A declared root or byte mutation makes integrity `FAIL`.
An unpinned catalog identifies the input but is not authenticated provenance;
use `--catalog-sha256` with a separately reviewed pin when trusting a baseline.
This protects against a complete rewrite of otherwise self-consistent pins.

## Bounds and reporting

Default `Limits` are explicit and caller-reducible:

| Scope | Limit |
| --- | --- |
| Public catalog and active manifest | 1 MiB each |
| Receipt/history/review/diff record | 2 MiB each |
| Raw/strict/normalized text record | 1 MiB each |
| Raster or encoded image | 64 MiB each |
| Source / runtime file | 1 GiB / 512 MiB each |
| Aggregate requested read bytes, including EOF checks and final audits | 4 GiB |
| New output bytes, including a reserved failure marker | 512 MiB |
| Physical pages / source or receipt arrays / distinct files | 4,096 / 4,096 / 16,384 |
| JSON nesting / lexical string / dimensions | 24 / 16,384 bytes / 32,768 |
| Read/write request / relative path depth | 65,536 bytes / 16 components |
| Verification or generation operation, including its audits | 60 s |

Limits bound owned records and buffers, not Python interpreter overhead or
process RSS. JSON trees are bounded by byte/depth/record limits. Ordinary
text is incremental; optional normalization retains at most one bounded
decoded record and derivative. Images and sources are never collected.
Catalog/manifest/receipt parsing may temporarily hold raw data and a bounded
decoded tree. File metadata and history are bounded by counts. Regeneration
shares one aggregate read budget/deadline across predecessor, candidate,
streamed copying, generated verification and final audits. Per-file caps do
not promise that a maximum-size collection fits the aggregate budget.

Returned counts measure this invocation's checked file/read/write/page work.
Acquisition receipt counts are reported separately as declarations.
`process_launches`, `vendor_passes` and `comparisons` are always zero: this
tool never starts external tools. Actual Python CLI subprocesses used by the
public test suite are original controls, not vendor work. The operation time
covers the function and final audits; interpreter startup, CLI parsing and
bounded final stdout JSON serialization are outside that timer. Explicit
missing inputs fail with nonzero exit. No-input mode performs only in-memory
schema controls and returns `NOT_RUN` with every I/O/output/vendor count zero.

## New versions, diffs and review

Acquisition first creates a separate candidate directory. Regeneration
requires verified predecessor and candidate inputs, the same profile/basis,
a new bundle ID, and `candidate.version == predecessor.version + 1`. A
candidate whose acquisition receipt or image/text record is `FAIL` is
refused. Unavailable/NO_TEXT/not-run outcomes remain explicit; they do not
become compatibility success by creating a new version.

The output must not exist, and cannot be under any input bundle/source/runtime
or catalog directory. Files use exclusive creation and descriptor-relative
paths. No input or earlier version is overwritten or removed. Read-only
file/directory modes discourage accidents; they are not an OS immutability
or authorization guarantee. A failed partial output is retained, with an
`INCOMPLETE.json` marker when the filesystem permits it, and a `FAIL` result.
Never treat a partial directory or a successful hash alone as approval.

The new version retains predecessor and candidate manifests, predecessor
acquisition receipt, every existing historical failure/diff/review/receipt,
and the candidate acquired bytes. The diff names schema field positions and
before/after byte-count/SHA identities, without copying private values.
Permitted reasons are `viewer-runtime-change`, `acquisition-improvement` and
`source-interpretation-correction`. A regeneration receipt binds exact raw
predecessor/candidate catalog identities, diff identity, declared code
commit and the actual reader source-file hash. The source is checked by the
same bounded I/O budget and again in final audits. The caller's commit string
is a declaration; repository/release review verifies it separately.

The first output is **REVIEW_REQUIRED**, even if integrity passes. To create
a separate reviewed version, supply a JSON review with exactly:

```json
{
  "schema": "cajviewer-fixtures/1",
  "decision": "APPROVE",
  "reviewer": "public-reviewer-id",
  "reason": "acquisition-improvement",
  "predecessor": {"bytes": 1, "sha256": "<exact predecessor catalog hash>"},
  "candidate": {"bytes": 1, "sha256": "<exact candidate catalog hash>"},
  "diff": {"bytes": 1, "sha256": "<exact diff hash>"},
  "code": {"commit": "<declared 40-hex commit>", "bytes": 1,
           "sha256": "<exact reader source hash>"}
}
```

The explanatory placeholders above are not valid fixture data. Copy exact
`pins` from the successful preparation result; the reader rejects any
unrelated predecessor, candidate, reason, diff or code pin. It retains and
validates the approval file and its identity. Rewriting only the public
catalog's review label cannot bless an unreviewed generation. `REVIEWED`
means a matching declared attestation exists; it is not cryptographic signer
authentication, vendor compatibility, permission to publish private data,
or completion of the project's release gates. Review and regenerate into
another fresh directory; never edit the prepared baseline in place.

## Reproduce with original controls

The following commands need no viewer, vendor installer, Docker, private
document or renderer. All generated file contents and pins are explicitly
invented controls, including the non-executable installer/runtime stand-ins.
Use a fresh external parent directory; the generator refuses an existing
output.

```sh
python3 scripts/vendor_fixtures.py --json
python3 scripts/vendor_fixtures.py example --output /external/controls/v1
python3 scripts/vendor_fixtures.py example --output /external/controls/candidate-v2 \
  --version 2 --bundle-id original-control-v2
python3 scripts/vendor_fixtures.py verify \
  --catalog /external/controls/v1/catalog.json \
  --bundle-root /external/controls/v1/bundle \
  --source-root /external/controls/v1/source \
  --runtime-root /external/controls/v1/runtime
python3 scripts/vendor_fixtures.py regenerate \
  --predecessor-catalog /external/controls/v1/catalog.json \
  --predecessor-bundle-root /external/controls/v1/bundle \
  --predecessor-source-root /external/controls/v1/source \
  --predecessor-runtime-root /external/controls/v1/runtime \
  --candidate-catalog /external/controls/candidate-v2/catalog.json \
  --candidate-bundle-root /external/controls/candidate-v2/bundle \
  --candidate-source-root /external/controls/candidate-v2/source \
  --candidate-runtime-root /external/controls/candidate-v2/runtime \
  --output /external/controls/prepared-v2 \
  --reason acquisition-improvement --code-commit 0000000000000000000000000000000000000000
python3 -m unittest discover -s tests/conformance -p test_vendor_fixtures.py -v
```

The zero commit is a declared synthetic stand-in. A real acquisition and
review use a separately verified exact commit. The prepared output's
returned `roots` retain the candidate source/runtime roots. Explicit verify
uses those paths and the newly generated bundle/catalog. Supply
`--predecessor-catalog-sha256` and `--candidate-catalog-sha256` when those
baseline pins come from a reviewed catalog.

The public tests run in the existing conformance discovery gate. They cover
complete/asymmetric and repeated-page controls, subset/no-output mapping,
raw layouts, strict Unicode/normalization, stale clipboard and receipt pins,
malformed JSON, confinement, exact lengths/hashes, short/failed reads/writes,
root/file mutation, bounded I/O, immutable generations, exact reviews and
actual CLI success/failure. No skipped optional vendor test is counted as
compatibility. The acquisition issues populate external versions only after
their capabilities and frozen protocols are independently reviewed.

# Command-line interface

This note documents the `caj2pdf` executable delivered for
[issue #12](https://github.com/rwv/caj2pdf-rust/issues/12). The command is a
thin native adapter over the platform-neutral core: it opens files, spools
forward-only input, stages path output, and chooses the core operation from
the input's leading signature. Format parsing and PDF writing stay in
`caj2pdf-core`. Versions `v0.x.y` may change this interface; such changes are
listed in the release notes.

```text
caj2pdf INPUT [-o OUTPUT] [--force] [--no-bookmarks] [--qm-states FILE] [--mq-states FILE]
caj2pdf inspect INPUT [--json] [--bookmarks]
caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]
caj2pdf --help | --version
```

A subcommand name is recognized only as the first argument. Convert a file
named `inspect` as `caj2pdf ./inspect`. Options may appear before or after
positional arguments; `--` ends option parsing. `-o FILE`, `--output FILE`,
and `--output=FILE` are equivalent. Every path argument is kept as an
`OsString`, so non-UTF-8 Linux file names work. The `--output=FILE` spelling
requires a UTF-8 value; use `-o FILE` for any other name. Diagnostics shown on
standard error replace invalid UTF-8 with U+FFFD.

## Formats

The format comes from the first bytes of the input, never from its name.
Some observed `.caj` files are plain PDFs. The signatures below only select a
parser; that parser then validates the full header, so, for example, a file
that starts with `CAJ` but lacks the CAJ header is reported as malformed.

| Signature | Format | Conversion | `inspect` |
| --- | --- | --- | --- |
| `%PDF-` | PDF | Validated copy through the core PDF reader and repair layer | Pages and outline presence |
| `CAJ` | CAJ | Reconstructed PDF with the CAJ outline | Pages and full outline |
| `KDH` | KDH | Decoded embedded PDF | Pages and outline presence |
| `HN` | HN | Experimental image-page conversion with built-in standard codec states | Variant/pages; HN-A full outline, HN-B outline unknown |
| `c8 00 00 00` | C8 | Experimental image pages; admitted native text/mixed pages with explicit fonts | Container variant and pages |
| `TEB` | TEB | Unsupported; exits with status 1 | Format only |

HN/C8 routes use the same independently implemented core page composer as
WASM. Malformed data and unsupported layouts produce located errors. No NH
signature has been measured, so NH input remains unrecognized. PDF/KDH
inspection reports outline presence rather than full outline entries;
HN-A inspection also validates and lists its outline. C8/HN-B outline metadata
remains unknown. Inspection needs neither state files nor scratch storage.
The CLI bounds retained outline records plus title capacities by
`max_allocation_bytes`. With a ranged input, image payloads are not read;
stdin still follows the bounded spooling rule below.

### Source geometry correction (breaking, v0.x)

HN-A/C8 now use declared page and image display extents independently of decoded
pixel dimensions. PDF image streams omit DIB storage padding. Earlier builds
used the first image at 300 DPI for the page and displayed padded image widths;
page sizes, transforms, image widths, PDF bytes and hashes can therefore change.
Regenerate affected PDF snapshots instead of preserving the old geometry.
Zero declared extents now produce a located geometry error. HN-B keeps its
separately measured single-JPEG behavior.

The source-unit-to-point factor remains empirical (`240 / 2473`); this correction
does not claim exact CAJViewer rasterization or establish a universal physical
unit. See the [controlled field checks](cajviewer-hnc8-kdh.md#controlled-geometry-checks).

### Experimental HN/C8 options

- `--qm-states FILE`: override the standard states for type-0 images.
- `--mq-states FILE`: override the standard states for arithmetic JBIG2 images.
- `--no-bookmarks`: skip CAJ/HN outline import. Required for C8/HN-B because
  their outlines are not yet validated. Existing embedded PDF/KDH outlines
  are not removed by this flag.

The state flags accept both `--qm-states FILE` and `--qm-states=FILE`
(and likewise MQ). Separate values preserve non-UTF-8 filenames. Values must
be nonempty paths, not `-`; document input may still come from stdin.
These options apply only to conversion, not `inspect` or `add-bookmarks`.
State files are protected inputs, including aliases: `--force` cannot overwrite
them. Unknown or duplicate state options are usage errors.

A state file is UTF-8 text, at most 16 KiB, with exactly 113 QM or 47 MQ rows.
Each row contains four whitespace-separated decimal integers in this order:
`qe next_lps next_mps switch_mps`. `qe` is in 1..32767, transition indices are
zero-based within the table, and switch is 0 or 1. A final newline and CRLF are
accepted; headers, comments, blank rows and extra fields are rejected. The
column order is explicit and differs from some standard-table presentations.
Standard T.82/T.88 states are built in. These files are optional overrides;
valid shape alone does not prove that a custom table is correct.

Omit both state flags for normal conversion. HN-A outlines are supported;
C8/HN-B use `--no-bookmarks`. Image-less HN-B rows, pure-text/searchable HN and
unverified profiles are rejected. The HN/C8 route admits the measured unused-refinement-template anomaly; other
malformed JBIG2 flags remain errors.

The command creates four private anonymous files in `TMPDIR` (or the system
temporary directory), each capped at 64 MiB by the composition budget. The
names are removed before conversion; the OS releases storage when handles
close, including on process exit. This reuses input spooling's file helper.
Forward-only document input is separately spooled within its input limit.
Output remains sequential; a failed conversion never commits a staged path
output. Stdout can contain a partial PDF on failure, as for other formats.

### Native C8 font resources

Supply all three ordinary roles to enable the admitted native C8 profile:

```sh
caj2pdf input.caj --no-bookmarks -o output.pdf \
  --font-cjk text.ttf \
  --font-latin text.ttf \
  --font-alternate-latin alternate.ttf \
  --font-decoration symbols.ttf
```

`--font-decoration` is optional; a document requiring decoration fails if it
is absent. Its default nonsemantic alias is `►`; use `--decoration-char CHAR`
for a different single BMP Unicode scalar supplied by that font. The alias
is not emitted as document text. Fonts must cover the Unicode characters
required by their assigned roles. No system lookup or missing-glyph fallback
is performed; substitution/font-identity limitations remain explicit.

Font path flags accept separate values or `--font-cjk=FILE` spelling.
Separate values preserve non-UTF-8 paths. Repeating a role, omitting one of
the three ordinary roles, or using `-` as a font path is a usage error.
Reuse the same path for multiple roles to embed it once. Native C8 font
options are rejected for other document formats.

Files are read through ranged/seekable handles. Forward-only named inputs
reuse the existing bounded temporary spooling path. Font files, including
hard-link/symlink aliases, are protected against output replacement even
with `--force`. A missing font fails before output staging; invalid fonts,
missing glyphs and later-page errors discard staged output and preserve an
existing destination. Standard output can still contain partial bytes on
failure. Fonts are caller-provided and are not bundled with the executable.

### v0.x migration

HN/C8 conversion now attempts supported page profiles instead of unconditionally
rejecting the format. Missing state data and unsupported metadata/layouts have
specific diagnostics. `inspect` reports `conversion_supported: true` for HN/C8
because this build has a conversion route; that is not proof that a particular
profile converts or its configured resource limits suffice. Human-readable
inspection marks this support as experimental. The JSON schema remains version 1. HN-A `has_outline`, `bookmark_count` and
`bookmarks` now contain validated metadata rather than unknown values; a malformed
outline fails inspection with its source location. Known empty outlines produce
`false`, zero and an empty list. C8/HN-B remain unknown.

## Conversion

A named input without `-o` writes a sibling file with the extension replaced
by `.pdf`, for example `paper.caj` to `paper.pdf`. If that path equals the
input, as for `paper.pdf`, the command exits with status 2 and asks for an
explicit output. `-` names standard input or output. Standard input without
`-o` writes to standard output.

Output rules:

- An existing output path, including a dangling symbolic link, is refused
  unless `--force` is given.
- An output that is the same file as an input is always refused, even with
  `--force`. The check compares device and inode numbers of the opened input
  with the output path after following symbolic links, so hard links,
  symbolic links, and differently spelled paths are all detected. Standard
  output is checked the same way when it is a regular file, such as
  `>> paper.caj`.
- A path output is written to a new hidden temporary file in the output's
  directory (`.NAME.PID-N.tmp`, created exclusively with mode `0666` before
  the umask; `NAME` is cut to 200 bytes). It is flushed and synchronized
  only after the whole conversion succeeds, and then given the target name.
  Every error path removes the temporary file. A process killed by a signal
  can leave it behind.
- The existence and same-file checks are repeated at that point, because the target may change
  during a conversion. Without `--force` the file is hard-linked to the
  target name, which fails atomically if any entry has appeared there; only
  on a file system without hard links does the command fall back to a
  re-check followed by a rename, which leaves a short race window.
- `--force` replaces the directory entry by rename. A symbolic link at the
  output path is replaced by the new file; its target is not written. An
  input swapped into the output path between the final same-file check and
  the rename would still be replaced; the input's own bytes are never
  written.
- Standard output receives only PDF bytes. The command refuses to write PDF
  bytes to a terminal, before it reads any input. Bytes already written to a pipe cannot be withdrawn
  when a later error occurs; the exit status reports the failure.

Input rules:

- A regular file, named or supplied as standard input, is read in place with
  positioned reads. Standard input is read from its beginning.
- Standard input that is a pipe, and any other non-regular input such as
  `/dev/stdin` or a FIFO, is copied in 64 KiB chunks to an unnamed file in
  `$TMPDIR` (or `/tmp`). The file is created with mode `0600` and unlinked
  before the copy starts, so its storage is released when the command exits,
  including after errors. The copy is bounded by the core's
  `Limits::max_input_bytes`, currently 8 GiB.
- A directory input is refused.

## `inspect`

`inspect` writes a report to standard output and exits with status 0 for any
recognized format, including those that cannot be converted. A malformed or
unrecognized input is an error with status 1.

The text form is intended for people and may change:

```text
Format: CAJ
Conversion: supported
Pages: 3
Outline: yes
Bookmarks: 3
  - Introduction (page 1)
    - Background (page 2)
  - Methods (page 3)
```

The bookmark lines appear only with `--bookmarks`. Each level is indented by
two further spaces. Control characters in titles are shown as `\u{..}`
escapes. HN and C8 add a `Variant:` line. Unknown values are shown as
`unknown`.

### JSON schema, version 1

`--json` writes one compact JSON object followed by a newline. Fields appear
in the order below. A later incompatible change increments `schema_version`;
adding a field is not considered incompatible.

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Always `1` for this schema. |
| `format` | string | `"PDF"`, `"CAJ"`, `"KDH"`, `"HN"`, `"C8"`, or `"TEB"`. |
| `variant` | string or null | Measured HN/C8 container layout: `"C8"`, `"HN-A"`, or `"HN-B"`; otherwise null. |
| `conversion_supported` | boolean | Whether this build has a conversion route; HN/C8 still require a supported profile. |
| `page_count` | integer or null | Declared page count; null when unknown (TEB). |
| `has_outline` | boolean or null | Whether the document has an outline; null when unknown (HN-B, C8, TEB). |
| `bookmark_count` | integer or null | Number of outline entries; null when this format's outline cannot be listed. |
| `bookmarks` | array or null | Present only with `--bookmarks`. The root entries, or null when the outline cannot be listed. |

Each bookmark object has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `title` | string | Title decoded to Unicode. |
| `page` | integer | One-based destination page. |
| `children` | array | Nested bookmark objects in document order; empty for a leaf. |

Example:

```json
{"schema_version":1,"format":"CAJ","variant":null,"conversion_supported":true,"page_count":3,"has_outline":true,"bookmark_count":3,"bookmarks":[{"title":"Introduction","page":1,"children":[{"title":"Background","page":2,"children":[]}]},{"title":"Methods","page":3,"children":[]}]}
```

Strings escape `"`, `\`, and control characters as required by RFC 8259;
other characters are written as UTF-8.

## `add-bookmarks`

`add-bookmarks` reads the outline of `SOURCE_CAJ` and writes a copy of
`INPUT_PDF` with that outline to `OUTPUT_PDF`. `INPUT_PDF` is never modified:
the output is a separate file (or standard output, `-o -`) and the same-file
and `--force` rules above apply to both inputs. At most one input can be `-`.

Before any output byte is written, the command checks that `SOURCE_CAJ` is a
valid CAJ file with at least one bookmark and that `INPUT_PDF` is a PDF that
the core reader accepts and that has no outline. A PDF that already has an
outline is refused rather than copied unchanged. The core appends the new
outline as an incremental update and rejects bookmarks whose pages are not in
the PDF, empty titles, and outlines deeper than 256 levels. Such a late error
removes the staged output file; bytes already sent to standard output remain.

## Exit status and streams

| Status | Meaning |
| --- | --- |
| 0 | Success, including `--help` and `--version`. |
| 1 | I/O, conversion, unsupported-format, refused-output, or inspection failure. |
| 2 | Invalid arguments, including a PDF input without a distinct output. |

Errors are written to standard error as `caj2pdf: error: MESSAGE`. Argument
errors add a line pointing to `--help`. Help and version text go to standard
output. The command prints no progress output and nothing on success.

## Verification

`crates/caj2pdf-cli/src/tests.rs` covers argument parsing, output-path
derivation, signatures, JSON escaping, report rendering, spooling limits,
same-file detection, and staged-output commit and cleanup. The process tests
in `crates/caj2pdf-cli/tests/cli.rs` run the built executable against
synthetic CAJ, KDH, C8, and HN containers and the MIT PDF fixtures. They cover
file, pipe, and standard-output conversion, stdin spooling, existing and
same-path refusal, a full-device stdout sink, terminal refusal (through
util-linux `script`), malformed and unsupported
inputs, non-UTF-8 names, `inspect` text and JSON, and `add-bookmarks`. Output
PDFs are checked with `qpdf --check`, `qpdf --show-npages`, and
`mutool show outline`.

```sh
cargo test --locked -p caj2pdf-cli
```


## HN/C8 integration validation

Original tests convert an asymmetric 3×2 type-0 HN-A page from a file and stdin,
reopen the PDF with qpdf and extract exact packed pixels. They also exercise
state-file bounds/syntax, malformed input, invalid state overrides, HN-B empty-row
rejection, state-file/hardlink overwrite protection, scratch creation failure,
existing-output preservation, anonymous-file cleanup and CAJ bookmark omission.

The release CLI converted the external four-page C8 issue-58 document described
in [the direct-record comparison](hnc8-direct-text.md), using a caller-supplied
MQ file and `--no-bookmarks`. All four pages passed qpdf; the 3,992,137-byte
PDF is byte-identical to the native example, Node and Chromium outputs:
`fffa38e8f2cd675352108488117f13983f959ead7500f9f4ba1cab9a2e74ef1e`.
The final run took 2.46 seconds and its scratch directory was empty.
Linux child-process `ru_maxrss` reported 14,844 KiB from a Python
subprocess harness. That process-lifetime measurement can include pre-exec
launcher overhead; it is not a precise core-allocation measurement or a
benchmark. Validation/hash allocations ran after conversion in the parent.
External documents, tables and output PDFs remain outside Git. This covers one
C8 document; broader compatibility remains a release check (#14), and
C8/HN-B outline semantics are still unverified.

The complete 163-page multi-image HN-A run, including 96 source bookmarks,
PDF structure/rendering checks and cross-interface hashes, is recorded in
[public-interface validation](js-validation.md#complete-multi-image-hn-a-public-interface-check).

## Interruption

SIGINT (Ctrl+C) and SIGTERM request cooperative cancellation. During conversion,
inspection and bookmark import, the core checks the request between bounded I/O
and decoder work units. A cancelled conversion exits with status 1, removes its
staged output, and leaves an existing destination unchanged. Bytes already sent
to stdout cannot be recalled.

An operating-system read or write can remain blocked until it returns. A second
termination signal forces the normal signal action; forced termination (including
SIGKILL) can leave a named output temporary file. Anonymous scratch files are
released by the operating system. Do not rely on forced termination for cleanup.


## Native platform adapters

The Windows CLI uses the same conversion, argument and report code as Unix.
Windows identity checks use volume and file indexes, including hard-link
aliases. Paths remain native OS strings. Temporary files inherit their parent
directory's ACL; use a private user temp/output directory. Scratch files are
marked for deletion through their open handles and removed when closed.

On Windows, Ctrl-C/Ctrl-Break request cooperative cancellation; a second
interrupt exits with status 130 immediately and may leave staged output.
On Unix, SIGINT/SIGTERM retain the documented cooperative/second-signal behavior.
Blocking OS I/O and forced process termination cannot guarantee normal cleanup.
The Linux 100% instrumented-line gate measures Linux-compiled Rust; separate
Windows execution tests validate the Windows-only adapter, without claiming
100% cross-platform coverage.

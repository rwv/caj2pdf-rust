# Command-line interface

This note documents the `caj2pdf` executable delivered for
[issue #12](https://github.com/rwv/caj2pdf-rust/issues/12). The command is a
thin Linux adapter over the platform-neutral core: it opens files, spools
forward-only input, stages path output, and chooses the core operation from
the input's leading signature. Format parsing and PDF writing stay in
`caj2pdf-core`. Versions `v0.x.y` may change this interface; such changes are
listed in the release notes.

```text
caj2pdf INPUT [-o OUTPUT] [--force]
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
| `HN` | HN | **Not implemented**; exits with status 1 | Container variant and pages |
| `c8 00 00 00` | C8 | **Not implemented**; exits with status 1 | Container variant and pages |
| `TEB` | TEB | Unsupported; exits with status 1 | Format only |

Before reporting that HN or C8 conversion is not implemented, the command
parses the container header and page index, so a malformed container fails
with a parse error instead. HN and C8 conversion needs the image decoders
tracked under
[#9](https://github.com/rwv/caj2pdf-rust/issues/9) and
[#23](https://github.com/rwv/caj2pdf-rust/issues/23). No NH signature has been
measured, so NH input is reported as an unrecognized format. Listing the
outline entries of a PDF or KDH input is not implemented: `inspect` reports
only whether such a document has an outline.

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
| `conversion_supported` | boolean | Whether this build converts the format. |
| `page_count` | integer or null | Declared page count; null when unknown (TEB). |
| `has_outline` | boolean or null | Whether the document has an outline; null when unknown (HN, C8, TEB). |
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

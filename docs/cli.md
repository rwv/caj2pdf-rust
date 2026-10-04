# Command-line interface

This note documents the `caj2pdf` executable delivered for
[issue #12](https://github.com/rwv/caj2pdf-rust/issues/12). The command is a
thin native adapter over the platform-neutral core: it opens files, spools
forward-only input, stages path output, and chooses the core operation from
the input's leading signature. Format parsing and PDF writing stay in
`caj2pdf-core`. Versions `v0.x.y` may change this interface; such changes are
listed in the release notes.

```text
caj2pdf INPUT [-o OUTPUT] [--force] [--quiet] [--no-bookmarks] [--qm-states FILE] [--mq-states FILE]
caj2pdf inspect INPUT [--json] [--bookmarks] [--pages]
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
| `%PDF-` (byte 0, else within the first 1,024 bytes) | PDF | Validated copy through the core PDF reader and repair layer | Pages and outline presence |
| `CAJ` | CAJ | Reconstructed PDF with the CAJ outline | Pages and full outline |
| `KDH` | KDH | Decoded embedded PDF | Pages and outline presence |
| `HN` | HN | Experimental image-page conversion with built-in standard codec states; HN-A pages are images, not searchable text ([why](hnc8-text-fidelity.md#hn-a-pages-carry-no-native-text)) | Variant/pages; HN-A full outline, HN-B outline unknown |
| `c8 00 00 00` | C8 | Experimental image pages; admitted native text/mixed pages with explicit fonts | Container variant and pages |
| `TEB` | TEB | Unsupported; exits with status 1 | Format only |

HN/C8 routes use the same independently implemented core page composer as
WASM. Malformed data and unsupported layouts produce located errors. No NH
signature has been measured, so NH input remains unrecognized. A PDF header
after leading bytes is read from the header; see the
[header offset rule](pdf-input.md#header-offset). PDF/KDH
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
- `--no-bookmarks`: skip CAJ/HN outline import. C8/HN-B outline layouts are
  not verified, so without this flag they convert with no outline and one
  warning on standard error (`C8/HN-B bookmarks are not verified; wrote no
  outline`); the flag gives the same PDF silently. Existing embedded PDF/KDH
  outlines are not removed by this flag.

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
C8/HN-B outlines are omitted with a warning (see `--no-bookmarks`). Admitted native-text pages require the explicit
font roles below; unverified profiles are rejected. Image-only pages receive
no OCR text layer. General text extraction and semantic reading order remain
outside the [verified text scope](hnc8-text-fidelity.md). The HN/C8 route admits the measured unused-refinement-template anomaly; other
malformed JBIG2 flags remain errors.

The command creates four private anonymous files in `TMPDIR` (or the system
temporary directory), each capped at 64 MiB by the composition budget. The
names are removed before conversion; the OS releases storage when handles
close, including on process exit. This reuses input spooling's file helper.
Forward-only document input is separately spooled within its input limit.
Output remains sequential; a failed conversion never commits a staged path
output. Stdout can contain a partial PDF on failure, as for other formats.

### Native C8 font resources

Native C8/HN-B text pages need a CJK and a Latin font. Put them in one
directory under fixed names and pass `--fonts DIR`:

| Role | File in `DIR` | Per-role flag |
| --- | --- | --- |
| CJK (required) | `cjk.ttf` | `--font-cjk FILE` |
| Ordinary Latin (required) | `latin.ttf` | `--font-latin FILE` |
| Alternate Latin (`801d/4`) | `alternate-latin.ttf` | `--font-alternate-latin FILE` |
| Decoration alias | `decoration.ttf` | `--font-decoration FILE` |
| HN-B mode-0 symbols | `symbols.ttf` | `--font-symbols FILE` |
| Latin states `801d/3`, `/28`, `/31` | `latin-state3.ttf`, `latin-state28.ttf`, `latin-state31.ttf` | `--font-latin-state3/28/31 FILE` |

A per-role flag overrides the directory's file for that role, and the
per-role flags alone also work: `--font-cjk FILE --font-latin FILE` is the
minimum. Only these names are looked up in `DIR`; there is no font
discovery, system lookup or bundled font. A missing `cjk.ttf` or `latin.ttf`
fails before output staging. Any other missing file leaves its role absent.

**Fallback rule.** Each glyph uses the font of the role the source selects.
If that role is absent, or its font has no cmap entry for the character,
the glyph uses the CJK font for CJK-coded characters (U+2E80–U+9FFF,
U+F900–U+FAFF, U+FE10–U+FE1F, U+FE30–U+FE6F and halfwidth/fullwidth forms
U+FF00–U+FFEF) and the Latin font for everything else, including the
decoration alias. If that font lacks the glyph too, conversion fails with
the page and source byte. Fallback picks only a font. Glyph positions
stay the same, so a substitute font can still look different or overlap.

#### Tested free-font recipe

These two TrueType fonts cover every glyph of the six pinned C8/HN-B corpus
documents. The converter reads only standalone TrueType (`glyf`) fonts,
not CFF/OpenType `.otf` or `.ttc` collections. That rules out Noto Sans CJK.
Noto Sans also lacks math symbols those documents use, such as U+2217.

```sh
sudo apt-get install -y fonts-droid-fallback fonts-dejavu-core
mkdir -p ~/caj2pdf-fonts
ln -s /usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf ~/caj2pdf-fonts/cjk.ttf
ln -s /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf ~/caj2pdf-fonts/latin.ttf
caj2pdf input.caj --no-bookmarks --fonts ~/caj2pdf-fonts -o output.pdf
```

Droid Sans Fallback is Apache-2.0. DejaVu Sans uses the free Bitstream Vera
license. Both stay external resources: they are not vendored or bundled.
The output is not a faithful copy of the source typography: weights,
bearings and widths differ from the CAJViewer fonts.

#### Notes

The decoration alias defaults to `►`. Use `--decoration-char CHAR` to pick
a different single BMP Unicode scalar from the decoration font. It requires
`--font-decoration` or `--fonts DIR`, and `DIR` must then contain
`decoration.ttf`. The alias is not emitted as document text.
Coverage alone does not guarantee compatible glyph widths or bearings: a
substitute can overlap at the fixed source positions, including in the viewer.
See the [C8 same-resource controls](c8-real-font-fidelity.md#same-resource-control-follow-up).
At most eight distinct sources are accepted. Node/browser expose the same
roles, and the same fallback, as `hnc8.fonts` options.

Font path flags accept separate values or `--font-cjk=FILE` spelling, and
`--fonts DIR` or `--fonts=DIR`. Separate values preserve non-UTF-8 paths.
Repeating a role or `--fonts`, giving per-role flags without both
`--font-cjk` and `--font-latin` (and without `--fonts`), or using `-` as a
font path is a usage error. Reuse the same path for multiple roles to embed
it once. Native C8/HN-B font options are rejected for other document formats.

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
`bookmarks` now contain validated metadata rather than unknown values. Known
empty outlines produce `false`, zero and an empty list. C8/HN-B remain unknown.

### HN-A bookmark defects

A malformed HN-A outline entry is a bookmark defect, not a document defect.
`convert` and `inspect` skip an entry whose title is not NUL-terminated
GB18030, whose page is not a NUL-terminated decimal within the source pages,
or whose level is zero. Later entries that were its children are re-parented
to the nearest written ancestor rather than dropped. An entry that skips a
parent level, or is deeper than the 64-level limit, is kept one level below
its written parent. The rest of the outline and every page are written, and
the command exits 0.

Each defect is reported on standard error with its absolute source byte offset:

```text
caj2pdf: warning: skipped HN-A bookmark at byte 1244: destination is outside source pages
caj2pdf: warning: re-parented HN-A bookmark at byte 960: level skips a parent
```

At most 16 locations are listed, followed by
`caj2pdf: warning: N more HN-A bookmark defects were not listed`. A table that
cannot be read at all still fails as before: an outline count outside the
container, a short or failed read, a bookmark limit, or cancellation.
`--no-bookmarks` does not read the outline and reports nothing.

**v0.x behavior change:** these entries previously failed `convert` and
`inspect` with `malformed outline ...` and exit status 1. Scripts that relied on
that failure should check `outline_warnings` in `inspect --json`.

### C8 application info

Some C8 files end with an application-info package (see
[the C8 record notes](c8-native-records.md#package-framing-and-reader-302)).
`inspect` reports its DOI, URL and annotation count. Conversion writes a PDF
document information dictionary only when a DOI or URL is present:
the custom keys `/CNKI_DOI` and `/CNKI_URL` hold the verbatim identifier and
URL as UTF-16BE text strings. No title is invented
and annotations are not converted. Other inputs and C8 files without a package
produce unchanged output.

A defective package never fails `convert` or `inspect`. The PDF is written
exactly as without a package, and one located line is printed on standard
error, for example:

```text
caj2pdf: warning: ignored C8 application-info package at byte 39433: application-info XML: malformed start tag
```

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
`unknown`. An HN-A outline with [bookmark defects](#hn-a-bookmark-defects)
adds an `Outline warnings: N` line after `Bookmarks:`. A C8 source with a
readable [application-info package](#c8-application-info) adds
`DOI:` and `URL:` lines (each only when present; control characters escaped as
in titles) and a `Notes: N` line after the outline lines, before any
[`--pages`](#structure-report---pages) structure lines.

### JSON schema, version 1

`--json` writes one compact JSON object followed by a newline. Fields appear
in the order below. A later incompatible change increments `schema_version`;
adding a field is not considered incompatible, so the `outline_warnings`,
`unsupported_reason`, `application_info`, `structure` and `pages` fields were
added without changing version 1.

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | integer | Always `1` for this schema. |
| `format` | string | `"PDF"`, `"CAJ"`, `"KDH"`, `"HN"`, `"C8"`, or `"TEB"`. |
| `variant` | string or null | Measured HN/C8 container layout: `"C8"`, `"HN-A"`, or `"HN-B"`; otherwise null. |
| `conversion_supported` | boolean | Whether this build has a conversion route; HN/C8 still require a supported profile. |
| `page_count` | integer or null | Declared page count; null when unknown (TEB). |
| `has_outline` | boolean or null | Whether the document has an outline; null when unknown (HN-B, C8, TEB). |
| `bookmark_count` | integer or null | Number of outline entries that conversion writes; null when this format's outline cannot be listed. |
| `bookmarks` | array or null | Present only with `--bookmarks`. The root entries, or null when the outline cannot be listed. |
| `outline_warnings` | integer or null | Number of [HN-A bookmark defects](#hn-a-bookmark-defects) skipped or re-parented; `0` for other listed outlines; null when `bookmark_count` is null. |
| `unsupported_reason` | string | Present only when a recognized format is never converted: `"drm-encrypted"` for TEB, whose document content is encrypted. |
| `application_info` | object | Present only for a C8 source with a readable [application-info package](#c8-application-info); omitted otherwise, including when a defective package is ignored. |

The `application_info` object has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `doi` | string or null | The package's `DOI` text, verbatim; observed values are CNKI identifiers, not checked as registered DOIs. |
| `url` | string or null | The package's `DURL` text. It is never fetched. |
| `note_count` | integer | Number of annotation entries (`NoteItems/Item`); they are not converted. |

Each bookmark object has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `title` | string | Title decoded to Unicode. |
| `page` | integer | One-based destination page. |
| `children` | array | Nested bookmark objects in document order; empty for a leaf. |

Example:

```json
{"schema_version":1,"format":"CAJ","variant":null,"conversion_supported":true,"page_count":3,"has_outline":true,"bookmark_count":3,"bookmarks":[{"title":"Introduction","page":1,"children":[{"title":"Background","page":2,"children":[]}]},{"title":"Methods","page":3,"children":[]}],"outline_warnings":0}
```

Strings escape `"`, `\`, and control characters as required by RFC 8259;
other characters are written as UTF-8.

### Structure report (`--pages`)

`--pages` adds a structure-only report for diagnosing a document that fails
to convert without sharing it ([#301](https://github.com/rwv/caj2pdf-rust/issues/301)).
It contains offsets, lengths, counts, header words and located reader errors;
it never contains document text, titles (unless `--bookmarks` is also given),
or image bytes, so it is meant to be pasted into a public issue. It adds no
format interpretation: each page is checked by the readers that conversion
already uses.

- HN/C8: the page-index offset, length and row size (20 bytes, or 12 for the
  compact HN-B layout selected by a zero word at `0x88`), the raw native mode
  (C8 `0x0c`, HN-B `0x94`) and origin, the declared page size, and whether
  the input ends with an `APPINFOSIGN <decimal offset>` trailer. Only the
  trailer's declared start and the byte length to the end of the input are
  reported here, for any variant and even when the package is defective; the
  values decoded from a C8 package are the separate top-level
  `application_info` field and `DOI:`/`URL:`/`Notes:` lines, which do not
  need `--pages` ([C8 application info](#c8-application-info)). Then
  one record per page: its text span, image descriptors as type and payload
  span, and the text framing accepted by the existing readers. HN-A/C8 text
  is checked by the compressed or raw page-text reader, with the composer's
  repeated coordinate-group rule; a C8 span without a compressed header, and
  every HN-B span, is framed as native records.
- KDH: the observed leading 32 bytes as ASCII (`\xNN` for any other byte,
  quote or backslash) and whether they equal the supported
  `KDH 2.00 Copyright(C) 2000 CAJCD`. With `--pages` a different signature is
  reported with unknown page count and outline and exit status 0; without
  `--pages` it remains an error.
- CAJ, PDF and TEB: no per-page records (`Page structure: not available`).

| Text framing | Meaning |
| --- | --- |
| `none` | The indexed text span is empty. |
| `raw` | Uncompressed HN-A records. |
| `raw-paired` | Uncompressed HN-A records after the paired `8003` page-size prefix. |
| `compresstext` | A 16-byte direct `COMPRESSTEXT` header and one zlib frame. |
| `legacy-24` | The 24-byte paired-`8003` and `COMPRESSTEXT` header and one zlib frame. |
| `native` | C8/HN-B native records. |

A page whose row or image descriptor is rejected reports that `error` and
the remaining pages are still inspected, each with a fresh reader. A text
reader failure is a `text_error`; it does not prove that image-only
conversion fails, because HN-B image-only conversion does not read native
records. Cancellation and read failures end the command with status 1; the
report already written to standard output is then incomplete.

Memory stays bounded: pages and descriptors are written as they are read,
and each page's text is checked with the conversion text budget
(`TextBudget::default()`). Image payloads are never read.

Text form, after the usual lines (the synthetic HN-A test input):

```text
Page index: 348+80 (20-byte rows)
Native mode: unknown
Native origin: unknown
Page size: 100 200
Application info: none
Page 1: text 428+32, images [type 0 at 472+49], framing raw (2 records)
Page 2: text 521+40, images [type 0 at 573+49], text error: HN/C8 HN-A at byte 521, page 2: malformed decoded text record: unknown control tag
Page 3: text 622+32, images [], error: HN/C8 HN-A at byte 658, page 3, image 1: truncated image payload: expected 1073741824 bytes, available 49
Page 4: error: HN/C8 HN-A at byte 408, page 4: truncated text span: expected 4 bytes, available 0
```

The JSON object gains two fields at the end, after `outline_warnings`,
`unsupported_reason` and `application_info`, only with `--pages`; as additive
fields they keep `schema_version` 1:

| Field | Type | Meaning |
| --- | --- | --- |
| `structure` | object or null | HN/C8 or KDH document structure below; null for other formats. |
| `pages` | array or null | One object per HN/C8 page in order; null for other formats. |

HN/C8 `structure` fields: `page_index_offset`, `page_index_length` and
`page_row_bytes` (integers); `native_mode` (integer or null for HN-A);
`native_origin` and `page_size` (`[x, y]` integer pairs or null); and
`application_info`, null or `{"offset": integer, "length": integer or null}`
where `length` is null when the declared offset is outside the input. This
`structure.application_info` only locates the trailer; it is not the
top-level `application_info` object, which holds the parsed DOI, URL and note
count of a readable C8 package.
KDH `structure` fields: `kdh_signature` (string) and
`kdh_signature_supported` (boolean).

Each page object has these fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `page` | integer | One-based source page. |
| `text_offset`, `text_length` | integer or null | Indexed text span; null when the page row was rejected. |
| `image_count` | integer or null | Declared image descriptors; null when the page row was rejected. |
| `images` | array | Descriptors read before any error: `{"type": integer, "offset": integer, "length": integer}` with the payload span. |
| `text_framing` | string or null | One of the framings above; null when the text was not accepted or not reached. |
| `text_records` | integer or null | Glyph, raw or native records counted by the accepting reader. |
| `text_decoded_length` | integer or null | Inflated length of a compressed frame; otherwise null. |
| `text_error` | string or null | The deciding text reader's located error. |
| `error` | string or null | The page-row or descriptor error that stopped this page. |

Example (a one-page HN-A input):

```json
{"schema_version":1,"format":"HN","variant":"HN-A","conversion_supported":true,"page_count":1,"has_outline":false,"bookmark_count":0,"outline_warnings":0,"structure":{"page_index_offset":348,"page_index_length":20,"page_row_bytes":20,"native_mode":null,"native_origin":null,"page_size":[100,200],"application_info":null},"pages":[{"page":1,"text_offset":368,"text_length":32,"image_count":1,"images":[{"type":0,"offset":412,"length":49}],"text_framing":"raw","text_records":2,"text_decoded_length":null,"text_error":null,"error":null}]}
```

The JavaScript `inspect` API does not expose this report yet.

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
output. On success it prints nothing except `caj2pdf: warning: ...` lines for
[HN-A bookmark defects](#hn-a-bookmark-defects), which keep exit status 0, and,
when standard error is a terminal, a single updating `caj2pdf: reading input
NN%` line during conversion. The percentage is the furthest input byte read;
the line is erased before exit. `-q`/`--quiet` disables it, and it is never
written when standard error is redirected.

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

For HN-B profiles that require them, `--font-symbols FILE` supplies semantic
mode-0 symbols and spaces, and `--font-latin-state3 FILE` supplies the distinct
Latin resource selected by state `801d/3`. Both use the existing bounded font
reader. Missing required resources fail explicitly; supplying a substitute
font does not establish source typeface fidelity.

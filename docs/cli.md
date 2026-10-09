# Command-line interface

This note documents the `caj2pdf` executable delivered for
[issue #12](https://github.com/rwv/caj2pdf-rust/issues/12). The command is a
thin native adapter over the platform-neutral core: it opens files, spools
forward-only input, stages path output, and chooses the core operation from
the input's leading signature. Format parsing and PDF writing stay in
`caj2pdf-core`. Versions `v0.x.y` may change this interface; such changes are
listed in the release notes.

```text
caj2pdf INPUT [-o OUTPUT] [--force] [--quiet] [--allow-damaged] [--no-bookmarks] [--no-system-fonts]
caj2pdf inspect INPUT [--json] [--bookmarks] [--pages]
caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]
caj2pdf --help | --version
```

A subcommand name is recognized only as the first argument. Convert a file
named `inspect` as `caj2pdf ./inspect`. Options may appear before or after
positional arguments; `--` ends option parsing. `-o FILE`, `-oFILE`,
`-o=FILE`, `--output FILE`, and `--output=FILE` are equivalent. Every path
argument, in every spelling, is kept as an `OsString`, so non-UTF-8 Linux file
names work.
Diagnostics shown on standard error replace invalid UTF-8 with U+FFFD.
Arguments are parsed with [clap](https://crates.io/crates/clap); `--help` (or
`caj2pdf COMMAND --help`) lists every option of a command.

## TTKN response input (unreleased)

`--ttkn-response-file FILE` supplies the matching case-sensitive 32-character
ASCII response for the [measured TTKN PDF profile](ttkn-pdf.md). The converter
performs no network authentication. The option applies to conversion only;
other encrypted profiles remain unsupported.

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
| `HN` | HN | Experimental image-page conversion with built-in standard codec states; HN-A pages are images, not searchable text ([why](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-text-fidelity.md#hn-a-pages-carry-no-native-text)) | Variant/pages; HN-A full outline, HN-B outline unknown |
| `c8 00 00 00` | C8 | Experimental image pages; admitted native text/mixed pages with installed or given fonts | Container variant and pages |
| `[TARGET]` and all observed CAA fields within 1,024 bytes | CAA | Target descriptor; exits with status 1 and asks for the referenced document | Format only; counts unknown |
| `TEB` | TEB | Unsupported; exits with status 1 | Format only |

HN/C8 routes use the same independently implemented core page composer as
WASM. Malformed data and unsupported layouts produce located errors. The
measured `.nh` file has an `HN` signature and follows the HN-A path; no separate
`NH` signature is registered. CAA detection requires the complete observed
field sequence, never just `[TARGET]` or the extension. Its opaque values are
not decoded, resolved over the network, or interpreted as page counts.
A PDF header
after leading bytes is read from the header; see the
[header offset rule](pdf-input.md#header-offset). PDF/KDH
inspection reports outline presence rather than full outline entries;
HN-A inspection also validates and lists its outline. C8/HN-B outline metadata
remains unknown.
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
unit. See the [controlled field checks](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/cajviewer-hnc8-kdh.md#controlled-geometry-checks).

### Experimental HN/C8 options

- `--allow-damaged`: explicitly replace damaged CAJ pages with blanks, warn for
  every affected page, and return status 3 when substitutions occur. See
  [partial conversion](pdf-input.md#explicit-partial-conversion-of-damaged-caj-inputs).
- `--no-bookmarks`: skip CAJ/HN outline import. C8/HN-B outline layouts are
  not verified, so without this flag they convert with no outline and one
  warning on standard error (`C8/HN-B bookmarks are not verified; wrote no
  outline`); the flag gives the same PDF silently. Existing embedded PDF/KDH
  outlines are not removed by this flag.

HN/C8 conversion always uses the built-in standard T.82/T.88 codec states.
The `--qm-states` and `--mq-states` overrides were removed (#348).
HN-A outlines are supported;
C8/HN-B outlines are omitted with a warning (see `--no-bookmarks`). Admitted native-text pages require the
fonts below, installed or given; unverified profiles are rejected. Image-only pages receive
no OCR text layer. General text extraction and semantic reading order remain
outside the [verified text scope](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-text-fidelity.md). The HN/C8 route admits the measured unused-refinement-template anomaly; other
malformed JBIG2 flags remain errors.

A conversion creates no temporary files (#355): type-3 symbol and text
bitmaps are held in memory, each capped by `max_allocation_bytes`, and
type-0 and JPEG images stream. Only forward-only document input (stdin) is
spooled, within its input limit, to an anonymous file in `TMPDIR`.
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
minimum. In `DIR`, each role is looked up as `NAME.ttf`, `NAME.otf`, then
`NAME.ttc` (the first face of a collection). Without any font option, the
[installed fonts](#installed-fonts) are searched instead; no font is bundled.
A missing CJK or Latin font fails before output staging.
Any other missing file leaves its role absent.

Fonts are static OpenType fonts with TrueType (`glyf`) or CFF outlines,
standalone (`.ttf`, `.otf`) or as a face of a collection (`.ttc`), such as
Windows `simsun.ttc` and `msyh.ttc`, `wqy-zenhei.ttc` or Noto Sans/Serif
CJK. `FILE#N` selects face `N` (from 0) of a collection, for example
`--font-cjk /usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc#2`
(Noto Serif CJK SC). Variable fonts (`fvar`, `CFF2`) are rejected.
The suffix needs a Unicode path and is ignored when `FILE#N` itself
exists. Tables need not be 4-byte aligned.

**Which documents use the fonts.** Fonts can be passed for any HN/C8 input.
Before converting, the header and the first page with text are inspected.
A C8 or HN-B document in a native rendering mode (`native_mode` 2, or 0 for
HN-B) whose first text holds native records uses native composition with the
fonts: any HN-B text, and C8 text that `inspect --pages` reports as `native`
or that neither text reader accepts, so a defect is reported by native
composition. All other documents, including every HN-A input and C8
documents with `COMPRESSTEXT` or raw text, are converted as images without
reading the fonts; the PDF is byte-identical to a conversion without fonts.
A C8 document mixing native and compressed text fails in either composer,
at the first page it cannot draw. Node and the browser follow the same rule.

**Fallback rule.** Each glyph uses the font of the role the source selects.
If that role is absent, or its font has no cmap entry for the character,
the glyph uses the CJK font for CJK-coded characters (U+2E80–U+9FFF,
U+F900–U+FAFF, U+FE10–U+FE1F, U+FE30–U+FE6F and halfwidth/fullwidth forms
U+FF00–U+FFEF) and the Latin font for everything else, including the
decoration alias. If that font lacks the glyph too, conversion fails with
the page and source byte. Fallback picks only a font. Glyph positions
stay the same, so a substitute font can still look different or overlap.

#### Installed fonts

When no font option (`--fonts`, `--font-*`) is given and the document routes
to native composition (the rule above), the CLI searches the installed fonts
for one CJK and one Latin face. Image documents never search. Any font option
disables the search entirely, so explicit fonts always win and the search
never fills a role you left out. `--no-system-fonts` disables it too: the
document is then converted without fonts, so a C8 document with native
pages, or an HN-B document with a text-only page, fails, while an HN-B
document whose pages all have images converts as images without its text.
Node and the browser never search: they take only explicit fonts.

The directories, in order:

| Platform | Directories |
| --- | --- |
| Linux and other Unix | `$XDG_DATA_HOME/fonts` (default `~/.local/share/fonts`), `~/.fonts`, then `DIR/fonts` for each `$XDG_DATA_DIRS` entry (default `/usr/local/share:/usr/share`) |
| macOS | `~/Library/Fonts`, `/Library/Fonts`, `/System/Library/Fonts` |
| Windows | `%LOCALAPPDATA%\Microsoft\Windows\Fonts`, `%WINDIR%\Fonts` |

`CAJ2PDF_FONT_DIRS`, a path list (`:`-separated, `;` on Windows), replaces
these directories; set it empty to search nothing. Relative entries are
ignored. The walk is deterministic and bounded: entries are visited in byte
order of their names, at most 6 directory levels below each searched directory, and
at most 20,000 directory entries in total (a note is printed if the bound
stops it; which entries of the last directory were read then depends on
the file system). Symbolic links to files are followed, links to directories are
not, and a directory reached twice is walked once, so a link cycle cannot
loop. Unreadable directories and entries are skipped.

Only files with the names below (compared without case) are opened. A face
is accepted when the font reader validates it and its PostScript name
matches; at most 64 faces of a collection are checked. The first face found
in list order wins, whatever its directory.

| Role | Faces in order (PostScript name; file names) |
| --- | --- |
| CJK | Noto Serif CJK SC (`NotoSerifCJKsc-Regular`; `NotoSerifCJK-Regular.ttc`, `NotoSerifCJKsc-Regular.otf`), Source Han Serif SC (`SourceHanSerifSC-Regular`; `SourceHanSerif-Regular.ttc`, `SourceHanSerifSC-Regular.otf`), SimSun (`SimSun`; `simsun.ttc`), Songti SC (`STSongti-SC-Regular`; `Songti.ttc`), Noto Sans CJK SC (`NotoSansCJKsc-Regular`; `NotoSansCJK-Regular.ttc`, `NotoSansCJKsc-Regular.otf`), Source Han Sans SC (`SourceHanSansSC-Regular`; `SourceHanSans-Regular.ttc`, `SourceHanSansSC-Regular.otf`), Microsoft YaHei (`MicrosoftYaHei`; `msyh.ttc`, `msyh.ttf`), PingFang SC (`PingFangSC-Regular`; `PingFang.ttc`), WenQuanYi Zen Hei (`WenQuanYiZenHei`; `wqy-zenhei.ttc`), Droid Sans Fallback (`DroidSansFallback`; `DroidSansFallbackFull.ttf`, `DroidSansFallback.ttf`) |
| Latin | FreeSerif (`FreeSerif`; `FreeSerif.ttf`, `FreeSerif.otf`), DejaVu Sans (`DejaVuSans`; `DejaVuSans.ttf`), Nimbus Roman (`NimbusRoman-Regular`; `NimbusRoman-Regular.otf`), DejaVu Serif (`DejaVuSerif`; `DejaVuSerif.ttf`), Liberation Serif (`LiberationSerif`; `LiberationSerif-Regular.ttf`), Times New Roman (`TimesNewRomanPSMT`; `times.ttf`, `Times New Roman.ttf`), Times (`Times-Roman`; `Times.ttc`) |

Serif CJK faces come first because the documents are printed in Song/Ming
style; sans faces follow. The Latin order comes from the measurement below:
faces that cover every Latin-font glyph of the pinned documents come first.
The choice is printed to standard error before conversion, in the `FILE#N`
form the options accept (`#N` only for a face other than 0); `-q` silences it:

```text
caj2pdf: using installed CJK font /usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc#2 (NotoSerifCJKsc-Regular)
caj2pdf: using installed Latin font /usr/share/fonts/truetype/freefont/FreeSerif.ttf (FreeSerif)
```

If either role is not found, conversion fails before output staging with a
message naming the missing role, the directories searched, the font options
and the `--no-system-fonts` image fallback for HN-B. This keeps native text
from being dropped silently.
The search reads the document's text framing once more before conversion
(bounded, no image payload) and opens only the listed files.

#### Recommended free fonts

On Debian and Ubuntu, this pair converts every pinned document with no font
option:

```sh
sudo apt-get install -y fonts-noto-cjk fonts-freefont-ttf
caj2pdf input.caj --no-bookmarks -o output.pdf
```

Noto Serif CJK is SIL OFL 1.1 and FreeSerif is GPL-3.0 with the font
exception. Both stay external resources: they are not vendored or bundled.
The output is not a faithful copy of the source typography: weights,
bearings and widths differ from the CAJViewer fonts.

**Measurement.** The six pinned C8/HN-B documents (issues 63, 65, 66,
90 `4-[21]` and `4-[24]`, 100) were converted with Noto Serif CJK SC and
each Latin candidate, and with FreeSerif and each CJK candidate. A face
"misses" a character when it lacks a glyph the document draws with the Latin
font (94 distinct characters over the six documents; a CJK-coded character
could fall back to the CJK font). "Overrun" counts adjacent glyph pairs on a
line where the face's advance passes the next glyph's source position by
more than 5% of the font size: high values look crowded or overlap.

| Latin face | Documents converted | Missing characters | Overrun pairs |
| --- | --- | --- | --- |
| FreeSerif | 6/6 | none | 924 / 6,764 |
| DejaVu Sans | 6/6 | none | 3,894 / 6,764 |
| Nimbus Roman (`.otf`) | 4/6 | `①`–`⑦`, `┆` | 957 / 6,762 |
| DejaVu Serif | 4/6 | `①`–`⑦` | 4,702 / 6,764 |
| Liberation Serif | 1/6 | `∗ ∥ ∪ ①`–`⑦ ┆` | 957 / 6,760 |
| Noto Serif | 1/6 | 15, including `∑ ∗ ∞ ①`–`⑦ ►` | 4,026 / 6,715 |
| Caladea | 1/6 | 16, including `Ω δ ε θ ∗ ①`–`⑦` | 1,057 / 6,716 |
| TeX Gyre Termes (`.otf`) | 2/6 | `∥ ∪ ①`–`⑦ ┆` | not measured |

FreeSerif's advances are Times-like (the Times-metric faces overrun least),
so it has both full coverage and the closest widths; DejaVu Sans covers
everything but is wider. Times New Roman and Times were not available for
measurement; they are listed last for systems without the free faces. All
measured CJK candidates (Noto Serif CJK SC, Noto Sans CJK SC, WenQuanYi
Zen Hei, Droid Sans Fallback) converted all six documents with FreeSerif;
Droid Sans Fallback lacks `l` and `p`, which then use the Latin font.
Source Han faces carry the same glyph set as the Noto CJK faces.

#### Notes

The decoration alias defaults to `►`. Use `--decoration-char CHAR` to pick
a different single BMP Unicode scalar from the decoration font. It requires
`--font-decoration` or `--fonts DIR`, and `DIR` must then contain
`decoration.ttf`. The alias is not emitted as document text.
Coverage alone does not guarantee compatible glyph widths or bearings: a
substitute can overlap at the fixed source positions, including in the viewer.
See the [C8 same-resource controls](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-real-font-fidelity.md#same-resource-control-follow-up).
At most eight distinct sources are accepted. Node/browser expose the same
roles, and the same fallback, as `hnc8.fonts` options.

Font path flags accept separate values or `--font-cjk=FILE` spelling, and
`--fonts DIR` or `--fonts=DIR`. Both spellings preserve non-UTF-8 paths.
Repeating a role or `--fonts`, giving per-role flags without both
`--font-cjk` and `--font-latin` (and without `--fonts`), or using `-` as a
font path is a usage error. Reuse the same path for multiple roles to embed
it once. Only the glyphs a document draws are embedded, as a compressed
subset, so the size of the supplied font file barely affects the output.
Native C8/HN-B font options are rejected for other document formats.

Font files are read twice: metadata before the first page, then metadata
and the drawn glyphs after the last page. They must not change during
conversion. Files are read through ranged/seekable handles. Forward-only named inputs
reuse the existing bounded temporary spooling path. Font files, including
hard-link/symlink aliases, are protected against output replacement even
with `--force`. A missing font fails before output staging; invalid fonts,
missing glyphs and later-page errors discard staged output and preserve an
existing destination. Standard output can still contain partial bytes on
failure. Fonts are installed or caller-provided and are not bundled with
the executable.

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
[the C8 record notes](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-records.md#package-framing-and-reader-302)).
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
  `--force`. The check compares the identity of each opened input that is a
  regular file (device and inode numbers on Unix, volume serial number and
  file index on Windows, through the `same-file` crate) with the output path
  after following symbolic links, so hard links, symbolic links, and
  differently spelled paths such as `./paper.caj` or `sub/../paper.caj` are
  all detected. Font files are protected the same way. Standard output is
  checked the same way when it is a regular file, such as `>> paper.caj`.
  Pipes, FIFOs, devices and consoles have no identity and are never
  compared: an input of that kind has already been spooled in full, and an
  output of that kind is subject only to the existence rule. Unix reads the
  output's identity from its metadata, so an unreadable existing output can
  still be replaced with `--force`; Windows has to open it, and an existing
  regular output file that cannot be opened for the comparison is refused
  there (`cannot open output '…' for identity check`).
- A path output is written to a new hidden temporary file in the output's
  directory (`.caj2pdf-XXXXXX.tmp` with six random letters and digits,
  created exclusively by the `tempfile` crate, with mode `0666` before the
  umask on Unix). The name does not depend on the output name, so an output
  name at the file-system limit still stages. The file is flushed and
  synchronized only after the whole conversion succeeds, and then given the
  target name. Every error path removes the temporary file. A process killed
  by a signal can leave it behind.
- The same-file check is repeated at that point, because the target may
  change during a conversion. Without `--force` the file is moved to the
  target name with an exclusive rename (`renameat2` with `RENAME_NOREPLACE`
  on Linux, `renameatx_np` with `RENAME_EXCL` on Apple systems, `MoveFileExW`
  without `MOVEFILE_REPLACE_EXISTING` on Windows) or, where that is not
  available, a hard link followed by removing the temporary name. Either
  fails atomically if any entry, including a dangling symbolic link, has
  appeared there. Only on a file system supporting neither does the command
  fall back to a re-check followed by a rename, which leaves a short race
  window.
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
  `/dev/stdin` or a FIFO, is copied in 64 KiB chunks to an anonymous file in
  `$TMPDIR` (or `/tmp`), created by `tempfile::tempfile_in`. On Linux it is
  opened with `O_TMPFILE`, so it never has a name; where that is not
  supported it is created with mode `0600` and unlinked before the copy
  starts. Either way its storage is released when the command exits,
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
| `format` | string | `"PDF"`, `"CAJ"`, `"KDH"`, `"HN"`, `"C8"`, `"CAA"`, or `"TEB"`. |
| `variant` | string or null | Measured HN/C8 container layout: `"C8"`, `"HN-A"`, or `"HN-B"`; otherwise null. |
| `conversion_supported` | boolean | Whether this build has a conversion route; HN/C8 still require a supported profile. |
| `page_count` | integer or null | Declared page count; null when unknown (CAA, TEB). |
| `has_outline` | boolean or null | Whether the document has an outline; null when unknown (HN-B, C8, CAA, TEB). |
| `bookmark_count` | integer or null | Number of outline entries that conversion writes; null when this format's outline cannot be listed. |
| `bookmarks` | array or null | Present only with `--bookmarks`. The root entries, or null when the outline cannot be listed. |
| `outline_warnings` | integer or null | Number of [HN-A bookmark defects](#hn-a-bookmark-defects) skipped or re-parented; `0` for other listed outlines; null when `bookmark_count` is null. |
| `unsupported_reason` | string | Present when this version does not convert a recognized format: `"target-descriptor"` for CAA, or `"not-implemented"` for TEB. TEB detection does not inspect its container, encryption or recoverability. Before #469, TEB returned `"drm-encrypted"`; update callers that match that value. |
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
- CAJ, PDF, CAA and TEB: no per-page records (`Page structure: not available`).

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
and each page's text is checked under the same `Limits` as conversion.
Image payloads are never read.

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
| 3 | Partial PDF committed with blank page substitutions (`--allow-damaged`). |

Errors are written to standard error as `caj2pdf: error: MESSAGE`. Argument
errors add a line pointing to `--help`. Help and version text go to standard
output. On success it prints nothing except `caj2pdf: warning: ...` lines for
[HN-A bookmark defects](#hn-a-bookmark-defects), which keep exit status 0, and,
when standard error is a terminal, a single updating `caj2pdf: reading input
NN%` line during conversion. The percentage is the furthest input byte read;
the line is erased before exit. `-q`/`--quiet` disables it, and it is never
written when standard error is redirected. A conversion using
[installed fonts](#installed-fonts) also prints two `caj2pdf: using
installed ...` lines unless `-q` is given.

## Verification

`crates/caj2pdf-cli/src/tests.rs` covers argument parsing and the options
that `--help` lists, output-path
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
malformed input, HN-B empty-row rejection, input/hardlink overwrite protection,
conversion with an unusable `TMPDIR` and CAJ bookmark omission.

The release CLI converted the external four-page C8 issue-58 document described
in [the direct-record comparison](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-direct-text.md), using a caller-supplied
MQ file and `--no-bookmarks`. All four pages passed qpdf; the 3,992,137-byte
PDF is byte-identical to the native example, Node and Chromium outputs:
`fffa38e8f2cd675352108488117f13983f959ead7500f9f4ba1cab9a2e74ef1e`.
The final run took 2.46 seconds and its scratch directory was empty (that
build still used scratch files; since #355 the CLI creates none).
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
SIGKILL) can leave a named output temporary file. An anonymous stdin spool is
released by the operating system. Do not rely on forced termination for cleanup.


## Native platform adapters

The Windows CLI uses the same conversion, argument and report code as Unix.
Windows identity checks use volume serial numbers and file indexes
(`same_file::Handle`), including hard-link aliases. Only disk files are
compared; a console, pipe or character device has no identity, so a console
standard output is never an error. Paths remain native OS strings. Temporary
files inherit their parent directory's ACL; use a private user temp/output
directory. The stdin spool is opened with `FILE_FLAG_DELETE_ON_CLOSE` and
removed when closed.

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

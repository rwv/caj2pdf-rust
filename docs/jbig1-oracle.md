# HN/C8 type-0 image observations and pixel oracle

This note supports [issue #22](https://github.com/rwv/caj2pdf-rust/issues/22).
It records independently measured container bytes and the output of an
**external black-box** image decoder. It does not specify or implement the
CAJ-specific JBIG1 algorithm. The image files, decoded pixels, reference
PDFs, and the external decoder stay outside this MIT repository.

The inputs are the 27 HN/C8 files in the
[SHA-256-pinned corpus matrix](../tests/conformance/matrix.json), from
CAJSamples revision `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`.
The matrix SHA-256 for this measurement is
`af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9`.
All 27 local input hashes matched the matrix before inspection. The public
[HN page-content note](https://github.com/caj2pdf/caj2pdf/wiki/HN-%E6%A0%BC%E5%BC%8F%E7%9A%84%E9%A1%B5%E9%9D%A2%E5%86%85%E5%AE%B9)
identifies image type 0 at a high level; it is incomplete and includes
decompiled snippets that are **not** algorithm sources for this project.
The files comprise 22 HN and five C8 inputs. Nineteen HN inputs use the
longer observed header, three use the shorter one; the latter include two
without type-0 images and the malformed `issue-100` input.

## Container and image records

All offsets below are absolute byte positions and observed multibyte fields
are little endian. The meanings of the variant markers and fields marked
unknown have not been established. Bounds must be checked before following
any offset or count.

| Variant | Observed magic and marker | Page count | Page-index start |
| --- | --- | ---: | ---: |
| C8 | `c8 00 00 00` at `0x00` | positive i32 at `0x08` | `0x50` |
| HN A | `48 4e 00 00` at `0x00`; marker `90 01 00 00` at `0x04` | positive i32 at `0x90` | `0x15c + 308 × (nonnegative i32 at 0x158)` |
| HN B | `48 4e 00 00` at `0x00`; marker `c8 00 00 00` at `0x04` | positive i32 at `0x90` | `0xd8` |

Each observed page-index row is 20 bytes: text offset (i32, `+0`), text
length (i32, `+4`), image count (i16, `+8`), a serial-like unknown (i16,
`+10`), and two unknown 32-bit fields (`+12`, `+16`). The first 12-byte image
record starts at `text_offset + text_length`. Its fields are image type
(i32, `+0`), absolute image offset (i32, `+4`), and image length (i32, `+8`).
Valid observed offsets, lengths, types, and counts are nonnegative; negative
encodings are rejected rather than reinterpreted as large unsigned values.
The **next record starts at the previous image's end**, not immediately after
the previous record. This was checked across the corpus, including pages with
two, three, and five images. Other image types are only inventoried here;
this oracle covers type 0.

The verified records contained the following image type values. A count is a
record count, not a claim that its image encoding is supported by the Rust
converter. Type 0 is the only type decoded for this oracle.

| Variant | Type 0 | Type 1 | Type 2 | Type 3 |
| --- | ---: | ---: | ---: | ---: |
| HN | 1,375 | 6 | 1,055 | 541 |
| C8 | 25 | 0 | 30 | 5 |

Each of the 1,400 observed type-0 image spans begins with a 40-byte
`BITMAPINFOHEADER`, then an eight-byte two-color palette, then the coded
payload. The [Microsoft DIB definition](https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader)
provides the field layout and 32-bit row-stride rule. Direct observations:

| Field or measurement | Type-0 corpus result |
| --- | --- |
| `biSize`, `biPlanes`, `biBitCount`, `biCompression` | 40, 1, 1, 0 in all 1,400 images |
| Eight palette bytes | `ff ff ff 00 00 00 00 00` in all 1,400 images |
| `biSizeImage` | 0 in 914 images; exactly `stride × height` in 486 |
| `biClrUsed` | 2 in 1,258 images; 0 in 142 |
| Width, height | 666–2,573 pixels; 172–3,669 pixels |
| Coded payload length | 4–79,342 bytes after the 48-byte wrapper |
| Aligned decoded size | `stride = ((width + 31) // 32) × 4`; maximum 1,094,808 bytes |

The published [ITU-T T.82, section 6.2.2](https://www.itu.int/rec/T-REC-T.82)
defines a self-describing bi-level image header with layer and plane counts,
a reserved fourth byte, and big-endian dimensions. None of these 1,400
post-DIB payloads has a complete matching T.82 header at its start; in
particular, zero starts with the usual single-plane, zero-layer
`00 00 01 00` prefix, and zero has the DIB dimensions in the T.82 header
positions. This establishes a framing difference, **not** yet the arithmetic
coder, context template, or predictor difference. Those remain to be
determined independently in #23.

## External oracle and hash definition

The behavioral oracle is the pinned Python converter at revision
`8cbc3c5721acb762f739434eb3d206171dbb022a`. Its native decoder is
under the differently licensed GLWT project, so its source and binary are
never imported, vendored, linked, or distributed with `caj2pdf-rust`.
The [pinned README](https://github.com/rwv/caj2pdf/blob/8cbc3c5721acb762f739434eb3d206171dbb022a/README.md)
documents an external local build. The measured build used Debian C++
14.2.0 (`x86_64-linux-gnu-g++-14` driver SHA-256
`6b3696e4dcb85e1c949c732a02befa50e3983ecf94ce7e8e58d9d503b954b79d`)
and produced `libjbigdec.so` SHA-256
`d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6`.
It stayed under `/tmp/caj2pdf-jbig-oracle/`. Source files, tables, and
decoder pseudocode were not read to design the Rust code.

The external build can be reproduced without bringing its code into this
repository. In a disposable checkout of the pinned revision, the measured
command was:

```sh
c++ -Wall -fPIC -shared -o /tmp/caj2pdf-jbig-oracle/libjbigdec.so \
  /tmp/caj2pdf-python-oracle/lib/jbigdec.cc \
  /tmp/caj2pdf-python-oracle/lib/JBigDecode.cc
```

The library digest above identifies this observed build; matching it also
depends on the external checkout and build environment. The ordinary Rust
build, tests, and release do not invoke this command.

The observed C ABI exports `jbigDecode` with six arguments: input pointer,
compressed byte count, height, width in bits, aligned output stride in bytes,
and output pointer. The input begins at image offset `+48`. The function
returns `void`, so a return from the call alone does not prove a valid image.
Each image was decoded in a separately timed process (20-second timeout) into
a checked `stride × height` allocation, once initialized to `0x00` and once
to `0xa5`. The two outputs matched byte for byte in all 1,400 cases. A
timeout, crash, mismatched prefill result, invalid range, or invalid dimension
is a failure and must never be counted as a pixel match.
The runner's default per-image limits are 64 MiB of encoded bytes and 128 MiB
of decoded bitmap bytes; the worker holds one bounded encoded image and one
bounded output allocation, not a whole source document.

The manifest stores only hashes and metadata. `encoded_sha256` covers the
whole image payload span named by its record: DIB, palette, and coded data;
it excludes the separate 12-byte descriptor.
`raw_stride_sha256` covers all emitted rows in decoder memory order, including
32-bit stride padding. `visible_bits_sha256` concatenates each row's first
`ceil(width / 8)` bytes after zeroing unused low bits of its last byte; it
omits alignment padding and does not reverse the row order. The hash convention
treats the most significant bit as the first bit of each row. All page and
image indices are one based. The `raw_stride_sha256` is the primary reference
for byte-exact output; the visible-bits hash permits an implementation with
a different row padding policy to compare image content deliberately.

The run found 1,400 type-0 images in 21 files and 1,388 pages: 1,382 pages
have one type-0 image, two have two, three have three, and one has five.
All 1,400 were decoded with matching prefill outputs and recorded hashes.
Three malformed discoveries in the `issue-100` HN sample were recorded
separately: page 2 image descriptor 1 points outside the file, page 3 declares
a negative image count, and page 4 has an out-of-range text span. Its Python
matrix outcome is already an invalid-image-offset/count error. These three
records are not successful image checks. No selected type-0 image was left
unresolved.

## Independent PDF cross-check

Python 3.13.5 with external PyPDF2 1.26.0 converted two pinned inputs in a
temporary directory. Poppler `pdfimages` 25.03.0 extracted the corresponding
PBM image data. After its two-line P4 header, each PBM payload was byte for
byte equal to the direct oracle's raw-stride output:

| Sample and one-based image | Source image span | DIB size; padded PDF/PBM size | PBM payload and oracle SHA-256 |
| --- | ---: | --- | --- |
| `issue-21`, page 2 image 1 (HN) | `342560 + 16159` | 2275 × 3425; 2304 × 3425 | `c19efe78c52ff6527ba9c8ddbbe389d9853c3e0e08c71998841120e24a30338c` |
| `issue-33/test1.caj`, page 1 image 1 (C8) | `14778 + 79390` | 2573 × 3285; 2592 × 3285 | `435d70126b94b59d1466b702b0f7e1fb4e2936b7e0d829c3cd968cbb3bfccf83` |

The PDF extraction confirms that the reference PDF embeds these exact padded
bytes for two examples. It does not establish rendered top/bottom orientation
or visible polarity, which also depend on the PDF image palette and page
transform. The generated PDFs and PBMs are not committed. A future Rust
decoder must still compare every available image hash, not just these two.

The opt-in [oracle runner](../scripts/jbig1_oracle.py) verifies the pinned
input inventory and image metadata independently of the external decoder.
Without the corpus or oracle, it reports `NOT_RUN`, not compatibility `PASS`.
The machine-readable [manifest](../tests/conformance/jbig1_oracle.json)
contains no document or pixel bytes. An exact match of the three expected
malformed records is reported as `expected_invalid_records: 3`, separately
from the 1,400 pixel comparisons; any new or changed malformed record fails
the discovery phase.

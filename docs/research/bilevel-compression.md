<!-- SPDX-License-Identifier: MIT -->

# Streaming bilevel PDF compression

Issue [#195](https://github.com/rwv/caj2pdf-rust/issues/195) changes the existing
bilevel image writer to emit zlib streams with `/Filter /FlateDecode`. It feeds
visible row bytes to the existing Rust compression backend and omits DIB stride
padding as before. JPEG passthrough, image placement, page order and bookmarks
are unchanged. Tiny images can grow slightly because of filter/framing overhead.

## Bounds and failure behavior

- One compressor exists per active image and is dropped when that image finishes.
- Before allocating the encoder, the writer checks a conservative **512 KiB**
  fixed-state reservation against `Limits::max_allocation_bytes`. This follows
  the locked backend's fixed-size buffers, not the image dimensions.
- Encoder input calls consume at most **16 KiB**; the fallibly allocated output
  buffer is `min(io_chunk_bytes, 16 KiB)`. PDF writes retain existing short-I/O,
  cancellation and output-limit checks. No whole image is buffered.
- `finish` drains the zlib trailer before the PDF stream's indirect length is
  closed. Compression/output failures make the image unusable; an incomplete
  stream prevents successful document completion.
- This is not a total-process memory limit. Existing decoder, page metadata,
  scratch and host-runtime allocations remain separately bounded/accounted.

PDF bytes and hashes change. Regenerate byte snapshots; inspect image content
through a PDF decoder. Clients that lowered their allocation limit below
512 KiB must raise it for bilevel output. Defaults already cover the reservation.

## Synthetic verification

Rust tests compare exact decoded row bytes across widths, stride padding,
orientation, mixed images, short writes and input/output chunk boundaries.
A 57,351-byte noise payload is written with input splits of 1, 13 and 57,372
bytes and output chunks of 1, 7 and 16,384 bytes; complete PDFs agree byte for
byte. Tests also cover reservation refusal before image output, backend failure,
no-progress rejection, poisoned state, finalization limits and cancellation.
Existing qpdf, MuPDF and Poppler tests independently check PDF structure, pixels,
placement and JPEG passthrough. Existing HN outline tests retain bookmark checks.

A fresh release WASM instance converted the original small HN fixture three
times: linear memory grew from **1,245,184** to **1,900,544 bytes** on the first
conversion and did not grow on later conversions. Output was 1,035 bytes each
time. The JS regression checks bounded growth, reuse, equal PDFs and rejection
below the encoder reservation. WASM linear memory does not shrink, so its final
size records the high-water allocation; it is not browser or Node process RSS.

## Representative external C8 check (2026-09-30 UTC)

Source: the existing CAJSamples issue-58 C8 document, 161,293 bytes, SHA-256
`8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85`.
All conversions use built-in codec states and explicit bookmark omission.
The same existing Node/Chromium measurement runners were reused with 4 KiB
chunks and four 64 MiB scratch caps; their working files remain external.

| Measurement | CLI | Node 24.13.0 | Chromium Worker |
| --- | ---: | ---: | ---: |
| Pages / bookmarks | 4 / 0 | 4 / 0 | 4 / 0 |
| Output bytes | 367,067 | 367,067 | 367,067 |
| Final WASM linear memory | N/A | 1,769,472 | 1,769,472 |
| Sampled process RSS high water | 3,809,280 | 82,112,512 | Not measured |
| Final scratch sizes | Anonymous files released | 0 / 0 / 0 / 0 | 0 / 0 / 0 / 0 |

All three output SHA-256 values are
`47afde9be4bba2061195327fa36829378b2028deb1171e1dad52437fb9c965cc`.
The old uncompressed output was 3,978,812 bytes, so this complete PDF is about
**90.8% smaller**. The earlier 1,376,256-byte WASM measurement grows by 393,216
bytes in this run. This is one sample, not a universal compression ratio.

Native RSS was sampled from `/proc/PID/status` every 2 ms; brief peaks may be
missed. `wait` reported 11,132,928 bytes, including process-launch overhead,
so it is not substituted for the sampled executable measurement. Node RSS was
sampled every 100 ms and includes the JS runtime. Browser checksum allocation
happened only after the conversion and recorded WASM measurement. The browser
removed all OPFS input, output and scratch entries. Node's caller-owned test
scratch handles were closed, leaving zero-length files in its external report
directory. No throughput claim is made from these runs.

qpdf accepted the new PDF. The complete page/image inventory, all four page
object dictionaries and all four decoded page-content streams equal the old
output. Independently decoded image bytes also match exactly:

| Image object | Decoded bytes | SHA-256 |
| --- | ---: | --- |
| 3 | 996,928 | `5ce7d86bbce0373f58a8e168e70c8542ef7b6cb530137e75cba56cad75fa4dcf` |
| 10 | 991,368 | `08a88f9b7b2ed2ff6872072adba2a542c353ad89db0421c7dacb09dfb5296819` |
| 15 | 997,520 | `80320fb26b0c1a119d3d86c5acd8b3dba1ef44d5456cd949499e0dcb98786069` |
| 20 | 990,192 | `6e74ae139c208abf463ccd634e39cbdf13bc6fae7d4b3b47c13f282b446e258c` |

These are lossless-compression checks against prior Rust output, not new
CAJViewer pixel-parity claims. HN/C8 rendering remains experimental. Other
external corpus conversions were **NOT_RUN** for this change; prior large-HN
reports retain their historical uncompressed byte counts and hashes.

## Decision

Include streaming Flate in the v0.1 candidate: representative size improves
substantially, added working memory is fixed, and independent decoded pixels
and cross-platform outputs agree. Final release artifacts must use new hashes.

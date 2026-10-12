# caj2pdf-rust

Convert CNKI CAJ-family documents (CAJ, KDH, HN, C8) to PDF from the command
line, Node.js or the browser. MIT-licensed Rust with a WebAssembly build.
Memory stays bounded: input is read by range and output is written as a stream.
Releases are `v0.x`: CLI behavior and APIs may still change, and HN/C8 support
is experimental.

## Quick start

Download the CLI archive for your platform from
[GitHub Releases](https://github.com/rwv/caj2pdf-rust/releases), extract it,
and run:

```sh
caj2pdf paper.caj                  # writes paper.pdf next to the input
caj2pdf paper.caj -o out.pdf       # explicit output; --force replaces a file
caj2pdf inspect paper.caj --json   # format, page count, bookmarks
```

Or install it from crates.io with Rust 1.88 or newer and curl:

```sh
cargo install --locked caj2pdf-cli
```

Releases also carry a container image ([Docker usage](docs/docker.md)) and a
JavaScript tarball for Node.js 22+ and browsers ([JS API](js/README.md)).
The JavaScript package is also on npm:

```sh
npm install caj2pdf-rust
```
Supported operating systems and CPUs are listed in the
[platform matrix](docs/platforms.md).

The JavaScript API runs each conversion in a Worker. It reads a browser
`File`, `Blob` or OPFS file handle, or a Node.js path, descriptor or `Blob`,
and writes to a stream:

```js
import { convert, loadModule, webWritableSink } from "caj2pdf-rust";

const writer = (await (await showSaveFilePicker()).createWritable()).getWriter();
await convert(await loadModule(), file, webWritableSink(writer), { signal });
await writer.close();
```

## What converts

| Input | Status |
| --- | --- |
| PDF | Supported: copied, with documented [repairs](docs/pdf-input.md). The [TTKN server profile](docs/ttkn-pdf.md) requires an explicit response. |
| CAJ | Supported, with its bookmarks. |
| KDH | Supported for embedded PDFs. |
| HN-A | Experimental: scanned page images and bookmarks; no text layer. |
| C8, HN-B | Experimental: image pages, and native text pages with automatic bundled fonts (below). No bookmarks yet. |
| CAA | Recognized target descriptor: inspection only; obtain the referenced document to convert it. |
| TEB | Recognized, conversion not implemented; detection does not establish encryption or recoverability. |

The [support matrix](docs/conformance.md#current-support-and-release-status)
is the source of truth, with the verified profiles and remaining differences.
Unsupported content fails with a located error instead of dropping pages.

Extensions do not determine the format: the measured `.nh` sample contains
HN-A bytes and converts through that path (433 pages, 365 bookmarks;
[validation](docs/conformance.md#nh-and-caa-discovery-checkpoint-424)). Historical
CAJViewer documentation also names CAS, but no authentic sample has been found;
[CAS research remains open](https://github.com/rwv/caj2pdf-samples/issues/28).

The measured C8 subset includes the NJU title, resource-control and line-segment
profiles documented in [provenance](docs/provenance.md#nju-native-c8-profiles-513-514).
Other record/style combinations can still be refused.

## Fonts for C8 and HN-B text pages

CLI and JavaScript conversions automatically use the measured CAJViewer font
preset, including mode-specific parameters. No font installation or extra
conversion arguments are needed. Native binaries work offline; Node reads
fonts from the package, and the browser downloads and caches them on first use.

The resource pack adds about 58.6 MiB before compression. It stays outside the
core/WASM module, and ranged reads keep it outside the WASM heap. Complete
original font programs also increase the size of native-text PDFs.
See [bundled fonts](docs/bundled-fonts.md) for browser hosting, offline source
builds and custom-font overrides. Explicit fonts remain available through
`--fonts DIR` or JavaScript `hnc8.fonts`.

## Limits

- C8/HN-B outlines are not verified, so those PDFs have no outline and the CLI
  prints a warning. `--no-bookmarks` silences it.
- HN-A pages are images. Run an OCR tool such as `ocrmypdf` for search.
- Substitute fonts do not reproduce the CAJViewer typography pixel for pixel.
  A measured HN-B private-use glyph may use a visual approximation with an
  explicit warning; its private-use code is retained in PDF ActualText.
- TEB files cannot be converted.

## Library

Add the Rust library with `cargo add caj2pdf-core`.

The `caj2pdf-core` crate is the engine behind the CLI and the WASM build. It
reads any `RangedSource` (`SeekableSource` over a `File`, or a byte slice)
and writes any `std::io::Write`:

```rust
use caj2pdf_core::{ConversionOptions, Limits, NeverCancel, convert, native::SeekableSource};
use std::{fs::File, io::BufWriter};

let mut input = SeekableSource::new(File::open("paper.caj")?)?;
let mut output = BufWriter::new(File::create("paper.pdf")?);
let report = convert(&mut input, &mut output, ConversionOptions::default(),
                     &Limits::default(), &mut NeverCancel)?;
```

`convert` detects the format and converts it; `inspect` returns the format,
page count, outline and structure. `ConversionOptions` selects bookmarks,
damaged-CAJ substitution and C8/HN-B `fonts`; a `Progress` receives the
detected format and read progress and can cancel. The
[I/O architecture](docs/io-architecture.md) describes the contract.

## More

- [CLI reference](docs/cli.md): every command, option, exit status and the
  `inspect` JSON schema.
- [I/O architecture](docs/io-architecture.md) and [JS validation](docs/js-validation.md).
- [Release policy](docs/release-policy.md) and
  [build provenance](docs/build-provenance.md) for verifying downloads.
- [Research notes](docs/research/README.md): the format investigations
  behind the decoders, kept with the oracles and conformance harnesses in
  [caj2pdf-samples](https://github.com/rwv/caj2pdf-samples/tree/main/research/README.md).

## Contributing, provenance and license

Source code is [MIT](LICENSE). Code must be original: do not copy from the
Python or Go converters or from decoders under other licenses. The
[provenance record](docs/provenance.md) lists format references, the
external test corpus rules and the dependency review. Sample documents are
not part of this repository; the separate
[CAJSamples](https://github.com/caj2pdf/CAJSamples) collection is an optional
compatibility corpus. See [CONTRIBUTING.md](CONTRIBUTING.md) for checks,
commit style and the merge gate, and [CHANGELOG.md](CHANGELOG.md) for
changes. The project's public language is English.

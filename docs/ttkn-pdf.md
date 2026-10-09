# TTKN server-response PDFs (unreleased)

The measured TTKN server/authentication profile converts when the caller
supplies its matching response. This is a 32-character hexadecimal ASCII
value, with case preserved, rather than a conventional PDF password.
The converter never contacts the authentication URL stored in the file.
Other TTKN profiles and missing responses remain unsupported.

```sh
caj2pdf input.pdf --ttkn-response-file response.txt -o output.pdf
```

The response file contains exactly 32 ASCII hex characters, optionally with
surrounding ASCII whitespace within a 34-byte file. Its contents are never
included in diagnostics. The input, response file and font resources are all
protected against accidental output overwrite. Path output is staged and
committed only after successful conversion. Standard output cannot be rolled
back on a late failure.

Rust callers construct `caj2pdf_core::pdf::TtknResponse::new(ascii)` and pass it
to `caj2pdf_core::convert_with_ttkn_response(source, sink, options, &response,
limits, progress)`. Existing `ConversionOptions` and `convert` are unchanged.
The response type has no `Debug` implementation and clears its owned bytes
on drop. It does not clear copies retained by the caller.

Node and browsers use the same option:

```js
await convert(module, input, sink, { ttknResponse: response });
```

`ttknResponse` is available only for conversion. Inspection and bookmark
editing do not accept credentials; inspect the converted PDF instead. JavaScript
strings and caller-owned sinks retain their existing lifecycle: the caller
must discard partial output after failure. Incorrect or missing responses in
the tested controls fail before the first output write.

## Admitted profile and bounds

The source has a single classic PDF xref table, a `/TTKN.PubSec` handler with
`/SubFilter /TTKN.PubSec.s1`, `/V 2`, `/R 2`, `/Length 40`, and
`/EncryptMetadata true`. Its sole crypt filter is `/DefaultCryptFilter`,
with `/CFM /AESV2` and the `AppendCA` recipient. Both strings and streams use
that filter. Per-stream `/Crypt` filters, xref streams, object streams and
incremental encrypted revisions are not admitted by this path.

`WebFastLoad` plus NUL separates the PDF from UTF-8 rights XML. A terminal
`startrights offset,length` describes its absolute source extent. XML is
bounded to 16 KiB and depth 32; DTDs, entities, processing instructions and
unmeasured encodings are not resolved. The admitted metadata has version
`2.0`, authentication type `1`, permit type `3`, a 48-byte decoded password,
a 32-byte decoded IV and block-aligned encrypted rights. Relevant elements
must be unique. Base64 leaf text uses the measured contiguous encoding.

Only object metadata and indexes are retained under the allocation limit.
Content streams stay in the original ranged source and are decrypted in
at most 8 KiB batches, split further at the caller's I/O limit. Output is
sequential. PKCS#7 padding, stream extents, object references, page trees and
outlines are checked; existing conservative PDF repairs still apply.
The rebuilt PDF preserves live object numbers, generations, catalog, Info,
first document ID, decrypted strings, stream bytes, geometry and outlines.
It removes the encryption dictionary/wrapper and writes a new xref and second
ID marker. Existing PDF outlines are preserved, so `bookmarksWritten` counts
newly imported entries and stays zero for this path.

## Independent provenance and validation

All implementation and fixture sources are original MIT code. No vendor,
Python/Go converter or private HN/JBIG implementation was read or translated.
Two wholly authored wrapper controls with different responses, IDs, seeds,
IVs and metadata established the first-layer initializer
`200CFC8299B84aa9` (16 ASCII bytes). It was also independently derived from
each control's known plaintext/ciphertext. A third authored encrypted PDF,
using that initializer, displayed its text, red/blue rectangles and outline
in the isolated offline viewer; both wrapper plaintexts matched the generator.
This format constant is distinct from a document's IV and response.

The original derivation is:

1. Decrypt the 48-byte password with AES-256-CBC, using the response's 32 ASCII
   bytes as the key and the measured initializer. No hex decoding or PKCS#7
   removal applies to this layer.
2. Hash its first 32 plaintext bytes followed by the exact original metadata
   XML, omitting only the `rights` element's base64 content, with SHA-256.
   Use that key and the first 16 decoded IV bytes to decrypt the rights.
   Only terminal zero bytes are removed from this XML.
3. Take the first 16 bytes of SHA-1 of the rights' 32 ASCII `encrypt` bytes
   followed by `AppendCA`. This is the PDF file key.
4. Apply the PDF AESV2 object-key rule: MD5 of that key, the three low bytes
   of the object number, two generation bytes (both little endian), and
   `sAlT`. Each encrypted string/stream carries a 16-byte IV followed by
   AES-128-CBC ciphertext with PKCS#7 padding.

The authored fixture and OpenSSL-based generator are in
[`tests/fixtures/ttkn`](../crates/caj2pdf-core/tests/fixtures/ttkn/README.md).
OpenSSL is only an independent fixture generator; the product uses RustCrypto.
The standard object-key rules follow PDF 1.7 §3.5.1, Algorithm 3.1; custom
wrapper rules come from the original controls and bounded measurements.

The unchanged public source SHA-256 is
`074cb4d57181e92826c37b549f811008c58c7b66366f9045e8985c58178263d8`.
Its author deliberately disclosed a matching response in the
[original discussion](https://chaoli.club/index.php/2979/5). No response,
source metadata, document bytes or derived page content is committed here.
The implementation requires explicit caller input and embeds no sample key.

Original-input native/Node/Chromium output is byte-identical. Independent
qpdf checks pass without warnings. All 234 plaintext streams match the
separate qpdf research recovery; all 180 pages match in geometry, word
positions and 72-dpi MuPDF pixels; all 97 outline entries and destinations
match. This comparison establishes agreement with the independent recovery,
not all-page pixel agreement with the vendor viewer. The original viewer
control covers opening, the first page and populated contents. Other TTKN
profiles, unavailable responses and the broader fidelity goal remain open
in #415/#406. No release was published for this change.

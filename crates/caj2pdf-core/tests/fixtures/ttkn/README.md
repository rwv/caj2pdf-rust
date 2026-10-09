# Original TTKN control

All files in this directory are independently authored for this project and
MIT-licensed. No external document fields, credentials, text, fonts or pixels
were copied. `generate.py` creates the encrypted one-page PDF, response and
expected content from `own-...-3` labels. The page has two colored rectangles,
Helvetica text and a UTF-16 outline. The positive control was also opened in
an isolated viewer before implementing the converter.

Run `python3 generate.py --check` from this directory with OpenSSL installed
to reproduce the checked-in bytes. The generator invokes OpenSSL's public CBC
interface only; no implementation code was consulted. OpenSSL is neither
linked into nor required by the converter. The synthetic response is public
test material and cannot decrypt an external document.

`ttkn_pdf.rs` checks decrypted content, outline strings, wrong/missing/case-
changed responses, truncation, malformed/profile metadata, bad stream padding,
short reads, 1-byte I/O chunks, exact input accounting, resource limits,
cancellation and sink failure. CLI tests check atomic-output cleanup and
response-file overwrite protection; Node/Chromium tests exercise the public
worker API and compare output bytes.

Pinned fixture SHA-256 values:

- `authored.pdf`: `5782429d292045eaf74226af2866e69b5496b9dedddc3419418f4b5a7ef172a3`
- `response.txt`: `84aad303aa42ba93bb3a6aad7a7f4dbe4b045bc4c36fb4360aae2240e707ec67`
- `content.txt`: `e33df6c91c881700699ac957e38b9beb29fd801bfb418734b9bc06ce3c143b7c`

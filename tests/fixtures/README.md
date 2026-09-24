# Synthetic fixtures

All files in this directory were authored for this repository and are licensed
under MIT. The bytes come from `scripts/generate_fixtures.py`, written from the
published [PDF 1.7 reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf)
and signature facts recorded in the public
[CAJSamples magic index](https://github.com/caj2pdf/CAJSamples/blob/7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07/magic).
No CAJSamples document, reference-converter output, or legacy converter source
was used.

Regenerate and check the committed bytes with:

```sh
python3 scripts/generate_fixtures.py
python3 scripts/generate_fixtures.py --check
python3 -m unittest discover -s tests/fixtures -p 'test_*.py'
```

`manifest.json` records each file's SHA-256 digest, size, format, and intended
test condition. `valid_nested_outline.pdf` has two pages (200 × 300 and
400 × 250 PDF points), an open parent outline targeting page 1, a nested child
targeting page 2, and a referenced embedded binary stream. The binary bytes
contain `endstream`, `endobj`, an apparent object header, `xref`, `startxref`,
and `%%EOF`; their presence must not change object or xref parsing.
`valid_out_of_order_objects.pdf` has the same logical content but places object
definitions in a different physical order. That order is legal and is a valid
parser stress case.

The other PDFs intentionally contain one named structural defect each: an
overstated stream length, an out-of-range xref offset, an incorrect page-tree
count, a duplicate object definition, or a cut-off xref table. The tiny
CAJ/HN/C8/KDH/TEB files contain only an observed signature and are deliberately
too short to be valid documents. A `malformed` entry describes input bytes; it
does not predetermine whether a future repair operation rejects or repairs
them. Tests of conversion outcomes should state that policy separately.

The clean-clone tests use Python's standard library only. Independent local
validation of the two valid PDFs can be repeated with `qpdf --check FILE` and
`mutool info FILE`; these tools are not required to generate or test fixtures.

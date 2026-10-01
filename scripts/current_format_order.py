# SPDX-License-Identifier: MIT
"""Independent source-page identity checks for the pinned current-format run.

Page identity is narrower than geometry or full rendered-page fidelity. CAJ
uses source page-table IDs. HN/C8 uses ordered images and the existing external
codec pixel hashes. PDF/KDH use independently opened source page/content IDs.
"""

from __future__ import annotations

from functools import cache
import hashlib
import json
from pathlib import Path
import struct

from hnc8_layout_source import FileInput, SourceExtractor
from hnc8_page_composition import pnm_header
import conformance

ROOT = Path(__file__).resolve().parent.parent


def bounded_json(path: Path, limit: int = 32 * 1024 * 1024):
    if path.stat().st_size > limit:
        raise ValueError("PDF metadata exceeds 32 MiB")
    return json.loads(path.read_text())


def pdf_pages(commands, pdf: Path, label: str) -> list:
    result = commands.run(["qpdf", "--json", "--json-key=pages", pdf], label)
    if result["exit_code"] not in (0, 3):
        raise ValueError("qpdf could not read page identities")
    pages = bounded_json(commands.directory / f"{label}.stdout")["pages"]
    if not 0 < len(pages) <= 10000:
        raise ValueError("page identity count outside 1..10000")
    return pages


def read_exact(source, length: int) -> bytes:
    data = source.read(length)
    if len(data) != length:
        raise ValueError("truncated source metadata")
    return data


def caj_page_ids(source: Path) -> list[str]:
    with source.open("rb") as stream:
        if read_exact(stream, 4) != b"CAJ\0":
            raise ValueError("not a CAJ source")
        stream.seek(0x10)
        count, index = struct.unpack("<II", read_exact(stream, 8))
        if not 0 < count <= 10000 or index + count * 12 > source.stat().st_size:
            raise ValueError("invalid CAJ page table")
        stream.seek(index)
        return [f"{struct.unpack('<III', read_exact(stream, 12))[2]} 0 R" for _ in range(count)]


def decoded_kdh(source: Path, output: Path) -> None:
    # Independently measured transformation documented in docs/kdh-format.md.
    # Let qpdf parse the decoded source, including its opaque trailing data.
    with source.open("rb") as src, output.open("xb") as dest:
        if not read_exact(src, 32).startswith(b"KDH 2.00"):
            raise ValueError("not a measured KDH source")
        src.seek(254)
        position = 0
        while block := src.read(65536):
            dest.write(bytes(value ^ b"FZHMEI"[(position + i) % 6] for i, value in enumerate(block)))
            position += len(block)


@cache
def oracle(name: str) -> dict:
    data = bounded_json(ROOT / f"tests/conformance/{name}.json")
    return {sample["id"]: sample for sample in data["samples"]}


def expected_image(sample: dict, image: dict, source_sha: str, source_id: str) -> tuple:
    kind = image["record_type"]
    if kind == 2:
        return ("jpeg", image["payload_sha256"])
    if kind not in (0, 3):
        raise ValueError("no independent pixel oracle for image type")
    data = oracle("jbig1_oracle" if kind == 0 else "jbig2_oracle")[source_id]
    if data["source_sha256"] != source_sha:
        raise ValueError("pixel oracle source identity differs")
    candidates = [entry for entry in data["images"]
                  if entry["page"] == sample["page_number"] and entry["image"] == image["image_number"]]
    if len(candidates) != 1:
        raise ValueError("missing or duplicate image oracle entry")
    entry = candidates[0]
    if (entry["encoded_sha256"], entry["offset"], entry["length"]) != (
        image["payload_sha256"], image["payload_offset"], image["payload_length"],
    ):
        raise ValueError("source image does not match pinned oracle")
    if entry.get("decoder_result", entry.get("status")) != "PASS":
        raise ValueError("image oracle is not a passing baseline")
    return ("bits", entry["width"], entry["height"],
            entry["visible_bits_sha256"] if kind == 0 else entry["normalized_pixel_sha256"])


def extracted_image(path: Path) -> tuple:
    if path.suffix == ".jpg":
        with path.open("rb") as stream:
            return ("jpeg", hashlib.file_digest(stream, "sha256").hexdigest())
    if path.suffix != ".pbm":
        raise ValueError("unexpected extracted image representation")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        header = pnm_header(stream)
        if header.magic != "P4" or header.width * header.height > 100_000_000:
            raise ValueError("unsupported or oversized extracted bitmap")
        for _ in range(header.height):
            row = bytearray(read_exact(stream, header.row_bytes))
            if header.width % 8:
                row[-1] &= (0xff << (8 - header.width % 8)) & 0xff
            digest.update(row)
        if stream.read(1):
            raise ValueError("trailing bitmap data")
    return ("bits", header.width, header.height, digest.hexdigest())


def images_match(expected: list[tuple], actual: list[tuple]) -> bool:
    # Extra source descriptors may repeat a complete first group. This checks
    # byte/pixel identity and page order only; coordinate/alias semantics are
    # covered separately by the composition tests and fidelity work.
    return bool(actual) and len(expected) % len(actual) == 0 and all(
        image == actual[index % len(actual)] for index, image in enumerate(expected)
    )


def check(commands, source: Path, pdf: Path, row: dict, info: dict, label: str) -> dict:
    format_name = row["detected_type"]
    pages = pdf_pages(commands, pdf, label + "-output-pages")
    if format_name == "CAJ":
        expected = caj_page_ids(source)
        actual = [page["object"] for page in pages]
        return {"status": "PASS" if expected == actual else "FAIL",
                "method": "source page-table object IDs versus qpdf output page order", "pages": len(actual)}
    if format_name in ("PDF", "KDH"):
        original = source
        if format_name == "KDH":
            original = commands.directory / f"{label}-decoded.pdf"
            decoded_kdh(source, original)
        try:
            source_pages = pdf_pages(commands, original, label + "-source-pages")
            expected = [(page["object"], page["contents"]) for page in source_pages]
            actual = [(page["object"], page["contents"]) for page in pages]
            return {"status": "PASS" if expected == actual else "FAIL",
                    "method": "qpdf source/output page and content-object identities", "pages": len(actual)}
        finally:
            if format_name == "KDH":
                original.unlink()
    if format_name not in ("HN", "C8"):
        return {"status": "NOT_RUN", "reason": "no page identity check for this format"}
    if len(pages) != info.get("page_count"):
        return {"status": "FAIL", "reason": "source and output page counts differ"}
    mismatches = []
    checked = images = aliases = 0
    with FileInput(source) as stream:
        extractor = SourceExtractor(stream, row["id"])
        for page in extractor.iter_pages():
            number = page["page_number"]
            expected = [expected_image(page, image, row["sha256"], row["id"]) for image in page["images"]]
            image_dir = commands.directory / f"{label}-page-{number}"
            image_dir.mkdir(mode=0o700)
            prefix = image_dir / "image"
            try:
                result = commands.run(["pdfimages", "-f", str(number), "-l", str(number), "-j", pdf, prefix],
                                      f"{label}-images-{number}")
                if result["exit_code"] != 0:
                    raise ValueError("pdfimages extraction failed")
                files = sorted(image_dir.iterdir(), key=lambda path: int(path.stem.rsplit("-", 1)[1]))
                actual = [extracted_image(path) for path in files]
                if not images_match(expected, actual):
                    mismatches.append(number)
                else:
                    aliases += len(expected) - len(actual)
                checked += 1
                images += len(actual)
            finally:
                for path in image_dir.iterdir():
                    path.unlink()
                image_dir.rmdir()
    return {"status": "FAIL" if mismatches or checked != len(pages) else "PASS", "pages": checked, "images": images,
            "repeated_source_descriptors": aliases, "mismatched_pages": mismatches,
            "method": "ordered source images versus Poppler extraction and pinned pixel oracles",
            "scope": "page/image identity and order; not placement, alias coordinates or rendered-page parity"}


def source_outline_hash(source: Path, format_name: str, info: dict) -> tuple[int, str] | None:
    if format_name != "CAJ" and info.get("variant") != "HN-A":
        return None
    start = 0x110 if format_name == "CAJ" else 0x158
    digest = hashlib.sha256()
    with source.open("rb") as stream:
        stream.seek(start)
        count = struct.unpack("<I", read_exact(stream, 4))[0]
        if count > 100000 or start + 4 + count * 308 > source.stat().st_size:
            raise ValueError("invalid source outline extent")
        for _ in range(count):
            raw = read_exact(stream, 308)
            title = raw[:256].split(b"\0", 1)[0].decode("gb18030")
            page = int(raw[280:292].split(b"\0", 1)[0].decode("ascii"))
            level = struct.unpack_from("<I", raw, 304)[0]
            if not 1 <= page <= info["page_count"] or level < 1:
                raise ValueError("invalid source outline destination or depth")
            conformance.update_outline_hash(digest, {"depth": level - 1, "title": title,
                                                    "page": page, "destination": f"#page={page}"})
    return count, digest.hexdigest()

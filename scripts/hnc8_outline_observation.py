#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded, opt-in outline discovery; no HN title field is implemented here.

Only original runtime fixtures may be used until an independently reviewed
execution contract is frozen. Private titles and object output stay external.
"""

from collections import Counter
from contextlib import ExitStack
from dataclasses import dataclass
from decimal import Decimal
import hashlib
import importlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import sys
import tempfile
import time

import hnc8_page_composition as process

ROOT = Path(__file__).resolve().parents[1]
MIB = 1024 * 1024
RECORD_BYTES = 308
MAX_RECORDS = 512
MAX_PAGES = 256
MAX_DEPTH = 64
MAX_TITLE_BYTES = 1232
MAX_SUMMARY_CANDIDATES = 16
MAX_QUERY_BYTES = MIB
MAX_SESSION_BYTES = 16 * MIB
MAX_HASH_REQUEST_BYTES = 1024 * MIB
MAX_FIELD_REQUEST_BYTES = MIB
MAX_CHILDREN = 24
CLOSING_CHILDREN = 6
TIME_LIMIT = 600.0
ENUMERATION = "308-byte-numeric-and-terminated-title-v1"
DISCOVERY_SHA = "33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4"
REFERENCE_SHA = process.REFERENCE_SHA
REFERENCE_BYTES = 40896
CODE_PATHS = ("scripts/hnc8_outline_observation.py", "scripts/hnc8_page_composition.py",
              "scripts/hnc8_layout_pdf.py", "scripts/hnc8_placement_rule.py",
              "tests/conformance/test_hnc8_outline_observation.py")
TOOL_KEYS = ("python", "qpdf", "mutool")
TOOL_VERSIONS = {"python": "Python 3.13.5", "qpdf": process.VERSIONS["qpdf"],
                 "mutool": process.VERSIONS["mutool"]}
_REF = re.compile(r"([1-9][0-9]*) 0 R\Z")
_SHA = re.compile(r"[0-9a-f]{64}\Z")
_NUMBER = re.compile(rb"[-+]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)\Z")


class ObservationError(Exception):
    """Requested observation, integrity or bounded resource failure."""


class UnsupportedObservation(ObservationError):
    """Unknown requested semantics; this never counts as a successful check."""


def integer(value, label, minimum=0, maximum=(1 << 31) - 1):
    if type(value) is not int or not minimum <= value <= maximum:
        raise ObservationError(f"{label}: integer outside declared range")
    return value


def sha(value):
    if not isinstance(value, str) or not _SHA.fullmatch(value):
        raise ObservationError("invalid SHA-256 identity")
    return value


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ObservationError("duplicate JSON key")
        result[key] = value
    return result


def _json_int(value):
    if len(value.lstrip("-")) > 19:
        raise ObservationError("JSON integer literal exceeds its cap")
    result = int(value)
    if not -(1 << 63) <= result < (1 << 63):
        raise ObservationError("JSON integer literal exceeds its cap")
    return result


def _json_real(value):
    if len(value) > 64:
        raise ObservationError("JSON real literal exceeds its cap")
    number = Decimal(value)
    if not number.is_finite() or abs(number.adjusted()) > 32:
        raise ObservationError("JSON real exponent exceeds its cap")
    return number


def decode_json(data: bytes, cap=MAX_QUERY_BYTES):
    """Bound literals/nesting before duplicate-safe JSON allocation."""
    if type(data) is not bytes or len(data) > cap:
        raise ObservationError("JSON byte ceiling exceeded")
    depth, quoted, escaped, length = 0, False, False, 0
    for byte in data:
        if quoted:
            if byte == 34 and not escaped:
                quoted = False
            else:
                length += 1
                if length > 16384:
                    raise ObservationError("JSON string ceiling exceeded")
                escaped = byte == 92 and not escaped
        elif byte == 34:
            quoted, escaped, length = True, False, 0
        elif byte in (123, 91):
            depth += 1
            if depth > MAX_DEPTH * 2 + 16:
                raise ObservationError("JSON nesting ceiling exceeded")
        elif byte in (125, 93):
            depth -= 1
    def reject(value):
        raise ObservationError("nonfinite JSON literal")
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=_pairs,
                          parse_int=_json_int, parse_float=_json_real,
                          parse_constant=reject)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ObservationError("malformed bounded JSON") from error


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True,
                       separators=(",", ":")) + "\n").encode()


def byte_identity(data):
    return {"size_bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def title_identity(title):
    if not isinstance(title, str):
        raise ObservationError("PDF title is not Unicode")
    try:
        data = title.encode("utf-8", "strict")
    except UnicodeError as error:
        raise ObservationError("PDF title has an unpaired surrogate") from error
    if len(data) > MAX_TITLE_BYTES:
        raise ObservationError("decoded title byte ceiling exceeded")
    return byte_identity(data)


@dataclass(frozen=True)
class Ref:
    number: int


def reference(value):
    if isinstance(value, Ref):
        return value
    if not isinstance(value, str) or not (match := _REF.fullmatch(value)):
        raise UnsupportedObservation("unsupported or unresolved PDF reference")
    return Ref(integer(int(match[1]), "PDF object number", 1))


def number_text(value):
    if type(value) not in (int, Decimal):
        raise UnsupportedObservation("destination parameter is not a number or null")
    number = Decimal(value)
    if not number.is_finite() or abs(number.adjusted()) > 32:
        raise UnsupportedObservation("destination real is outside its numeric profile")
    text = format(number, "f")
    if len(text) > 96:
        raise ObservationError("destination number expansion exceeds its cap")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if number.is_zero() else text


def destination(value):
    """Preserve nulls and exact finite real values; no URI/view transformation."""
    if not isinstance(value, list) or len(value) < 2:
        raise UnsupportedObservation("named, missing or non-array destination")
    page = reference(value[0])
    counts = {"/Fit": 0, "/XYZ": 3, "/FitH": 1, "/FitV": 1,
              "/FitR": 4, "/FitB": 0, "/FitBH": 1, "/FitBV": 1}
    kind = value[1]
    if type(kind) is not str or kind not in counts or len(value) != counts[kind] + 2:
        raise UnsupportedObservation("unsupported destination kind or arity")
    parameters = [None if item is None else number_text(item) for item in value[2:]]
    return page, kind, parameters


def common_entry(ordinal, depth, title, page_index, kind, parameters):
    identity = title_identity(title)
    return {"ordinal": ordinal, "depth": depth,
            "title_utf8_sha256": identity["sha256"],
            "title_utf8_bytes": identity["size_bytes"],
            "resolved_pdf_page_index": page_index,
            "destination_kind": kind, "destination_parameters": parameters}


def parse_qpdf_outlines(data):
    document = decode_json(data)
    if not isinstance(document, dict) or set(document) != {"version", "parameters", "outlines"}:
        raise ObservationError("unexpected qpdf outline JSON framing")
    if type(document["version"]) is not int or document["version"] != 2:
        raise UnsupportedObservation("unsupported qpdf JSON version")
    if document["parameters"] != {"decodelevel": "generalized"}:
        raise UnsupportedObservation("unexpected qpdf decode profile")
    entries, objects, seen = [], [], set()
    def walk(nodes, depth):
        if not isinstance(nodes, list) or (nodes and depth > MAX_DEPTH):
            raise ObservationError("outline list/depth ceiling exceeded")
        for node in nodes:
            if not isinstance(node, dict) or set(node) != {"dest", "destpageposfrom1", "kids", "object", "open", "title"}:
                raise ObservationError("unexpected qpdf outline entry schema")
            if len(entries) >= MAX_RECORDS or type(node["open"]) is not bool:
                raise ObservationError("outline count or open-state framing invalid")
            object_id = reference(node["object"])
            if object_id in seen:
                raise ObservationError("duplicate or cyclic qpdf outline object")
            seen.add(object_id)
            page, kind, parameters = destination(node["dest"])
            position = integer(node["destpageposfrom1"], "resolved PDF page", 1, MAX_PAGES)
            entries.append(common_entry(len(entries) + 1, depth, node["title"], position - 1, kind, parameters))
            objects.append({"reference": object_id, "page": page, "depth": depth})
            walk(node["kids"], depth + 1)
    walk(document["outlines"], 0)
    return entries, objects


class PdfValueParser:
    """Original bounded grammar for MuPDF's non-stream object display only."""

    def __init__(self, data):
        if type(data) is not bytes or len(data) > 16384:
            raise ObservationError("MuPDF object line exceeds its cap")
        self.data, self.cursor = data, 0

    def space(self):
        while self.cursor < len(self.data):
            byte = self.data[self.cursor]
            if byte in b" \t\r\n\x00\x0c":
                self.cursor += 1
            elif byte == 37:
                end = self.data.find(b"\n", self.cursor)
                self.cursor = len(self.data) if end < 0 else end + 1
            else:
                break

    def token(self):
        self.space()
        start = self.cursor
        while self.cursor < len(self.data) and self.data[self.cursor] not in b" \t\r\n\x00\x0c()<>[]{}/%":
            self.cursor += 1
        return self.data[start:self.cursor]

    def string(self):
        output, nesting = bytearray(), 1
        self.cursor += 1
        while self.cursor < len(self.data):
            byte = self.data[self.cursor]; self.cursor += 1
            if byte == 92:
                if self.cursor >= len(self.data):
                    break
                byte = self.data[self.cursor]; self.cursor += 1
                escapes = {110: 10, 114: 13, 116: 9, 98: 8, 102: 12}
                if byte in escapes:
                    output.append(escapes[byte])
                elif byte in (10, 13):
                    if byte == 13 and self.data[self.cursor:self.cursor+1] == b"\n":
                        self.cursor += 1
                elif 48 <= byte <= 55:
                    digits = bytes([byte])
                    for _ in range(2):
                        if self.cursor < len(self.data) and 48 <= self.data[self.cursor] <= 55:
                            digits += self.data[self.cursor:self.cursor+1]; self.cursor += 1
                    output.append(int(digits, 8) & 255)
                else:
                    output.append(byte)
            elif byte == 40:
                nesting += 1
                if nesting > MAX_DEPTH:
                    raise ObservationError("PDF literal-string depth ceiling exceeded")
                output.append(byte)
            elif byte == 41:
                nesting -= 1
                if nesting == 0:
                    return bytes(output)
                output.append(byte)
            else:
                output.append(byte)
            if len(output) > MAX_TITLE_BYTES * 2 + 2:
                raise ObservationError("PDF string exceeds its byte ceiling")
        raise ObservationError("truncated PDF literal string")

    def value(self, depth=0):
        self.space()
        if depth > MAX_DEPTH or self.cursor >= len(self.data):
            raise ObservationError("PDF value nesting/truncation refusal")
        data = self.data
        if data[self.cursor:self.cursor+2] == b"<<":
            self.cursor += 2; result = {}
            while True:
                self.space()
                if data[self.cursor:self.cursor+2] == b">>":
                    self.cursor += 2; return result
                key = self.value(depth + 1)
                if type(key) is not str or not key.startswith("/") or key in result:
                    raise ObservationError("PDF dictionary key is invalid or duplicated")
                result[key] = self.value(depth + 1)
                if len(result) > 64:
                    raise ObservationError("PDF dictionary key ceiling exceeded")
        byte = data[self.cursor]
        if byte == 91:
            self.cursor += 1; result = []
            while True:
                self.space()
                if data[self.cursor:self.cursor+1] == b"]":
                    self.cursor += 1; return result
                result.append(self.value(depth + 1))
                if len(result) > MAX_RECORDS:
                    raise ObservationError("PDF array item ceiling exceeded")
        if byte == 40:
            return self.string()
        if byte == 60:
            end = data.find(b">", self.cursor + 1)
            if end < 0:
                raise ObservationError("truncated PDF hex string")
            text = re.sub(rb"[\x00\x09\x0a\x0c\x0d\x20]", b"", data[self.cursor+1:end])
            if len(text) > MAX_TITLE_BYTES * 4 + 4 or not re.fullmatch(rb"[0-9a-fA-F]*", text):
                raise ObservationError("PDF hex string bytes/framing invalid")
            if len(text) % 2:
                text += b"0"
            self.cursor = end + 1
            return bytes.fromhex(text.decode("ascii"))
        if byte == 47:
            self.cursor += 1; start = self.cursor
            while self.cursor < len(data) and data[self.cursor] not in b" \t\r\n\x00\x0c()<>[]{}/%":
                self.cursor += 1
            encoded = data[start:self.cursor]
            if len(encoded) > 128 or re.search(rb"#(?![0-9a-fA-F]{2})", encoded):
                raise ObservationError("PDF name framing invalid")
            encoded = re.sub(rb"#([0-9a-fA-F]{2})", lambda m: bytes([int(m[1], 16)]), encoded)
            try:
                return "/" + encoded.decode("ascii")
            except UnicodeError as error:
                raise UnsupportedObservation("non-ASCII PDF name") from error
        token = self.token()
        if token == b"null": return None
        if token == b"true": return True
        if token == b"false": return False
        if len(token) <= 64 and _NUMBER.fullmatch(token):
            number = Decimal(token.decode("ascii"))
            if abs(number.adjusted()) > 32:
                raise ObservationError("PDF numeric exponent ceiling exceeded")
            checkpoint = self.cursor
            second = self.token()
            if token.isdigit() and second.isdigit():
                third = self.token()
                if third == b"R":
                    if second != b"0" or token == b"0":
                        raise UnsupportedObservation("unsupported PDF reference generation/number")
                    return Ref(integer(int(token), "PDF object number", 1))
            self.cursor = checkpoint
            return number
        raise UnsupportedObservation("unsupported PDF value or stream marker")

    def complete(self):
        value = self.value()
        self.space()
        if self.cursor != len(self.data):
            raise UnsupportedObservation("trailing PDF value or stream data")
        return value


def pdf_title(value):
    if isinstance(value, Ref):
        raise UnsupportedObservation("unmeasured indirect PDF title profile")
    if type(value) is not bytes:
        raise ObservationError("PDF outline Title is not a direct string")
    try:
        if value.startswith(b"\xfe\xff"):
            title = value[2:].decode("utf-16-be", "strict")
        elif all(byte < 128 for byte in value):
            title = value.decode("ascii")
        else:
            raise UnsupportedObservation("unmeasured PDFDocEncoding/binary title profile")
    except UnicodeError as error:
        raise ObservationError("malformed PDF title Unicode") from error
    title_identity(title)
    return title


def _dict_ref(dictionary, key):
    value = dictionary.get(key)
    if value is None: return None
    if not isinstance(value, Ref):
        raise UnsupportedObservation("outline link is not a supported indirect reference")
    return value


def parse_mutool_objects(data, expected_objects):
    if type(data) is not bytes or len(data) > MAX_QUERY_BYTES:
        raise ObservationError("MuPDF output byte ceiling exceeded")
    pages, objects, root_ref, absent = {}, {}, None, False
    for line in data.splitlines():
        if not line.strip(): continue
        if match := re.fullmatch(rb"page ([0-9]+) = ([0-9]+) ([0-9]+) R", line):
            number = integer(int(match[1]), "PDF page ordinal", 1, MAX_PAGES)
            if number != len(pages) + 1 or match[3] != b"0":
                raise ObservationError("MuPDF pages are nonsequential or unsupported")
            ref = Ref(integer(int(match[2]), "page object", 1))
            if ref in pages:
                raise ObservationError("MuPDF pages share an object")
            pages[ref] = number - 1
        elif line == b"null" and root_ref is None and not objects and not absent:
            absent = True
        elif match := re.fullmatch(rb"([0-9]+) ([0-9]+) obj (.+)", line):
            if match[2] != b"0" or absent:
                raise UnsupportedObservation("MuPDF outline object framing/generation invalid")
            ref = Ref(integer(int(match[1]), "outline object", 1))
            if ref in objects or len(objects) >= MAX_RECORDS + 1:
                raise ObservationError("duplicate/excess MuPDF outline object")
            value = PdfValueParser(match[3]).complete()
            if not isinstance(value, dict):
                raise ObservationError("outline object is not a non-stream dictionary")
            if root_ref is None: root_ref = ref
            objects[ref] = value
        else:
            raise ObservationError("unrecognized MuPDF metadata line")
    if not pages:
        raise ObservationError("MuPDF returned no page mapping")
    expected = {item["reference"] for item in expected_objects}
    if root_ref is None:
        if not absent or expected:
            raise ObservationError("missing MuPDF outline root")
        return [], pages
    if root_ref in expected or set(objects) != expected | {root_ref}:
        raise ObservationError("MuPDF selected-object coverage differs")
    root = objects[root_ref]
    if root.get("/Type", "/Outlines") != "/Outlines":
        raise ObservationError("MuPDF root is not Outlines")
    entries, visited, ordered = [], set(), []
    def siblings(first, last, parent, depth):
        if (first is not None and depth > MAX_DEPTH) or (first is None) != (last is None):
            raise ObservationError("outline child/depth framing invalid")
        previous, current = None, first
        while current is not None:
            if current not in expected or current in visited:
                raise ObservationError("outline dangling link, repeated object or cycle")
            visited.add(current); ordered.append(current); node = objects[current]
            if _dict_ref(node, "/Parent") != parent or _dict_ref(node, "/Prev") != previous:
                raise ObservationError("outline parent/previous links disagree")
            if "/Dest" in node and "/A" in node:
                raise UnsupportedObservation("outline has competing target forms")
            target = node.get("/Dest")
            if "/A" in node:
                action = node["/A"]
                if not isinstance(action, dict) or set(action) != {"/S", "/D"} or action["/S"] != "/GoTo":
                    raise UnsupportedObservation("unsupported outline action")
                target = action["/D"]
            page, kind, parameters = destination(target)
            if page not in pages:
                raise ObservationError("outline destination is not an emitted page object")
            title = pdf_title(node.get("/Title"))
            entries.append(common_entry(len(entries) + 1, depth, title, pages[page], kind, parameters))
            siblings(_dict_ref(node, "/First"), _dict_ref(node, "/Last"), current, depth + 1)
            previous, current = current, _dict_ref(node, "/Next")
        if previous != last:
            raise ObservationError("outline Last link disagrees with traversal")
    siblings(_dict_ref(root, "/First"), _dict_ref(root, "/Last"), root_ref, 0)
    if visited != expected:
        raise ObservationError("outline traversal omits selected entries")
    if [item["reference"] for item in expected_objects] != ordered:
        raise ObservationError("independent outline object order differs")
    by_ref = {item["reference"]: item for item in expected_objects}
    for ref in visited:
        target = objects[ref].get("/Dest")
        if target is None: target = objects[ref]["/A"]["/D"]
        if destination(target)[0] != by_ref[ref]["page"]:
            raise ObservationError("independent destination object references disagree")
    return entries, pages


def compare_outline_parsers(qpdf_data, mutool_data):
    left, objects = parse_qpdf_outlines(qpdf_data)
    right, pages = parse_mutool_objects(mutool_data, objects)
    if left != right:
        raise ObservationError("independent title/hierarchy/destination observations differ")
    return {"entry_count": len(left), "page_count": len(pages),
            "entries": left, "fingerprint_sha256": hashlib.sha256(json_bytes(left)).hexdigest()}


def title_windows(record):
    """Exactly 308 starts × four strict terminated codec hypotheses."""
    if type(record) is not bytes or len(record) != RECORD_BYTES:
        raise ObservationError("title hypotheses require exactly one 308-byte record")
    for start in range(RECORD_BYTES):
        for codec in ("utf-8", "gb18030", "utf-16-le", "utf-16-be"):
            width = 2 if codec.startswith("utf-16") else 1
            end = start
            while end + width <= RECORD_BYTES and record[end:end+width] != b"\0" * width:
                end += width
            terminated = end + width <= RECORD_BYTES
            if not terminated:
                end = RECORD_BYTES
            encoded = record[start:end]
            try:
                title = encoded.decode(codec, "strict")
                decoded = title.encode("utf-8", "strict")
            except UnicodeError:
                yield start, codec, end, terminated, None
                continue
            if len(decoded) > MAX_TITLE_BYTES:
                raise ObservationError("candidate decoded title exceeds its cap")
            yield start, codec, end, terminated, byte_identity(decoded)


def numeric_candidates(record):
    """2,448 fixed-width candidates, then maximal 1..10-digit ASCII runs."""
    for width in (2, 4):
        for offset in range(RECORD_BYTES - width + 1):
            for order in ("little", "big"):
                for signed in (False, True):
                    value = int.from_bytes(record[offset:offset+width], order, signed=signed)
                    yield offset, width, order, signed, value
    for match in re.finditer(rb"[0-9]+", record):
        if len(match[0]) <= 10:
            yield match.start(), len(match[0]), "ascii-decimal", False, int(match[0])


def enumerate_records(records, reference_entries, page_count, *, record_count=None):
    """Descriptive candidate scores; no chosen source title/number field."""
    integer(page_count, "source pages", 1, MAX_PAGES)
    if record_count is None:
        if not isinstance(records, (list, tuple)):
            raise ObservationError("streamed discovery needs its declared record count")
        record_count = len(records)
    integer(record_count, "discovery record count", 1, MAX_RECORDS)
    aligned = record_count == len(reference_entries)
    title_stats, number_stats, facts = {}, {}, []
    title_attempts = numeric_attempts = 0
    all_reference_hashes = {entry["title_utf8_sha256"] for entry in reference_entries}
    observed = 0
    for ordinal, record in enumerate(records, 1):
        if ordinal > record_count or type(record) is not bytes or len(record) != RECORD_BYTES:
            raise ObservationError("discovery record count/width differs from declaration")
        observed += 1
        frequencies = Counter(record)
        entropy = -sum((count/RECORD_BYTES)*math.log2(count/RECORD_BYTES) for count in frequencies.values())
        matches, match_count = [], 0
        for offset, codec, end, terminated, identity in title_windows(record):
            title_attempts += 1
            key = (offset, codec)
            score = title_stats.setdefault(key, {"valid": 0, "any_title_matches": 0, "ordered_matches": 0})
            if identity is not None:
                score["valid"] += 1
                if identity["sha256"] in all_reference_hashes:
                    match_count += 1
                    score["any_title_matches"] += 1
                    if len(matches) < MAX_SUMMARY_CANDIDATES:
                        matches.append({"offset": offset, "end": end, "codec": codec,
                                        "terminated": terminated, **identity})
                if aligned and identity["sha256"] == reference_entries[ordinal-1]["title_utf8_sha256"]:
                    score["ordered_matches"] += 1
        for offset, width, order, signed, value in numeric_candidates(record):
            numeric_attempts += 1
            score = number_stats.setdefault((offset, width, order, signed), {"page_range": 0, "depth_range": 0, "ordered_page_matches": 0, "ordered_depth_matches": 0, "ordered_level_matches": 0})
            score["page_range"] += 1 <= value <= page_count
            score["depth_range"] += 0 <= value <= MAX_DEPTH
            if aligned:
                entry = reference_entries[ordinal-1]
                score["ordered_page_matches"] += value == entry["resolved_pdf_page_index"] + 1
                score["ordered_depth_matches"] += value == entry["depth"]
                score["ordered_level_matches"] += value == entry["depth"] + 1
        facts.append({"record": ordinal, "offset": 0x15c + (ordinal-1)*RECORD_BYTES,
                      **byte_identity(record), "entropy_bits_per_byte": entropy,
                      "nul_bytes": frequencies[0], "title_matches_total": match_count,
                      "first_title_matches": matches})
    if observed != record_count:
        raise ObservationError("discovery record stream ended before its declared count")
    titles = [{"offset": key[0], "codec": key[1], **value} for key, value in title_stats.items() if value["any_title_matches"]]
    titles.sort(key=lambda item: (-item["ordered_matches"], -item["any_title_matches"], item["offset"], item["codec"]))
    numbers = [{"offset": key[0], "width": key[1], "order": key[2], "signed": key[3], **value} for key, value in number_stats.items()]
    numbers.sort(key=lambda item: (-max(item["ordered_page_matches"], item["ordered_depth_matches"], item["ordered_level_matches"]), -item["page_range"], item["offset"], item["width"], item["order"], item["signed"]))
    return {"status": "DISCOVERY_ONLY", "enumeration": ENUMERATION,
            "record_count": record_count, "reference_entry_count": len(reference_entries),
            "ordered_alignment_hypothesis": aligned,
            "title_candidates_attempted": title_attempts,
            "numeric_candidates_attempted": numeric_attempts,
            "matching_title_candidate_count": len(titles),
            "title_candidates": titles[:MAX_SUMMARY_CANDIDATES],
            "numeric_candidate_count": len(numbers),
            "numeric_candidates": numbers[:MAX_SUMMARY_CANDIDATES], "records": facts}


def _stat_identity(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def open_regular_nofollow(path):
    """Hold each POSIX directory while opening the next; no resolve/open race."""
    path = Path(path).absolute()
    if ".." in path.parts or os.name != "posix":
        raise ObservationError("absolute confined POSIX path required")
    directory = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in path.parts[1:-1]:
            following = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=directory)
            os.close(directory); directory = following
        # Nonblocking open lets fstat refuse FIFOs without waiting for a writer;
        # regular files retain the same pread/hash semantics.
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                             dir_fd=directory)
    finally:
        os.close(directory)
    if not stat.S_ISREG(os.fstat(descriptor).st_mode):
        os.close(descriptor)
        raise ObservationError("requested input is not a regular file")
    return descriptor


class FileReader:
    """Same-inode exact reads; opaque hashes and logical windows are metered apart."""

    def __init__(self, path, report, expected=None, cap=128*MIB, deadline=None):
        self.path, self.report, self.deadline = Path(path).absolute(), report, deadline
        self.descriptor = open_regular_nofollow(self.path)
        self.original = os.fstat(self.descriptor)
        self.expected, self.cap = expected, cap
        if self.original.st_size > cap:
            self.close(); raise ObservationError("file byte ceiling exceeded")
        if expected is not None and self.original.st_size != expected["size_bytes"]:
            self.close(); raise ObservationError("file size differs from its frozen pin")

    def __enter__(self): return self

    def __exit__(self, *unused): self.close()

    def close(self):
        if self.descriptor is not None:
            os.close(self.descriptor); self.descriptor = None

    def check(self):
        if self.deadline is not None and time.monotonic() >= self.deadline:
            raise ObservationError("whole observation deadline exceeded")
        if self.report.get("cancelled", lambda: False)():
            raise KeyboardInterrupt("observation cancelled")
        if _stat_identity(os.fstat(self.descriptor)) != _stat_identity(self.original):
            raise ObservationError("input changed during its held-descriptor observation")
        measured = process.pdf._self_vm_hwm_kib()
        if measured is not None:
            self.report["resources"]["self_vm_hwm_kib"] = measured
            if measured > 256*1024:
                raise ObservationError("adapter RSS ceiling exceeded")

    def request(self, offset, length, scope):
        self.check()
        if scope not in ("opaque", "field"):
            raise ObservationError("unknown ranged-read accounting scope")
        integer(offset, "ranged read offset", 0, self.original.st_size)
        integer(length, "ranged read length", 1, 65536 if scope == "opaque" else 4096)
        resources = self.report["resources"]
        ceiling = MAX_HASH_REQUEST_BYTES if scope == "opaque" else MAX_FIELD_REQUEST_BYTES
        if resources[f"{scope}_bytes_requested"] + length > ceiling:
            raise ObservationError(f"{scope} requested-read ceiling exceeded")
        resources[f"{scope}_read_calls"] += 1
        resources[f"{scope}_bytes_requested"] += length
        resources[f"max_{scope}_read_request_bytes"] = max(resources[f"max_{scope}_read_request_bytes"], length)
        data = os.pread(self.descriptor, length, offset)
        resources[f"{scope}_bytes_read"] += len(data)
        if len(data) > length:
            raise ObservationError("ranged source overreported a read")
        self.check()
        return data

    def window(self, offset, length, chunk=308):
        integer(offset, "field offset", 0, self.original.st_size)
        integer(length, "field window", 0, MAX_PAGES*20)
        integer(chunk, "field chunk", 1, 4096)
        if offset < 0 or offset > self.original.st_size or length > self.original.st_size-offset:
            raise ObservationError("field window lies outside the declared source")
        result = bytearray()
        while len(result) < length:
            data = self.request(offset + len(result), min(chunk, length-len(result)), "field")
            if not data:
                raise ObservationError("short or zero-progress source field read")
            result.extend(data)
        return bytes(result)

    def identity(self, retain=False):
        digest, offset, data = hashlib.sha256(), 0, bytearray()
        if retain and self.original.st_size > MAX_QUERY_BYTES:
            raise ObservationError("same-read JSON byte ceiling exceeded")
        while offset < self.original.st_size:
            chunk = self.request(offset, min(65536, self.original.st_size-offset), "opaque")
            if not chunk:
                raise ObservationError("opaque hash read made no progress")
            digest.update(chunk); offset += len(chunk)
            if retain: data.extend(chunk)
        if self.request(offset, 1, "opaque"):
            raise ObservationError("opaque file has trailing growth")
        self.check()
        result = {"size_bytes": offset, "sha256": digest.hexdigest()}
        if self.expected is not None and result != self.expected:
            raise ObservationError("file differs from its frozen SHA-256 pin")
        with FileReader(self.path, self.report, cap=self.cap, deadline=self.deadline) as reopened:
            if _stat_identity(reopened.original) != _stat_identity(self.original):
                raise ObservationError("input path was replaced during observation")
        return result, bytes(data) if retain else None

    def tool_path(self):
        # Linux procfs opens the held regular inode. The child never reopens a
        # mutable caller pathname; this declared internal alias is not an input.
        self.check()
        return f"/proc/{os.getpid()}/fd/{self.descriptor}"


def inventory_header(reader):
    magic = reader.window(0, 8)
    if magic[:4] == b"\xc8\0\0\0":
        variant, prefix_size, count_at, index_at = "C8", 0x50, 8, 0x50
    elif magic[:4] == b"HN\0\0" and magic[4:] == b"\x90\x01\0\0":
        variant, prefix_size, count_at, index_at = "HN-A", 0x15c, 0x90, 0x15c
    elif magic[:4] == b"HN\0\0" and magic[4:] == b"\xc8\0\0\0":
        variant, prefix_size, count_at, index_at = "HN-B", 0xd8, 0x90, 0xd8
    else:
        raise UnsupportedObservation("unmeasured HN/C8 header signature or marker")
    prefix = reader.window(0, prefix_size)
    pages = integer(struct.unpack_from("<i", prefix, count_at)[0], "source pages", 1, MAX_PAGES)
    outline_count = None
    if variant == "HN-A":
        outline_count = integer(struct.unpack_from("<i", prefix, 0x158)[0], "outline-like count", 0, MAX_RECORDS)
        index_at += RECORD_BYTES * outline_count
    index = reader.window(index_at, pages*20, 4096)
    rows = []
    for ordinal in range(pages):
        offset, length, images = struct.unpack_from("<iih", index, ordinal*20)
        integer(offset, "text span offset"); integer(length, "text span length")
        integer(images, "page-row image count", 0, 8192)
        if offset > reader.original.st_size or length > reader.original.st_size-offset:
            raise ObservationError("declared text span is outside the source")
        rows.append({"source_page": ordinal+1, "image_count": images})
    return {"variant": variant, "source_pages": pages,
            "outline_like_records": outline_count,
            "outline_contents_status": "UNINTERPRETED",
            "outline_applicability": "UNMEASURED" if variant != "HN-A" else "RECORD_INTERVAL_ONLY",
            "prefix": {"offset": 0, **byte_identity(prefix)},
            "page_index": {"offset": index_at, **byte_identity(index)}, "rows": rows}


def new_report(origin="pinned-external-inputs"):
    return {"schema_version": 1, "status": "NOT_RUN", "origin": origin,
            "compatibility_status": "UNVERIFIED", "errors": [], "attempts": [],
            "counts": {"runner": 0, "validator_launches": 0, "native_launches": 0,
                       "render_launches": 0, "converter_launches": 0, "vendor_launches": 0,
                       "aggregate_launches": 0},
            "progress": {kind: {"planned": 0, "attempted": 0, "completed": 0,
                                "failed": 0, "remaining": 0, "unsupported": 0}
                         for kind in ("sources", "pdfs", "queries", "discovery")},
            "resources": {"max_observed_session_bytes": 0, "self_vm_hwm_kib": None,
                          **{f"{scope}_{name}": 0 for scope in ("opaque", "field")
                             for name in ("read_calls", "bytes_requested", "bytes_read")},
                          "max_opaque_read_request_bytes": 0, "max_field_read_request_bytes": 0},
            "source_inventory": [], "outline_observations": [], "audits": {}}


class Commands(process.Commands):
    """Reuse the original controller; narrow caps without changing its globals."""

    def __init__(self, session, report, deadline):
        super().__init__(session, report)
        self.deadline, self.closing = deadline, False
        self.directory_caps[session] = (MAX_QUERY_BYTES, MAX_SESSION_BYTES)

    def disk(self, reserve=0):
        total = super().disk(reserve)
        if total + reserve > MAX_SESSION_BYTES:
            raise ObservationError("outline session disk ceiling exceeded")
        return total

    def free(self):
        value = os.statvfs(self.session)
        if value.f_bavail * value.f_frsize < MAX_SESSION_BYTES:
            raise ObservationError("outline session lacks its free-space reserve")

    def child_resources(self, child, attempt, rss_cap):
        super().child_resources(child, attempt, min(rss_cap, 256*1024))

    def query(self, arguments, label, *, provenance=False):
        cap = MAX_CHILDREN if self.closing else MAX_CHILDREN-CLOSING_CHILDREN
        if self.report["counts"]["validator_launches"] >= cap:
            raise ObservationError("outline child ceiling or closing-slot reserve exceeded")
        if not provenance:
            self.report["progress"]["queries"]["attempted"] += 1
        try:
            data, _ = super().run(arguments, label, process.pdf.PdfMetadataLimits(),
                                  process.pdf._Usage(), MAX_QUERY_BYTES,
                                  include_stderr=provenance)
            attempt = self.report["attempts"][-1]
            if attempt["peak_rss_kib"] is not None and attempt["peak_rss_kib"] > 256*1024:
                attempt["status"] = "FAIL"
                raise ObservationError("reaped outline child RSS ceiling exceeded")
            if not provenance and attempt["stderr_bytes"]:
                raise ObservationError("outline tool emitted a warning; no repaired oracle accepted")
        except BaseException:
            if not provenance:
                self.report["progress"]["queries"]["failed"] += 1
            raise
        if not provenance:
            self.report["progress"]["queries"]["completed"] += 1
        return data


def capture(commands, data, name):
    if not isinstance(name, str) or not re.fullmatch(r"[a-z0-9_.-]+", name) or name in (".", ".."):
        raise ObservationError("owned artifact requires a plain basename")
    if len(data) > MAX_QUERY_BYTES:
        raise ObservationError("artifact byte ceiling exceeded")
    commands.disk(len(data))
    path = commands.session / name
    with path.open("xb") as output:
        output.write(data)
    os.chmod(path, 0o400)
    commands.disk()
    return {"name": name, **byte_identity(data)}


def observe_pdf(commands, reader, tools, label, *, expected_pages=None, previous_entries=None):
    progress = commands.report["progress"]["pdfs"]
    progress["attempted"] += 1
    try:
        data = commands.query([str(tools["qpdf"]), "--json", "--json-key=outlines",
                               "--json-stream-data=none", reader.tool_path()], label+"-qpdf")
        qpdf_artifact = capture(commands, data, label+".qpdf.json")
        entries, objects = parse_qpdf_outlines(data)
        arguments = [str(tools["mutool"]), "show", "-g", reader.tool_path(), "pages", "trailer/Root/Outlines",
                     *[str(item["reference"].number) for item in objects]]
        if sum(len(item.encode()) + 1 for item in arguments) > 16384:
            raise ObservationError("outline query argv exceeds its cap")
        second = commands.query(arguments, label+"-mutool-objects")
        mutool_artifact = capture(commands, second, label+".mutool.txt")
        facts = compare_outline_parsers(data, second)
        if expected_pages is not None and facts["page_count"] != expected_pages:
            raise ObservationError("independent PDF page count differs from pinned receipt")
        if previous_entries is not None and facts["entries"] != previous_entries:
            raise ObservationError("deterministic PDF repeats have differing outlines")
        facts.update({"label": label, "qpdf_output": qpdf_artifact, "mutool_output": mutool_artifact})
        progress["completed"] += 1
        return facts
    except BaseException as error:
        progress["failed"] += 1
        progress["unsupported"] += isinstance(error, UnsupportedObservation)
        raise


def audit(inputs, report, deadline, *, held=None, stack=None):
    """Try every declared identity; a failed earlier file cannot hide later ones."""
    result = {"status": "PASS", "attempted": 0, "verified": 0, "failed": 0, "files": {}}
    for label, path, expected, cap in inputs:
        result["attempted"] += 1
        try:
            preserve = held is not None and (label.startswith("source:") or re.fullmatch(r"(?:hn_a|c8|hn_b)_[12]", label))
            if preserve:
                reader = stack.enter_context(FileReader(path, report, expected, cap, deadline))
                identity, _ = reader.identity()
                held[label] = reader
            else:
                with FileReader(path, report, expected, cap, deadline) as reader:
                    identity, _ = reader.identity()
            result["files"][label] = identity; result["verified"] += 1
        except (ObservationError, OSError, KeyboardInterrupt) as error:
            result["status"] = "FAIL"; result["failed"] += 1
            result["files"][label] = {"status": "FAIL", "error_type": type(error).__name__}
    return result


def _startup(commands, tools, report):
    """Public-only probes, used before private audit and again during final audit."""
    libraries, versions, failures = {}, {}, {}
    for key in TOOL_KEYS:
        option = "-v" if key == "mutool" else "--version"
        for operation in ("version", "startup-libraries"):
            try:
                argv = ([str(tools[key]), option] if operation == "version" else
                        [str(tools["ldd"]), str(tools[key])])
                data = commands.query(argv, key+"-"+operation, provenance=True)
                if operation == "version":
                    text = data.decode("utf-8", "strict").strip()
                    # qpdf includes its public copyright help after the exact
                    # version line; retain the complete bounded output.
                    if not text.splitlines() or text.splitlines()[0] != TOOL_VERSIONS[key]:
                        raise ObservationError("public tool version differs from frozen profile")
                    versions[key] = {"text": text, **byte_identity(data)}
                else:
                    paths = re.findall(rb"(?:=>\s+)?(/[^\s()]+)", data)
                    if b"not found" in data or not paths or len(paths) > 128:
                        raise ObservationError("startup library discovery failed or exceeded its cap")
                    for encoded in paths:
                        path = Path(os.fsdecode(encoded)).resolve(strict=True)
                        if str(path) not in libraries:
                            with FileReader(path, report, cap=128*MIB, deadline=commands.deadline) as reader:
                                libraries[str(path)], _ = reader.identity()
            except (ObservationError, process.CompositionError, OSError, UnicodeError, KeyboardInterrupt) as error:
                failures[key+"-"+operation] = type(error).__name__
    return {"status": "FAIL" if failures else "PASS", "failures": failures,
            "versions": versions, "libraries": libraries,
            "environment": process._environment_identity(commands.environment)}


def _codec_files():
    """Initialize public CPython providers before any private title window."""
    for codec in ("utf-8", "gb18030", "utf-16-le", "utf-16-be"):
        b"".decode(codec, "strict")
    return sorted({Path(importlib.import_module(name).__file__).resolve(strict=True)
                   for name in ("encodings.utf_8", "encodings.gb18030", "encodings.utf_16_le",
                                "encodings.utf_16_be", "_codecs_cn", "_multibytecodec")})


def _source_files():
    """Only already loaded original helpers, plus the adapter's own test source."""
    paths = {ROOT / value for value in CODE_PATHS}
    for module in tuple(sys.modules.values()):
        name = getattr(module, "__file__", None)
        if name:
            path = Path(name).absolute()
            if path.is_relative_to(ROOT / "scripts") and path.suffix == ".py":
                paths.add(path)
    return sorted(paths)


def _load_plan(path, expected_sha, report, deadline):
    with FileReader(path, report, cap=256*1024, deadline=deadline) as reader:
        identity, data = reader.identity(retain=True)
    if identity["sha256"] != sha(expected_sha):
        raise ObservationError("protocol identity differs from supplied frozen pin")
    if b"Status: **FROZEN BEFORE PRIVATE OBSERVATION.**" not in data:
        raise ObservationError("protocol is not frozen; private observations remain disabled")
    match = re.search(rb"<!-- execution-contract -->\s*```json\s*\n(.*?)\n```", data, re.S)
    if match is None:
        raise ObservationError("frozen protocol lacks its exact execution contract")
    contract = decode_json(match[1], 256*1024)
    if not isinstance(contract, dict) or set(contract) != {"schema_version", "stage", "enumeration", "queries", "code_sha256"}:
        raise ObservationError("unexpected frozen execution contract schema")
    if type(contract["schema_version"]) is not int or contract["schema_version"] != 1 or contract["stage"] != "A" or contract["enumeration"] != ENUMERATION or contract["queries"] != "qpdf-outlines+mutool-g-objects":
        raise ObservationError("execution contract differs from implemented Stage A")
    pins = contract["code_sha256"]
    if not isinstance(pins, dict) or set(pins) != {str(value.relative_to(ROOT)) for value in _source_files()}:
        raise ObservationError("frozen source pin coverage differs from loaded original modules")
    for value in pins.values(): sha(value)
    return identity, pins


def _public_inputs(report, deadline):
    inputs, matrix, layout = [], None, None
    for key, (relative, expected_sha) in process.PUBLIC_PINS.items():
        path = ROOT / relative
        with FileReader(path, report, cap=MAX_QUERY_BYTES, deadline=deadline) as reader:
            identity, data = reader.identity(retain=True)
        if identity["sha256"] != expected_sha:
            raise ObservationError("public corpus metadata differs from frozen input")
        inputs.append((key, path, identity, MAX_QUERY_BYTES))
        if key == "matrix": matrix = decode_json(data)
        if key == "layout": layout = decode_json(data)
    if not isinstance(matrix, dict) or not isinstance(matrix.get("samples"), list):
        raise ObservationError("public corpus matrix framing invalid")
    rows = [row for row in matrix["samples"] if row.get("detected_type") in ("HN", "C8")]
    if len(rows) != 27 or len({row["sha256"] for row in rows}) != 27:
        raise ObservationError("Stage A requires exactly the pinned 27 source identities")
    if not isinstance(layout, dict) or not isinstance(layout.get("cases"), list):
        raise ObservationError("pinned layout count basis framing invalid")
    by_path, basis = {row["path"]: row for row in rows}, {}
    for case in layout["cases"]:
        profile = case.get("case")
        if profile not in process.BASELINE_PINS or profile in basis or case.get("source_id") not in by_path:
            raise ObservationError("pinned layout profile/source mapping differs")
        source = by_path[case["source_id"]]
        output_map = case.get("output_page_to_source_page")
        if (not isinstance(output_map, list) or not output_map or len(output_map) > MAX_PAGES
                or case.get("pdf_sha256") != process.BASELINE_PINS[profile][1]):
            raise ObservationError("pinned output count/map profile differs")
        for number in output_map: integer(number, "mapped source page", 1, source["page_count"])
        if sorted(set(output_map)) != output_map:
            raise ObservationError("pinned source/output map is not increasing and unique")
        basis[profile] = {"source_sha256": source["sha256"], "output_page_count": len(output_map)}
    if set(basis) != set(process.BASELINE_PINS):
        raise ObservationError("pinned layout profile count coverage differs")
    return inputs, rows, basis


def _matrix_source_path(root, row):
    relative = Path(row["path"])
    if relative.is_absolute() or ".." in relative.parts or not relative.parts:
        raise ObservationError("public source path leaves declared corpus root")
    return Path(root).absolute() / relative


def _reference_basis(data, source_rows, basis):
    document = decode_json(data)
    if not isinstance(document, dict) or document.get("status") != "PASS" or not isinstance(document.get("generations"), list):
        raise ObservationError("existing reference report is not a passing generation receipt")
    generations = document["generations"]
    if len(generations) != 3 or {row.get("profile") for row in generations} != set(process.BASELINE_PINS):
        raise ObservationError("reference report profile coverage differs")
    sources = {row["sha256"]: row for row in source_rows}
    output_pages = {}
    for generation in generations:
        size, expected = process.BASELINE_PINS[generation["profile"]]
        if generation.get("deterministic") is not True or generation.get("source_sha256") not in sources:
            raise ObservationError("reference generation source/determinism differs")
        if generation["source_sha256"] != basis[generation["profile"]]["source_sha256"]:
            raise ObservationError("reference generation differs from pinned profile source")
        runs = generation.get("runs")
        if not isinstance(runs, list) or len(runs) != 2:
            raise ObservationError("reference generation lacks both immutable repeats")
        for row in runs:
            if row.get("status") != "PASS" or row.get("exit_code") != 0 or row.get("pdf_size_bytes") != size or row.get("pdf_sha256") != expected:
                raise ObservationError("reference PDF differs from inherited fixed size/hash")
        # This count is inherited from the hash-pinned public layout inventory,
        # not guessed from source rows or an absent generation-report field.
        output_pages[generation["profile"]] = basis[generation["profile"]]["output_page_count"]
    return output_pages


def _complete_progress(report):
    for value in report["progress"].values():
        value["remaining"] = max(0, value["planned"] - value["attempted"])


def persist_report(commands, report):
    """A persistence/deadline failure cannot leave an overall PASS artifact."""
    path = commands.session / "observation-report.json"
    report["elapsed_seconds"] = time.monotonic() - report.pop("_started")
    try:
        if time.monotonic() >= commands.deadline:
            raise ObservationError("deadline expired before report persistence")
        data = json_bytes(report)
        if len(data) > MAX_QUERY_BYTES:
            raise ObservationError("final report byte ceiling exceeded")
        commands.disk(len(data))
        with path.open("xb") as output: output.write(data)
        if time.monotonic() >= commands.deadline:
            raise ObservationError("deadline expired during report persistence")
        os.chmod(path, 0o400)
        if time.monotonic() >= commands.deadline:
            raise ObservationError("deadline expired during final report sealing")
        return byte_identity(data)
    except (ObservationError, OSError, KeyboardInterrupt) as error:
        report["status"] = "FAIL"
        report["errors"].append(type(error).__name__ + ": final report persistence refused")
        if path.exists(): path.unlink()
        compact = {"schema_version": 1, "status": "FAIL", "origin": report["origin"],
                   "errors": report["errors"], "counts": report["counts"],
                   "progress": report["progress"], "elapsed_seconds": report["elapsed_seconds"]}
        data = json_bytes(compact)
        if len(data) <= MAX_QUERY_BYTES:
            try:
                commands.disk(len(data))
                with path.open("xb") as output: output.write(data)
                os.chmod(path, 0o400)
                return byte_identity(data)
            except (ObservationError, OSError, KeyboardInterrupt):
                report["errors"].append("FAIL summary could not be persisted")
        return None


def run(paths=None, *, plan_sha256=None):
    """Explicit inputs only; the frozen contract gate precedes every private read."""
    report = new_report()
    if paths is None:
        return report
    required = {"corpus", "reference_report", "artifact_root", "plan"} | {
        f"{profile}_{repeat}" for profile in process.BASELINE_PINS for repeat in (1, 2)}
    if (not isinstance(paths, dict) or set(paths) != required or plan_sha256 is None
            or any(not isinstance(value, (str, Path)) for value in paths.values())):
        report["status"] = "FAIL"; report["errors"].append("explicit complete inputs and frozen plan hash required")
        return report
    started = time.monotonic()
    deadline = started + TIME_LIMIT
    commands, session, stack = None, None, ExitStack()
    inputs, before_startup = [], None
    report["_started"] = started
    try:
        artifact_root = Path(paths["artifact_root"]).absolute()
        if artifact_root.is_relative_to(Path("/tmp")):
            raise ObservationError("heavy observation artifacts must use the home cache")
        plan_identity, source_pins = _load_plan(paths["plan"], plan_sha256, report, deadline)
        artifact_root.mkdir(parents=True, exist_ok=True)
        session = Path(tempfile.mkdtemp(prefix="outline-stage-a-", dir=artifact_root))
        os.chmod(session, 0o700); (session/"scratch").mkdir()
        report["session"] = str(session)
        commands = Commands(session, report, deadline)
        report["counts"]["runner"] = 1
        tools = {key: Path(sys.executable if key == "python" else shutil.which(key) or "").resolve(strict=True)
                 for key in (*TOOL_KEYS, "ldd")}
        public_inputs, rows, basis = _public_inputs(report, deadline)
        inputs.extend(public_inputs)
        inputs.append(("protocol", Path(paths["plan"]), plan_identity, 256*1024))
        for relative, expected in source_pins.items():
            path = ROOT/relative
            with FileReader(path, report, cap=2*MIB, deadline=deadline) as reader:
                identity, _ = reader.identity()
            if identity["sha256"] != expected:
                raise ObservationError("loaded original helper differs from its reviewed pin")
            inputs.append((relative, path, identity, 2*MIB))
        for key, path in tools.items():
            with FileReader(path, report, cap=128*MIB, deadline=deadline) as reader:
                identity, _ = reader.identity()
            if key in TOOL_KEYS and identity["sha256"] != process.TOOL_PINS[key]:
                raise ObservationError("tool executable differs from inherited fixed pin")
            inputs.append((key+"-executable", path, identity, 128*MIB))
        before_startup = _startup(commands, tools, report)
        if before_startup["status"] != "PASS":
            raise ObservationError("public startup provenance failed")
        for name, identity in before_startup["libraries"].items():
            inputs.append(("startup-library:"+name, Path(name), identity, 128*MIB))
        for path in _codec_files():
            with FileReader(path, report, cap=128*MIB, deadline=deadline) as reader:
                identity, _ = reader.identity()
            inputs.append(("codec-provider:"+str(path), path, identity, 128*MIB))
        sources = [(row, _matrix_source_path(paths["corpus"], row)) for row in rows]
        private_inputs = [("source:"+row["sha256"], path,
                           {"size_bytes": row["size_bytes"], "sha256": row["sha256"]}, 128*MIB)
                          for row, path in sources]
        reference_pin = {"size_bytes": REFERENCE_BYTES, "sha256": REFERENCE_SHA}
        private_inputs.append(("reference-report", Path(paths["reference_report"]), reference_pin, MAX_QUERY_BYTES))
        for profile in process.BASELINE_PINS:
            size, digest = process.BASELINE_PINS[profile]
            for repeat in (1, 2):
                label = f"{profile}_{repeat}"
                private_inputs.append((label, Path(paths[label]), {"size_bytes": size, "sha256": digest}, 128*MIB))
        inputs.extend(private_inputs)
        report["progress"]["sources"]["planned"] = len(sources)
        report["progress"]["pdfs"]["planned"] = 6
        report["progress"]["queries"]["planned"] = 12
        report["progress"]["discovery"]["planned"] = 1
        receipt = {"schema_version": 1, "stage": "A", "protocol": plan_identity,
                   "enumeration": ENUMERATION, "paths": {key: str(Path(value).absolute()) for key, value in paths.items()},
                   "tools": {key: str(value) for key, value in tools.items()},
                   "startup": before_startup, "code_sha256": source_pins,
                   "effective_environment": commands.environment,
                   "expected_inputs": {label: identity for label, _, identity, _ in inputs},
                   "queries": "qpdf-outlines+mutool-g-objects",
                   "budgets": {"children": MAX_CHILDREN, "closing_slots": CLOSING_CHILDREN,
                               "seconds": TIME_LIMIT, "disk_bytes": MAX_SESSION_BYTES}}
        report["execution_receipt"] = capture(commands, json_bytes(receipt), "execution-receipt.json")
        held = {}
        report["audits"]["before"] = audit(inputs, report, deadline, held=held, stack=stack)
        if report["audits"]["before"]["status"] != "PASS":
            raise ObservationError("complete pre-observation audit failed")
        with FileReader(paths["reference_report"], report, reference_pin, deadline=deadline) as reader:
            _, data = reader.identity(retain=True)
        reference_pages = _reference_basis(data, rows, basis)
        del data
        held_sources = {}
        for row, path in sources:
            progress = report["progress"]["sources"]; progress["attempted"] += 1
            try:
                reader = held["source:"+row["sha256"]]
                reader.check()
                header = inventory_header(reader)
                if header["source_pages"] != row["page_count"]:
                    raise ObservationError("source physical page count differs from pinned metadata")
                report["source_inventory"].append({"source_sha256": row["sha256"], **header})
                held_sources[row["sha256"]] = (reader, header)
                progress["completed"] += 1
            except BaseException as error:
                progress["failed"] += 1; progress["unsupported"] += isinstance(error, UnsupportedObservation)
                raise
        observations = {}
        for profile in process.BASELINE_PINS:
            for repeat in (1, 2):
                label = f"{profile}_{repeat}"
                reader = held[label]
                reader.check()
                facts = observe_pdf(commands, reader, tools, label,
                                    expected_pages=reference_pages[profile],
                                    previous_entries=observations[profile]["entries"] if repeat == 2 else None)
                report["outline_observations"].append(facts)
                observations[profile] = facts
        reader, header = held_sources[DISCOVERY_SHA]
        if header["variant"] != "HN-A" or header["outline_like_records"] != 52 or header["page_index"]["offset"] != 16364:
            raise ObservationError("declared HN-A discovery record window changed")
        progress = report["progress"]["discovery"]; progress["attempted"] += 1
        try:
            records = (reader.window(0x15c + number*RECORD_BYTES, RECORD_BYTES) for number in range(52))
            report["discovery"] = enumerate_records(records, observations["hn_a"]["entries"], header["source_pages"], record_count=52)
            progress["completed"] += 1
        except BaseException as error:
            progress["failed"] += 1; progress["unsupported"] += isinstance(error, UnsupportedObservation)
            raise
        report["status"] = "PASS" if observations["hn_a"]["entry_count"] else "BLOCKED"
        report["oracle_status"] = "POSITIVE_OUTLINE_OBSERVED" if report["status"] == "PASS" else "NO_POSITIVE_OUTLINE_ORACLE"
        if report["status"] == "BLOCKED": report["errors"].append("positive title/hierarchy oracle criterion remains unmet")
    except (ObservationError, process.CompositionError, process.pdf.PdfMetadataError, OSError, KeyboardInterrupt, ValueError) as error:
        report["status"] = "FAIL"; report["errors"].append(type(error).__name__ + ": requested observation failed")
    finally:
        if commands is not None:
            commands.closing = True
            if before_startup is not None:
                try:
                    after_startup = _startup(commands, tools, report)
                    report["audits"]["runtime"] = {
                        "status": "PASS" if before_startup["status"] == after_startup["status"] == "PASS"
                        and after_startup == before_startup else "FAIL"}
                except (ObservationError, process.CompositionError, OSError, KeyboardInterrupt) as error:
                    report["audits"]["runtime"] = {"status": "FAIL", "error_type": type(error).__name__}
                if report["audits"]["runtime"]["status"] != "PASS": report["status"] = "FAIL"
            report["audits"]["after"] = audit(inputs, report, deadline)
            if report["audits"]["after"]["status"] != "PASS": report["status"] = "FAIL"
            _complete_progress(report)
            report["counts"]["aggregate_launches"] = 1 + report["counts"]["validator_launches"]
            receipt = report.get("execution_receipt")
            if receipt is not None:
                try:
                    with FileReader(session/receipt["name"], report, {key:receipt[key] for key in ("size_bytes", "sha256")}, MAX_QUERY_BYTES, deadline) as reader:
                        reader.identity()
                except (ObservationError, OSError): report["status"] = "FAIL"
            report["report_file"] = persist_report(commands, report)
        else:
            report.pop("_started", None)
        stack.close()
    return report


def main(argv=None):
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--reference-report", type=Path)
    parser.add_argument("--artifact-root", type=Path)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--plan-sha256")
    for profile in process.BASELINE_PINS:
        for repeat in (1, 2): parser.add_argument(f"--{profile.replace('_','-')}-pdf-{repeat}", type=Path)
    arguments = parser.parse_args(argv)
    values = {"corpus": arguments.corpus_dir, "reference_report": arguments.reference_report,
              "artifact_root": arguments.artifact_root, "plan": arguments.plan,
              **{f"{profile}_{repeat}": getattr(arguments, f"{profile}_pdf_{repeat}")
                 for profile in process.BASELINE_PINS for repeat in (1, 2)}}
    report = run(None if not any(value is not None for value in values.values()) and arguments.plan_sha256 is None else values,
                 plan_sha256=arguments.plan_sha256)
    print(json.dumps(report, ensure_ascii=True, sort_keys=True))
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 2


if __name__ == "__main__":
    raise SystemExit(main())

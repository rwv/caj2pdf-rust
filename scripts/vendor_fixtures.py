#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Original bounded integrity tooling for external vendor fixture bundles.

This validates declarations and bytes, never runs a viewer or claims vendor
compatibility. Capturing/decoding artifacts and establishing their provenance
belong to the acquisition issues, not this reader.
"""

from __future__ import annotations

import argparse
import codecs
import copy
from contextlib import contextmanager
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time
import unicodedata


SCHEMA = "cajviewer-fixtures/1"
ORIGINS = {"viewer-native-page-image", "viewer-complete-page-capture",
           "viewer-exported-pdf-render"}
TEXT_MODES = {"existing-text-confirmed", "standard-copy-origin-unverified",
              "enhanced-copy", "ocr", "repair"}
OUTCOMES = {"PASS", "FAIL", "UNAVAILABLE", "NOT_RUN", "NO_TEXT"}
SHA = re.compile(r"[0-9a-f]{64}\Z")
IDENTIFIER = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}\Z")


class FixtureError(Exception):
    """A located, non-content integrity/schema refusal."""


@dataclass(frozen=True)
class Limits:
    manifest_bytes: int = 1 << 20
    receipt_bytes: int = 2 << 20
    text_bytes: int = 1 << 20
    artifact_bytes: int = 64 << 20
    source_bytes: int = 1 << 30
    runtime_bytes: int = 512 << 20
    total_read_bytes: int = 4 << 30
    output_bytes: int = 512 << 20
    records: int = 4096
    pages: int = 4096
    dimension: int = 32768
    files: int = 16384
    depth: int = 24
    string_bytes: int = 16384
    read_chunk: int = 65536
    seconds: float = 60.0

    def __post_init__(self):
        for name, value in vars(self).items():
            if name == "seconds":
                if type(value) not in (int, float) or not 0 < value <= 86400:
                    raise FixtureError("limits.seconds is outside the supported range")
            elif type(value) is not int or not 0 < value <= (1 << 63) - 1:
                raise FixtureError(f"limits.{name} must be a positive bounded integer")
        if self.read_chunk > 65536 or self.depth > 64:
            raise FixtureError("read/depth limits exceed the reader profile")


def _object(value, keys, location):
    if type(value) is not dict or set(value) != set(keys.split()):
        raise FixtureError(f"{location}: missing, extra or non-object fields")
    return value


def _string(value, location, maximum=512):
    try:
        valid = type(value) is str and bool(value) and len(value.encode("utf-8")) <= maximum
    except UnicodeError:
        valid = False
    if not valid:
        raise FixtureError(f"{location}: invalid bounded string")
    if any(ord(c) < 32 for c in value):
        raise FixtureError(f"{location}: control characters are not allowed")
    return value


def _id(value, location):
    if type(value) is not str or not IDENTIFIER.fullmatch(value):
        raise FixtureError(f"{location}: invalid identifier")
    return value


def _sha(value, location):
    if type(value) is not str or not SHA.fullmatch(value):
        raise FixtureError(f"{location}: invalid SHA-256")
    return value


def _integer(value, location, maximum=(1 << 63) - 1, minimum=0):
    if type(value) is not int or not minimum <= value <= maximum:
        raise FixtureError(f"{location}: integer outside the declared range")
    return value


def _choice(value, choices, location):
    if type(value) is not str or value not in choices:
        raise FixtureError(f"{location}: unknown incompatible value")
    return value


def _boolean(value, location):
    if type(value) is not bool:
        raise FixtureError(f"{location}: expected a boolean")
    return value


def _list(value, location, maximum, minimum=0):
    if type(value) is not list or not minimum <= len(value) <= maximum:
        raise FixtureError(f"{location}: invalid bounded array")
    return value


def _decimal(value, location):
    _string(value, location, 48)
    if not re.fullmatch(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?", value):
        raise FixtureError(f"{location}: expected a finite decimal string")
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        raise FixtureError(f"{location}: invalid decimal") from error
    if number.copy_abs() > Decimal(2147483647):
        raise FixtureError(f"{location}: decimal outside the geometry range")
    return number


def _path(value, location):
    _string(value, location, 2048)
    parts = value.split("/")
    if len(parts) > 16 or any(p in ("", ".", "..") for p in parts) or "\\" in value or ":" in value:
        raise FixtureError(f"{location}: expected a confined relative path")
    return parts


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise FixtureError("JSON: duplicate object key")
        result[key] = value
    return result


def _json_integer(value):
    if len(value.lstrip("-")) > 19:
        raise FixtureError("JSON: integer literal exceeds the profile")
    number = int(value)
    if not -(1 << 63) <= number <= (1 << 63) - 1:
        raise FixtureError("JSON: integer literal exceeds the profile")
    return number


def _reject_number(value):
    raise FixtureError("JSON: floating/non-finite literals are not supported; use decimal strings")


def decode_json(data, limits=Limits(), cap=None):
    """Bound depth and strings before allocating the duplicate-safe JSON tree."""
    if type(data) is not bytes or len(data) > (cap or limits.manifest_bytes):
        raise FixtureError("JSON: byte limit exceeded")
    depth, quoted, escaped, string_length = 0, False, False, 0
    for byte in data:
        if quoted:
            if byte == 34 and not escaped:
                quoted = False
            else:
                string_length += 1
                if string_length > limits.string_bytes:
                    raise FixtureError("JSON: string limit exceeded")
                escaped = byte == 92 and not escaped
        elif byte == 34:
            quoted, escaped, string_length = True, False, 0
        elif byte in (123, 91):
            depth += 1
            if depth > limits.depth:
                raise FixtureError("JSON: nesting limit exceeded")
        elif byte in (125, 93):
            depth -= 1
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=_pairs,
                          parse_int=_json_integer, parse_float=_reject_number,
                          parse_constant=_reject_number)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise FixtureError("JSON: malformed input") from error


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(data):
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def _file(value, location, limits, scope=None, cap=None):
    _object(value, "scope path bytes sha256", location)
    _choice(value["scope"], {scope} if scope else {"bundle", "source", "runtime"}, location + ".scope")
    _path(value["path"], location + ".path")
    _integer(value["bytes"], location + ".bytes", cap or limits.artifact_bytes)
    _sha(value["sha256"], location + ".sha256")


def _observation(value, location, validator):
    _object(value, "status value", location)
    _choice(value["status"], {"OBSERVED", "PINNED", "REQUESTED", "UNAVAILABLE", "NOT_RUN"}, location + ".status")
    if value["status"] in ("UNAVAILABLE", "NOT_RUN"):
        if value["value"] is not None:
            raise FixtureError(f"{location}: unavailable observation must be null")
    else:
        validator(value["value"], location + ".value")
    return value["value"]


def _identity(value, location):
    _object(value, "bytes sha256", location)
    _integer(value["bytes"], location + ".bytes")
    _sha(value["sha256"], location + ".sha256")


def _runtime(value, limits):
    _object(value, "application platform resources settings environment_sha256", "runtime")
    _sha(value["environment_sha256"], "runtime.environment_sha256")
    app = _object(value["application"], "installer installer_url build aur_commit aur_sha256 license", "runtime.application")
    _file(app["installer"], "runtime.installer", limits, "runtime", limits.runtime_bytes)
    _string(app["installer_url"], "runtime.installer_url", 2048)
    if not app["installer_url"].startswith("https://"):
        raise FixtureError("runtime.installer_url: expected a declared HTTPS provenance URL")
    _observation(app["build"], "runtime.build", _string)
    if type(app["aur_commit"]) is not str or not re.fullmatch(r"[0-9a-f]{40}", app["aur_commit"]):
        raise FixtureError("runtime.aur_commit: invalid commit")
    _sha(app["aur_sha256"], "runtime.aur_sha256")
    _string(app["license"], "runtime.license")
    platform = _object(value["platform"], "os architecture image_reference container_sha256", "runtime.platform")
    _string(platform["os"], "runtime.os")
    _string(platform["architecture"], "runtime.architecture")
    _string(platform["image_reference"], "runtime.image_reference")
    _sha(platform["container_sha256"], "runtime.container_sha256")
    resources = _object(value["resources"], "libraries plugins fonts tools", "runtime.resources")
    ids = set()
    for kind, items in resources.items():
        for item in _list(items, "runtime." + kind, limits.records):
            _object(item, "id version file", "runtime.resource")
            name = _id(item["id"], "runtime.resource.id")
            if name in ids:
                raise FixtureError("runtime.resources: duplicate resource identity")
            ids.add(name)
            _string(item["version"], "runtime.resource.version")
            _file(item["file"], "runtime.resource.file", limits, "runtime", limits.runtime_bytes)
    settings = _object(value["settings"], "locale timezone display screen_width screen_height dpi qt_scale render print clipboard", "runtime.settings")
    for key in ("locale", "timezone", "display", "render", "print", "clipboard"):
        _string(settings[key], "runtime.settings." + key)
    for key in ("screen_width", "screen_height"):
        _integer(settings[key], "runtime.settings." + key, limits.dimension, 1)
    for key in ("dpi", "qt_scale"):
        number = _observation(settings[key], "runtime.settings." + key, _decimal)
        if number is not None and Decimal(number) <= 0:
            raise FixtureError("runtime.settings: nonpositive scale")


def _raster(value, location, limits):
    _object(value, "width height depth channels alpha icc_sha256 background row_order", location)
    width = _integer(value["width"], location + ".width", limits.dimension, 1)
    height = _integer(value["height"], location + ".height", limits.dimension, 1)
    depth = _integer(value["depth"], location + ".depth", 16, 1)
    channels = _integer(value["channels"], location + ".channels", 4, 1)
    if depth not in (1, 8, 16) or channels not in (1, 3, 4) or depth == 1 and channels != 1:
        raise FixtureError(f"{location}: unsupported pixel layout")
    _choice(value["alpha"], {"none", "straight", "premultiplied"}, location + ".alpha")
    if (channels == 4) != (value["alpha"] != "none"):
        raise FixtureError(f"{location}: inconsistent alpha/channels")
    if value["icc_sha256"] is not None:
        _sha(value["icc_sha256"], location + ".icc_sha256")
    if value["background"] != "transparent" and (type(value["background"]) is not str or not re.fullmatch(r"#[0-9a-f]{6}", value["background"])):
        raise FixtureError(f"{location}: expected an explicit RGB background or transparent")
    _choice(value["row_order"], {"top-to-bottom", "bottom-to-top"}, location + ".row_order")
    size = ((width * depth * channels + 7) // 8) * height
    _integer(size, location + ".payload_size", limits.artifact_bytes)
    return size


def _image(value, location, limits):
    _object(value, "status origin format coverage artifact payload raster", location)
    _choice(value["status"], OUTCOMES - {"NO_TEXT"}, location + ".status")
    if value["status"] != "PASS":
        if any(value[key] is not None for key in ("origin", "format", "coverage", "artifact", "payload", "raster")):
            raise FixtureError(f"{location}: unavailable image must not imply acquired pixels")
        return
    _choice(value["origin"], ORIGINS, location + ".origin")
    _choice(value["format"], {"png", "tiff", "ppm", "pgm", "pbm", "raw"}, location + ".format")
    _choice(value["coverage"], {"complete-page"}, location + ".coverage")
    _file(value["artifact"], location + ".artifact", limits, "bundle")
    _file(value["payload"], location + ".payload", limits, "bundle")
    if value["payload"]["bytes"] != _raster(value["raster"], location + ".raster", limits):
        raise FixtureError(f"{location}: grid and full payload length differ")


def _text(value, location, limits):
    _object(value, "status mode selected_page coverage range mime encoding raw unicode unicode_contract code_points normalization clipboard", location)
    _choice(value["status"], OUTCOMES, location + ".status")
    if value["status"] not in ("PASS", "NO_TEXT"):
        if any(value[key] is not None for key in value if key != "status"):
            raise FixtureError(f"{location}: unavailable text must not imply acquired content")
        return
    _choice(value["mode"], TEXT_MODES, location + ".mode")
    _integer(value["selected_page"], location + ".selected_page", limits.pages, 1)
    _choice(value["coverage"], {"complete-page", "explicit-region"}, location + ".coverage")
    if value["coverage"] == "complete-page":
        if value["range"] is not None:
            raise FixtureError(f"{location}: whole-page selection must not imply a region")
    else:
        region = _object(value["range"], "unit bounds", location + ".range")
        _choice(region["unit"], {"page-points", "viewport-pixels"}, location + ".range.unit")
        bounds = [_decimal(n, location + ".range.bounds") for n in _list(region["bounds"], location + ".range.bounds", 4, 4)]
        if bounds[2] <= bounds[0] or bounds[3] <= bounds[1]:
            raise FixtureError(f"{location}: invalid selection region")
    _string(value["mime"], location + ".mime", 256)
    _choice(value["encoding"], {"utf-8", "utf-16-le", "utf-16-be", "latin-1"}, location + ".encoding")
    _file(value["raw"], location + ".raw", limits, "bundle", limits.text_bytes)
    _file(value["unicode"], location + ".unicode", limits, "bundle", limits.text_bytes)
    _choice(value["unicode_contract"], {"utf-8-strict"}, location + ".unicode_contract")
    count = _integer(value["code_points"], location + ".code_points", limits.text_bytes)
    if (value["status"] == "NO_TEXT") != (count == 0):
        raise FixtureError(f"{location}: empty copy cannot be a positive text pass")
    if value["normalization"] is not None:
        norm = _object(value["normalization"], "name file", location + ".normalization")
        _choice(norm["name"], {"NFC", "newline-lf"}, location + ".normalization.name")
        _file(norm["file"], location + ".normalization.file", limits, "bundle", limits.text_bytes)
    clip = _object(value["clipboard"], "sentinel_sha256 fresh mechanism", location + ".clipboard")
    _sha(clip["sentinel_sha256"], location + ".clipboard.sentinel_sha256")
    if _boolean(clip["fresh"], location + ".clipboard.fresh") is not True:
        raise FixtureError(f"{location}: clipboard freshness was not established")
    _string(clip["mechanism"], location + ".clipboard.mechanism")
    if clip["sentinel_sha256"] == value["raw"]["sha256"]:
        raise FixtureError(f"{location}: stale clipboard sentinel")


def validate_manifest(value, limits=Limits()):
    """Validate all metadata; acquisition claims remain externally reviewed facts."""
    _object(value, "schema profile bundle_id version basis creator runtime sources receipt history", "manifest")
    _choice(value["schema"], {SCHEMA}, "manifest.schema")
    _id(value["profile"], "manifest.profile")
    _id(value["bundle_id"], "manifest.bundle_id")
    _integer(value["version"], "manifest.version", minimum=1)
    _choice(value["basis"], {"original-synthetic", "vendor-observation"}, "manifest.basis")
    creator = _object(value["creator"], "protocol_sha256 code_commit code_sha256", "creator")
    for key in ("protocol_sha256", "code_sha256"):
        _sha(creator[key], "creator." + key)
    if type(creator["code_commit"]) is not str or not re.fullmatch(r"[0-9a-f]{40}", creator["code_commit"]):
        raise FixtureError("creator.code_commit: invalid commit")
    _runtime(value["runtime"], limits)
    source_ids, total_pages = set(), 0
    for source in _list(value["sources"], "sources", limits.records, 1):
        _object(source, "id file variant source_pages vendor_pages output_pages coverage pages", "source")
        source_id = _id(source["id"], "source.id")
        if source_id in source_ids:
            raise FixtureError("sources: duplicate canonical source ID")
        source_ids.add(source_id)
        _file(source["file"], "source.file", limits, "source", limits.source_bytes)
        _choice(source["variant"], {"CAJ", "HN-A", "HN-B", "C8", "KDH", "PDF", "TEB"}, "source.variant")
        counts = {}
        for key in ("source_pages", "vendor_pages", "output_pages"):
            _object(source[key], "status value", "source." + key)
            if source[key].get("status") == "REQUESTED":
                raise FixtureError("source: physical counts cannot be inferred from requested settings")
            counts[key] = _observation(source[key], "source." + key,
                                      lambda n, loc: _integer(n, loc, limits.pages))
        coverage = _object(source["coverage"], "kind source_pages", "source.coverage")
        _choice(coverage["kind"], {"complete", "subset"}, "source.coverage.kind")
        indices = _list(coverage["source_pages"], "source.coverage.source_pages", limits.pages)
        for index in indices:
            _integer(index, "source.coverage.source_pages", counts["source_pages"] if counts["source_pages"] is not None else limits.pages, 1)
        if indices != sorted(set(indices)):
            raise FixtureError("source.coverage: missing order or duplicate physical page")
        if coverage["kind"] == "complete" and (counts["source_pages"] is None or indices != list(range(1, counts["source_pages"] + 1))):
            raise FixtureError("source.coverage: complete scope lacks its declared source pages")
        total_pages += len(indices)
        _integer(total_pages, "sources.total_pages", limits.pages)
        records = _list(source["pages"], "source.pages", len(indices), len(indices))
        vendor_order, output_order = [], []
        for index, page in zip(indices, records):
            _object(page, "source_page vendor_page output_page box rotation capture image text", "page")
            if _integer(page["source_page"], "page.source_page", limits.pages, 1) != index:
                raise FixtureError("pages: missing, duplicate or reordered source page")
            for name, maximum, order in (("vendor_page", counts["vendor_pages"], vendor_order), ("output_page", counts["output_pages"], output_order)):
                if page[name] is not None:
                    order.append(_integer(page[name], "page." + name, maximum if maximum is not None else limits.pages, 1))
            def box_check(value, loc):
                box = [_decimal(n, loc) for n in _list(value, loc, 4, 4)]
                if box[2] <= box[0] or box[3] <= box[1]:
                    raise FixtureError("page.box: invalid physical bounds")
            _observation(page["box"], "page.box", box_check)
            rotation = _observation(page["rotation"], "page.rotation", lambda n, loc: _integer(n, loc, 270))
            if rotation is not None and rotation not in (0, 90, 180, 270):
                raise FixtureError("page.rotation: unsupported rotation")
            capture = _object(page["capture"], "dpi device_pixel_ratio zoom", "page.capture")
            for name, observation in capture.items():
                number = _observation(observation, "page.capture." + name, _decimal)
                if number is not None and Decimal(number) <= 0:
                    raise FixtureError("page.capture: nonpositive scale observation")
            _image(page["image"], "page.image", limits)
            _text(page["text"], "page.text", limits)
            if page["image"]["status"] == "PASS" or page["text"]["status"] in ("PASS", "NO_TEXT"):
                if page["vendor_page"] is None:
                    raise FixtureError("page: acquired content lacks a vendor page")
            if page["text"]["status"] in ("PASS", "NO_TEXT") and page["text"]["selected_page"] != page["vendor_page"]:
                raise FixtureError("page.text: selected page differs from mapping")
        for order in (vendor_order, output_order):
            if order != sorted(set(order)):
                raise FixtureError("pages: duplicate or reordered vendor/output mapping")
        if coverage["kind"] == "complete":
            for count, order in ((counts["vendor_pages"], vendor_order), (counts["output_pages"], output_order)):
                if count is not None and order != list(range(1, count + 1)):
                    raise FixtureError("pages: incomplete declared vendor/output mapping")
    _file(value["receipt"], "manifest.receipt", limits, "bundle", limits.receipt_bytes)
    for item in _list(value["history"], "history", limits.records):
        _object(item, "kind file", "history.item")
        _choice(item["kind"], {"baseline-manifest", "baseline-receipt", "failure-receipt", "regeneration-receipt", "review", "diff"}, "history.kind")
        _file(item["file"], "history.file", limits, "bundle", limits.receipt_bytes)
    return value


class Assets:
    """POSIX-only descriptor-anchored, no-symlink asset reader and auditor."""

    def __init__(self, roots, limits, counters, deadline):
        if not hasattr(os, "O_NOFOLLOW") or os.open not in os.supports_dir_fd:
            raise FixtureError("confined asset verification requires POSIX dir-fd/no-follow support")
        self.limits, self.counters, self.deadline = limits, counters, deadline
        self.roots, self.known = {}, {}
        try:
            for scope, path in roots.items():
                if not isinstance(path, (str, os.PathLike)):
                    raise FixtureError("declared root is not a filesystem path")
                fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                self.roots[scope] = (fd, Path(path), os.fstat(fd))
        except (OSError, FixtureError) as error:
            self.close()
            raise FixtureError("declared root is missing, symlinked or not a directory") from error

    def close(self):
        for fd, _, _ in self.roots.values():
            os.close(fd)
        self.roots.clear()

    def check(self):
        if time.monotonic() > self.deadline:
            raise FixtureError("reader wall-time limit exceeded")

    @contextmanager
    def opened(self, scope, path):
        self.check()
        parts = _path(path, "asset.path")
        if scope not in self.roots:
            raise FixtureError("asset scope lacks its declared root")
        current = os.dup(self.roots[scope][0])
        fd = None
        try:
            for component in parts[:-1]:
                following = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
                os.close(current)
                current = following
            fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=current)
            info = os.fstat(fd)
            if not stat.S_ISREG(info.st_mode):
                raise FixtureError("asset is not a regular file")
            yield fd, info
        except OSError as error:
            raise FixtureError("declared asset cannot be opened safely") from error
        finally:
            if fd is not None:
                os.close(fd)
            os.close(current)

    def declare(self, ref):
        """Register a validated identity, including work not started yet."""
        key = (ref["scope"], ref["path"])
        if key in self.known and self.known[key] != ref:
            raise FixtureError("one asset path has conflicting integrity declarations")
        if key not in self.known:
            if len(self.known) >= self.limits.files:
                raise FixtureError("asset count limit exceeded")
            self.known[key] = dict(ref)

    def read(self, ref, collect=False, cap=None, consume=None):
        """Hash/validate exactly these bytes; optional collection is record-bounded."""
        self.declare(ref)
        key = (ref["scope"], ref["path"])
        if collect and ref["bytes"] > (cap or self.limits.manifest_bytes):
            raise FixtureError("collected record limit exceeded")
        self.counters["file_attempts"] += 1
        with self.opened(*key) as (fd, before):
            if before.st_size != ref["bytes"]:
                raise FixtureError("asset exact byte length differs")
            remaining, hasher, chunks = ref["bytes"], hashlib.sha256(), []
            while True:
                self.check()
                request = min(self.limits.read_chunk, remaining) if remaining else 1
                if self.counters["requested_bytes"] + request > self.limits.total_read_bytes:
                    raise FixtureError("total requested/read byte limit exceeded")
                self.counters["read_calls"] += 1
                self.counters["requested_bytes"] += request
                self.counters["maximum_request"] = max(self.counters["maximum_request"], request)
                try:
                    block = os.read(fd, request)
                except OSError as error:
                    raise FixtureError("asset read failed") from error
                self.counters["read_bytes"] += len(block)
                if len(block) > request:
                    raise FixtureError("asset reader overreported a request")
                if not remaining:
                    if block:
                        raise FixtureError("asset has trailing bytes")
                    break
                if not block:
                    raise FixtureError("asset is truncated or made zero progress")
                remaining -= len(block)
                hasher.update(block)
                if consume is not None:
                    consume(block)
                if collect:
                    chunks.append(block)
            self.check()
            after = os.fstat(fd)
            fields = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns", "st_mode")
            if any(getattr(before, k) != getattr(after, k) for k in fields):
                raise FixtureError("asset changed during its checked read")
            if hasher.hexdigest() != ref["sha256"]:
                raise FixtureError("asset SHA-256 differs")
        self.counters["file_checks_passed"] += 1
        if collect:
            self.counters["largest_collected_record"] = max(self.counters["largest_collected_record"], ref["bytes"])
            return b"".join(chunks)
        return None

    def local(self, path, cap, expected_sha256=None, scope="catalog"):
        """Bounded same-read public record; an optional caller SHA is a trust pin."""
        path = Path(path)
        if expected_sha256 is not None:
            _sha(expected_sha256, "record.expected_sha256")
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        if scope in self.roots:
            os.close(fd)
            raise FixtureError("public record root already registered")
        self.roots[scope] = (fd, path.parent, os.fstat(fd))
        if len(self.known) >= self.limits.files:
            raise FixtureError("asset count limit exceeded")
        data, hasher = bytearray(), hashlib.sha256()
        self.counters["file_attempts"] += 1
        with self.opened(scope, path.name) as (opened, before):
            if before.st_size > cap:
                raise FixtureError("public record byte limit exceeded")
            while True:
                self.check()
                request = min(self.limits.read_chunk, before.st_size - len(data)) if len(data) < before.st_size else 1
                if self.counters["requested_bytes"] + request > self.limits.total_read_bytes:
                    raise FixtureError("public record/total read limit exceeded")
                self.counters["read_calls"] += 1
                self.counters["requested_bytes"] += request
                self.counters["maximum_request"] = max(self.counters["maximum_request"], request)
                block = os.read(opened, request)
                self.counters["read_bytes"] += len(block)
                if len(block) > request:
                    raise FixtureError("public record reader overreported a request")
                if len(data) == before.st_size:
                    if block:
                        raise FixtureError("public record has trailing bytes")
                    break
                if not block:
                    raise FixtureError("public record is truncated or made zero progress")
                data.extend(block)
                hasher.update(block)
            self.check()
            after = os.fstat(opened)
            if any(getattr(before, k) != getattr(after, k) for k in ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns", "st_mode")):
                raise FixtureError("public record changed during its checked read")
        identity = {"bytes": len(data), "sha256": hasher.hexdigest()}
        self.known[(scope, path.name)] = {"scope": scope, "path": path.name, **identity}
        if expected_sha256 is not None and identity["sha256"] != expected_sha256:
            self.known[(scope, path.name)]["sha256"] = expected_sha256
            raise FixtureError("public record SHA-256 differs from its caller pin")
        self.counters["file_checks_passed"] += 1
        self.counters["largest_collected_record"] = max(self.counters["largest_collected_record"], len(data))
        return bytes(data)

    def check_bundle_entries(self):
        """Extra/symlinked files cannot quietly join an immutable bundle."""
        expected = {path for scope, path in self.known if scope == "bundle"}
        found, entries = set(), 0
        def walk(fd, prefix, depth):
            nonlocal entries
            if depth > 16:
                raise FixtureError("bundle directory nesting limit exceeded")
            with os.scandir(fd) as iterator:
                for entry in iterator:
                    self.check()
                    entries += 1
                    if entries > self.limits.files * 2:
                        raise FixtureError("bundle directory entry limit exceeded")
                    path = prefix + entry.name
                    _path(path, "bundle.entry")
                    if entry.is_symlink():
                        raise FixtureError("bundle contains a symlink")
                    if entry.is_dir(follow_symlinks=False):
                        child = os.open(entry.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                        try:
                            walk(child, path + "/", depth + 1)
                        finally:
                            os.close(child)
                    elif entry.is_file(follow_symlinks=False):
                        found.add(path)
                    else:
                        raise FixtureError("bundle contains a non-regular entry")
        walk(self.roots["bundle"][0], "", 0)
        if found != expected:
            raise FixtureError("bundle has missing or undeclared extra files")

    def audit(self):
        failed = False
        for ref in list(self.known.values()):
            try:
                self.read(ref)
            except (FixtureError, OSError, KeyboardInterrupt):
                failed = True
        for fd, path, before in self.roots.values():
            try:
                after = os.stat(path, follow_symlinks=False)
                if not stat.S_ISDIR(after.st_mode) or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                    failed = True
            except (OSError, KeyboardInterrupt):
                failed = True
        if "bundle" in self.roots:
            try:
                self.check_bundle_entries()
            except (FixtureError, OSError, KeyboardInterrupt):
                failed = True
        if failed:
            raise FixtureError("one or more final declared-file/root/membership audits failed")


def _counters():
    return {name: 0 for name in ("file_attempts", "file_checks_passed", "read_calls", "requested_bytes",
                                "read_bytes", "maximum_request", "largest_collected_record",
                                "pages_planned", "pages_attempted", "pages_verified", "pages_failed",
                                "pages_unstarted", "image_payloads_verified", "text_records_verified",
                                "write_calls", "written_bytes", "maximum_write", "output_files",
                                "process_launches", "vendor_passes", "comparisons")}


def _text_bytes(store, text):
    decoder = codecs.getincrementaldecoder(text["encoding"])(errors="strict")
    hasher, points, byte_count, pieces = hashlib.sha256(), 0, 0, []
    def decoded_chunk(value):
        nonlocal points, byte_count
        encoded = value.encode("utf-8", errors="strict")
        points += len(value)
        byte_count += len(encoded)
        if byte_count > store.limits.text_bytes:
            raise FixtureError("text: decoded Unicode byte limit exceeded")
        hasher.update(encoded)
        if text["normalization"] is not None:
            pieces.append(value)
    try:
        store.read(text["raw"], consume=lambda block: decoded_chunk(decoder.decode(block)))
        decoded_chunk(decoder.decode(b"", final=True))
    except UnicodeError as error:
        raise FixtureError("text: invalid or truncated strict encoding") from error
    if points != text["code_points"]:
        raise FixtureError("text: Unicode length/code point contract differs")
    if {"bytes": byte_count, "sha256": hasher.hexdigest()} != {k: text["unicode"][k] for k in ("bytes", "sha256")}:
        raise FixtureError("text: raw and decoded Unicode identities differ")
    store.read(text["unicode"])
    if text["normalization"] is not None:
        norm = text["normalization"]
        decoded = "".join(pieces)
        normalized = unicodedata.normalize("NFC", decoded) if norm["name"] == "NFC" else decoded.replace("\r\n", "\n").replace("\r", "\n")
        data = normalized.encode("utf-8")
        if len(data) > store.limits.text_bytes or digest(data) != {k: norm["file"][k] for k in ("bytes", "sha256")}:
            raise FixtureError("text: named normalization identity differs")
        store.read(norm["file"])


def _pixel_bytes(store, image):
    """v1 packs samples tightly; only the final low bilevel bits are padding."""
    raster = image["raster"]
    position = 0
    row_bytes = (raster["width"] + 7) // 8
    mask = (1 << (8 - raster["width"] % 8)) - 1 if raster["width"] % 8 else 0
    def padding(block):
        nonlocal position
        if raster["depth"] == 1 and mask:
            for index in range((row_bytes - 1 - position) % row_bytes, len(block), row_bytes):
                if block[index] & mask:
                    raise FixtureError("pixels: nonzero canonical bilevel row padding")
        position += len(block)
    store.read(image["payload"], consume=padding)


def _verify(store, catalog_path, catalog_sha256=None):
    catalog_data = store.local(catalog_path, store.limits.manifest_bytes, catalog_sha256)
    try:
        os.stat("INCOMPLETE.json", dir_fd=store.roots["catalog"][0], follow_symlinks=False)
    except FileNotFoundError:
        pass
    else:
        raise FixtureError("version is marked incomplete; retained failure cannot be verified as PASS")
    catalog = decode_json(catalog_data, store.limits)
    _object(catalog, "schema profile creator bundle_id version basis review manifest summary", "catalog")
    _identity(catalog["manifest"], "catalog.manifest")
    _integer(catalog["manifest"]["bytes"], "catalog.manifest.bytes", store.limits.manifest_bytes)
    _choice(catalog["review"], {"REVIEW_REQUIRED", "REVIEWED", "ORIGINAL_CONTROL"}, "catalog.review")
    ref = {"scope": "bundle", "path": "manifest.json", **catalog["manifest"]}
    data = store.read(ref, True, store.limits.manifest_bytes)
    manifest = validate_manifest(decode_json(data, store.limits), store.limits)
    if json_bytes(catalog) != json_bytes(make_catalog(manifest, data, catalog["review"])):
        raise FixtureError("catalog: source/page/runtime projection or manifest pins differ")
    if manifest["basis"] != "original-synthetic" and catalog["review"] == "ORIGINAL_CONTROL":
        raise FixtureError("catalog: vendor data cannot be an original control")
    store.counters["pages_planned"] += sum(len(s["pages"]) for s in manifest["sources"])
    for ref in _file_refs(manifest):
        store.declare(ref)
    receipt = _receipt(decode_json(store.read(manifest["receipt"], True, store.limits.receipt_bytes), store.limits, store.limits.receipt_bytes), manifest, store.limits)
    for ref in _file_refs(manifest["runtime"]):
        store.read(ref)
    for source in manifest["sources"]:
        store.read(source["file"])
        for page in source["pages"]:
            store.counters["pages_attempted"] += 1
            try:
                if page["image"]["status"] == "PASS":
                    store.read(page["image"]["artifact"])
                    _pixel_bytes(store, page["image"])
                    store.counters["image_payloads_verified"] += 1
                if page["text"]["status"] in ("PASS", "NO_TEXT"):
                    _text_bytes(store, page["text"])
                    store.counters["text_records_verified"] += 1
                store.counters["pages_verified"] += 1
            except (FixtureError, OSError, KeyboardInterrupt):
                store.counters["pages_failed"] += 1
                raise
    for item in manifest["history"]:
        historic = decode_json(store.read(item["file"], True, store.limits.receipt_bytes), store.limits, store.limits.receipt_bytes)
        if type(historic) is not dict:
            raise FixtureError("history: receipt/manifest must be an object")
        if item["kind"] == "baseline-manifest":
            validate_manifest(historic, store.limits)
        if item["kind"] == "failure-receipt" and historic.get("status") != "FAIL":
            raise FixtureError("history: named failure receipt lacks FAIL")
    store.check_bundle_entries()
    regeneration_status = _history(store, manifest, store.limits)
    if regeneration_status is not None and catalog["review"] != regeneration_status:
        raise FixtureError("catalog: review state differs from the bound regeneration receipt")
    if regeneration_status is None and catalog["review"] == "REVIEWED":
        raise FixtureError("catalog: reviewed state lacks a bound review/regeneration receipt")
    return catalog, manifest, receipt


def run(catalog=None, roots=None, limits=Limits(), catalog_sha256=None):
    """Return fixture integrity only. No viewer, decoder or comparator is launched."""
    began = time.monotonic()
    result = {"status": "NOT_RUN", "kind": "fixture-integrity", "schema": SCHEMA,
              "schema_control": "PASS", "counts": _counters(),
              "before_audit": "NOT_RUN", "after_audit": "NOT_RUN", "error": None}
    if catalog is None and roots is None:
        try:
            validate_manifest(control_data()[0], limits)
        except FixtureError as error:
            result.update(status="FAIL", schema_control="FAIL", error=str(error)[:512])
        return result
    store = None
    try:
        if not isinstance(catalog, (str, os.PathLike)) or type(roots) is not dict or set(roots) != {"bundle", "source", "runtime"}:
            raise FixtureError("explicit verification requires catalog and bundle/source/runtime roots")
        store = Assets(roots, limits, result["counts"], began + limits.seconds)
        public, manifest, receipt = _verify(store, catalog, catalog_sha256)
        result.update({"basis": manifest["basis"], "bundle_id": manifest["bundle_id"],
                       "version": manifest["version"], "review": public["review"],
                       "catalog_identity": {k: store.known[("catalog", Path(catalog).name)][k] for k in ("bytes", "sha256")},
                       "receipt_status": receipt["status"],
                       "receipt_claimed_counts": dict(receipt["counts"])})
        result["status"] = result["before_audit"] = "PASS"
    except (FixtureError, OSError, KeyboardInterrupt) as error:
        result["status"] = result["before_audit"] = "FAIL"
        result["error"] = str(error)[:512] if isinstance(error, FixtureError) else "I/O failure or interrupted verification"
    finally:
        if store is not None:
            try:
                store.audit()
                result["after_audit"] = "PASS"
            except (FixtureError, OSError, KeyboardInterrupt):
                result["status"] = result["after_audit"] = "FAIL"
                result["error"] = result["error"] or "final integrity/root audit failed"
            finally:
                store.close()
        result["counts"]["pages_unstarted"] = result["counts"]["pages_planned"] - result["counts"]["pages_attempted"]
        result["elapsed_ms"] = int((time.monotonic() - began) * 1000)
        if time.monotonic() > began + limits.seconds:
            result["status"] = "FAIL"
            result["error"] = result["error"] or "reader wall-time limit exceeded"
    return result


def _file_refs(value):
    """Only validated objects may enter this bounded manifest traversal."""
    if type(value) is dict:
        if set(value) == {"scope", "path", "bytes", "sha256"}:
            yield value
        else:
            for item in value.values():
                yield from _file_refs(item)
    elif type(value) is list:
        for item in value:
            yield from _file_refs(item)


def public_summary(manifest):
    """A non-content projection: paths, settings, commands and names are opaque pins."""
    sources = []
    for source_index, source in enumerate(manifest["sources"], 1):
        pages = []
        for page in source["pages"]:
            image, text = page["image"], page["text"]
            pages.append({"source_page": page["source_page"], "vendor_page": page["vendor_page"],
                          "output_page": page["output_page"], "box": page["box"], "rotation": page["rotation"], "capture": page["capture"],
                          "image_status": image["status"], "image_origin": image["origin"],
                          "raster": image["raster"], "image_file": None if image["artifact"] is None else {k: image["artifact"][k] for k in ("bytes", "sha256")},
                          "pixel_payload": None if image["payload"] is None else {k: image["payload"][k] for k in ("bytes", "sha256")},
                          "text_status": text["status"], "text_mode": text["mode"],
                          "text_coverage": text["coverage"], "text_encoding": text["encoding"],
                          "text_range": text["range"], "text_unicode_contract": text["unicode_contract"],
                          "text_mime": text["mime"], "text_code_points": text["code_points"],
                          "text_raw": None if text["raw"] is None else {k: text["raw"][k] for k in ("bytes", "sha256")},
                          "text_unicode": None if text["unicode"] is None else {k: text["unicode"][k] for k in ("bytes", "sha256")},
                          "text_normalization": None if text["normalization"] is None else
                          {"name": text["normalization"]["name"], **{k: text["normalization"]["file"][k] for k in ("bytes", "sha256")}}})
        sources.append({"id": f"source-{source_index:06}",
                        "identity": {k: source["file"][k] for k in ("bytes", "sha256")},
                        "variant": source["variant"], "source_pages": source["source_pages"],
                        "vendor_pages": source["vendor_pages"], "output_pages": source["output_pages"],
                        "coverage": source["coverage"], "pages": pages})
    runtime = manifest["runtime"]
    runtime_identities = [{"kind": kind, "index": index, **{k: item["file"][k] for k in ("bytes", "sha256")}}
                          for kind, items in sorted(runtime["resources"].items()) for index, item in enumerate(items, 1)]
    return {"runtime_sha256": digest(json_bytes(runtime))["sha256"],
            "runtime_files": runtime_identities,
            "installer": {k: runtime["application"]["installer"][k] for k in ("bytes", "sha256")},
            "application_build_sha256": digest(json_bytes(runtime["application"]["build"]))["sha256"],
            "aur_commit": runtime["application"]["aur_commit"], "aur_sha256": runtime["application"]["aur_sha256"],
            "container_sha256": runtime["platform"]["container_sha256"], "sources": sources,
            "receipt": {k: manifest["receipt"][k] for k in ("bytes", "sha256")},
            "history": [{"kind": item["kind"], **{k: item["file"][k] for k in ("bytes", "sha256")}} for item in manifest["history"]]}


def make_catalog(manifest, manifest_data, review="REVIEW_REQUIRED"):
    return {"schema": SCHEMA, "profile": manifest["profile"], "creator": manifest["creator"],
            "bundle_id": manifest["bundle_id"], "version": manifest["version"],
            "basis": manifest["basis"], "review": review, "manifest": digest(manifest_data),
            "summary": public_summary(manifest)}


def observations_sha256(manifest):
    """Non-circular acquisition identity, excluding receipt/history bookkeeping."""
    return digest(json_bytes({key: manifest[key] for key in ("creator", "runtime", "sources")}))["sha256"]


def _receipt(value, manifest, limits):
    _object(value, "schema bundle_id observations_sha256 status caps resources actions counts before_audit after_audit", "receipt")
    _choice(value["schema"], {SCHEMA}, "receipt.schema")
    if value["bundle_id"] != manifest["bundle_id"]:
        raise FixtureError("receipt.bundle_id: manifest mismatch")
    if value["observations_sha256"] != observations_sha256(manifest):
        raise FixtureError("receipt.observations_sha256: stale creator/runtime/source/page declarations")
    _choice(value["status"], {"PASS", "FAIL", "NOT_RUN"}, "receipt.status")
    passing = value["status"] == "PASS"
    caps = _object(value["caps"], "wall_ms child_wall_ms memory_bytes pids file_bytes session_bytes stdout_bytes stderr_bytes", "receipt.caps")
    for key, number in caps.items():
        _integer(number, "receipt.caps." + key, minimum=1)
    if passing:
        for source in manifest["sources"]:
            for page in source["pages"]:
                for kind in ("image", "text"):
                    for ref in _file_refs(page[kind]):
                        if ref["bytes"] > caps["file_bytes"]:
                            raise FixtureError("receipt.caps.file_bytes: acquired page artifact exceeds passing acquisition cap")
    actions = _list(value["actions"], "receipt.actions", limits.records)
    completed = failed = 0
    for index, action in enumerate(actions, 1):
        _object(action, "number kind command completed status return_code wall_ms rss_bytes stdout stderr", "receipt.action")
        if action["number"] != index or type(action["number"]) is not int:
            raise FixtureError("receipt.actions: missing or reordered attempt")
        _choice(action["kind"], {"process", "ui"}, "receipt.action.kind")
        for word in _list(action["command"], "receipt.action.command", 64, 1):
            _string(word, "receipt.action.command", 4096)
        done = _boolean(action["completed"], "receipt.action.completed")
        _choice(action["status"], {"PASS", "FAIL"}, "receipt.action.status")
        if action["status"] == "PASS" and not done:
            raise FixtureError("receipt.action: incomplete passing attempt")
        if action["return_code"] is not None:
            _integer(action["return_code"], "receipt.action.return_code", 255, -255)
        if action["kind"] == "process" and done and action["return_code"] is None:
            raise FixtureError("receipt.action: missing process exit status")
        if action["status"] == "PASS" and action["return_code"] not in (None, 0):
            raise FixtureError("receipt.action: passing nonzero exit")
        _integer(action["wall_ms"], "receipt.action.wall_ms", caps["child_wall_ms"] if passing else (1 << 63) - 1)
        if action["rss_bytes"] is not None:
            _integer(action["rss_bytes"], "receipt.action.rss_bytes", caps["memory_bytes"] if passing else (1 << 63) - 1)
        for kind in ("stdout", "stderr"):
            _identity(action[kind], "receipt.action." + kind)
            _integer(action[kind]["bytes"], "receipt.action." + kind, caps[kind + "_bytes"] if passing else (1 << 63) - 1)
        completed += done
        failed += action["status"] == "FAIL"
    counts = _object(value["counts"], "planned attempted completed failed unstarted", "receipt.counts")
    for key, number in counts.items():
        _integer(number, "receipt.counts." + key, limits.records)
    if (counts["attempted"], counts["completed"], counts["failed"], counts["unstarted"]) != (len(actions), completed, failed, counts["planned"] - len(actions)):
        raise FixtureError("receipt.counts: inconsistent actual work")
    if value["status"] == "NOT_RUN" and actions:
        raise FixtureError("receipt: NOT_RUN cannot contain attempted actions")
    for key in ("before_audit", "after_audit"):
        _choice(value[key], {"PASS", "FAIL", "NOT_RUN"}, "receipt." + key)
    resources = _object(value["resources"], "elapsed_ms owned_disk_peak_bytes process_tree_memory", "receipt.resources")
    for key, cap in (("elapsed_ms", "wall_ms"), ("owned_disk_peak_bytes", "session_bytes")):
        # Failed/manual observations may lack telemetry; never invent zeroes.
        # A passing acquisition still needs measured values within its caps.
        if resources[key] is not None or value["status"] != "FAIL":
            _integer(resources[key], "receipt.resources." + key, caps[cap] if passing else (1 << 63) - 1)
    memory = _object(resources["process_tree_memory"], "status peak_bytes method", "receipt.resources.process_tree_memory")
    _choice(memory["status"], {"MEASURED", "UNAVAILABLE"}, "receipt.memory.status")
    _string(memory["method"], "receipt.memory.method")
    if memory["status"] == "MEASURED":
        _integer(memory["peak_bytes"], "receipt.memory.peak_bytes", caps["memory_bytes"] if passing else (1 << 63) - 1)
    elif memory["peak_bytes"] is not None:
        raise FixtureError("receipt.memory: unavailable memory is not zero")
    if manifest["basis"] == "vendor-observation" and value["status"] == "PASS" and memory["status"] != "MEASURED":
        raise FixtureError("receipt: passing vendor acquisition lacks process-tree memory evidence")
    if value["status"] == "PASS" and (failed or counts["unstarted"] or value["before_audit"] != "PASS" or value["after_audit"] != "PASS"):
        raise FixtureError("receipt: failed/unstarted work or audits cannot imply PASS")
    return value


def control_data(bundle_id="original-control-v1", version=1):
    """Invented runtime stand-ins, two asymmetric grids and strict-copy records.

    These are schema/integrity controls, not a converter or vendor fixture.
    Even the declared installer/build/commit are explicitly invented pins.
    """
    files = {}
    def asset(scope, path, data):
        files[(scope, path)] = data
        return {"scope": scope, "path": path, **digest(data)}
    def observed(value):
        return {"status": "OBSERVED", "value": value}
    runtime = {
        "application": {
            "installer": asset("runtime", "standins/installer.txt", b"Original MIT installer stand-in; not executable.\n"),
            "installer_url": "https://example.invalid/original-control-no-installer",
            "build": {"status": "PINNED", "value": "original-synthetic-build"},
            "aur_commit": "0" * 40, "aur_sha256": digest(b"invented recipe pin")["sha256"],
            "license": "MIT original stand-ins only; no vendor application"},
        "platform": {"os": "original-synthetic", "architecture": "not-a-runtime", "image_reference": "original-control-no-container",
                     "container_sha256": digest(b"invented container pin")["sha256"]},
        "resources": {kind: [{"id": kind + "-control", "version": "invented-v1",
                              "file": asset("runtime", "standins/" + kind + ".txt", (kind + " original MIT stand-in\n").encode())}]
                      for kind in ("libraries", "plugins", "fonts", "tools")},
        "settings": {"locale": "C.UTF-8", "timezone": "UTC", "display": "original-control-no-display",
                     "screen_width": 3, "screen_height": 2,
                     "dpi": {"status": "REQUESTED", "value": "72"},
                     "qt_scale": {"status": "UNAVAILABLE", "value": None},
                     "render": "no rendering performed", "print": "no printing performed",
                     "clipboard": "original byte fixture; no clipboard accessed"},
        "environment_sha256": digest(b"invented original control environment pin")["sha256"]}
    text_value = "A\u0301 \u4e2d\r\nfirst\ufffd"
    pages = []
    for index, pixels in enumerate((bytes((255, 0, 0, 0, 255, 0, 0, 0, 255,
                                          0, 0, 0, 255, 255, 255, 17, 83, 201)),
                                   bytes((17, 83, 201, 0, 0, 0, 255, 255, 255,
                                          0, 0, 255, 255, 0, 0, 0, 255, 0))), 1):
        text = text_value if index == 1 else ""
        raw = asset("bundle", f"text/{index}.raw", text.encode())
        unicode_file = asset("bundle", f"text/{index}.utf8", text.encode())
        normalization = None if index == 2 else {
            "name": "NFC", "file": asset("bundle", "text/1.nfc.utf8", unicodedata.normalize("NFC", text).encode())}
        pages.append({"source_page": index, "vendor_page": index, "output_page": index,
                      "box": observed(["0", "0", "3", "2"]), "rotation": observed(0),
                      "capture": {"dpi": observed("72"), "zoom": observed("1"),
                                  "device_pixel_ratio": {"status": "UNAVAILABLE", "value": None}},
                      "image": {"status": "PASS", "origin": "viewer-complete-page-capture", "format": "ppm",
                                "coverage": "complete-page",
                                "artifact": asset("bundle", f"images/{index}.ppm", b"P6\n3 2\n255\n" + pixels),
                                "payload": asset("bundle", f"pixels/{index}.rgb", pixels),
                                "raster": {"width": 3, "height": 2, "depth": 8, "channels": 3,
                                           "alpha": "none", "icc_sha256": None, "background": "#ffffff",
                                           "row_order": "top-to-bottom"}},
                      "text": {"status": "PASS" if index == 1 else "NO_TEXT",
                               "mode": "standard-copy-origin-unverified", "selected_page": index,
                               "coverage": "complete-page", "range": None, "mime": "text/plain",
                               "encoding": "utf-8", "raw": raw, "unicode": unicode_file,
                               "unicode_contract": "utf-8-strict", "code_points": len(text),
                               "normalization": normalization,
                               "clipboard": {"sentinel_sha256": digest(b"original synthetic sentinel")["sha256"],
                                             "fresh": True, "mechanism": "invented original transaction control"}}})
    manifest = {"schema": SCHEMA, "profile": "original-integrity-control-v1", "bundle_id": bundle_id,
                "version": version, "basis": "original-synthetic",
                "creator": {"protocol_sha256": digest(b"invented original control protocol")["sha256"],
                            "code_commit": "0" * 40, "code_sha256": digest(b"invented original code pin")["sha256"]},
                "runtime": runtime,
                "sources": [{"id": "original-source", "file": asset("source", "standins/source.bin", b"Original MIT source stand-in, not a document.\n"),
                             "variant": "PDF", "source_pages": observed(2), "vendor_pages": observed(2),
                             "output_pages": observed(2), "coverage": {"kind": "complete", "source_pages": [1, 2]},
                             "pages": pages}], "receipt": None, "history": []}
    receipt = {"schema": SCHEMA, "bundle_id": bundle_id, "observations_sha256": observations_sha256(manifest),
               "status": "PASS", "caps": {"wall_ms": 60000, "child_wall_ms": 1000,
                                           "memory_bytes": 1 << 20, "pids": 1, "file_bytes": 1 << 20,
                                           "session_bytes": 1 << 20, "stdout_bytes": 65536, "stderr_bytes": 32768},
               "resources": {"elapsed_ms": 0, "owned_disk_peak_bytes": 0,
                             "process_tree_memory": {"status": "UNAVAILABLE", "peak_bytes": None,
                                                     "method": "original control; no processes executed"}},
               "actions": [], "counts": {"planned": 0, "attempted": 0, "completed": 0, "failed": 0, "unstarted": 0},
               "before_audit": "PASS", "after_audit": "PASS"}
    manifest["receipt"] = asset("bundle", "receipt.json", json_bytes(receipt))
    validate_manifest(manifest)
    _receipt(receipt, manifest, Limits())
    return manifest, files


class NewVersion:
    """Exclusive new directory; no operation overwrites a predecessor or file.

    Read-only modes discourage accidental edits. They are not a filesystem
    immutability guarantee: identities and external reviewed pins remain needed.
    """

    def __init__(self, output, limits, counters, deadline, forbidden_roots=()):
        self.path, self.limits, self.counters, self.deadline = Path(output), limits, counters, deadline
        self.fd, self.files, self.directories, self.size = None, set(), {""}, 0
        if limits.output_bytes < 1024:
            raise FixtureError("output budget cannot reserve a bounded failure receipt")
        parent = os.open(self.path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            ancestor = os.dup(parent)
            try:
                for _ in range(256):
                    info = os.fstat(ancestor)
                    identity = (info.st_dev, info.st_ino)
                    if identity in forbidden_roots:
                        raise FixtureError("output version cannot be inside an input root")
                    following = os.open("..", os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=ancestor)
                    next_info = os.fstat(following)
                    if (next_info.st_dev, next_info.st_ino) == identity:
                        os.close(following)
                        break
                    os.close(ancestor)
                    ancestor = following
                else:
                    raise FixtureError("output ancestor limit exceeded")
            finally:
                os.close(ancestor)
            os.mkdir(self.path.name, mode=0o700, dir_fd=parent)
            self.fd = os.open(self.path.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            info = os.fstat(self.fd)
            self.identity = (info.st_dev, info.st_ino)
        except OSError as error:
            raise FixtureError("output version must be a fresh directory under a real parent") from error
        finally:
            os.close(parent)

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    @contextmanager
    def file(self, path, length, cap=None):
        parts = _path(path, "output.path")
        _integer(length, "output.length", cap or self.limits.artifact_bytes)
        if path in self.files or len(self.files) >= self.limits.files:
            raise FixtureError("output duplicate/file limit exceeded")
        if self.size + length > self.limits.output_bytes - 1024:
            raise FixtureError("output budget exceeded (failure receipt reserve retained)")
        if time.monotonic() > self.deadline:
            raise FixtureError("writer wall-time limit exceeded")
        current, fd = os.dup(self.fd), None
        try:
            prefix = ""
            for part in parts[:-1]:
                prefix = prefix + part + "/"
                if prefix not in self.directories:
                    os.mkdir(part, mode=0o700, dir_fd=current)
                    self.directories.add(prefix)
                following = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
                os.close(current)
                current = following
            fd = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=current)
            self.files.add(path)
            self.counters["output_files"] += 1
            amount = 0
            def write(block):
                nonlocal amount
                if amount + len(block) > length:
                    raise FixtureError("output producer exceeded declared bytes")
                position = 0
                while position < len(block):
                    if time.monotonic() > self.deadline:
                        raise FixtureError("writer wall-time limit exceeded")
                    requested = min(self.limits.read_chunk, len(block) - position)
                    self.counters["write_calls"] += 1
                    self.counters["maximum_write"] = max(self.counters["maximum_write"], requested)
                    written = os.write(fd, block[position:position + requested])
                    if not 0 < written <= requested:
                        raise FixtureError("output made zero progress or overreported a write")
                    position += written
                    amount += written
                    self.size += written
                    self.counters["written_bytes"] += written
            yield write
            if amount != length:
                raise FixtureError("output producer ended before declared bytes")
            os.fsync(fd)
            os.fchmod(fd, 0o400)
        finally:
            if fd is not None:
                os.close(fd)
            os.close(current)

    def put(self, path, data, cap=None):
        with self.file(path, len(data), cap) as write:
            write(data)
        return {"scope": "bundle", "path": path, **digest(data)}

    def copy(self, path, store, ref, cap=None):
        with self.file(path, ref["bytes"], cap) as write:
            store.read(ref, consume=write)
        return {"scope": "bundle", "path": path, **{k: ref[k] for k in ("bytes", "sha256")}}

    def failed(self):
        """Keep partial bytes; a separate located marker cannot approve them."""
        data = json_bytes({"schema": SCHEMA, "status": "FAIL", "kind": "incomplete-version",
                           "written_bytes": self.size, "files_attempted": len(self.files)})
        held, reopened = self.fd, False
        try:
            if held is None:
                held = os.open(self.path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                reopened = True
                info = os.fstat(held)
                if (info.st_dev, info.st_ino) != self.identity:
                    return False
            os.fchmod(held, 0o700)
            fd = os.open("INCOMPLETE.json", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400, dir_fd=held)
            self.files.add("INCOMPLETE.json")
            self.counters["output_files"] += 1
            try:
                position = 0
                while position < len(data):
                    request = min(self.limits.read_chunk, len(data) - position)
                    self.counters["write_calls"] += 1
                    self.counters["maximum_write"] = max(self.counters["maximum_write"], request)
                    amount = os.write(fd, data[position:position + request])
                    if not 0 < amount <= request:
                        return False
                    position += amount
                    self.size += amount
                    self.counters["written_bytes"] += amount
                os.fsync(fd)
            finally:
                os.close(fd)
            return True
        except FileExistsError:
            return True  # A marker is already present; it always refuses use.
        except OSError:
            # Partial directory is retained even if the filesystem cannot write
            # its marker; the result must still report FAIL.
            return False
        finally:
            if reopened:
                os.close(held)

    def seal(self):
        for prefix in sorted(self.directories, key=len, reverse=True):
            if prefix:
                current = os.dup(self.fd)
                try:
                    for part in prefix.rstrip("/").split("/"):
                        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=current)
                        os.close(current)
                        current = child
                    os.fchmod(current, 0o500)
                finally:
                    os.close(current)
        os.fchmod(self.fd, 0o500)
        os.fsync(self.fd)


def create_example(output, limits=Limits(), bundle_id="original-control-v1", version=1):
    """Create a new runtime-only original control tree and validate its bytes."""
    began, counters = time.monotonic(), _counters()
    writer = store = None
    result = {"status": "FAIL", "kind": "original-control-generation", "basis": "original-synthetic",
              "counts": counters, "error": None}
    try:
        manifest, files = control_data(bundle_id, version)
        writer = NewVersion(output, limits, counters, began + limits.seconds)
        for (scope, path), data in files.items():
            writer.put(scope + "/" + path, data)
        data = json_bytes(manifest)
        writer.put("bundle/manifest.json", data, limits.manifest_bytes)
        writer.put("catalog.json", json_bytes(make_catalog(manifest, data, "ORIGINAL_CONTROL")), limits.manifest_bytes)
        roots = {scope: Path(output) / scope for scope in ("bundle", "source", "runtime")}
        store = Assets(roots, limits, counters, began + limits.seconds)
        _verify(store, Path(output) / "catalog.json")
        store.audit()
        writer.seal()
        result.update({"status": "PASS", "catalog": str(Path(output) / "catalog.json"),
                       "roots": {key: str(path) for key, path in roots.items()}})
    except (FixtureError, OSError, KeyboardInterrupt) as error:
        result["error"] = str(error)[:512] if isinstance(error, FixtureError) else "generation I/O failure or interruption"
        if writer is not None:
            writer.failed()
    finally:
        if store is not None:
            store.close()
        if writer is not None:
            writer.close()
        result["elapsed_ms"] = int((time.monotonic() - began) * 1000)
        if time.monotonic() > began + limits.seconds:
            result["status"], result["error"] = "FAIL", "generation wall-time limit exceeded"
        if writer is not None and result["status"] != "PASS":
            result["incomplete_marker"] = "WRITTEN" if writer.failed() else "UNAVAILABLE"
    return result


def manifest_diff(predecessor, candidate, limits=Limits()):
    """Bounded redacted exact-scalar diff; never expose paths/text/commands."""
    differences = []
    def identity(value):
        return digest(json_bytes(value))
    def walk(before, after, path):
        if type(before) is type(after) and before == after:
            return
        if type(before) is dict and type(after) is dict and set(before) == set(after):
            for key in sorted(before):
                walk(before[key], after[key], path + "/" + key)
        elif type(before) is list and type(after) is list and len(before) == len(after):
            for index, (old, new) in enumerate(zip(before, after)):
                walk(old, new, path + "/" + str(index))
        else:
            if len(differences) >= limits.records:
                raise FixtureError("regeneration diff record limit exceeded")
            differences.append({"field": path, "before": identity(before), "after": identity(after)})
    walk(predecessor, candidate, "manifest")
    data = json_bytes({"schema": SCHEMA, "changes": differences})
    if len(data) > limits.receipt_bytes:
        raise FixtureError("regeneration diff byte limit exceeded")
    return data


def _raw_identity(store, scope, path):
    return {key: store.known[(scope, Path(path).name)][key] for key in ("bytes", "sha256")}


def _approval(value, pins):
    _object(value, "schema decision reviewer reason predecessor candidate diff code", "review")
    _choice(value["schema"], {SCHEMA}, "review.schema")
    _choice(value["decision"], {"APPROVE"}, "review.decision")
    _string(value["reviewer"], "review.reviewer")
    for key in ("reason", "predecessor", "candidate", "diff", "code"):
        if json_bytes(value[key]) != json_bytes(pins[key]):
            raise FixtureError("review: approval does not bind this exact successful regeneration")
    return value


def _eligible(manifest, receipt):
    if receipt["status"] != "PASS":
        raise FixtureError("regeneration candidate acquisition is not PASS")
    if any(page[kind]["status"] == "FAIL" for source in manifest["sources"] for page in source["pages"] for kind in ("image", "text")):
        raise FixtureError("regeneration candidate contains failed acquisition records")


def _history(store, manifest, limits):
    """Validate generated receipts against their co-stored immutable history."""
    refs = {(entry["file"]["bytes"], entry["file"]["sha256"]): entry for entry in manifest["history"]}
    latest = None
    for entry in manifest["history"]:
        if entry["kind"] != "regeneration-receipt":
            continue
        ref = entry["file"]
        value = decode_json(store.read(ref, True, limits.receipt_bytes), limits, limits.receipt_bytes)
        _object(value, "schema status reason predecessor candidate diff code review predecessor_manifest predecessor_receipt candidate_manifest", "regeneration.receipt")
        _choice(value["schema"], {SCHEMA}, "regeneration.schema")
        _choice(value["status"], {"REVIEW_REQUIRED", "REVIEWED"}, "regeneration.status")
        _choice(value["reason"], {"viewer-runtime-change", "acquisition-improvement", "source-interpretation-correction"}, "regeneration.reason")
        for key in ("predecessor", "candidate", "diff", "predecessor_manifest", "predecessor_receipt", "candidate_manifest"):
            _identity(value[key], "regeneration." + key)
        _object(value["code"], "commit bytes sha256", "regeneration.code")
        if type(value["code"]["commit"]) is not str or not re.fullmatch(r"[0-9a-f]{40}", value["code"]["commit"]):
            raise FixtureError("regeneration.code: invalid commit")
        _identity({k: value["code"][k] for k in ("bytes", "sha256")}, "regeneration.code")
        # Diff/candidate/predecessor inputs are retained in this version. Their
        # own integrity cannot silently be replaced with an approval string.
        for key in ("diff", "predecessor_manifest", "predecessor_receipt", "candidate_manifest"):
            identity = (value[key]["bytes"], value[key]["sha256"])
            if identity not in refs:
                raise FixtureError("regeneration: missing bound historical input")
        if value["review"] is not None:
            _identity(value["review"], "regeneration.review")
            identity = (value["review"]["bytes"], value["review"]["sha256"])
            if identity not in refs or refs[identity]["kind"] != "review":
                raise FixtureError("regeneration: missing bound review")
            review_data = store.read(refs[identity]["file"], True, limits.receipt_bytes)
            pins = {key: value[key] for key in ("reason", "predecessor", "candidate", "diff", "code")}
            _approval(decode_json(review_data, limits, limits.receipt_bytes), pins)
        if (value["status"] == "REVIEWED") != (value["review"] is not None):
            raise FixtureError("regeneration: reviewed state lacks exact approval")
        latest = value
    if latest is not None:
        entry = refs[(latest["candidate_manifest"]["bytes"], latest["candidate_manifest"]["sha256"])]
        candidate = validate_manifest(decode_json(store.read(entry["file"], True, limits.receipt_bytes), limits, limits.receipt_bytes), limits)
        if any(candidate[key] != manifest[key] for key in ("schema", "profile", "bundle_id", "version", "basis", "creator", "runtime", "sources", "receipt")):
            raise FixtureError("regeneration: current bundle differs from its bound candidate")
    return None if latest is None else latest["status"]


def regenerate(predecessor_catalog, predecessor_roots, candidate_catalog, candidate_roots,
               output, reason, code_commit, review=None, limits=Limits(),
               predecessor_sha256=None, candidate_sha256=None):
    """Produce a new reviewable version; never mutate, approve or replace inputs.

    Optional review is a hash-bound declared attestation, not a signature or
    proof of a human's authority. Publication remains a separate release gate.
    """
    began, counters = time.monotonic(), _counters()
    deadline = began + limits.seconds
    stores, writer, generated = [], None, None
    result = {"schema": SCHEMA, "kind": "fixture-regeneration", "status": "FAIL", "counts": counters,
              "before_audit": "NOT_RUN", "after_audit": "NOT_RUN", "review": "REVIEW_REQUIRED", "error": None}
    try:
        _choice(reason, {"viewer-runtime-change", "acquisition-improvement", "source-interpretation-correction"}, "regeneration.reason")
        if type(code_commit) is not str or not re.fullmatch(r"[0-9a-f]{40}", code_commit):
            raise FixtureError("regeneration.code_commit: invalid declared commit")
        for roots in (predecessor_roots, candidate_roots):
            if type(roots) is not dict or set(roots) != {"bundle", "source", "runtime"}:
                raise FixtureError("regeneration requires both exact input root sets")
        old = Assets(predecessor_roots, limits, counters, deadline)
        stores.append(old)
        _, baseline, prior_receipt = _verify(old, predecessor_catalog, predecessor_sha256)
        new = Assets(candidate_roots, limits, counters, deadline)
        stores.append(new)
        _, candidate, receipt = _verify(new, candidate_catalog, candidate_sha256)
        _eligible(candidate, receipt)
        if candidate["version"] != baseline["version"] + 1 or candidate["bundle_id"] == baseline["bundle_id"] or candidate["profile"] != baseline["profile"] or candidate["basis"] != baseline["basis"]:
            raise FixtureError("candidate must declare the next version, a new identity and the same profile/basis")
        result["before_audit"] = "PASS"
        diff = manifest_diff(baseline, candidate, limits)
        code_data = old.local(Path(__file__), limits.receipt_bytes, scope="tool")
        pins = {"reason": reason, "predecessor": _raw_identity(old, "catalog", predecessor_catalog),
                "candidate": _raw_identity(new, "catalog", candidate_catalog), "diff": digest(diff),
                "code": {"commit": code_commit, **digest(code_data)}}
        review_data = None
        if review is not None:
            review_data = old.local(review, limits.receipt_bytes, scope="approval")
            _approval(decode_json(review_data, limits, limits.receipt_bytes), pins)
        forbidden = {(info.st_dev, info.st_ino) for store in stores
                     for scope, (_, _, info) in store.roots.items()
                     if scope in ("bundle", "source", "runtime", "catalog")}
        writer = NewVersion(output, limits, counters, deadline, forbidden)
        produced = copy.deepcopy(candidate)
        # Copy every candidate bundle asset other than its manifest. Copying is
        # streamed and checked against the already verified identity again.
        for (scope, path), ref in list(new.known.items()):
            if scope == "bundle" and path != "manifest.json":
                writer.copy("bundle/" + path, new, ref, max(limits.artifact_bytes, limits.receipt_bytes))
        history = produced["history"]
        def preserve(kind, store, ref):
            path = "history/" + kind + "-" + ref["sha256"] + ".json"
            destination = "bundle/" + path
            if destination not in writer.files:
                writer.copy(destination, store, ref, limits.receipt_bytes)
            record = {"kind": kind, "file": {**ref, "scope": "bundle", "path": path}}
            if record not in history:
                history.append(record)
            return {k: ref[k] for k in ("bytes", "sha256")}
        old_manifest = preserve("baseline-manifest", old, old.known[("bundle", "manifest.json")])
        old_receipt = preserve("failure-receipt" if prior_receipt["status"] == "FAIL" else "baseline-receipt", old, baseline["receipt"])
        candidate_manifest = preserve("baseline-manifest", new, new.known[("bundle", "manifest.json")])
        for item in baseline["history"]:
            preserve(item["kind"], old, item["file"])
        diff_path = "history/diff-" + digest(diff)["sha256"] + ".json"
        diff_ref = writer.put("bundle/" + diff_path, diff, limits.receipt_bytes)
        diff_ref["path"] = diff_path
        history.append({"kind": "diff", "file": diff_ref})
        review_ref = None
        if review_data is not None:
            review_path = "history/review-" + digest(review_data)["sha256"] + ".json"
            review_ref = writer.put("bundle/" + review_path, review_data, limits.receipt_bytes)
            review_ref["path"] = review_path
            history.append({"kind": "review", "file": review_ref})
        regeneration_receipt = {"schema": SCHEMA, "status": "REVIEWED" if review_ref else "REVIEW_REQUIRED",
                                **pins, "review": None if review_ref is None else digest(review_data),
                                "predecessor_manifest": old_manifest, "predecessor_receipt": old_receipt,
                                "candidate_manifest": candidate_manifest}
        regen_data = json_bytes(regeneration_receipt)
        regen_path = "history/regeneration-" + digest(regen_data)["sha256"] + ".json"
        regen_ref = writer.put("bundle/" + regen_path, regen_data, limits.receipt_bytes)
        regen_ref["path"] = regen_path
        history.append({"kind": "regeneration-receipt", "file": regen_ref})
        validate_manifest(produced, limits)
        data = json_bytes(produced)
        _integer(len(data), "generated manifest", limits.manifest_bytes)
        writer.put("bundle/manifest.json", data, limits.manifest_bytes)
        public = json_bytes(make_catalog(produced, data, regeneration_receipt["status"]))
        writer.put("catalog.json", public, limits.manifest_bytes)
        generated_roots = {"bundle": Path(output) / "bundle", "source": candidate_roots["source"], "runtime": candidate_roots["runtime"]}
        generated = Assets(generated_roots, limits, counters, deadline)
        stores.append(generated)
        _verify(generated, Path(output) / "catalog.json")
        result.update({"status": "PASS", "review": regeneration_receipt["status"], "pins": pins,
                       "catalog": str(Path(output) / "catalog.json"), "catalog_identity": digest(public),
                       "roots": {key: str(path) for key, path in generated_roots.items()},
                       "changes": len(decode_json(diff, limits, limits.receipt_bytes)["changes"]),
                       "predecessor_preserved": True})
    except (FixtureError, OSError, KeyboardInterrupt) as error:
        result["error"] = str(error)[:512] if isinstance(error, FixtureError) else "regeneration I/O failure or interruption"
        if writer is not None:
            writer.failed()
    finally:
        audit_failed = False
        for store in stores:
            try:
                store.audit()
            except (FixtureError, OSError, KeyboardInterrupt):
                audit_failed = True
            finally:
                store.close()
        if stores:
            result["after_audit"] = "FAIL" if audit_failed else "PASS"
        if audit_failed:
            result["status"], result["error"] = "FAIL", result["error"] or "final regeneration/input audit failed"
        if writer is not None:
            if result["status"] == "PASS":
                try:
                    writer.seal()
                except (FixtureError, OSError, KeyboardInterrupt):
                    result["status"], result["error"] = "FAIL", "version sealing failed"
            if result["status"] != "PASS":
                writer.failed()
            writer.close()
        counters["pages_unstarted"] = counters["pages_planned"] - counters["pages_attempted"]
        result["elapsed_ms"] = int((time.monotonic() - began) * 1000)
        if time.monotonic() > deadline:
            result["status"], result["error"] = "FAIL", result["error"] or "regeneration wall-time limit exceeded"
        if writer is not None and result["status"] != "PASS":
            result["incomplete_marker"] = "WRITTEN" if writer.failed() else "UNAVAILABLE"
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit a bounded JSON result (the default)")
    sub = parser.add_subparsers(dest="operation")
    example = sub.add_parser("example", help="create original MIT integrity controls only")
    example.add_argument("--output", required=True, type=Path)
    example.add_argument("--bundle-id", default="original-control-v1")
    example.add_argument("--version", default=1, type=int)
    verify = sub.add_parser("verify", help="validate explicitly supplied external artifacts")
    regen = sub.add_parser("regenerate", help="create a new hash-bound reviewable version")
    def inputs(target, prefix=""):
        for name in ("catalog", "bundle-root", "source-root", "runtime-root"):
            target.add_argument("--" + prefix + name, required=True, type=Path)
        target.add_argument("--" + prefix + "catalog-sha256")
    inputs(verify)
    inputs(regen, "predecessor-")
    inputs(regen, "candidate-")
    regen.add_argument("--output", required=True, type=Path)
    regen.add_argument("--reason", required=True, choices=("viewer-runtime-change", "acquisition-improvement", "source-interpretation-correction"))
    regen.add_argument("--code-commit", required=True)
    regen.add_argument("--review", type=Path)
    args = parser.parse_args(argv)
    def roots(prefix=""):
        return {scope: getattr(args, prefix + scope + "_root") for scope in ("bundle", "source", "runtime")}
    if args.operation == "example":
        result = create_example(args.output, bundle_id=args.bundle_id, version=args.version)
    elif args.operation == "verify":
        result = run(args.catalog, roots(), catalog_sha256=args.catalog_sha256)
    elif args.operation == "regenerate":
        result = regenerate(args.predecessor_catalog, roots("predecessor_"), args.candidate_catalog,
                            roots("candidate_"), args.output, args.reason, args.code_commit, args.review,
                            predecessor_sha256=args.predecessor_catalog_sha256, candidate_sha256=args.candidate_catalog_sha256)
    else:
        result = run()
    encoded = json_bytes(result)
    if len(encoded) > Limits().receipt_bytes:
        encoded = json_bytes({"schema": SCHEMA, "status": "FAIL", "error": "result byte limit exceeded"})
    print(encoded.decode("ascii"), end="")
    return 0 if result["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

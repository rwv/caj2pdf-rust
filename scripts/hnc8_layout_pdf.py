#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded, black-box PDF image-placement metadata extraction.

Only a small, explicit PDF graphics subset is interpreted: q, Q, cm, and
image-XObject Do. Other content operators fail as unsupported. This is an
optional development oracle, not a general PDF renderer or conversion path.
No PDF stream, rendered pixel, or text is stored in the returned metadata.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import resource
import selectors
import shutil
import signal
import subprocess
import time
from typing import Callable, Mapping
from xml.parsers import expat


class PdfMetadataError(Exception):
    """A requested PDF, tool, limit, or cross-check failed."""


class PdfMetadataUnsupported(PdfMetadataError):
    """The PDF uses an operator or structure outside the measured subset."""


class PdfMetadataCancelled(PdfMetadataError):
    """The caller cancelled an in-progress metadata extraction."""


@dataclass(frozen=True)
class PdfMetadataLimits:
    max_pdf_bytes: int = 128 * 1024 * 1024
    max_pages: int = 4096
    max_draws: int = 16384
    max_draws_per_page: int = 256
    max_json_bytes: int = 16 * 1024 * 1024
    max_page_object_bytes: int = 64 * 1024
    max_content_bytes_per_page: int = 4 * 1024 * 1024
    max_trace_bytes_per_page: int = 16 * 1024 * 1024
    max_listing_bytes: int = 4 * 1024 * 1024
    max_stream_bytes: int = 64 * 1024 * 1024
    max_diagnostic_bytes: int = 32 * 1024
    max_child_virtual_bytes: int = 1024 * 1024 * 1024
    timeout_seconds: float = 45.0

    def __post_init__(self) -> None:
        if any(value <= 0 for value in vars(self).values()):
            raise ValueError("all PDF metadata limits must be positive")


@dataclass
class _Usage:
    max_tool_output_bytes: int = 0
    total_tool_output_bytes: int = 0
    max_child_rss_kib: int = 0
    cancelled: Callable[[], bool] | None = None


_REF = re.compile(r"^(\d+) (\d+) R$")
_NUMBER = rb"[-+]?(?:\d+(?:\.\d*)?|\.\d+)"
_NUMBER_AT = re.compile(_NUMBER)
_BOX = re.compile(rb"/MediaBox\s*\[\s*(" + _NUMBER + rb")\s+(" +
                  _NUMBER + rb")\s+(" + _NUMBER + rb")\s+(" +
                  _NUMBER + rb")\s*\]")
_PARENT = re.compile(rb"/Parent\s+(\d+)\s+(\d+)\s+R\b")
_ROTATE = re.compile(rb"/Rotate\s+([-+]?\d+)\b")
_SPACE = b"\x00\x09\x0a\x0c\x0d\x20"
_DELIMITERS = b"()<>[]{}/%"
_IDENTITY = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0)


def _check_cancel(cancelled: Callable[[], bool] | None) -> None:
    if cancelled is not None and cancelled():
        raise PdfMetadataCancelled("PDF metadata extraction was cancelled")


def _file_hash(path: Path, limit: int,
               cancelled: Callable[[], bool] | None = None) -> tuple[str, int]:
    _check_cancel(cancelled)
    try:
        size = path.stat().st_size
        if not path.is_file() or size > limit:
            raise PdfMetadataError(f"file is absent, nonregular, or exceeds {limit} bytes: {path}")
        digest = hashlib.sha256()
        total = 0
        with path.open("rb") as source:
            while chunk := source.read(65536):
                _check_cancel(cancelled)
                total += len(chunk)
                if total > limit:
                    raise PdfMetadataError(f"file changed or exceeds {limit} bytes: {path}")
                digest.update(chunk)
    except OSError as exc:
        raise PdfMetadataError(f"cannot hash requested file {path}: {exc}") from exc
    if total != size:
        raise PdfMetadataError(f"requested file changed while hashing: {path}")
    return digest.hexdigest(), size


def _self_vm_hwm_kib() -> int | None:
    """Linux process high-water RSS since exec (unlike inherited ru_maxrss)."""
    try:
        with Path("/proc/self/status").open("rb") as status:
            for line in status:
                if line.startswith(b"VmHWM:"):
                    parts = line.split()
                    if len(parts) == 3 and parts[2] == b"kB":
                        return int(parts[1])
    except (OSError, ValueError):
        pass
    return None


def _tool_path(value: Path | str, name: str) -> Path:
    candidate = shutil.which(str(value)) if not Path(value).is_absolute() else str(value)
    if candidate is None:
        raise PdfMetadataError(f"required {name} binary is absent: {value}")
    try:
        path = Path(candidate).resolve(strict=True)
    except OSError as exc:
        raise PdfMetadataError(f"required {name} binary is unavailable: {exc}") from exc
    if not path.is_file() or not os.access(path, os.X_OK):
        raise PdfMetadataError(f"required {name} binary is not executable: {path}")
    return path


def _child_limit(virtual_bytes: int) -> None:
    resource.setrlimit(resource.RLIMIT_AS, (virtual_bytes, virtual_bytes))


def _run(arguments: list[str], label: str, limits: PdfMetadataLimits, usage: _Usage,
         max_stdout: int, *, consume: Callable[[bytes], None] | None = None,
         digest_only: bool = False, include_stderr: bool = False) -> tuple[bytes | str, int]:
    """Drain both pipes with caps; kill the whole process group on any error."""
    if (consume is not None and digest_only) or (include_stderr and
                                                (consume is not None or digest_only)):
        raise ValueError("choose either a consumer or a digest")
    _check_cancel(usage.cancelled)
    env = dict(os.environ, LC_ALL="C", TZ="UTC")
    try:
        process = subprocess.Popen(
            arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, start_new_session=True, env=env,
            preexec_fn=lambda: _child_limit(limits.max_child_virtual_bytes),
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise PdfMetadataError(f"{label} could not start: {exc}") from exc
    output = bytearray()
    diagnostics = bytearray()
    digest = hashlib.sha256()
    stdout_bytes = 0
    deadline = time.monotonic() + limits.timeout_seconds
    try:
        with selectors.DefaultSelector() as selector:
            assert process.stdout is not None and process.stderr is not None
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                _check_cancel(usage.cancelled)
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise PdfMetadataError(f"{label} timed out")
                for key, _ in selector.select(min(remaining, 0.1) if
                                              usage.cancelled is not None else remaining):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                    elif key.data == "stderr":
                        diagnostics.extend(chunk)
                        if len(diagnostics) > limits.max_diagnostic_bytes:
                            raise PdfMetadataError(f"{label} diagnostic output exceeds limit")
                    else:
                        stdout_bytes += len(chunk)
                        if stdout_bytes > max_stdout:
                            raise PdfMetadataError(f"{label} output exceeds limit")
                        if consume is not None:
                            consume(chunk)
                        elif digest_only:
                            digest.update(chunk)
                        else:
                            output.extend(chunk)
        # wait4 gives this child's own peak, without mixing earlier oracle or
        # converter subprocesses into an RUSAGE_CHILDREN process-wide maximum.
        while True:
            reaped_pid, status, child_usage = os.wait4(process.pid, os.WNOHANG)
            if reaped_pid == process.pid:
                code = os.waitstatus_to_exitcode(status)
                process.returncode = code
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise PdfMetadataError(f"{label} timed out")
            _check_cancel(usage.cancelled)
            time.sleep(min(0.01, remaining))
        if code != 0:
            # A validator may quote private PDF text on stderr. Keep diagnostics
            # in this process, but never return them for a committed report.
            raise PdfMetadataError(
                f"{label} exited {code}; diagnostic bytes: {len(diagnostics)}")
        usage.max_tool_output_bytes = max(usage.max_tool_output_bytes, stdout_bytes)
        usage.total_tool_output_bytes += stdout_bytes
        usage.max_child_rss_kib = max(usage.max_child_rss_kib, child_usage.ru_maxrss)
        return (digest.hexdigest() if digest_only else
                bytes(output) + (bytes(diagnostics) if include_stderr else b"")), stdout_bytes
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        raise
    finally:
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()


def _text(data: bytes, label: str) -> str:
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise PdfMetadataError(f"{label} output is not UTF-8") from exc


def _reference(value: object, label: str) -> tuple[int, int]:
    match = _REF.fullmatch(value) if isinstance(value, str) else None
    if match is None or int(match[1]) <= 0:
        raise PdfMetadataError(f"{label} has invalid PDF object reference")
    object_id, generation = map(int, match.groups())
    if generation != 0:
        raise PdfMetadataUnsupported(f"{label} has nonzero object generation")
    return object_id, generation


def _positive_int(value: object, label: str) -> int:
    if type(value) is not int or value <= 0:
        raise PdfMetadataError(f"{label} must be a positive integer")
    return value


def _number(value: str | bytes, label: str) -> float:
    try:
        result = float(value)
    except (ValueError, TypeError) as exc:
        raise PdfMetadataError(f"{label} is not a number") from exc
    if not math.isfinite(result) or abs(result) > 1e9:
        raise PdfMetadataError(f"{label} is nonfinite or outside coordinate limit")
    return result


def _box(values: list[float], label: str) -> list[float]:
    if len(values) != 4 or not all(math.isfinite(v) and abs(v) <= 1e9 for v in values):
        raise PdfMetadataError(f"{label} has invalid MediaBox values")
    if values[0] >= values[2] or values[1] >= values[3]:
        raise PdfMetadataError(f"{label} has a nonpositive MediaBox")
    return values


def _close(left: float, right: float) -> bool:
    # MuPDF trace uses float graphics arithmetic; retain qpdf's exact numbers.
    return math.isclose(left, right, rel_tol=1e-7, abs_tol=0.002)


def _same_numbers(left: list[float], right: list[float], label: str) -> None:
    if len(left) != len(right) or any(not _close(a, b) for a, b in zip(left, right)):
        raise PdfMetadataError(f"{label} differs between independent PDF tools")


def _mul(left: tuple[float, ...], right: tuple[float, ...]) -> tuple[float, ...]:
    a, b, c, d, e, f = left
    g, h, i, j, k, l = right
    return (a*g+c*h, b*g+d*h, a*i+c*j, b*i+d*j,
            a*k+c*l+e, b*k+d*l+f)


def _top_left(matrix: tuple[float, ...], box: list[float]) -> list[float]:
    a, b, c, d, e, f = matrix
    return [a, -b, -c, d, e+c-box[0], box[3]-d-f]


def _decode_name(raw: bytes) -> str:
    def escape(match: re.Match[bytes]) -> bytes:
        return bytes((int(match[1], 16),))

    if re.search(rb"#(?![0-9A-Fa-f]{2})", raw):
        raise PdfMetadataUnsupported("content stream has invalid escaped PDF name")
    try:
        return "/" + re.sub(rb"#([0-9A-Fa-f]{2})", escape, raw).decode("ascii")
    except UnicodeDecodeError as exc:
        raise PdfMetadataUnsupported("non-ASCII PDF image name is unsupported") from exc


def _tokens(data: bytes):
    at = 0
    while at < len(data):
        item = data[at]
        if item in _SPACE:
            at += 1
        elif item == ord("%"):
            end = data.find(b"\n", at)
            at = len(data) if end < 0 else end + 1
        elif item == ord("/"):
            end = at + 1
            while end < len(data) and data[end] not in _SPACE + _DELIMITERS:
                end += 1
            if end == at + 1:
                raise PdfMetadataUnsupported("empty PDF image name")
            yield _decode_name(data[at+1:end])
            at = end
        elif item in b"+-0123456789.":
            match = _NUMBER_AT.match(data, at)
            if match is None:
                raise PdfMetadataUnsupported("malformed PDF content number")
            end = match.end()
            if end < len(data) and data[end] not in _SPACE + _DELIMITERS:
                raise PdfMetadataUnsupported("PDF content number has invalid suffix")
            yield _number(match[0], "content number")
            at = end
        elif (65 <= item <= 90) or (97 <= item <= 122):
            end = at + 1
            while end < len(data) and ((65 <= data[end] <= 90) or
                                       (97 <= data[end] <= 122)):
                end += 1
            yield data[at:end].decode("ascii")
            at = end
        else:
            raise PdfMetadataUnsupported("content uses an unsupported PDF token")


def _draws(content: bytes, images: dict[str, dict], box: list[float],
           limits: PdfMetadataLimits,
           cancelled: Callable[[], bool] | None = None) -> list[dict]:
    ctm = _IDENTITY
    stack: list[tuple[float, ...]] = []
    operands: list[str | float] = []
    result: list[dict] = []
    for token in _tokens(content):
        _check_cancel(cancelled)
        if isinstance(token, float) or token.startswith("/"):
            operands.append(token)
            if len(operands) > 6:
                raise PdfMetadataUnsupported("too many PDF content operands")
            continue
        if token == "q" and not operands:
            if len(stack) >= 64:
                raise PdfMetadataError("PDF graphics stack exceeds depth limit")
            stack.append(ctm)
        elif token == "Q" and not operands:
            if not stack:
                raise PdfMetadataError("PDF graphics stack underflow")
            ctm = stack.pop()
        elif token == "cm" and len(operands) == 6 and all(
                isinstance(v, float) for v in operands):
            ctm = _mul(ctm, tuple(operands))
            if any(not math.isfinite(v) or abs(v) > 1e9 for v in ctm):
                raise PdfMetadataError("PDF CTM is nonfinite or outside coordinate limit")
            operands.clear()
        elif token == "Do" and len(operands) == 1 and isinstance(operands[0], str):
            name = operands.pop()
            image = images.get(name)
            if image is None:
                raise PdfMetadataUnsupported("Do resource is not a supported image")
            a, b, c, d, _, _ = ctm
            if a*d-b*c == 0:
                raise PdfMetadataError("image draw has singular CTM")
            if len(result) >= limits.max_draws_per_page:
                raise PdfMetadataError("image draws per page exceed limit")
            result.append({**image, "draw_number": len(result)+1,
                           "pdf_ctm": list(ctm), "top_left_ctm": _top_left(ctm, box)})
        else:
            raise PdfMetadataUnsupported("unsupported PDF content operator or operands")
    if operands or stack:
        raise PdfMetadataError("PDF content ends with operands or unbalanced graphics state")
    return result


def _qpdf_object(pdf: Path, object_id: int, qpdf: Path, limits: PdfMetadataLimits,
                 usage: _Usage) -> bytes:
    output, _ = _run([str(qpdf), f"--show-object={object_id}", str(pdf)],
                     "qpdf object", limits, usage, limits.max_page_object_bytes)
    assert isinstance(output, bytes)
    return output


def _page_box(pdf: Path, page_id: int, qpdf: Path, limits: PdfMetadataLimits,
              usage: _Usage) -> list[float]:
    seen: set[int] = set()
    object_id = page_id
    for _ in range(32):
        if object_id in seen:
            raise PdfMetadataError("page parent cycle")
        seen.add(object_id)
        data = _qpdf_object(pdf, object_id, qpdf, limits, usage)
        rotate = _ROTATE.findall(data)
        if len(rotate) > 1:
            raise PdfMetadataError("duplicate PDF page rotation")
        if rotate and int(rotate[0]) % 360:
            raise PdfMetadataUnsupported("nonzero PDF page rotation is unsupported")
        matches = _BOX.findall(data)
        if len(matches) > 1:
            raise PdfMetadataError("duplicate PDF MediaBox")
        if matches:
            return _box([_number(part, "MediaBox") for part in matches[0]], "qpdf page")
        parent = _PARENT.search(data)
        if parent is None or int(parent[2]) != 0:
            raise PdfMetadataUnsupported("page lacks a supported inherited MediaBox")
        object_id = int(parent[1])
    raise PdfMetadataError("page parent depth exceeds limit")


def _image_resources(page: dict, page_number: int) -> dict[str, dict]:
    rows = page.get("images")
    if not isinstance(rows, list):
        raise PdfMetadataError(f"page {page_number} lacks qpdf image metadata")
    images: dict[str, dict] = {}
    for row in rows:
        if not isinstance(row, dict):
            raise PdfMetadataError("qpdf image resource is malformed")
        name = row.get("name")
        if not isinstance(name, str) or not name.startswith("/") or name in images:
            raise PdfMetadataError("qpdf image resource has invalid/duplicate name")
        object_id, generation = _reference(row.get("object"), "image resource")
        width = _positive_int(row.get("width"), "image width")
        height = _positive_int(row.get("height"), "image height")
        bits = _positive_int(row.get("bitspercomponent"), "image bits per component")
        filters = row.get("filter")
        if not isinstance(filters, list) or len(filters) != 1 or filters[0] not in (
                "/DCTDecode", "/FlateDecode"):
            raise PdfMetadataUnsupported("image XObject filter is outside DCT/Flate subset")
        color = row.get("colorspace")
        if (isinstance(color, list) and len(color) == 4 and
                color[0] == "/Indexed" and color[1] in
                ("/DeviceRGB", "/DeviceGray") and
                type(color[2]) is int and 0 <= color[2] <= 255 and
                isinstance(color[3], str)):
            color_name = "Indexed"
        elif color in ("/DeviceRGB", "/DeviceGray", "/DeviceCMYK"):
            color_name = color[1:]
        else:
            raise PdfMetadataUnsupported("image XObject color space is unsupported")
        images[name] = {
            "xobject_name": name, "object_id": object_id, "generation": generation,
            "xobject_type": "Image", "width": width, "height": height,
            "bits_per_component": bits, "color_space": color_name,
            "filter": filters[0],
        }
    return images


def _mutool_pages(data: bytes, limits: PdfMetadataLimits) -> list[list[float]]:
    try:
        from xml.etree import ElementTree as ET
        root = ET.fromstring(b"<root>" + data + b"</root>")
    except ET.ParseError as exc:
        raise PdfMetadataError("mutool pages output is malformed") from exc
    pages: list[list[float]] = []
    for page in root:
        if page.tag != "page" or page.attrib.get("pagenum") != str(len(pages)+1):
            raise PdfMetadataError("mutool pages output has invalid page order")
        if len(page) != 1 or page[0].tag != "MediaBox":
            raise PdfMetadataError("mutool pages output lacks one MediaBox")
        box = page[0]
        pages.append(_box([_number(box.attrib.get(key), "mutool MediaBox")
                           for key in ("l", "b", "r", "t")], "mutool page"))
        if len(pages) > limits.max_pages:
            raise PdfMetadataError("mutool page count exceeds limit")
    return pages


class _Trace:
    def __init__(self, limits: PdfMetadataLimits):
        self.box: list[float] | None = None
        self.draws: list[tuple[list[float], int, int]] = []
        self.pages = 0
        self.parser = expat.ParserCreate()
        self.parser.StartElementHandler = self._start
        self.parser.StartDoctypeDeclHandler = self._forbid_doctype
        self.limits = limits

    @staticmethod
    def _forbid_doctype(*_args: object) -> None:
        raise PdfMetadataUnsupported("mutool trace XML has a DOCTYPE")

    def _start(self, name: str, attrs: dict[str, str]) -> None:
        if name == "page":
            self.pages += 1
            if self.pages != 1:
                raise PdfMetadataError("mutool trace has multiple pages")
            raw = attrs.get("mediabox", "").split()
            self.box = _box([_number(value, "trace MediaBox") for value in raw],
                            "mutool trace")
        elif name == "fill_image":
            if self.pages != 1 or len(self.draws) >= self.limits.max_draws_per_page:
                raise PdfMetadataError("mutool trace image count exceeds page limit")
            matrix = [_number(value, "trace CTM") for value in
                      attrs.get("transform", "").split()]
            if len(matrix) != 6:
                raise PdfMetadataError("mutool trace image CTM is malformed")
            try:
                width = int(attrs["width"])
                height = int(attrs["height"])
            except (KeyError, ValueError) as exc:
                raise PdfMetadataError("mutool trace image dimensions are malformed") from exc
            self.draws.append((matrix, _positive_int(width, "trace image width"),
                               _positive_int(height, "trace image height")))
        elif name in ("fill_image_mask", "clip_image_mask"):
            raise PdfMetadataUnsupported("mutool trace has unsupported image mask")

    def feed(self, data: bytes) -> None:
        try:
            self.parser.Parse(data, False)
        except expat.ExpatError as exc:
            raise PdfMetadataError("mutool trace XML is malformed") from exc

    def finish(self) -> None:
        try:
            self.parser.Parse(b"", True)
        except expat.ExpatError as exc:
            raise PdfMetadataError("mutool trace XML is truncated") from exc
        if self.pages != 1 or self.box is None:
            raise PdfMetadataError("mutool trace omitted its page")


def _pdfimages(data: bytes) -> list[dict]:
    lines = _text(data, "pdfimages list").splitlines()
    if len(lines) < 2 or not lines[0].startswith("page") or not lines[1].startswith("---"):
        raise PdfMetadataError("pdfimages list header is missing")
    result: list[dict] = []
    for line in lines[2:]:
        parts = line.split()
        if len(parts) != 16:
            raise PdfMetadataError("pdfimages list row has unexpected column count")
        try:
            page, index, width, height = (int(parts[k]) for k in (0, 1, 3, 4))
            components, bits = int(parts[6]), int(parts[7])
            object_id, generation = int(parts[10]), int(parts[11])
        except ValueError as exc:
            raise PdfMetadataError("pdfimages list row has malformed integer") from exc
        if parts[2] != "image" or generation != 0:
            raise PdfMetadataUnsupported("pdfimages found a non-image/mask or generation")
        if index != len(result) or min(page, width, height, components, bits, object_id) <= 0:
            raise PdfMetadataError("pdfimages list row has invalid draw order/dimension")
        result.append({"page_number": page, "object_id": object_id,
                       "generation": generation, "width": width, "height": height,
                       "color": parts[5], "components": components,
                       "bits_per_component": bits, "encoding": parts[8]})
    return result


def _expected_poppler(image: dict) -> tuple[str, int, str]:
    colors = {"DeviceRGB": ("rgb", 3), "DeviceGray": ("gray", 1),
              "DeviceCMYK": ("cmyk", 4), "Indexed": ("index", 1)}
    color, components = colors[image["color_space"]]
    encoding = "jpeg" if image["filter"] == "/DCTDecode" else "image"
    return color, components, encoding


def extract_pdf_metadata(
    pdf: Path, tools: Mapping[str, Path | str], *,
    limits: PdfMetadataLimits = PdfMetadataLimits(),
    cancelled: Callable[[], bool] | None = None,
) -> dict:
    """Return checked, compact page/image metadata; fail on tool disagreement.

    `tools` must supply qpdf, mutool, and pdfimages executables. Stream hashes
    cover original encoded PDF image stream bytes, not decoded pixels.
    """
    try:
        pdf = Path(pdf).resolve(strict=True)
    except OSError as exc:
        raise PdfMetadataError(f"reference PDF is unavailable: {exc}") from exc
    if set(tools) != {"qpdf", "mutool", "pdfimages"}:
        raise PdfMetadataError("tools must name qpdf, mutool, and pdfimages")
    usage = _Usage(cancelled=cancelled)
    pdf_digest, pdf_size = _file_hash(pdf, limits.max_pdf_bytes, cancelled)
    paths = {name: _tool_path(tools[name], name) for name in
             ("qpdf", "mutool", "pdfimages")}
    identities: dict[str, dict] = {}
    for name, path in paths.items():
        digest, size = _file_hash(path, limits.max_pdf_bytes, cancelled)
        command = [str(path), "-v" if name != "qpdf" else "--version"]
        version, _ = _run(command, f"{name} version", limits, usage,
                          limits.max_diagnostic_bytes, include_stderr=True)
        assert isinstance(version, bytes)
        identities[name] = {"path": str(path), "sha256": digest,
                            "size_bytes": size, "version": _text(version, name).strip()}
    check, _ = _run([str(paths["qpdf"]), "--check", str(pdf)], "qpdf check", limits,
                    usage, limits.max_diagnostic_bytes)
    if not isinstance(check, bytes) or b"File is not encrypted" not in check:
        raise PdfMetadataUnsupported("encrypted PDF or unknown qpdf check protocol")
    raw_json, _ = _run([str(paths["qpdf"]), "--json", "--json-key=pages", str(pdf)],
                       "qpdf pages JSON", limits, usage, limits.max_json_bytes)
    assert isinstance(raw_json, bytes)
    try:
        catalog = json.loads(raw_json)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise PdfMetadataError("qpdf pages JSON is malformed") from exc
    rows = catalog.get("pages") if isinstance(catalog, dict) else None
    if not isinstance(rows, list) or not 0 < len(rows) <= limits.max_pages:
        raise PdfMetadataError("qpdf page count is empty or exceeds limit")
    mutool_pages, _ = _run([str(paths["mutool"]), "pages", str(pdf)],
                           "mutool pages", limits, usage, limits.max_listing_bytes)
    assert isinstance(mutool_pages, bytes)
    boxes = _mutool_pages(mutool_pages, limits)
    if len(boxes) != len(rows):
        raise PdfMetadataError("qpdf and mutool disagree on page count")
    poppler_rows, _ = _run([str(paths["pdfimages"]), "-list", str(pdf)],
                           "pdfimages list", limits, usage, limits.max_listing_bytes)
    assert isinstance(poppler_rows, bytes)
    listed = _pdfimages(poppler_rows)
    if len(listed) > limits.max_draws:
        raise PdfMetadataError("pdfimages draw count exceeds limit")
    hashes: dict[int, tuple[str, int]] = {}
    pages: list[dict] = []
    next_listed = 0
    for page_number, row in enumerate(rows, 1):
        if not isinstance(row, dict) or row.get("pageposfrom1") != page_number:
            raise PdfMetadataError("qpdf page positions are invalid")
        page_id, _ = _reference(row.get("object"), "page")
        box = _page_box(pdf, page_id, paths["qpdf"], limits, usage)
        _same_numbers(box, boxes[page_number-1], f"page {page_number} MediaBox")
        resources = _image_resources(row, page_number)
        references = row.get("contents")
        if not isinstance(references, list):
            raise PdfMetadataError("qpdf page contents metadata is malformed")
        content = bytearray()
        for value in references:
            object_id, _ = _reference(value, "page content")
            remaining = limits.max_content_bytes_per_page - len(content)
            if remaining <= 0:
                raise PdfMetadataError("page content exceeds limit")
            decoded, _ = _run(
                [str(paths["qpdf"]), f"--show-object={object_id}",
                 "--filtered-stream-data", str(pdf)],
                "qpdf content", limits, usage, remaining,
            )
            assert isinstance(decoded, bytes)
            content.extend(decoded)
            content.extend(b"\n")
            if len(content) > limits.max_content_bytes_per_page:
                raise PdfMetadataError("page content exceeds limit")
        draws = _draws(bytes(content), resources, box, limits, cancelled)
        if next_listed + len(draws) > limits.max_draws:
            raise PdfMetadataError("total image draws exceed limit")
        trace = _Trace(limits)
        _run([str(paths["mutool"]), "draw", "-q", "-F", "trace", "-o", "-",
              str(pdf), str(page_number)], "mutool page trace", limits, usage,
             limits.max_trace_bytes_per_page, consume=trace.feed)
        trace.finish()
        assert trace.box is not None
        # `mutool pages` reports the original MediaBox; `draw -F trace` moves
        # the page origin to (0, 0), retaining only its width and height.
        _same_numbers([0.0, 0.0, box[2]-box[0], box[3]-box[1]], trace.box,
                      f"page {page_number} trace MediaBox")
        if len(draws) != len(trace.draws):
            raise PdfMetadataError(f"page {page_number} qpdf/mutool image draw count differs")
        for draw, traced in zip(draws, trace.draws):
            matrix, width, height = traced
            _same_numbers(draw["top_left_ctm"], matrix,
                          f"page {page_number} draw {draw['draw_number']} CTM")
            if (draw["width"], draw["height"]) != (width, height):
                raise PdfMetadataError("qpdf/mutool image dimensions differ")
            if next_listed >= len(listed):
                raise PdfMetadataError("pdfimages list has fewer draws than qpdf/mutool")
            poppler = listed[next_listed]
            next_listed += 1
            color, components, encoding = _expected_poppler(draw)
            if (poppler["page_number"], poppler["object_id"],
                    poppler["generation"], poppler["width"], poppler["height"],
                    poppler["bits_per_component"], poppler["color"],
                    poppler["components"], poppler["encoding"]) != (
                    page_number, draw["object_id"], draw["generation"],
                    draw["width"], draw["height"], draw["bits_per_component"],
                    color, components, encoding):
                raise PdfMetadataError("Poppler image order/identity/type differs")
            object_id = draw["object_id"]
            if object_id not in hashes:
                qhash, qlength = _run(
                    [str(paths["qpdf"]), f"--show-object={object_id}",
                     "--raw-stream-data", str(pdf)],
                    "qpdf raw image", limits, usage, limits.max_stream_bytes,
                    digest_only=True)
                mhash, mlength = _run(
                    [str(paths["mutool"]), "show", "-b", "-e", str(pdf),
                     str(object_id)], "mutool raw image", limits, usage,
                    limits.max_stream_bytes, digest_only=True)
                if qhash != mhash or qlength != mlength:
                    raise PdfMetadataError("qpdf/mutool raw image stream hash differs")
                assert isinstance(qhash, str)
                hashes[object_id] = (qhash, qlength)
            draw["raw_stream_sha256"], draw["raw_stream_length"] = hashes[object_id]
        pages.append({"page_number": page_number, "media_box": box, "draws": draws})
    if next_listed != len(listed):
        raise PdfMetadataError("pdfimages list has extra image draws")
    if _file_hash(pdf, limits.max_pdf_bytes, cancelled) != (pdf_digest, pdf_size):
        raise PdfMetadataError("reference PDF changed during extraction")
    for name, identity in identities.items():
        if _file_hash(paths[name], limits.max_pdf_bytes, cancelled) != (
                identity["sha256"], identity["size_bytes"]):
            raise PdfMetadataError(f"{name} binary changed during extraction")
    return {"status": "PASS", "pdf_sha256": pdf_digest, "pdf_size_bytes": pdf_size,
            "page_count": len(pages), "draw_count": next_listed, "pages": pages,
            "tools": identities,
            "resources": {"max_tool_output_bytes": usage.max_tool_output_bytes,
                          "total_tool_output_bytes": usage.total_tool_output_bytes,
                          "max_child_rss_kib": usage.max_child_rss_kib,
                          "self_vm_hwm_kib": _self_vm_hwm_kib(),
                          "max_temporary_bytes": 0,
                          "temporary_bytes_scope": "extractor-managed files only",
                          "file_hash_read_bytes": 65536}}

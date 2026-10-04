#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in full-array and complete-page checks for source-derived HN/C8 PDFs.

No external input is opened in a no-argument invocation. Private arrays,
renders and diagnostics stay in a caller-owned external session. This is an
original development verifier, not a converter or production dispatch path.
"""

from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import resource
import selectors
import signal
import subprocess
import sys
import tempfile
import time
from typing import BinaryIO, Callable, Mapping

import hnc8_layout_pdf as pdf
import hnc8_placement_rule as placement

ROOT = Path(__file__).resolve().parent.parent
CHUNK = 65536
MIB = 1024 * 1024
SOURCE_LIMIT = 8 * 1024 * MIB
PDF_LIMIT = 128 * MIB
RASTER_LIMIT = 64 * MIB
DISK_LIMIT = 512 * MIB
FREE_BYTES = 1024 * MIB
STDOUT_LIMIT = MIB
DIAGNOSTIC_LIMIT = 32768
TIME_LIMIT = 1800.0
TOOL_LIMIT = 2048
NATIVE_LAUNCH_LIMIT = 3
RENDER_LAUNCH_LIMIT = 308
TOLERANCE = 0.00005
TABLE_SHA = "11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7"
REFERENCE_SHA = "8cc9fce6c8c0f1f97f7d39453d0734d053fd543aa023a04ef835bf187918d959"
PUBLIC_PINS = {
    "matrix": ("tests/conformance/matrix.json", "af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9"),
    "layout": ("tests/conformance/hnc8_layout_oracle.json", "4b88befeecf9a68dd6eca4966c79ea8cdb130c43e3c6d92f4cb56fb34dfb665e"),
    "jbig1": ("tests/conformance/jbig1_oracle.json", "e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a"),
}
TOOL_PINS = {
    "git": "356db14e102d68a1a37d8a1ac577dfd678d45d46e92f468bef8b7154e7bfdc60",
    "qpdf": "30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792",
    "mutool": "b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7",
    "pdfinfo": "a1a371340d7b76e7d501da9136cc9256dfdb9cbdf09300520d7a3b4465343e67",
    "pdfimages": "213eba4a36ef021f49a0abc94292a7566baba8dfafef5017467166d9f06074f5",
    "pdftoppm": "f22d753dfb4c31c9f0198d608982dac5b06a3c5d9a08d7d9bf0c6004f08a1a56",
    "python": "889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9",
}
VERSIONS = {"git": "git version 2.47.3", "qpdf": "qpdf version 12.2.0",
            "mutool": "mutool version 1.25.1", "pdfinfo": "pdfinfo version 25.03.0",
            "pdfimages": "pdfimages version 25.03.0", "pdftoppm": "pdftoppm version 25.03.0"}
BASELINE_PINS = {
    "hn_a": (8239244, "833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40"),
    "c8": (2759609, "acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885"),
    "hn_b": (826645, "b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51"),
}
# External grayscale oracle: all nine objects/four raw streams were proven
# unchanged after only the two image ColorSpace corrections. Legacy repeats
# stay in the six-file audit set even when this explicit diagnostic is chosen.
CORRECTED_HNB_PIN = (826724, "2d423e1262142030b9b042a54735edc1776b132ae2fc54a3cbfc4b5f4d6f10fd")
# The independent 72/300 dimension revision is bound to one source/binary
# pair. Its external full-page plan must be frozen before private execution.
RATIONAL_NATIVE_PIN = ("e76f557009fea368714fc5866b996dc13b660df52a44598c640d89e0a844cb90",
                       "aa85136b67cc450957d610476c97aab76eb86b6c9a4730f7e261cdbbc5a9cb53")
PROTOCOL_PINS = {
    "protocol": ("docs/research/hnc8-page-composition-protocol.md", "6b423115b903521ffc7d99f2ce592c45e1933649e941afd485fe6a51aa292c93"),
    "dictionary_probe_protocol": ("docs/research/hnc8-page-composition-dictionary-probe.md", "abb348a40495a5ff12e1f38f8dc7f568f3596dc49f9041c70e313a40398f08fb"),
    "identity_params_protocol": ("docs/research/hnc8-page-composition-identity-params-rerun.md", "7cfe8b87430f94bb3cc291e88739d1214ad6e69ec7390c623d8941251539a133"),
}
PRESERVED_PINS = {
    "first_failed_report": (Path("/home/hzc/.cache/caj2pdf-issue117-validation/composition-report.json"), 733091,
                            "7e9d0e6d43d1f0f3e428da636f82566f7fcac44e4868347093edd652735816e5"),
    "dictionary_probe_report": (Path("/home/hzc/.cache/caj2pdf-issue117-dictionary-probe/dictionary-srrp1_nc/probe-report.json"), 21470,
                                "ec6a38adda35e156ad5b67857471414a1208953c9aa8c7d3d50e0675ca513fa4"),
}
EXPECTED = {"source_rows": 81, "output_pages": 77, "draws": 127,
            "type0_arrays": 74, "jpeg_streams": 53, "jpeg_color_spaces": 53,
            "page_renderer_pairs": 154}
REQUIRED = {"corpus", "reference_report", "table", "artifact_root", "native_tool"}
_UINT = re.compile(r"^(?:0|[1-9][0-9]*)$")
INVERSE_BITS = bytes.maketrans(bytes(range(256)), bytes(255-x for x in range(256)))
WHITE_RUNS = re.compile(b"\xff{3,}")


class CompositionError(Exception):
    """Requested validation, provenance, resource or protocol failure."""


class CompositionUnsupported(CompositionError):
    """A requested validator profile is unsupported; required work still fails."""


def _unsupported(error: BaseException) -> bool:
    return isinstance(error, (CompositionUnsupported, pdf.PdfMetadataUnsupported))


def _integer(value: str | int, label: str, minimum: int = 0,
             maximum: int = (1 << 64) - 1) -> int:
    if isinstance(value, bool) or (not isinstance(value, int) and
                                  (not isinstance(value, str) or not _UINT.fullmatch(value))):
        raise CompositionError(f"{label} is not a canonical unsigned integer")
    result = int(value)
    if not minimum <= result <= maximum:
        raise CompositionError(f"{label} exceeds its declared range")
    return result


def _numbers(values: list[str], label: str) -> list[float]:
    try:
        result = [float(value) for value in values]
    except ValueError as exc:
        raise CompositionError(f"{label} contains a nonnumeric component") from exc
    if any(not math.isfinite(value) or abs(value) > 2147483647 for value in result):
        raise CompositionError(f"{label} contains an unsupported component")
    return result


def _sha(value: str) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise CompositionError("an execution SHA-256 pin is malformed")
    return value


def file_identity(path: Path, limit: int, *, consume: Callable[[bytes], None] | None = None) -> dict:
    """Hash a stable regular file with a fixed 64 KiB read request."""
    path = path.resolve(strict=True)
    before = path.stat()
    if not path.is_file() or before.st_size > limit:
        raise CompositionError("requested input is absent, nonregular or over its byte cap")
    digest = hashlib.sha256()
    count = 0
    with path.open("rb") as stream:
        while chunk := stream.read(CHUNK):
            count += len(chunk)
            if count > limit:
                raise CompositionError("requested input grew beyond its byte cap")
            digest.update(chunk)
            if consume is not None:
                consume(chunk)
    after = path.stat()
    if (count != before.st_size or
            (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) !=
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)):
        raise CompositionError("requested input changed while being hashed")
    return {"path": str(path), "size_bytes": count, "sha256": digest.hexdigest()}


def _pinned_json(path: Path, expected: str) -> dict:
    data = bytearray()
    identity = file_identity(path, MIB, consume=data.extend)
    if identity["sha256"] != expected:
        raise CompositionError("pinned metadata digest differs")
    try:
        result = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise CompositionError("pinned metadata JSON is malformed") from exc
    if not isinstance(result, dict):
        raise CompositionError("pinned metadata root is not an object")
    return result


def _protocol_inputs() -> dict:
    inputs = {name: file_identity(ROOT/relative, MIB) for name, (relative, _) in PROTOCOL_PINS.items()}
    if any(inputs[name]["sha256"] != pin for name, (_, pin) in PROTOCOL_PINS.items()):
        raise CompositionError("frozen composition protocol or amendment digest differs")
    return inputs


def _preserved_inputs() -> dict:
    inputs = {name: file_identity(path, MIB) for name, (path, _, _) in PRESERVED_PINS.items()}
    if any(inputs[name]["size_bytes"] != size or inputs[name]["sha256"] != pin
           for name, (_, size, pin) in PRESERVED_PINS.items()):
        raise CompositionError("preserved first-failure or dictionary-probe metadata identity differs")
    return inputs


def _corrected_hnb_identity(path: Path) -> dict:
    identity = file_identity(path, PDF_LIMIT)
    if (identity["size_bytes"], identity["sha256"]) != CORRECTED_HNB_PIN:
        raise CompositionError("corrected HN-B reference differs from its separate complete identity pin")
    return identity


def _corrected_hnb_case(case: dict) -> dict:
    corrected = copy.deepcopy(case)
    if (corrected.get("source_variant") != "HN-B" or
            corrected.get("output_page_to_source_page") != [1, 6] or len(corrected.get("pdf_pages", [])) != 2):
        raise CompositionError("corrected HN-B metadata basis differs from the observed two-page profile")
    for page in corrected["pdf_pages"]:
        draws = page.get("draws", [])
        if len(draws) != 1 or draws[0].get("color_space") != "DeviceRGB":
            raise CompositionError("corrected HN-B metadata basis is not the sole two-color-declaration change")
        draws[0]["color_space"] = "DeviceGray"
    return corrected


def _require_original_native(native_sha: str, source_sha: str) -> None:
    path, _, pin = PRESERVED_PINS["first_failed_report"]
    report = _pinned_json(path, pin)
    audit = report.get("native_audit", {})
    if (report.get("status") != "FAIL" or audit.get("status") != "PASS" or audit.get("after_status") != "PASS"):
        raise CompositionError("preserved first-attempt provenance audit differs")
    original = (audit.get("binary", {}).get("sha256"), audit.get("source", {}).get("sha256"))
    if (native_sha, source_sha) not in (original, RATIONAL_NATIVE_PIN):
        raise CompositionError("native/source pair is outside the declared composition revisions")


def _environment(session: Path) -> dict[str, str]:
    return dict(os.environ, LC_ALL="C", TZ="UTC", PYTHONHASHSEED="0",
                PYTHONDONTWRITEBYTECODE="1", PYTHONNOUSERSITE="1",
                TMPDIR=str(session / "scratch"))


def _environment_identity(environment: Mapping[str, str]) -> dict:
    encoded = json.dumps(dict(environment), sort_keys=True, ensure_ascii=True,
                         separators=(",", ":")).encode("ascii")
    return {"sha256": hashlib.sha256(encoded).hexdigest(),
            "variable_names": sorted(environment),
            "overrides": {key: environment[key] for key in (
                "LC_ALL", "TZ", "PYTHONHASHSEED", "PYTHONDONTWRITEBYTECODE",
                "PYTHONNOUSERSITE", "TMPDIR")}}


def _memory_kib(pid: int) -> int | None:
    try:
        for line in Path(f"/proc/{pid}/status").read_bytes().splitlines():
            if line.startswith(b"VmHWM:"):
                return int(line.split()[1])
    except (OSError, ValueError):
        pass
    return None


class Commands:
    """Bounded serial process controller, including failed launch accounting."""

    def __init__(self, session: Path, report: dict):
        self.session = session
        self.report = report
        self.environment = _environment(session)
        self.deadline = time.monotonic() + TIME_LIMIT
        self.kind = "validator"
        self.context: dict = {}
        self.directory_caps: dict[Path, tuple[int, int]] = {}

    def disk(self, reserve: int = 0) -> int:
        total = 0
        directories = {directory: 0 for directory in self.directory_caps}
        for path in self.session.rglob("*"):
            if path.is_symlink():
                raise CompositionError("owned artifact session contains a symlink")
            if path.is_file():
                size = path.stat().st_size
                total += size
                for directory, (file_cap, _) in self.directory_caps.items():
                    if path.is_relative_to(directory):
                        directories[directory] += size
                        if size > file_cap:
                            raise CompositionError("extracted file exceeds its monitored byte cap")
        for directory, size in directories.items():
            if size > self.directory_caps[directory][1]:
                raise CompositionError("page extraction exceeds its monitored directory cap")
        measured = total + reserve
        resources = self.report["resources"]
        resources["max_observed_session_bytes"] = max(resources["max_observed_session_bytes"], measured)
        if measured > DISK_LIMIT:
            raise CompositionError("owned artifact session exceeds its disk cap")
        return total

    @contextmanager
    def extraction_caps(self, directory: Path):
        self.directory_caps[directory] = (RASTER_LIMIT, 128*MIB)
        try:
            yield
            self.disk()
        finally:
            self.directory_caps.pop(directory)

    def free(self) -> None:
        available = os.statvfs(self.session)
        if available.f_bavail * available.f_frsize < FREE_BYTES:
            raise CompositionError("artifact filesystem has less than the declared free-space reserve")

    def child_resources(self, process: subprocess.Popen, attempt: dict, rss_cap: int) -> None:
        peak = _memory_kib(process.pid)
        if peak is not None:
            attempt["sampled_peak_rss_kib"] = max(attempt["sampled_peak_rss_kib"] or 0, peak)
            if peak > rss_cap:
                raise CompositionError("requested child exceeded its RSS ceiling")
        self.disk(2*MIB if self.kind == "native" else 0)

    def run(self, arguments: list[str], label: str, limits: pdf.PdfMetadataLimits,
            usage: pdf._Usage, max_stdout: int, *, consume: Callable[[bytes], None] | None = None,
            digest_only: bool = False, include_stderr: bool = False) -> tuple[bytes | str, int]:
        if (consume is not None and digest_only) or (include_stderr and (consume or digest_only)):
            raise ValueError("choose one output consumer")
        if time.monotonic() >= self.deadline:
            raise CompositionError("whole experiment deadline exceeded")
        self.free()
        counts = self.report["counts"]
        if counts["native_launches"]+counts["validator_launches"] >= TOOL_LIMIT:
            raise CompositionError("combined native/validator launch ceiling exceeded")
        counter = "native_launches" if self.kind == "native" else "validator_launches"
        cap = NATIVE_LAUNCH_LIMIT if self.kind == "native" else TOOL_LIMIT
        if counts[counter] >= cap:
            raise CompositionError("declared process-launch ceiling exceeded")
        if self.kind == "render" and counts["render_launches"] >= RENDER_LAUNCH_LIMIT:
            raise CompositionError("declared page-render ceiling exceeded")
        counts[counter] += 1
        if self.kind == "render":
            counts["render_launches"] += 1
        attempt = {"number": len(self.report["attempts"]) + 1, "kind": self.kind,
                   "label": label, "command": arguments, **self.context,
                   "status": "ATTEMPTED", "exit_code": None, "timed_out": False,
                   "stdout_bytes": 0, "stderr_bytes": 0, "peak_rss_kib": None,
                   "sampled_peak_rss_kib": None}
        self.report["attempts"].append(attempt)
        started = time.monotonic()
        attempt["start_monotonic_seconds"] = started
        output = bytearray()
        diagnostics = bytearray()
        digest = hashlib.sha256()
        stderr_digest = hashlib.sha256()
        process = None
        child_usage = None
        error = None
        rss_cap = (128 if self.kind == "native" else 512) * 1024
        timeout = min(limits.timeout_seconds, self.deadline - started)
        deadline = started + timeout
        file_cap = min((caps[0] for caps in self.directory_caps.values()), default=PDF_LIMIT)
        def child_limits():
            resource.setrlimit(resource.RLIMIT_AS, (limits.max_child_virtual_bytes,
                                                   limits.max_child_virtual_bytes))
            resource.setrlimit(resource.RLIMIT_FSIZE, (file_cap, file_cap))
        try:
            process = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, start_new_session=True,
                                       env=self.environment,
                                       preexec_fn=child_limits)
            with selectors.DefaultSelector() as selector:
                assert process.stdout is not None and process.stderr is not None
                selector.register(process.stdout, selectors.EVENT_READ, "stdout")
                selector.register(process.stderr, selectors.EVENT_READ, "stderr")
                while selector.get_map():
                    pdf._check_cancel(usage.cancelled)
                    if time.monotonic() >= deadline:
                        attempt["timed_out"] = True
                        raise CompositionError("requested child exceeded its timeout")
                    self.child_resources(process, attempt, rss_cap)
                    for key, _ in selector.select(min(0.02, max(0.0, deadline - time.monotonic()))):
                        chunk = os.read(key.fileobj.fileno(), CHUNK)
                        if not chunk:
                            selector.unregister(key.fileobj)
                        elif key.data == "stderr":
                            attempt["stderr_bytes"] += len(chunk)
                            stderr_digest.update(chunk)
                            diagnostics.extend(chunk)
                            if len(diagnostics) > DIAGNOSTIC_LIMIT:
                                raise CompositionError("child diagnostic byte ceiling exceeded")
                        else:
                            attempt["stdout_bytes"] += len(chunk)
                            digest.update(chunk)
                            if attempt["stdout_bytes"] > max_stdout:
                                raise CompositionError("child stdout byte ceiling exceeded")
                            if consume is not None:
                                consume(chunk)
                            elif not digest_only:
                                output.extend(chunk)
            while True:
                pid, status, child_usage = os.wait4(process.pid, os.WNOHANG)
                if pid == process.pid:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() >= deadline:
                    attempt["timed_out"] = True
                    raise CompositionError("requested child did not exit before its timeout")
                pdf._check_cancel(usage.cancelled)
                self.child_resources(process, attempt, rss_cap)
                time.sleep(0.01)
            attempt["exit_code"] = process.returncode
            if process.returncode != 0:
                raise CompositionError("requested child exited unsuccessfully")
            if child_usage.ru_maxrss > rss_cap:
                raise CompositionError("reaped child peak RSS exceeded its ceiling")
            attempt["status"] = "PASS"
        except BaseException as exc:
            error = exc
            attempt["status"] = "FAIL"
            if process is not None:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if process.returncode is None:
                    try:
                        _, status, child_usage = os.wait4(process.pid, 0)
                        process.returncode = os.waitstatus_to_exitcode(status)
                        attempt["exit_code"] = process.returncode
                    except ChildProcessError:
                        pass
        finally:
            if process is not None:
                if process.stdout is not None:
                    process.stdout.close()
                if process.stderr is not None:
                    process.stderr.close()
            if child_usage is not None:
                attempt["peak_rss_kib"] = child_usage.ru_maxrss
                usage.max_child_rss_kib = max(usage.max_child_rss_kib, child_usage.ru_maxrss)
            usage.max_tool_output_bytes = max(usage.max_tool_output_bytes, attempt["stdout_bytes"])
            usage.total_tool_output_bytes += attempt["stdout_bytes"]
            attempt["end_monotonic_seconds"] = time.monotonic()
            attempt["elapsed_seconds"] = attempt["end_monotonic_seconds"] - started
            attempt["stdout_sha256"] = digest.hexdigest()
            attempt["stderr_sha256"] = stderr_digest.hexdigest()
            if error is not None:
                attempt["error_type"] = type(error).__name__
            if error is not None and diagnostics:
                path = self.session / f"failed-attempt-{attempt['number']}.stderr"
                path.write_bytes(diagnostics[:DIAGNOSTIC_LIMIT])
                attempt["retained_diagnostics"] = file_identity(path, DIAGNOSTIC_LIMIT)
            self.disk()
        if error is not None:
            raise error
        return (digest.hexdigest() if digest_only else
                bytes(output) + (bytes(diagnostics) if include_stderr else b"")), attempt["stdout_bytes"]

    @contextmanager
    def metadata_controller(self):
        """Reuse the existing parser with this invocation's audited controller.

        The optional CLI is serial; restore the module function on every exit.
        This avoids another copy of the PDF syntax/identity parser and exposes
        all of its subprocess attempts through the same failure accounting.
        """
        previous = pdf._run
        pdf._run = self.run
        try:
            yield
        finally:
            pdf._run = previous


@dataclass(frozen=True)
class Pnm:
    magic: str
    width: int
    height: int
    offset: int
    row_bytes: int


def pnm_header(stream: BinaryIO, *, rgb: bool = False) -> Pnm:
    tokens: list[bytes] = []
    token = bytearray()
    comment = False
    consumed = 0
    while len(tokens) < (3 if tokens and tokens[0] == b"P4" else 4):
        byte = stream.read(1)
        consumed += len(byte)
        if not byte or consumed > 4096:
            raise CompositionError("PNM header is truncated or exceeds its cap")
        if comment:
            if byte == b"\n":
                comment = False
            continue
        if byte == b"#" and not token:
            comment = True
            continue
        if byte in b" \t\r\n":
            if token:
                tokens.append(bytes(token))
                token.clear()
        else:
            token.extend(byte)
            if len(token) > 32:
                raise CompositionError("PNM header token exceeds its cap")
    try:
        magic = tokens[0].decode("ascii")
        width = _integer(tokens[1].decode("ascii"), "PNM width", 1, 32768)
        height = _integer(tokens[2].decode("ascii"), "PNM height", 1, 32768)
    except (UnicodeDecodeError, IndexError) as exc:
        raise CompositionError("PNM header is malformed") from exc
    if magic not in ("P4", "P5", "P6") or (rgb and magic != "P6"):
        raise CompositionError("PNM pixel profile is unsupported")
    if magic != "P4" and tokens[3] != b"255":
        raise CompositionError("PNM samples are not 8-bit")
    row_bytes = (width + 7) // 8 if magic == "P4" else width * (3 if magic == "P6" else 1)
    if row_bytes * height > RASTER_LIMIT:
        raise CompositionError("PNM pixel payload exceeds its cap")
    return Pnm(magic, width, height, stream.tell(), row_bytes)


def _raster(path: Path, width: int | None = None, height: int | None = None, *, rgb: bool = False) -> Pnm:
    if path.stat().st_size > RASTER_LIMIT:
        raise CompositionError("raster file exceeds its cap")
    with path.open("rb") as stream:
        info = pnm_header(stream, rgb=rgb)
    if (((width is not None or height is not None) and (info.width, info.height) != (width, height)) or
            path.stat().st_size != info.offset + info.row_bytes * info.height):
        raise CompositionError("raster dimensions or exact payload length differ")
    return info


def compare_pixels(baseline: Path, candidate: Path, width: int, height: int) -> dict:
    infos = [_raster(path, width, height, rgb=True) for path in (baseline, candidate)]
    hashes = [hashlib.sha256(), hashlib.sha256()]
    changed = channels = absolute = worst = 0
    nonwhite = [0, 0]
    with baseline.open("rb") as first, candidate.open("rb") as second:
        first.seek(infos[0].offset)
        second.seek(infos[1].offset)
        remaining = width * height * 3
        # Multiples of three retain complete RGB samples at chunk boundaries.
        while remaining:
            count = min(CHUNK - CHUNK % 3, remaining)
            left, right = first.read(count), second.read(count)
            if len(left) != count or len(right) != count:
                raise CompositionError("raster shortened during full-page comparison")
            hashes[0].update(left)
            hashes[1].update(right)
            # C-level comparison still covers every channel. Count whole white
            # RGB samples from aligned runs without allocating a pixel list.
            left_nonwhite = _nonwhite_count(left)
            nonwhite[0] += left_nonwhite
            if left == right:
                nonwhite[1] += left_nonwhite
                remaining -= count
                continue
            nonwhite[1] += _nonwhite_count(right)
            for offset in range(0, count, 3):
                a, b = left[offset:offset+3], right[offset:offset+3]
                changed += int(a != b)
                for x, y in zip(a, b):
                    difference = abs(x-y)
                    channels += int(difference != 0)
                    absolute += difference
                    worst = max(worst, difference)
            remaining -= count
    return {"status": "PASS" if channels == 0 else "FAIL", "width": width, "height": height,
            "compared_pixels": width*height, "compared_channels": width*height*3,
            "baseline_sha256": hashes[0].hexdigest(), "candidate_sha256": hashes[1].hexdigest(),
            "changed_pixels": changed, "changed_channels": channels,
            "maximum_channel_difference": worst, "absolute_difference_sum": absolute,
            "mean_absolute_channel_difference": absolute/(width*height*3),
            "baseline_nonwhite_pixels": nonwhite[0], "candidate_nonwhite_pixels": nonwhite[1]}


def compare_page_pixels(baseline: Path, candidate: Path, media_box: list) -> dict:
    """Compare every pixel of the observed zero-origin 300-DPI page profile.

    Original controls demonstrate an N or N+1 canvas axis at integral device
    boundaries. Discover the bounded renderer canvas; require both outputs to
    have the same complete grid. This grants no pixel tolerance or cropping.
    """
    if (not isinstance(media_box, (list, tuple)) or len(media_box) != 4 or
            any(type(value) not in (int, float) or abs(value) > 14400 or not math.isfinite(value) for value in media_box) or
            media_box[:2] not in ([0, 0], (0, 0)) or any(value < 1.0 for value in media_box[2:])):
        raise CompositionUnsupported("page raster geometry is outside the zero-origin diagnostic profile")
    nominal = [round(value*300/72) for value in media_box[2:]]
    if any(not 1 <= axis <= 32768 or abs(value*300/72-axis) > 1e-8
           for value, axis in zip(media_box[2:], nominal)):
        raise CompositionUnsupported("page raster geometry is outside the integral 300-DPI profile")
    info = _raster(baseline, rgb=True)
    if any(actual not in (axis, axis+1) for actual, axis in zip((info.width, info.height), nominal)):
        raise CompositionError("complete page raster is outside the observed canvas boundary")
    comparison = compare_pixels(baseline, candidate, info.width, info.height)
    comparison["nominal_grid"] = nominal
    comparison["canvas_policy"] = "complete equal grids; observed integral axis N or N+1; zero pixel tolerance"
    return comparison


def _nonwhite_count(data: bytes) -> int:
    if len(data) % 3:
        raise CompositionError("RGB statistic chunk is not sample-aligned")
    white = sum(match.end()//3 - (match.start()+2)//3 for match in WHITE_RUNS.finditer(data))
    return len(data)//3-white


def canonical_binary(path: Path, output: Path, width: int, height: int) -> dict:
    """Pack every proven binary display sample, including all padding columns."""
    if width % 32:
        raise CompositionError("Type0 padded width is not a complete DIB stride")
    info = _raster(path, width, height)
    digest = hashlib.sha256()
    with path.open("rb") as source, output.open("xb") as sink:
        source.seek(info.offset)
        for _ in range(height):
            if info.magic == "P4":
                packed = source.read(info.row_bytes)
                if len(packed) != info.row_bytes:
                    raise CompositionError("binary image shortened during canonicalization")
            else:
                packed = bytearray(width//8)
                components = 3 if info.magic == "P6" else 1
                done = 0
                while done < width:
                    count = min(CHUNK//components, width-done)
                    stripe = source.read(count*components)
                    if len(stripe) != count*components:
                        raise CompositionError("binary image shortened during canonicalization")
                    for index in range(count):
                        sample = stripe[index*components:(index+1)*components]
                        if sample not in (b"\x00"*components, b"\xff"*components):
                            raise CompositionError("extracted Type0 sample is not proven binary black/white")
                        if sample[0] == 0:
                            x = done+index
                            packed[x//8] |= 0x80 >> (x%8)
                    done += count
            digest.update(packed)
            sink.write(packed)
    return {"sha256": digest.hexdigest(), "size_bytes": width//8*height,
            "width": width, "height": height, "normalization": "proven binary black=1"}


def compare_samples(baseline: Path, candidate: Path, width: int, height: int) -> dict:
    length, stride = width//8*height, width//8
    if width % 32 or any(path.stat().st_size != length for path in (baseline, candidate)):
        raise CompositionError("full padded sample array length differs")
    hashes = [hashlib.sha256(), hashlib.sha256()]
    reverse = hashlib.sha256()
    changed_bytes = changed_bits = 0
    with baseline.open("rb") as left, candidate.open("rb") as right:
        for _ in range(height):
            a, b = left.read(stride), right.read(stride)
            if len(a) != stride or len(b) != stride:
                raise CompositionError("sample array shortened during comparison")
            hashes[0].update(a)
            hashes[1].update(b)
            if a != b:
                changed_bytes += sum(x != y for x, y in zip(a, b))
                changed_bits += sum((x ^ y).bit_count() for x, y in zip(a, b))
        for row in range(height-1, -1, -1):
            right.seek(row*stride)
            reverse.update(right.read(stride))
    return {"status": "PASS" if changed_bits == 0 else "FAIL", "compared_bytes": length,
            "compared_bits": length*8, "changed_bytes": changed_bytes, "changed_bits": changed_bits,
            "baseline_sha256": hashes[0].hexdigest(), "candidate_sha256": hashes[1].hexdigest(),
            "candidate_reverse_rows_sha256": reverse.hexdigest(),
            "reverse_only_match": bool(changed_bits and hashes[0].digest() == reverse.digest())}


def parse_native(data: bytes, source_size: int) -> dict:
    if len(data) > STDOUT_LIMIT:
        raise CompositionError("native output exceeds its cap")
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeDecodeError as exc:
        raise CompositionError("native metadata is not ASCII") from exc
    pages = []
    summary = None
    previous_row = None
    output_pages = 0
    for line in lines:
        if not line or len(line) > 2048:
            raise CompositionError("native line is empty or over its cap")
        parts = line.split("\t")
        if summary is not None:
            raise CompositionError("native summary is not the final record")
        if parts[0] == "P" and len(parts) == 11:
            if pages and len(pages[-1]["images"]) != pages[-1]["image_count"]:
                raise CompositionError("native page image records are incomplete")
            number = _integer(parts[1], "source page", 1, 4096)
            row = _integer(parts[2], "source row", 0, max(0, source_size-20))
            offset = _integer(parts[3], "source text offset", 0, source_size)
            length = _integer(parts[4], "source text length", 0, MIB)
            count = _integer(parts[5], "source image count", 0, 256)
            output = _integer(parts[6], "output page", 0, 4096)
            if number != len(pages)+1 or (previous_row is not None and row != previous_row+20) or length > source_size-offset:
                raise CompositionError("native source row ordering/span differs")
            box = _numbers(parts[7:], "native page box")
            if output:
                output_pages += 1
                if output != output_pages or count == 0 or box[:2] != [0.0, 0.0] or min(box[2:]) <= 0:
                    raise CompositionError("native output mapping or page box differs")
            elif count or box != [0.0]*4:
                raise CompositionError("native no-image row has output geometry")
            pages.append({"source_page": number, "row_offset": row, "text_offset": offset,
                          "text_length": length, "image_count": count, "output_page": output or None,
                          "media_box": box if output else None, "images": []})
            previous_row = row
        elif parts[0] == "I" and len(parts) == 16 and pages:
            values = [_integer(value, "native image field") for value in parts[1:10]]
            page, number, kind, descriptor, offset, length, visible, display, height = values
            current = pages[-1]
            if (page != current["source_page"] or number != len(current["images"])+1 or
                    number > current["image_count"] or kind not in (0, 2) or
                    descriptor > source_size-12 or length == 0 or offset > source_size or
                    length > min(64*MIB, source_size-offset) or not 0 < visible <= display <= 32768 or
                    not 0 < height <= 32768 or (kind == 0 and display != (visible+31)//32*32) or
                    (kind == 2 and display != visible)):
                raise CompositionError("native ordered image metadata is invalid")
            current["images"].append({"page_number": page, "image_number": number,
                                      "record_type": kind, "descriptor_offset": descriptor,
                                      "payload_offset": offset, "payload_length": length,
                                      "width": visible, "display_width": display, "height": height,
                                      "pdf_ctm": _numbers(parts[10:], "native image CTM")})
        elif parts[0] == "R" and len(parts) == 19:
            if parts[1] not in ("HN-A", "C8", "HN-B") or not pages:
                raise CompositionError("native variant/empty summary is unsupported")
            values = [_integer(value, "native resource summary") for value in parts[2:]]
            keys = ["source_pages", "output_pages", "no_image_pages", "type0_images", "jpeg_images",
                    "peak_page_metadata_bytes", "peak_text_working_bytes", "peak_row_store_bytes",
                    "row_store_read_bytes", "row_store_written_bytes", "source_read_bytes", "max_source_request_bytes",
                    "output_bytes", "max_sink_request_bytes", "scratch_read_bytes", "scratch_written_bytes",
                    "max_scratch_request_bytes"]
            summary = {"variant": parts[1], **dict(zip(keys, values))}
        else:
            raise CompositionError("native line protocol is unsupported or out of order")
    if not summary or len(pages[-1]["images"]) != pages[-1]["image_count"]:
        raise CompositionError("native metadata is incomplete")
    images = [image for page in pages for image in page["images"]]
    if (summary["source_pages"] != len(pages) or summary["output_pages"] != output_pages or
            summary["no_image_pages"] != sum(page["output_page"] is None for page in pages) or
            summary["type0_images"] != sum(image["record_type"] == 0 for image in images) or
            summary["jpeg_images"] != sum(image["record_type"] == 2 for image in images) or
            summary["row_store_read_bytes"] != summary["scratch_read_bytes"] or
            summary["row_store_written_bytes"] != summary["scratch_written_bytes"]):
        raise CompositionError("native summary disagrees with its rows/physical scratch counters")
    for key, cap in (("peak_page_metadata_bytes", 65536), ("peak_text_working_bytes", MIB),
                     ("peak_row_store_bytes", 2*MIB), ("max_source_request_bytes", 4096),
                     ("max_sink_request_bytes", 4096), ("max_scratch_request_bytes", 4096),
                     ("output_bytes", PDF_LIMIT)):
        if summary[key] > cap:
            raise CompositionError("native resource summary exceeds a frozen ceiling")
    if not summary["source_read_bytes"] or not summary["max_source_request_bytes"] or not summary["output_bytes"]:
        raise CompositionError("native resource summary lacks completed I/O")
    return {"variant": summary["variant"], "pages": pages, "resources": summary}


def _same_numbers(left: list[float], right: list[float], label: str) -> float:
    if len(left) != len(right) or any(not math.isfinite(x) for x in (*left, *right)):
        raise CompositionError(f"{label} has invalid components")
    difference = max(abs(x-y) for x, y in zip(left, right))
    if difference > TOLERANCE:
        raise CompositionError(f"{label} exceeds the frozen absolute CTM/box tolerance")
    return difference


@contextmanager
def _comparison(counts: dict | None, kind: str, progress: list | None = None, **location):
    if counts is not None:
        counts[f"{kind}_attempted"] += 1
    entry = {"kind": kind, **location, "status": "ATTEMPTED"}
    if progress is not None:
        progress.append(entry)
    try:
        yield
    except (Exception, KeyboardInterrupt) as exc:
        entry["status"] = "FAIL"
        entry["unsupported"] = _unsupported(exc)
        if counts is not None:
            counts[f"{kind}_failing"] += 1
            if _unsupported(exc):
                counts[f"{kind}_unsupported"] += 1
        raise
    else:
        entry["status"] = "PASS"
        if counts is not None:
            counts[f"{kind}_passing"] += 1


def compare_metadata(native: dict, baseline: dict, candidate: dict, case: dict,
                     counts: dict | None = None, progress: list | None = None) -> dict:
    source = case["source_pages"]
    if native["variant"] != case["source_variant"] or len(native["pages"]) != len(source):
        raise CompositionError("native source profile/row count differs from pinned observations")
    mapping = [page["source_page"] for page in native["pages"] if page["output_page"] is not None]
    if mapping != case["output_page_to_source_page"]:
        raise CompositionError("native source-to-output mapping differs")
    if any(len(document["pages"]) != len(case["pdf_pages"]) for document in (baseline, candidate)):
        raise CompositionError("independently reopened PDF page count differs")
    if native["variant"] == "HN-B" and ([page["image_count"] for page in native["pages"]] != [1,0,0,0,0,1] or mapping != [1,6]):
        raise CompositionError("HN-B six-row/two-JPEG mapping differs")
    max_residual = 0.0
    for actual, expected in zip(native["pages"], source):
        with _comparison(counts, "source_rows", progress, page=expected["page_number"]):
            if (actual["source_page"] != expected["page_number"] or
                    actual["text_offset"] != expected["text_offset"] or
                    actual["text_length"] != expected["text_length"] or
                    actual["image_count"] != len(expected["images"]) or
                    len(actual["images"]) != len(expected["images"])):
                raise CompositionError("native source page span/ordered count differs")
            for image, observed in zip(actual["images"], expected["images"]):
                keys = ("image_number", "record_type", "descriptor_offset", "payload_offset", "payload_length", "width", "height")
                if any(image[key] != observed[key] for key in keys) or image["display_width"] != observed.get("stride_width", observed["width"]):
                    raise CompositionError("native source image identity or padded dimension differs")
        if actual["output_page"] is None:
            continue
        index = actual["output_page"]-1
        reference, produced = baseline["pages"][index], candidate["pages"][index]
        public = case["pdf_pages"][index]
        with _comparison(counts, "output_pages", progress, page=index+1):
            for box in (actual["media_box"], produced["media_box"], public["media_box"]):
                max_residual = max(max_residual, _same_numbers(reference["media_box"], box, "page box"))
            if any(len(page["draws"]) != actual["image_count"] for page in (reference, produced, public)):
                raise CompositionError("PDF ordered draw count differs, including blank/missing draws")
        for image, original, emitted, pinned in zip(actual["images"], reference["draws"], produced["draws"], public["draws"]):
            with _comparison(counts, "draws", progress, page=index+1, image=image["image_number"]):
                for draw in (original, emitted, pinned):
                    if (draw["draw_number"] != image["image_number"] or
                            draw["width"] != image["display_width"] or draw["height"] != image["height"] or
                            draw["bits_per_component"] != (1 if image["record_type"] == 0 else 8)):
                        raise CompositionError("ordered PDF image dimension/bit depth differs")
                    max_residual = max(max_residual, _same_numbers(original["pdf_ctm"], draw["pdf_ctm"], "ordered draw CTM"))
                max_residual = max(max_residual, _same_numbers(original["pdf_ctm"], image["pdf_ctm"], "native source CTM"))
                if image["record_type"] == 0 and (original["raw_stream_sha256"] != pinned["raw_stream_sha256"] or original["raw_stream_length"] != pinned["raw_stream_length"]):
                    raise CompositionError("reference Type0 encoded stream changed from its public identity")
            if image["record_type"] == 2:
                with _comparison(counts, "jpeg_streams", progress, page=index+1, image=image["image_number"]):
                    observed = expected["images"][image["image_number"]-1]
                    if any(draw["filter"] != "/DCTDecode" or
                           draw["raw_stream_sha256"] != observed["payload_sha256"] or
                           draw["raw_stream_length"] != observed["payload_length"] for draw in (original, emitted, pinned)):
                        raise CompositionError("complete original JPEG stream identity differs")
                with _comparison(counts, "jpeg_color_spaces", progress, page=index+1, image=image["image_number"]):
                    colors = [draw.get("color_space") for draw in (original, emitted, pinned)]
                    if any(color not in ("DeviceGray", "DeviceRGB") for color in colors):
                        raise CompositionUnsupported("JPEG ColorSpace is outside the explicit Gray/RGB profile")
                    if colors[0] != colors[2]:
                        raise CompositionError("reference JPEG ColorSpace changed from its pinned metadata")
                    if colors[0] != colors[1]:
                        raise CompositionError("reference and native JPEG ColorSpace interpretations differ")
    return {"status": "PASS", "source_rows": len(source), "output_pages": len(mapping),
            "draws": sum(len(page["images"]) for page in source), "output_to_source": mapping,
            "no_image_source_rows": [page["source_page"] for page in native["pages"] if page["output_page"] is None],
            "max_absolute_ctm_or_box_residual_pt": max_residual}


def _sample_dictionary(data: bytes, width: int, height: int) -> bool:
    """Validate only the two explicitly understood binary sample dictionaries.

    Return whether bit inversion is needed to obtain black=1. Explicit Flate
    Predictor=1 with exact declared one-bit/one-color row metadata applies no
    prediction. Masks, indirect parameters and other predictors are refused.
    """
    if len(data) > 65536:
        raise CompositionError("Type0 dictionary exceeds its byte cap")
    if any(re.search(token+rb"\b", data) for token in (b"/SMask", b"/Mask", b"/ImageMask")):
        raise CompositionUnsupported("Type0 dictionary uses an unsupported sample interpretation")
    filters = re.findall(rb"/Filter\s+(/\w+|\[[^]]*\])", data)
    if re.search(rb"/Filter\b", data) and filters != [b"/FlateDecode"]:
        raise CompositionUnsupported("Type0 dictionary filter is unsupported")
    parameters = re.findall(rb"/DecodeParms\s*<<(.*?)>>", data, re.DOTALL)
    if re.search(rb"/DecodeParms\b", data):
        if filters != [b"/FlateDecode"] or len(re.findall(rb"/DecodeParms\b", data)) != 1 or len(parameters) != 1:
            raise CompositionUnsupported("Type0 prediction parameters require one direct Flate dictionary")
        fields = re.findall(rb"/(BitsPerComponent|Colors|Columns|Predictor)\s+((?:0|[1-9][0-9]{0,9}))\b", parameters[0])
        remainder = re.sub(rb"/(BitsPerComponent|Colors|Columns|Predictor)\s+(?:0|[1-9][0-9]{0,9})\b", b"", parameters[0])
        if (remainder.strip() or len(fields) != 4 or
                {key: int(value) for key, value in fields} !=
                {b"BitsPerComponent": 1, b"Colors": 1, b"Columns": width, b"Predictor": 1}):
            raise CompositionUnsupported("Type0 prediction parameters are outside the explicit no-prediction profile")
    # Parameter BPC describes Flate's row profile, not another image field.
    properties = re.sub(rb"/DecodeParms\s*<<.*?>>", b"", data, flags=re.DOTALL)
    for key, expected in ((b"Width", width), (b"Height", height), (b"BitsPerComponent", 1)):
        found = re.findall(rb"/" + key + rb"\s+(\d+)\b", properties)
        if len(found) != 1 or len(found[0]) > 10 or int(found[0]) != expected:
            raise CompositionError("Type0 dictionary sample dimensions/depth differ")
    if len(re.findall(rb"/Subtype\s+/Image\b", properties)) != 1:
        raise CompositionError("Type0 dictionary is not an image")
    decode = re.findall(rb"/Decode\s*\[\s*([01])\s+([01])\s*\]", data)
    if re.search(rb"/Decode\b", data) and not decode:
        raise CompositionUnsupported("Type0 dictionary decode array is unsupported")
    if len(decode) > 1:
        raise CompositionError("Type0 dictionary has duplicate decode arrays")
    if re.search(rb"/ColorSpace\s+/DeviceGray\b", data):
        if parameters or decode != [(b"1", b"0")]:
            raise CompositionUnsupported("native binary gray dictionary lacks the explicit inverse-gray profile")
        return False
    indexed = re.search(rb"/ColorSpace\s*\[\s*/Indexed\s+/DeviceRGB\s+1\s+"
                        rb"<([0-9A-Fa-f\s]+)>\s*\]", data)
    if not indexed or decode not in ([], [(b"0", b"1")]):
        raise CompositionUnsupported("reference binary palette/decode is unsupported")
    palette = re.sub(rb"\s", b"", indexed[1]).lower()
    if palette not in (b"ffffff000000", b"000000ffffff"):
        raise CompositionUnsupported("reference palette is not proven binary black/white")
    return palette == b"000000ffffff"


def _to_file(commands: Commands, arguments: list[str], label: str, path: Path,
             limit: int, *, timeout: float = 45, invert: bool = False) -> dict:
    with path.open("xb") as stream:
        def consume(chunk: bytes) -> None:
            stream.write(chunk.translate(INVERSE_BITS) if invert else chunk)
        commands.run(arguments, label, pdf.PdfMetadataLimits(timeout_seconds=timeout),
                     pdf._Usage(), limit, consume=consume)
    return file_identity(path, limit)


def _qpdf_samples(commands: Commands, pdf_path: Path, draw: dict, output: Path,
                  tools: dict) -> dict:
    object_id = str(draw["object_id"])
    data, _ = commands.run([str(tools["qpdf"]), f"--show-object={object_id}", str(pdf_path)],
                           "Type0 sample dictionary", pdf.PdfMetadataLimits(), pdf._Usage(), 65536)
    assert isinstance(data, bytes)
    invert = _sample_dictionary(data, draw["width"], draw["height"])
    identity = _to_file(commands, [str(tools["qpdf"]), f"--show-object={object_id}",
                                  "--filtered-stream-data", str(pdf_path)],
                        "complete padded Type0 samples", output, RASTER_LIMIT, invert=invert)
    if identity["size_bytes"] != draw["width"]//8*draw["height"]:
        raise CompositionError("qpdf decoded sample array has a wrong exact length")
    return identity


def _poppler_samples(commands: Commands, pdf_path: Path, page: int, draws: list[dict],
                     output: Path, tools: dict) -> dict[int, Path]:
    output.mkdir(mode=0o700)
    with commands.extraction_caps(output):
        return _extract_page_images(commands, pdf_path, page, draws, output, tools)


def _extract_page_images(commands: Commands, pdf_path: Path, page: int, draws: list[dict],
                         output: Path, tools: dict) -> dict[int, Path]:
    prefix = output / "image"
    commands.run([str(tools["pdfimages"]), "-f", str(page), "-l", str(page), "-j", str(pdf_path), str(prefix)],
                 "independent complete image extraction", pdf.PdfMetadataLimits(), pdf._Usage(), DIAGNOSTIC_LIMIT)
    files = sorted(output.glob("image-*"))
    if len(files) != len(draws):
        raise CompositionError("Poppler extracted file count differs from complete ordered draws")
    if sum(path.stat().st_size for path in files) > 128*MIB:
        raise CompositionError("one-page extraction directory exceeds its cap")
    result = {}
    for index, (path, draw) in enumerate(zip(files, draws)):
        expected_stem = f"image-{index:03}"
        if path.stem != expected_stem or path.is_symlink():
            raise CompositionError("Poppler image filenames do not preserve verified draw order")
        if draw["bits_per_component"] == 1:
            if path.suffix not in (".pbm", ".pgm", ".ppm"):
                raise CompositionError("Poppler Type0 sample wrapper is unsupported")
            canonical = output / f"canonical-{index}.bits"
            canonical_binary(path, canonical, draw["width"], draw["height"])
            result[draw["draw_number"]] = canonical
        else:
            identity = file_identity(path, 64*MIB)
            if (path.suffix not in (".jpg", ".jpeg") or
                    identity["sha256"] != draw["raw_stream_sha256"] or
                    identity["size_bytes"] != draw["raw_stream_length"]):
                raise CompositionError("Poppler original JPEG extraction identity differs")
        path.unlink()
    return result


def _remove_files(directory: Path) -> None:
    for path in sorted(directory.rglob("*"), reverse=True):
        if path.is_symlink():
            raise CompositionError("owned artifacts unexpectedly contain a symlink")
        if path.is_file():
            path.unlink()
        elif path.is_dir():
            path.rmdir()
    directory.rmdir()


def check_arrays(commands: Commands, baseline_path: Path, candidate_path: Path,
                 baseline: dict, candidate: dict, tools: dict, result: dict) -> None:
    for original, produced in zip(baseline["pages"], candidate["pages"]):
        type0 = [draw for draw in original["draws"] if draw["bits_per_component"] == 1]
        if not type0:
            continue
        page = original["page_number"]
        commands.context.update(page=page)
        directory = commands.session / f"samples-{result['profile']}-{page}"
        directory.mkdir(mode=0o700)
        extracted = None
        for draw in type0:
            commands.context.update(image=draw["draw_number"])
            with _comparison(commands.report["counts"], "type0_arrays", result.setdefault("progress", []),
                             page=page, image=draw["draw_number"]):
                if extracted is None:
                    extracted = [_poppler_samples(commands, path, page, document["draws"], directory / name, tools)
                                 for path, document, name in ((baseline_path, original, "baseline"),
                                                              (candidate_path, produced, "candidate"))]
                emitted = produced["draws"][draw["draw_number"]-1]
                first = directory / "reference.bits"
                second = directory / "native.bits"
                _qpdf_samples(commands, baseline_path, draw, first, tools)
                _qpdf_samples(commands, candidate_path, emitted, second, tools)
                comparison = compare_samples(first, second, draw["width"], draw["height"])
                comparison.update(page=page, image=draw["draw_number"])
                for path, canonical in zip((first, second), extracted):
                    independent = compare_samples(path, canonical[draw["draw_number"]], draw["width"], draw["height"])
                    if independent["status"] != "PASS":
                        raise CompositionError("qpdf/Poppler complete binary sample arrays differ")
                result["type0_arrays"].append(comparison)
                if comparison["status"] != "PASS":
                    raise CompositionError("complete padded Type0 samples differ")
                first.unlink()
                second.unlink()
        _remove_files(directory)
    commands.context.pop("image", None)


def check_pixels(commands: Commands, baseline: Path, candidate: Path, pages: list[dict],
                 tools: dict, result: dict) -> None:
    commands.kind = "render"
    for page in pages:
        number = page["page_number"]
        box = page["media_box"]
        for renderer in ("mutool", "pdftoppm"):
            commands.context.update(page=number, renderer=renderer)
            with _comparison(commands.report["counts"], "page_renderer_pairs", result.setdefault("progress", []),
                             page=number, renderer=renderer):
                pair = []
                identities = []
                for label, source in (("baseline", baseline), ("candidate", candidate)):
                    output = commands.session / f"{result['profile']}-{number}-{renderer}-{label}.ppm"
                    if renderer == "mutool":
                        arguments = [str(tools[renderer]), "draw", "-q", "-r", "300", "-A", "0",
                                     "-c", "rgb", "-F", "pnm", "-o", "-", str(source), str(number)]
                    else:
                        arguments = [str(tools[renderer]), "-r", "300", "-singlefile", "-aa", "no",
                                     "-aaVector", "no", "-f", str(number), "-l", str(number), str(source)]
                    identities.append(_to_file(commands, arguments, "complete page RGB pixels", output, RASTER_LIMIT, timeout=60))
                    pair.append(output)
                comparison = compare_page_pixels(*pair, box)
                comparison.update(page=number, renderer=renderer)
                comparison["whole_file_identities"] = {"baseline": identities[0], "candidate": identities[1]}
                result["page_pixels"].append(comparison)
                if (comparison["status"] != "PASS" or
                        any(identities[0][key] != identities[1][key] for key in ("sha256", "size_bytes"))):
                    comparison["status"] = "FAIL"
                    raise CompositionError("complete-page RGB samples differ at the frozen zero tolerance")
                for path in pair:
                    path.unlink()
    commands.context.pop("renderer", None)
    commands.kind = "validator"


def _report() -> dict:
    return {"schema_version": 1, "protocol": "hnc8-source-page-composition-v1", "status": "NOT_RUN",
            "scope": "three previously observed source documents; image-only caller-table diagnostic",
            "production_composition": "NOT_ENABLED", "tolerance_pt": TOLERANCE,
            "unsupported_count_scope": "subset of failing requested-profile comparisons: explicit sample-dictionary or metadata-validator refusals; never a passing compatibility check",
            "pixel_tolerance": {"maximum_channel_difference": 0, "changed_pixels": 0},
            "planned": {"profiles": 3, "source_audit_rows": 27, "baseline_pdf_files": 6,
                        "native_launches": 3, "render_launches": 308, "converter_launches": 0, **EXPECTED},
            "counts": {**{key: 0 for key in ("native_launches", "native_completed", "native_failed",
                                            "converter_launches", "validator_launches", "render_launches",
                                            "metadata_groups_attempted", "metadata_groups_passing", "metadata_groups_failing", "metadata_groups_skipped", "metadata_groups_unsupported",
                                            "profiles_attempted", "profiles_passing", "profiles_failing", "profiles_skipped", "profiles_unsupported",
                                            "source_checks_before", "source_checks_after", "baseline_checks_before", "baseline_checks_after",
                                            "corrected_reference_checks_before", "corrected_reference_checks_after")},
                       **{f"{kind}_{outcome}": 0 for kind in EXPECTED for outcome in ("attempted", "passing", "failing", "skipped", "unsupported")}},
            **{name: {"status": "NOT_RUN"} for name in ("source_audit", "baseline_audit", "table_audit",
                                                       "environment_audit", "input_audit", "native_audit")},
            "resources": {"read_request_limit_bytes": CHUNK, "native_request_limit_bytes": 4096,
                          "disk_limit_bytes": DISK_LIMIT, "raster_limit_bytes": RASTER_LIMIT,
                          "whole_timeout_seconds": TIME_LIMIT, "max_observed_session_bytes": 0,
                          "scope": "owned files and reserved native row store; checked allocations, named buffers and process RSS are distinct",
                          "harness_vmhwm_kib": None, "retained_artifact_bytes": 0},
            "profiles": [], "attempts": [], "errors": []}


def _audit(paths: dict, baselines: dict, rows: list[dict], tools: dict,
           environment: Mapping[str, str], native_sha: str, source_sha: str) -> dict:
    sources = []
    for row in rows:
        relative = Path(row["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise CompositionError("matrix source path is unsafe")
        path = (paths["corpus"] / relative).resolve(strict=True)
        if not path.is_relative_to(paths["corpus"]):
            raise CompositionError("matrix source escaped the corpus root")
        identity = file_identity(path, SOURCE_LIMIT)
        if identity["sha256"] != row["sha256"] or identity["size_bytes"] != row["size_bytes"]:
            raise CompositionError("original corpus source identity differs")
        sources.append({"id": row["id"], **identity})
    baseline = {name: file_identity(path, PDF_LIMIT) for name, path in baselines.items()}
    if set(baseline) != {f"{profile}-run{repeat}" for profile in BASELINE_PINS for repeat in (1, 2)}:
        raise CompositionError("baseline audit set differs from six frozen repeat identities")
    for name, identity in baseline.items():
        expected_size, expected_sha = BASELINE_PINS[name.rsplit("-run", 1)[0]]
        if identity["size_bytes"] != expected_size or identity["sha256"] != expected_sha:
            raise CompositionError("baseline PDF differs from its complete frozen hash/size pin")
    inputs = {name: file_identity(ROOT / entry[0], MIB) for name, entry in PUBLIC_PINS.items()}
    if any(inputs[name]["sha256"] != entry[1] for name, entry in PUBLIC_PINS.items()):
        raise CompositionError("public metadata pin changed before/during the experiment")
    inputs["reference_report"] = file_identity(paths["reference_report"], MIB)
    if inputs["reference_report"]["sha256"] != REFERENCE_SHA:
        raise CompositionError("reference-report pin changed before/during the experiment")
    inputs.update(_protocol_inputs())
    inputs.update(_preserved_inputs())
    if "corrected_hn_b" in paths:
        inputs["corrected_hn_b_reference"] = _corrected_hnb_identity(paths["corrected_hn_b"])
    for name in ("hnc8_page_composition.py", "hnc8_layout_pdf.py", "hnc8_placement_rule.py", "hnc8_layout_reference.py"):
        inputs[name] = file_identity(ROOT / "scripts" / name, MIB)
    inputs["synthetic_tests"] = file_identity(ROOT / "tests/conformance/test_hnc8_page_composition.py", MIB)
    table = file_identity(paths["table"], 16384)
    if table["sha256"] != TABLE_SHA:
        raise CompositionError("caller table digest differs")
    binaries = {name: file_identity(path, 256*MIB) for name, path in tools.items()}
    binaries["python"] = file_identity(Path(sys.executable), 256*MIB)
    for name, identity in binaries.items():
        if identity["sha256"] != TOOL_PINS[name]:
            raise CompositionError(f"required {name} executable digest differs")
    binary = file_identity(paths["native_tool"], 256*MIB)
    fingerprint = placement.source_fingerprint(ROOT)
    if binary["sha256"] != native_sha or fingerprint["sha256"] != source_sha:
        raise CompositionError("native executable or Rust/Cargo source pin differs")
    return {"source_audit": {"status": "PASS", "sources": sources},
            "baseline_audit": {"status": "PASS", "files": baseline},
            "table_audit": {"status": "PASS", **table},
            "input_audit": {"status": "PASS", "files": inputs},
            "environment_audit": {"status": "PASS", "binaries": binaries,
                                  "environment": _environment_identity(environment),
                                  "linked_runtime_scope": "executable hashes only; linked-library identities must be separately frozen in the external receipt"},
            "native_audit": {"status": "PASS", "binary": binary, "source": fingerprint}}


def _baselines(report: dict, oracle: dict) -> dict[str, Path]:
    if report.get("status") != "PASS" or len(report.get("generations", [])) != 3:
        raise CompositionError("reference report is not the pinned three-profile PASS")
    result = {}
    for generation in report["generations"]:
        profile = generation["profile"]
        case = oracle.get(profile)
        runs = generation.get("runs")
        if case is None or not isinstance(runs, list) or len(runs) != 2:
            raise CompositionError("reference repeat/profile set differs")
        for repeat, run in enumerate(runs, 1):
            if run.get("status") != "PASS" or run.get("pdf_sha256") != case["pdf_sha256"]:
                raise CompositionError("reference PDF repeat identity differs")
            path = Path(run["output_path"]).resolve(strict=True)
            if not path.is_relative_to(Path(report["artifact_session_path"]).resolve(strict=True)):
                raise CompositionError("baseline PDF escaped its report-owned session")
            identity = file_identity(path, PDF_LIMIT)
            if identity["sha256"] != case["pdf_sha256"] or identity["size_bytes"] != run["pdf_size_bytes"]:
                raise CompositionError("baseline PDF hash/size differs from its frozen report")
            result[f"{profile}-run{repeat}"] = path
    if len(result) != 6:
        raise CompositionError("baseline audit set is not six unique repeat records")
    return result


def _execution_receipt(paths: dict, tools: dict, oracle: dict, rows: list[dict],
                       commands: Commands, native_sha: str, source_sha: str) -> dict:
    """Freeze actual child derivation before any source/PDF/table byte audit."""
    binary = file_identity(paths["native_tool"], 256*MIB)
    fingerprint = placement.source_fingerprint(ROOT)
    if binary["sha256"] != native_sha or fingerprint["sha256"] != source_sha:
        raise CompositionError("internal execution receipt native/source pin differs")
    _protocol_inputs()
    files = {name: file_identity(ROOT/name, MIB) for name in (
        *(entry[0] for entry in PROTOCOL_PINS.values()), "scripts/hnc8_page_composition.py",
        "scripts/hnc8_layout_pdf.py", "scripts/hnc8_layout_reference.py",
        "scripts/hnc8_placement_rule.py", "tests/conformance/test_hnc8_page_composition.py")}
    selected = {row["id"]: row for row in rows}
    invocations = []
    for name in ("hn_a", "c8", "hn_b"):
        source = (paths["corpus"]/selected[oracle[name]["source_id"]]["path"]).resolve(strict=True)
        if not source.is_relative_to(paths["corpus"]):
            raise CompositionError("receipt source path escaped the corpus root")
        invocations.append({"profile": name, "command": [str(paths["native_tool"]), str(source),
                            str(commands.session/f"{name}.pdf"), str(paths["table"]),
                            str(commands.session/"scratch")],
                            "source_sha256": selected[oracle[name]["source_id"]]["sha256"]})
    receipt = {"schema_version": 1, "protocol": "hnc8-source-page-composition-v1",
               "phase": "BEFORE_PRIVATE_BYTE_AUDITS", "native": binary,
               "rust_cargo_source": fingerprint, "harness_files": files,
               "preserved_metadata_files": _preserved_inputs(),
               "tools": {name: file_identity(path, 256*MIB) for name, path in tools.items()},
               "effective_environment": _environment_identity(commands.environment),
               "roots": {name: str(paths[name]) for name in REQUIRED},
               "invocations": invocations, "table_declared_sha256": TABLE_SHA,
               "maximum_native_launches": NATIVE_LAUNCH_LIMIT,
               "maximum_converter_launches": 0, "maximum_render_launches": RENDER_LAUNCH_LIMIT}
    if "corrected_hn_b" in paths:
        receipt["corrected_hn_b_reference"] = {
            "path": str(paths["corrected_hn_b"]), "size_bytes": CORRECTED_HNB_PIN[0], "sha256": CORRECTED_HNB_PIN[1]}
        receipt["reference_policy"] = "HN-A/C8 legacy; HN-B separately pinned corrected Gray; legacy parity not claimed"
    encoded = json.dumps(receipt, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(encoded) > MIB:
        raise CompositionError("internal execution receipt exceeds its metadata cap")
    path = commands.session/"execution-receipt.json"
    with path.open("xb") as stream:
        stream.write(encoded)
    path.chmod(0o400)
    return file_identity(path, MIB)


def _verify_receipt(identity: dict) -> None:
    path = Path(identity["path"])
    if file_identity(path, MIB) != identity:
        raise CompositionError("immutable internal execution receipt changed")
    receipt = _pinned_json(path, identity["sha256"])
    for name, expected in receipt["harness_files"].items():
        if file_identity(ROOT/name, MIB) != expected:
            raise CompositionError("protocol, runner, helper or test changed from the internal execution receipt")
    for expected in receipt.get("preserved_metadata_files", {}).values():
        if file_identity(Path(expected["path"]), MIB) != expected:
            raise CompositionError("preserved failure/probe metadata changed from the internal execution receipt")
    if "corrected_hn_b_reference" in receipt:
        expected = receipt["corrected_hn_b_reference"]
        if _corrected_hnb_identity(Path(expected["path"])) != expected:
            raise CompositionError("corrected HN-B reference changed from its internal execution receipt")


def run(paths: Mapping[str, Path] | None = None, *, native_sha256: str | None = None,
        native_source_sha256: str | None = None) -> dict:
    report = _report()
    if paths is None and native_sha256 is None and native_source_sha256 is None:
        return report
    before = commands = session = resolved = baseline_paths = rows = tools = None
    try:
        if paths is None or not REQUIRED.issubset(paths) or set(paths)-REQUIRED-set(TOOL_PINS)-{"python", "corrected_hn_b"}:
            raise CompositionError("complete explicit external paths are required")
        expected_binary, expected_source = _sha(native_sha256), _sha(native_source_sha256)
        resolved = {name: Path(paths[name]).resolve(strict=True) for name in REQUIRED}
        if "corrected_hn_b" in paths:
            resolved["corrected_hn_b"] = Path(paths["corrected_hn_b"]).resolve(strict=True)
            if resolved["corrected_hn_b"].is_relative_to(ROOT):
                raise CompositionError("corrected HN-B reference must remain outside Git")
            report["reference_policy"] = "HN-A/C8 legacy; HN-B separately pinned corrected Gray; legacy parity not claimed"
        if not resolved["corpus"].is_dir() or not resolved["artifact_root"].is_dir():
            raise CompositionError("corpus/artifact roots must be existing directories")
        if resolved["artifact_root"].is_relative_to(ROOT) or resolved["artifact_root"].is_relative_to(Path("/tmp")):
            raise CompositionError("heavy artifacts must stay outside Git and /tmp")
        tools = {name: pdf._tool_path(paths.get(name, Path("/usr/bin") / name), name)
                 for name in TOOL_PINS if name != "python"}
        matrix = _pinned_json(ROOT / PUBLIC_PINS["matrix"][0], PUBLIC_PINS["matrix"][1])
        rows = [row for row in matrix["samples"] if row["detected_type"] in ("HN", "C8")]
        if len(rows) != 27 or len({row["id"] for row in rows}) != 27:
            raise CompositionError("original matrix source audit set differs")
        layout = _pinned_json(ROOT / PUBLIC_PINS["layout"][0], PUBLIC_PINS["layout"][1])
        oracle = {case["case"]: case for case in layout["cases"]}
        if set(oracle) != {"hn_a", "c8", "hn_b"}:
            raise CompositionError("layout oracle three-profile set differs")
        reference_report = _pinned_json(resolved["reference_report"], REFERENCE_SHA)
        session = Path(tempfile.mkdtemp(prefix="hnc8-composition-", dir=resolved["artifact_root"]))
        session.chmod(0o700)
        (session / "scratch").mkdir(mode=0o700)
        report["artifact_session_path"] = str(session)
        commands = Commands(session, report)
        report["internal_execution_receipt"] = _execution_receipt(
            resolved, tools, oracle, rows, commands, expected_binary, expected_source)
        _verify_receipt(report["internal_execution_receipt"])
        _require_original_native(expected_binary, expected_source)
        baseline_paths = _baselines(reference_report, oracle)
        before = _audit(resolved, baseline_paths, rows, tools, commands.environment,
                        expected_binary, expected_source)
        report.update(copy.deepcopy(before))
        _verify_receipt(report["internal_execution_receipt"])
        report["counts"]["source_checks_before"] = 27
        report["counts"]["baseline_checks_before"] = 6
        report["counts"]["corrected_reference_checks_before"] = int("corrected_hn_b" in resolved)
        versions = {}
        for name, path in tools.items():
            data, _ = commands.run([str(path), "--version" if name in ("qpdf", "git") else "-v"],
                                   f"{name} version", pdf.PdfMetadataLimits(), pdf._Usage(),
                                   DIAGNOSTIC_LIMIT, include_stderr=True)
            assert isinstance(data, bytes)
            text = data.decode("ascii")
            if text.splitlines()[0] != VERSIONS[name]:
                raise CompositionError(f"required {name} version output differs")
            versions[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "text": text}
        report["environment_audit"]["versions"] = versions
        limits = pdf.PdfMetadataLimits(max_draws_per_page=256)
        source_by_id = {row["id"]: row for row in rows}
        aggregate_pdf = 0
        for name in ("hn_a", "c8", "hn_b"):
            case = oracle[name]
            corrected_hn_b = name == "hn_b" and "corrected_hn_b" in resolved
            if corrected_hn_b:
                case = _corrected_hnb_case(case)
            source = (resolved["corpus"] / source_by_id[case["source_id"]]["path"]).resolve(strict=True)
            output = session / f"{name}.pdf"
            result = {"profile": name, "status": "ATTEMPTED", "source_id": case["source_id"],
                      "source_sha256": source_by_id[case["source_id"]]["sha256"],
                      "type0_arrays": [], "page_pixels": []}
            result["comparison_basis"] = "separately pinned corrected grayscale reference" if corrected_hn_b else "pinned legacy reference"
            report["profiles"].append(result)
            report["counts"]["profiles_attempted"] += 1
            commands.kind, commands.context = "native", {"profile": name}
            arguments = [str(resolved["native_tool"]), str(source), str(output),
                         str(resolved["table"]), str(session / "scratch")]
            result["command"] = arguments
            try:
                data, _ = commands.run(arguments, "original source-page native diagnostic",
                                       pdf.PdfMetadataLimits(timeout_seconds=180), pdf._Usage(), STDOUT_LIMIT)
                report["counts"]["native_completed"] += 1
                assert isinstance(data, bytes)
                result["native_output"] = {"size_bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                native = parse_native(data, source_by_id[case["source_id"]]["size_bytes"])
                result["native"] = native
                identity = file_identity(output, PDF_LIMIT)
                if identity["size_bytes"] != native["resources"]["output_bytes"] or any((session / "scratch").iterdir()):
                    raise CompositionError("native PDF byte counter/row-store cleanup differs")
                aggregate_pdf += identity["size_bytes"]
                if aggregate_pdf > 256*MIB:
                    raise CompositionError("aggregate native PDF output exceeds its cap")
                result["output_pdf"] = identity
                commands.kind = "validator"
                baseline_path = resolved["corrected_hn_b"] if corrected_hn_b else baseline_paths[f"{name}-run1"]
                with _comparison(report["counts"], "metadata_groups", result.setdefault("progress", [])):
                    with commands.metadata_controller():
                        metadata_tools = {key: tools[key] for key in ("qpdf", "mutool", "pdfimages")}
                        original = pdf.extract_pdf_metadata(baseline_path, metadata_tools, limits=limits)
                        produced = pdf.extract_pdf_metadata(output, metadata_tools, limits=limits,
                                                            allow_raw_bilevel=True)
                    result["baseline_metadata"] = original
                    result["candidate_metadata"] = produced
                    result["metadata"] = compare_metadata(native, original, produced, case,
                                                          report["counts"], result["progress"])
                check_arrays(commands, baseline_path, output, original, produced, tools, result)
                check_pixels(commands, baseline_path, output, original["pages"], tools, result)
                result["status"] = "PASS"
                report["counts"]["profiles_passing"] += 1
            except (Exception, KeyboardInterrupt) as exc:
                result["status"] = "FAIL"
                report["counts"]["profiles_failing"] += 1
                if _unsupported(exc):
                    result["unsupported"] = True
                    report["counts"]["profiles_unsupported"] += 1
                if report["counts"]["native_completed"] < report["counts"]["native_launches"]:
                    report["counts"]["native_failed"] += 1
                raise
        for kind, expected in EXPECTED.items():
            if (report["counts"][f"{kind}_passing"] != expected or
                    report["counts"][f"{kind}_attempted"] != expected or
                    report["counts"][f"{kind}_failing"] or report["counts"][f"{kind}_unsupported"]):
                raise CompositionError("complete required comparison count differs")
        if (report["counts"]["native_launches"] != 3 or report["counts"]["native_completed"] != 3 or
                report["counts"]["native_failed"] or report["counts"]["converter_launches"] or
                report["counts"]["render_launches"] != RENDER_LAUNCH_LIMIT or report["counts"]["profiles_passing"] != 3 or
                report["counts"]["profiles_failing"] or report["counts"]["profiles_unsupported"] or
                report["counts"]["metadata_groups_unsupported"] or report["counts"]["metadata_groups_failing"] or
                report["counts"]["metadata_groups_attempted"] != 3 or report["counts"]["metadata_groups_passing"] != 3):
            raise CompositionError("complete required launch/profile count differs")
        if time.monotonic() > commands.deadline:
            raise CompositionError("whole experiment deadline exceeded")
        report["status"] = "PASS"
    except (Exception, KeyboardInterrupt) as exc:
        report["status"] = "FAIL"
        report["errors"].append(str(exc) or type(exc).__name__)
    finally:
        if before is not None:
            try:
                after = _audit(resolved, baseline_paths, rows, tools, commands.environment,
                               expected_binary, expected_source)
                for name, audit in before.items():
                    # Versions are diagnostic additions on the report only.
                    if after[name] != audit:
                        report[name]["status"] = "FAIL"
                        report[name]["after_status"] = "FAIL"
                        raise CompositionError(f"{name} changed between complete before/after audits")
                    report[name]["after_status"] = "PASS"
                report["counts"]["source_checks_after"] = 27
                report["counts"]["baseline_checks_after"] = 6
                report["counts"]["corrected_reference_checks_after"] = int("corrected_hn_b" in resolved)
            except (Exception, KeyboardInterrupt) as exc:
                report["status"] = "FAIL"
                report["errors"].append(f"post-run audit: {exc}")
                for name in before:
                    if "after_status" not in report[name]:
                        report[name]["after_status"] = "NOT_COMPLETED"
                        report[name]["status"] = "PARTIAL"
        report["counts"]["profiles_skipped"] = max(0, 3-report["counts"]["profiles_attempted"])
        report["counts"]["metadata_groups_skipped"] = max(0, 3-report["counts"]["metadata_groups_attempted"])
        for kind, expected in EXPECTED.items():
            report["counts"][f"{kind}_skipped"] = max(0, expected-report["counts"][f"{kind}_attempted"])
        if commands is not None:
            try:
                if "internal_execution_receipt" in report:
                    _verify_receipt(report["internal_execution_receipt"])
                report["resources"]["retained_artifact_bytes"] = commands.disk()
                report["resources"]["harness_vmhwm_kib"] = pdf._self_vm_hwm_kib()
                report["resources"]["environment"] = _environment_identity(commands.environment)
                report["resources"]["elapsed_seconds"] = time.monotonic()-(commands.deadline-TIME_LIMIT)
                for kind in ("native", "validator"):
                    peaks = [attempt["peak_rss_kib"] for attempt in report["attempts"]
                             if (attempt["kind"] == "native") == (kind == "native") and attempt["peak_rss_kib"] is not None]
                    report["resources"][f"max_{kind}_rss_kib"] = max(peaks, default=None)
                    report["resources"][f"{kind}_rss_measured_children"] = len(peaks)
                report["resources"]["total_child_stdout_bytes"] = sum(attempt["stdout_bytes"] for attempt in report["attempts"])
                report["resources"]["full_pixel_compare_buffers_bytes"] = 2*CHUNK
                report["resources"]["binary_canonicalization_buffer_limit_bytes"] = CHUNK+32768//8
                if time.monotonic() > commands.deadline:
                    report["status"] = "FAIL"
                    report["errors"].append("whole experiment deadline exceeded, including required post-audits")
            except Exception as exc:
                report["status"] = "FAIL"
                report["errors"].append(f"final artifact accounting: {exc}")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    optional_paths = (set(TOOL_PINS)-{"python"}) | {"corrected_hn_b"}
    for name in sorted(REQUIRED | optional_paths):
        parser.add_argument("--"+name.replace("_", "-"), type=Path)
    parser.add_argument("--native-sha256")
    parser.add_argument("--native-source-sha256")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    supplied = {name: getattr(args, name) for name in REQUIRED | optional_paths
                if getattr(args, name) is not None}
    report = run(supplied or None, native_sha256=args.native_sha256,
                 native_source_sha256=args.native_source_sha256)
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        print(f"HN/C8 page composition [{report['status']}]: "
              f"{report['counts']['profiles_passing']}/3 profiles; "
              f"{report['counts']['page_renderer_pairs_passing']}/154 complete-page comparisons")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

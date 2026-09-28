#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bounded process/file primitives for the opt-in external CAJViewer canary.

No installer is downloaded and no vendor program is executed by default.
Vendor-containing containers and capture receipts must remain outside Git.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import stat
import subprocess
import time


INSTALLER_SHA256 = "3142c633d74dcf34ebaca9b7653f88ad3619f0b7a6cb689487b6cc583ec926d3"
INSTALLER_SIZE = 235087704
CHUNK_BYTES = 65536
MAX_CAPTURE_BYTES = 1600 * 1200 * 3 + 64


class CanaryError(Exception):
    """A failed canary precondition; it is never a compatibility pass."""


def _observe_regular(path: Path, *, expected_size=None, expected_sha256=None,
                     max_bytes=2 * 1024 ** 3, retain=False):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as file:
        before = os.fstat(file.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size > max_bytes:
            raise CanaryError("input must be a bounded regular file")
        if expected_size is not None and before.st_size != expected_size:
            raise CanaryError("input size mismatch")
        digest = hashlib.sha256()
        size = 0
        payload = bytearray() if retain else None
        while chunk := file.read(CHUNK_BYTES):
            size += len(chunk)
            if size > max_bytes:
                raise CanaryError("input grew past byte limit")
            digest.update(chunk)
            if payload is not None:
                payload.extend(chunk)
        after = os.fstat(file.fileno())
        identity = lambda value: (value.st_dev, value.st_ino, value.st_size,
                                  value.st_mtime_ns, value.st_ctime_ns)
        if identity(before) != identity(after) or size != before.st_size:
            raise CanaryError("input changed while hashing")
        actual = digest.hexdigest()
        if expected_sha256 is not None and actual != expected_sha256:
            raise CanaryError("input SHA-256 mismatch")
        return payload, {"size_bytes": size, "sha256": actual}


def hash_regular(path: Path, **kwargs) -> dict:
    return _observe_regular(path, **kwargs)[1]


def read_pinned_metadata(path: Path, *, max_bytes=65536):
    if max_bytes > 4 * 1024 ** 2:
        raise CanaryError("metadata retention limit exceeded")
    payload, identity = _observe_regular(path, max_bytes=max_bytes, retain=True)
    return bytes(payload), identity


def verify_installer(path: Path) -> dict:
    return hash_regular(path, expected_size=INSTALLER_SIZE,
                        expected_sha256=INSTALLER_SHA256, max_bytes=256 * 1024 ** 2)


def run_bounded(argv: list[str], *, deadline_seconds: float, output_limit=65536,
                env=None, stdout_sink=None) -> dict:
    """Capture bounded helper output and always reap its complete process group.

    This manages host helpers, not Docker-daemon children. Container teardown
    needs an independent finally action using its predeclared unique name.
    """
    started = time.monotonic()
    streams = {"stdout": bytearray(), "stderr": bytearray()}
    counts = {"stdout": 0, "stderr": 0}
    status = "PASS"
    process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True, env=env)
    with selectors.DefaultSelector() as selector:
        for name in streams:
            pipe = getattr(process, name)
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, name)
        try:
            while selector.get_map():
                remaining = deadline_seconds - (time.monotonic() - started)
                if remaining <= 0:
                    status = "TIMEOUT"
                    break
                for key, _ in selector.select(min(0.1, remaining)):
                    output = streams[key.data]
                    limit = output_limit if key.data == "stdout" else min(65536, output_limit)
                    chunk = os.read(key.fileobj.fileno(), min(CHUNK_BYTES, limit + 1 - counts[key.data]))
                    if not chunk:
                        selector.unregister(key.fileobj)
                    else:
                        counts[key.data] += len(chunk)
                        if key.data == "stdout" and stdout_sink is not None:
                            stdout_sink.write(chunk[:max(0, limit - (counts[key.data] - len(chunk)))])
                        else:
                            output.extend(chunk)
                        if counts[key.data] > limit:
                            status = "OUTPUT_LIMIT"
                            break
                if status != "PASS":
                    break
            if status == "PASS":
                process.wait(timeout=max(0.001, deadline_seconds - (time.monotonic() - started)))
                if process.returncode:
                    status = "FAIL"
        except subprocess.TimeoutExpired:
            status = "TIMEOUT"
        finally:
            # Even a successful launcher may have left background helpers.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)
            process.stdout.close()
            process.stderr.close()
    return {"status": status, "exit_code": process.returncode,
            "stdout": bytes(streams["stdout"][:output_limit]),
            "stderr": bytes(streams["stderr"][:output_limit]),
            "bytes_read": counts, "prefix_truncated": status == "OUTPUT_LIMIT",
            "elapsed_seconds": time.monotonic() - started}


def fresh_clipboard(*, sentinel_owner: int, observed_owner: int,
                    sentinel: bytes, payload: bytes | None, limit=1024 ** 2) -> bytes:
    if not sentinel_owner or not observed_owner or observed_owner == sentinel_owner:
        raise CanaryError("no fresh selection ownership transaction")
    if payload is None or payload == sentinel or len(payload) > limit:
        raise CanaryError("clipboard data missing, stale, or over limit")
    # Empty bytes are a valid observed empty transaction, never an absent copy.
    return payload


def original_control_edges(pixels: bytes, *, width: int, height: int) -> dict:
    """Check our original four-color edge control on an already bounded crop.

    A viewport image cannot become complete simply by cropping it. All four
    original edge bars must be observed before this check can succeed.
    """
    if (type(width) is not int or type(height) is not int or (width, height) != (1026, 769)
            or len(pixels) != width * height * 3):
        raise CanaryError("invalid bounded RGB8 grid")
    edges = {"top": (width // 2, 0, (255, 0, 0)),
             "bottom": (width // 2, height - 1, (0, 0, 255)),
             "left": (0, height // 2, (0, 255, 0)),
             "right": (width - 1, height // 2, (255, 0, 255))}
    for name, (x, y, expected) in edges.items():
        offset = (y * width + x) * 3
        if tuple(pixels[offset:offset + 3]) != expected:
            raise CanaryError(f"original {name} edge absent or changed")
    return {"status": "PASS", "scope": "original-control-edge-integrity",
            "width": width, "height": height, "vendor_passes": 0}


def verify_viewport_capture(path: Path, *, pixel_sha256: str) -> dict:
    header = b"P6\n1600 1200\n255\n"
    expected_size = len(header) + 1600 * 1200 * 3
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as file:
        before = os.fstat(file.fileno())
        if not stat.S_ISREG(before.st_mode) or before.st_size != expected_size or file.read(len(header)) != header:
            raise CanaryError("diagnostic viewport header/size mismatch")
        pixels = hashlib.sha256()
        encoded = hashlib.sha256(header)
        remaining = 1600 * 1200 * 3
        while remaining:
            chunk = file.read(min(CHUNK_BYTES, remaining))
            if not chunk:
                raise CanaryError("truncated diagnostic viewport")
            pixels.update(chunk)
            encoded.update(chunk)
            remaining -= len(chunk)
        if file.read(1) or pixels.hexdigest() != pixel_sha256:
            raise CanaryError("diagnostic viewport tail or pixel identity mismatch")
        after = os.fstat(file.fileno())
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
                after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            raise CanaryError("diagnostic viewport changed during validation")
        return {"scope": "viewport-artifact-integrity-only", "size_bytes": expected_size,
                "sha256": encoded.hexdigest(), "pixel_sha256": pixels.hexdigest(), "vendor_passes": 0}


def oom_kill_delta(before: dict, after: dict) -> int:
    def count(metrics):
        value = metrics.get("memory.events")
        if not isinstance(value, str) or len(value) > 4096:
            raise CanaryError("cgroup memory.events unavailable")
        records = {}
        for line in value.splitlines():
            fields = line.split()
            if len(fields) != 2 or not fields[1].isdigit() or fields[0] in records:
                raise CanaryError("invalid cgroup memory.events")
            records[fields[0]] = int(fields[1])
        if "oom_kill" not in records:
            raise CanaryError("cgroup OOM-kill counter unavailable")
        return records["oom_kill"]
    delta = count(after) - count(before)
    if delta < 0:
        raise CanaryError("cgroup OOM counter reset")
    return delta


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", type=Path, help="explicit local official installer to verify")
    parser.add_argument("--json", action="store_true", help="emit metadata JSON")
    args = parser.parse_args(argv)
    report = {"status": "NOT_RUN", "app_launches": 0, "vendor_passes": 0,
              "reason": "No explicit external installer; no download, build, or application launch."}
    code = 0
    if args.installer is not None:
        try:
            report = {"status": "PASS", "scope": "installer-integrity-only",
                      "installer": verify_installer(args.installer), "app_launches": 0,
                      "vendor_passes": 0}
        except (OSError, CanaryError) as error:
            report = {"status": "FAIL", "error_type": type(error).__name__,
                      "app_launches": 0, "vendor_passes": 0}
            code = 1
    print(json.dumps(report, sort_keys=True) if args.json else report["status"])
    return code


if __name__ == "__main__":
    raise SystemExit(main())

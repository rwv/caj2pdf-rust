#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prepare an external opaque vendor tree; never run installation hooks.

Only archive metadata, VERSION/desktop documentation and opaque file hashes are
observed. Application implementation and bundled example documents are not
examined. Use an empty external directory, never a repository build context.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import resource
import selectors
import signal
import stat
import subprocess
import sys
import tarfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from cajviewer_canary import CanaryError, hash_regular, verify_installer


class ArchivePipe:
    def __init__(self, process, *, deadline_seconds=120, byte_limit=2 * 1024 ** 3):
        self.process = process
        self.deadline = time.monotonic() + deadline_seconds
        self.byte_limit = byte_limit
        self.count = 0
        self.stderr = bytearray()
        self.selector = selectors.DefaultSelector()
        for pipe, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
            os.set_blocking(pipe.fileno(), False)
            self.selector.register(pipe, selectors.EVENT_READ, name)

    def read(self, size):
        while True:
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise CanaryError("archive producer deadline exceeded")
            if not any(key.data == "stdout" for key in self.selector.get_map().values()):
                return b""
            for key, _ in self.selector.select(min(remaining, 0.1)):
                chunk = os.read(key.fileobj.fileno(), min(size, 65536))
                if not chunk:
                    self.selector.unregister(key.fileobj)
                elif key.data == "stderr":
                    self.stderr.extend(chunk)
                    if len(self.stderr) > 65536:
                        raise CanaryError("archive producer stderr limit exceeded")
                else:
                    self.count += len(chunk)
                    if self.count > self.byte_limit:
                        raise CanaryError("archive producer byte limit exceeded")
                    return chunk

    def close(self):
        self.selector.close()

    def finish(self):
        # Drain trailing producer diagnostics even after stdout reaches EOF.
        while self.selector.get_map():
            remaining = self.deadline - time.monotonic()
            if remaining <= 0:
                raise CanaryError("archive producer completion deadline exceeded")
            for key, _ in self.selector.select(min(remaining, 0.1)):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    self.selector.unregister(key.fileobj)
                elif key.data == "stderr":
                    self.stderr.extend(chunk)
                    if len(self.stderr) > 65536:
                        raise CanaryError("archive producer stderr limit exceeded")
                else:
                    self.count += len(chunk)
                    if self.count > self.byte_limit:
                        raise CanaryError("archive producer byte limit exceeded")


def producer_limits():
    resource.setrlimit(resource.RLIMIT_AS, (512 * 1024 ** 2,) * 2)
    resource.setrlimit(resource.RLIMIT_CPU, (90,) * 2)


def extract(installer: Path, output: Path) -> dict:
    before = verify_installer(installer)
    receipt_path = output.parent / (output.name + "-receipt.json")
    stderr_path = output.parent / (output.name + "-stderr.bin")
    if output.exists() or output.is_symlink() or stderr_path.exists() or stderr_path.is_symlink():
        raise FileExistsError("preparation outputs already exist")
    # Reserve the receipt before starting the producer; preserve first failures.
    receipt = receipt_path.open("x")
    try:
        output.mkdir(parents=True, mode=0o700, exist_ok=False)
        producer = subprocess.Popen(["dpkg-deb", "--threads-max=1", "--fsys-tarfile", str(installer)],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                    start_new_session=True, preexec_fn=producer_limits)
    except OSError as error:
        with receipt:
            json.dump({"status": "FAIL", "error_type": type(error).__name__, "installer": before,
                       "app_launches": 0, "vendor_passes": 0}, receipt)
            receipt.write("\n")
        receipt_path.chmod(0o400)
        raise
    reader = ArchivePipe(producer)
    report = {"status": "FAIL", "scope": "opaque-extraction-only", "installer": before,
              "app_launches": 0, "sample_documents_opened": 0, "vendor_passes": 0,
              "members": 0, "declared_bytes": 0, "omitted": [], "files": []}
    try:
        with tarfile.open(fileobj=reader, mode="r|", bufsize=65536) as archive:
            for member in archive:
                report["members"] += 1
                report["declared_bytes"] += member.size
                if report["members"] > 10000 or report["declared_bytes"] > 2 * 1024 ** 3:
                    raise CanaryError("archive member or size limit exceeded")
                parts = PurePosixPath(member.name).parts
                if member.name.startswith("/") or ".." in parts:
                    raise CanaryError("unsafe archive member")
                path = PurePosixPath(*parts)
                # Only the application runtime tree enters the external context.
                # Skip package hooks, desktop installation and all documents.
                if (path.parts[:2] != ("opt", "cajviewer")
                        or path.parts[:3] == ("opt", "cajviewer", "doc")):
                    report["omitted"].append({"path": str(path), "size_bytes": member.size})
                    continue
                if not (member.isfile() or member.isdir() or member.issym()):
                    raise CanaryError("unsupported archive member type")
                archive.extract(member, path=output, filter="data")
        while reader.read(65536):
            pass
        reader.finish()
        producer.wait(timeout=max(0.01, reader.deadline - time.monotonic()))
        if producer.returncode:
            raise CanaryError("archive producer failed")
        for directory, subdirectories, files in os.walk(output, followlinks=False):
            for name in sorted(subdirectories + files):
                path = Path(directory) / name
                metadata = path.lstat()
                record = {"path": str(path.relative_to(output)), "mode": stat.S_IMODE(metadata.st_mode)}
                if stat.S_ISLNK(metadata.st_mode):
                    record.update(type="symlink", target=os.readlink(path))
                elif stat.S_ISDIR(metadata.st_mode):
                    record.update(type="directory")
                elif stat.S_ISREG(metadata.st_mode):
                    record.update(type="opaque-file", **hash_regular(path))
                else:
                    raise CanaryError("unexpected extracted file type")
                report["files"].append(record)
        report["files"].sort(key=lambda record: record["path"])
        if verify_installer(installer) != before:
            raise CanaryError("installer identity changed")
        report["status"] = "PASS"
    except (OSError, CanaryError, tarfile.TarError, subprocess.TimeoutExpired) as error:
        report["error_type"] = type(error).__name__
        raise
    finally:
        # Reserve a separate bounded diagnostic-closing window on parse failure.
        # Do not retry the producer or accept a failed extraction.
        if report["status"] != "PASS":
            reader.deadline = time.monotonic() + 5
            try:
                reader.finish()
            except (OSError, CanaryError) as error:
                report["diagnostic_drain_error_type"] = type(error).__name__
        try:
            os.killpg(producer.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except OSError as error:
            report["status"] = "FAIL"
            report["cleanup_error_type"] = type(error).__name__
        try:
            producer.wait(timeout=5)
            report["producer_reap"] = "PASS"
        except subprocess.TimeoutExpired:
            report["status"] = "FAIL"
            report["producer_reap"] = "FAIL"
        report["producer_exit_code"] = producer.returncode
        report["archive_bytes_read"] = reader.count
        report["stderr_bytes"] = len(reader.stderr)
        report["stderr_sha256"] = hashlib.sha256(reader.stderr).hexdigest()
        try:
            report["installer_after"] = verify_installer(installer)
            report["source_post_audit"] = "PASS" if report["installer_after"] == before else "FAIL"
        except (OSError, CanaryError) as error:
            report["source_post_audit"] = "FAIL"
            report["post_audit_error_type"] = type(error).__name__
        if report["source_post_audit"] != "PASS":
            report["status"] = "FAIL"
        reader.close()
        producer.stdout.close()
        producer.stderr.close()
        # Public package metadata and hashes only; no opaque payload is logged.
        with receipt:
            json.dump(report, receipt, indent=2)
            receipt.write("\n")
        receipt_path.chmod(0o400)
        with stderr_path.open("xb") as file:
            file.write(reader.stderr[:65536])
        stderr_path.chmod(0o400)
    if report["status"] != "PASS":
        raise CanaryError("preparation final audit failed")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        report = extract(args.installer, args.output_dir)
    except (OSError, CanaryError, tarfile.TarError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"preparation failed: {type(error).__name__}\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ("files", "omitted")}))


if __name__ == "__main__":
    main()

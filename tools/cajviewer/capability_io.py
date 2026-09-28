# SPDX-License-Identifier: MIT
"""Descriptor-confined, exact-name acquisition transport for capabilities."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import stat
import struct

from capability_protocol import (AUX_LIMIT, CONTENT_LIMIT, FILE_LIMIT, PAGE_KEYS,
                                 RECEIPT_LIMIT, TRANSPORT_LIMIT, Refusal, require)

CHUNK = 65536
MAGIC = b"CAJ-CAP-1\n"
ALLOWLIST = {key + ".ppm": FILE_LIMIT for key in PAGE_KEYS}
ALLOWLIST.update({"digital-1.clipboard": 1024 ** 2, "image-only-1.clipboard": 1024 ** 2,
                  "session.json": RECEIPT_LIMIT, "ready": 64,
                  "application.log": 512 * 1024, "xvfb.log": 64 * 1024,
                  "window-manager.log": 64 * 1024, "display.json": 65536,
                  "observations.json": 512 * 1024})
REQUIRED = set(ALLOWLIST) - {"digital-1.clipboard", "image-only-1.clipboard"}
RESERVE = RECEIPT_LIMIT + 64


class Meter:
    def __init__(self, limit=256 * 1024 ** 2):
        self.limit = limit
        self.requested = self.returned = self.calls = self.written = self.write_submitted = 0
        self.write_completion = "COMPLETE"

    def read(self, stream, count):
        require(type(count) is int and 0 < count <= CHUNK, "invalid-read-size")
        require(self.requested + count <= self.limit, "read-budget")
        self.requested += count
        self.calls += 1
        data = stream.read(count)
        require(type(data) is bytes and len(data) <= count, "invalid-read-result")
        self.returned += len(data)
        return data

    def exact(self, stream, count):
        chunks, remaining = [], count
        while remaining:
            data = self.read(stream, min(CHUNK, remaining))
            require(data, "short-read")
            chunks.append(data)
            remaining -= len(data)
        return b"".join(chunks)

    def write(self, stream, payload):
        view = memoryview(payload)
        while view:
            offered = min(CHUNK, len(view))
            require(self.write_submitted + offered <= self.limit, "write-budget")
            self.write_submitted += offered
            self.write_completion = "UNKNOWN_AFTER_SUBMISSION"
            count = stream.write(view[:offered])
            require(type(count) is int and 0 < count <= offered, "short-or-invalid-write")
            self.written += count
            self.write_completion = "COMPLETE"
            view = view[count:]

    def summary(self):
        return {"requested_bytes": self.requested, "returned_bytes": self.returned,
                "read_calls": self.calls, "written_bytes": self.written, "write_submitted_bytes": self.write_submitted,
                "write_completion": self.write_completion, "read_limit_bytes": self.limit,
                "write_submitted_limit_bytes": self.limit}


def stable(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def directory(path):
    """Hold every ancestor during no-follow traversal; never resolve links."""
    path = Path(path).absolute()
    components = path.parts[1:]
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for component in components:
            require(component not in ("", ".", ".."), "unsafe-path")
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def read_file(path, meter, *, pin=None, limit=FILE_LIMIT, retain=True, metadata=False):
    path = Path(path)
    parent = directory(path.parent)
    try:
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    finally:
        os.close(parent)
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode) and 0 <= before.st_size <= limit, "not-bounded-regular-file")
        digest, data, returned = hashlib.sha256(), bytearray(), 0
        while returned < before.st_size:
            part = meter.read(stream, min(CHUNK, before.st_size - returned))
            require(part, "short-file-read")
            returned += len(part)
            digest.update(part)
            if retain:
                data.extend(part)
        require(not meter.read(stream, 1) and stable(before) == stable(os.fstat(stream.fileno())), "file-changed")
        actual = {"size_bytes": returned, "sha256": digest.hexdigest()}
        require(pin is None or pin == actual, "file-pin-mismatch")
        if metadata:
            actual["file_identity"] = list(stable(before))
        return bytes(data) if retain else None, actual


def exclusive(path, payload, meter):
    parent = directory(Path(path).parent)
    try:
        fd = os.open(Path(path).name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=parent)
    finally:
        os.close(parent)
    with os.fdopen(fd, "wb") as stream:
        meter.write(stream, payload)
        stream.flush()
        os.fsync(stream.fileno())
        os.fchmod(stream.fileno(), 0o400)


def artifact_names(root_fd):
    names = os.listdir(root_fd)
    require(len(names) <= 32 and len(names) == len(set(names)) and set(names) <= set(ALLOWLIST), "unexpected-artifact")
    return sorted(names)


def produce(root, output, meter):
    """Flat length-framed stream. No tar parser, path names, or extraction."""
    root_fd = directory(root)
    records, content, auxiliary, transport = {}, 0, 0, len(MAGIC)
    try:
        names = artifact_names(root_fd)
        require({"session.json", "ready"} <= set(names), "incomplete-terminal-artifacts")
        meter.write(output, MAGIC)
        for name in names:
            fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=root_fd)
            with os.fdopen(fd, "rb") as stream:
                before = os.fstat(stream.fileno())
                require(stat.S_ISREG(before.st_mode) and before.st_size <= ALLOWLIST[name], "artifact-type-or-size")
                content += before.st_size
                auxiliary += 0 if name.endswith(".ppm") else before.st_size
                require(content <= CONTENT_LIMIT and auxiliary <= AUX_LIMIT, "artifact-aggregate-limit")
                header = struct.pack("<BQ", len(name), before.st_size) + name.encode("ascii")
                transport += len(header) + before.st_size
                require(transport + 1 <= TRANSPORT_LIMIT, "transport-limit")
                meter.write(output, header)
                digest, remaining = hashlib.sha256(), before.st_size
                while remaining:
                    payload = meter.read(stream, min(CHUNK, remaining))
                    require(payload, "artifact-short-read")
                    digest.update(payload)
                    meter.write(output, payload)
                    remaining -= len(payload)
                require(not meter.read(stream, 1) and stable(before) == stable(os.fstat(stream.fileno())), "artifact-changed")
                records[name] = {"size_bytes": before.st_size, "sha256": digest.hexdigest()}
        require(names == artifact_names(root_fd), "artifact-set-changed")
        meter.write(output, b"\0")
        return records
    finally:
        os.close(root_fd)


def consume(source, destination, meter):
    root_fd = directory(destination)
    records, content, auxiliary, transport = {}, 0, 0, len(MAGIC)
    try:
        require(not os.listdir(root_fd), "collector-output-not-empty")
        require(meter.exact(source, len(MAGIC)) == MAGIC, "bad-transport-header")
        while True:
            length = meter.exact(source, 1)[0]
            if length == 0:
                break
            require(len(records) < 32 and length <= 80, "member-count-or-name-limit")
            size = struct.unpack("<Q", meter.exact(source, 8))[0]
            raw_name = meter.exact(source, length)
            try:
                name = raw_name.decode("ascii", "strict")
            except UnicodeError:
                raise Refusal("invalid-artifact-name") from None
            require(name in ALLOWLIST and name not in records and size <= ALLOWLIST[name], "extra-duplicate-or-oversize-artifact")
            content += size
            auxiliary += 0 if name.endswith(".ppm") else size
            transport += 9 + length + size
            require(content <= CONTENT_LIMIT and auxiliary <= AUX_LIMIT and transport + 1 <= TRANSPORT_LIMIT, "collection-aggregate-limit")
            fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=root_fd)
            digest, remaining = hashlib.sha256(), size
            with os.fdopen(fd, "wb") as stream:
                while remaining:
                    payload = meter.read(source, min(CHUNK, remaining))
                    require(payload, "incomplete-transport")
                    digest.update(payload)
                    meter.write(stream, payload)
                    remaining -= len(payload)
                stream.flush()
                os.fsync(stream.fileno())
                os.fchmod(stream.fileno(), 0o400)
            records[name] = {"size_bytes": size, "sha256": digest.hexdigest()}
        require(not meter.read(source, 1), "transport-trailing-bytes")
        require({"session.json", "ready"} <= set(records), "missing-terminal-artifacts")
        require(set(os.listdir(root_fd)) == set(records), "collector-set-changed")
        return records
    finally:
        os.close(root_fd)

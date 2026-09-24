#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in CAJSamples inventory and PDF-output checks (MIT licensed).

The external corpus is never fetched by this script. Inventory validation and
PDF-output checks are separate: valid input bytes do not prove conversion.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import selectors
import shutil
import subprocess
import sys
import tempfile
import time
import xml.etree.ElementTree as ET


DEFAULT_MATRIX = Path(__file__).resolve().parent.parent / "tests/conformance/matrix.json"
CHUNK_SIZE = 1024 * 1024
HEX_SHA1 = re.compile(r"[0-9a-f]{40}\Z")
HEX_SHA256 = re.compile(r"[0-9a-f]{64}\Z")
OUTLINE_LINE = re.compile(r'^[|+-](\t+)(".*")\t(.+)$')


class ConformanceError(Exception):
    """A matrix, input, or optional tool could not be checked."""


def relative_path(value: object) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value or "\x00" in value:
        raise ConformanceError(f"unsafe relative path: {value!r}")
    path = PurePosixPath(value)
    if (
        path.is_absolute()
        or value != path.as_posix()
        or any(part in (".", "..") for part in value.split("/"))
        or ":" in path.parts[0]
    ):
        raise ConformanceError(f"unsafe relative path: {value!r}")
    return path


def load_matrix(path: Path) -> list[dict]:
    try:
        matrix = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ConformanceError(f"cannot read matrix {path}: {exc}") from exc
    if (
        not isinstance(matrix, dict)
        or matrix.get("schema_version") != 1
        or not isinstance(matrix.get("samples"), list)
    ):
        raise ConformanceError("matrix must have schema_version 1 and a samples array")
    samples = matrix["samples"]
    if not samples:
        raise ConformanceError("matrix contains no samples")
    ids: set[str] = set()
    paths: set[str] = set()
    for row in samples:
        if not isinstance(row, dict):
            raise ConformanceError("each matrix sample must be an object")
        sample_id = row.get("id")
        sample_path = relative_path(row.get("path"))
        if not isinstance(sample_id, str) or not sample_id:
            raise ConformanceError(f"sample at {sample_path} has no ID")
        if sample_id in ids or str(sample_path) in paths:
            raise ConformanceError(f"duplicate sample ID or path: {sample_id}")
        ids.add(sample_id)
        paths.add(str(sample_path))
        size = row.get("size_bytes")
        if type(size) is not int or size < 0:
            raise ConformanceError(f"{sample_id}: invalid size_bytes")
        if not isinstance(row.get("git_blob_oid"), str) or not HEX_SHA1.fullmatch(
            row["git_blob_oid"]
        ):
            raise ConformanceError(f"{sample_id}: invalid git_blob_oid")
        sha256 = row.get("sha256")
        if sha256 is not None and (
            not isinstance(sha256, str) or not HEX_SHA256.fullmatch(sha256)
        ):
            raise ConformanceError(f"{sample_id}: invalid sha256")
        aliases = row.get("aliases", [])
        if not isinstance(aliases, list):
            raise ConformanceError(f"{sample_id}: aliases must be an array")
        for alias in aliases:
            relative_path(alias)
        if row.get("expected_outcome") not in ("success", "unsupported", "error", "unknown"):
            raise ConformanceError(f"{sample_id}: invalid expected_outcome")
        for key in ("page_count", "outline_count"):
            count = row.get(key)
            if count is not None and (type(count) is not int or count < 0):
                raise ConformanceError(f"{sample_id}: invalid {key}")
        expected_pdf = row.get("expected_pdf")
        if expected_pdf is not None:
            if not isinstance(expected_pdf, dict):
                raise ConformanceError(f"{sample_id}: expected_pdf must be an object")
            version = expected_pdf.get("mutool_version")
            if version is not None and not isinstance(version, str):
                raise ConformanceError(f"{sample_id}: invalid mutool_version")
            outline_hash = expected_pdf.get("outline_sha256")
            if outline_hash is not None and (
                not isinstance(outline_hash, str) or not HEX_SHA256.fullmatch(outline_hash)
            ):
                raise ConformanceError(f"{sample_id}: invalid outline_sha256")
            dimensions = expected_pdf.get("page_dimensions_pt")
            if dimensions is not None and (
                not isinstance(dimensions, list)
                or any(
                    not isinstance(pair, list)
                    or len(pair) != 2
                    or any(
                        type(value) not in (int, float) or not math.isfinite(value)
                        for value in pair
                    )
                    for pair in dimensions
                )
            ):
                raise ConformanceError(f"{sample_id}: invalid page_dimensions_pt")
            rendered = expected_pdf.get("rendered_pages")
            if rendered is not None and (
                not isinstance(rendered, list)
                or any(
                    not isinstance(item, dict)
                    or type(item.get("page")) is not int
                    or item["page"] < 1
                    or not isinstance(item.get("sha256"), str)
                    or not HEX_SHA256.fullmatch(item["sha256"])
                    for item in rendered
                )
            ):
                raise ConformanceError(f"{sample_id}: invalid rendered_pages")
    return samples


def contained_file(root: Path, relative: PurePosixPath) -> Path:
    candidate = root.joinpath(*relative.parts)
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as exc:
        raise ConformanceError(f"missing file {relative}: {exc.strerror or exc}") from exc
    if not resolved.is_relative_to(root):
        raise ConformanceError(f"path escapes root: {relative}")
    if candidate.is_symlink():
        raise ConformanceError(f"canonical file is a symlink: {relative}")
    if not resolved.is_file():
        raise ConformanceError(f"not a regular file: {relative}")
    return resolved


def file_hashes(path: Path, size: int, want_sha256: bool) -> tuple[str, str | None]:
    blob = hashlib.sha1(f"blob {size}\0".encode("ascii"))
    sha256 = hashlib.sha256() if want_sha256 else None
    actual_size = 0
    with path.open("rb") as source:
        while chunk := source.read(CHUNK_SIZE):
            actual_size += len(chunk)
            blob.update(chunk)
            if sha256 is not None:
                sha256.update(chunk)
    if actual_size != size:
        raise ConformanceError(f"file changed while reading: {path}")
    return blob.hexdigest(), sha256.hexdigest() if sha256 is not None else None


def audit_inventory(samples: list[dict], corpus_dir: Path | None) -> dict:
    report = {
        "status": "NOT_RUN",
        "expected": len(samples),
        "passed": 0,
        "failed": 0,
        "not_run": len(samples),
        "failures": [],
    }
    if corpus_dir is None:
        report["reason"] = "CAJ2PDF_CORPUS_DIR is unset"
        return report
    try:
        root = corpus_dir.resolve(strict=True)
        if not root.is_dir():
            raise ConformanceError(f"corpus directory is not a directory: {corpus_dir}")
    except OSError as exc:
        report.update(status="FAIL", reason=f"corpus directory is missing: {corpus_dir}: {exc}")
        return report
    except ConformanceError as exc:
        report.update(status="FAIL", reason=str(exc))
        return report

    seen_files: set[Path] = set()
    report["not_run"] = 0
    for row in samples:
        sample_id = row["id"]
        try:
            source = contained_file(root, relative_path(row["path"]))
            if source in seen_files:
                raise ConformanceError("duplicate canonical input via alias")
            seen_files.add(source)
            actual_size = source.stat().st_size
            if actual_size != row["size_bytes"]:
                raise ConformanceError(
                    f"size mismatch: expected {row['size_bytes']}, got {actual_size}"
                )
            blob, sha256 = file_hashes(source, actual_size, row.get("sha256") is not None)
            if blob != row["git_blob_oid"]:
                raise ConformanceError(f"Git blob hash mismatch: {blob}")
            if row.get("sha256") is not None and sha256 != row["sha256"]:
                raise ConformanceError(f"SHA-256 mismatch: {sha256}")
        except (ConformanceError, OSError) as exc:
            report["failed"] += 1
            report["failures"].append({"id": sample_id, "reason": str(exc)})
        else:
            report["passed"] += 1
    report["status"] = "FAIL" if report["failed"] else "PASS"
    return report


def mutool_run(executable: str, *args: str) -> str:
    try:
        result = subprocess.run(
            [executable, *args], capture_output=True, text=True, timeout=120, check=False
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise ConformanceError(f"mutool could not run: {exc}") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise ConformanceError(f"mutool {' '.join(args[:1])} failed: {detail}")
    if args == ("-v",):
        return result.stdout or result.stderr
    return result.stdout


def parse_pages(output: str) -> list[tuple[float, float]]:
    start = output.find("<page ")
    if start < 0:
        raise ConformanceError("mutool pages returned no page metadata")
    try:
        root = ET.fromstring("<pages>" + output[start:] + "</pages>")
        pages = []
        for index, page in enumerate(root.findall("page"), 1):
            if page.attrib.get("pagenum") != str(index):
                raise ConformanceError("mutool pages returned non-sequential page numbers")
            box = page.find("MediaBox")
            if box is None:
                raise ConformanceError(f"page {index} has no MediaBox")
            left, bottom, right, top = (float(box.attrib[key]) for key in ("l", "b", "r", "t"))
            pages.append((right - left, top - bottom))
    except (ET.ParseError, KeyError, ValueError) as exc:
        raise ConformanceError(f"cannot parse mutool pages output: {exc}") from exc
    return pages


def parse_outline_line(line: str) -> dict:
    match = OUTLINE_LINE.fullmatch(line)
    if match is None:
        raise ConformanceError("cannot parse mutool outline line")
    try:
        title = json.loads(match.group(2))
    except json.JSONDecodeError as exc:
        raise ConformanceError("cannot parse mutool outline title") from exc
    destination = match.group(3)
    page_match = re.search(r"(?:^#|&)page=(\d+)(?:&|$)", destination)
    return {
        "depth": len(match.group(1)) - 1,
        "title": title,
        "page": int(page_match.group(1)) if page_match else None,
        "destination": destination,
    }


def parse_outlines(output: str) -> list[dict]:
    return [parse_outline_line(line) for line in output.splitlines()]


def update_outline_hash(digest: object, entry: dict) -> None:
    normalized = json.dumps(entry, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    digest.update((normalized + "\n").encode("utf-8"))


def outline_sha256(outlines: list[dict]) -> str:
    """Hash parsed hierarchy and destinations without publishing document titles."""
    digest = hashlib.sha256()
    for entry in outlines:
        update_outline_hash(digest, entry)
    return digest.hexdigest()


def stream_mutool(executable: str, args: list[str], consume: object) -> None:
    """Feed bounded stdout chunks to a consumer, with a finite tool deadline."""
    with tempfile.TemporaryFile() as errors:
        try:
            process = subprocess.Popen([executable, *args], stdout=subprocess.PIPE, stderr=errors)
        except OSError as exc:
            raise ConformanceError(f"mutool {args[0]} could not start: {exc}") from exc
        try:
            assert process.stdout is not None
            deadline = time.monotonic() + 120
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while True:
                    ready = selector.select(max(0, deadline - time.monotonic()))
                    if not ready:
                        raise ConformanceError(f"mutool {args[0]} timed out")
                    chunk = os.read(process.stdout.fileno(), CHUNK_SIZE)
                    if not chunk:
                        break
                    consume(chunk)
            process.wait(timeout=max(0, deadline - time.monotonic()))
            if process.returncode:
                errors.seek(0)
                detail = errors.read(4096).decode("utf-8", errors="replace").strip()
                raise ConformanceError(f"mutool {args[0]} failed: {detail}")
        except subprocess.TimeoutExpired as exc:
            raise ConformanceError(f"mutool {args[0]} timed out") from exc
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()


def scan_pdf_outlines(executable: str, pdf: Path, keep_entries: bool) -> tuple[int, str, list[dict]]:
    """Count/hash the hierarchy without retaining reference document titles."""
    digest = hashlib.sha256()
    pending = bytearray()
    count = 0
    entries = []

    def accept_line(raw: bytes) -> None:
        nonlocal count
        if not raw:
            return
        try:
            entry = parse_outline_line(raw.decode("utf-8"))
        except UnicodeDecodeError as exc:
            raise ConformanceError("mutool outline output is not UTF-8") from exc
        update_outline_hash(digest, entry)
        count += 1
        if keep_entries:
            entries.append(entry)

    def consume(chunk: bytes) -> None:
        pending.extend(chunk)
        while (newline := pending.find(b"\n")) >= 0:
            accept_line(bytes(pending[:newline]))
            del pending[: newline + 1]
        if len(pending) > CHUNK_SIZE:
            raise ConformanceError("mutool outline line exceeds 1 MiB")

    stream_mutool(executable, ["show", str(pdf), "outline"], consume)
    accept_line(bytes(pending))
    return count, digest.hexdigest(), entries


def rendered_page_sha256(executable: str, pdf: Path, page: int) -> str:
    """Hash PAM pixel output directly, never buffering a rendered page in memory."""
    arguments = [
        "draw",
        "-q",
        "-L",
        "-B",
        "128",
        "-F",
        "pam",
        "-c",
        "rgb",
        "-r",
        "72",
        "-o",
        "-",
        str(pdf),
        str(page),
    ]
    digest = hashlib.sha256()
    stream_mutool(executable, arguments, digest.update)
    return digest.hexdigest()


def compare_pdf(executable: str, pdf: Path, row: dict, tool_version: str | None = None) -> dict:
    checks = {
        "page_count": "NOT_RUN",
        "page_dimensions": "NOT_RUN",
        "outline_count": "NOT_RUN",
        "outline_hierarchy_destinations": "NOT_RUN",
        "rendered_pages": "NOT_RUN",
    }
    failures = []
    expected = row.get("expected_pdf") or {}
    required_version = expected.get("mutool_version")
    if required_version is not None and required_version != tool_version:
        raise ConformanceError(
            f"mutool version mismatch: expected {required_version!r}, got {tool_version!r}"
        )
    pages = parse_pages(mutool_run(executable, "pages", str(pdf)))

    def check(name: str, actual: object, wanted: object) -> None:
        checks[name] = "PASS" if actual == wanted else "FAIL"
        if checks[name] == "FAIL":
            if name == "outline_hierarchy_destinations":
                failures.append("outline_hierarchy_destinations: mismatch")
            else:
                failures.append(f"{name}: expected {wanted!r}, got {actual!r}")

    if row.get("page_count") is not None:
        check("page_count", len(pages), row["page_count"])
    dimensions = expected.get("page_dimensions_pt")
    if dimensions is not None:
        if not isinstance(dimensions, list) or len(dimensions) != len(pages):
            checks["page_dimensions"] = "FAIL"
            failures.append("page_dimensions: page count differs from expected dimensions")
        else:
            for index, ((width, height), pair) in enumerate(zip(pages, dimensions), 1):
                if (
                    not isinstance(pair, list)
                    or len(pair) != 2
                    or any(type(value) not in (int, float) for value in pair)
                    or abs(width - pair[0]) > 0.01
                    or abs(height - pair[1]) > 0.01
                ):
                    checks["page_dimensions"] = "FAIL"
                    failures.append(f"page_dimensions: page {index} differs by more than 0.01 pt")
                    break
            else:
                checks["page_dimensions"] = "PASS"

    wanted_outlines = expected.get("outlines")
    wanted_outline_hash = expected.get("outline_sha256")
    if row.get("outline_count") is not None or wanted_outlines is not None or wanted_outline_hash:
        outline_count, outline_hash, outlines = scan_pdf_outlines(
            executable, pdf, wanted_outlines is not None
        )
        if row.get("outline_count") is not None:
            check("outline_count", outline_count, row["outline_count"])
        if wanted_outline_hash is not None:
            check("outline_hierarchy_destinations", outline_hash, wanted_outline_hash)
        if wanted_outlines is not None:
            actual = []
            for entry in outlines:
                actual.append({key: entry[key] for key in ("depth", "title", "page")})
                if len(actual) <= len(wanted_outlines) and "destination" in wanted_outlines[len(actual) - 1]:
                    actual[-1]["destination"] = entry["destination"]
            check("outline_hierarchy_destinations", actual, wanted_outlines)

    wanted_rendered = expected.get("rendered_pages")
    if wanted_rendered:
        rendered_failures = []
        for item in wanted_rendered:
            number = item["page"]
            if type(number) is not int or not 1 <= number <= len(pages):
                raise ConformanceError(f"invalid rendered page number: {number!r}")
            if rendered_page_sha256(executable, pdf, number) != item["sha256"]:
                rendered_failures.append(f"page {number} hash mismatch")
        checks["rendered_pages"] = "FAIL" if rendered_failures else "PASS"
        failures.extend(rendered_failures)

    status = "FAIL" if failures else "PASS" if "PASS" in checks.values() else "NOT_RUN"
    return {"id": row["id"], "status": status, "checks": checks, "failures": failures}


def audit_pdfs(samples: list[dict], pdf_dir: Path | None, inventory: dict, mutool: str) -> dict:
    report = {
        "status": "NOT_RUN",
        "passed": 0,
        "failed": 0,
        "unsupported": 0,
        "not_run": len(samples),
        "tool": None,
        "results": [],
    }
    if pdf_dir is None:
        report["reason"] = "--pdf-dir was not supplied"
        return report
    if inventory["status"] != "PASS":
        report.update(status="FAIL", reason="PDF checks require a valid corpus inventory")
        return report
    try:
        root = pdf_dir.resolve(strict=True)
        if not root.is_dir():
            raise ConformanceError(f"PDF output directory is not a directory: {pdf_dir}")
        executable = shutil.which(mutool)
        if executable is None:
            raise ConformanceError(f"mutool executable not found: {mutool}")
        report["tool"] = mutool_run(executable, "-v").strip()
    except OSError as exc:
        report.update(status="FAIL", reason=f"PDF output directory is missing: {pdf_dir}: {exc}")
        return report
    except ConformanceError as exc:
        report.update(status="FAIL", reason=str(exc))
        return report

    report["not_run"] = 0
    for row in samples:
        outcome = row["expected_outcome"]
        if outcome == "unsupported":
            report["unsupported"] += 1
            report["results"].append(
                {"id": row["id"], "status": "UNSUPPORTED", "reason": "reference expectation only"}
            )
            continue
        if outcome != "success":
            report["not_run"] += 1
            report["results"].append(
                {"id": row["id"], "status": "NOT_RUN", "reason": "no supported reference expectation"}
            )
            continue
        pdf_relative = relative_path(row["path"]).with_suffix(".pdf")
        try:
            pdf = contained_file(root, pdf_relative)
            result = compare_pdf(executable, pdf, row, report["tool"])
        except (ConformanceError, OSError, KeyError, TypeError) as exc:
            result = {"id": row["id"], "status": "FAIL", "failures": [str(exc)]}
        report["results"].append(result)
        if result["status"] == "PASS":
            report["passed"] += 1
        elif result["status"] == "FAIL":
            report["failed"] += 1
        else:
            report["not_run"] += 1
    report["status"] = (
        "FAIL" if report["failed"] else "PASS" if report["passed"] else "NOT_RUN"
    )
    return report


def run(matrix_path: Path, corpus_dir: Path | None, pdf_dir: Path | None, mutool: str) -> dict:
    samples = load_matrix(matrix_path)
    inventory = audit_inventory(samples, corpus_dir)
    pdf = audit_pdfs(samples, pdf_dir, inventory, mutool)
    return {
        "schema_version": 1,
        "sample_count": len(samples),
        "reference_unsupported": sum(
            row["expected_outcome"] == "unsupported" for row in samples
        ),
        "inventory": inventory,
        "pdf": pdf,
    }


def print_text(report: dict) -> None:
    inventory = report["inventory"]
    pdf = report["pdf"]
    print(
        f"Corpus inventory [{inventory['status']}]: PASS={inventory['passed']} FAIL={inventory['failed']} "
        f"NOT_RUN={inventory['not_run']} / {report['sample_count']}"
    )
    if inventory.get("reason"):
        print(f"  {inventory['reason']}")
    for failure in inventory["failures"][:10]:
        print(f"  FAIL {failure['id'] or 'corpus'}: {failure['reason']}")
    if len(inventory["failures"]) > 10:
        print(f"  ... {len(inventory['failures']) - 10} more failures")
    print(
        f"PDF output checks [{pdf['status']}]: PASS={pdf['passed']} FAIL={pdf['failed']} "
        f"UNSUPPORTED={pdf['unsupported']} NOT_RUN={pdf['not_run']} / {report['sample_count']}"
    )
    if pdf.get("reason"):
        print(f"  {pdf['reason']}")
    if pdf["tool"]:
        print(f"  Renderer: {pdf['tool']}")
    for result in pdf["results"]:
        if result["status"] == "FAIL":
            print(f"  FAIL {result['id']}: {'; '.join(result.get('failures', []))}")
    print(
        f"Reference unsupported: {report['reference_unsupported']} "
        "(matrix expectation; no candidate converter was run)"
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--corpus-dir", type=Path, default=None)
    parser.add_argument("--pdf-dir", type=Path, default=None)
    parser.add_argument("--mutool", default="mutool", help="optional PDF checker executable")
    parser.add_argument("--json", action="store_true", help="print a machine-readable report")
    args = parser.parse_args(argv)
    corpus_value = args.corpus_dir or os.environ.get("CAJ2PDF_CORPUS_DIR")
    corpus_dir = Path(corpus_value) if corpus_value else None
    try:
        report = run(args.matrix, corpus_dir, args.pdf_dir, args.mutool)
    except ConformanceError as exc:
        print(f"Conformance setup FAIL: {exc}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_text(report)
    return 1 if "FAIL" in (report["inventory"]["status"], report["pdf"]["status"]) else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run the public CLI against the pinned external corpus (Linux/POSIX only).

This measures current conversion behavior, not Python-reference or pixel parity.
Documents, PDFs and tool logs stay in the explicitly selected external directory.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import time

import conformance
import current_format_order


class Commands:
    """Bound each child and retain private logs without buffering tool output."""

    def __init__(self, directory: Path, timeout: float = 180):
        if not 0 < timeout <= 3600:
            raise ValueError("timeout must be in (0, 3600] seconds")
        self.directory = directory
        self.timeout = timeout

    @staticmethod
    def limits() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (1024**3, 1024**3))
        resource.setrlimit(resource.RLIMIT_FSIZE, (512 * 1024**2, 512 * 1024**2))

    def run(self, arguments: list[str | Path], label: str) -> dict:
        stdout = self.directory / f"{label}.stdout"
        stderr = self.directory / f"{label}.stderr"
        started = time.monotonic()
        environment = dict(os.environ, TMPDIR=str(self.directory))
        with stdout.open("xb") as out, stderr.open("xb") as err:
            process = subprocess.Popen(
                [str(arg) for arg in arguments], stdout=out, stderr=err,
                env=environment, preexec_fn=self.limits, start_new_session=True,
            )
            try:
                code = process.wait(timeout=self.timeout)
            except subprocess.TimeoutExpired:
                code = "TIMEOUT"
            finally:
                # Also reap children on Ctrl-C or an exception in the harness.
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        with stdout.open("rb") as out, stderr.open("rb") as err:
            result = {
                "exit_code": code,
                "seconds": round(time.monotonic() - started, 3),
                "stdout": out.read(65537).decode("utf-8", errors="replace"),
                "stderr": err.read(8192).decode("utf-8", errors="replace"),
            }
        result["stdout_truncated"] = stdout.stat().st_size > 65536
        return result


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def metadata(result: dict) -> dict | None:
    if result["exit_code"] != 0 or result["stdout_truncated"]:
        return None
    try:
        value = json.loads(result["stdout"])
    except json.JSONDecodeError:
        return None
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        return None
    return value


def conversion_status(code: int | str, diagnostic: str) -> str:
    if code == 0:
        return "PASS"
    if code == 1 and any(term in diagnostic for term in (
        "not supported", "unsupported", "cannot omit source pages", "no image to draw",
    )):
        return "UNSUPPORTED"
    return "FAIL"


def run(matrix: Path, corpus: Path | None, candidate: Path | None,
        directory: Path | None, *, timeout: float = 180) -> dict:
    samples = conformance.load_matrix(matrix)
    inventory = conformance.audit_inventory(samples, corpus)
    report = {"schema_version": 1, "inventory_before": inventory,
              "scope": "Public CLI conversion, PDF structure and source-page identity/order; not rendered-page fidelity",
              "status": "NOT_RUN", "results": []}
    if inventory["status"] == "NOT_RUN":
        return report
    if inventory["status"] != "PASS":
        report["status"] = "FAIL"
        return report
    if candidate is None or directory is None:
        raise ValueError("--candidate and --output-dir are required with a corpus")
    candidate = candidate.resolve(strict=True)
    corpus = corpus.resolve(strict=True)
    directory = directory.resolve()
    # This tool must never create corpus-derived artifacts inside the repository.
    repository = Path(__file__).resolve().parent.parent
    if directory.is_relative_to(repository) or directory.is_relative_to(corpus):
        raise ValueError("output directory must be outside the repository and corpus")
    commands = Commands(directory, timeout)
    binary_hash = digest(candidate)
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    report.update(binary_sha256=binary_hash, status="FAIL", limits={
        "timeout_seconds": timeout, "address_space_bytes": 1024**3,
        "per_file_bytes": 512 * 1024**2,
    })
    report["matrix_sha256"] = digest(matrix)
    report["tools"] = {}
    for name, args in (("candidate", [candidate, "--version"]), ("qpdf", ["qpdf", "--version"]),
                       ("mutool", ["mutool", "-v"]), ("pdfimages", ["pdfimages", "-v"])):
        result = commands.run(args, "version-" + name)
        report["tools"][name] = {"exit_code": result["exit_code"],
                                  "version": (result["stdout"] or result["stderr"]).splitlines()[:1]}
    for index, row in enumerate(samples):
        source = conformance.contained_file(corpus, conformance.relative_path(row["path"]))
        inspection = commands.run([candidate, "inspect", source, "--json"], f"{index:02}-inspect")
        info = metadata(inspection)
        entry = {"id": row["id"], "source_sha256": digest(source),
                 "format": row["detected_type"], "metadata": info,
                 "inspection_exit_code": inspection["exit_code"], "attempts": []}
        # Do not infer a variant or silently retry after an arbitrary failure.
        modes = [False, True] if info and info.get("variant") in ("HN-B", "C8") else [False]
        for omit in modes:
            label = f"{index:02}-{'omit' if omit else 'default'}"
            pdf = directory / f"{label}.pdf"
            args = [candidate, source, "-o", pdf] + (["--no-bookmarks"] if omit else [])
            result = commands.run(args, label)
            diagnostic = result["stderr"].replace(str(source), row["id"]).strip()
            attempt = {"no_bookmarks": omit, "exit_code": result["exit_code"],
                       "conversion_status": conversion_status(result["exit_code"], diagnostic),
                       "seconds": result["seconds"], "diagnostic": diagnostic,
                       "output_exists": pdf.is_file()}
            if result["exit_code"] == 0:
                if not pdf.is_file():
                    attempt.update(conversion_status="FAIL", diagnostic="successful CLI produced no PDF")
                else:
                    validity = commands.run(["qpdf", "--check", pdf], label + "-qpdf")
                    pages = commands.run(["qpdf", "--show-npages", pdf], label + "-pages")
                    count = None
                    if pages["exit_code"] == 0 and not pages["stdout_truncated"]:
                        value = pages["stdout"].strip()
                        if value.isdecimal() and len(value) <= 10:
                            count = int(value)
                    attempt.update(
                        output_sha256=digest(pdf), output_bytes=pdf.stat().st_size,
                        pages=count, pdf_check_exit_code=validity["exit_code"],
                        pdf_check="PASS" if validity["exit_code"] == 0 else "WARNING" if validity["exit_code"] == 3 else "FAIL",
                        page_count_check="PASS" if count is not None and info and count == info.get("page_count") else "FAIL",
                        page_order_check="NOT_RUN", source_outline_check="NOT_RUN", pixel_check="NOT_RUN",
                    )
                    try:
                        outlines = commands.run(["mutool", "show", pdf, "outline"], label + "-outlines")
                        if outlines["exit_code"] != 0:
                            raise conformance.ConformanceError("mutool outline inspection failed")
                        count = 0
                        outline_hash = hashlib.sha256()
                        with (directory / f"{label}-outlines.stdout").open("rb") as lines:
                            while line := lines.readline(conformance.CHUNK_SIZE + 1):
                                if len(line) > conformance.CHUNK_SIZE:
                                    raise conformance.ConformanceError("outline line exceeds 1 MiB")
                                if line.strip():
                                    outline = conformance.parse_outline_line(line.decode("utf-8").strip())
                                    conformance.update_outline_hash(outline_hash, outline)
                                    count += 1
                        attempt.update(outline_count=count, outline_sha256=outline_hash.hexdigest())
                    except (conformance.ConformanceError, UnicodeError) as error:
                        attempt["outline_inspection_error"] = str(error)
                    try:
                        order = current_format_order.check(commands, source, pdf, row, info or {}, label)
                        attempt["page_order"] = order
                        attempt["page_order_check"] = order["status"]
                        expected_outline = current_format_order.source_outline_hash(source, row["detected_type"], info or {})
                        if expected_outline is not None and not omit:
                            attempt["source_outline_check"] = "PASS" if expected_outline == (
                                attempt.get("outline_count"), attempt.get("outline_sha256"),
                            ) else "FAIL"
                    except (OSError, ValueError, KeyError, conformance.ConformanceError) as error:
                        attempt["page_order_check"] = "FAIL"
                        attempt["source_check_error"] = str(error)
            elif pdf.exists():
                attempt["failed_output_cleanup"] = True
            entry["attempts"].append(attempt)
        report["results"].append(entry)
        (directory / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    report["inventory_after"] = conformance.audit_inventory(samples, corpus)
    report["binary_unchanged"] = digest(candidate) == binary_hash
    # COMPLETE means attempts finished, including explicit failures/unsupported cases.
    # It never means all inputs converted or source fidelity was established.
    if report["inventory_after"]["status"] == "PASS" and report["binary_unchanged"]:
        report["status"] = "COMPLETE"
    (directory / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, default=conformance.DEFAULT_MATRIX)
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--output-dir", type=Path, help="new external directory; must not already exist")
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    corpus = args.corpus_dir or os.environ.get("CAJ2PDF_CORPUS_DIR")
    try:
        report = run(args.matrix, Path(corpus) if corpus else None,
                     args.candidate, args.output_dir, timeout=args.timeout)
    except (OSError, ValueError, conformance.ConformanceError) as error:
        parser.exit(2, f"Current format check failed: {error}\n")
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())

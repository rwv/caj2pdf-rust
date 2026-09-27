#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, hash-pinned black-box HN/C8 layout reference protocol.

This runner treats the external converter as an executable only. It never
imports or reads its implementation. Source documents, reference PDFs, native
libraries, and mutated copies remain outside this repository. No result from
an omitted external input counts as a layout comparison.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import resource
import selectors
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any, Mapping


ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "tests/conformance/matrix.json"
MATRIX_SHA256 = "af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9"
REFERENCE_REVISION = "8cbc3c5721acb762f739434eb3d206171dbb022a"
PYPDF2_VERSION = "1.26.0"
PYPDF2_TREE_SHA256 = "4d33afdda9bb9ed730fba0355c42291d3380ce93f89cf4ec489338dcd385b36c"
READ_CHUNK = 1024 * 1024
MAX_PDF_BYTES = 128 * 1024 * 1024
MAX_CHILD_VIRTUAL_BYTES = 1024 * 1024 * 1024
MAX_PERTURBATIONS = 24
CONVERTER_TIMEOUT_SECONDS = 180

PINNED_HASHES = {
    "python": "889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9",
    "converter": "c2bede4bd4e1308fb9f7ec7e5593106c5b41188c337158a61cfadadc6f81f739",
    "libjbigdec": "d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6",
    "git": "356db14e102d68a1a37d8a1ac577dfd678d45d46e92f468bef8b7154e7bfdc60",
    "qpdf": "30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792",
    "mutool": "b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7",
    "pdfinfo": "a1a371340d7b76e7d501da9136cc9256dfdb9cbdf09300520d7a3b4465343e67",
    "pdfimages": "213eba4a36ef021f49a0abc94292a7566baba8dfafef5017467166d9f06074f5",
}
PINNED_VERSIONS = {
    "python": "Python 3.13.5",
    "git": "git version 2.47.3",
    "qpdf": "qpdf version 12.2.0",
    "mutool": "mutool version 1.25.1",
    "pdfinfo": "pdfinfo version 25.03.0",
    "pdfimages": "pdfimages version 25.03.0",
}
NATIVE_COMPILER_RECORD = {
    "version": "Debian g++ 14.2.0 (x86_64-linux-gnu)",
    "sha256": "6b3696e4dcb85e1c949c732a02befa50e3983ecf94ce7e8e58d9d503b954b79d",
    "note": "provenance of external libjbigdec.so; compiler is not invoked by this runner",
}


@dataclass(frozen=True)
class Profile:
    name: str
    source_id: str
    expected_pdf_sha256: str
    expected_pages: int
    expected_draws: int
    mutation_page: int


@dataclass(frozen=True)
class MutationField:
    profile: str
    page_number: int
    name: str
    absolute_offset: int
    length: int
    byte_index: int = 0


PROFILES = (
    Profile(
        "hn_a",
        "issue-21/实时网络流量异常检测算法研究和系统实现_林尚朕.caj",
        "833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40",
        68,
        91,
        16,
    ),
    Profile(
        "c8",
        "issue-33/test1.caj",
        "acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885",
        7,
        34,
        1,
    ),
    Profile(
        "hn_b",
        "issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj",
        "b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51",
        2,
        2,
        1,
    ),
)


class ReferenceError(Exception):
    """A requested external precondition or black-box comparison failed."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(READ_CHUNK):
            digest.update(block)
    return digest.hexdigest()


def hash_pypdf2_tree(pydeps: Path) -> tuple[str, int]:
    """Hash package files by relative name and content, ignoring bytecode."""
    roots = (pydeps / "PyPDF2", pydeps / "pypdf2-1.26.0.dist-info")
    if any(not root.is_dir() for root in roots):
        raise ReferenceError("PyPDF2 1.26.0 package and dist-info directories are required")
    files = sorted(
        path
        for root in roots
        for path in root.rglob("*")
        if path.is_file() and "__pycache__" not in path.parts and path.suffix != ".pyc"
    )
    digest = hashlib.sha256()
    for path in files:
        digest.update(path.relative_to(pydeps).as_posix().encode("utf-8"))
        digest.update(b"\x00")
        digest.update(bytes.fromhex(sha256_file(path)))
    return digest.hexdigest(), len(files)


def _output(
    argv: list[str], *, cwd: Path | None = None, timeout: int = 10,
    env: Mapping[str, str] | None = None,
) -> str:
    try:
        child = subprocess.Popen(
            argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
    except OSError as exc:
        raise ReferenceError(f"tool could not start: {argv[0]}") from exc
    output = bytearray()
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            assert child.stdout is not None
            selector.register(child.stdout, selectors.EVENT_READ)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise ReferenceError(f"tool timed out: {argv[0]}")
                chunk = os.read(child.stdout.fileno(), min(4096, 4097 - len(output)))
                if not chunk:
                    selector.unregister(child.stdout)
                else:
                    output.extend(chunk)
                    if len(output) > 4096:
                        raise ReferenceError(f"tool output exceeded 4 KiB: {argv[0]}")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ReferenceError(f"tool timed out: {argv[0]}")
        try:
            exit_code = child.wait(timeout=remaining)
        except subprocess.TimeoutExpired as exc:
            raise ReferenceError(f"tool timed out: {argv[0]}") from exc
        if exit_code != 0:
            raise ReferenceError(f"tool exited {exit_code}: {argv[0]}")
        return output.decode("utf-8", "replace").strip()
    except OSError as exc:
        raise ReferenceError(f"tool output read failed: {argv[0]}") from exc
    finally:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
        if child.stdout is not None:
            child.stdout.close()


def _pin_file(name: str, path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ReferenceError(f"missing required {name} file: {path}")
    actual = sha256_file(path)
    if actual != PINNED_HASHES[name]:
        raise ReferenceError(f"{name} SHA-256 differs from pinned executable/library")
    return {"path": str(path.resolve()), "sha256": actual}


def _version(name: str, path: Path) -> str:
    arg = "-v" if name in ("mutool", "pdfinfo", "pdfimages") else "--version"
    result = _output([str(path), arg])
    expected = PINNED_VERSIONS[name]
    if not result.startswith(expected):
        raise ReferenceError(f"{name} version differs from pinned {expected}")
    return result.splitlines()[0]


def audit_environment(paths: Mapping[str, Path]) -> dict[str, Any]:
    """Require the same clean reference checkout and external executables."""
    repository = paths["reference_repo"]
    if not repository.is_dir():
        raise ReferenceError(f"missing Python reference checkout: {repository}")
    converter = repository / "caj2pdf"
    binaries = {name: _pin_file(name, paths[name]) for name in PINNED_HASHES if name != "converter"}
    binaries["converter"] = _pin_file("converter", converter)
    git = paths["git"]
    revision = _output([str(git), "-C", str(repository), "rev-parse", "HEAD"])
    if revision != REFERENCE_REVISION:
        raise ReferenceError("Python reference revision differs from pinned commit")
    cleanliness = _output(
        [str(git), "-C", str(repository), "status", "--porcelain", "--untracked-files=all"]
    )
    if cleanliness:
        raise ReferenceError("Python reference checkout is not clean")
    for name in PINNED_VERSIONS:
        binaries[name]["version"] = _version(name, paths[name])
    pydeps = paths["pydeps"]
    tree_hash, file_count = hash_pypdf2_tree(pydeps)
    if tree_hash != PYPDF2_TREE_SHA256:
        raise ReferenceError("PyPDF2 package tree SHA-256 differs from pinned fixture")
    # The version/import check is run with exactly the same module path as the converter.
    with tempfile.TemporaryDirectory(prefix="hnc8-pycache-check-") as cache:
        py_check = _output(
            [
                str(paths["python"]), "-W", "ignore", "-c",
                "import PyPDF2; print(PyPDF2.__version__); print(PyPDF2.__file__)",
            ],
            env={
                **os.environ,
                "PYTHONPATH": str(pydeps.resolve()),
                "PYTHONPYCACHEPREFIX": cache,
                "PYTHONDONTWRITEBYTECODE": "1",
                "PYTHONNOUSERSITE": "1",
            },
            cwd=repository,
            timeout=10,
        )
    lines = py_check.splitlines()
    if len(lines) != 2 or lines[0] != PYPDF2_VERSION:
        raise ReferenceError("PyPDF2 import/version check failed")
    if not Path(lines[1]).resolve().is_relative_to(pydeps.resolve()):
        raise ReferenceError("PyPDF2 resolved outside the pinned dependency directory")
    return {
        "reference_revision": revision,
        "reference_clean": True,
        "pypdf2": {"version": PYPDF2_VERSION, "tree_sha256": tree_hash, "file_count": file_count},
        "native_compiler_record": NATIVE_COMPILER_RECORD,
        "binaries": binaries,
        "environment": {
            "PYTHONPATH": str(pydeps.resolve()),
            "PYTHONHASHSEED": "0",
            "PYTHONPYCACHEPREFIX": "fresh empty case-specific path",
            "PYTHONDONTWRITEBYTECODE": "1",
            "PYTHONNOUSERSITE": "1",
            "LC_ALL": "C.UTF-8",
            "TZ": "UTC",
            "cwd": "fresh case directory with ./libjbigdec.so symlink",
        },
        "invocation": ["python", "converter", "convert", "SOURCE", "-o", "OUTPUT"],
        "timeout_seconds": CONVERTER_TIMEOUT_SECONDS,
    }


def load_source_rows(matrix_path: Path = MATRIX) -> tuple[str, list[dict[str, Any]]]:
    matrix_hash = sha256_file(matrix_path)
    if matrix_hash != MATRIX_SHA256:
        raise ReferenceError("corpus matrix SHA-256 differs from #22/#61 pin")
    try:
        document = json.loads(matrix_path.read_text(encoding="utf-8"))
        rows = [row for row in document["samples"] if row["detected_type"] in ("HN", "C8")]
    except (KeyError, TypeError, ValueError) as exc:
        raise ReferenceError("corpus matrix is malformed") from exc
    if len(rows) != 27 or len({row["id"] for row in rows}) != 27:
        raise ReferenceError("HN/C8 source matrix must have exactly 27 unique IDs")
    if any(not isinstance(row.get("sha256"), str) or len(row["sha256"]) != 64 for row in rows):
        raise ReferenceError("HN/C8 source matrix has an invalid SHA-256")
    if not {profile.source_id for profile in PROFILES}.issubset({row["id"] for row in rows}):
        raise ReferenceError("one or more layout sources are absent from the matrix")
    return matrix_hash, rows


def audit_sources(corpus: Path, rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if not corpus.is_dir():
        raise ReferenceError(f"missing requested corpus directory: {corpus}")
    root = corpus.resolve()
    checked = []
    for row in rows:
        relative = Path(row["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ReferenceError(f"unsafe source path in matrix: {row['id']}")
        path = (root / relative).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            raise ReferenceError(f"missing or escaping requested source: {row['id']}")
        size = path.stat().st_size
        if size != row["size_bytes"] or sha256_file(path) != row["sha256"]:
            raise ReferenceError(f"requested source changed: {row['id']}")
        checked.append({"id": row["id"], "path": str(path), "size_bytes": size, "sha256": row["sha256"]})
    return checked


def _tree_size(root: Path) -> int:
    total = 0
    for path in root.rglob("*"):
        try:
            if path.is_file() and not path.is_symlink():
                total += path.stat().st_size
        except FileNotFoundError:
            # The black-box child may remove a temporary file during sampling.
            pass
    return total


def _read_rss() -> dict[str, int]:
    return {"harness_vmhwm_kib": _vm_hwm(os.getpid()) or 0}


def _vm_hwm(pid: int) -> int | None:
    """Read Linux's per-process high-water RSS; ru_maxrss is inherited here."""
    try:
        with Path(f"/proc/{pid}/status").open("rb") as status:
            for line in status:
                if line.startswith(b"VmHWM:"):
                    parts = line.split()
                    if len(parts) == 3 and parts[2] == b"kB":
                        return int(parts[1])
    except (OSError, ValueError):
        pass
    return None


def _child_limits() -> None:
    resource.setrlimit(resource.RLIMIT_AS, (MAX_CHILD_VIRTUAL_BYTES, MAX_CHILD_VIRTUAL_BYTES))
    resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_PDF_BYTES, MAX_PDF_BYTES))
    resource.setrlimit(resource.RLIMIT_CPU, (CONVERTER_TIMEOUT_SECONDS, CONVERTER_TIMEOUT_SECONDS))


def _converter_env(pydeps: Path, cwd: Path) -> dict[str, str]:
    return {
        **os.environ,
        "PYTHONPATH": str(pydeps.resolve()),
        "PYTHONHASHSEED": "0",
        "PYTHONPYCACHEPREFIX": str(cwd / "pycache"),
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONNOUSERSITE": "1",
        "LC_ALL": "C.UTF-8",
        "TZ": "UTC",
    }


def run_converter(
    source: Path, artifact_root: Path, paths: Mapping[str, Path], *, label: str
) -> dict[str, Any]:
    """Run the external CLI from a fresh directory with a bounded lifetime."""
    cwd = Path(tempfile.mkdtemp(prefix=f"{label}-", dir=artifact_root))
    (cwd / "libjbigdec.so").symlink_to(paths["libjbigdec"].resolve())
    output = cwd / "output.pdf"
    command = [
        str(paths["python"]),
        str(paths["reference_repo"] / "caj2pdf"),
        "convert",
        str(source),
        "-o",
        str(output),
    ]
    start = time.monotonic()
    try:
        child = subprocess.Popen(
            command,
            cwd=cwd,
            env=_converter_env(paths["pydeps"], cwd),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
            preexec_fn=_child_limits,
        )
    except OSError as exc:
        raise ReferenceError(f"reference converter could not start ({type(exc).__name__})") from exc
    sampled_temporary = 0
    sampled_session = 0
    sampled_vmhwm = 0
    timed_out = False
    try:
        while True:
            remaining = CONVERTER_TIMEOUT_SECONDS - (time.monotonic() - start)
            if remaining <= 0:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                exit_code = child.wait()
                timed_out = True
                break
            try:
                exit_code = child.wait(timeout=min(0.02, remaining))
                break
            except subprocess.TimeoutExpired:
                sampled_temporary = max(sampled_temporary, _tree_size(cwd))
                sampled_session = max(sampled_session, _tree_size(artifact_root))
                sampled_vmhwm = max(sampled_vmhwm, _vm_hwm(child.pid) or 0)
    finally:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()
    sampled_temporary = max(sampled_temporary, _tree_size(cwd))
    sampled_session = max(sampled_session, _tree_size(artifact_root))
    result: dict[str, Any] = {
        "cwd": str(cwd),
        "output_path": str(output),
        "elapsed_milliseconds": round((time.monotonic() - start) * 1000),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "max_observed_temporary_bytes": sampled_temporary,
        "max_observed_session_bytes": sampled_session,
        "temporary_sample_interval_milliseconds": 20,
        "max_observed_child_vmhwm_kib": sampled_vmhwm,
        "status": "TIMEOUT" if timed_out else "EXIT_NONZERO" if exit_code else "PASS",
    }
    if output.exists():
        size = output.stat().st_size
        result["pdf_size_bytes"] = size
        if size <= MAX_PDF_BYTES:
            result["pdf_sha256"] = sha256_file(output)
        else:
            result["status"] = "PDF_TOO_LARGE"
    elif not timed_out and exit_code == 0:
        result["status"] = "MISSING_PDF"
    if result["status"] != "PASS":
        return result
    if result.get("pdf_size_bytes", 0) == 0:
        result["status"] = "EMPTY_PDF"
    return result


def _pdfinfo_pages(pdf: Path, tool: Path) -> int:
    output = _output([str(tool), str(pdf)], timeout=30)
    matches = [line for line in output.splitlines() if line.startswith("Pages:")]
    if len(matches) != 1:
        raise ReferenceError("pdfinfo did not report one page count")
    try:
        return int(matches[0].partition(":")[2].strip())
    except ValueError as exc:
        raise ReferenceError("pdfinfo page count is not an integer") from exc


def pdf_metadata(pdf: Path, paths: Mapping[str, Path]) -> dict[str, Any]:
    from hnc8_layout_pdf import PdfMetadataError, extract_pdf_metadata

    try:
        metadata = extract_pdf_metadata(
            pdf,
            {name: paths[name] for name in ("qpdf", "mutool", "pdfimages")},
        )
    except PdfMetadataError as exc:
        raise ReferenceError(f"independent PDF metadata extraction failed: {exc}") from exc
    info_pages = _pdfinfo_pages(pdf, paths["pdfinfo"])
    if metadata["page_count"] != info_pages:
        raise ReferenceError("pdfinfo and qpdf/mutool disagree on page count")
    metadata["pdfinfo_page_count"] = info_pages
    return metadata


def _profile_source(rows: list[dict[str, Any]], profile: Profile, corpus: Path) -> Path:
    row = next(row for row in rows if row["id"] == profile.source_id)
    return (corpus / row["path"]).resolve()


def hn_b_mapping(source: Path, source_id: str, metadata: dict[str, Any]) -> dict[str, Any]:
    pages = [_source_page(source, source_id, number) for number in range(1, 7)]
    counts = [page["image_count"] for page in pages]
    if counts != [1, 0, 0, 0, 0, 1] or metadata["page_count"] != 2:
        raise ReferenceError("HN-B six-source-row/two-output-page profile changed")
    if any(len(page["draws"]) != 1 for page in metadata["pages"]):
        raise ReferenceError("HN-B output pages must each draw exactly one JPEG")
    positive_text = all(page["text_length"] > 0 for page in pages[1:5])
    if not positive_text:
        raise ReferenceError("HN-B image-free source rows lost their positive text spans")
    source_streams = [page["images"][0]["payload_sha256"] for page in pages if page["images"]]
    output_streams = [page["draws"][0]["raw_stream_sha256"] for page in metadata["pages"]]
    if source_streams != output_streams:
        raise ReferenceError("HN-B output images do not match source rows 1 and 6")
    return {
        "source_page_count": 6,
        "source_image_counts": counts,
        "source_text_lengths": [page["text_length"] for page in pages],
        "positive_text_spans_on_image_free_rows": positive_text,
        "output_page_to_source_page": [1, 6],
        "jpeg_stream_sha256": source_streams,
    }


def run_profiles(
    rows: list[dict[str, Any]], corpus: Path, artifact_root: Path, paths: Mapping[str, Path]
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]]:
    generations: list[dict[str, Any]] = []
    baseline: dict[str, dict[str, Any]] = {}
    for profile in PROFILES:
        source = _profile_source(rows, profile, corpus)
        first = run_converter(source, artifact_root, paths, label=f"{profile.name}-run1")
        second = run_converter(source, artifact_root, paths, label=f"{profile.name}-run2")
        if first["status"] != "PASS" or second["status"] != "PASS":
            raise ReferenceError(f"{profile.name} black-box conversion failed: {first['status']}/{second['status']}")
        same = first["pdf_sha256"] == second["pdf_sha256"]
        if not same or first["pdf_sha256"] != profile.expected_pdf_sha256:
            raise ReferenceError(f"{profile.name} PDF is nondeterministic or differs from pinned SHA-256")
        metadata = pdf_metadata(Path(first["output_path"]), paths)
        if (metadata["page_count"], metadata["draw_count"]) != (
            profile.expected_pages,
            profile.expected_draws,
        ):
            raise ReferenceError(f"{profile.name} page/draw counts differ from pinned exploration")
        result = {
            "profile": profile.name,
            "source_id": profile.source_id,
            "source_sha256": next(row["sha256"] for row in rows if row["id"] == profile.source_id),
            "expected_pdf_sha256": profile.expected_pdf_sha256,
            "runs": [first, second],
            "deterministic": same,
            "page_count": metadata["page_count"],
            "draw_count": metadata["draw_count"],
            "pdf_metadata_resources": metadata["resources"],
        }
        if profile.name == "hn_b":
            result["source_page_mapping"] = hn_b_mapping(source, profile.source_id, metadata)
        generations.append(result)
        baseline[profile.name] = metadata
    return generations, baseline


def _source_page(source: Path, source_id: str, page_number: int) -> dict[str, Any]:
    from hnc8_layout_source import FileInput, SourceExtractor, SourceMetadataError

    try:
        with FileInput(source) as ranged:
            extractor = SourceExtractor(ranged, source_id)
            return extractor.read_page(page_number)
    except (OSError, SourceMetadataError) as exc:
        raise ReferenceError(f"source metadata extraction failed: {exc}") from exc


def _field_for_page(
    profile: Profile, page: dict[str, Any], name: str, byte_index: int = 0
) -> MutationField:
    suffixes = {"row_10": (10, 2), "row_12": (12, 4), "row_16": (16, 4)}
    if name not in suffixes:
        raise ReferenceError(f"unrecognized mutation field: {name}")
    relative, length = suffixes[name]
    if byte_index < 0 or byte_index >= length:
        raise ReferenceError("mutation byte index is outside its field")
    return MutationField(
        profile.name, page["page_number"], name, page["row_offset"] + relative, length, byte_index
    )


def plan_mutations(
    rows: list[dict[str, Any]], corpus: Path
) -> tuple[list[MutationField], list[dict[str, Any]]]:
    """Inspect unknown row fields; never mutate structural descriptors."""
    candidates: list[MutationField] = []
    exclusions: list[dict[str, Any]] = []
    for profile in (PROFILES[1], PROFILES[0]):
        source = _profile_source(rows, profile, corpus)
        page = _source_page(source, profile.source_id, profile.mutation_page)
        if page["image_count"] < 2:
            raise ReferenceError(f"{profile.name} mutation page no longer has extra images")
        candidates.extend(_field_for_page(profile, page, name) for name in ("row_10", "row_12", "row_16"))
        candidates.extend(
            _field_for_page(profile, page, name, index)
            for name, index in (("row_10", 1), ("row_12", 2), ("row_16", 2))
        )
        exclusions.append(
            {
                "profile": profile.name,
                "page_number": profile.mutation_page,
                "descriptor_fields": "excluded: type, payload offset and length are structural",
                "image_count": page["image_count"],
                "nonzero_descriptor_gaps": sum(image["gap_length"] > 0 for image in page["images"]),
                "text_span": "opaque; no numeric placement field isolated",
            }
        )
    return candidates, exclusions


def _different_positions(first: Path, second: Path) -> list[int]:
    if first.stat().st_size != second.stat().st_size:
        raise ReferenceError("mutation changed source length")
    changed: list[int] = []
    offset = 0
    with first.open("rb") as left, second.open("rb") as right:
        while block := left.read(READ_CHUNK):
            other = right.read(len(block))
            if len(other) != len(block):
                raise ReferenceError("mutation copy was truncated")
            if block != other:
                for i, (a, b) in enumerate(zip(block, other)):
                    if a != b:
                        changed.append(offset + i)
                        if len(changed) > 4:
                            raise ReferenceError("mutation changed more than four byte positions")
            offset += len(block)
    return changed


def copy_with_one_field_mutated(
    source: Path, artifact_root: Path, field: MutationField
) -> dict[str, Any]:
    directory = Path(tempfile.mkdtemp(prefix=f"mutate-{field.profile}-{field.name}-", dir=artifact_root))
    output = directory / "source-mutated.caj"
    size = source.stat().st_size
    if (
        field.absolute_offset < 0
        or field.length not in (2, 4)
        or not 0 <= field.byte_index < field.length
        or field.absolute_offset + field.length > size
    ):
        raise ReferenceError("mutation field is outside the checked source")
    with source.open("rb") as original, output.open("wb") as target:
        shutil.copyfileobj(original, target, READ_CHUNK)
    with source.open("rb") as original:
        original.seek(field.absolute_offset)
        before = original.read(field.length)
    if len(before) != field.length:
        raise ReferenceError("mutation field shortened during copying")
    after = (
        before[: field.byte_index]
        + bytes([before[field.byte_index] ^ 1])
        + before[field.byte_index + 1 :]
    )
    with output.open("r+b") as target:
        target.seek(field.absolute_offset)
        target.write(after)
    positions = _different_positions(source, output)
    if positions != [field.absolute_offset + field.byte_index]:
        raise ReferenceError("mutation did not change exactly its one intended byte")
    return {
        "source_path": str(output),
        "source_sha256": sha256_file(source),
        "mutated_source_sha256": sha256_file(output),
        "changed_byte_positions": positions,
        "field_span": [field.absolute_offset, field.length],
        "mutated_field_byte_index": field.byte_index,
        "before_field_sha256": hashlib.sha256(before).hexdigest(),
        "after_field_sha256": hashlib.sha256(after).hexdigest(),
    }


def _page_geometry(metadata: dict[str, Any], page_number: int) -> dict[str, Any] | None:
    if not 1 <= page_number <= metadata["page_count"]:
        return None
    page = metadata["pages"][page_number - 1]
    return {
        "media_box": page["media_box"],
        "draws": [
            {
                "draw_number": draw["draw_number"],
                "width": draw["width"],
                "height": draw["height"],
                "pdf_ctm": draw["pdf_ctm"],
            }
            for draw in page["draws"]
        ],
    }


def compare_geometry(
    baseline: dict[str, Any], mutant: dict[str, Any], page_number: int
) -> tuple[str, dict[str, Any], bool]:
    before = _page_geometry(baseline, page_number)
    after = _page_geometry(mutant, page_number)
    delta = {"page_number": page_number, "before": before, "after": after}
    if before is None or after is None or len(before["draws"]) != len(after["draws"]):
        return "PAGE_OR_DRAW_COUNT_CHANGED", delta, False
    if any(
        (a["width"], a["height"]) != (b["width"], b["height"])
        for a, b in zip(before["draws"], after["draws"])
    ):
        return "IMAGE_DIMENSIONS_CHANGED", delta, False
    if before == after:
        return "NO_GEOMETRY_CHANGE", delta, False
    # A candidate for extra-image x/y changes must leave first-image/page
    # geometry and every draw's linear scale intact, changing only an extra
    # draw's translation. This is a hypothesis filter, not a recovered rule.
    candidate = (
        before["media_box"] == after["media_box"]
        and before["draws"]
        and before["draws"][0]["pdf_ctm"] == after["draws"][0]["pdf_ctm"]
        and all(a["pdf_ctm"][:4] == b["pdf_ctm"][:4] for a, b in zip(before["draws"], after["draws"]))
        and any(a["pdf_ctm"][4:] != b["pdf_ctm"][4:] for a, b in zip(before["draws"][1:], after["draws"][1:]))
    )
    return "EXTRA_IMAGE_TRANSLATION_CHANGED" if candidate else "OTHER_GEOMETRY_CHANGE", delta, bool(candidate)


def run_mutation(
    profile: Profile,
    source: Path,
    field: MutationField,
    baseline: dict[str, Any],
    artifact_root: Path,
    paths: Mapping[str, Path],
) -> dict[str, Any]:
    copy = copy_with_one_field_mutated(source, artifact_root, field)
    converted = run_converter(
        Path(copy["source_path"]), artifact_root, paths,
        label=f"mutant-{profile.name}-{field.name}-b{field.byte_index}",
    )
    if sha256_file(Path(copy["source_path"])) != copy["mutated_source_sha256"]:
        raise ReferenceError("mutated temporary source changed during black-box conversion")
    result: dict[str, Any] = {
        "profile": profile.name,
        "source_id": profile.source_id,
        "page_number": field.page_number,
        "field": field.name,
        **copy,
        "conversion": converted,
        "candidate_survives": False,
        "geometry_delta": {
            "page_number": field.page_number,
            "before": _page_geometry(baseline, field.page_number),
            "after": None,
        },
    }
    if converted["status"] != "PASS":
        result["outcome"] = "CONVERSION_FAILED"
        return result
    if converted["pdf_sha256"] == baseline["pdf_sha256"]:
        result["outcome"] = "PDF_IDENTICAL"
        result["geometry_delta"]["after"] = result["geometry_delta"]["before"]
        result["geometry_delta"]["proof"] = "byte-identical PDF SHA-256"
        return result
    try:
        metadata = pdf_metadata(Path(converted["output_path"]), paths)
    except ReferenceError as exc:
        for name in ("qpdf", "mutool", "pdfimages", "pdfinfo"):
            _pin_file(name, paths[name])
        result["outcome"] = "PDF_METADATA_FAILED"
        result["pdf_metadata_failure"] = str(exc)
        return result
    result["output_page_count"] = metadata["page_count"]
    result["output_draw_count"] = metadata["draw_count"]
    outcome, delta, survives = compare_geometry(baseline, metadata, field.page_number)
    result["outcome"] = outcome
    result["geometry_delta"] = delta
    result["candidate_survives"] = survives
    return result


def _held_out_page(profile: Profile, source: Path) -> dict[str, Any] | None:
    from hnc8_layout_source import FileInput, SourceExtractor, SourceMetadataError

    try:
        with FileInput(source) as ranged:
            extractor = SourceExtractor(ranged, profile.source_id)
            for number in range(1, extractor.header["page_count"] + 1):
                if number == profile.mutation_page:
                    continue
                page = extractor.read_page(number)
                if page["image_count"] > 1:
                    return page
    except (OSError, SourceMetadataError) as exc:
        raise ReferenceError(f"held-out source measurement failed: {exc}") from exc
    return None


def run_perturbations(
    rows: list[dict[str, Any]],
    corpus: Path,
    artifact_root: Path,
    paths: Mapping[str, Path],
    baseline: Mapping[str, dict[str, Any]],
    maximum: int,
) -> dict[str, Any]:
    if maximum == 0:
        return {
            "status": "NOT_RUN",
            "attempted": 0,
            "planned_initial": 12,
            "tested_initial": 0,
            "candidate_survivors": 0,
            "placement_rule_status": "UNKNOWN_NOT_TESTED",
            "exclusions": [],
            "cases": [],
            "held_out": [],
        }
    fields, exclusions = plan_mutations(rows, corpus)
    profiles = {profile.name: profile for profile in PROFILES}
    cases: list[dict[str, Any]] = []
    survivors: list[MutationField] = []
    for field in fields[:maximum]:
        profile = profiles[field.profile]
        source = _profile_source(rows, profile, corpus)
        result = run_mutation(profile, source, field, baseline[profile.name], artifact_root, paths)
        cases.append(result)
        if result["candidate_survives"]:
            survivors.append(field)
    held_out: list[dict[str, Any]] = []
    for field in survivors:
        if len(cases) + len(held_out) >= maximum:
            break
        profile = profiles[field.profile]
        source = _profile_source(rows, profile, corpus)
        page = _held_out_page(profile, source)
        if page is None:
            held_out.append({"profile": profile.name, "field": field.name, "outcome": "NO_HELD_OUT_PAGE"})
            continue
        held_field = _field_for_page(profile, page, field.name, field.byte_index)
        result = run_mutation(profile, source, held_field, baseline[profile.name], artifact_root, paths)
        result["held_out_for"] = {
            "page_number": field.page_number,
            "field": field.name,
            "mutated_field_byte_index": field.byte_index,
        }
        held_out.append(result)
    incomplete = len(cases) < len(fields) or len(held_out) < len(survivors)
    if len(cases) < len(fields):
        rule_status = "UNKNOWN_PARTIAL_PROBE"
    elif not survivors:
        rule_status = "UNKNOWN_NO_SURVIVING_CANDIDATE_IN_TESTED_FIELDS"
    elif len(held_out) < len(survivors):
        rule_status = "UNKNOWN_HELD_OUT_BUDGET_EXHAUSTED"
    else:
        # An observed translation effect on two pages is still not a numeric
        # coordinate formula. Keep the compositor blocked until one is derived.
        rule_status = "UNKNOWN_NO_NUMERIC_RULE"
    return {
        "status": "PARTIAL" if incomplete else "PASS",
        "attempted": len(cases) + len(held_out),
        "planned_initial": len(fields),
        "tested_initial": len(cases),
        "candidate_survivors": len(survivors),
        "placement_rule_status": rule_status,
        "scope": (
            "one low-order and one higher-order byte flip in each unknown page-row "
            "field on C8 page 1 and HN-A page 16; no general placement rule inferred"
        ),
        "exclusions": exclusions,
        "cases": cases,
        "held_out": held_out,
    }


def _empty_report() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "protocol": "hnc8-layout-reference-v1",
        "status": "NOT_RUN",
        "matrix_sha256": MATRIX_SHA256,
        "source_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "environment_audit": {"status": "NOT_RUN"},
        "generations": [],
        "perturbations": {
            "status": "NOT_RUN",
            "attempted": 0,
            "planned_initial": 12,
            "tested_initial": 0,
            "candidate_survivors": 0,
            "placement_rule_status": "UNKNOWN_NOT_TESTED",
            "exclusions": [],
            "cases": [],
            "held_out": [],
        },
        "counts": {
            "source_checks_before": 0,
            "source_checks_after": 0,
            "pdf_generations": 0,
            "deterministic_profiles": 0,
            "existing_page_comparisons": 0,
            "existing_draw_comparisons": 0,
            "hn_b_page_comparisons": 0,
            "hn_b_draw_comparisons": 0,
            "perturbation_attempts": 0,
        },
        "resources": {
            "max_file_read_request_bytes": READ_CHUNK,
            "max_observed_session_bytes": 0,
            "retained_artifact_bytes_at_completion": 0,
            "temporary_sample_interval_milliseconds": 20,
            **_read_rss(),
        },
        "errors": [],
    }


def run(paths: Mapping[str, Path] | None, *, max_perturbations: int = 0) -> dict[str, Any]:
    report = _empty_report()
    if paths is None and max_perturbations == 0:
        return report
    if paths is None:
        report["status"] = "FAIL"
        report["errors"].append("perturbations were requested without external inputs")
        return report
    if not 0 <= max_perturbations <= MAX_PERTURBATIONS:
        report["status"] = "FAIL"
        report["errors"].append(f"max perturbations must be between zero and {MAX_PERTURBATIONS}")
        return report
    rows: list[dict[str, Any]] | None = None
    before: list[dict[str, Any]] | None = None
    environment: dict[str, Any] | None = None
    artifact_root: Path | None = None
    try:
        required = {
            "corpus", "reference_repo", "python", "pydeps", "libjbigdec",
            "artifact_root", "git", "qpdf", "mutool", "pdfinfo", "pdfimages",
        }
        if set(paths) != required:
            raise ReferenceError(f"external path set is incomplete: {sorted(required - set(paths))}")
        paths = {name: Path(path).expanduser().resolve() for name, path in paths.items()}
        matrix_hash, rows = load_source_rows()
        report["matrix_sha256"] = matrix_hash
        before = audit_sources(paths["corpus"], rows)
        report["source_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before), "after_checked": 0}
        report["counts"]["source_checks_before"] = len(before)
        artifact_root = paths["artifact_root"].resolve()
        if any(
            artifact_root.is_relative_to(path.resolve())
            for path in (ROOT, paths["corpus"], paths["reference_repo"])
        ):
            raise ReferenceError("artifact directory must be outside the repository, corpus, and reference checkout")
        artifact_root.mkdir(parents=True, exist_ok=True)
        artifact_root = Path(tempfile.mkdtemp(prefix="hnc8-layout-run-", dir=artifact_root))
        report["artifact_session_path"] = str(artifact_root)
        environment = audit_environment(paths)
        report["environment_audit"] = {"status": "BEFORE_PASS", **environment}
        generations, baseline = run_profiles(rows, paths["corpus"], artifact_root, paths)
        report["generations"] = generations
        report["counts"]["pdf_generations"] = len(generations) * 2
        report["counts"]["deterministic_profiles"] = len(generations)
        report["counts"]["existing_page_comparisons"] = sum(
            g["page_count"] for g in generations if g["profile"] != "hn_b"
        )
        report["counts"]["existing_draw_comparisons"] = sum(
            g["draw_count"] for g in generations if g["profile"] != "hn_b"
        )
        hn_b = next(g for g in generations if g["profile"] == "hn_b")
        report["counts"]["hn_b_page_comparisons"] = hn_b["page_count"]
        report["counts"]["hn_b_draw_comparisons"] = hn_b["draw_count"]
        perturbations = run_perturbations(
            rows, paths["corpus"], artifact_root, paths, baseline, max_perturbations
        )
        report["perturbations"] = perturbations
        report["counts"]["perturbation_attempts"] = perturbations["attempted"]
        report["status"] = "PASS" if perturbations["status"] == "PASS" else "PARTIAL"
    except (ReferenceError, OSError, ValueError) as exc:
        report["status"] = "FAIL"
        report["errors"].append(str(exc))
    finally:
        if rows is not None and before is not None:
            try:
                after = audit_sources(paths["corpus"], rows)
                if after != before:
                    raise ReferenceError("requested source matrix changed between audits")
                report["source_audit"] = {
                    "status": "PASS",
                    "before_checked": len(before),
                    "after_checked": len(after),
                    "sources": before,
                }
                report["counts"]["source_checks_after"] = len(after)
            except (ReferenceError, OSError) as exc:
                report["status"] = "FAIL"
                report["source_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run source audit: {exc}")
        if environment is not None:
            try:
                after_environment = audit_environment(paths)
                if after_environment != environment:
                    raise ReferenceError("reference environment changed between audits")
                report["environment_audit"]["status"] = "PASS"
            except (ReferenceError, OSError) as exc:
                report["status"] = "FAIL"
                report["environment_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run environment audit: {exc}")
        report["resources"].update(_read_rss())
        if artifact_root is not None and artifact_root.is_dir():
            retained = _tree_size(artifact_root)
            converted = [run for generation in report["generations"] for run in generation["runs"]]
            converted.extend(
                case["conversion"]
                for case in (report["perturbations"]["cases"] + report["perturbations"]["held_out"])
                if "conversion" in case
            )
            report["resources"]["retained_artifact_bytes_at_completion"] = retained
            report["resources"]["max_observed_session_bytes"] = max(
                [retained, *(run.get("max_observed_session_bytes", 0) for run in converted)]
            )
    return report


def _paths(args: argparse.Namespace) -> Mapping[str, Path] | None:
    external = (
        args.corpus_dir, args.reference_repo, args.python_bin, args.pydeps_dir,
        args.jbig_lib, args.artifact_dir,
    )
    if all(value is None for value in external):
        if any(value is not None for value in (args.git, args.qpdf, args.mutool, args.pdfinfo, args.pdfimages)):
            raise ReferenceError("an explicitly requested tool requires all six external path flags")
        return None
    if any(value is None for value in external):
        raise ReferenceError("all six external path flags are required when any one is supplied")
    defaults = {name: shutil.which(name) for name in ("git", "qpdf", "mutool", "pdfinfo", "pdfimages")}
    overrides = {
        "git": args.git, "qpdf": args.qpdf, "mutool": args.mutool,
        "pdfinfo": args.pdfinfo, "pdfimages": args.pdfimages,
    }
    resolved = {name: overrides[name] or defaults[name] for name in defaults}
    if any(path is None for path in resolved.values()):
        raise ReferenceError("one or more required PDF/Git tools are unavailable")
    return {
        "corpus": Path(args.corpus_dir),
        "reference_repo": Path(args.reference_repo),
        "python": Path(args.python_bin),
        "pydeps": Path(args.pydeps_dir),
        "libjbigdec": Path(args.jbig_lib),
        "artifact_root": Path(args.artifact_dir),
        **{name: Path(path) for name, path in resolved.items()},
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--reference-repo", type=Path)
    parser.add_argument("--python-bin", type=Path)
    parser.add_argument("--pydeps-dir", type=Path)
    parser.add_argument("--jbig-lib", type=Path)
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--git", type=Path)
    parser.add_argument("--qpdf", type=Path)
    parser.add_argument("--mutool", type=Path)
    parser.add_argument("--pdfinfo", type=Path)
    parser.add_argument("--pdfimages", type=Path)
    parser.add_argument("--max-perturbations", type=int, default=0)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        paths = _paths(args)
        report = run(paths, max_perturbations=args.max_perturbations)
    except ReferenceError as exc:
        report = _empty_report()
        report["status"] = "FAIL"
        report["errors"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        counts = report["counts"]
        print(
            f"HN/C8 layout reference [{report['status']}]: "
            f"source audits {counts['source_checks_before']}/{counts['source_checks_after']}, "
            f"PDF pages/draws {counts['existing_page_comparisons']}/{counts['existing_draw_comparisons']}, "
            f"perturbations {counts['perturbation_attempts']}"
        )
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, finite T.82 BIE probe for a pinned HN/C8 type-0 image.

This script uses caller-supplied standard CLI binaries only as black boxes.
It does not use their code or ship a decoder. A match for an all-zero image is
non-discriminating; a mismatch excludes only the finite settings tested here.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import sys
import tempfile

import conformance
import jbig1_oracle as oracle


DEFAULT_MATRIX = oracle.DEFAULT_MATRIX
DEFAULT_MANIFEST = oracle.DEFAULT_MANIFEST
MAX_CODED_BYTES = 128 * 1024
MAX_BIE_BYTES = 2 * MAX_CODED_BYTES + 22
MAX_STDOUT_BYTES = 16 * 1024 * 1024
PBM_HEADER = re.compile(rb"P4\n *([0-9]+)\n *([0-9]+)\n")
OPTIONS = (0, 8, 64, 72)
ORDERS = (0, 3)
TERMINATORS = (b"\xff\x02", b"\xff\x03")


class ProbeError(Exception):
    """A requested probe cannot be carried out safely or reproducibly."""


def require_cli(path: Path, label: str) -> tuple[Path, str]:
    try:
        resolved = path.resolve(strict=True)
    except OSError as exc:
        raise ProbeError(f"requested {label} is missing: {path}") from exc
    if not resolved.is_file() or not os.access(resolved, os.X_OK):
        raise ProbeError(f"requested {label} is not an executable file: {path}")
    return resolved, oracle.sha256_file(resolved)


def selected_image(
    corpus_dir: Path, matrix_path: Path, manifest_path: Path,
    sample_id: str, page: int, image_number: int,
) -> tuple[dict, bytes]:
    matrix = conformance.load_matrix(matrix_path)
    manifest = oracle.load_manifest(manifest_path)
    if oracle.sha256_file(matrix_path) != manifest["corpus_matrix_sha256"]:
        raise ProbeError("corpus matrix SHA-256 differs from the oracle manifest")
    rows = {row["id"]: row for row in matrix if row["detected_type"] in ("HN", "C8")}
    samples = {sample["id"]: sample for sample in manifest["samples"]}
    if sample_id not in rows or sample_id not in samples:
        raise ProbeError(f"sample ID is absent from the HN/C8 matrix or manifest: {sample_id}")
    row, sample = rows[sample_id], samples[sample_id]
    if sample["path"] != row["path"] or sample["source_sha256"] != row["sha256"]:
        raise ProbeError("sample path or source SHA-256 differs between matrix and manifest")
    matches = [
        image for image in sample["images"]
        if (image["page"], image["image"]) == (page, image_number)
    ]
    if len(matches) != 1 or matches[0]["decoder_result"] != "PASS":
        raise ProbeError("selected page/image has no PASS pixel oracle in the manifest")
    image = matches[0]
    try:
        root = corpus_dir.resolve(strict=True)
    except OSError as exc:
        raise ProbeError(f"requested corpus directory is missing: {corpus_dir}") from exc
    if not root.is_dir():
        raise ProbeError(f"requested corpus directory is not a directory: {corpus_dir}")
    source = conformance.contained_file(root, conformance.relative_path(sample["path"]))
    if source.stat().st_size != row["size_bytes"]:
        raise ProbeError("source size differs from the matrix")
    git_blob, source_sha = conformance.file_hashes(source, row["size_bytes"], True)
    if git_blob != row["git_blob_oid"] or source_sha != sample["source_sha256"]:
        raise ProbeError("source bytes differ from the matrix or manifest hashes")
    if image["length"] <= 48 or image["length"] - 48 > MAX_CODED_BYTES:
        raise ProbeError("selected coded image exceeds the finite probe input limit")
    with source.open("rb") as stream:
        encoded = oracle.read_exact(
            stream, image["offset"], image["length"], row["size_bytes"], "selected image"
        )
    if hashlib.sha256(encoded).hexdigest() != image["encoded_sha256"]:
        raise ProbeError("selected image bytes differ from the manifest encoded SHA-256")
    dimensions = oracle.validate_image_dimensions(
        encoded[:oracle.DIB_AND_PALETTE_BYTES], oracle.DEFAULT_MAX_BITMAP_BYTES
    )
    if dimensions != (image["width"], image["height"], image["stride"]):
        raise ProbeError("selected DIB dimensions differ from the manifest")
    return image, encoded[48:]


def pbm_candidate_hashes(
    bitmap: bytes, width: int, height: int, stride: int
) -> tuple[dict[str, dict[str, str]], int]:
    """Hash both row orders; hypothesize zero bytes for absent DIB row padding.

    PBM carries the unused low bits in its final byte, but has no DIB stride
    padding. Visible hashes mask those bits; raw hashes retain them and append
    candidate zero padding. Neither operation changes the supplied bitmap.
    """
    row_bytes = (width + 7) // 8
    if len(bitmap) != row_bytes * height or stride != ((width + 31) // 32) * 4:
        raise ProbeError("standard CLI returned an incomplete PBM bitmap")
    mask = 0xFF if width % 8 == 0 else (0xFF << (8 - width % 8)) & 0xFF
    padding = bytes(stride - row_bytes)
    pixels = memoryview(bitmap)
    hashes = {}
    one_bits = 0
    for orientation, rows in (
        ("direct", range(height)),
        ("reversed_rows", range(height - 1, -1, -1)),
    ):
        visible_digest = hashlib.sha256()
        raw_digest = hashlib.sha256()
        for row in rows:
            start = row * row_bytes
            row_pixels = pixels[start : start + row_bytes]
            last = row_pixels[-1] & mask
            visible_digest.update(row_pixels[:-1])
            visible_digest.update(bytes((last,)))
            raw_digest.update(row_pixels)
            raw_digest.update(padding)
            if orientation == "direct":
                one_bits += sum(byte.bit_count() for byte in row_pixels[:-1]) + last.bit_count()
        hashes[orientation] = {
            "visible_bits_sha256": visible_digest.hexdigest(),
            "raw_stride_sha256": raw_digest.hexdigest(),
        }
    return hashes, one_bits


def standard_bie(width: int, height: int, l0: int, order: int, options: int,
                 coded: bytes, stuff_ff: bool, terminator: bytes) -> bytes:
    header = (
        bytes((0, 0, 1, 0))
        + width.to_bytes(4, "big")
        + height.to_bytes(4, "big")
        + l0.to_bytes(4, "big")
        + bytes((0, 0, order, options))
    )
    payload = coded.replace(b"\xff", b"\xff\x00") if stuff_ff else coded
    bie = header + payload + terminator
    if len(bie) > MAX_BIE_BYTES:
        raise ProbeError("constructed BIE exceeds the finite probe input limit")
    return bie


def call_cli(
    binary: Path, arguments: list[str], data: bytes, timeout: float, output_limit: int
) -> tuple[int | None, bytes | None]:
    """Bound child output on disk and load only a checked byte count."""
    if len(data) > MAX_STDOUT_BYTES:
        raise ProbeError("CLI input exceeds the bounded probe limit")
    if not 0 < output_limit <= MAX_STDOUT_BYTES:
        raise ProbeError("CLI output limit is outside the bounded probe range")
    try:
        with tempfile.TemporaryFile() as output:
            soft_limit, hard_limit = resource.getrlimit(resource.RLIMIT_FSIZE)
            child_limit = output_limit + 1
            if soft_limit != resource.RLIM_INFINITY:
                child_limit = min(child_limit, soft_limit)
            if hard_limit != resource.RLIM_INFINITY:
                child_limit = min(child_limit, hard_limit)
            completed = subprocess.run(
                [str(binary), *arguments], input=data, stdout=output,
                stderr=subprocess.DEVNULL, timeout=timeout, check=False,
                preexec_fn=lambda: resource.setrlimit(
                    resource.RLIMIT_FSIZE, (child_limit, hard_limit)
                ),
            )
            size = output.seek(0, os.SEEK_END)
            if size > output_limit:
                return completed.returncode, None
            output.seek(0)
            return completed.returncode, output.read(size)
    except subprocess.TimeoutExpired:
        return None, None
    except OSError as exc:
        raise ProbeError(f"external CLI could not start or spool output: {exc}") from exc


def decoder_probe(decoder: Path, coded: bytes, image: dict, timeout: float) -> dict:
    width, height = image["width"], image["height"]
    row_bytes = (width + 7) // 8
    output_limit = row_bytes * height + 1024
    if output_limit > MAX_STDOUT_BYTES:
        raise ProbeError("selected PBM output exceeds the bounded probe limit")
    zero_digest = hashlib.sha256()
    zero_chunk = bytes(8192)
    zero_size = row_bytes * height
    for _ in range(zero_size // len(zero_chunk)):
        zero_digest.update(zero_chunk)
    zero_digest.update(zero_chunk[:zero_size % len(zero_chunk)])
    zero_visible_hash = zero_digest.hexdigest()
    blank_reference = image["visible_bits_sha256"] == zero_visible_hash
    results = []
    for l0 in (height, 128):
        for options in OPTIONS:
            for order in ORDERS:
                for stuff_ff in (False, True):
                    for terminator in TERMINATORS:
                        bie = standard_bie(width, height, l0, order, options,
                                           coded, stuff_ff, terminator)
                        settings = {
                            "l0": l0, "options": options, "order": order,
                            "stuff_ff": stuff_ff, "terminator": terminator.hex(),
                        }
                        result = {"settings": settings, "bie_sha256": hashlib.sha256(bie).hexdigest()}
                        code, output = call_cli(
                            decoder, ["-"], bie, timeout, output_limit,
                        )
                        if code is None:
                            result["status"] = "TIMEOUT"
                        elif output is None:
                            result["status"] = "BAD_PBM"
                        elif code != 0:
                            result.update(status="DECODE_ERROR", exit_code=code)
                        else:
                            match = PBM_HEADER.match(output)
                            if (
                                match is None
                                or len(match[1]) > 10
                                or len(match[2]) > 10
                                or (int(match[1]), int(match[2])) != (width, height)
                            ):
                                result["status"] = "BAD_PBM"
                            else:
                                try:
                                    candidate_hashes, one_bits = pbm_candidate_hashes(
                                        output[match.end():], width, height, image["stride"]
                                    )
                                except ProbeError:
                                    result["status"] = "BAD_PBM"
                                else:
                                    result.update(candidate_hashes=candidate_hashes, one_bits=one_bits)
                                    visible_matches = [
                                        orientation for orientation, hashes in candidate_hashes.items()
                                        if hashes["visible_bits_sha256"] == image["visible_bits_sha256"]
                                    ]
                                    exact_matches = [
                                        orientation for orientation in visible_matches
                                        if candidate_hashes[orientation]["raw_stride_sha256"]
                                        == image["raw_stride_sha256"]
                                    ]
                                    if exact_matches:
                                        result["matching_orientations"] = exact_matches
                                        result["status"] = (
                                            "MATCH_NON_DISCRIMINATING" if blank_reference else "MATCH"
                                        )
                                    elif visible_matches:
                                        result["visible_matching_orientations"] = visible_matches
                                        result["status"] = "VISIBLE_MATCH_RAW_MISMATCH"
                                    else:
                                        result["status"] = "HASH_MISMATCH"
                        results.append(result)
    counts: dict[str, int] = {}
    for result in results:
        counts[result["status"]] = counts.get(result["status"], 0) + 1
    if counts.get("TIMEOUT", 0) or counts.get("BAD_PBM", 0):
        finding = "FAIL"
    elif counts.get("MATCH", 0):
        finding = "MATCH_IN_TESTED_GRID"
    elif counts.get("MATCH_NON_DISCRIMINATING", 0):
        finding = "BLANK_MATCH_NON_DISCRIMINATING"
    elif counts.get("HASH_MISMATCH", 0) + counts.get("VISIBLE_MATCH_RAW_MISMATCH", 0) == 0:
        finding = "INCONCLUSIVE_NO_DECODABLE_SETTINGS"
    else:
        finding = "NO_MATCH_IN_TESTED_GRID"
    return {
        "status": finding,
        "scope": "finite BIH/stripe hypotheses only; no conclusion about every T.82 parameter",
        "blank_reference": blank_reference,
        "settings_count": len(results),
        "status_counts": counts,
        "results": results,
    }


def blank_control(encoder: Path, coded: bytes, width: int, height: int,
                  timeout: float) -> dict:
    """Encode an independently authored blank P4 at the selected dimensions."""
    pbm = f"P4\n{width} {height}\n".encode("ascii") + bytes(((width + 7) // 8) * height)
    arguments = ["-q", "-p", "0", "-m", "0", "-s", str(height), "-o", "0", "-"]
    code, bie = call_cli(encoder, arguments, pbm, timeout, MAX_STDOUT_BYTES)
    if code is None:
        raise ProbeError(f"requested standard encoder timed out after {timeout}s")
    if bie is None:
        raise ProbeError("standard encoder output exceeds the bounded probe limit")
    if code != 0:
        raise ProbeError(f"requested standard encoder exited {code}")
    if len(bie) < 22 or len(bie) > MAX_STDOUT_BYTES or not bie.endswith(b"\xff\x02"):
        raise ProbeError("standard encoder returned an invalid single-stripe BIE")
    expected_header = (
        b"\x00\x00\x01\x00" + width.to_bytes(4, "big")
        + height.to_bytes(4, "big") + height.to_bytes(4, "big") + bytes(4)
    )
    if bie[:20] != expected_header:
        raise ProbeError("standard encoder BIE header differs from the blank control settings")
    scd = bie[20:-2]
    if not scd or b"\xff" in scd:
        raise ProbeError("blank control unexpectedly contains a protected SCD escape")
    suffix = coded[len(scd):] if coded.startswith(scd) else None
    zero_tail = suffix is not None and all(byte == 0 for byte in suffix)
    return {
        "status": "COMPLETE",
        "flags": arguments,
        "pbm_sha256": hashlib.sha256(pbm).hexdigest(),
        "bie_sha256": hashlib.sha256(bie).hexdigest(),
        "bie_length": len(bie),
        "scd_sha256": hashlib.sha256(scd).hexdigest(),
        "scd_length": len(scd),
        "selected_coded_is_scd_plus_zero_tail": zero_tail,
        "selected_coded_zero_tail_bytes": len(suffix) if zero_tail else None,
        "interpretation": "blank SCD consistency does not identify a nonblank context model",
    }


def run(
    corpus_dir: Path, decoder_path: Path, sample_id: str, page: int,
    image_number: int = 1, encoder_path: Path | None = None,
    matrix_path: Path = DEFAULT_MATRIX, manifest_path: Path = DEFAULT_MANIFEST,
    timeout_seconds: float = 5.0,
) -> dict:
    if page <= 0 or image_number <= 0 or not 0 < timeout_seconds <= 30:
        raise ProbeError("page, image, and timeout must be positive and bounded")
    decoder, decoder_sha = require_cli(decoder_path, "standard decoder")
    encoder_info = require_cli(encoder_path, "standard encoder") if encoder_path else None
    image, coded = selected_image(
        corpus_dir, matrix_path, manifest_path, sample_id, page, image_number
    )
    grid = decoder_probe(decoder, coded, image, timeout_seconds)
    report = {
        "schema_version": 1,
        "status": grid["status"],
        "sample_id": sample_id,
        "page": page,
        "image": image_number,
        "image_offset": image["offset"],
        "image_length": image["length"],
        "encoded_sha256": image["encoded_sha256"],
        "coded_length": len(coded),
        "coded_sha256": hashlib.sha256(coded).hexdigest(),
        "width": image["width"],
        "height": image["height"],
        "expected_raw_stride_sha256": image["raw_stride_sha256"],
        "expected_visible_bits_sha256": image["visible_bits_sha256"],
        "decoder_sha256": decoder_sha,
        "decoder_probe": grid,
        "blank_control": {"status": "NOT_RUN", "reason": "standard encoder was not requested"},
    }
    if encoder_info is not None:
        encoder, encoder_sha = encoder_info
        report["encoder_sha256"] = encoder_sha
        report["blank_control"] = blank_control(
            encoder, coded, image["width"], image["height"], timeout_seconds
        )
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--decoder", type=Path, help="external jbgtopbm executable")
    parser.add_argument("--encoder", type=Path, help="optional external pbmtojbg blank control")
    parser.add_argument("--sample-id")
    parser.add_argument("--page", type=int)
    parser.add_argument("--image", type=int, default=1)
    parser.add_argument("--matrix", type=Path, default=DEFAULT_MATRIX)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--timeout-seconds", type=float, default=5.0)
    parser.add_argument("--json", action="store_true")
    raw_arguments = list(sys.argv[1:] if argv is None else argv)
    options = parser.parse_args(raw_arguments)
    requested = any(argument != "--json" for argument in raw_arguments)
    if not requested:
        report = {"status": "NOT_RUN", "reason": "external corpus and standard CLI were not requested"}
        print(json.dumps(report) if options.json else "T.82 standard probe [NOT_RUN]: external inputs unset")
        return 0
    try:
        if any(value is None for value in (
            options.corpus_dir, options.decoder, options.sample_id, options.page
        )):
            raise ProbeError("requested probe needs corpus-dir, decoder, sample-id, and page")
        report = run(
            options.corpus_dir, options.decoder, options.sample_id,
            options.page, options.image, options.encoder,
            options.matrix, options.manifest, options.timeout_seconds,
        )
    except (ProbeError, oracle.OracleError, conformance.ConformanceError, OSError) as exc:
        report = {"status": "FAIL", "reason": str(exc)}
        print(json.dumps(report) if options.json else f"T.82 standard probe [FAIL]: {exc}")
        return 1
    if options.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True))
    else:
        print(
            f"T.82 standard probe [{report['status']}]: {report['sample_id']} "
            f"page {report['page']} image {report['image']}; "
            f"{report['decoder_probe']['settings_count']} finite settings, "
            f"{report['decoder_probe']['status_counts']}"
        )
        print(f"Blank control: {report['blank_control']['status']}")
    return int(report["status"] in ("FAIL", "INCONCLUSIVE_NO_DECODABLE_SETTINGS"))


if __name__ == "__main__":
    raise SystemExit(main())

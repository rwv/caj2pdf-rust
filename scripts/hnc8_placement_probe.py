#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, pinned JFIF placement probes for issue #110.

Only the four edits predeclared in docs/hnc8-placement-experiments.md are
allowed. The external converter is executed as a black box. Source documents,
mutated copies and PDFs stay outside the repository; reports contain metadata
and SHA-256 values only. A clean clone performs no private comparison.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from typing import Any, Mapping

import hnc8_layout_reference as reference
from hnc8_layout_source import FileInput, SourceExtractor, SourceMetadataError


ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / "tests/conformance/hnc8_layout_oracle.json"
ORACLE_SHA256 = "4b88befeecf9a68dd6eca4966c79ea8cdb130c43e3c6d92f4cb56fb34dfb665e"
MAX_REPORT_BYTES = 8 * 1024 * 1024
COPY_CHUNK = 64 * 1024
JFIF_HEADER = bytes.fromhex("ffd8ffe000104a46494600010100000100010000")


class ProbeError(ValueError):
    """A declared probe or requested input failed a checked precondition."""


@dataclass(frozen=True)
class Probe:
    name: str
    profile: str
    page: int
    image: int
    payload_offset: int
    field: str
    field_offset: int
    field_width: int
    changed_offset: int
    before: int
    after: int


PROBES = (
    Probe("c8-units", "c8", 1, 2, 94180, "JFIF APP0 units", 94193, 1, 94193, 0, 1),
    Probe("c8-xdensity", "c8", 1, 2, 94180, "JFIF APP0 Xdensity", 94194, 2, 94195, 1, 2),
    Probe("hn-a-units", "hn_a", 16, 2, 1004167, "JFIF APP0 units", 1004180, 1, 1004180, 0, 1),
    Probe("hn-a-xdensity", "hn_a", 16, 2, 1004167, "JFIF APP0 Xdensity", 1004181, 2, 1004182, 1, 2),
)


def _report() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "protocol": "hnc8-placement-jfif-batch-v1",
        "status": "NOT_RUN",
        "placement_rule_status": "UNKNOWN_NOT_TESTED",
        "status_semantics": (
            "PASS means all four declared probes completed with audited, repeatable, "
            "independently parsed PDF outcomes; "
            "a converter rejection is recorded as CONVERSION_FAILED and never counts as "
            "placement evidence. PARTIAL means at least one conversion or independent "
            "PDF metadata check failed."
        ),
        "count_semantics": (
            "attempted counts probes at launch; completed counts probes with two recorded runs; "
            "probe_runs counts returned run records from completed probes, not all "
            "converter launches; "
            "passing means repeatable, independently parsed PDF outcomes; failing includes "
            "converter rejection, unsupported PDF metadata, nondeterminism, or probe protocol errors; "
            "skipped counts unstarted probes"
        ),
        "matrix_sha256": reference.MATRIX_SHA256,
        "layout_oracle_sha256": ORACLE_SHA256,
        "reference_revision": reference.REFERENCE_REVISION,
        "source_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "environment_audit": {"status": "NOT_RUN"},
        "input_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "counts": {
            "private_source_checks_before": 0,
            "private_source_checks_after": 0,
            "baseline_pdf_checks_before": 0,
            "baseline_pdf_checks_after": 0,
            "baseline_pdf_metadata_compared": 0,
            "probes_planned": len(PROBES),
            "probes_attempted": 0,
            "probes_completed": 0,
            "probes_passing": 0,
            "probes_failing": 0,
            "probe_runs": 0,
            "repeatable_probes": 0,
            "pdf_changed_probes": 0,
            "placement_changed_probes": 0,
            "passing_causal_probes": 0,
            "conversion_failed_probes": 0,
            "unsupported_probes": 0,
            "skipped_probes": 0,
        },
        "probes": [],
        "resources": {
            "max_ranged_request_bytes": 0,
            "max_source_hash_read_request_bytes": 0,
            "mutation_copy_chunk_bytes": COPY_CHUNK,
            "max_observed_child_vmhwm_kib": 0,
            "max_pdf_tool_output_bytes": 0,
            "max_pdf_tool_child_rss_kib": 0,
            "max_observed_session_bytes": 0,
            "retained_artifact_bytes_at_completion": 0,
            "harness_vmhwm_kib": 0,
            "temporary_sample_interval_milliseconds": 20,
            "converter_timeout_seconds": reference.CONVERTER_TIMEOUT_SECONDS,
        },
        "errors": [],
    }


def _file_digest(path: Path, *, limit: int | None = None) -> str:
    if limit is not None and path.stat().st_size > limit:
        raise ProbeError("requested metadata input exceeds size limit")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(COPY_CHUNK):
            digest.update(block)
    return digest.hexdigest()


def _external_file(path: Path, roots: tuple[Path, ...], label: str) -> Path:
    path = path.expanduser().resolve(strict=True)
    if not path.is_file() or any(path.is_relative_to(root) for root in roots):
        raise ProbeError(f"{label} must be a regular file outside protected roots")
    return path


def _environment_record(environment: dict, status: str) -> dict:
    """Carry the full pinned black-box protocol into the external report."""
    required = {"reference_revision", "reference_clean", "pypdf2", "binaries",
                "native_compiler_record", "environment", "invocation", "timeout_seconds"}
    if set(environment) != required or set(environment["binaries"]) != set(reference.PINNED_HASHES):
        raise ProbeError("reference environment audit lacks required pinned facts")
    return {"status": status, **environment}


def _load_oracle() -> dict[str, dict]:
    digest = hashlib.sha256()
    data = bytearray()
    try:
        with ORACLE.open("rb") as source:
            while block := source.read(COPY_CHUNK):
                data.extend(block)
                if len(data) > MAX_REPORT_BYTES:
                    raise ProbeError("committed #107 layout oracle exceeds size limit")
                digest.update(block)
    except OSError as exc:
        raise ProbeError("committed #107 layout oracle is unavailable") from exc
    if digest.hexdigest() != ORACLE_SHA256:
        raise ProbeError("committed #107 layout oracle differs from pinned SHA-256")
    try:
        document = json.loads(data)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise ProbeError("committed #107 layout oracle is malformed") from exc
    if (document.get("schema_version") != 1 or
            document.get("matrix_sha256") != reference.MATRIX_SHA256 or
            document.get("pdf_tool_hashes") != {
                name: reference.PINNED_HASHES[name]
                for name in ("qpdf", "mutool", "pdfimages")
            }):
        raise ProbeError("committed #107 oracle metadata differs from pinned protocol")
    cases = {case["case"]: case for case in document["cases"]}
    if set(cases) != {"hn_a", "c8", "hn_b"}:
        raise ProbeError("committed #107 oracle has unexpected cases")
    for profile in reference.PROFILES:
        case = cases[profile.name]
        if case["source_id"] != profile.source_id or case["pdf_sha256"] != profile.expected_pdf_sha256:
            raise ProbeError("committed #107 oracle source/PDF identity differs")
    return cases


def _read_reference_report(path: Path, rows: list[dict], environment: dict,
                           source_audit: list[dict], roots: tuple[Path, ...]) -> tuple[dict, dict[str, Path]]:
    path = _external_file(path, roots, "reference report")
    if path.stat().st_size > MAX_REPORT_BYTES:
        raise ProbeError("reference report exceeds size limit")
    with path.open("r", encoding="utf-8") as source:
        report = json.load(source)
    if (report.get("schema_version") != 1 or
            report.get("protocol") != "hnc8-layout-reference-v1" or
            report.get("status") != "PASS" or
            report.get("matrix_sha256") != reference.MATRIX_SHA256 or
            report.get("source_audit", {}).get("status") != "PASS" or
            report.get("source_audit", {}).get("sources") != source_audit or
            report.get("environment_audit") != {"status": "PASS", **environment}):
        raise ProbeError("reference report is not the pinned completed #107 run")
    generations = report.get("generations")
    if not isinstance(generations, list) or [g.get("profile") for g in generations] != [
            profile.name for profile in reference.PROFILES]:
        raise ProbeError("reference report baseline profile order differs")
    row_hashes = {row["id"]: row["sha256"] for row in rows}
    baseline_pdfs = {}
    for generation, profile in zip(generations, reference.PROFILES):
        if (generation.get("source_id") != profile.source_id or
                generation.get("source_sha256") != row_hashes[profile.source_id] or
                generation.get("expected_pdf_sha256") != profile.expected_pdf_sha256 or
                generation.get("page_count") != profile.expected_pages or
                generation.get("draw_count") != profile.expected_draws or
                generation.get("deterministic") is not True):
            raise ProbeError("reference report baseline metadata differs")
        runs = generation.get("runs")
        if not isinstance(runs, list) or len(runs) != 2:
            raise ProbeError("reference report must contain two baseline runs per profile")
        for index, run in enumerate(runs):
            if (run.get("status") != "PASS" or run.get("timed_out") is not False or
                    run.get("pdf_sha256") != profile.expected_pdf_sha256):
                raise ProbeError("reference report baseline PDF run differs")
            pdf = _external_file(Path(run["output_path"]), roots, "baseline PDF")
            if _file_digest(pdf, limit=reference.MAX_PDF_BYTES) != profile.expected_pdf_sha256:
                raise ProbeError("baseline PDF SHA-256 differs from pinned #107 PDF")
            baseline_pdfs[f"{profile.name}-run{index + 1}"] = pdf
    if len(set(baseline_pdfs.values())) != len(baseline_pdfs):
        raise ProbeError("reference report reuses a baseline PDF path across runs")
    return report, baseline_pdfs


def _checked_page(source: Path, source_id: str, page_number: int) -> tuple[dict, int]:
    try:
        with FileInput(source) as ranged:
            extractor = SourceExtractor(ranged, source_id)
            page = extractor.read_page(page_number)
            return page, extractor.max_request_bytes
    except (OSError, SourceMetadataError) as exc:
        raise ProbeError("source page failed independent bounded structure check") from exc


def _jpeg_header(source: Path, image: dict, probe: Probe,
                 *, mutated: bool) -> tuple[int, int, int]:
    """Check JFIF, framed SOF0 and SOS, and a terminal EOI without decoding pixels."""
    start = image["payload_offset"]
    end = start + image["payload_length"]
    if image["payload_length"] < len(JFIF_HEADER) + 4:
        raise ProbeError("probe JPEG is shorter than JFIF and EOI")
    with FileInput(source) as ranged:
        expected_header = bytearray(JFIF_HEADER)
        if mutated:
            expected_header[probe.changed_offset - start] = probe.after
        if ranged.read_at(start, len(JFIF_HEADER)) != bytes(expected_header):
            raise ProbeError("probe JPEG is not the predeclared JFIF 1.01 APP0 header")
        if ranged.read_at(end - 2, 2) != b"\xff\xd9":
            raise ProbeError("probe JPEG has no terminal EOI marker")
        at = start + 2
        marker_count = 0
        sof = None
        while at < end:
            if marker_count >= 4096 or at - start > 1024 * 1024:
                raise ProbeError("probe JPEG marker scan exceeds bound")
            head = ranged.read_at(at, 2)
            if len(head) != 2 or head[0] != 0xff or head[1] in (0x00, 0xff):
                raise ProbeError("probe JPEG has an invalid marker before SOS")
            marker = head[1]
            if marker in (0x01, 0xd8, 0xd9) or 0xd0 <= marker <= 0xd7:
                raise ProbeError("probe JPEG has a nonsegment marker before SOS")
            length_bytes = ranged.read_at(at + 2, 2)
            if len(length_bytes) != 2:
                raise ProbeError("probe JPEG segment length is truncated")
            length = int.from_bytes(length_bytes, "big")
            next_at = at + 2 + length
            if length < 2 or next_at > end or next_at - start > 1024 * 1024:
                raise ProbeError("probe JPEG segment is outside the bounded header")
            if marker == 0xc0:
                frame = ranged.read_at(at + 4, 6)
                if len(frame) != 6 or frame[0] != 8:
                    raise ProbeError("probe JPEG SOF0 is malformed")
                sof = (int.from_bytes(frame[3:5], "big"), int.from_bytes(frame[1:3], "big"))
                if sof != (image["width"], image["height"]):
                    raise ProbeError("probe JPEG SOF0 dimensions differ from source descriptor")
            if marker == 0xda:
                if sof is None or next_at >= end - 2:
                    raise ProbeError("probe JPEG SOS lacks SOF0 or entropy bytes")
                return sof[0], sof[1], next_at
            at = next_at
            marker_count += 1
    raise ProbeError("probe JPEG has no SOS marker")


def _check_probe_source(source: Path, source_id: str, probe: Probe,
                        oracle_page: dict, *, mutated: bool) -> tuple[dict, int]:
    page, request = _checked_page(source, source_id, probe.page)
    if len(page["images"]) < probe.image:
        raise ProbeError("probe source page lacks declared image")
    image = page["images"][probe.image - 1]
    pinned = oracle_page["images"][probe.image - 1]
    if (image["image_number"] != probe.image or image["record_type"] != 2 or
            image["payload_offset"] != probe.payload_offset or
            image["payload_length"] != pinned["payload_length"] or
            (image["width"], image["height"]) != (pinned["width"], pinned["height"])):
        raise ProbeError("probe image descriptor differs from predeclared #107 metadata")
    if not mutated and image["payload_sha256"] != pinned["payload_sha256"]:
        raise ProbeError("probe source image hash differs from #107 oracle")
    if (probe.field_offset, probe.changed_offset) != (
            probe.payload_offset + (13 if probe.field == "JFIF APP0 units" else 14),
            probe.payload_offset + (13 if probe.field == "JFIF APP0 units" else 15)):
        raise ProbeError("probe field offset differs from JFIF APP0 structure")
    width, height, sos_offset = _jpeg_header(source, image, probe, mutated=mutated)
    return {"page": page, "image": image, "jpeg_width": width, "jpeg_height": height,
            "sos_offset": sos_offset}, request


def _different_positions(left: Path, right: Path) -> list[int]:
    if left.stat().st_size != right.stat().st_size:
        raise ProbeError("mutated copy differs in source length")
    positions = []
    offset = 0
    with left.open("rb") as original, right.open("rb") as mutated:
        while block := original.read(COPY_CHUNK):
            other = mutated.read(len(block))
            if len(other) != len(block):
                raise ProbeError("mutated copy was truncated")
            for index, (a, b) in enumerate(zip(block, other)):
                if a != b:
                    positions.append(offset + index)
                    if len(positions) > 1:
                        raise ProbeError("mutated copy changed more than one byte")
            offset += len(block)
    return positions


def copy_probe(source: Path, directory: Path, probe: Probe) -> dict[str, Any]:
    """Write one temporary source copy and prove the exact one-byte diff."""
    directory.mkdir(parents=False, exist_ok=False)
    target = directory / "source-mutated.caj"
    if probe.changed_offset < 0 or probe.changed_offset >= source.stat().st_size:
        raise ProbeError("declared changed byte is outside source")
    original_hash = hashlib.sha256()
    changed_hash = hashlib.sha256()
    with source.open("rb") as original, target.open("xb") as output:
        offset = 0
        while block := original.read(COPY_CHUNK):
            original_hash.update(block)
            if offset <= probe.changed_offset < offset + len(block):
                index = probe.changed_offset - offset
                if block[index] != probe.before:
                    raise ProbeError("declared JFIF field has unexpected original value")
                copy = bytearray(block)
                copy[index] = probe.after
                block = bytes(copy)
            output.write(block)
            changed_hash.update(block)
            offset += len(block)
    positions = _different_positions(source, target)
    if positions != [probe.changed_offset] or original_hash.hexdigest() == changed_hash.hexdigest():
        raise ProbeError("temporary source does not have the exact declared one-byte change")
    if (_file_digest(source) != original_hash.hexdigest() or
            _file_digest(target) != changed_hash.hexdigest()):
        raise ProbeError("source changed during checked mutation copy")
    return {
        "path": target,
        "source_sha256": original_hash.hexdigest(),
        "mutated_source_sha256": changed_hash.hexdigest(),
        "changed_byte_positions": positions,
    }


def _draw_identity(draw: dict) -> dict:
    return {name: draw[name] for name in (
        "draw_number", "object_id", "generation", "xobject_name", "xobject_type", "filter",
        "color_space", "bits_per_component", "width", "height",
    )}


def _page_summary(page: dict) -> dict:
    return {"page_number": page["page_number"], "media_box": page["media_box"],
            "draws": [{**_draw_identity(draw), "pdf_ctm": draw["pdf_ctm"],
                       "raw_stream_sha256": draw["raw_stream_sha256"],
                       "raw_stream_length": draw["raw_stream_length"]}
                      for draw in page["draws"]]}


def compare_pdf(baseline: dict, mutant: dict, probe: Probe, mutated_image_sha: str) -> dict:
    """Compare every page and ordered draw; expose only compact deltas."""
    result: dict[str, Any] = {
        "page_count_before": baseline["page_count"], "page_count_after": mutant["page_count"],
        "draw_count_before": baseline["draw_count"], "draw_count_after": mutant["draw_count"],
        "changed_page_numbers": [], "changed_media_box_pages": [],
        "changed_draw_order_or_identity": [], "changed_ctms": [],
        "changed_streams": [], "other_draw_streams_unchanged": False,
        "target_stream_equals_mutated_source": False,
        "target_translation_changed": False,
        "unrelated_geometry_stable": False,
        "candidate_causal_effect": False,
    }
    if (baseline["page_count"], baseline["draw_count"]) != (
            mutant["page_count"], mutant["draw_count"]):
        result["outcome"] = "PAGE_OR_DRAW_COUNT_CHANGED"
        return result
    for before, after in zip(baseline["pages"], mutant["pages"]):
        number = before["page_number"]
        if number != after["page_number"]:
            result["changed_draw_order_or_identity"].append(number)
            result["changed_page_numbers"].append(number)
            continue
        if before["media_box"] != after["media_box"]:
            result["changed_media_box_pages"].append(number)
        if len(before["draws"]) != len(after["draws"]):
            result["changed_draw_order_or_identity"].append(number)
            result["changed_page_numbers"].append(number)
            continue
        changed = before["media_box"] != after["media_box"]
        for original_draw, changed_draw in zip(before["draws"], after["draws"]):
            draw_number = original_draw["draw_number"]
            if _draw_identity(original_draw) != _draw_identity(changed_draw):
                result["changed_draw_order_or_identity"].append(
                    {"page_number": number, "draw_number": draw_number})
                changed = True
            if original_draw["pdf_ctm"] != changed_draw["pdf_ctm"]:
                result["changed_ctms"].append({
                    "page_number": number, "draw_number": draw_number,
                    "before": original_draw["pdf_ctm"], "after": changed_draw["pdf_ctm"],
                })
                changed = True
            if (original_draw["raw_stream_sha256"], original_draw["raw_stream_length"]) != (
                    changed_draw["raw_stream_sha256"], changed_draw["raw_stream_length"]):
                result["changed_streams"].append({
                    "page_number": number, "draw_number": draw_number,
                    "before_sha256": original_draw["raw_stream_sha256"],
                    "after_sha256": changed_draw["raw_stream_sha256"],
                    "before_length": original_draw["raw_stream_length"],
                    "after_length": changed_draw["raw_stream_length"],
                })
                changed = True
        if changed:
            result["changed_page_numbers"].append(number)
    target_key = {"page_number": probe.page, "draw_number": probe.image}
    changed_other_streams = [row for row in result["changed_streams"]
                             if (row["page_number"], row["draw_number"]) !=
                             (probe.page, probe.image)]
    result["other_draw_streams_unchanged"] = not changed_other_streams
    if probe.page <= len(mutant["pages"]) and probe.image <= len(mutant["pages"][probe.page - 1]["draws"]):
        target = mutant["pages"][probe.page - 1]["draws"][probe.image - 1]
        result["target_stream_equals_mutated_source"] = (
            target["raw_stream_sha256"] == mutated_image_sha)
    changed_target_ctms = [row for row in result["changed_ctms"]
                           if (row["page_number"], row["draw_number"]) ==
                           (probe.page, probe.image)]
    result["target_translation_changed"] = bool(changed_target_ctms) and all(
        row["before"][:4] == row["after"][:4] and row["before"][4:] != row["after"][4:]
        for row in changed_target_ctms)
    result["unrelated_geometry_stable"] = (
        not result["changed_media_box_pages"] and
        not result["changed_draw_order_or_identity"] and
        all((row["page_number"], row["draw_number"]) ==
            (probe.page, probe.image) for row in result["changed_ctms"]))
    result["candidate_causal_effect"] = (
        result["target_translation_changed"] and result["unrelated_geometry_stable"] and
        result["other_draw_streams_unchanged"] and result["target_stream_equals_mutated_source"])
    if result["candidate_causal_effect"]:
        result["outcome"] = "TARGET_TRANSLATION_CHANGED"
    elif not result["changed_page_numbers"]:
        result["outcome"] = "PDF_METADATA_IDENTICAL"
    elif (result["changed_page_numbers"] == [probe.page] and
          not result["changed_ctms"] and not result["changed_media_box_pages"] and
          not result["changed_draw_order_or_identity"] and
          result["other_draw_streams_unchanged"] and
          result["target_stream_equals_mutated_source"] and
          len(result["changed_streams"]) == 1 and
          result["changed_streams"][0]["page_number"] == probe.page and
          result["changed_streams"][0]["draw_number"] == probe.image):
        result["outcome"] = "TARGET_STREAM_ONLY_CHANGED"
    else:
        result["outcome"] = "OTHER_PDF_CHANGE"
    result["target"] = target_key
    result["baseline_target"] = _page_summary(baseline["pages"][probe.page - 1])
    result["mutant_target"] = _page_summary(mutant["pages"][probe.page - 1])
    return result


def _validate_mutant_page(before: dict, after: dict, probe: Probe) -> None:
    first = before["page"].copy()
    second = after["page"].copy()
    first["images"] = [item.copy() for item in first["images"]]
    second["images"] = [item.copy() for item in second["images"]]
    old_image = first["images"][probe.image - 1]
    new_image = second["images"][probe.image - 1]
    if old_image["payload_sha256"] == new_image["payload_sha256"]:
        raise ProbeError("mutated JPEG payload hash did not change")
    new_image["payload_sha256"] = old_image["payload_sha256"]
    if (first != second or before["sos_offset"] != after["sos_offset"] or
            (before["jpeg_width"], before["jpeg_height"]) !=
            (after["jpeg_width"], after["jpeg_height"])):
        raise ProbeError("mutation changed page structure, JPEG dimensions or SOS position")


def _run_probe(probe: Probe, profile: reference.Profile, source: Path, case: dict,
               baseline: dict, session: Path, paths: Mapping[str, Path],
               expected_source_sha256: str,
               resources: dict) -> dict:
    before, request = _check_probe_source(source, profile.source_id, probe,
                                          case["source_pages"][probe.page - 1], mutated=False)
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    directory = Path(tempfile.mkdtemp(prefix=f"{probe.name}-", dir=session))
    # copy_probe owns a new child directory so a failed copy cannot overwrite a
    # prior probe's private artifact.
    copy = copy_probe(source, directory / "copy", probe)
    if copy["source_sha256"] != expected_source_sha256:
        raise ProbeError("probe source hash differs from pinned source matrix")
    mutated = Path(copy["path"])
    after, request = _check_probe_source(mutated, profile.source_id, probe,
                                         case["source_pages"][probe.page - 1], mutated=True)
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    _validate_mutant_page(before, after, probe)
    result: dict[str, Any] = {
        "name": probe.name, "profile": probe.profile, "source_id": profile.source_id,
        "page_number": probe.page, "image_number": probe.image,
        "field": probe.field, "field_span": [probe.field_offset, probe.field_width],
        "changed_byte_positions": copy["changed_byte_positions"],
        "source_sha256": copy["source_sha256"],
        "mutated_source_sha256": copy["mutated_source_sha256"],
        "original_image_sha256": before["image"]["payload_sha256"],
        "mutated_image_sha256": after["image"]["payload_sha256"],
        "jpeg_structure": {"jfif": "APP0 1.01; SOF0/SOS/terminal EOI checked",
                           "width": after["jpeg_width"], "height": after["jpeg_height"],
                           "sos_offset": after["sos_offset"]},
        "runs": [], "repeatable": False, "outcome": "NOT_RUN",
    }
    metadata_count = 0
    for index in (1, 2):
        conversion = reference.run_converter(mutated, session, paths,
                                             label=f"{probe.name}-run{index}")
        if _file_digest(source) != copy["source_sha256"] or _file_digest(mutated) != copy["mutated_source_sha256"]:
            raise ProbeError("original or mutated source changed during black-box conversion")
        resources["max_observed_child_vmhwm_kib"] = max(
            resources["max_observed_child_vmhwm_kib"], conversion["max_observed_child_vmhwm_kib"])
        resources["max_observed_session_bytes"] = max(
            resources["max_observed_session_bytes"], conversion["max_observed_session_bytes"])
        run = {name: conversion.get(name) for name in (
            "status", "exit_code", "timed_out", "elapsed_milliseconds",
            "pdf_sha256", "pdf_size_bytes", "max_observed_temporary_bytes",
            "max_observed_child_vmhwm_kib", "max_observed_session_bytes",
        )}
        if conversion["status"] == "PASS":
            try:
                parsed = reference.pdf_metadata(Path(conversion["output_path"]), paths)
            except reference.ReferenceError:
                run["metadata_status"] = "UNSUPPORTED"
            else:
                if parsed["pdf_sha256"] != conversion["pdf_sha256"]:
                    raise ProbeError("converted PDF changed between generation and metadata extraction")
                if any(parsed["tools"][name]["sha256"] != reference.PINNED_HASHES[name]
                       for name in ("qpdf", "mutool", "pdfimages")):
                    raise ProbeError("PDF metadata extractor tool differs from pinned executable")
                run["metadata_status"] = "PASS"
                run["pdf_tool_hashes"] = {name: parsed["tools"][name]["sha256"]
                                          for name in ("qpdf", "mutool", "pdfimages")}
                resources["max_pdf_tool_output_bytes"] = max(
                    resources["max_pdf_tool_output_bytes"], parsed["resources"]["max_tool_output_bytes"])
                resources["max_pdf_tool_child_rss_kib"] = max(
                    resources["max_pdf_tool_child_rss_kib"], parsed["resources"]["max_child_rss_kib"])
                delta = compare_pdf(baseline, parsed, probe, after["image"]["payload_sha256"])
                run["pdf_comparison"] = delta
                metadata_count += 1
        result["runs"].append(run)
    result["repeatable"] = (
        result["runs"][0]["status"] == result["runs"][1]["status"] and
        result["runs"][0]["exit_code"] == result["runs"][1]["exit_code"] and
        result["runs"][0]["timed_out"] == result["runs"][1]["timed_out"] and
        result["runs"][0].get("pdf_sha256") == result["runs"][1].get("pdf_sha256") and
        result["runs"][0].get("metadata_status") == result["runs"][1].get("metadata_status") and
        result["runs"][0].get("pdf_comparison") == result["runs"][1].get("pdf_comparison"))
    if not result["repeatable"]:
        result["outcome"] = "NONDETERMINISTIC"
    elif result["runs"][0]["status"] != "PASS":
        result["outcome"] = "CONVERSION_FAILED"
    elif metadata_count != 2:
        result["outcome"] = "PDF_METADATA_UNSUPPORTED"
    else:
        result["outcome"] = result["runs"][0]["pdf_comparison"]["outcome"]
        result["candidate_causal_effect"] = result["runs"][0]["pdf_comparison"]["candidate_causal_effect"]
    if _file_digest(source) != copy["source_sha256"] or _file_digest(mutated) != copy["mutated_source_sha256"]:
        raise ProbeError("original or mutated source changed after PDF inspection")
    return result


def run(paths: Mapping[str, Path] | None = None,
        reference_report: Path | None = None) -> dict[str, Any]:
    report = _report()
    if paths is None and reference_report is None:
        return report
    if paths is None or reference_report is None:
        report["status"] = "FAIL"
        report["counts"]["skipped_probes"] = len(PROBES)
        report["errors"].append("all external paths and --reference-report are required together")
        return report
    rows = None
    before_sources = None
    before_environment = None
    before_inputs = None
    session = None
    resolved = None
    try:
        required = {"corpus", "reference_repo", "python", "pydeps", "libjbigdec",
                    "artifact_root", "git", "qpdf", "mutool", "pdfinfo", "pdfimages"}
        if set(paths) != required:
            raise ProbeError("external path set is incomplete")
        resolved = {name: Path(path).expanduser().resolve() for name, path in paths.items()}
        roots = (ROOT.resolve(), resolved["corpus"], resolved["reference_repo"])
        artifact_root = resolved["artifact_root"]
        if any(artifact_root.is_relative_to(root) for root in roots):
            raise ProbeError("artifact directory must be outside repository, corpus and reference checkout")
        matrix_sha, rows = reference.load_source_rows()
        report["matrix_sha256"] = matrix_sha
        oracle = _load_oracle()
        before_sources = reference.audit_sources(resolved["corpus"], rows)
        report["resources"]["max_source_hash_read_request_bytes"] = max(
            min(row["size_bytes"], reference.READ_CHUNK) for row in rows)
        report["source_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before_sources),
                                  "after_checked": 0, "sources": before_sources}
        report["counts"]["private_source_checks_before"] = len(before_sources)
        before_environment = reference.audit_environment(resolved)
        report["environment_audit"] = _environment_record(before_environment, "BEFORE_PASS")
        reference_report = _external_file(Path(reference_report), roots, "reference report")
        baseline_report, baseline_pdfs = _read_reference_report(
            reference_report, rows, before_environment, before_sources, roots)
        before_inputs = {"matrix": _file_digest(reference.MATRIX),
                         "oracle": _file_digest(ORACLE),
                         "reference_report": _file_digest(reference_report),
                         **{key: _file_digest(pdf) for key, pdf in baseline_pdfs.items()}}
        report["input_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before_inputs),
                                 "after_checked": 0,
                                 "matrix_sha256": before_inputs["matrix"],
                                 "layout_oracle_sha256": before_inputs["oracle"],
                                 "reference_report_sha256": before_inputs["reference_report"],
                                 "baseline_pdf_sha256": {
                                     key: before_inputs[key] for key in baseline_pdfs}}
        report["counts"]["baseline_pdf_checks_before"] = len(baseline_pdfs)
        artifact_root.mkdir(parents=True, exist_ok=True)
        session = Path(tempfile.mkdtemp(prefix="hnc8-placement-jfif-", dir=artifact_root))
        report["artifact_session_path"] = str(session)
        baselines = {}
        for profile in reference.PROFILES:
            parsed = reference.pdf_metadata(baseline_pdfs[f"{profile.name}-run1"], resolved)
            if (parsed["pdf_sha256"] != profile.expected_pdf_sha256 or
                    parsed["page_count"] != profile.expected_pages or
                    parsed["draw_count"] != profile.expected_draws or
                    any(parsed["tools"][name]["sha256"] != reference.PINNED_HASHES[name]
                        for name in ("qpdf", "mutool", "pdfimages")) or
                    parsed["pages"] != oracle[profile.name]["pdf_pages"]):
                raise ProbeError("pinned baseline PDF geometry/order differs from #107 oracle")
            baselines[profile.name] = parsed
            report["counts"]["baseline_pdf_metadata_compared"] += 1
            report["resources"]["max_pdf_tool_output_bytes"] = max(
                report["resources"]["max_pdf_tool_output_bytes"], parsed["resources"]["max_tool_output_bytes"])
            report["resources"]["max_pdf_tool_child_rss_kib"] = max(
                report["resources"]["max_pdf_tool_child_rss_kib"], parsed["resources"]["max_child_rss_kib"])
        profiles = {profile.name: profile for profile in reference.PROFILES}
        source_rows = {row["id"]: row for row in rows}
        for probe in PROBES:
            profile = profiles[probe.profile]
            source = (resolved["corpus"] / source_rows[profile.source_id]["path"]).resolve(strict=True)
            if not source.is_relative_to(resolved["corpus"]):
                raise ProbeError("probe source escaped audited corpus root")
            report["counts"]["probes_attempted"] += 1
            try:
                result = _run_probe(probe, profile, source, oracle[probe.profile],
                                    baselines[probe.profile], session, resolved,
                                    source_rows[profile.source_id]["sha256"], report["resources"])
            except (reference.ReferenceError, ProbeError, OSError, ValueError,
                    KeyError, TypeError) as exc:
                report["counts"]["probes_failing"] += 1
                report["probes"].append({
                    "name": probe.name, "profile": probe.profile,
                    "page_number": probe.page, "image_number": probe.image,
                    "outcome": "PROBE_PROTOCOL_FAILED", "error_kind": type(exc).__name__,
                })
                raise
            report["probes"].append(result)
            report["counts"]["probes_completed"] += 1
            report["counts"]["probe_runs"] += len(result["runs"])
            report["counts"]["repeatable_probes"] += result["repeatable"]
            report["counts"]["pdf_changed_probes"] += (
                result["runs"][0].get("pdf_sha256") is not None and
                result["runs"][0]["pdf_sha256"] != profile.expected_pdf_sha256)
            report["counts"]["placement_changed_probes"] += result["outcome"] == "TARGET_TRANSLATION_CHANGED"
            report["counts"]["passing_causal_probes"] += result.get("candidate_causal_effect", False)
            report["counts"]["conversion_failed_probes"] += result["outcome"] == "CONVERSION_FAILED"
            report["counts"]["unsupported_probes"] += result["outcome"] == "PDF_METADATA_UNSUPPORTED"
            if result["outcome"] in ("CONVERSION_FAILED", "PDF_METADATA_UNSUPPORTED",
                                      "NONDETERMINISTIC"):
                report["counts"]["probes_failing"] += 1
            else:
                report["counts"]["probes_passing"] += 1
            if result["outcome"] == "NONDETERMINISTIC":
                raise ProbeError("probe produced nondeterministic repeated conversions")
        if report["counts"]["probes_attempted"] != len(PROBES):
            raise ProbeError("not all predeclared JFIF probes were attempted")
        report["status"] = "PASS" if report["counts"]["probes_failing"] == 0 else "PARTIAL"
        report["placement_rule_status"] = "UNKNOWN_JFIF_DEPENDENCY_ONLY"
        report["baseline_reference_report_status"] = baseline_report["status"]
    except (reference.ReferenceError, ProbeError, OSError, ValueError, KeyError,
            TypeError, json.JSONDecodeError) as exc:
        report["status"] = "FAIL"
        report["errors"].append(str(exc))
    finally:
        report["counts"]["skipped_probes"] = max(
            0, len(PROBES) - report["counts"]["probes_attempted"])
        if resolved is not None and rows is not None and before_sources is not None:
            try:
                after_sources = reference.audit_sources(resolved["corpus"], rows)
                if after_sources != before_sources:
                    raise ProbeError("source matrix changed between audits")
                report["source_audit"] = {"status": "PASS", "before_checked": len(before_sources),
                                          "after_checked": len(after_sources), "sources": before_sources}
                report["counts"]["private_source_checks_after"] = len(after_sources)
            except (reference.ReferenceError, ProbeError, OSError) as exc:
                report["status"] = "FAIL"
                report["source_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run source audit: {exc}")
        if resolved is not None and before_environment is not None:
            try:
                if reference.audit_environment(resolved) != before_environment:
                    raise ProbeError("reference environment changed between audits")
                report["environment_audit"]["status"] = "PASS"
            except (reference.ReferenceError, ProbeError, OSError) as exc:
                report["status"] = "FAIL"
                report["environment_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run environment audit: {exc}")
        if before_inputs is not None and reference_report is not None:
            try:
                after_inputs = {"matrix": _file_digest(reference.MATRIX),
                                "oracle": _file_digest(ORACLE),
                                "reference_report": _file_digest(reference_report),
                                **{key: _file_digest(pdf) for key, pdf in baseline_pdfs.items()}}
                if after_inputs != before_inputs:
                    raise ProbeError("requested matrix/oracle/report/PDF input changed between audits")
                report["input_audit"]["status"] = "PASS"
                report["input_audit"]["after_checked"] = len(after_inputs)
                report["counts"]["baseline_pdf_checks_after"] = len(baseline_pdfs)
            except (ProbeError, OSError) as exc:
                report["status"] = "FAIL"
                report["input_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run input audit: {exc}")
        if session is not None:
            retained = reference._tree_size(session)
            report["resources"]["retained_artifact_bytes_at_completion"] = retained
            report["resources"]["max_observed_session_bytes"] = max(
                report["resources"]["max_observed_session_bytes"], retained)
        report["resources"]["harness_vmhwm_kib"] = reference._read_rss()["harness_vmhwm_kib"]
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus-dir", type=Path)
    parser.add_argument("--reference-repo", type=Path)
    parser.add_argument("--python-bin", type=Path)
    parser.add_argument("--pydeps-dir", type=Path)
    parser.add_argument("--jbig-lib", type=Path)
    parser.add_argument("--artifact-dir", type=Path)
    parser.add_argument("--reference-report", type=Path)
    for name in ("git", "qpdf", "mutool", "pdfinfo", "pdfimages"):
        parser.add_argument(f"--{name}", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        paths = reference._paths(args)
        report = run(paths, args.reference_report)
    except reference.ReferenceError as exc:
        report = _report()
        report["status"] = "FAIL"
        report["counts"]["skipped_probes"] = len(PROBES)
        report["errors"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        counts = report["counts"]
        print(f"JFIF placement probes [{report['status']}]: "
              f"{counts['probes_attempted']}/{counts['probes_planned']} probes, "
              f"{counts['passing_causal_probes']} causal candidates; "
              f"rule {report['placement_rule_status']}")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

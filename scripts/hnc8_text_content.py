#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in, fixed-row HN/C8 text-content controls for issue #111.

The two declared controls replace a complete framed text component while
preserving its source span and page-index row. The external converter is used
only as a black box. A clean clone makes zero private comparisons.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
from typing import Any, Mapping
import zlib

import hnc8_layout_reference as reference
from hnc8_layout_source import FileInput, SourceExtractor, SourceMetadataError
import hnc8_placement_probe as shared
import hnc8_text_transplant as previous
import hnc8_text_frame as text_frame


COPY_CHUNK = 64 * 1024
MAX_TEXT_BYTES = 64 * 1024
MAX_INFLATED_BYTES = 64 * 1024
FRAME_HEADER_BYTES = 24
ZLIB_RUNTIME_VERSION = "1.3.1"


class ContentError(ValueError):
    """A pinned input, framed text, or declared source change failed a check."""


@dataclass(frozen=True)
class ContentProbe:
    name: str
    profile: str
    page: int
    donor_page: int
    row_offset: int
    target_offset: int
    target_length: int
    donor_offset: int
    donor_length: int
    first_descriptor: int
    level: int
    mem_level: int
    strategy: int
    chunk_size: int
    expected_mutated_source_sha256: str | None = None


PROBES = (
    ContentProbe(
        "c8-row-fixed-text", "c8", 1, 2, 80, 220, 14546, 132124, 10690,
        14766, 6, 8, zlib.Z_DEFAULT_STRATEGY, 384,
        "1a2eff7b1dffc3b81f559bed1c9c405fed552c5e966a1b62e6a90866d492f95f",
    ),
    ContentProbe(
        "hn-a-row-fixed-text", "hn_a", 16, 22, 16664, 953320, 7501,
        1354683, 5344, 960821, 1, 1, zlib.Z_FIXED, 488,
        "2f7b01ad1beaa619f5722cc88bde30ad8739423cdea2e90bbbbf5459934bc1e1",
    ),
)


def _report() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "protocol": "hnc8-row-fixed-text-content-v1",
        "status": "NOT_RUN",
        "placement_rule_status": "UNKNOWN_NOT_TESTED",
        "status_semantics": (
            "PASS means both declared controls produced repeatable, independently parsed "
            "PDF outcomes under every geometry guard. A rejected or unsupported conversion "
            "is PARTIAL, and changed or missing explicit inputs are FAIL."
        ),
        "interpretation": (
            "A guarded target supplemental-image translation change isolates framed "
            "text content from unchanged index-row text address/length. It does not "
            "identify a coordinate field or placement formula. Conversion rejection "
            "or unrelated geometry is UNSUPPORTED for placement inference."
        ),
        "count_semantics": (
            "Attempted counts probe launches; completed counts probes returning two run "
            "records; probe_runs counts returned records, not all converter launches. "
            "Skipped counts unstarted probes."
        ),
        "matrix_sha256": reference.MATRIX_SHA256,
        "layout_oracle_sha256": shared.ORACLE_SHA256,
        "reference_revision": reference.REFERENCE_REVISION,
        "zlib_runtime_version": zlib.ZLIB_RUNTIME_VERSION,
        "source_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "environment_audit": {"status": "NOT_RUN"},
        "input_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "counts": {
            "private_source_checks_before": 0, "private_source_checks_after": 0,
            "baseline_pdf_checks_before": 0, "baseline_pdf_checks_after": 0,
            "baseline_pdf_metadata_compared": 0,
            "probes_planned": len(PROBES), "probes_attempted": 0,
            "probes_completed": 0, "probes_passing": 0, "probes_failing": 0,
            "probe_runs": 0, "repeatable_probes": 0,
            "text_content_effect_probes": 0, "unsupported_probes": 0,
            "conversion_failed_probes": 0, "skipped_probes": 0,
        },
        "probes": [], "errors": [],
        "resources": {
            "mutation_copy_chunk_bytes": COPY_CHUNK,
            "max_text_buffer_bytes": MAX_TEXT_BYTES,
            "max_inflated_buffer_bytes": MAX_INFLATED_BYTES,
            "max_ranged_request_bytes": 0,
            "max_source_hash_read_request_bytes": 0,
            "max_observed_child_vmhwm_kib": 0,
            "max_pdf_tool_output_bytes": 0,
            "max_pdf_tool_child_rss_kib": 0,
            "max_observed_session_bytes": 0,
            "retained_artifact_bytes_at_completion": 0,
            "harness_vmhwm_kib": 0,
            "converter_timeout_seconds": reference.CONVERTER_TIMEOUT_SECONDS,
        },
    }


def _check_plan(probe: ContentProbe) -> None:
    if (probe.row_offset < 0 or probe.target_offset < 0 or probe.donor_offset < 0 or
            probe.target_length <= FRAME_HEADER_BYTES or
            probe.donor_length <= FRAME_HEADER_BYTES or
            probe.target_length > MAX_TEXT_BYTES or probe.donor_length > MAX_TEXT_BYTES or
            probe.target_offset + probe.target_length != probe.first_descriptor or
            probe.chunk_size <= 0 or probe.chunk_size > COPY_CHUNK or
            not 0 <= probe.level <= 9 or not 1 <= probe.mem_level <= 9 or
            probe.strategy not in (zlib.Z_DEFAULT_STRATEGY, zlib.Z_FIXED) or
            probe.expected_mutated_source_sha256 is not None and
            len(probe.expected_mutated_source_sha256) != 64):
        raise ContentError("invalid fixed-row text-content plan")


def _frame(text: bytes, *, image_count: int = 1,
           profile: text_frame.FrameProfile | None = None) -> tuple[bytes, text_frame.FrameMetadata]:
    """Reuse the bounded frame diagnostic; copy one validated private spool."""
    plain = bytearray()

    def receive(spool, metadata: text_frame.FrameMetadata) -> None:
        while block := spool.read(COPY_CHUNK):
            plain.extend(block)
            if len(plain) > MAX_INFLATED_BYTES:
                raise ContentError("validated plaintext exceeds control buffer bound")
        if len(plain) != metadata.decoded_length:
            raise ContentError("validated plaintext spool changed during callback")

    try:
        metadata = text_frame.inspect_frame(
            io.BytesIO(text), offset=0, length=len(text), image_count=image_count,
            profile=profile or text_frame.FrameProfile("synthetic"),
            limits=text_frame.FrameLimits(max_span_bytes=MAX_TEXT_BYTES,
                                          max_decoded_bytes=MAX_INFLATED_BYTES),
            decoded_callback=receive)
    except text_frame.FrameError as exc:
        raise ContentError(f"invalid text frame: {exc}") from exc
    return bytes(plain), metadata


def _compressed(plain: bytes, probe: ContentProbe) -> bytes:
    if len(plain) > MAX_INFLATED_BYTES:
        raise ContentError("donor plaintext exceeds bound")
    compressor = zlib.compressobj(probe.level, zlib.DEFLATED, 15,
                                  probe.mem_level, probe.strategy)
    output = bytearray()
    for at in range(0, len(plain), probe.chunk_size):
        end = min(at + probe.chunk_size, len(plain))
        output.extend(compressor.compress(plain[at:end]))
        if end < len(plain):
            output.extend(compressor.flush(zlib.Z_FULL_FLUSH))
        if len(output) > probe.target_length - FRAME_HEADER_BYTES:
            raise ContentError("recompressed text exceeds fixed target frame")
    output.extend(compressor.flush(zlib.Z_FINISH))
    if len(output) != probe.target_length - FRAME_HEADER_BYTES:
        raise ContentError("recompressed text misses exact target frame length")
    return bytes(output)


def _page(source: Path, source_id: str, number: int) -> tuple[dict, int]:
    try:
        with FileInput(source) as ranged:
            extractor = SourceExtractor(ranged, source_id)
            return extractor.read_page(number), extractor.max_request_bytes
    except (OSError, SourceMetadataError) as exc:
        raise ContentError("source page failed bounded structure check") from exc


def _text(source: Path, offset: int, length: int) -> bytes:
    if length > MAX_TEXT_BYTES or offset < 0:
        raise ContentError("requested text span exceeds bound")
    with FileInput(source) as ranged:
        if offset + length > ranged.size:
            raise ContentError("requested text span is truncated")
        data = ranged.read_at(offset, length)
    if len(data) != length:
        raise ContentError("short ranged text read")
    return data


def _check_source(source: Path, source_id: str, case: dict, probe: ContentProbe,
                  *, mutated_text_sha256: str | None = None) -> int:
    _check_plan(probe)
    target, first_request = _page(source, source_id, probe.page)
    donor, second_request = _page(source, source_id, probe.donor_page)
    pinned_target = case["source_pages"][probe.page - 1]
    pinned_donor = case["source_pages"][probe.donor_page - 1]
    if not previous._oracle_page_matches(donor, pinned_donor):
        raise ContentError("donor page differs from pinned #107 metadata")
    if (pinned_target["text_offset"] != probe.target_offset or
            pinned_target["text_length"] != probe.target_length or
            pinned_donor["text_offset"] != probe.donor_offset or
            pinned_donor["text_length"] != probe.donor_length):
        raise ContentError("declared text spans differ from #107 metadata")
    if not pinned_target["images"] or len(pinned_target["images"]) != len(pinned_donor["images"]):
        raise ContentError("target and donor image counts differ or are empty")
    expected = {**pinned_target, "text_sha256": (
        mutated_text_sha256 if mutated_text_sha256 is not None
        else pinned_target["text_sha256"])}
    if (not previous._oracle_page_matches(target, expected) or
            target["row_offset"] != probe.row_offset or
            target["images"][0]["descriptor_offset"] != probe.first_descriptor):
        raise ContentError("fixed target row, text, or image chain differs")
    return max(first_request, second_request)


def _audit_diff(source: Path, mutant: Path, probe: ContentProbe) -> dict:
    if source.stat().st_size != mutant.stat().st_size:
        raise ContentError("text-content copy changed source length")
    allowed_start = probe.target_offset + 20
    allowed_end = probe.first_descriptor
    changed = 0
    runs: list[list[int]] = []
    with source.open("rb") as old, mutant.open("rb") as new:
        at = 0
        while before := old.read(COPY_CHUNK):
            after = new.read(len(before))
            if len(after) != len(before):
                raise ContentError("text-content copy was truncated")
            for index, (left, right) in enumerate(zip(before, after)):
                if left == right:
                    continue
                position = at + index
                if not allowed_start <= position < allowed_end:
                    raise ContentError("source changed outside declared text-content span")
                changed += 1
                if runs and position == runs[-1][0] + runs[-1][1]:
                    runs[-1][1] += 1
                else:
                    runs.append([position, 1])
            at += len(before)
    if not changed:
        raise ContentError("fixed-row text-content control made no source change")
    return {"changed_byte_count": changed, "changed_offset_runs": runs,
            "allowed_changed_spans": [[allowed_start, allowed_end - allowed_start]]}


def copy_content(source: Path, directory: Path, probe: ContentProbe,
                 expected_source_sha256: str, target_text_sha256: str,
                 donor_text_sha256: str, image_count: int) -> dict:
    """Write one bounded copy with a complete, exact-size donor text frame."""
    _check_plan(probe)
    if zlib.ZLIB_RUNTIME_VERSION != ZLIB_RUNTIME_VERSION:
        raise ContentError("local zlib runtime differs from predeclared compression runtime")
    original_text = _text(source, probe.target_offset, probe.target_length)
    donor_text = _text(source, probe.donor_offset, probe.donor_length)
    if (hashlib.sha256(original_text).hexdigest() != target_text_sha256 or
            hashlib.sha256(donor_text).hexdigest() != donor_text_sha256):
        raise ContentError("target or donor text differs from pinned #107 oracle")
    profile = text_frame.PROFILES[probe.profile]
    _, original_frame = _frame(original_text, image_count=image_count, profile=profile)
    donor_plain, donor_frame = _frame(donor_text, image_count=image_count, profile=profile)
    if original_text[:20] != donor_text[:20]:
        raise ContentError("variant text header differs between target and donor")
    replacement = (original_text[:20] + len(donor_plain).to_bytes(4, "little") +
                   _compressed(donor_plain, probe))
    replacement_plain, replacement_frame = _frame(
        replacement, image_count=image_count, profile=profile)
    if len(replacement) != probe.target_length or replacement_plain != donor_plain:
        raise ContentError("replacement is not the complete donor plaintext frame")
    replacement_sha = hashlib.sha256(replacement).hexdigest()
    directory.mkdir(parents=False, exist_ok=False)
    mutant = directory / "source-mutated.caj"
    digest = hashlib.sha256()
    with source.open("rb") as original, mutant.open("xb") as output:
        while block := original.read(COPY_CHUNK):
            digest.update(block)
            output.write(block)
    if digest.hexdigest() != expected_source_sha256:
        raise ContentError("source copy differs from pinned matrix SHA-256")
    with mutant.open("r+b") as output:
        output.seek(probe.target_offset + 20)
        output.write(replacement[20:])
    diff = _audit_diff(source, mutant, probe)
    if (shared._file_digest(source) != expected_source_sha256 or
            hashlib.sha256(_text(mutant, probe.target_offset, probe.target_length)).hexdigest()
            != replacement_sha):
        raise ContentError("original or mutated text changed during copy")
    mutated_sha256 = shared._file_digest(mutant)
    if (mutated_sha256 == expected_source_sha256 or
            probe.expected_mutated_source_sha256 is not None and
            mutated_sha256 != probe.expected_mutated_source_sha256):
        raise ContentError("mutated source differs from predeclared SHA-256")
    return {"path": mutant, "source_sha256": expected_source_sha256,
            "mutated_source_sha256": mutated_sha256,
            "mutated_text_sha256": replacement_sha,
            "target_text_sha256": target_text_sha256,
            "donor_text_sha256": donor_text_sha256,
            "donor_inflated_sha256": hashlib.sha256(donor_plain).hexdigest(),
            "donor_inflated_length": len(donor_plain),
            "target_frame_length": probe.target_length - FRAME_HEADER_BYTES,
            "frame_checks": {
                "target_decoded_sha256": original_frame.decoded_sha256,
                "donor_decoded_sha256": donor_frame.decoded_sha256,
                "mutant_decoded_sha256": replacement_frame.decoded_sha256,
                "max_source_read_request_bytes": max(frame.max_source_read_request_bytes
                    for frame in (original_frame, donor_frame, replacement_frame)),
                "max_decoder_output_chunk_bytes": max(frame.max_decoder_output_chunk_bytes
                    for frame in (original_frame, donor_frame, replacement_frame)),
                "max_decoded_spool_bytes": max(frame.decoded_spool_bytes
                    for frame in (original_frame, donor_frame, replacement_frame)),
                "complete_no_tail_length_marker_profile": "PASS",
            },
            **diff}


def compare_pdf(baseline: dict, mutant: dict, probe: ContentProbe,
                expected_pages: int, expected_draws: int) -> dict:
    result = previous.compare_pdf(baseline, mutant, probe, expected_pages, expected_draws)
    if result["outcome"] == "TEXT_ROW_COMPONENT_DEPENDENCY":
        result["outcome"] = "TEXT_CONTENT_DEPENDENCY"
    result["text_content_effect"] = result.pop("component_placement_effect")
    return result


def _run_probe(probe: ContentProbe, profile: reference.Profile, source: Path,
               case: dict, baseline: dict, session: Path, paths: Mapping[str, Path],
               expected_source_sha256: str, resources: dict) -> dict:
    request = _check_source(source, profile.source_id, case, probe)
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    directory = Path(tempfile.mkdtemp(prefix=f"{probe.name}-", dir=session))
    pinned_target = case["source_pages"][probe.page - 1]
    pinned_donor = case["source_pages"][probe.donor_page - 1]
    copy = copy_content(source, directory / "copy", probe, expected_source_sha256,
                        pinned_target["text_sha256"], pinned_donor["text_sha256"],
                        pinned_target["image_count"])
    mutant = Path(copy["path"])
    request = _check_source(mutant, profile.source_id, case, probe,
                            mutated_text_sha256=copy["mutated_text_sha256"])
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    result: dict[str, Any] = {
        "name": probe.name, "profile": probe.profile, "source_id": profile.source_id,
        "target_page": probe.page, "donor_page": probe.donor_page,
        "unchanged_target_row_span": [probe.row_offset, 20],
        "original_text_span": [probe.target_offset, probe.target_length],
        "donor_text_span": [probe.donor_offset, probe.donor_length],
        "mutant_text_span": [probe.target_offset, probe.target_length],
        "first_descriptor": probe.first_descriptor,
        "compression": {"level": probe.level, "mem_level": probe.mem_level,
                        "strategy": probe.strategy, "chunk_size": probe.chunk_size,
                        "flush": "Z_FULL_FLUSH between nonempty chunks",
                        "runtime_version": zlib.ZLIB_RUNTIME_VERSION},
        **{key: value for key, value in copy.items() if key != "path"},
        "runs": [], "repeatable": False, "outcome": "NOT_RUN",
    }
    for index in (1, 2):
        conversion = reference.run_converter(mutant, session, paths,
                                             label=f"{probe.name}-run{index}")
        if (shared._file_digest(source) != copy["source_sha256"] or
                shared._file_digest(mutant) != copy["mutated_source_sha256"]):
            raise ContentError("original or mutated source changed during conversion")
        resources["max_observed_child_vmhwm_kib"] = max(
            resources["max_observed_child_vmhwm_kib"], conversion["max_observed_child_vmhwm_kib"])
        resources["max_observed_session_bytes"] = max(
            resources["max_observed_session_bytes"], conversion["max_observed_session_bytes"])
        run = {key: conversion.get(key) for key in (
            "status", "exit_code", "timed_out", "elapsed_milliseconds",
            "pdf_sha256", "pdf_size_bytes", "max_observed_temporary_bytes",
            "max_observed_child_vmhwm_kib", "max_observed_session_bytes")}
        if conversion["status"] == "PASS":
            try:
                parsed = reference.pdf_metadata(Path(conversion["output_path"]), paths)
            except reference.ReferenceError:
                run["metadata_status"] = "UNSUPPORTED"
            else:
                if parsed["pdf_sha256"] != conversion["pdf_sha256"]:
                    raise ContentError("converted PDF changed before metadata extraction")
                if any(parsed["tools"][name]["sha256"] != reference.PINNED_HASHES[name]
                       for name in ("qpdf", "mutool", "pdfimages")):
                    raise ContentError("PDF extractor executable differs from pinned tool")
                run["metadata_status"] = "PASS"
                run["pdf_tool_hashes"] = {name: parsed["tools"][name]["sha256"]
                                          for name in ("qpdf", "mutool", "pdfimages")}
                resources["max_pdf_tool_output_bytes"] = max(
                    resources["max_pdf_tool_output_bytes"], parsed["resources"]["max_tool_output_bytes"])
                resources["max_pdf_tool_child_rss_kib"] = max(
                    resources["max_pdf_tool_child_rss_kib"], parsed["resources"]["max_child_rss_kib"])
                run["pdf_comparison"] = compare_pdf(
                    baseline, parsed, probe, profile.expected_pages, profile.expected_draws)
        result["runs"].append(run)
    first, second = result["runs"]
    result["repeatable"] = all(first.get(key) == second.get(key) for key in (
        "status", "exit_code", "timed_out", "pdf_sha256", "metadata_status", "pdf_comparison"))
    if not result["repeatable"]:
        result["outcome"] = "NONDETERMINISTIC"
    elif first["status"] != "PASS":
        result["outcome"] = "CONVERSION_FAILED"
    elif first.get("metadata_status") != "PASS":
        result["outcome"] = "UNSUPPORTED"
    else:
        result["outcome"] = first["pdf_comparison"]["outcome"]
        result["text_content_effect"] = first["pdf_comparison"]["text_content_effect"]
    if (shared._file_digest(source) != copy["source_sha256"] or
            shared._file_digest(mutant) != copy["mutated_source_sha256"]):
        raise ContentError("original or mutated source changed after PDF inspection")
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
    rows = before_sources = before_environment = before_inputs = None
    resolved = session = baseline_pdfs = None
    try:
        required = {"corpus", "reference_repo", "python", "pydeps", "libjbigdec",
                    "artifact_root", "git", "qpdf", "mutool", "pdfinfo", "pdfimages"}
        if set(paths) != required:
            raise ContentError("external path set is incomplete")
        resolved = {name: Path(path).expanduser().resolve() for name, path in paths.items()}
        roots = (shared.ROOT.resolve(), resolved["corpus"], resolved["reference_repo"])
        artifact_root = resolved["artifact_root"]
        if any(artifact_root.is_relative_to(root) for root in roots):
            raise ContentError("artifact directory must be outside repository, corpus and reference checkout")
        if zlib.ZLIB_RUNTIME_VERSION != ZLIB_RUNTIME_VERSION:
            raise ContentError("local zlib runtime differs from predeclared compression runtime")
        matrix_sha, rows = reference.load_source_rows()
        report["matrix_sha256"] = matrix_sha
        oracle = shared._load_oracle()
        before_sources = reference.audit_sources(resolved["corpus"], rows)
        report["resources"]["max_source_hash_read_request_bytes"] = max(
            min(row["size_bytes"], reference.READ_CHUNK) for row in rows)
        report["source_audit"] = {"status": "BEFORE_PASS", "before_checked": len(before_sources),
                                  "after_checked": 0, "sources": before_sources}
        report["counts"]["private_source_checks_before"] = len(before_sources)
        before_environment = reference.audit_environment(resolved)
        report["environment_audit"] = shared._environment_record(before_environment, "BEFORE_PASS")
        reference_report = shared._external_file(Path(reference_report), roots, "reference report")
        baseline_report, baseline_pdfs = shared._read_reference_report(
            reference_report, rows, before_environment, before_sources, roots)
        before_inputs = {"matrix": shared._file_digest(reference.MATRIX),
                         "oracle": shared._file_digest(shared.ORACLE),
                         "reference_report": shared._file_digest(reference_report),
                         **{key: shared._file_digest(pdf) for key, pdf in baseline_pdfs.items()}}
        report["input_audit"] = {
            "status": "BEFORE_PASS", "before_checked": len(before_inputs), "after_checked": 0,
            "matrix_sha256": before_inputs["matrix"],
            "layout_oracle_sha256": before_inputs["oracle"],
            "reference_report_sha256": before_inputs["reference_report"],
            "baseline_pdf_sha256": {key: before_inputs[key] for key in baseline_pdfs},
        }
        report["counts"]["baseline_pdf_checks_before"] = len(baseline_pdfs)
        artifact_root.mkdir(parents=True, exist_ok=True)
        session = Path(tempfile.mkdtemp(prefix="hnc8-text-content-", dir=artifact_root))
        report["artifact_session_path"] = str(session)
        baselines = {}
        profiles = {profile.name: profile for profile in reference.PROFILES}
        for profile in reference.PROFILES:
            if profile.name == "hn_b":
                continue
            parsed = reference.pdf_metadata(baseline_pdfs[f"{profile.name}-run1"], resolved)
            if (parsed["pdf_sha256"] != profile.expected_pdf_sha256 or
                    parsed["page_count"] != profile.expected_pages or
                    parsed["draw_count"] != profile.expected_draws or
                    any(parsed["tools"][name]["sha256"] != reference.PINNED_HASHES[name]
                        for name in ("qpdf", "mutool", "pdfimages")) or
                    parsed["pages"] != oracle[profile.name]["pdf_pages"]):
                raise ContentError("pinned baseline PDF differs from #107 oracle")
            baselines[profile.name] = parsed
            report["counts"]["baseline_pdf_metadata_compared"] += 1
            report["resources"]["max_pdf_tool_output_bytes"] = max(
                report["resources"]["max_pdf_tool_output_bytes"], parsed["resources"]["max_tool_output_bytes"])
            report["resources"]["max_pdf_tool_child_rss_kib"] = max(
                report["resources"]["max_pdf_tool_child_rss_kib"], parsed["resources"]["max_child_rss_kib"])
        source_rows = {row["id"]: row for row in rows}
        for probe in PROBES:
            profile = profiles[probe.profile]
            source = (resolved["corpus"] / source_rows[profile.source_id]["path"]).resolve(strict=True)
            if not source.is_relative_to(resolved["corpus"]):
                raise ContentError("probe source escaped audited corpus root")
            report["counts"]["probes_attempted"] += 1
            try:
                result = _run_probe(probe, profile, source, oracle[probe.profile],
                                    baselines[probe.profile], session, resolved,
                                    source_rows[profile.source_id]["sha256"], report["resources"])
                if len(result["runs"]) != 2 or result["outcome"] not in (
                        "NO_PLACEMENT_CHANGE", "TEXT_CONTENT_DEPENDENCY",
                        "UNSUPPORTED", "CONVERSION_FAILED", "NONDETERMINISTIC"):
                    raise ContentError("probe did not return two classified conversion runs")
            except (reference.ReferenceError, ContentError, OSError, ValueError,
                    KeyError, TypeError) as exc:
                report["counts"]["probes_failing"] += 1
                report["probes"].append({"name": probe.name, "profile": probe.profile,
                                         "outcome": "PROBE_PROTOCOL_FAILED", "error_kind": type(exc).__name__})
                raise
            report["probes"].append(result)
            report["counts"]["probes_completed"] += 1
            report["counts"]["probe_runs"] += len(result["runs"])
            report["counts"]["repeatable_probes"] += result["repeatable"]
            report["counts"]["text_content_effect_probes"] += result["outcome"] == "TEXT_CONTENT_DEPENDENCY"
            report["counts"]["conversion_failed_probes"] += result["outcome"] == "CONVERSION_FAILED"
            report["counts"]["unsupported_probes"] += result["outcome"] in (
                "CONVERSION_FAILED", "UNSUPPORTED", "NONDETERMINISTIC")
            if result["outcome"] in ("CONVERSION_FAILED", "UNSUPPORTED", "NONDETERMINISTIC"):
                report["counts"]["probes_failing"] += 1
            else:
                report["counts"]["probes_passing"] += 1
            if result["outcome"] == "NONDETERMINISTIC":
                raise ContentError("probe produced nondeterministic repeated conversions")
        report["status"] = "PASS" if report["counts"]["unsupported_probes"] == 0 else "PARTIAL"
        report["placement_rule_status"] = "UNKNOWN_TEXT_CONTENT_ONLY"
        report["baseline_reference_report_status"] = baseline_report["status"]
    except (reference.ReferenceError, ContentError, OSError, ValueError,
            KeyError, TypeError, json.JSONDecodeError) as exc:
        report["status"] = "FAIL"
        report["errors"].append(str(exc))
    finally:
        report["counts"]["skipped_probes"] = max(0, len(PROBES) - report["counts"]["probes_attempted"])
        if resolved is not None and rows is not None and before_sources is not None:
            try:
                after_sources = reference.audit_sources(resolved["corpus"], rows)
                if after_sources != before_sources:
                    raise ContentError("source matrix changed between audits")
                report["source_audit"] = {"status": "PASS", "before_checked": len(before_sources),
                                          "after_checked": len(after_sources), "sources": before_sources}
                report["counts"]["private_source_checks_after"] = len(after_sources)
            except (reference.ReferenceError, ContentError, OSError) as exc:
                report["status"] = "FAIL"
                report["source_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run source audit: {exc}")
        if resolved is not None and before_environment is not None:
            try:
                if reference.audit_environment(resolved) != before_environment:
                    raise ContentError("reference environment changed between audits")
                report["environment_audit"]["status"] = "PASS"
            except (reference.ReferenceError, ContentError, OSError) as exc:
                report["status"] = "FAIL"
                report["environment_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run environment audit: {exc}")
        if before_inputs is not None and reference_report is not None and baseline_pdfs is not None:
            try:
                after_inputs = {"matrix": shared._file_digest(reference.MATRIX),
                                "oracle": shared._file_digest(shared.ORACLE),
                                "reference_report": shared._file_digest(reference_report),
                                **{key: shared._file_digest(pdf) for key, pdf in baseline_pdfs.items()}}
                if after_inputs != before_inputs:
                    raise ContentError("matrix/oracle/report/PDF input changed between audits")
                report["input_audit"]["status"] = "PASS"
                report["input_audit"]["after_checked"] = len(after_inputs)
                report["counts"]["baseline_pdf_checks_after"] = len(baseline_pdfs)
            except (ContentError, OSError) as exc:
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
    for name in ("corpus-dir", "reference-repo", "python-bin", "pydeps-dir",
                 "jbig-lib", "artifact-dir", "reference-report", "git", "qpdf",
                 "mutool", "pdfinfo", "pdfimages"):
        parser.add_argument(f"--{name}", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = run(reference._paths(args), args.reference_report)
    except reference.ReferenceError as exc:
        report = _report()
        report["status"] = "FAIL"
        report["counts"]["skipped_probes"] = len(PROBES)
        report["errors"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        counts = report["counts"]
        print(f"HN/C8 fixed-row text controls [{report['status']}]: "
              f"{counts['probes_attempted']}/{counts['probes_planned']} attempted; "
              f"rule {report['placement_rule_status']}")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

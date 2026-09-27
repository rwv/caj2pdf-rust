#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Opt-in HN/C8 text-component probes predeclared for issue #110.

The external converter is executed only as a black box. Temporary sources,
PDFs and the private report stay outside this repository. A clean clone makes
zero private comparisons.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import struct
import sys
import tempfile
from typing import Any, Mapping

import hnc8_layout_reference as reference
import hnc8_placement_probe as shared
from hnc8_layout_source import FileInput, SourceExtractor, SourceMetadataError


COPY_CHUNK = 64 * 1024


class TransplantError(ValueError):
    """A pinned input or declared source transformation failed a check."""


@dataclass(frozen=True)
class Transplant:
    name: str
    profile: str
    page: int
    donor_page: int
    row_offset: int
    original_text_offset: int
    original_text_length: int
    donor_offset: int
    donor_length: int
    target_offset: int
    first_descriptor: int


TRANSPLANTS = (
    Transplant("c8-text", "c8", 1, 2, 80, 220, 14546,
               132124, 10690, 4076, 14766),
    Transplant("hn-a-text", "hn_a", 16, 22, 16664, 953320, 7501,
               1354683, 5344, 955477, 960821),
)


def _report() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "protocol": "hnc8-text-component-batch-v1",
        "status": "NOT_RUN",
        "placement_rule_status": "UNKNOWN_NOT_TESTED",
        "interpretation": (
            "A changed supplemental CTM under every guard implicates the combined "
            "text content and row address/length component, not an individual "
            "coordinate field or a placement rule. Rejection, nonlocal geometry, "
            "or changed image identity is UNSUPPORTED for placement inference."
        ),
        "count_semantics": (
            "probes_attempted counts probe launches; probes_completed counts probes with "
            "two returned run records; probe_runs counts those returned records only. "
            "A protocol failure may have launched a converter before returning, so "
            "probe_runs is not a count of all converter launches. Skipped counts "
            "unstarted probes."
        ),
        "matrix_sha256": reference.MATRIX_SHA256,
        "layout_oracle_sha256": shared.ORACLE_SHA256,
        "reference_revision": reference.REFERENCE_REVISION,
        "source_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "environment_audit": {"status": "NOT_RUN"},
        "input_audit": {"status": "NOT_RUN", "before_checked": 0, "after_checked": 0},
        "counts": {
            "private_source_checks_before": 0, "private_source_checks_after": 0,
            "baseline_pdf_checks_before": 0, "baseline_pdf_checks_after": 0,
            "baseline_pdf_metadata_compared": 0,
            "probes_planned": len(TRANSPLANTS), "probes_attempted": 0,
            "probes_completed": 0, "probes_passing": 0, "probes_failing": 0,
            "probe_runs": 0, "repeatable_probes": 0,
            "placement_changed_probes": 0, "unsupported_probes": 0,
            "conversion_failed_probes": 0, "skipped_probes": 0,
        },
        "probes": [], "errors": [],
        "resources": {
            "mutation_copy_chunk_bytes": COPY_CHUNK,
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


def _check_plan(probe: Transplant) -> None:
    if (probe.row_offset < 0 or probe.original_text_offset < 0 or
            probe.donor_offset < 0 or probe.donor_length <= 0 or
            probe.original_text_length < probe.donor_length or
            probe.original_text_offset + probe.original_text_length != probe.first_descriptor or
            probe.target_offset + probe.donor_length != probe.first_descriptor or
            probe.target_offset < probe.original_text_offset or
            probe.donor_offset < probe.first_descriptor):
        raise TransplantError("transplant does not preserve the target descriptor boundary")


def _page(source: Path, source_id: str, page_number: int) -> tuple[dict, int]:
    try:
        with FileInput(source) as ranged:
            extractor = SourceExtractor(ranged, source_id)
            return extractor.read_page(page_number), extractor.max_request_bytes
    except (OSError, SourceMetadataError) as exc:
        raise TransplantError("source page failed independent bounded structure check") from exc


def _oracle_page_matches(actual: dict, expected: dict) -> bool:
    return all(actual[key] == value for key, value in expected.items() if key != "images") and (
        len(actual["images"]) == len(expected["images"]) and
        all(all(image[key] == value for key, value in pinned.items())
            for image, pinned in zip(actual["images"], expected["images"])))


def _check_source(source: Path, source_id: str, case: dict,
                  probe: Transplant, *, mutated: bool) -> int:
    _check_plan(probe)
    target, first_request = _page(source, source_id, probe.page)
    donor, second_request = _page(source, source_id, probe.donor_page)
    pinned_target = case["source_pages"][probe.page - 1]
    pinned_donor = case["source_pages"][probe.donor_page - 1]
    if not _oracle_page_matches(donor, pinned_donor):
        raise TransplantError("donor page differs from pinned #107 metadata")
    if (pinned_target["text_offset"] != probe.original_text_offset or
            pinned_target["text_length"] != probe.original_text_length or
            pinned_donor["text_offset"] != probe.donor_offset or
            pinned_donor["text_length"] != probe.donor_length):
        raise TransplantError("predeclared text spans differ from #107 metadata")
    if len(pinned_target["images"]) != len(pinned_donor["images"]):
        raise TransplantError("target/donor image counts differ")
    if not mutated:
        if not _oracle_page_matches(target, pinned_target):
            raise TransplantError("target page differs from pinned #107 metadata")
    else:
        expected = {**pinned_target, "text_offset": probe.target_offset,
                    "text_length": probe.donor_length,
                    "text_sha256": pinned_donor["text_sha256"]}
        if not _oracle_page_matches(target, expected):
            raise TransplantError("transplanted text or target image chain differs")
    if target["row_offset"] != probe.row_offset or target["images"][0]["descriptor_offset"] != probe.first_descriptor:
        raise TransplantError("target index row or first descriptor moved")
    return max(first_request, second_request)


def _hash_span(source: Path, offset: int, length: int) -> str:
    digest = hashlib.sha256()
    with source.open("rb") as stream:
        stream.seek(offset)
        remaining = length
        while remaining:
            block = stream.read(min(remaining, COPY_CHUNK))
            if not block:
                raise TransplantError("donor text span truncated")
            digest.update(block)
            remaining -= len(block)
    return digest.hexdigest()


def _audit_diff(source: Path, mutant: Path, probe: Transplant) -> dict:
    if source.stat().st_size != mutant.stat().st_size:
        raise TransplantError("transplanted copy changed source length")
    row_start, row_end = probe.row_offset, probe.row_offset + 8
    text_start, text_end = probe.target_offset, probe.first_descriptor
    row_count = text_count = 0
    changed_offset_runs: list[list[int]] = []
    with source.open("rb") as old, mutant.open("rb") as new:
        at = 0
        while before := old.read(COPY_CHUNK):
            after = new.read(len(before))
            if len(after) != len(before):
                raise TransplantError("transplanted copy was truncated")
            for index, (a, b) in enumerate(zip(before, after)):
                if a != b:
                    position = at + index
                    if row_start <= position < row_end:
                        row_count += 1
                    elif text_start <= position < text_end:
                        text_count += 1
                    else:
                        raise TransplantError("transplant changed a byte outside the declared spans")
                    if (changed_offset_runs and position ==
                            changed_offset_runs[-1][0] + changed_offset_runs[-1][1]):
                        changed_offset_runs[-1][1] += 1
                    else:
                        changed_offset_runs.append([position, 1])
            at += len(before)
    if not row_count or not text_count:
        raise TransplantError("declared row and text component were not both changed")
    return {"changed_row_bytes": row_count, "changed_text_bytes": text_count,
            "changed_byte_count": row_count + text_count,
            "changed_offset_runs": changed_offset_runs,
            "allowed_changed_spans": [[row_start, 8], [text_start, probe.donor_length]]}


def copy_transplant(source: Path, directory: Path, probe: Transplant,
                    expected_source_sha256: str, donor_sha256: str) -> dict:
    """Copy in bounded chunks, edit only declared spans, and audit every byte."""
    _check_plan(probe)
    size = source.stat().st_size
    if (probe.donor_offset + probe.donor_length > size or
            probe.first_descriptor > size or probe.row_offset + 20 > size):
        raise TransplantError("declared text or row span is outside source")
    if _hash_span(source, probe.donor_offset, probe.donor_length) != donor_sha256:
        raise TransplantError("donor text SHA-256 differs from pinned #107 oracle")
    directory.mkdir(parents=False, exist_ok=False)
    mutant = directory / "source-mutated.caj"
    source_hash = hashlib.sha256()
    with source.open("rb") as original, mutant.open("xb") as output:
        while block := original.read(COPY_CHUNK):
            source_hash.update(block)
            output.write(block)
    if source_hash.hexdigest() != expected_source_sha256:
        raise TransplantError("source copy differs from pinned matrix SHA-256")
    with source.open("rb") as original, mutant.open("r+b") as output:
        original.seek(probe.row_offset)
        row = original.read(20)
        if len(row) != 20 or struct.unpack_from("<ii", row) != (
                probe.original_text_offset, probe.original_text_length):
            raise TransplantError("target row text offset/length differs from plan")
        original.seek(probe.donor_offset)
        output.seek(probe.target_offset)
        remaining = probe.donor_length
        while remaining:
            block = original.read(min(remaining, COPY_CHUNK))
            if not block:
                raise TransplantError("donor text truncated during transplant")
            output.write(block)
            remaining -= len(block)
        output.seek(probe.row_offset)
        output.write(struct.pack("<ii", probe.target_offset, probe.donor_length))
    diff = _audit_diff(source, mutant, probe)
    if (_hash_span(mutant, probe.target_offset, probe.donor_length) != donor_sha256 or
            shared._file_digest(source) != expected_source_sha256):
        raise TransplantError("source or transplanted text changed during copy")
    mutated_sha256 = shared._file_digest(mutant)
    if mutated_sha256 == expected_source_sha256:
        raise TransplantError("transplanted source hash did not change")
    return {"path": mutant, "source_sha256": expected_source_sha256,
            "mutated_source_sha256": mutated_sha256,
            "donor_text_sha256": donor_sha256, **diff}


def compare_pdf(baseline: dict, mutant: dict, probe: Transplant,
                expected_pages: int, expected_draws: int) -> dict:
    """Require stable full-document structure before interpreting target CTMs."""
    result: dict[str, Any] = {
        "page_count_before": baseline["page_count"], "page_count_after": mutant["page_count"],
        "draw_count_before": baseline["draw_count"], "draw_count_after": mutant["draw_count"],
        "changed_ctms": [], "guard_failures": [], "component_placement_effect": False,
        "non_target_pages_checked": 0,
    }
    failures = result["guard_failures"]
    if ((baseline["page_count"], baseline["draw_count"]) != (expected_pages, expected_draws) or
            (mutant["page_count"], mutant["draw_count"]) != (expected_pages, expected_draws) or
            len(baseline["pages"]) != expected_pages or len(mutant["pages"]) != expected_pages):
        failures.append("page or ordered-draw count changed")
        result["outcome"] = "UNSUPPORTED"
        return result
    result["target_page_before"] = shared._page_summary(baseline["pages"][probe.page - 1])
    result["target_page_after"] = shared._page_summary(mutant["pages"][probe.page - 1])
    for original_page, changed_page in zip(baseline["pages"], mutant["pages"]):
        page_number = original_page["page_number"]
        if page_number != probe.page:
            result["non_target_pages_checked"] += 1
        if changed_page["page_number"] != page_number or original_page["media_box"] != changed_page["media_box"]:
            failures.append(f"page {page_number}: number or MediaBox changed")
        if len(original_page["draws"]) != len(changed_page["draws"]):
            failures.append(f"page {page_number}: draw count changed")
            continue
        for old, new in zip(original_page["draws"], changed_page["draws"]):
            number = old["draw_number"]
            identity = ("draw_number", "object_id", "generation", "xobject_name",
                        "xobject_type", "filter", "color_space",
                        "bits_per_component", "width", "height", "raw_stream_sha256",
                        "raw_stream_length")
            if any(old[key] != new[key] for key in identity):
                failures.append(f"page {page_number} draw {number}: image order or identity changed")
            if old["pdf_ctm"] != new["pdf_ctm"]:
                result["changed_ctms"].append({"page_number": page_number, "draw_number": number,
                                                "before": old["pdf_ctm"], "after": new["pdf_ctm"]})
                if page_number != probe.page or number == 1 or old["pdf_ctm"][:4] != new["pdf_ctm"][:4]:
                    failures.append(f"page {page_number} draw {number}: unrelated geometry changed")
    if len(baseline["pages"][probe.page - 1]["draws"]) != len(
            mutant["pages"][probe.page - 1]["draws"]):
        failures.append("target page draw count changed")
    donor_draws = baseline["pages"][probe.donor_page - 1]["draws"]
    target_draws = mutant["pages"][probe.page - 1]["draws"]
    donor_comparisons = []
    for index, target_draw in enumerate(target_draws[1:], start=1):
        donor_xy = donor_draws[index]["pdf_ctm"][4:] if index < len(donor_draws) else None
        target_xy = target_draw["pdf_ctm"][4:]
        donor_comparisons.append({
            "draw_number": index + 1, "donor_xy": donor_xy, "target_after_xy": target_xy,
            "matched_at_recorded_precision": donor_xy is not None and donor_xy == target_xy,
        })
    result["donor_translation_comparisons"] = donor_comparisons
    result["donor_translation_matches"] = sum(
        row["matched_at_recorded_precision"] for row in donor_comparisons)
    result["component_placement_effect"] = bool(result["changed_ctms"]) and not failures
    result["outcome"] = "UNSUPPORTED" if failures else (
        "TEXT_ROW_COMPONENT_DEPENDENCY" if result["component_placement_effect"]
        else "NO_PLACEMENT_CHANGE")
    return result


def _run_probe(probe: Transplant, profile: reference.Profile, source: Path, case: dict,
               baseline: dict, session: Path, paths: Mapping[str, Path],
               expected_source_sha256: str, resources: dict) -> dict:
    request = _check_source(source, profile.source_id, case, probe, mutated=False)
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    directory = Path(tempfile.mkdtemp(prefix=f"{probe.name}-", dir=session))
    donor_sha256 = case["source_pages"][probe.donor_page - 1]["text_sha256"]
    copy = copy_transplant(source, directory / "copy", probe,
                           expected_source_sha256, donor_sha256)
    mutant = Path(copy["path"])
    request = _check_source(mutant, profile.source_id, case, probe, mutated=True)
    resources["max_ranged_request_bytes"] = max(resources["max_ranged_request_bytes"], request)
    result: dict[str, Any] = {
        "name": probe.name, "profile": probe.profile, "source_id": profile.source_id,
        "target_page": probe.page, "donor_page": probe.donor_page,
        "target_row_span": [probe.row_offset, 20],
        "original_text_span": [probe.original_text_offset, probe.original_text_length],
        "donor_text_span": [probe.donor_offset, probe.donor_length],
        "mutant_text_span": [probe.target_offset, probe.donor_length],
        "first_descriptor": probe.first_descriptor,
        "source_sha256": copy["source_sha256"],
        "mutated_source_sha256": copy["mutated_source_sha256"],
        "donor_text_sha256": copy["donor_text_sha256"],
        "changed_byte_count": copy["changed_byte_count"],
        "changed_row_bytes": copy["changed_row_bytes"],
        "changed_text_bytes": copy["changed_text_bytes"],
        "changed_offset_runs": copy["changed_offset_runs"],
        "allowed_changed_spans": copy["allowed_changed_spans"],
        "runs": [], "repeatable": False, "outcome": "NOT_RUN",
    }
    for index in (1, 2):
        conversion = reference.run_converter(mutant, session, paths,
                                             label=f"{probe.name}-run{index}")
        if (shared._file_digest(source) != copy["source_sha256"] or
                shared._file_digest(mutant) != copy["mutated_source_sha256"]):
            raise TransplantError("original or mutated source changed during conversion")
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
                    raise TransplantError("converted PDF changed before metadata extraction")
                if any(parsed["tools"][name]["sha256"] != reference.PINNED_HASHES[name]
                       for name in ("qpdf", "mutool", "pdfimages")):
                    raise TransplantError("PDF extractor executable differs from pinned tool")
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
        result["component_placement_effect"] = first["pdf_comparison"]["component_placement_effect"]
    if (shared._file_digest(source) != copy["source_sha256"] or
            shared._file_digest(mutant) != copy["mutated_source_sha256"]):
        raise TransplantError("original or mutated source changed after PDF inspection")
    return result


def run(paths: Mapping[str, Path] | None = None,
        reference_report: Path | None = None) -> dict[str, Any]:
    report = _report()
    if paths is None and reference_report is None:
        return report
    if paths is None or reference_report is None:
        report["status"] = "FAIL"
        report["counts"]["skipped_probes"] = len(TRANSPLANTS)
        report["errors"].append("all external paths and --reference-report are required together")
        return report
    rows = before_sources = before_environment = before_inputs = None
    resolved = session = baseline_pdfs = None
    try:
        required = {"corpus", "reference_repo", "python", "pydeps", "libjbigdec",
                    "artifact_root", "git", "qpdf", "mutool", "pdfinfo", "pdfimages"}
        if set(paths) != required:
            raise TransplantError("external path set is incomplete")
        resolved = {name: Path(path).expanduser().resolve() for name, path in paths.items()}
        roots = (shared.ROOT.resolve(), resolved["corpus"], resolved["reference_repo"])
        artifact_root = resolved["artifact_root"]
        if any(artifact_root.is_relative_to(root) for root in roots):
            raise TransplantError("artifact directory must be outside repository, corpus and reference checkout")
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
        session = Path(tempfile.mkdtemp(prefix="hnc8-text-component-", dir=artifact_root))
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
                raise TransplantError("pinned baseline PDF differs from #107 oracle")
            baselines[profile.name] = parsed
            report["counts"]["baseline_pdf_metadata_compared"] += 1
            report["resources"]["max_pdf_tool_output_bytes"] = max(
                report["resources"]["max_pdf_tool_output_bytes"], parsed["resources"]["max_tool_output_bytes"])
            report["resources"]["max_pdf_tool_child_rss_kib"] = max(
                report["resources"]["max_pdf_tool_child_rss_kib"], parsed["resources"]["max_child_rss_kib"])
        source_rows = {row["id"]: row for row in rows}
        for probe in TRANSPLANTS:
            profile = profiles[probe.profile]
            source = (resolved["corpus"] / source_rows[profile.source_id]["path"]).resolve(strict=True)
            if not source.is_relative_to(resolved["corpus"]):
                raise TransplantError("probe source escaped audited corpus root")
            report["counts"]["probes_attempted"] += 1
            try:
                result = _run_probe(probe, profile, source, oracle[probe.profile],
                                    baselines[probe.profile], session, resolved,
                                    source_rows[profile.source_id]["sha256"], report["resources"])
                if len(result["runs"]) != 2 or result["outcome"] not in (
                        "NO_PLACEMENT_CHANGE", "TEXT_ROW_COMPONENT_DEPENDENCY",
                        "UNSUPPORTED", "CONVERSION_FAILED", "NONDETERMINISTIC"):
                    raise TransplantError("probe did not return two classified conversion runs")
            except (reference.ReferenceError, TransplantError, OSError, ValueError,
                    KeyError, TypeError) as exc:
                report["counts"]["probes_failing"] += 1
                report["probes"].append({"name": probe.name, "profile": probe.profile,
                                         "outcome": "PROBE_PROTOCOL_FAILED", "error_kind": type(exc).__name__})
                raise
            report["probes"].append(result)
            report["counts"]["probes_completed"] += 1
            report["counts"]["probe_runs"] += len(result["runs"])
            report["counts"]["repeatable_probes"] += result["repeatable"]
            report["counts"]["placement_changed_probes"] += result["outcome"] == "TEXT_ROW_COMPONENT_DEPENDENCY"
            report["counts"]["conversion_failed_probes"] += result["outcome"] == "CONVERSION_FAILED"
            report["counts"]["unsupported_probes"] += result["outcome"] in (
                "CONVERSION_FAILED", "UNSUPPORTED", "NONDETERMINISTIC")
            if result["outcome"] in ("CONVERSION_FAILED", "UNSUPPORTED", "NONDETERMINISTIC"):
                report["counts"]["probes_failing"] += 1
            else:
                report["counts"]["probes_passing"] += 1
            if result["outcome"] == "NONDETERMINISTIC":
                raise TransplantError("probe produced nondeterministic repeated conversions")
        report["status"] = "PASS" if report["counts"]["unsupported_probes"] == 0 else "PARTIAL"
        report["placement_rule_status"] = "UNKNOWN_COMPONENT_DEPENDENCY_ONLY"
        report["baseline_reference_report_status"] = baseline_report["status"]
    except (reference.ReferenceError, TransplantError, OSError, ValueError,
            KeyError, TypeError, json.JSONDecodeError) as exc:
        report["status"] = "FAIL"
        report["errors"].append(str(exc))
    finally:
        report["counts"]["skipped_probes"] = max(
            0, len(TRANSPLANTS) - report["counts"]["probes_attempted"])
        if resolved is not None and rows is not None and before_sources is not None:
            try:
                after_sources = reference.audit_sources(resolved["corpus"], rows)
                if after_sources != before_sources:
                    raise TransplantError("source matrix changed between audits")
                report["source_audit"] = {"status": "PASS", "before_checked": len(before_sources),
                                          "after_checked": len(after_sources), "sources": before_sources}
                report["counts"]["private_source_checks_after"] = len(after_sources)
            except (reference.ReferenceError, TransplantError, OSError) as exc:
                report["status"] = "FAIL"
                report["source_audit"]["status"] = "FAIL"
                report["errors"].append(f"post-run source audit: {exc}")
        if resolved is not None and before_environment is not None:
            try:
                if reference.audit_environment(resolved) != before_environment:
                    raise TransplantError("reference environment changed between audits")
                report["environment_audit"]["status"] = "PASS"
            except (reference.ReferenceError, TransplantError, OSError) as exc:
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
                    raise TransplantError("matrix/oracle/report/PDF input changed between audits")
                report["input_audit"]["status"] = "PASS"
                report["input_audit"]["after_checked"] = len(after_inputs)
                report["counts"]["baseline_pdf_checks_after"] = len(baseline_pdfs)
            except (TransplantError, OSError) as exc:
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
        report["counts"]["skipped_probes"] = len(TRANSPLANTS)
        report["errors"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        counts = report["counts"]
        print(f"HN/C8 text probes [{report['status']}]: "
              f"{counts['probes_attempted']}/{counts['probes_planned']} attempted; "
              f"rule {report['placement_rule_status']}")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 0 if report["status"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())

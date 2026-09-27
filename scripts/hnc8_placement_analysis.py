#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Evaluate frozen geometry-only HN/C8 placement controls from the #107 oracle.

The committed metadata oracle is the only input. This diagnostic never reads
private documents or calls the reference converter. An observed transform is
used only as a comparison target, never as an input to a candidate predictor.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
ORACLE = ROOT / "tests/conformance/hnc8_layout_oracle.json"
ORACLE_SHA256 = "4b88befeecf9a68dd6eca4966c79ea8cdb130c43e3c6d92f4cb56fb34dfb665e"
MATRIX_SHA256 = "af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9"
MAX_ORACLE_BYTES = 4 * 1024 * 1024
DPI = 300
POINTS_PER_INCH = 72
TOLERANCE_PT = 0.001
HYPOTHESES = ("top-left", "center", "bottom-right")
AFFINE_HYPOTHESIS = "variant-affine-width-height-order"
AFFINE_FEATURES = ("intercept", "JPEG pixel width", "JPEG pixel height", "draw number")
PAGE_SPLITS = {
    "discovery": {
        "hn_a": (16, 22, 26, 29, 30, 31, 32, 33, 34, 35, 36, 41, 47, 48, 50),
        "c8": (1, 2, 3, 4, 5),
    },
    "validation": {
        "hn_a": (52, 53, 54, 57, 58, 60, 61),
        "c8": (6, 7),
    },
}
EXPECTED_EXTRA_DRAWS = {"discovery": 36, "validation": 14}


class AnalysisError(ValueError):
    """The pinned oracle or one of its selected observations is invalid."""


def initial_report(mode: str | None) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "protocol": "hnc8-placement-geometry-controls-v1",
        "status": "NOT_RUN",
        "placement_rule_status": "UNKNOWN_NOT_TESTED",
        "mode": mode,
        "oracle_sha256": ORACLE_SHA256,
        "matrix_sha256": MATRIX_SHA256,
        "tolerance_pt": TOLERANCE_PT,
        "scale_dpi": DPI,
        "candidate_inputs": ["MediaBox", "JPEG pixel width", "JPEG pixel height",
                             "draw number", "fixed 300 dpi", "HN-A/C8 variant"],
        "excluded_candidate_inputs": ["source ID", "page number", "image hash", "observed CTM"],
        "counts": {"private_comparisons": 0, "oracle_pages_compared": 0,
                   "oracle_additional_draws_compared": 0},
        "hypotheses": {},
        "errors": [],
    }


def _load_oracle() -> dict[str, dict]:
    digest = hashlib.sha256()
    data = bytearray()
    try:
        with ORACLE.open("rb") as source:
            while block := source.read(64 * 1024):
                data.extend(block)
                if len(data) > MAX_ORACLE_BYTES:
                    raise AnalysisError("committed #107 oracle exceeds size limit")
                digest.update(block)
    except OSError as exc:
        raise AnalysisError("committed #107 oracle is unavailable") from exc
    if digest.hexdigest() != ORACLE_SHA256:
        raise AnalysisError("committed #107 oracle SHA-256 differs from pin")
    try:
        document = json.loads(data)
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise AnalysisError("committed #107 oracle is malformed") from exc
    if document.get("schema_version") != 1 or document.get("matrix_sha256") != MATRIX_SHA256:
        raise AnalysisError("committed #107 oracle schema or matrix differs")
    cases = {case["case"]: case for case in document["cases"]}
    if set(cases) != {"hn_a", "c8", "hn_b"} or len(document["cases"]) != 3:
        raise AnalysisError("committed #107 oracle case set differs")
    return cases


def _checked_box(box: object) -> tuple[float, float, float, float]:
    if (not isinstance(box, list) or len(box) != 4 or
            any(type(value) not in (int, float) or not math.isfinite(value)
                or abs(value) > 1e9 for value in box)):
        raise AnalysisError("selected MediaBox is invalid")
    left, bottom, right, top = (float(value) for value in box)
    if not left < right or not bottom < top:
        raise AnalysisError("selected MediaBox has nonpositive size")
    return left, bottom, right, top


def predict(hypothesis: str, media_box: list[float], width: int,
            height: int) -> list[float]:
    """Predict a full PDF CTM from box and pixel dimensions only."""
    if hypothesis not in HYPOTHESES:
        raise AnalysisError("unrecognized frozen geometry-only hypothesis")
    left, bottom, right, top = _checked_box(media_box)
    if (type(width) is not int or type(height) is not int or
            width <= 0 or height <= 0 or width > 100_000 or height > 100_000):
        raise AnalysisError("selected JPEG dimensions are invalid")
    draw_width = width * POINTS_PER_INCH / DPI
    draw_height = height * POINTS_PER_INCH / DPI
    if hypothesis == "top-left":
        x, y = left, top
    elif hypothesis == "center":
        x = left + (right - left - draw_width) / 2
        y = bottom + (top - bottom + draw_height) / 2
    else:
        x, y = right - draw_width, bottom + draw_height
    return [draw_width, 0.0, 0.0, -draw_height, x, y]


def _solve_four(matrix: list[list[float]], target: list[float]) -> list[float]:
    """Solve a four-column normal system by partial pivoting on scaled features."""
    system = [row[:] + [value] for row, value in zip(matrix, target)]
    for column in range(4):
        pivot = max(range(column, 4), key=lambda row: abs(system[row][column]))
        if abs(system[pivot][column]) < 1e-12:
            raise AnalysisError("affine discovery features are rank deficient")
        system[column], system[pivot] = system[pivot], system[column]
        for row in range(column + 1, 4):
            factor = system[row][column] / system[column][column]
            for index in range(column, 5):
                system[row][index] -= factor * system[column][index]
    solution = [0.0] * 4
    for row in range(3, -1, -1):
        solution[row] = (system[row][4] - math.fsum(
            system[row][column] * solution[column] for column in range(row + 1, 4)
        )) / system[row][row]
    return solution


def fit_variant_affine(rows: list[dict]) -> dict[str, list[float]]:
    """Fit discovery x/y only; identifiers and hashes never enter features."""
    if len(rows) < 4:
        raise AnalysisError("affine discovery fit requires at least four draws")
    triples = [(row["width"], row["height"], row["draw_number"]) for row in rows]
    means = [math.fsum(values) / len(values) for values in zip(*triples)]
    scales = [max(abs(value - mean) for value in values)
              for values, mean in zip(zip(*triples), means)]
    if any(scale <= 0 for scale in scales):
        raise AnalysisError("affine discovery features are rank deficient")
    features = [[1.0, *(float(value - mean) / scale
                        for value, mean, scale in zip(triple, means, scales))]
                for triple in triples]
    normal = [[math.fsum(feature[i] * feature[j] for feature in features)
               for j in range(4)] for i in range(4)]
    coefficients = {}
    for coordinate, component in (("x", 4), ("y", 5)):
        rhs = [math.fsum(feature[index] * row["observed_ctm"][component]
                         for feature, row in zip(features, rows)) for index in range(4)]
        centered = _solve_four(normal, rhs)
        slopes = [centered[index] / scales[index - 1] for index in range(1, 4)]
        intercept = centered[0] - math.fsum(
            slope * mean for slope, mean in zip(slopes, means))
        coefficients[coordinate] = [intercept, *slopes]
    return coefficients


def predict_variant_affine(coefficients: dict[str, list[float]],
                           width: int, height: int, draw_number: int) -> list[float]:
    """Apply one variant's frozen fitted coefficients without outcome lookups."""
    if any(type(value) is not int or value <= 0 for value in (width, height, draw_number)):
        raise AnalysisError("affine predictor dimensions/order are invalid")
    features = (1, width, height, draw_number)
    x = math.fsum(a * b for a, b in zip(coefficients["x"], features))
    y = math.fsum(a * b for a, b in zip(coefficients["y"], features))
    return [width * POINTS_PER_INCH / DPI, 0.0, 0.0,
            -height * POINTS_PER_INCH / DPI, x, y]


def _selected_draws(cases: dict[str, dict], mode: str) -> tuple[list[dict], int]:
    if mode not in PAGE_SPLITS:
        raise AnalysisError("mode must be discovery or validation")
    rows = []
    selected_pages = 0
    for case_name, page_numbers in PAGE_SPLITS[mode].items():
        case = cases[case_name]
        pdf_pages = case["pdf_pages"]
        source_pages = case["source_pages"]
        for page_number in page_numbers:
            if page_number > len(pdf_pages) or page_number > len(source_pages):
                raise AnalysisError("frozen split page is absent from #107 oracle")
            pdf_page = pdf_pages[page_number - 1]
            source_page = source_pages[page_number - 1]
            if (pdf_page["page_number"] != page_number or
                    source_page["page_number"] != page_number or
                    len(pdf_page["draws"]) != len(source_page["images"]) or
                    len(pdf_page["draws"]) < 2):
                raise AnalysisError("selected page order or image count differs")
            _checked_box(pdf_page["media_box"])
            selected_pages += 1
            for image, draw in zip(source_page["images"][1:], pdf_page["draws"][1:]):
                if (image["record_type"] != 2 or draw["filter"] != "/DCTDecode" or
                        draw["xobject_type"] != "Image" or
                        (image["width"], image["height"]) != (draw["width"], draw["height"]) or
                        image["payload_sha256"] != draw["raw_stream_sha256"] or
                        image["image_number"] != draw["draw_number"]):
                    raise AnalysisError("selected additional JPEG identity differs")
                observed = draw["pdf_ctm"]
                if (not isinstance(observed, list) or len(observed) != 6 or
                        any(type(value) not in (int, float) or not math.isfinite(value)
                            for value in observed)):
                    raise AnalysisError("selected observed CTM is invalid")
                rows.append({"source_variant": case["source_variant"],
                             "source_id": case["source_id"],
                             "page_number": page_number,
                             "draw_number": draw["draw_number"],
                             "width": draw["width"], "height": draw["height"],
                             "media_box": pdf_page["media_box"],
                             "observed_ctm": observed})
    if len(rows) != EXPECTED_EXTRA_DRAWS[mode]:
        raise AnalysisError("frozen split additional draw count differs")
    return rows, selected_pages


def evaluate(mode: str, cases: dict[str, dict]) -> dict[str, Any]:
    report = initial_report(mode)
    rows, page_count = _selected_draws(cases, mode)
    report["counts"]["oracle_pages_compared"] = page_count
    report["counts"]["oracle_additional_draws_compared"] = len(rows)
    discovery_rows, discovery_pages = _selected_draws(cases, "discovery")
    variants = ("HN-A", "C8")
    coefficients = {variant: fit_variant_affine(
        [row for row in discovery_rows if row["source_variant"] == variant])
        for variant in variants}
    report["affine_fit"] = {
        "training_mode": "discovery", "training_pages": discovery_pages,
        "training_additional_draws": len(discovery_rows),
        "evidence_label": (
            "retrospective negative exploration; validation CTMs were already public and "
            "inspected, so held-out scores are descriptive, not independent validation"
        ),
        "method": "ordinary least squares; centered/scaled features, four-column normal equations with pivoting",
        "features": list(AFFINE_FEATURES),
        "formula": (
            "CTM=[width*72/300,0,0,-height*72/300,x,y]; "
            "x=beta_x[0]+beta_x[1]*width+beta_x[2]*height+beta_x[3]*draw_number; "
            "y=beta_y[0]+beta_y[1]*width+beta_y[2]*height+beta_y[3]*draw_number"
        ),
        "coefficients_by_variant": coefficients,
        "validation_fit_policy": "fit discovery only; held-out CTMs are comparison targets",
    }
    for hypothesis in (*HYPOTHESES, AFFINE_HYPOTHESIS):
        comparisons = []
        passing = 0
        largest_error = 0.0
        for row in rows:
            predicted = (predict_variant_affine(
                coefficients[row["source_variant"]], row["width"], row["height"],
                row["draw_number"]) if hypothesis == AFFINE_HYPOTHESIS else
                predict(hypothesis, row["media_box"], row["width"], row["height"]))
            error = [abs(a - b) for a, b in zip(predicted, row["observed_ctm"])]
            maximum = max(error)
            matched = maximum <= TOLERANCE_PT
            passing += matched
            largest_error = max(largest_error, maximum)
            comparisons.append({
                "source_variant": row["source_variant"],
                "source_id": row["source_id"],
                "page_number": row["page_number"], "draw_number": row["draw_number"],
                "predicted_ctm": predicted, "observed_ctm": row["observed_ctm"],
                "absolute_errors_pt": error, "max_absolute_error_pt": maximum,
                "matched_all_six": matched,
            })
        report["hypotheses"][hypothesis] = {
            "attempted": len(rows), "passing": passing,
            "failing": len(rows) - passing,
            "max_absolute_error_pt": largest_error,
            "evidence_label": (
                "retrospective negative exploration; not independent validation"
                if hypothesis == AFFINE_HYPOTHESIS else "fixed geometry-only control"
            ),
            "comparisons": comparisons,
            "counterexamples": [
                {"source_variant": row["source_variant"],
                 "page_number": row["page_number"], "draw_number": row["draw_number"],
                 "max_absolute_error_pt": row["max_absolute_error_pt"]}
                for row in comparisons if not row["matched_all_six"]
            ],
        }
    report["status"] = "UNKNOWN"
    report["placement_rule_status"] = "UNKNOWN_GEOMETRY_ONLY_CONTROLS"
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=tuple(PAGE_SPLITS))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    report = initial_report(args.mode)
    if args.mode is not None:
        try:
            report = evaluate(args.mode, _load_oracle())
        except (AnalysisError, KeyError, TypeError, IndexError) as exc:
            report["status"] = "FAIL"
            report["errors"].append(str(exc))
    if args.json:
        print(json.dumps(report, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    else:
        count = report["counts"]["oracle_additional_draws_compared"]
        print(f"HN/C8 geometry-only placement [{report['status']}]: "
              f"{count} committed-oracle additional draws; "
              f"private comparisons {report['counts']['private_comparisons']}")
        for error in report["errors"]:
            print(f"  {error}", file=sys.stderr)
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())

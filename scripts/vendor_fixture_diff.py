#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare validated, externally decoded fixture payloads one page at a time."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import time

import vendor_fixtures as fixtures


def identity(value):
    """Runtime identity excludes local storage paths, but retains content pins."""
    if isinstance(value, dict):
        if set(value) == {"scope", "path", "bytes", "sha256"}:
            return {k: value[k] for k in ("bytes", "sha256")}
        return {k: identity(v) for k, v in value.items()}
    if isinstance(value, list):
        return [identity(v) for v in value]
    return value


def pixels(left_store, right_store, left, right):
    if left["status"] != "PASS" or right["status"] != "PASS":
        return {"status": "UNAVAILABLE", "reference": left["status"], "candidate": right["status"]}
    a, b = left["raster"], right["raster"]
    if (a["width"], a["height"]) != (b["width"], b["height"]):
        return {"status": "DIFFERENT", "reason": "image dimensions differ"}
    if a != b or left["origin"] != right["origin"]:
        return {"status": "INCOMPARABLE", "reason": "pixel interpretation or acquisition origin differs"}
    # At most one reference page is retained; candidate reads stay chunk-bounded.
    reference = left_store.read(left["payload"], collect=True, cap=left_store.limits.artifact_bytes)
    view = memoryview(reference)
    offset, changed, first, previous_pixel = 0, 0, None, None
    row_bytes = (a["width"] * a["depth"] * a["channels"] + 7) // 8

    def consume(block):
        nonlocal offset, changed, first, previous_pixel
        end = offset + len(block)
        if view[offset:end] != block:
            for index, value in enumerate(block):
                difference = value ^ reference[offset + index]
                if not difference:
                    continue
                row, column_byte = divmod(offset + index, row_bytes)
                if a["depth"] == 1:
                    columns = [column_byte * 8 + bit for bit in range(8)
                               if difference & (128 >> bit)]
                else:
                    columns = [column_byte // (a["channels"] * (a["depth"] // 8))]
                for column in columns:
                    pixel = row * a["width"] + column
                    if pixel != previous_pixel:
                        changed += 1
                        previous_pixel = pixel
                        if first is None:
                            y = row if a["row_order"] == "top-to-bottom" else a["height"] - 1 - row
                            first = [column, y]
        offset = end

    right_store.read(right["payload"], consume=consume)
    return {"status": "DIFFERENT" if changed else "EQUAL", "changed_pixels": changed,
            "first_pixel": first, "pixels": a["width"] * a["height"]}


def first_difference(left, right):
    for index, (a, b) in enumerate(zip(left, right)):
        if a != b:
            return index
    return min(len(left), len(right)) if len(left) != len(right) else None


def text(left_store, right_store, left, right, include_context):
    if left["status"] != "PASS" or right["status"] != "PASS":
        return {"status": "UNAVAILABLE", "reference": left["status"], "candidate": right["status"]}
    # No normalization/reflow/OCR is introduced by the comparator.
    contract = ("mode", "coverage", "range", "mime", "encoding", "unicode_contract")
    if any(left[k] != right[k] for k in contract):
        return {"status": "INCOMPARABLE", "reason": "text acquisition or encoding differs"}
    raw = [s.read(v["raw"], collect=True, cap=s.limits.text_bytes)
           for s, v in ((left_store, left), (right_store, right))]
    position = first_difference(*raw)
    result = {"status": "EQUAL" if position is None else "DIFFERENT", "first_byte": position}
    if position is not None:
        values = [s.read(v["unicode"], collect=True, cap=s.limits.text_bytes).decode("utf-8")
                  for s, v in ((left_store, left), (right_store, right))]
        codepoint = first_difference(*values)
        result["first_codepoint"] = codepoint
        if include_context and codepoint is not None:
            start = max(0, codepoint - 16)
            result["context"] = {"start_codepoint": start,
                                 "reference": values[0][start:codepoint + 17],
                                 "candidate": values[1][start:codepoint + 17]}
    return result


def compare(reference_catalog=None, candidate_catalog=None, reference_roots=None,
            candidate_roots=None, *, limits=fixtures.Limits(), include_text_context=False):
    result = {"kind": "decoded-fixture-comparison", "status": "NOT_RUN", "error": None,
              "pages": [], "counts": {k: 0 for k in ("EQUAL", "DIFFERENT", "UNAVAILABLE", "INCOMPARABLE")},
              "integrity": []}
    if all(v is None for v in (reference_catalog, candidate_catalog, reference_roots, candidate_roots)):
        return result
    stores = []
    deadline = time.monotonic() + limits.seconds
    try:
        manifests, catalogs = [], []
        for catalog, roots in ((reference_catalog, reference_roots), (candidate_catalog, candidate_roots)):
            if not isinstance(catalog, (str, Path)) or not isinstance(roots, dict) or set(roots) != {"bundle", "source", "runtime"}:
                raise fixtures.FixtureError("both catalogs and bundle/source/runtime roots are required")
            counts = fixtures._counters()
            result["integrity"].append(counts)
            store = fixtures.Assets(roots, limits, counts, deadline)
            stores.append(store)
            public, manifest, _ = fixtures._verify(store, catalog)
            catalogs.append(public)
            manifests.append(manifest)
        left, right = manifests
        result["basis"] = [m["basis"] for m in manifests]
        result["review"] = [c["review"] for c in catalogs]
        result["bundles"] = [c["manifest"] for c in catalogs]
        if left["profile"] != right["profile"] or identity(left["runtime"]) != identity(right["runtime"]):
            result.update(status="INCOMPARABLE", reason="runtime or capture profile differs")
            return result
        if [s["id"] for s in left["sources"]] != [s["id"] for s in right["sources"]]:
            result.update(status="DIFFERENT", reason="source identities or order differ")
            return result
        source_contract = ("variant", "source_pages", "vendor_pages", "output_pages", "coverage")
        for source, candidate in zip(left["sources"], right["sources"]):
            if (identity(source["file"]) != identity(candidate["file"])
                    or any(source[k] != candidate[k] for k in source_contract)):
                result.update(status="DIFFERENT", reason="source content, page counts or coverage differ")
                return result
            for page, other in zip(source["pages"], candidate["pages"]):
                if any(page[k] != other[k] for k in ("source_page", "vendor_page", "output_page")):
                    result.update(status="DIFFERENT", reason="source/output page mapping differs")
                    return result
        for source, candidate in zip(left["sources"], right["sources"]):
            for page, other in zip(source["pages"], candidate["pages"]):
                same_capture = all(page[k] == other[k] for k in ("box", "rotation", "capture"))
                image_result = (pixels(*stores, page["image"], other["image"]) if same_capture else
                                {"status": "INCOMPARABLE", "reason": "page geometry or capture settings differ"})
                text_result = text(*stores, page["text"], other["text"], include_text_context)
                record = {"source": source["id"], "page": page["source_page"],
                          "image": image_result, "text": text_result}
                result["pages"].append(record)
                for observation in (image_result, text_result):
                    result["counts"][observation["status"]] += 1
        counts = result["counts"]
        result["status"] = ("DIFFERENT" if counts["DIFFERENT"] else
                            "INCOMPARABLE" if counts["INCOMPARABLE"] else
                            "UNAVAILABLE" if any(p["image"]["status"] == "UNAVAILABLE" for p in result["pages"]) else
                            "EQUAL" if counts["EQUAL"] else "UNAVAILABLE")
    except (fixtures.FixtureError, OSError, KeyboardInterrupt) as error:
        result.update(status="ERROR", error=str(error)[:512] if isinstance(error, fixtures.FixtureError)
                      else "I/O failure or interrupted comparison")
    finally:
        for store in stores:
            try:
                store.audit()
            except (fixtures.FixtureError, OSError, KeyboardInterrupt):
                result.update(status="ERROR", error=result["error"] or "final fixture integrity audit failed")
            finally:
                store.close()
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for side in ("reference", "candidate"):
        parser.add_argument("--" + side, type=Path, help="catalog.json for this bundle")
        for scope in ("bundle", "source", "runtime"):
            parser.add_argument(f"--{side}-{scope}", type=Path, help=f"{scope} root for this bundle")
    parser.add_argument("--text-context", action="store_true", help="include up to 33 private text code points per side")
    args = parser.parse_args(argv)
    roots = []
    for side in ("reference", "candidate"):
        values = {scope: getattr(args, side + "_" + scope) for scope in ("bundle", "source", "runtime")}
        roots.append(values if any(v is not None for v in values.values()) else None)
    result = compare(args.reference, args.candidate, *roots, include_text_context=args.text_context)
    print(json.dumps(result, ensure_ascii=True, sort_keys=True))
    return {"EQUAL": 0, "NOT_RUN": 0, "DIFFERENT": 1, "ERROR": 2,
            "INCOMPARABLE": 3, "UNAVAILABLE": 3}[result["status"]]


if __name__ == "__main__":
    raise SystemExit(main())

# SPDX-License-Identifier: MIT
"""Independent source/page and complete-grid validation for original controls."""

from __future__ import annotations

import hashlib
from fractions import Fraction
from capability_protocol import DISPLAY, PAGE_KEYS, Refusal, require, validate_observation


def expected_grid(key, fixture_module):
    """Rasterize authored rectangles; no vendor or installed font is consulted."""
    require(key in PAGE_KEYS, "unknown-original-page")
    width, height = 1026, 769
    pixels = bytearray(b"\xff" * (width * height * 3))
    if key != "digital-2":
        text = ((24, 125, "RUST 321"),) if key == "digital-3" else fixture_module.TEXT
        for x, y, box_width, box_height, color in fixture_module.rectangles(text):
            left, right = round(x * 4), round((x + box_width) * 4)
            top, bottom = height - round((y + box_height) * 4), height - round(y * 4)
            for row in range(top, bottom):
                begin = (row * width + left) * 3
                pixels[begin:begin + (right - left) * 3] = bytes(color) * (right - left)
    if key == "digital-3":
        # PDF clockwise 90 degree orientation, preserving every pixel.
        rotated = bytearray(len(pixels))
        for y in range(height):
            for x in range(width):
                source = (y * width + x) * 3
                target = (x * height + (height - 1 - y)) * 3
                rotated[target:target + 3] = pixels[source:source + 3]
        return bytes(rotated), height, width
    return bytes(pixels), width, height


def crop(pixels, area, width=DISPLAY[0], height=DISPLAY[1]):
    x, y, w, h = area
    require(len(pixels) == width * height * 3 and x >= 0 and y >= 0 and x + w <= width and y + h <= height,
            "invalid-decoded-grid")
    return b"".join(pixels[((y + row) * width + x) * 3:((y + row) * width + x + w) * 3]
                    for row in range(h))


def verify_page_frame(raw_ppm, key, contract, fixture_module):
    header = b"P6\n1600 1200\n255\n"
    require(type(raw_ppm) is bytes and len(raw_ppm) == len(header) + DISPLAY[0] * DISPLAY[1] * 3
            and raw_ppm.startswith(header), "full-display-capture-layout")
    pixels = raw_ppm[len(header):]
    x, y, width, height = contract["rect"]
    require(x > 0 and y > 0 and x + width < DISPLAY[0] and y + height < DISPLAY[1], "page-boundary-unobservable")
    grid = crop(pixels, contract["rect"])
    expected, expected_width, expected_height = expected_grid(key, fixture_module)
    require((width, height) == (expected_width, expected_height) and grid == expected,
            "original-full-grid-mismatch")
    # Every exterior boundary pixel must have the separately frozen chrome
    # colour, including four corners. Blank-page completeness uses this ring
    # plus verified source/page/physical-extent evidence, never a white crop.
    outside = bytes(contract["outside_rgb"])
    top = crop(pixels, [x - 1, y - 1, width + 2, 1])
    bottom = crop(pixels, [x - 1, y + height, width + 2, 1])
    left = crop(pixels, [x - 1, y, 1, height])
    right = crop(pixels, [x + width, y, 1, height])
    require(top == outside * (width + 2) and bottom == outside * (width + 2)
            and left == outside * height and right == outside * height, "complete-page-boundary-mismatch")
    points = list(reversed(contract["points"])) if contract["rotation"] == 90 else contract["points"]
    scales = [Fraction(length) / Fraction(point) for length, point in zip((width, height), points)]
    return {"status": "COMPLETE_PAGE_OBSERVED", "origin": "complete-page-capture",
            "raw": {"size_bytes": len(raw_ppm), "sha256": hashlib.sha256(raw_ppm).hexdigest()},
            "decoded": {"width": width, "height": height, "layout": "RGB8-row-major",
                        "size_bytes": len(grid), "sha256": hashlib.sha256(grid).hexdigest()},
            "page_key": key, "physical_points": contract["points"], "rotation": contract["rotation"],
            "measured_pixels_per_point": [[scale.numerator, scale.denominator] for scale in scales],
            "all_boundary_pixels": "PASS", "four_corner_orientation": "PASS",
            "full_grid": "EXACT_ORIGINAL", "rescale_or_alignment": "NONE", "vendor_passes": 0}


class PageGate:
    """Independent ordered validation of raw desktop observations.

    Pixel-equivalent digital/negative and normal/large pages are disambiguated
    by live document/count/physical-extent bindings, not their raster hash.
    """

    def __init__(self, profile):
        self.profile = profile
        self.document = None
        self.document_generation = 0
        self.page_generation = 0
        self.next_index = 0
        self.last_serial = 0
        self.records = []
        self.owner = None

    def validate_raw(self, observations, contract, values):
        require(type(observations) is dict and set(observations) == set(contract) | {"serial", "owner"},
                "missing-raw-identity-evidence")
        serial = observations["serial"]
        require(type(serial) is int and serial > self.last_serial, "stale-desktop-observation")
        owner = observations["owner"]
        require(type(owner) is tuple and len(owner) == 2 and all(type(v) is int and v > 0 for v in owner),
                "missing-owned-window-evidence")
        evidence = {key: validate_observation(observations[key], binding, values) for key, binding in contract.items()}
        self.last_serial = serial
        return evidence, owner

    def opened(self, document, observations):
        require(document in ("digital.pdf", "image-only.pdf") and
                ((self.document is None and document == "digital.pdf") or
                 (self.document == "digital.pdf" and document == "image-only.pdf" and self.next_index == 4)),
                "document-transition-out-of-order")
        values = {"document": document, "count": 4 if document == "digital.pdf" else 1,
                  "width": "256.5", "height": "192.25"}
        evidence, owner = self.validate_raw(observations, self.profile["bindings"]["documents"][document], values)
        if self.records:
            require(owner == self.records[-1]["owner"], "document-owner-changed")
        self.document = document
        self.owner = owner
        self.document_generation += 1
        return evidence

    def page(self, key, observations):
        require(self.next_index < 5 and key == PAGE_KEYS[self.next_index], "physical-page-order")
        require(self.document == ("image-only.pdf" if key == "image-only-1" else "digital.pdf"), "wrong-source-document")
        points = self.profile["pages"][key]["points"]
        values = {"document": self.document, "page": 1 if key == "image-only-1" else self.next_index + 1,
                  "count": 1 if key == "image-only-1" else 4, "width": points[0], "height": points[1]}
        evidence, owner = self.validate_raw(observations, self.profile["bindings"]["pages"][key], values)
        require(owner == self.owner, "document-page-owner-changed")
        if self.records:
            require(owner == self.records[-1]["owner"], "page-owner-changed")
        self.page_generation += 1
        self.records.append({"key": key, "physical_index": 0 if key == "image-only-1" else self.next_index,
                             "source": self.profile["controls"][self.document], "source_name": self.document,
                             "physical_page_count": 1 if key == "image-only-1" else 4, "physical_points": points,
                             "ui_label": "raw-binding-retained", "document_generation": self.document_generation,
                             "navigation_generation": self.page_generation, "owner": owner, "evidence": evidence})
        self.next_index += 1
        return self.records[-1]

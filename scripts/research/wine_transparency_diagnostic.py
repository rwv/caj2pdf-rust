#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Wrap the original axis controls in PDF forms and transparency groups.

The input PDF must match the selected independently authored axis fixture.
This research helper keeps font programs and mathematical glyph transforms.
It is not a production rendering correction. Font/PDF bytes remain external.
"""

import argparse
from decimal import Decimal, localcontext
import hashlib
import json
from pathlib import Path
import subprocess

import fitz
import pikepdf

from wine_axis_rasters import axis_grid
from wine_type3_diagnostic import TEXT, character, fmt, font_hashes, sha


KINDS = ["form-direct", "isolated-direct", "isolated-split", "isolated-axes", "page-isolated"]


def generate(source, output, family, reverse, kind, alpha=Decimal(1)):
    if output.exists():
        raise ValueError("Output already exists")
    _, glyphs = axis_grid(family, reverse)
    records = []
    with pikepdf.open(source) as pdf, localcontext() as context:
        context.prec = 60
        if len(pdf.pages) != 1:
            raise ValueError("Expected a one-page control")
        page = pdf.pages[0]
        media_box = list(page.MediaBox)
        resources = page.Resources
        before = font_hashes(pdf)
        content = page.Contents.read_bytes()
        matches = list(TEXT.finditer(content))
        if len(matches) != len(glyphs):
            raise ValueError("Input glyph count does not match the selected fixture")
        forms = pikepdf.Dictionary()

        def form(body, box):
            obj = pikepdf.Stream(pdf, body)
            obj.Type = pikepdf.Name.XObject
            obj.Subtype = pikepdf.Name.Form
            obj.FormType = 1
            obj.BBox = pikepdf.Array(box)
            # Independent dictionary avoids a recursive resource graph when the
            # page's resources are subsequently replaced.
            obj.Resources = pikepdf.Dictionary({str(k): v for k, v in resources.items()})
            if kind != "form-direct":
                obj.Group = pikepdf.Dictionary(S=pikepdf.Name.Transparency,
                                               I=True, K=False, CS=pikepdf.Name.DeviceRGB)
            if alpha != 1:
                obj.Resources.ExtGState = pikepdf.Dictionary(
                    DiagnosticAlpha=pdf.make_indirect(pikepdf.Dictionary(
                        Type=pikepdf.Name.ExtGState, ca=alpha, CA=alpha)))
                obj.write(b"q /DiagnosticAlpha gs\n" + body + b"Q\n")
            return obj

        if kind == "page-isolated":
            forms.PageGroup = form(content, media_box)
            changed = b"q /PageGroup Do Q\n"
        else:
            def replace(match):
                index = len(records)
                glyph = glyphs[index]
                a, d, x, y = map(lambda value: Decimal(value.decode()), match.group(2, 3, 4, 5))
                sx = sy = Decimal(1)
                outer_x, outer_y = x, y
                if kind == "isolated-split":
                    outer_x = Decimal(glyph["x"] + 20) * 72 / 742
                    outer_y = Decimal(str(media_box[3])) - Decimal(glyph["y"]) * 72 / 742
                elif kind == "isolated-axes":
                    sx, sy = a, d
                tx, ty = (x - outer_x) / sx, (y - outer_y) / sy
                inner_a, inner_d = a / sx, d / sy
                expected = [a, d, x, y]
                reconstructed = [sx * inner_a, sy * inner_d, sx * tx + outer_x, sy * ty + outer_y]
                if max(abs(v - w) for v, w in zip(expected, reconstructed)) >= Decimal("1e-25"):
                    raise ValueError("Transform decomposition changed mathematical geometry")
                body = ("BT " + match[1].decode() + " 1 Tf\n"
                        + " ".join([fmt(inner_a), "0", "0", fmt(inner_d), fmt(tx), fmt(ty)])
                        + " Tm <" + match[6].decode() + "> Tj ET\n").encode()
                # Generous bounds isolate compositing without clipping the glyph.
                box = [tx - 2*inner_a, ty - 2*inner_d, tx + 3*inner_a, ty + 3*inner_d]
                name = f"/Glyph{index}"
                forms[name] = form(body, box)
                outer = [sx, Decimal(0), Decimal(0), sy, outer_x, outer_y]
                records.append(dict(index=index, code=glyph["code"], font=match[1].decode(),
                                    cid=match[6].decode(), before=[fmt(v) for v in expected],
                                    outer=[fmt(v) for v in outer],
                                    inner=[fmt(v) for v in [inner_a, inner_d, tx, ty]]))
                # Keep full decimal precision in stream operators. Form /Matrix
                # uses its identity default; dictionary-number rounding cannot
                # change the glyph transform.
                return ("q " + " ".join(fmt(v) for v in outer)
                        + " cm " + name + " Do Q").encode()

            changed, count = TEXT.subn(replace, content)
            if count != len(glyphs):
                raise ValueError("Not every glyph was wrapped")
        page.Contents = pikepdf.Stream(pdf, changed)
        page.Resources = pikepdf.Dictionary(XObject=forms)
        pdf.save(output, min_version="1.4", deterministic_id=True)
    with pikepdf.open(output) as candidate:
        if font_hashes(candidate) != before or list(candidate.pages[0].MediaBox) != media_box:
            raise ValueError("Font programs or page bounds changed")
    subprocess.run(["qpdf", "--check", str(output)], capture_output=True, check=True)
    expected_text = "".join(character(g["code"]) for g in glyphs)
    with fitz.open(source) as baseline, fitz.open(output) as candidate:
        old = "".join(baseline[0].get_text().split())
        new = "".join(candidate[0].get_text().split())
        if old != new or new != expected_text:
            raise ValueError("Semantic text identity changed")
    return dict(kind=kind, family=family, reverse=reverse, alpha=str(alpha),
                input_sha256=sha(source), output_sha256=sha(output), records=records,
                embedded_program_sha256=before, font_programs_unchanged=True,
                semantic_glyphs=len(glyphs), semantic_text_sha256=hashlib.sha256(expected_text.encode()).hexdigest(),
                transform_error_bound="1e-25 points", positive_control=alpha != 1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--family", choices=["low", "high"], required=True)
    parser.add_argument("--reverse", action="store_true")
    parser.add_argument("--kind", choices=KINDS, required=True)
    parser.add_argument("--alpha", type=Decimal, default=Decimal(1))
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    if not 0 < args.alpha <= 1:
        parser.error("alpha must be greater than zero and at most one")
    receipt = generate(args.source, args.output, args.family, args.reverse, args.kind, args.alpha)
    if args.receipt:
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: receipt[key] for key in ["output_sha256", "semantic_glyphs", "positive_control"]}))

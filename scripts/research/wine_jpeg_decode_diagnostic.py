#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Create JPEG/Flate/interpolation controls from an external PDF.

Research only: this is not a converter or a codec-conformance checker. Inputs
must contain only direct grayscale JPEG image XObjects, or RGB with --allow-rgb.
No document, font,
decoded image or output PDF belongs in the source repository.
"""

import argparse
import hashlib
from io import BytesIO
import json
from pathlib import Path
import subprocess
import zlib

import PIL
from PIL import Image, features
import pikepdf


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def preserved(pdf):
    """Record page bounds, decoded drawing operators and font program bytes."""
    return {
        "boxes": [[str(value) for value in page.MediaBox] for page in pdf.pages],
        "contents": [
            hashlib.sha256(page.Contents.read_bytes()).hexdigest()
            for page in pdf.pages
        ],
        "fonts": sorted(
            hashlib.sha256(obj.read_bytes()).hexdigest()
            for obj in pdf.objects
            if isinstance(obj, pikepdf.Stream) and obj.get("/Length1")
        ),
    }


def generate(source, output_directory, allow_rgb=False):
    output_directory.mkdir(exist_ok=False)
    files = []
    for kind in ["baseline", "flate", "jpeg-interpolate", "flate-interpolate", "flate-positive"]:
        output = output_directory / (kind + ".pdf")
        images = []
        with pikepdf.open(source) as pdf:
            before = preserved(pdf)
            for number, page in enumerate(pdf.pages, 1):
                for name, obj in page.Resources.get("/XObject", {}).items():
                    color_space = obj.get("/ColorSpace")
                    mode = "RGB" if allow_rgb and color_space == pikepdf.Name.DeviceRGB else "L"
                    channels = 3 if mode == "RGB" else 1
                    parameters = obj.get("/DecodeParms")
                    supported_parameters = parameters is None or (
                        mode == "RGB" and isinstance(parameters, pikepdf.Dictionary)
                        and dict(parameters) == {"/ColorTransform": 1}
                    )
                    if not (
                        obj.Subtype == pikepdf.Name.Image
                        and obj.Filter == pikepdf.Name.DCTDecode
                        and color_space == (pikepdf.Name.DeviceRGB if mode == "RGB" else pikepdf.Name.DeviceGray)
                        and obj.BitsPerComponent == 8
                        and supported_parameters
                        and "/Decode" not in obj
                        and "/Mask" not in obj
                        and "/SMask" not in obj
                    ):
                        raise ValueError("Expected an unmasked grayscale JPEG (or RGB with --allow-rgb)")
                    width, height = int(obj.Width), int(obj.Height)
                    if not (0 < width * height <= 10_000_000):
                        raise ValueError("Image exceeds the diagnostic pixel bound")
                    encoded = obj.read_raw_bytes()
                    with Image.open(BytesIO(encoded)) as image:
                        if image.size != (width, height) or image.mode != mode:
                            raise ValueError("JPEG dimensions or mode disagree")
                        image.load()
                        raw = image.tobytes()
                    if len(raw) != width * height * channels:
                        raise ValueError("Decoded image byte count disagrees")
                    images.append({
                        "page": number,
                        "resource": str(name),
                        "width": width,
                        "height": height,
                        "jpeg_sha256": hashlib.sha256(encoded).hexdigest(),
                        "jpeg_bytes": len(encoded),
                        "decoded_sha256": hashlib.sha256(raw).hexdigest(),
                        "decoded_bytes": len(raw),
                    })
                    if allow_rgb:
                        images[-1].update(
                            color_space=str(color_space), decoded_mode=mode, channels=channels,
                            color_transform=None if parameters is None else int(parameters.ColorTransform),
                        )
                    if kind == "flate-positive":
                        # Deliberately invert sample values. This is an image
                        # rendering control, never a fidelity candidate.
                        raw = raw.translate(bytes(reversed(range(256))))
                        images[-1]["positive_sha256"] = hashlib.sha256(raw).hexdigest()
                    if kind.startswith("flate"):
                        obj.write(zlib.compress(raw), filter=pikepdf.Name.FlateDecode)
                        if "/DecodeParms" in obj:
                            del obj["/DecodeParms"]
                        if obj.read_bytes() != raw:
                            raise ValueError("Flate round trip changed decoded bytes")
                    if kind.endswith("interpolate"):
                        obj.Interpolate = True
            if preserved(pdf) != before:
                raise ValueError("Page bounds, operators or font bytes changed")
            pdf.save(output, deterministic_id=True)
        with pikepdf.open(output) as pdf:
            if preserved(pdf) != before:
                raise ValueError("Serialized PDF changed preserved content")
        subprocess.run(["qpdf", "--check", str(output)], capture_output=True, check=True)
        files.append({
            "kind": kind,
            "filename": output.name,
            "sha256": digest(output),
            "bytes": output.stat().st_size,
            "images": images,
            "preserved": before,
        })
    receipt = {
        "input_pdf_sha256": digest(source),
        "decoding": {
            "pillow": PIL.__version__,
            "jpeg": features.version("jpg"),
            "libjpeg_turbo": features.version("libjpeg_turbo"),
            "pikepdf": pikepdf.__version__,
            "zlib": zlib.ZLIB_VERSION,
        },
        "files": files,
    }
    (output_directory / "manifest.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input_pdf", type=Path)
    parser.add_argument("output_directory", type=Path)
    parser.add_argument("--allow-rgb", action="store_true", help="Also admit direct DeviceRGB JPEGs")
    arguments = parser.parse_args()
    receipt = generate(arguments.input_pdf, arguments.output_directory, arguments.allow_rgb)
    print(f"Created {len(receipt['files'])} diagnostic PDFs; no viewer result implied")

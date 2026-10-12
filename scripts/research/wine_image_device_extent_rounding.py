#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Bound binary64 extent multiplication at a declared device scale."""
from pathlib import Path
from fractions import Fraction
from decimal import Decimal
import hashlib, math, re, subprocess
import pikepdf
IMAGE = re.compile(b'q\\s+([-0-9.]+) 0 0 ([-0-9.]+) ([-0-9.]+) ([-0-9.]+) cm (/(?:Im|GuardIm)\\d+) Do Q')

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def fonts(pdf):
    return sorted((hashlib.sha256(o.read_bytes()).hexdigest() for o in pdf.objects if isinstance(o, pikepdf.Stream) and o.get('/Length1')))

def convert(source, output, zoom, strict=False):
    source, output = (Path(source), Path(output))
    if output.exists() or not 1 <= zoom <= 1600:
        raise ValueError('Expected a new output and bounded declared zoom')
    q = Fraction(zoom, 75)
    records, afters, payloads = ([], [], [])
    with pikepdf.open(source) as pdf:
        boxes, original_fonts = ([list(p.MediaBox) for p in pdf.pages], fonts(pdf))
        for page_number, page in enumerate(pdf.pages, 1):
            images = page.Resources.XObject
            expected = {name for name in images.keys() if re.fullmatch('/(?:Im|GuardIm)\\d+', name)}
            seen = set()

            def replace(match):
                name = match[5].decode()
                assert name not in seen
                seen.add(name)
                payloads.append((page_number, name, hashlib.sha256(images[name].read_raw_bytes()).hexdigest()))
                tokens, changes = ([], [])
                for token in match.group(1, 2):
                    exact = Fraction(token.decode())
                    desired = round(abs(exact) * q)
                    if desired <= 0 or abs(abs(exact) * q - desired) > Fraction(1, 10 ** 20):
                        raise ValueError('Expected an integral declared device extent')
                    value = float(abs(exact))
                    initial = value
                    steps = 0
                    bound = Fraction(desired) / max(q, Fraction.from_float(float(q)))

                    def too_large():
                        if strict:
                            return Fraction.from_float(value) > bound or Fraction(format(Decimal(repr(value)), 'f')) > bound
                        return math.ceil(value * float(q)) > desired
                    while too_large():
                        value = math.nextafter(value, 0)
                        steps += 1
                        if steps > 4:
                            raise ValueError('Unexpected binary64 correction size')
                    assert math.ceil(value * float(q)) == desired
                    if steps or strict:
                        text = format(Decimal(repr(value if exact > 0 else -value)), 'f')
                        assert float(text) == (value if exact > 0 else -value)
                        tokens.append(text.encode())
                    else:
                        tokens.append(token)
                    changes.append(dict(desired_device_extent=desired, initial_binary64_extent=initial * float(q), final_binary64_extent=value * float(q), nextafter_steps=steps, before=token.decode(), after=tokens[-1].decode()))
                    if strict:
                        assert abs(Fraction(tokens[-1].decode())) * max(q, Fraction.from_float(float(q))) <= desired
                        assert Fraction.from_float(value) * max(q, Fraction.from_float(float(q))) <= desired
                        changes[-1].update(exact_coefficient_bound=str(bound), parsed_binary64=str(Fraction.from_float(value)), serialized_fraction=str(Fraction(tokens[-1].decode())))
                records.append(dict(page=page_number, image=name, axes=changes))
                return b'q\n' + tokens[0] + b' 0 0 ' + tokens[1] + b' ' + match[3] + b' ' + match[4] + b' cm ' + match[5] + b' Do Q'
            before = page.Contents.read_bytes()
            after, count = IMAGE.subn(replace, before)
            assert seen == expected and count == len(expected)
            assert IMAGE.sub(b'IMAGE', before) == IMAGE.sub(b'IMAGE', after)
            page.Contents = pikepdf.Stream(pdf, after)
            afters.append(after)
        pdf.save(output, deterministic_id=True)
    with pikepdf.open(output) as pdf:
        assert fonts(pdf) == original_fonts and [list(p.MediaBox) for p in pdf.pages] == boxes
        assert [p.Contents.read_bytes() for p in pdf.pages] == afters
        for page, name, checksum in payloads:
            assert hashlib.sha256(pdf.pages[page - 1].Resources.XObject[name].read_raw_bytes()).hexdigest() == checksum
    subprocess.run(['qpdf', '--check', str(output)], capture_output=True, check=True)
    result = dict(source_sha256=sha(source), output_sha256=sha(output), zoom=zoom, records=records, scope='Declared-device diagnostic only. Recover integral target extents from its matrices, then step binary64 coefficients toward zero only while their device-scale product would ceil beyond the target. Image samples, origins, font programs and other operators remain unchanged; this is not a zoom-independent production fix.')
    if strict:
        result.update(strict=True, scope='Declared-device diagnostic only. Bound both serialized decimal and parsed binary64 coefficients by target/max(exact scale, binary64 scale), so an unrounded product also cannot exceed the integer target. No screenshot-fitted epsilon; no source-image, origin, font or other-operator change. Still not a production correction.')
    return result

if __name__ == '__main__':
    import argparse, json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--zoom', type=int, required=True)
    parser.add_argument('--strict-bound', action='store_true')
    args = parser.parse_args()
    print(json.dumps(convert(args.source, args.output, args.zoom, args.strict_bound), indent=2))

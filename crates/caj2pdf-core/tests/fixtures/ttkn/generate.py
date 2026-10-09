# SPDX-License-Identifier: MIT
"""Authored PDF/TTKN inputs only. No bytes or credentials from an external source."""
import argparse, base64, hashlib, subprocess, tempfile, zlib
from pathlib import Path
HANDLER = b'<</Length 40 /SubFilter /TTKN.PubSec.s1 /CF <</DefaultCryptFilter <</CFM /AESV2 /Recipients [(AppendCA)]>>>> /Filter /TTKN.PubSec /StrF /DefaultCryptFilter /StmF /DefaultCryptFilter /EncryptMetadata true /R 2 /V 2>>'

def cbc(key, iv, data):
    assert len(data) % 16 == 0
    return subprocess.run(['openssl', 'enc', f'-aes-{len(key) * 8}-cbc', '-K', key.hex(), '-iv', iv.hex(), '-nopad'], input=data, check=True, capture_output=True).stdout

def pad(data):
    n = 16 - len(data) % 16
    return data + bytes([n]) * n

def generate(directory, variant, initial_iv):
    d = Path(directory)
    d.mkdir()
    file_id = hashlib.md5(('own-file-' + str(variant)).encode()).hexdigest()
    response = hashlib.md5(('own-response-' + str(variant)).encode()).hexdigest().encode()
    seed = hashlib.sha256(('own-seed-' + str(variant)).encode()).digest()
    iv = hashlib.sha256(('own-iv-' + str(variant)).encode()).digest()
    encrypt = hashlib.md5(('own-encrypt-' + str(variant)).encode()).hexdigest().encode()
    password_plain = seed + b'\x00' * 16
    password = cbc(response, initial_iv, password_plain)
    empty = f'<right-meta><file-id>{file_id}</file-id><file-app>Original MIT control {variant}</file-app><o-name>control-{variant}.pdf</o-name><version>2.0</version><protect><auth type="1"><permit type="3"><server><url>http://127.0.0.1:8123/auth</url></server><password>{base64.b64encode(password).decode()}</password></permit><iv>{base64.b64encode(iv).decode()}</iv></auth></protect><rights></rights></right-meta>'.encode()
    rights_xml = b'<rights><encrypt>' + encrypt + b'</encrypt><base-rights><print allow="1"><limit dpi="300" page="-1"/></print><copy><text allow="1"><limit char-count="-1"/></text><image allow="1"><limit dpi="300"/></image></copy><note><add allow="1"/><edit-org allow="1"/></note><valid><valid-date/><valid-open-times>-1</valid-open-times></valid></base-rights><user-rights><print allow="1"><limit dpi="300" page="-1"/></print><copy><text allow="1"><limit char-count="-1"/></text><image allow="1"/></copy><note><add allow="1"/><edit-org allow="1"/></note><valid><valid-date/><valid-open-times>-1</valid-open-times></valid></user-rights></rights>'
    assert len(rights_xml) < 704
    rights_plain = rights_xml + b'\x00' * (704 - len(rights_xml))
    rights = cbc(hashlib.sha256(seed + empty).digest(), iv[:16], rights_plain)
    xml = empty.replace(b'<rights></rights>', b'<rights>' + base64.b64encode(rights) + b'</rights>')
    file_key = hashlib.sha1(encrypt + b'AppendCA').digest()[:16]

    def encrypt_object(obj, plain):
        key = hashlib.md5(file_key + obj.to_bytes(3, 'little') + b'\x00\x00sAlT').digest()
        block_iv = hashlib.sha256(f'own-object-{variant}-{obj}'.encode()).digest()[:16]
        return block_iv + cbc(key, block_iv, pad(plain))
    content = f'q 1 0 0 rg 20 20 70 40 re f 0 0 1 rg 120 20 70 40 re f Q BT /F1 20 Tf 20 180 Td (CONTROL {variant}) Tj ET\n'.encode()
    stream = encrypt_object(4, zlib.compress(content))
    title = encrypt_object(7, b'\xfe\xff' + f'Authored outline {variant}'.encode('utf-16be'))
    objects = [b'<</Type /Catalog /Pages 2 0 R /Outlines 6 0 R /PageMode /UseOutlines>>', b'<</Type /Pages /Kids [3 0 R] /Count 1>>', b'<</Type /Page /Parent 2 0 R /MediaBox [0 0 300 240] /Resources <</Font <</F1 5 0 R>>>> /Contents 4 0 R>>', b'<</Length ' + str(len(stream)).encode() + b' /Filter /FlateDecode>>\nstream\n' + stream + b'\nendstream', b'<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>', b'<</Type /Outlines /First 7 0 R /Last 7 0 R /Count 1>>', b'<</Title <' + title.hex().encode() + b'> /Parent 6 0 R /Dest [3 0 R /Fit]>>', HANDLER]
    with (d / 'source.pdf').open('xb') as f:
        f.write(b'%PDF-1.6\n%\xe2\xe3\xcf\xd3\n')
        offsets = []
        for n, obj in enumerate(objects, 1):
            offsets.append(f.tell())
            f.write(str(n).encode() + b' 0 obj\n' + obj + b'\nendobj\n')
        xref = f.tell()
        f.write(b'xref\n0 9\n0000000000 65535 f \n')
        for offset in offsets:
            f.write(f'{offset:010d} 00000 n \n'.encode())
        f.write(b'trailer\n<</Root 1 0 R /Size 9 /Encrypt 8 0 R /ID [<' + b'0' * 32 + b'><' + b'1' * 32 + b'>]>>\nstartxref\n' + str(xref).encode() + b'\n%%EOF\nWebFastLoad\x00')
        xml_offset = f.tell()
        f.write(xml)
        f.write(f'startrights {xml_offset},{len(xml)}'.encode())
    (d / 'response.txt').write_bytes(response + b'\n')
    (d / 'content.txt').write_bytes(content)
    return d
if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    destination = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix='caj2pdf-authored-ttkn-') as temporary:
        generated = generate(Path(temporary) / 'control', 3, b'200CFC8299B84aa9')
        for name, source in [('authored.pdf', 'source.pdf'), ('response.txt', 'response.txt'), ('content.txt', 'content.txt')]:
            expected = (generated / source).read_bytes()
            if args.check:
                assert (destination / name).read_bytes() == expected, name + ' does not match original generator'
            else:
                (destination / name).write_bytes(expected)
    print('Original MIT TTKN fixture matches its generator.' if args.check else 'Generated original MIT TTKN fixture.')

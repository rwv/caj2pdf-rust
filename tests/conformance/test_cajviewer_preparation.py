# SPDX-License-Identifier: MIT
"""Exercise archive/extraction failure policy with original stand-ins only."""

import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("original_vendor_preparation", ROOT / "tools/cajviewer/prepare.py")
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)
REAL_POPEN = subprocess.Popen


class PreparationTests(unittest.TestCase):
    def archive(self, path):
        with tarfile.open(path, "w") as archive:
            for name, data in (("opt/cajviewer/bin/original-control", b"original MIT stand-in, not executable"),
                               ("opt/cajviewer/doc/example.caj", b"original omitted marker, not CAJ")):
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))

    def producer(self, archive, exit_code=0):
        def start(*args, **kwargs):
            return REAL_POPEN([sys.executable, "-c",
                               "import sys,os; sys.stdout.buffer.write(open(sys.argv[1],'rb').read()); "
                               "sys.stdout.flush(); os.close(1); sys.stderr.write('original trailing diagnostic'); "
                               "sys.stderr.flush(); sys.exit(int(sys.argv[2]))", str(archive), str(exit_code)],
                              **kwargs)
        return start

    def test_opaque_original_extraction_omits_document_and_drains_trailing_stderr(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "original.tar"
            self.archive(archive)
            with mock.patch.object(PREPARE, "verify_installer", return_value={"size_bytes": 1, "sha256": "0" * 64}), \
                    mock.patch.object(PREPARE.subprocess, "Popen", side_effect=self.producer(archive)):
                report = PREPARE.extract(root / "original.deb", root / "tree")
            self.assertEqual((report["status"], report["app_launches"], report["vendor_passes"]), ("PASS", 0, 0))
            self.assertEqual(report["source_post_audit"], "PASS")
            self.assertFalse((root / "tree/opt/cajviewer/doc").exists())
            self.assertTrue((root / "tree/opt/cajviewer/bin/original-control").is_file())
            self.assertEqual((root / "tree-stderr.bin").read_bytes(), b"original trailing diagnostic")

    def test_failed_producer_preserves_first_receipt_diagnostics_and_post_audit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "original.tar"
            self.archive(archive)
            with mock.patch.object(PREPARE, "verify_installer", return_value={"size_bytes": 1, "sha256": "0" * 64}), \
                    mock.patch.object(PREPARE.subprocess, "Popen", side_effect=self.producer(archive, 7)):
                with self.assertRaises(PREPARE.CanaryError):
                    PREPARE.extract(root / "original.deb", root / "tree")
            report = json.loads((root / "tree-receipt.json").read_text())
            self.assertEqual((report["status"], report["producer_exit_code"], report["source_post_audit"]), ("FAIL", 7, "PASS"))
            self.assertEqual((root / "tree-stderr.bin").read_bytes(), b"original trailing diagnostic")

    def test_existing_receipt_stops_before_producer(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            receipt = root / "tree-receipt.json"
            receipt.write_text("original prior FAIL\n")
            with mock.patch.object(PREPARE, "verify_installer", return_value={"size_bytes": 1, "sha256": "0" * 64}), \
                    mock.patch.object(PREPARE.subprocess, "Popen") as producer:
                with self.assertRaises(FileExistsError):
                    PREPARE.extract(root / "original.deb", root / "tree")
            producer.assert_not_called()
            self.assertEqual(receipt.read_text(), "original prior FAIL\n")
            self.assertFalse((root / "tree").exists())

    def test_early_tar_failure_still_preserves_trailing_stderr_and_source_audit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "original-empty.tar"
            archive.write_bytes(b"")
            with mock.patch.object(PREPARE, "verify_installer", return_value={"size_bytes": 1, "sha256": "0" * 64}), \
                    mock.patch.object(PREPARE.subprocess, "Popen", side_effect=self.producer(archive, 7)):
                with self.assertRaises(tarfile.TarError):
                    PREPARE.extract(root / "original.deb", root / "tree")
            report = json.loads((root / "tree-receipt.json").read_text())
            self.assertEqual((report["status"], report["source_post_audit"]), ("FAIL", "PASS"))
            self.assertEqual((root / "tree-stderr.bin").read_bytes(), b"original trailing diagnostic")


if __name__ == "__main__":
    unittest.main()

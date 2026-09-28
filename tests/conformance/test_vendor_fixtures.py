# SPDX-License-Identifier: MIT
"""Original runtime-only schema, byte-integrity and immutable-version controls."""

from copy import deepcopy
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import vendor_fixtures as subject


class OriginalBundle:
    """Fixture compiler for invented bytes, not an acquisition implementation."""

    def __init__(self, directory, version=1):
        self.root = directory
        self.manifest, self.files = subject.control_data("original-control-v" + str(version), version)
        self.receipt = json.loads(self.files[("bundle", "receipt.json")])
        self.catalog = directory / "catalog.json"
        self.roots = {kind: directory / kind for kind in ("bundle", "source", "runtime")}
        self.emit()

    @property
    def pages(self):
        return self.manifest["sources"][0]["pages"]

    def asset(self, ref, data):
        ref.update(subject.digest(data))
        self.files[(ref["scope"], ref["path"])] = data

    def emit(self, bind=True):
        if bind:
            self.receipt["observations_sha256"] = subject.observations_sha256(self.manifest)
        self.asset(self.manifest["receipt"], subject.json_bytes(self.receipt))
        referenced = {(ref["scope"], ref["path"]) for ref in subject._file_refs(self.manifest)}
        for (scope, path), data in self.files.items():
            if scope == "bundle" and (scope, path) not in referenced:
                continue
            destination = self.roots[scope] / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        data = subject.json_bytes(self.manifest)
        (self.roots["bundle"] / "manifest.json").write_bytes(data)
        self.catalog.write_bytes(subject.json_bytes(subject.make_catalog(self.manifest, data, "ORIGINAL_CONTROL")))

    def verify(self, **kwargs):
        return subject.run(self.catalog, self.roots, **kwargs)


class VendorFixtures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = OriginalBundle(self.root / "control")

    def assert_fail(self, result, located=None):
        self.assertEqual(result["status"], "FAIL", result)
        if located:
            self.assertIn(located, result["error"])
        self.assertEqual(result["counts"]["process_launches"], 0)
        self.assertEqual(result["counts"]["vendor_passes"], 0)
        self.assertEqual(result["counts"]["comparisons"], 0)

    def assert_schema_failure(self, mutate, located=None):
        model = deepcopy(self.bundle.manifest)
        mutate(model)
        with self.assertRaises(subject.FixtureError) as context:
            subject.validate_manifest(model)
        if located:
            self.assertIn(located, str(context.exception))

    def test_no_input_is_schema_control_and_zero_external_work(self):
        with patch.object(subject.Assets, "__init__", side_effect=AssertionError("external open")):
            result = subject.run()
        self.assertEqual(result["status"], "NOT_RUN")
        self.assertEqual(result["schema_control"], "PASS")
        self.assertTrue(all(value == 0 for value in result["counts"].values()))

    def test_reduced_no_input_limits_return_located_failure_without_external_work(self):
        result = subject.run(limits=replace(subject.Limits(), pages=1))
        self.assertEqual(result["status"], "FAIL", result)
        self.assertEqual(result["schema_control"], "FAIL")
        self.assertTrue(all(value == 0 for value in result["counts"].values()))

    def test_complete_original_bundle_passes_integrity_only(self):
        result = self.bundle.verify()
        self.assertEqual(result["status"], "PASS", result)
        self.assertEqual(result["basis"], "original-synthetic")
        self.assertEqual(result["after_audit"], "PASS")
        self.assertEqual(result["counts"]["pages_verified"], 2)
        self.assertEqual(result["counts"]["text_records_verified"], 2)
        self.assertEqual(result["counts"]["image_payloads_verified"], 2)
        self.assertEqual(result["counts"]["vendor_passes"], 0)
        self.assertLessEqual(result["counts"]["maximum_request"], 65536)

    def test_encoded_pixels_unicode_and_normalization_are_distinct(self):
        page = self.bundle.pages[0]
        self.assertNotEqual(page["image"]["artifact"]["sha256"], page["image"]["payload"]["sha256"])
        self.assertNotEqual(page["text"]["unicode"]["sha256"], page["text"]["normalization"]["file"]["sha256"])
        decoded = self.bundle.files[("bundle", "text/1.utf8")].decode("utf-8")
        self.assertIn("\ufffd", decoded)  # A literal expected character is not a decoding repair.
        self.assertIn("\r\n", decoded)

    def test_json_refuses_duplicate_float_nonfinite_huge_integer_and_malformed(self):
        for data in (b'{"a":1,"a":2}', b'{"a":0.5}', b'{"a":NaN}', b'{"a":Infinity}',
                     b'{"a":9223372036854775808}', b'{"a":111111111111111111111}', b'{', b'"\xff"'):
            with self.subTest(data=data), self.assertRaises(subject.FixtureError):
                subject.decode_json(data)

    def test_json_depth_and_string_caps_are_preparse(self):
        limits = replace(subject.Limits(), depth=4, string_bytes=8, manifest_bytes=128)
        with patch.object(subject.json, "loads", side_effect=AssertionError("tree allocation")):
            for data in (b'[[[[[0]]]]]', b'"123456789"', b' ' * 129):
                with self.subTest(data=data), self.assertRaises(subject.FixtureError):
                    subject.decode_json(data, limits)
        self.assertEqual(subject.decode_json(b'{"s":"\\\"[]"}', limits), {"s": '"[]'})

    def test_unpaired_surrogate_is_not_a_valid_schema_string(self):
        self.assert_schema_failure(lambda m: m["runtime"]["settings"].update(locale="\ud800"), "bounded string")

    def test_schema_unknown_fields_modes_versions_and_decimal_caps(self):
        changes = [lambda m: m.update(extra=True), lambda m: m.update(schema="cajviewer-fixtures/2"),
                   lambda m: m["sources"][0]["pages"][0]["image"].update(origin="viewport-region"),
                   lambda m: m["sources"][0]["pages"][0]["text"].update(mode="native-unicode-assumed"),
                   lambda m: m["sources"][0]["pages"][0]["box"].update(value=["0", "0", "1e3", "2"]),
                   lambda m: m["sources"][0]["pages"][0]["capture"]["zoom"].update(value="1" * 49),
                   lambda m: m.update(version=True)]
        for change in changes:
            with self.subTest(change=changes.index(change)):
                self.assert_schema_failure(change)

    def test_unknown_observations_remain_null_and_requested_counts_fail(self):
        model = deepcopy(self.bundle.manifest)
        model["runtime"]["application"]["build"] = {"status": "UNAVAILABLE", "value": None}
        model["runtime"]["settings"]["qt_scale"] = {"status": "UNAVAILABLE", "value": None}
        subject.validate_manifest(model)
        self.assert_schema_failure(lambda m: m["runtime"]["application"]["build"].update(status="UNAVAILABLE", value="guessed"))
        self.assert_schema_failure(lambda m: m["sources"][0]["vendor_pages"].update(status="REQUESTED"))

    def test_missing_duplicate_reordered_and_extra_physical_pages_refused(self):
        for records in ([self.bundle.pages[0]], self.bundle.pages[::-1],
                        [self.bundle.pages[0], self.bundle.pages[0]], self.bundle.pages + [self.bundle.pages[0]]):
            with self.subTest(length=len(records)):
                self.assert_schema_failure(lambda m: m["sources"][0].update(pages=deepcopy(records)))

    def test_duplicate_output_mapping_refused(self):
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][1].update(output_page=1), "mapping")

    def test_partial_scope_is_explicit_and_repeated_pixels_do_not_deduplicate(self):
        model = deepcopy(self.bundle.manifest)
        source = model["sources"][0]
        source["coverage"] = {"kind": "subset", "source_pages": [2]}
        source["pages"] = [source["pages"][1]]
        source["vendor_pages"] = {"status": "UNAVAILABLE", "value": None}
        subject.validate_manifest(model)
        model = deepcopy(self.bundle.manifest)
        image = model["sources"][0]["pages"][0]["image"]
        model["sources"][0]["pages"][1]["image"] = deepcopy(image)
        subject.validate_manifest(model)
        self.assertEqual(len(subject.public_summary(model)["sources"][0]["pages"]), 2)

    def test_zero_output_rows_are_represented_in_six_to_two_mapping(self):
        model = deepcopy(self.bundle.manifest)
        source = model["sources"][0]
        first, last = source["pages"]
        source["source_pages"]["value"] = 6
        source["coverage"]["source_pages"] = list(range(1, 7))
        last["source_page"] = 6
        middle = []
        for index in range(2, 6):
            page = deepcopy(first)
            page.update(source_page=index, vendor_page=None, output_page=None)
            page["image"] = {key: "NOT_RUN" if key == "status" else None for key in page["image"]}
            page["text"] = {key: "NOT_RUN" if key == "status" else None for key in page["text"]}
            middle.append(page)
        source["pages"] = [first, *middle, last]
        subject.validate_manifest(model)
        self.assertEqual([p["output_page"] for p in subject.public_summary(model)["sources"][0]["pages"]], [1, None, None, None, None, 2])

    def test_grid_negative_overflow_alpha_and_exact_payload_length_refused(self):
        for field, value in (("width", -1), ("height", 32769), ("depth", 4), ("channels", 2), ("alpha", "straight")):
            self.assert_schema_failure(lambda m, f=field, v=value: m["sources"][0]["pages"][0]["image"]["raster"].update({f: v}))
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][0]["image"]["payload"].update(bytes=17), "payload length")
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][0]["image"]["raster"].update(width=32768, height=32768, depth=16, channels=4, alpha="straight"))

    def test_bilevel_padding_is_checked_across_read_chunks(self):
        image = self.bundle.pages[0]["image"]
        image["raster"].update(width=9, height=2, depth=1, channels=1)
        self.bundle.asset(image["payload"], bytes((0x80, 0x80, 0x55, 0x00)))
        self.bundle.emit()
        self.assertEqual(self.bundle.verify(limits=replace(subject.Limits(), read_chunk=3))["status"], "PASS")
        self.bundle.asset(image["payload"], bytes((0x80, 0x80, 0x55, 0x01)))
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "bilevel row padding")

    def test_rgba_and_big_endian_16bit_profiles_validate_complete_bytes(self):
        image = self.bundle.pages[0]["image"]
        image["raster"].update(width=1, height=1, depth=16, channels=4, alpha="straight")
        self.bundle.asset(image["payload"], b"\x12\x34\x56\x78\x9a\xbc\xde\xf0")
        self.bundle.emit()
        self.assertEqual(self.bundle.verify()["status"], "PASS")

    def test_invalid_utf8_and_truncated_utf16_cannot_be_repaired(self):
        text = self.bundle.pages[0]["text"]
        for encoding, raw in (("utf-8", b"\xe4\xb8"), ("utf-8", b"\xff"), ("utf-16-le", b"\x41")):
            with self.subTest(encoding=encoding, raw=raw):
                text["encoding"] = encoding
                self.bundle.asset(text["raw"], raw)
                self.bundle.emit()
                self.assert_fail(self.bundle.verify(), "strict encoding")

    def test_incremental_utf8_utf16_and_latin1_unicode_contract(self):
        text = self.bundle.pages[0]["text"]
        for encoding, value in (("utf-8", "A中\ufffd"), ("utf-16-le", "A中🙂"),
                                ("utf-16-be", "A中🙂"), ("latin-1", "café")):
            with self.subTest(encoding=encoding):
                text.update(encoding=encoding, code_points=len(value), normalization=None)
                self.bundle.asset(text["raw"], value.encode(encoding))
                self.bundle.asset(text["unicode"], value.encode("utf-8"))
                self.bundle.emit()
                (self.bundle.roots["bundle"] / "text/1.nfc.utf8").unlink(missing_ok=True)
                result = self.bundle.verify(limits=replace(subject.Limits(), read_chunk=3))
                self.assertEqual(result["status"], "PASS", result)

    def test_wrong_decoded_or_normalized_identity_refused(self):
        text = self.bundle.pages[0]["text"]
        self.bundle.asset(text["unicode"], b"changed decoded text")
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "decoded Unicode identities")
        self.bundle = OriginalBundle(self.root / "normalization")
        self.bundle.asset(self.bundle.pages[0]["text"]["normalization"]["file"], b"wrong normalization")
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "normalization identity")

    def test_newline_normalization_does_not_collapse_spaces_or_unicode(self):
        text = self.bundle.pages[0]["text"]
        value = "A\r\nB\rC  D\u0301"
        text.update(code_points=len(value))
        self.bundle.asset(text["raw"], value.encode())
        self.bundle.asset(text["unicode"], value.encode())
        text["normalization"]["name"] = "newline-lf"
        self.bundle.asset(text["normalization"]["file"], b"A\nB\nC  D\xcc\x81")
        self.bundle.emit()
        self.assertEqual(self.bundle.verify()["status"], "PASS")

    def test_stale_clipboard_and_two_empty_passes_are_refused(self):
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][0]["text"]["clipboard"].update(fresh=False), "freshness")
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][0]["text"]["clipboard"].update(sentinel_sha256=m["sources"][0]["pages"][0]["text"]["raw"]["sha256"]), "sentinel")
        self.assert_schema_failure(lambda m: m["sources"][0]["pages"][1]["text"].update(status="PASS"), "empty copy")

    def test_ocr_enhanced_repair_and_existing_modes_never_become_native_copy(self):
        for mode in subject.TEXT_MODES:
            model = deepcopy(self.bundle.manifest)
            model["sources"][0]["pages"][0]["text"]["mode"] = mode
            subject.validate_manifest(model)
            self.assertEqual(subject.public_summary(model)["sources"][0]["pages"][0]["text_mode"], mode)

    def test_path_traversal_absolute_empty_and_foreign_separator_refused(self):
        for path in ("../escape", "/absolute", "images//1", "images/./1", "C:x", "x\\y", "a/" * 17 + "b"):
            self.assert_schema_failure(lambda m, p=path: m["sources"][0]["pages"][0]["image"]["payload"].update(path=p))

    def test_symlink_final_component_and_intermediate_directory_refused(self):
        path = self.bundle.roots["bundle"] / "pixels/1.rgb"
        target = self.root / "elsewhere"
        target.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(target)
        self.assert_fail(self.bundle.verify(), "safely")
        path.unlink()
        path.write_bytes(target.read_bytes())
        pixels = path.parent
        external = self.root / "external-pixels"
        pixels.rename(external)
        pixels.symlink_to(external, target_is_directory=True)
        self.assert_fail(self.bundle.verify(), "safely")

    def test_root_symlink_fifo_extra_file_and_conflicting_refs_fail(self):
        alias = self.root / "alias"
        alias.symlink_to(self.bundle.roots["bundle"], target_is_directory=True)
        self.assert_fail(subject.run(self.bundle.catalog, {**self.bundle.roots, "bundle": alias}), "root")
        payload = self.bundle.roots["bundle"] / "pixels/1.rgb"
        payload.unlink()
        os.mkfifo(payload)
        self.assert_fail(self.bundle.verify(), "regular file")
        payload.unlink()
        payload.write_bytes(self.bundle.files[("bundle", "pixels/1.rgb")])
        extra = self.bundle.roots["bundle"] / "unrelated"
        extra.write_bytes(b"extra")
        self.assert_fail(self.bundle.verify(), "undeclared extra")
        extra.unlink()
        self.bundle.pages[1]["image"]["payload"].update(path="pixels/1.rgb")
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "conflicting")

    def test_source_runtime_artifact_and_payload_mutations_fail(self):
        refs = [self.bundle.manifest["sources"][0]["file"],
                self.bundle.manifest["runtime"]["application"]["installer"],
                self.bundle.pages[0]["image"]["artifact"], self.bundle.pages[0]["image"]["payload"]]
        for ref in refs:
            with self.subTest(scope=ref["scope"], path=ref["path"]):
                path = self.bundle.roots[ref["scope"]] / ref["path"]
                data = path.read_bytes()
                path.write_bytes(bytes((data[0] ^ 1,)) + data[1:])
                self.assert_fail(self.bundle.verify(), "SHA-256")
                path.write_bytes(data)

    def test_exact_lengths_refuse_truncation_and_tail(self):
        path = self.bundle.roots["bundle"] / "pixels/1.rgb"
        data = path.read_bytes()
        for wrong in (data[:-1], data + b"\x00"):
            path.write_bytes(wrong)
            self.assert_fail(self.bundle.verify(), "byte length")

    def test_stale_acquisition_receipt_rejects_changed_runtime_source_and_output(self):
        for mutate in (lambda b: b.manifest["runtime"]["settings"].update(locale="changed"),
                       lambda b: b.asset(b.manifest["sources"][0]["file"], b"different original source"),
                       lambda b: b.pages[0]["capture"]["dpi"].update(value="144")):
            bundle = OriginalBundle(self.root / ("stale-" + str(time.monotonic_ns())))
            mutate(bundle)
            bundle.emit(bind=False)
            self.assert_fail(bundle.verify(), "stale creator/runtime/source/page")

    def test_trusted_catalog_pin_refuses_fully_rewritten_metadata(self):
        pin = hashlib.sha256(self.bundle.catalog.read_bytes()).hexdigest()
        self.bundle.manifest["runtime"]["settings"]["locale"] = "new-declared-pin"
        self.bundle.emit()
        self.assertEqual(self.bundle.verify()["status"], "PASS")  # Integrity is not authenticity.
        self.assert_fail(self.bundle.verify(catalog_sha256=pin), "caller pin")

    def test_receipt_attempts_failures_caps_and_unavailable_memory_are_honest(self):
        receipt = deepcopy(self.bundle.receipt)
        receipt["resources"]["process_tree_memory"]["peak_bytes"] = 0
        with self.assertRaises(subject.FixtureError):
            subject._receipt(receipt, self.bundle.manifest, subject.Limits())
        receipt = deepcopy(self.bundle.receipt)
        receipt["counts"]["attempted"] = 1
        with self.assertRaises(subject.FixtureError):
            subject._receipt(receipt, self.bundle.manifest, subject.Limits())
        receipt = deepcopy(self.bundle.receipt)
        receipt["status"] = "FAIL"
        receipt["after_audit"] = "FAIL"
        receipt["resources"]["elapsed_ms"] = receipt["caps"]["wall_ms"] + 1
        subject._receipt(receipt, self.bundle.manifest, subject.Limits())

    def test_bound_passing_receipt_file_cap_covers_acquired_page_files(self):
        self.bundle.receipt["caps"]["file_bytes"] = 29  # Full PPM is 11 header + 18 pixel bytes.
        self.bundle.emit()
        self.assertEqual(self.bundle.verify()["status"], "PASS")
        self.assertGreater(self.bundle.manifest["runtime"]["application"]["installer"]["bytes"], 29)
        self.assertGreater(self.bundle.manifest["receipt"]["bytes"], 29)
        self.bundle.receipt["caps"]["file_bytes"] = 1
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "acquired page artifact")
        # A failed acquisition may preserve an observed over-cap file without
        # rewriting its failure or permitting regeneration promotion.
        self.bundle.receipt.update(status="FAIL", after_audit="FAIL")
        self.bundle.emit()
        result = self.bundle.verify()
        self.assertEqual(result["status"], "PASS", result)
        self.assertEqual(result["receipt_status"], "FAIL")
        self.assertEqual(result["counts"]["vendor_passes"], 0)

    def test_vendor_claim_cannot_use_original_control_or_unmeasured_pass(self):
        self.bundle.manifest["basis"] = "vendor-observation"
        self.bundle.emit()
        self.assert_fail(self.bundle.verify(), "original control")
        data = (self.bundle.roots["bundle"] / "manifest.json").read_bytes()
        self.bundle.catalog.write_bytes(subject.json_bytes(subject.make_catalog(self.bundle.manifest, data)))
        self.assert_fail(self.bundle.verify(), "process-tree memory")

    def test_short_reads_progress_and_total_io_limit(self):
        original = os.read
        with patch.object(subject.os, "read", side_effect=lambda fd, count: original(fd, min(count, 7))):
            result = self.bundle.verify()
        self.assertEqual(result["status"], "PASS", result)
        limited = self.bundle.verify(limits=replace(subject.Limits(), total_read_bytes=100))
        self.assert_fail(limited, "read limit")
        self.assertLessEqual(limited["counts"]["requested_bytes"], 100)

    def test_zero_overreport_and_raised_reads_retain_attempt_accounting(self):
        for effect in (lambda fd, count: b"", lambda fd, count: b"x" * (count + 1), OSError("original I/O failure")):
            with self.subTest(effect=effect), patch.object(subject.os, "read", side_effect=effect):
                result = self.bundle.verify()
            self.assert_fail(result)
            self.assertGreaterEqual(result["counts"]["read_calls"], 1)
            self.assertGreater(result["counts"]["requested_bytes"], 0)

    def test_mutation_during_read_and_after_verification_cannot_pass(self):
        original = os.read
        payload = self.bundle.roots["bundle"] / "pixels/1.rgb"
        mutated = False
        def replace_while_reading(fd, count):
            nonlocal mutated
            data = original(fd, count)
            if not mutated and os.fstat(fd).st_ino == payload.stat().st_ino:
                mutated = True
                payload.write_bytes(payload.read_bytes())  # Same bytes, modified identity.
            return data
        with patch.object(subject.os, "read", side_effect=replace_while_reading):
            self.assert_fail(self.bundle.verify(), "changed during")
        original_verify = subject._verify
        def change_after(*args, **kwargs):
            answer = original_verify(*args, **kwargs)
            payload.write_bytes(b"x" * 18)
            return answer
        with patch.object(subject, "_verify", side_effect=change_after):
            result = self.bundle.verify()
        self.assert_fail(result, "final integrity")
        self.assertEqual(result["after_audit"], "FAIL")

    def test_root_replacement_final_audit_is_located(self):
        original_verify = subject._verify
        bundle_root = self.bundle.roots["bundle"]
        def replace_root(*args, **kwargs):
            result = original_verify(*args, **kwargs)
            bundle_root.rename(bundle_root.with_name("moved"))
            bundle_root.mkdir()
            return result
        with patch.object(subject, "_verify", side_effect=replace_root):
            self.assert_fail(self.bundle.verify(), "final integrity")

    def test_byte_file_and_time_limits_and_incomplete_inputs_fail(self):
        for limits in (replace(subject.Limits(), manifest_bytes=32), replace(subject.Limits(), files=1),
                       replace(subject.Limits(), seconds=1e-9)):
            self.assert_fail(self.bundle.verify(limits=limits))
        self.assert_fail(subject.run(self.bundle.catalog, None), "explicit verification")
        self.assert_fail(subject.run(self.bundle.catalog, {**self.bundle.roots, "source": None}), "root")

    def test_partial_failure_counts_planned_and_unstarted_pages(self):
        (self.bundle.roots["bundle"] / "pixels/1.rgb").write_bytes(b"x" * 18)
        result = self.bundle.verify()
        self.assert_fail(result)
        self.assertEqual(tuple(result["counts"][name] for name in ("pages_planned", "pages_attempted", "pages_verified", "pages_failed", "pages_unstarted")), (2, 1, 0, 1, 1))

    def test_page_stage_interruption_counts_failure_and_runs_final_audit(self):
        with patch.object(subject, "_pixel_bytes", side_effect=KeyboardInterrupt):
            result = self.bundle.verify()
        self.assert_fail(result, "interrupted")
        self.assertEqual(tuple(result["counts"][name] for name in ("pages_planned", "pages_attempted", "pages_verified", "pages_failed", "pages_unstarted")), (2, 1, 0, 1, 1))
        self.assertEqual(result["after_audit"], "PASS")

    def test_final_audit_attempts_every_declared_file_after_first_failure(self):
        root = self.root / "audit-only"
        root.mkdir()
        counters = subject._counters()
        store = subject.Assets({"source": root}, subject.Limits(), counters, time.monotonic() + 60)
        self.addCleanup(store.close)
        for name in ("first", "last"):
            (root / name).write_bytes(b"wrong")
            store.declare({"scope": "source", "path": name, **subject.digest(b"right")})
        with self.assertRaises(subject.FixtureError):
            store.audit()
        self.assertEqual(counters["file_attempts"], 2)
        self.assertEqual(counters["file_checks_passed"], 0)
        self.assertEqual(counters["read_bytes"], 10)

    def test_generation_creates_readonly_runtime_controls_without_overwrite(self):
        output = self.root / "generated"
        result = subject.create_example(output)
        self.assertEqual(result["status"], "PASS", result)
        self.assertEqual(stat.S_IMODE((output / "catalog.json").stat().st_mode), 0o400)
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o500)
        self.assert_fail(subject.create_example(output), "fresh directory")
        self.assertEqual(subject.run(result["catalog"], result["roots"])["status"], "PASS")

    def test_writer_short_zero_raised_and_partial_failure_retention(self):
        original = os.write
        with patch.object(subject.os, "write", side_effect=lambda fd, data: original(fd, data[:3])):
            result = subject.create_example(self.root / "short")
        self.assertEqual(result["status"], "PASS", result)
        with patch.object(subject.os, "write", return_value=0):
            result = subject.create_example(self.root / "zero")
        self.assert_fail(result, "zero progress")
        self.assertTrue((self.root / "zero").is_dir())
        with patch.object(subject.os, "write", side_effect=OSError("invented write fault")):
            result = subject.create_example(self.root / "raised")
        self.assert_fail(result, "I/O failure")
        self.assertTrue((self.root / "raised").is_dir())
        result = subject.create_example(self.root / "budget", limits=replace(subject.Limits(), output_bytes=1200))
        self.assert_fail(result, "output budget")
        self.assertTrue((self.root / "budget/INCOMPLETE.json").is_file())

    def test_final_generation_deadline_marks_sealed_version_and_refuses_reuse(self):
        moment = [100.0]
        original_seal = subject.NewVersion.seal
        def expire_after_sealing(writer):
            original_seal(writer)
            moment[0] = 161.0
        output = self.root / "expired"
        with patch.object(subject.time, "monotonic", side_effect=lambda: moment[0]), \
                patch.object(subject.NewVersion, "seal", side_effect=expire_after_sealing, autospec=True):
            result = subject.create_example(output)
        self.assert_fail(result, "wall-time")
        self.assertEqual(result["incomplete_marker"], "WRITTEN")
        roots = {scope: output / scope for scope in ("bundle", "source", "runtime")}
        self.assert_fail(subject.run(output / "catalog.json", roots), "marked incomplete")

    def test_interruption_retains_read_attempts_and_fails(self):
        with patch.object(subject.os, "read", side_effect=KeyboardInterrupt):
            result = self.bundle.verify()
        self.assert_fail(result, "interrupted")
        self.assertGreater(result["counts"]["read_calls"], 0)


class Regeneration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.old = OriginalBundle(self.root / "old", 1)
        self.new = OriginalBundle(self.root / "candidate", 2)

    def regen(self, output="version-2", **kwargs):
        return subject.regenerate(self.old.catalog, self.old.roots, self.new.catalog, self.new.roots,
                                  self.root / output, "acquisition-improvement", "1" * 40, **kwargs)

    def tree_hashes(self, root):
        return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in root.rglob("*") if path.is_file()}

    def test_new_version_diff_history_and_predecessor_unchanged(self):
        before = self.tree_hashes(self.old.root)
        result = self.regen()
        self.assertEqual(result["status"], "PASS", result)
        self.assertEqual(result["review"], "REVIEW_REQUIRED")
        self.assertTrue(result["predecessor_preserved"])
        self.assertEqual(before, self.tree_hashes(self.old.root))
        produced = subject.decode_json((self.root / "version-2/bundle/manifest.json").read_bytes())
        self.assertTrue({"baseline-manifest", "baseline-receipt", "diff", "regeneration-receipt"} <= {h["kind"] for h in produced["history"]})
        self.assertEqual(subject.run(result["catalog"], result["roots"])["status"], "PASS")

    def test_exact_successful_review_is_bound_but_never_implicit(self):
        result = self.regen()
        review = {"schema": subject.SCHEMA, "decision": "APPROVE", "reviewer": "original-review-control", **result["pins"]}
        path = self.root / "review.json"
        path.write_bytes(subject.json_bytes(review))
        approved = self.regen("reviewed-version", review=path)
        self.assertEqual(approved["status"], "PASS", approved)
        self.assertEqual(approved["review"], "REVIEWED")
        checked = subject.run(approved["catalog"], approved["roots"])
        self.assertEqual(checked["status"], "PASS", checked)

    def test_unrelated_review_changed_code_diff_reason_and_failed_candidate_refused(self):
        first = self.regen()
        base = {"schema": subject.SCHEMA, "decision": "APPROVE", "reviewer": "invented-reviewer", **first["pins"]}
        for index, key in enumerate(("predecessor", "candidate", "diff", "code", "reason")):
            review = deepcopy(base)
            if key == "reason":
                review[key] = "viewer-runtime-change"
            else:
                review[key]["sha256"] = "f" * 64
            path = self.root / "review.json"
            path.write_bytes(subject.json_bytes(review))
            result = self.regen("bad-review-" + str(index), review=path)
            self.assertEqual(result["status"], "FAIL", result)
            self.assertIn("approval", result["error"])
            self.assertFalse((self.root / ("bad-review-" + str(index))).exists())
        self.new.receipt.update(status="FAIL", after_audit="FAIL")
        self.new.emit()
        result = self.regen("failed-candidate")
        self.assertEqual(result["status"], "FAIL", result)
        self.assertIn("acquisition is not PASS", result["error"])
        self.assertFalse((self.root / "failed-candidate").exists())

    def test_failed_baseline_receipt_and_all_history_remain_retained(self):
        self.old.receipt.update(status="FAIL", before_audit="FAIL")
        self.old.emit()
        result = self.regen()
        self.assertEqual(result["status"], "PASS", result)
        produced = subject.decode_json((self.root / "version-2/bundle/manifest.json").read_bytes())
        failures = [item for item in produced["history"] if item["kind"] == "failure-receipt"]
        self.assertEqual(len(failures), 1)
        raw = (self.root / "version-2/bundle" / failures[0]["file"]["path"]).read_bytes()
        self.assertEqual(json.loads(raw)["status"], "FAIL")
        self.assertEqual(hashlib.sha256(raw).hexdigest(), failures[0]["file"]["sha256"])

    def test_existing_or_nested_output_cannot_modify_a_baseline(self):
        first = self.regen()
        before = self.tree_hashes(self.old.root)
        again = self.regen()
        self.assertEqual(again["status"], "FAIL")
        nested = subject.regenerate(self.old.catalog, self.old.roots, self.new.catalog, self.new.roots,
                                    self.old.roots["bundle"] / "new-version", "acquisition-improvement", "1" * 40)
        self.assertEqual(nested["status"], "FAIL")
        self.assertIn("inside an input root", nested["error"])
        self.assertFalse((self.old.roots["bundle"] / "new-version").exists())
        self.assertEqual(before, self.tree_hashes(self.old.root))
        self.assertEqual(subject.run(first["catalog"], first["roots"])["status"], "PASS")

    def test_failed_copy_retains_partial_directory_and_final_audits(self):
        before = self.tree_hashes(self.old.root)
        with patch.object(subject.NewVersion, "copy", side_effect=OSError("invented copy failure")):
            result = self.regen()
        self.assertEqual(result["status"], "FAIL", result)
        self.assertEqual(result["after_audit"], "PASS")
        self.assertTrue((self.root / "version-2/INCOMPLETE.json").is_file())
        self.assertEqual(before, self.tree_hashes(self.old.root))
        self.assertFalse((self.root / "version-2/catalog.json").exists())

    def test_diff_is_redacted_bounded_and_not_a_review(self):
        model = deepcopy(self.old.manifest)
        model["runtime"]["settings"]["locale"] = "private-setting-must-not-be-public"
        data = subject.manifest_diff(self.old.manifest, model)
        self.assertNotIn(b"private-setting", data)
        self.assertEqual(json.loads(data)["changes"][0]["field"], "manifest/runtime/settings/locale")
        with self.assertRaises(subject.FixtureError):
            subject.manifest_diff(self.old.manifest, model, replace(subject.Limits(), receipt_bytes=32))
        model["runtime"]["settings"]["timezone"] = "different"
        with self.assertRaises(subject.FixtureError):
            subject.manifest_diff(self.old.manifest, model, replace(subject.Limits(), records=1))

    def test_review_state_cannot_be_blessed_by_rewriting_catalog(self):
        result = self.regen()
        path = Path(result["catalog"])
        os.chmod(path, 0o600)
        catalog = json.loads(path.read_bytes())
        catalog["review"] = "REVIEWED"
        path.write_bytes(subject.json_bytes(catalog))
        checked = subject.run(path, result["roots"])
        self.assertEqual(checked["status"], "FAIL", checked)
        self.assertIn("review state", checked["error"])

    def test_next_version_unique_identity_and_profile_required(self):
        for key, value in (("version", 1), ("bundle_id", self.old.manifest["bundle_id"]), ("profile", "different-profile")):
            candidate = OriginalBundle(self.root / ("bad-" + key), 2)
            candidate.manifest[key] = value
            candidate.receipt["bundle_id"] = candidate.manifest["bundle_id"]
            candidate.emit()
            result = subject.regenerate(self.old.catalog, self.old.roots, candidate.catalog, candidate.roots,
                                        self.root / ("refused-" + key), "acquisition-improvement", "1" * 40)
            self.assertEqual(result["status"], "FAIL", result)

    def test_multiple_generations_retain_all_failure_and_review_history(self):
        self.old.receipt.update(status="FAIL", before_audit="FAIL")
        self.old.emit()
        first = self.regen()
        self.assertEqual(first["status"], "PASS", first)
        next_candidate = OriginalBundle(self.root / "candidate-3", 3)
        next_result = subject.regenerate(first["catalog"], first["roots"], next_candidate.catalog, next_candidate.roots,
                                         self.root / "version-3", "viewer-runtime-change", "1" * 40)
        self.assertEqual(next_result["status"], "PASS", next_result)
        checked = subject.run(next_result["catalog"], next_result["roots"])
        self.assertEqual(checked["status"], "PASS", checked)
        manifest = json.loads((self.root / "version-3/bundle/manifest.json").read_bytes())
        self.assertEqual(sum(item["kind"] == "regeneration-receipt" for item in manifest["history"]), 2)
        self.assertEqual(sum(item["kind"] == "failure-receipt" for item in manifest["history"]), 1)

    def test_final_audit_failure_after_output_creation_is_unusable(self):
        audit = subject.Assets.audit
        def fail_only_generated(store):
            audit(store)
            if store.roots["bundle"][1].parent.name == "version-2":
                raise subject.FixtureError("invented final generated audit failure")
        with patch.object(subject.Assets, "audit", side_effect=fail_only_generated, autospec=True):
            result = self.regen()
        self.assertEqual(result["status"], "FAIL", result)
        self.assertEqual(result["after_audit"], "FAIL")
        self.assertEqual(result["incomplete_marker"], "WRITTEN")
        output = self.root / "version-2"
        roots = {**self.new.roots, "bundle": output / "bundle"}
        checked = subject.run(output / "catalog.json", roots)
        self.assertEqual(checked["status"], "FAIL", checked)
        self.assertIn("marked incomplete", checked["error"])

    def test_failed_page_cannot_become_a_regeneration_candidate(self):
        image = self.new.pages[0]["image"]
        self.new.pages[0]["image"] = {key: "FAIL" if key == "status" else None for key in image}
        for ref in (image["artifact"], image["payload"]):
            (self.new.roots["bundle"] / ref["path"]).unlink()
        self.new.emit()
        result = self.regen()
        self.assertEqual(result["status"], "FAIL", result)
        self.assertIn("failed acquisition", result["error"])


class ActualCliControls(unittest.TestCase):
    """Actual Python CLI child controls; no viewer/native/converter invocation."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def cli(self, *arguments):
        return subprocess.run([sys.executable, str(ROOT / "scripts/vendor_fixtures.py"), *map(str, arguments)],
                              stdin=subprocess.DEVNULL, capture_output=True, timeout=15, check=False,
                              env={**os.environ, "LC_ALL": "C.UTF-8"})

    def test_actual_no_argument_missing_input_and_bad_cli(self):
        result = self.cli("--json")
        self.assertEqual(result.returncode, 0)
        data = json.loads(result.stdout)
        self.assertEqual(data["status"], "NOT_RUN")
        self.assertTrue(all(value == 0 for value in data["counts"].values()))
        incomplete = self.cli("verify", "--catalog", self.root / "missing")
        self.assertEqual(incomplete.returncode, 2)
        self.assertIn(b"required", incomplete.stderr)

    def test_actual_original_generation_verify_and_regeneration(self):
        results = []
        for name, version in (("old", 1), ("candidate", 2)):
            command = self.cli("example", "--output", self.root / name, "--version", version, "--bundle-id", "original-v" + str(version))
            self.assertEqual(command.returncode, 0, command.stderr + command.stdout)
            results.append(json.loads(command.stdout))
        old, new = results
        def args(model, prefix=""):
            return ["--" + prefix + "catalog", model["catalog"], *[word for kind, path in model["roots"].items() for word in ("--" + prefix + kind + "-root", path)]]
        checked = self.cli("verify", *args(old))
        self.assertEqual(checked.returncode, 0, checked.stdout)
        made = self.cli("regenerate", *args(old, "predecessor-"), *args(new, "candidate-"),
                        "--output", self.root / "v2", "--reason", "acquisition-improvement", "--code-commit", "1" * 40)
        self.assertEqual(made.returncode, 0, made.stdout + made.stderr)
        data = json.loads(made.stdout)
        self.assertEqual(data["review"], "REVIEW_REQUIRED")
        self.assertEqual(data["counts"]["process_launches"], 0)
        final = self.cli("verify", *args(data))
        self.assertEqual(final.returncode, 0, final.stdout)
        missing = self.cli("verify", "--catalog", self.root / "missing", "--bundle-root", self.root,
                           "--source-root", self.root, "--runtime-root", self.root)
        self.assertEqual(missing.returncode, 1)
        self.assertEqual(json.loads(missing.stdout)["status"], "FAIL")


if __name__ == "__main__":
    unittest.main()

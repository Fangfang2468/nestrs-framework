"""Contract and failure checks for the maintenance scripts' doctor JSON client."""

from copy import deepcopy
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from toolchain_support import parse_doctor_output, query_doctor


def doctor_record():
    return {
        "version": 1,
        "rustc": {"release": "1.98.0", "commit_hash": "compiler-commit", "host": "test-host"},
        "compiler": "/selected compiler/rustc",
        "sysroot": "/selected sysroot",
        "driver": "/selected tools/driver",
        "macro_bridge": "/selected tools/bridge",
        "fingerprint": "opaque-tool-identity",
        "target_directory": None,
        "cache_directory": None,
        "compiler_output_directory": None,
    }


class DoctorContractTests(unittest.TestCase):
    def test_query_preserves_cli_selected_paths_without_reconstructing_them(self):
        record = doctor_record()
        record.update({
            "target_directory": "C:/项目 with spaces/target",
            "cache_directory": "C:/cli-selected-layout/opaque-cache",
            "compiler_output_directory": "C:/another-chosen-directory",
        })
        cli = Path("/tools/cargo-nestrs")
        command = [str(cli), "doctor", "--json", "--target-dir", "relative target"]
        completed = subprocess.CompletedProcess(command, 0, json.dumps(record), "doctor warning\n")
        environment = {"NESTRS_RUSTC": "chosen-rustc"}
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "doctor"
            with patch("toolchain_support.subprocess.run", return_value=completed) as run:
                actual = query_doctor(
                    cli, cwd=Path("/project"), environment=environment,
                    target_dir="relative target", log=log,
                )
            run.assert_called_once_with(
                command, cwd=Path("/project"), env=environment, text=True,
                capture_output=True, check=False, encoding="utf-8",
            )
            self.assertEqual(actual, record)
            self.assertEqual(log.with_suffix(".stdout.txt").read_text(), completed.stdout)
            self.assertEqual(log.with_suffix(".stderr.txt").read_text(), completed.stderr)
            self.assertEqual(json.loads(log.with_suffix(".status.json").read_text()),
                             {"command": command, "exit_code": 0})

    def test_query_without_target_accepts_null_directories(self):
        record = doctor_record()
        completed = subprocess.CompletedProcess([], 0, json.dumps(record), "")
        with patch("toolchain_support.subprocess.run", return_value=completed) as run:
            self.assertEqual(query_doctor("cargo-nestrs"), record)
        self.assertEqual(run.call_args.args[0], ["cargo-nestrs", "doctor", "--json"])

    def test_rejects_non_json_and_legacy_human_output(self):
        for value in ["compiler: /rustc\ndriver fingerprint: old\n", "{} trailing", "null", "[]"]:
            with self.subTest(output=value), self.assertRaises(ValueError):
                parse_doctor_output(value)

    def test_rejects_missing_unknown_and_boolean_versions(self):
        for version in [None, 0, 2, "1", True]:
            record = doctor_record()
            record["version"] = version
            with self.subTest(version=version), self.assertRaisesRegex(ValueError, "version"):
                parse_doctor_output(json.dumps(record))

    def test_requires_typed_nonempty_identity_and_tool_fields(self):
        fields = ["compiler", "sysroot", "driver", "macro_bridge", "fingerprint"]
        for nested, names in [(False, fields), (True, ["release", "commit_hash", "host"])]:
            for field in names:
                for invalid in [None, "", "  ", 23, ["value"]]:
                    record = doctor_record()
                    parent = record["rustc"] if nested else record
                    parent[field] = invalid
                    with self.subTest(field=field, value=invalid), self.assertRaisesRegex(ValueError, field):
                        parse_doctor_output(json.dumps(record))
                del parent[field]
                with self.subTest(missing=field), self.assertRaisesRegex(ValueError, field):
                    parse_doctor_output(json.dumps(record))
        for identity in [None, [], "rustc 1.98"]:
            record = doctor_record()
            record["rustc"] = identity
            with self.subTest(identity=identity), self.assertRaisesRegex(ValueError, "rustc"):
                parse_doctor_output(json.dumps(record))

    def test_requires_directory_contract_and_explicit_target_results(self):
        complete = doctor_record()
        fields = ["target_directory", "cache_directory", "compiler_output_directory"]
        for field in fields:
            complete[field] = f"/cli-chosen/{field}"
        self.assertEqual(parse_doctor_output(json.dumps(complete), require_directories=True), complete)
        for field in fields:
            for invalid in [None, "", False, 123]:
                record = deepcopy(complete)
                record[field] = invalid
                with self.subTest(field=field, value=invalid), self.assertRaisesRegex(ValueError, field):
                    parse_doctor_output(json.dumps(record), require_directories=True)
            del record[field]
            with self.subTest(missing=field), self.assertRaisesRegex(ValueError, field):
                parse_doctor_output(json.dumps(record))

    def test_failed_command_preserves_status_and_raw_logs_before_json_parsing(self):
        command = ["cargo-nestrs", "doctor", "--json"]
        completed = subprocess.CompletedProcess(command, 37, "partial non-JSON output", "missing driver\n")
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "doctor"
            with patch("toolchain_support.subprocess.run", return_value=completed):
                with self.assertRaises(subprocess.CalledProcessError) as caught:
                    query_doctor("cargo-nestrs", log=log)
            self.assertEqual(caught.exception.returncode, 37)
            self.assertEqual(caught.exception.stdout, completed.stdout)
            self.assertEqual(caught.exception.stderr, completed.stderr)
            self.assertEqual(log.with_suffix(".stdout.txt").read_text(), completed.stdout)
            self.assertEqual(log.with_suffix(".stderr.txt").read_text(), completed.stderr)
            self.assertEqual(json.loads(log.with_suffix(".status.json").read_text())["exit_code"], 37)

    def test_invalid_success_output_is_retained_and_never_parsed_as_human_text(self):
        completed = subprocess.CompletedProcess([], 0, "compiler: /rustc\n", "")
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "doctor"
            with patch("toolchain_support.subprocess.run", return_value=completed):
                with self.assertRaisesRegex(ValueError, "Invalid doctor JSON"):
                    query_doctor("cargo-nestrs", log=log)
            self.assertEqual(log.with_suffix(".stdout.txt").read_text(), completed.stdout)

    def test_process_launch_errors_are_propagated(self):
        with patch("toolchain_support.subprocess.run", side_effect=FileNotFoundError("missing CLI")):
            with self.assertRaisesRegex(FileNotFoundError, "missing CLI"):
                query_doctor("missing-cargo-nestrs")


if __name__ == "__main__":
    unittest.main()

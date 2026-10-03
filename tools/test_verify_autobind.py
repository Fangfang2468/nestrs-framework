"""Check that the auto-binding wrapper delegates to the Rust harness unchanged."""

from contextlib import redirect_stderr, redirect_stdout
from copy import deepcopy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from test_toolchain_support import doctor_record


spec = importlib.util.spec_from_file_location(
    "verify_autobind", Path(__file__).with_name("compiler-probe") / "verify_autobind.py"
)
verify_autobind = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify_autobind)


class AutoBindingWrapperTests(unittest.TestCase):
    """Verify delegation and failure reporting; Rust owns the actual DI contract assertions."""
    def test_prepares_selected_tools_and_delegates_all_contracts_to_rust(self):
        for rustc, skip_build in [(None, False), (None, True), ("chosen-rustc", False), ("chosen-rustc", True)]:
            with self.subTest(rustc=rustc, skip_build=skip_build):
                calls = []
                doctor_calls = []
                stages = []
                selected = doctor_record()
                report = {}

                def invoke(command, environment, root, log):
                    stages.append(log.name)
                    calls.append(([str(item) for item in command], environment.copy(), root, log))
                    stdout = "/resolved sysroot\n" if command[1:] == ["--print", "sysroot"] else json.dumps({"target_directory": "/cargo target"})
                    return subprocess.CompletedProcess(command, 0, stdout, "")

                def query(cli, **kwargs):
                    stages.append(kwargs["log"].name)
                    doctor_calls.append((cli, kwargs["environment"].copy()))
                    return selected

                inherited = {"RUSTC_BOOTSTRAP": "1", "NESTRS_DRIVER": "stale", "NESTRS_MACRO_BRIDGE": "stale"}
                with patch.dict(verify_autobind.os.environ, inherited, clear=True), \
                     patch.object(verify_autobind, "invoke", side_effect=invoke), \
                     patch.object(verify_autobind, "query_doctor", side_effect=query), \
                     patch.object(verify_autobind, "compiler_command_environment"), \
                     patch.object(verify_autobind, "compiler_library_environment") as libraries:
                    verify_autobind.run_contracts(Path("/workspace"), Path("/logs"), rustc, skip_build, report)
                if not skip_build:
                    expected = [str(Path("/workspace/tools/build-toolchain.py"))]
                    if rustc is not None:
                        expected.extend(["--rustc", rustc])
                    self.assertEqual(calls[0][0][1:], expected)
                    self.assertNotIn("RUSTC_BOOTSTRAP", calls[0][1])
                self.assertEqual(len(calls), 3 + int(not skip_build) + int(rustc is not None))
                expected_stages = ([] if skip_build else ["toolchain-build"])
                if rustc is not None:
                    expected_stages.append("selected-sysroot")
                expected_stages.extend([
                    "cargo-metadata", "doctor-preparation", "autobind-contracts-build",
                    "doctor-execution", "autobind-contracts", "doctor-after",
                ])
                self.assertEqual(stages, expected_stages)
                self.assertEqual(calls[-1][0], [
                    "cargo", "test", "-p", "cargo-nestrs", "--features", "compiler-driver",
                    "--test", "autobind_contracts", "--locked", "--offline",
                ])
                self.assertEqual(calls[-2][0], [*calls[-1][0], "--no-run"])
                self.assertEqual(calls[-2][1], calls[-1][1])
                environment = calls[-1][1]
                self.assertEqual(environment["RUSTC_BOOTSTRAP"], "nestrs_driver")
                self.assertEqual(environment["RUSTC"], selected["compiler"])
                self.assertEqual(environment["NESTRS_RUSTC"], selected["compiler"])
                self.assertEqual(environment["NESTRS_DRIVER"], selected["driver"])
                self.assertEqual(environment["NESTRS_MACRO_BRIDGE"], selected["macro_bridge"])
                self.assertEqual(environment["CARGO_TARGET_DIR"], str(Path("/cargo target")))
                self.assertEqual(doctor_calls[0][0], Path("/cargo target/debug") / verify_autobind.executable_name("cargo-nestrs"))
                self.assertEqual(len(doctor_calls), 3)
                self.assertTrue(all(call[0] == doctor_calls[0][0] for call in doctor_calls))
                self.assertEqual(doctor_calls[1][1], environment)
                self.assertEqual(doctor_calls[2][1], environment)
                if rustc is None:
                    self.assertNotIn("NESTRS_RUSTC", doctor_calls[0][1])
                else:
                    self.assertEqual(doctor_calls[0][1]["NESTRS_RUSTC"],
                                     str(Path("/resolved sysroot/bin") / verify_autobind.executable_name("rustc")))
                libraries.assert_called_once()
                self.assertEqual(libraries.call_args.args[1:], (Path(selected["sysroot"]), selected["rustc"]["host"]))
                self.assertEqual(report["toolchain"], selected)
                self.assertEqual(report["preparation_toolchain"], selected)
                self.assertEqual(report["toolchain_after"], selected)
                self.assertTrue(report["toolchain_stable"])

    def run_with_toolchain_responses(self, identities, *, build_failure=None, test_failure=None):
        """Execute the real wrapper with scripted tool changes and command failures."""
        report = {}
        stages = []
        remaining = iter(identities)

        def invoke(command, environment, root, log):
            stages.append(log.name)
            if log.name == "autobind-contracts-build" and build_failure is not None:
                raise build_failure
            if log.name == "autobind-contracts" and test_failure is not None:
                raise test_failure
            return subprocess.CompletedProcess(command, 0, json.dumps({"target_directory": "/cargo target"}), "")

        def query(cli, **kwargs):
            stages.append(kwargs["log"].name)
            response = next(remaining)
            if isinstance(response, Exception):
                raise response
            return deepcopy(response)

        failure = None
        with patch.dict(verify_autobind.os.environ, {}, clear=True), \
             patch.object(verify_autobind, "invoke", side_effect=invoke), \
             patch.object(verify_autobind, "query_doctor", side_effect=query), \
             patch.object(verify_autobind, "compiler_library_environment"):
            try:
                verify_autobind.run_contracts(Path("/workspace"), Path("/logs"), None, True, report)
            except Exception as error:
                failure = error
        return report, stages, failure

    def test_harness_build_changes_are_recorded_as_the_execution_identity(self):
        prepared = doctor_record()
        executed = deepcopy(prepared)
        executed["fingerprint"] = "newly-built-driver-and-bridge"
        report, stages, failure = self.run_with_toolchain_responses([prepared, executed, executed])
        self.assertIsNone(failure)
        self.assertEqual(report["preparation_toolchain"], prepared)
        self.assertEqual(report["toolchain"], executed)
        self.assertEqual(report["toolchain_after"], executed)
        self.assertTrue(report["toolchain_stable"])
        self.assertEqual(stages, [
            "cargo-metadata", "doctor-preparation", "autobind-contracts-build",
            "doctor-execution", "autobind-contracts", "doctor-after",
        ])

    def test_successful_tests_reject_any_execution_toolchain_change(self):
        executed = doctor_record()
        replacements = [
            ("fingerprint", "changed-content"),
            ("driver", "/changed/driver"),
            ("macro_bridge", "/changed/bridge"),
            ("compiler", "/changed/compiler"),
            ("sysroot", "/changed/sysroot"),
            ("rustc", {**executed["rustc"], "commit_hash": "changed-commit"}),
        ]
        for field, changed in replacements:
            with self.subTest(field=field):
                after = deepcopy(executed)
                after[field] = changed
                report, stages, failure = self.run_with_toolchain_responses([executed, executed, after])
                self.assertIsInstance(failure, RuntimeError)
                self.assertEqual(report["toolchain"], executed)
                self.assertEqual(report["toolchain_after"], after)
                self.assertFalse(report["toolchain_stable"])
                self.assertEqual(stages[-2:], ["autobind-contracts", "doctor-after"])

    def test_successful_tests_do_not_hide_a_failed_post_test_doctor(self):
        selected = doctor_record()
        for failure in [subprocess.CalledProcessError(37, ["doctor"]), ValueError("invalid doctor JSON")]:
            with self.subTest(error=type(failure).__name__):
                report, stages, raised = self.run_with_toolchain_responses([selected, selected, failure])
                self.assertIs(raised, failure)
                self.assertEqual(report["toolchain"], selected)
                self.assertFalse(report["toolchain_stable"])
                self.assertNotIn("toolchain_after", report)
                self.assertEqual(stages[-1], "doctor-after")

    def test_harness_compile_failure_stops_before_claiming_an_execution_identity(self):
        selected = doctor_record()
        failure = subprocess.CalledProcessError(101, ["cargo", "test", "--no-run"])
        report, stages, raised = self.run_with_toolchain_responses([selected], build_failure=failure)
        self.assertIs(raised, failure)
        self.assertEqual(report["preparation_toolchain"], selected)
        self.assertNotIn("toolchain", report)
        self.assertNotIn("toolchain_after", report)
        self.assertEqual(stages, ["cargo-metadata", "doctor-preparation", "autobind-contracts-build"])

    def test_execution_doctor_failure_prevents_the_contract_command(self):
        selected = doctor_record()
        failure = subprocess.CalledProcessError(37, ["doctor"])
        report, stages, raised = self.run_with_toolchain_responses([selected, failure])
        self.assertIs(raised, failure)
        self.assertNotIn("toolchain", report)
        self.assertEqual(stages, [
            "cargo-metadata", "doctor-preparation", "autobind-contracts-build", "doctor-execution",
        ])

    def test_failed_tests_keep_the_original_exit_code_when_post_test_audit_fails(self):
        selected = doctor_record()
        changed = {**selected, "fingerprint": "changed-during-test"}
        endings = [changed, subprocess.CalledProcessError(37, ["doctor"]), ValueError("invalid JSON")]
        for ending in endings:
            with self.subTest(ending=type(ending).__name__):
                failure = subprocess.CalledProcessError(101, ["cargo", "test"])
                report, stages, raised = self.run_with_toolchain_responses(
                    [selected, selected, ending], test_failure=failure,
                )
                self.assertIs(raised, failure)
                self.assertEqual(raised.returncode, 101)
                self.assertEqual(report["toolchain"], selected)
                self.assertFalse(report["toolchain_stable"])
                self.assertEqual(stages[-2:], ["autobind-contracts", "doctor-after"])
                if isinstance(ending, dict):
                    self.assertEqual(report["toolchain_after"], ending)

    def test_failed_tests_are_not_overridden_by_a_successful_identity_audit(self):
        selected = doctor_record()
        failure = subprocess.CalledProcessError(101, ["cargo", "test"])
        report, stages, raised = self.run_with_toolchain_responses(
            [selected, selected, selected], test_failure=failure,
        )
        self.assertIs(raised, failure)
        self.assertTrue(report["toolchain_stable"])
        self.assertEqual(report["toolchain_after"], selected)
        self.assertEqual(stages[-1], "doctor-after")

    def test_command_failure_retains_unmodified_output_and_status(self):
        result = subprocess.CompletedProcess(["cargo", "test"], 101, "failed test stdout", "diagnostic stderr")
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "contracts"
            with patch.object(verify_autobind.subprocess, "run", return_value=result):
                with self.assertRaises(subprocess.CalledProcessError) as caught:
                    verify_autobind.invoke(result.args, {}, Path(directory), log)
            self.assertEqual(caught.exception.returncode, 101)
            self.assertEqual(log.with_suffix(".stdout.txt").read_text(), result.stdout)
            self.assertEqual(log.with_suffix(".stderr.txt").read_text(), result.stderr)
            self.assertEqual(json.loads(log.with_suffix(".status.json").read_text())["exit_code"], 101)

    def test_main_preserves_failed_harness_exit_code_in_process_and_report(self):
        for raw_code, shell_code in [(101, 101), (-9, 137)]:
            with self.subTest(raw_code=raw_code), tempfile.TemporaryDirectory() as directory:
                error = subprocess.CalledProcessError(raw_code, ["cargo", "test"])
                with patch.object(verify_autobind.sys, "argv", ["verify_autobind.py", "--skip-build"]), \
                     patch.object(verify_autobind.tempfile, "mkdtemp", return_value=directory), \
                     patch.object(verify_autobind, "run_contracts", side_effect=error), \
                     redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                    self.assertEqual(verify_autobind.main(), shell_code)
                report = json.loads((Path(directory) / "report.json").read_text())
                self.assertFalse(report["passed"])
                self.assertEqual(report["failed_command_exit_code"], raw_code)
                self.assertEqual(report["exit_code"], shell_code)

    def test_main_only_passes_when_the_rust_contract_command_succeeds(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(verify_autobind.sys, "argv", ["verify_autobind.py", "--rustc", "selected-rustc"]), \
                 patch.object(verify_autobind.tempfile, "mkdtemp", return_value=directory), \
                 patch.object(verify_autobind, "run_contracts") as run, redirect_stdout(io.StringIO()):
                self.assertEqual(verify_autobind.main(), 0)
            self.assertEqual(run.call_args.args[2:4], ("selected-rustc", False))
            report = json.loads((Path(directory) / "report.json").read_text())
            self.assertTrue(report["passed"])
            self.assertEqual(report["exit_code"], 0)

    def test_main_never_marks_changed_execution_tools_as_passed(self):
        def reject_change(root, output, rustc, skip_build, report):
            report["toolchain_stable"] = False
            raise RuntimeError("toolchain changed during contract execution")

        with tempfile.TemporaryDirectory() as directory:
            with patch.object(verify_autobind.sys, "argv", ["verify_autobind.py", "--skip-build"]), \
                 patch.object(verify_autobind.tempfile, "mkdtemp", return_value=directory), \
                 patch.object(verify_autobind, "run_contracts", side_effect=reject_change), \
                 redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                self.assertEqual(verify_autobind.main(), 1)
            report = json.loads((Path(directory) / "report.json").read_text())
            self.assertFalse(report["passed"])
            self.assertFalse(report["toolchain_stable"])
            self.assertEqual(report["exit_code"], 1)


if __name__ == "__main__":
    unittest.main()

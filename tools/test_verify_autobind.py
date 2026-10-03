"""Check that the auto-binding wrapper delegates to the Rust harness unchanged."""

from contextlib import redirect_stderr, redirect_stdout
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
    def test_prepares_selected_tools_and_delegates_all_contracts_to_rust(self):
        for rustc, skip_build in [(None, False), (None, True), ("chosen-rustc", False), ("chosen-rustc", True)]:
            with self.subTest(rustc=rustc, skip_build=skip_build):
                calls = []
                doctor_calls = []
                selected = doctor_record()
                report = {}

                def invoke(command, environment, root, log):
                    calls.append(([str(item) for item in command], environment.copy(), root, log))
                    stdout = "/resolved sysroot\n" if command[1:] == ["--print", "sysroot"] else json.dumps({"target_directory": "/cargo target"})
                    return subprocess.CompletedProcess(command, 0, stdout, "")

                def query(cli, **kwargs):
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
                self.assertEqual(len(calls), 2 + int(not skip_build) + int(rustc is not None))
                self.assertEqual(calls[-1][0], [
                    "cargo", "test", "-p", "cargo-nestrs", "--features", "compiler-driver",
                    "--test", "autobind_contracts", "--locked", "--offline",
                ])
                environment = calls[-1][1]
                self.assertEqual(environment["RUSTC_BOOTSTRAP"], "nestrs_driver")
                self.assertEqual(environment["RUSTC"], selected["compiler"])
                self.assertEqual(environment["NESTRS_RUSTC"], selected["compiler"])
                self.assertEqual(environment["NESTRS_DRIVER"], selected["driver"])
                self.assertEqual(environment["NESTRS_MACRO_BRIDGE"], selected["macro_bridge"])
                self.assertEqual(environment["CARGO_TARGET_DIR"], str(Path("/cargo target")))
                self.assertEqual(doctor_calls[0][0], Path("/cargo target/debug") / verify_autobind.executable_name("cargo-nestrs"))
                if rustc is None:
                    self.assertNotIn("NESTRS_RUSTC", doctor_calls[0][1])
                else:
                    self.assertEqual(doctor_calls[0][1]["NESTRS_RUSTC"],
                                     str(Path("/resolved sysroot/bin") / verify_autobind.executable_name("rustc")))
                libraries.assert_called_once()
                self.assertEqual(libraries.call_args.args[1:], (Path(selected["sysroot"]), selected["rustc"]["host"]))
                self.assertEqual(report["toolchain"], selected)

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


if __name__ == "__main__":
    unittest.main()

"""Check crate-scoped bootstrap authorization in the real macro-probe orchestration."""

from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from toolchain_support import bridge_name, executable_name


spec = importlib.util.spec_from_file_location(
    "verify_macro_editor", Path(__file__).with_name("verify-macro-editor.py")
)
verify_macro_editor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify_macro_editor)


# These fixed expansion stand-ins only let main finish its protocol exchanges.
# Actual macro syntax and generated behavior remain the real probe's assertions.
EXPANSIONS = {
    "injectable": (
        "structConsumer{port:::nestrs_core::Injection<dynPort>,"
        "optional:::core::option::Option<::nestrs_core::Injection<dynPort>>}"
        "compiler_dependency::<dynPort,0usize>();compiler_dependency::<dynPort,1usize>();"
    ),
    "factory": (
        "asyncfncreate<'__nestrs_factory_frame>(port:&'__nestrs_factory_framedynPort)->Product{}"
        "FactoryInputs<'frame>FactoryFuture<'frame>"
        "FactoryInvoker::Async(__nestrs_factory_construct)"
    ),
}


class MacroEditorBootstrapTests(unittest.TestCase):
    """Keep private bridge authorization out of compiler queries and macro servers."""

    def exercise(self, root, inherited, *, build_exit=0):
        server = root / "sysroot/libexec" / executable_name("rust-analyzer-proc-macro-srv")
        server.parent.mkdir(parents=True)
        server.touch()
        library = root / "artifacts" / bridge_name()
        queries = []
        builds = []
        servers = []
        result = {"queries": queries, "builds": builds, "servers": servers}

        def query(command, **kwargs):
            queries.append((command, kwargs["env"].copy()))
            if command[1:] == ["--print", "sysroot"]:
                return str(root / "sysroot") + "\n"
            self.assertEqual(command[1:], ["-vV"])
            return "release: 1.98.0\nhost: x86_64-unknown-linux-gnu\n"

        def run(command, **kwargs):
            if command[0] == "cargo":
                builds.append((command, kwargs["env"].copy()))
                self.assertEqual(command, ["cargo", "build", "-p", "nestrs-tool-bridge", "--message-format=json"])
                artifact = {"reason": "compiler-artifact", "target": {"name": "nestrs_tool_bridge"},
                            "filenames": [str(library)]}
                return subprocess.CompletedProcess(command, build_exit, json.dumps(artifact) + "\n", "build diagnostic")
            self.assertEqual(command, [str(server)])
            requests = [json.loads(line) for line in kwargs["input"].splitlines()]
            servers.append((command, kwargs["env"].copy(), requests))
            responses = []
            for request in requests:
                if "ApiVersionCheck" in request:
                    responses.append({"ApiVersionCheck": 6})
                elif "ListMacros" in request:
                    responses.append({"ListMacros": {"Ok": [[name, "Attr"] for name in ["injectable", "factory", "primary"]]}})
                else:
                    responses.append({"ExpandMacro": {"Ok": {"fixture_macro": request["ExpandMacro"]["macro_name"]}}})
            return subprocess.CompletedProcess(command, 0, "".join(json.dumps(item) + "\n" for item in responses), "")

        with patch.dict(verify_macro_editor.os.environ, inherited, clear=True), \
             patch.object(verify_macro_editor, "__file__", str(root / "tools/verify-macro-editor.py")), \
             patch.object(verify_macro_editor.subprocess, "check_output", side_effect=query), \
             patch.object(verify_macro_editor.subprocess, "run", side_effect=run), \
             patch.object(verify_macro_editor, "macro_server", return_value=server), \
             patch.object(verify_macro_editor, "tokens", side_effect=lambda tree: [EXPANSIONS[tree["fixture_macro"]]]), \
             redirect_stdout(io.StringIO()):
            before = dict(verify_macro_editor.os.environ)
            try:
                result["exit_code"] = verify_macro_editor.main()
            except Exception as error:
                result["failure"] = error
            self.assertEqual(dict(verify_macro_editor.os.environ), before, "probe mutated its parent process environment")
        return result

    def test_inherited_authorization_is_replaced_only_for_the_private_bridge_build(self):
        for inherited_bootstrap in ["1", "nestrs_driver,another_crate"]:
            with self.subTest(bootstrap=inherited_bootstrap), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                inherited = {"RUSTC_BOOTSTRAP": inherited_bootstrap, "NESTRS_RUSTC": "chosen-rustc",
                             "RUSTC_WRAPPER": "inherited-wrapper", "RUSTC_WORKSPACE_WRAPPER": "inherited-workspace-wrapper",
                             "RUSTDOC": "inherited-rustdoc", "USER_VALUE": "preserved"}
                result = self.exercise(root, inherited)
                self.assertNotIn("failure", result, result.get("failure"))
                self.assertEqual(result["exit_code"], 0)
                self.assertEqual(len(result["queries"]), 2)
                self.assertEqual(len(result["builds"]), 1)
                self.assertEqual(len(result["servers"]), 3)
                build_environment = result["builds"][0][1]
                self.assertEqual(build_environment["RUSTC_BOOTSTRAP"], "nestrs_tool_bridge")
                self.assertEqual(build_environment["RUSTC"], "chosen-rustc")
                for command, environment in result["queries"]:
                    self.assertEqual(command[0], "chosen-rustc")
                    self.assertNotIn("RUSTC_BOOTSTRAP", environment)
                for _, environment, requests in result["servers"]:
                    self.assertNotIn("RUSTC_BOOTSTRAP", environment)
                    for request in requests:
                        if "ExpandMacro" in request:
                            self.assertNotIn("RUSTC_BOOTSTRAP", dict(request["ExpandMacro"]["env"]))
                for environment in [build_environment, *[call[1] for call in result["queries"]], *[call[1] for call in result["servers"]]]:
                    self.assertEqual(environment["USER_VALUE"], "preserved")
                    for key in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTDOC"]:
                        self.assertNotIn(key, environment)
                report = json.loads((root / "target/nestrs-tool-bridge-probe/report.json").read_text())
                self.assertTrue(report["passed"])
                self.assertEqual([case["macro"] for case in report["cases"]], ["injectable", "factory"])

    def test_clean_parent_environment_stays_unprivileged_outside_the_build(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.exercise(Path(directory), {"USER_VALUE": "untouched"})
        self.assertNotIn("failure", result, result.get("failure"))
        self.assertEqual(result["builds"][0][1]["RUSTC_BOOTSTRAP"], "nestrs_tool_bridge")
        self.assertTrue(all("RUSTC_BOOTSTRAP" not in call[1] for call in result["queries"]))
        self.assertTrue(all("RUSTC_BOOTSTRAP" not in call[1] for call in result["servers"]))

    def test_failed_bridge_build_does_not_start_a_macro_server(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.exercise(root, {"RUSTC_BOOTSTRAP": "1"}, build_exit=101)
            self.assertIsInstance(result.get("failure"), AssertionError)
            self.assertIn("macro build failed", str(result["failure"]))
            self.assertEqual(len(result["queries"]), 2)
            self.assertEqual(len(result["builds"]), 1)
            self.assertEqual(result["builds"][0][1]["RUSTC_BOOTSTRAP"], "nestrs_tool_bridge")
            self.assertEqual(result["servers"], [])
            self.assertEqual((root / "target/nestrs-tool-bridge-probe/build.stderr.txt").read_text(), "build diagnostic")
            self.assertFalse((root / "target/nestrs-tool-bridge-probe/report.json").exists())


if __name__ == "__main__":
    unittest.main()

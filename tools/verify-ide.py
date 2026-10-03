#!/usr/bin/env python3
"""Exercise the generated IDE project with an installed, unmodified rust-analyzer.

No tool is installed. All edited sources, settings and LSP records live below
target; the repository fixture is copied before any changes are made.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
import time
from urllib.parse import unquote, urlsplit
from urllib.request import url2pathname

from toolchain_support import bridge_name, executable_name


def source_hashes(directory):
    paths = list(directory.rglob("*.rs")) + [directory / "Cargo.toml", directory / "Cargo.lock"]
    return {str(p.relative_to(directory)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def locate_server(explicit):
    if explicit:
        path = Path(explicit).resolve()
        assert path.is_file(), f"rust-analyzer does not exist: {path}"
        return path
    candidates = sorted(
        path
        for directory in [".vscode", ".vscode-insiders", ".vscode-server"]
        for path in Path.home().glob(f"{directory}/extensions/rust-lang.rust-analyzer-*/server/{executable_name('rust-analyzer')}")
    )
    if candidates:
        return candidates[-1]
    candidate = shutil.which("rust-analyzer")
    assert candidate, "No rust-analyzer found; pass --rust-analyzer /path/to/an/installed/server"
    return Path(candidate).resolve()


def nested_settings(settings):
    result = {}
    for key, value in settings.items():
        if not key.startswith("rust-analyzer."):
            continue
        parts = key.removeprefix("rust-analyzer.").split(".")
        current = result
        for part in parts[:-1]:
            current = current.setdefault(part, {})
        current[parts[-1]] = value
    return result


class Lsp:
    def __init__(self, server, root, options, environment, output):
        self.output = output
        self.options = options
        self.root = root
        self.messages = []
        self.queue = queue.Queue()
        self.diagnostics = {}
        self.sequence = 0
        self.next_id = 0
        self.stderr = output.with_suffix(".stderr.txt").open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [str(server)], cwd=root, env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=self.stderr,
        )
        threading.Thread(target=self.read, daemon=True).start()
        initialized = self.request("initialize", {
            "processId": os.getpid(), "rootUri": root.as_uri(),
            "workspaceFolders": [{"uri": root.as_uri(), "name": "ide-fixture"}],
            "capabilities": {
                "workspace": {"configuration": True, "workspaceFolders": True},
                "textDocument": {"publishDiagnostics": {"versionSupport": True},
                                 "diagnostic": {"dynamicRegistration": False},
                                 "completion": {"completionItem": {"snippetSupport": True}}},
                "experimental": {"serverStatusNotification": True},
            },
            "initializationOptions": options,
        })
        assert "capabilities" in initialized, initialized
        self.notify("initialized", {})

    def read(self):
        try:
            while True:
                headers = {}
                while True:
                    line = self.process.stdout.readline()
                    if not line:
                        return
                    if line in (b"\r\n", b"\n"):
                        break
                    name, value = line.decode().split(":", 1)
                    headers[name.lower()] = value.strip()
                payload = self.process.stdout.read(int(headers["content-length"]))
                self.queue.put(json.loads(payload))
        except Exception as error:
            self.queue.put({"reader_error": str(error)})

    def send(self, message):
        data = json.dumps(message).encode()
        self.process.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
        self.process.stdin.flush()

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def consume(self, timeout):
        try:
            message = self.queue.get(timeout=max(0.01, timeout))
        except queue.Empty:
            assert self.process.poll() is None, f"rust-analyzer exited: {self.process.returncode}"
            return None
        self.messages.append(message)
        assert "reader_error" not in message, message
        if message.get("method") == "textDocument/publishDiagnostics":
            self.sequence += 1
            self.diagnostics[message["params"]["uri"]] = (self.sequence, message["params"])
        if "id" in message and "method" in message:
            response = None
            if message["method"] == "workspace/configuration":
                response = []
                for item in message.get("params", {}).get("items", []):
                    section = item.get("section", "rust-analyzer")
                    value = self.options
                    for part in section.removeprefix("rust-analyzer").strip(".").split("."):
                        if part:
                            value = value.get(part) if isinstance(value, dict) else None
                    response.append(value)
            elif message["method"] == "workspace/workspaceFolders":
                response = [{"uri": self.root.as_uri(), "name": "ide-fixture"}]
            self.send({"jsonrpc": "2.0", "id": message["id"], "result": response})
        return message

    def request(self, method, params=None, timeout=30):
        self.next_id += 1
        identifier = self.next_id
        self.send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            message = self.consume(deadline - time.monotonic())
            if message and message.get("id") == identifier and "method" not in message:
                error = message.get("error", {})
                data = error.get("data")
                retry_diagnostics = (
                    method == "textDocument/diagnostic"
                    and error.get("code") == -32802
                    and isinstance(data, dict)
                    and data.get("retriggerRequest") is True
                )
                if error.get("code") in [-32800, -32801] or retry_diagnostics:
                    # A cold workspace or an edit can invalidate an in-flight
                    # analysis snapshot. Diagnostic ServerCancelled responses
                    # permit another request only when retriggerRequest is true.
                    # Keep the original deadline, including the retry delay;
                    # do not consume and discard a queued message during it.
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    time.sleep(min(0.05, remaining))
                    if time.monotonic() >= deadline:
                        break
                    self.next_id += 1
                    identifier = self.next_id
                    self.send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
                    continue
                assert "error" not in message, f"{method}: {message}"
                return message.get("result")
        raise TimeoutError(f"LSP request timed out: {method}")

    def until(self, action, predicate, label, timeout=60):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            last = action()
            if predicate(last):
                return last
            self.consume(min(0.25, deadline - time.monotonic()))
        raise AssertionError(f"{label} never became available: {last}")

    def pull_diagnostics(self, uri):
        return self.request("textDocument/diagnostic", {"textDocument": {"uri": uri}, "identifier": "rust-analyzer"})

    def clean_diagnostics(self, uri):
        found = self.pull_diagnostics(uri)
        assert found and found["kind"] == "full", found
        errors = [entry for entry in found["items"] if entry.get("severity") == 1]
        assert not errors, f"unexpected IDE errors: {errors}"
        return found

    def close(self):
        try:
            self.request("shutdown", timeout=5)
            self.notify("exit", None)
            self.process.wait(timeout=5)
        except (AssertionError, OSError, TimeoutError, subprocess.TimeoutExpired):
            self.process.kill()
            self.process.wait()
        finally:
            self.stderr.close()
            self.output.with_suffix(".messages.json").write_text(json.dumps(self.messages, indent=2) + "\n", encoding="utf-8")


def position(source, fragment, offset=0):
    index = source.index(fragment) + offset
    prefix = source[:index]
    return {"line": prefix.count("\n"), "character": len(prefix.rsplit("\n", 1)[-1])}


def params(uri, source, fragment, offset=0):
    return {"textDocument": {"uri": uri}, "position": position(source, fragment, offset)}


def completion_labels(answer):
    items = answer.get("items", []) if isinstance(answer, dict) else answer or []
    return [item.get("label", "") for item in items]


def paths_match(left, right):
    def normalized(path):
        path = os.fspath(path)
        if os.name == "nt":
            # Rust canonicalize returns extended-length paths on Windows.
            if path.startswith("\\\\?\\UNC\\"):
                path = "\\\\" + path[8:]
            elif path.startswith("\\\\?\\"):
                path = path[4:]
        return os.path.normcase(os.path.abspath(path))

    return normalized(left) == normalized(right)


def file_uri_matches(uri, path):
    """LSP servers may encode drive colons and lowercase Windows drive names."""
    if not uri:
        return False
    parsed = urlsplit(uri)
    if parsed.scheme != "file":
        return False
    uri_path = ("//" + parsed.netloc if parsed.netloc else "") + parsed.path
    # url2pathname decodes URI escapes itself on Windows, but not on Unix.
    local = url2pathname(uri_path) if os.name == "nt" else unquote(uri_path)
    return paths_match(local, path)


def validate_model(path, project, alternate, release):
    model = json.loads(path.read_text(encoding="utf-8"))
    crates = model["crates"]
    candidates = [item for item in crates if paths_match(item["root_module"], project / "src/main.rs")
                  and "test" not in item.get("cfg", [])]
    assert len(candidates) == 1, f"expected one actual application crate: {candidates}"
    app = candidates[0]
    cfg = app.get("cfg", [])
    assert "nestrs_fixture_generated" in cfg, cfg
    assert ('feature="alternate"' in cfg) == alternate, cfg
    assert ("debug_assertions" in cfg) != release, cfg
    assert 'panic="unwind"' in cfg and 'target_arch="x86_64"' in cfg, cfg
    assert app["env"]["NESTRS_FIXTURE_LABEL"] == "generated-by-build-script", app["env"]
    constructor_model = json.loads(app["env"]["NESTRS_IDE_CONSTRUCTORS"])
    assert constructor_model["version"] == 1, constructor_model
    selections = [entry["selection"] for entry in constructor_model["declarations"] if "struct ConstructorService" in entry["input"]]
    assert len(selections) == 1 and selections[0]["constructor"], selections
    fields = {field["name"]: field for field in selections[0]["fields"]}
    assert fields["constructor_port"]["slot"] == 0 and not fields["constructor_port"]["lazy"], fields
    assert fields["later"]["slot"] == 1 and fields["later"]["lazy"], fields
    assert (Path(app["env"]["OUT_DIR"]) / "generated.rs").is_file(), app["env"]
    dependencies = {entry["name"]: crates[entry["crate"]] for entry in app["deps"]}
    assert {"nestrs", "nestrs_core", "tokio"} <= dependencies.keys(), dependencies.keys()
    bridge = dependencies["nestrs"]
    assert bridge["is_proc_macro"] and Path(bridge["proc_macro_dylib_path"]).is_file(), bridge
    assert any(item.get("display_name") == "tokio_macros" and item.get("is_proc_macro")
               and Path(item["proc_macro_dylib_path"]).is_file() for item in crates), "missing Tokio macro artifact"
    assert Path(model["sysroot"]).is_dir(), model
    return {"crate_count": len(crates), "cfg": cfg, "env": app["env"], "bridge": bridge["proc_macro_dylib_path"]}


def lsp_cases(server, project, settings, environment, output, full, release):
    main = project / "src/main.rs"
    source = main.read_text(encoding="utf-8")
    uri = main.as_uri()
    options = nested_settings(settings)
    session = Lsp(server, project, options, environment, output)
    evidence = {}
    try:
        session.notify("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "rust", "version": 1, "text": source,
        }})
        expansion_params = params(uri, source, "#[injectable]\nstruct Service", len("#["))
        expanded = session.until(lambda: session.request("rust-analyzer/expandMacro", expansion_params),
                                 lambda item: item and "Injection" in item.get("expansion", ""), "service expansion")
        evidence["expansion"] = expanded
        constructor_params = params(uri, source, "#[injectable]\nstruct ConstructorService", len("#["))
        constructor_expanded = session.until(
            lambda: session.request("rust-analyzer/expandMacro", constructor_params),
            lambda item: item and "LazyInjection" in item.get("expansion", "") and "__nestrs_constructor_activate" in item.get("expansion", ""),
            "constructor service expansion",
        )
        assert "Default::default" not in constructor_expanded["expansion"], constructor_expanded
        evidence["constructor_expansion"] = constructor_expanded
        evidence["cold_diagnostics"] = session.clean_diagnostics(uri)
        profile_hover = session.request("textDocument/hover", params(uri, source, "let _ = editor_profile_value();", len("let _ = ")))
        assert profile_hover and ("usize" if release else "str") in json.dumps(profile_hover), profile_hover
        evidence["profile_cfg_hover"] = profile_hover
        print(f"PASS IDE cold initialization ({output.name})", flush=True)
        if not full:
            return evidence

        field = session.request("textDocument/hover", params(uri, source, "self.port.label()", len("self.")))
        assert field and "Injection" in json.dumps(field), field
        evidence["field_hover"] = field
        constructor_field = session.request("textDocument/hover", params(uri, source, "self.constructor_port.label()", len("self.")))
        assert constructor_field and "Injection" in json.dumps(constructor_field), constructor_field
        lazy_field = session.request("textDocument/hover", params(uri, source, "self.later.get()", len("self.")))
        assert lazy_field and "LazyInjection" in json.dumps(lazy_field), lazy_field
        evidence["constructor_field_hover"] = constructor_field
        evidence["constructor_lazy_hover"] = lazy_field
        factory = session.request("textDocument/hover", params(uri, source, "Client(port.label())", len("Client(")))
        assert factory and "Port" in json.dumps(factory) and "&" in json.dumps(factory), factory
        evidence["factory_hover"] = factory
        labels = completion_labels(session.request("textDocument/completion", params(uri, source, "self.port.label()", len("self.port."))))
        assert any(label.startswith("label") for label in labels), labels
        evidence["completion"] = labels
        definitions = session.request("textDocument/definition", params(uri, source, "use external::External", len("use external::")))
        definitions = [definitions] if isinstance(definitions, dict) else definitions or []
        assert any(file_uri_matches(item.get("targetUri", item.get("uri")), project / "src/external.rs") for item in definitions), definitions
        evidence["definition"] = definitions
        print("PASS IDE field/factory hover, member completion and definition navigation", flush=True)

        updated = source.replace("fn label(&self) -> &'static str;", "fn label(&self) -> &'static str;\n    fn edited_label(&self) -> &'static str { self.label() }")
        session.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": updated}]})
        edited = session.until(
            lambda: completion_labels(session.request("textDocument/completion", params(uri, updated, "self.port.label()", len("self.port.")))),
            lambda labels: any(label.startswith("edited_label") for label in labels), "unsaved method completion",
        )
        evidence["unsaved_completion"] = edited
        evidence["unsaved_diagnostics"] = session.clean_diagnostics(uri)
        assert main.read_text(encoding="utf-8") == source, "LSP edit unexpectedly changed disk content"

        # Type mismatches are diagnosed by stock RA's default native checks.
        # The separate configured check below verifies a rustc E0425 value error.
        broken = updated + '\nfn deliberately_invalid() { let _: usize = "wrong_editor_type"; }\n'
        session.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": broken}]})

        def errors():
            return [entry for entry in session.pull_diagnostics(uri).get("items", []) if entry.get("severity") == 1]

        error_position = position(broken, '"wrong_editor_type"')
        evidence["real_error"] = session.until(
            errors,
            lambda entries: any(entry.get("code") == "E0308" and entry["range"]["start"] == error_position for entry in entries),
            "actual unsaved error diagnostic",
        )
        session.notify("textDocument/didChange", {"textDocument": {"uri": uri, "version": 4}, "contentChanges": [{"text": source}]})
        evidence["restored_diagnostics"] = session.clean_diagnostics(uri)
        print("PASS IDE unsaved edits, real error reporting and diagnostic recovery", flush=True)
        return evidence
    finally:
        session.close()


def configured_check_cases(project, settings, environment, output, model_path):
    command = settings["rust-analyzer.check.overrideCommand"]
    assert command[1:3] == ["init", "check"], command
    settings_path = project / ".vscode/settings.json"
    main = project / "src/main.rs"
    original = main.read_text(encoding="utf-8")
    model = model_path.read_bytes()
    settings_bytes = settings_path.read_bytes()
    before_mtime = model_path.stat().st_mtime_ns
    check_environment = environment.copy()
    for name in ["NESTRS_DRIVER", "NESTRS_MACRO_BRIDGE", "NESTRS_RUSTC"]:
        check_environment.pop(name, None)
    check_environment.update(settings["rust-analyzer.check.extraEnv"])

    def execute(name):
        result = subprocess.run(command, cwd=project, env=check_environment, text=True, capture_output=True, encoding="utf-8")
        (output / f"configured-check-{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
        (output / f"configured-check-{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
        return result

    cached = execute("cached")
    assert cached.returncode == 0, cached.stderr
    assert model_path.read_bytes() == model and model_path.stat().st_mtime_ns == before_mtime, "unchanged configured check rewrote the model"
    assert settings_path.read_bytes() == settings_bytes, "configured check changed user settings"
    try:
        main.write_text(original + "\nfn compiler_error_control() { unknown_saved_editor_value; }\n", encoding="utf-8")
        invalid = execute("invalid")
        assert invalid.returncode != 0, "a real saved compiler error was hidden"
        messages = [json.loads(line) for line in invalid.stdout.splitlines() if line.strip()]
        assert any(item.get("reason") == "compiler-message" and item.get("message", {}).get("level") == "error"
                   and "unknown_saved_editor_value" in json.dumps(item) for item in messages), messages
        assert model_path.read_bytes() == model, "failed editor check replaced a valid model"
        assert settings_path.read_bytes() == settings_bytes, "failed editor check replaced user settings"
    finally:
        main.write_text(original, encoding="utf-8")
    restored = execute("restored")
    assert restored.returncode == 0, restored.stderr
    assert model_path.read_bytes() == model, "restoring the original source changed its project model"
    print("PASS IDE configured Cargo check, cache stability, real saved errors and recovery", flush=True)
    return {"cached_passed": True, "failed_check_preserved_model_and_settings": True, "recovered": True}


def rejected_cfg_case(cli, project, environment, output, cargo_target):
    model_path = cargo_target / "nestrs/ide/rust-project.json"
    settings_path = project / ".vscode/settings.json"
    model = model_path.read_bytes()
    settings = settings_path.read_bytes()
    result = subprocess.run([
        str(cli), "init", "--manifest-path", str(project / "Cargo.toml"),
        "--target-dir", str(cargo_target), "--locked", "--offline", "--vscode",
        "--config", 'profile.dev.panic="abort"',
    ], cwd=project, env=environment, text=True, capture_output=True, encoding="utf-8")
    (output / "rejected-cfg.stdout.txt").write_text(result.stdout, encoding="utf-8")
    (output / "rejected-cfg.stderr.txt").write_text(result.stderr, encoding="utf-8")
    assert result.returncode != 0, "panic=abort produced an inaccurate IDE model"
    assert "cannot represent compiler cfg" in result.stderr and 'panic="unwind"' in result.stderr, result.stderr
    assert model_path.read_bytes() == model, "unsupported cfg replaced a valid project model"
    assert settings_path.read_bytes() == settings, "unsupported cfg changed user settings"
    print("PASS IDE rejects unsupported compiler cfg while preserving valid model/settings", flush=True)
    return {"panic_abort_rejected": True, "model_and_settings_preserved": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-analyzer", help="path to an already installed standard LSP server")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    fixture = root / "cargo-nestrs/tests/fixtures/ide"
    output = root / "target/nestrs-ide-verification"
    output.mkdir(parents=True, exist_ok=True)
    server = locate_server(args.rust_analyzer)
    environment = os.environ.copy()
    for name in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTDOC", "NESTRS_REAL_RUSTDOC", "NESTRS_MACRO_BRIDGE", "NESTRS_IDE_CAPTURE"]:
        environment.pop(name, None)
    if not args.skip_build:
        subprocess.run([sys.executable, str(root / "tools/build-toolchain.py")], cwd=root, env=environment, check=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=environment, text=True, encoding="utf-8"))
    binary_directory = Path(metadata["target_directory"]) / "debug"
    toolchain = output / "toolchain"
    toolchain.mkdir(exist_ok=True)
    for name in [executable_name("cargo-nestrs"), executable_name("nestrs-driver"), bridge_name()]:
        shutil.copy2(binary_directory / name, toolchain / name)
    cli = toolchain / executable_name("cargo-nestrs")
    environment["NESTRS_DRIVER"] = str(toolchain / executable_name("nestrs-driver"))
    environment["NESTRS_MACRO_BRIDGE"] = str(toolchain / bridge_name())
    doctor = subprocess.run([str(cli), "doctor"], cwd=root, env=environment, text=True, capture_output=True, encoding="utf-8")
    assert doctor.returncode == 0, doctor.stderr
    (output / "doctor.stdout.txt").write_text(doctor.stdout, encoding="utf-8")
    selected_tools = dict(line.split(": ", 1) for line in doctor.stdout.splitlines() if ": " in line)
    expected_check_env = {
        "NESTRS_DRIVER": selected_tools["driver"],
        "NESTRS_MACRO_BRIDGE": selected_tools["macro bridge"],
        "NESTRS_RUSTC": selected_tools["compiler"],
    }
    before = source_hashes(fixture)
    project = output / "project"
    project.mkdir(exist_ok=True)
    shutil.copytree(fixture, project, dirs_exist_ok=True, ignore=shutil.ignore_patterns("target", ".vscode"))
    main_path = project / "src/main.rs"
    main_path.write_text(main_path.read_text(encoding="utf-8") + '''
#[cfg(debug_assertions)]
fn editor_profile_value() -> &'static str { "debug" }
#[cfg(not(debug_assertions))]
fn editor_profile_value() -> usize { 1 }
fn editor_profile_check() { let _ = editor_profile_value(); }
''', encoding="utf-8")
    manifest = project / "Cargo.toml"
    manifest.write_text(manifest.read_text(encoding="utf-8").replace("../../../../nestrs-core", (root / "nestrs-core").as_posix()), encoding="utf-8")
    settings_path = project / ".vscode/settings.json"
    settings_path.parent.mkdir(exist_ok=True)
    original_settings = {
        "editor.fontSize": 17,
        "files.exclude": {"keep_me": True},
        "rust-analyzer.diagnostics.disabled": ["inactive_code"],
        "rust-analyzer.check.extraEnv": {"NESTRS_IDE_TEST": "preserved", "NESTRS_DRIVER": "old-tool-choice"},
    }
    settings_path.write_text(json.dumps(original_settings, indent=2) + "\n", encoding="utf-8")
    application = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", str(manifest), "--offline"], cwd=root, env=environment, text=True, encoding="utf-8"))
    assert {item["name"] for item in application["packages"][0]["dependencies"]} == {"nestrs-core", "tokio"}
    report = {"passed": False, "server": str(server), "application_dependencies": ["nestrs-core", "tokio"], "cases": []}
    try:
        for name, alternate, release in [("default", False, False), ("alternate", True, False), ("release", False, True)]:
            cargo_target = output / "cargo"
            command = [str(cli), "init", "--manifest-path", str(manifest), "--target-dir", str(cargo_target), "--locked", "--offline", "--vscode"]
            if alternate:
                command.extend(["--features", "alternate"])
            if release:
                command.append("--release")
            result = subprocess.run(command, cwd=project, env=environment, text=True, capture_output=True, encoding="utf-8")
            (output / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
            (output / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
            assert result.returncode == 0, f"IDE generation failed; inspect {output / (name + '.stderr.txt')}"
            model_path = cargo_target / "nestrs/ide/rust-project.json"
            evidence = validate_model(model_path, project, alternate, release)
            shutil.copy2(model_path, output / f"{name}.rust-project.json")
            settings = json.loads(settings_path.read_text(encoding="utf-8"))
            for key, value in original_settings.items():
                if key != "rust-analyzer.check.extraEnv":
                    assert settings[key] == value, f"user setting was overwritten: {key}"
            assert settings["rust-analyzer.check.extraEnv"]["NESTRS_IDE_TEST"] == "preserved", settings
            for key, value in expected_check_env.items():
                assert settings["rust-analyzer.check.extraEnv"][key] == value, f"selected tool path not saved: {key}"
                assert Path(value).is_file(), value
            assert any(paths_match(model_path, path) for path in settings["rust-analyzer.linkedProjects"]), settings
            assert Path(settings["rust-analyzer.procMacro.server"]).is_file(), settings
            assert settings["rust-analyzer.check.overrideCommand"][1:3] == ["init", "check"], settings
            assert settings["rust-analyzer.cfg.setTest"] is False, settings
            assert settings["rust-analyzer.cargo.cfgs"] == [], settings
            evidence["lsp"] = lsp_cases(server, project, settings, environment, output / f"{name}-lsp", name == "default", release)
            if name == "default":
                evidence["configured_check"] = configured_check_cases(project, settings, environment, output, model_path)
            report["cases"].append({"name": name, "passed": True, **evidence})
            print(f"PASS IDE generated project {name}: {evidence['crate_count']} crates, real build-script cfg/env", flush=True)
        report["rejected_cfg"] = rejected_cfg_case(cli, project, environment, output, cargo_target)
        assert source_hashes(fixture) == before, "original IDE fixture changed"
        report["sources_unchanged"] = True
        report["passed"] = True
    finally:
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Verified stock rust-analyzer IDE behavior; report: {output / 'report.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, ValueError, KeyError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

#!/usr/bin/env python3
"""Run the 14 automatic-binding regressions through the production Nestrs CLI.

The compiler implementation is built from cargo-nestrs, including its private
proc-macro bridge and snapshot/overlay tests. Application files are never
rewritten by the compiler. All verifier-owned artifacts live below target.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from verify import ProbeFailure, require
from toolchain_support import bridge_name, cache_directory, compiler_command_environment, compiler_library_environment, executable_name, validate_compiler


def invoke(args: list[str], environment: dict[str, str], root: Path, log: Path) -> subprocess.CompletedProcess:
    result = subprocess.run(args, cwd=root, env=environment, text=True, capture_output=True, check=False, encoding="utf-8")
    log.with_suffix(".stdout.txt").write_text(result.stdout, encoding="utf-8")
    log.with_suffix(".stderr.txt").write_text(result.stderr, encoding="utf-8")
    return result


def source_hashes(directory: Path) -> dict[str, str]:
    files = list(directory.rglob("*.rs")) + [directory / "Cargo.toml", directory / "Cargo.lock"]
    return {str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest() for path in files}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default=os.environ.get("NESTRS_RUSTC", "rustc"), help="Compiler executable; its full identity must match the production pin")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    root = directory.parents[1]
    output = root / "target" / "nestrs-autobind"
    output.mkdir(parents=True, exist_ok=True)
    report: dict = {"scope": "Production cargo nestrs proc-macro declarations and automatic bindings", "passed": False, "cases": []}
    environment = os.environ.copy()
    for name in ("RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "NESTRS_GRAPH_TARGET", "NESTRS_DRIVER", "NESTRS_RUSTC"):
        environment.pop(name, None)
    compiler_command_environment(environment, args.rustc)
    try:
        expected = json.loads((root / "cargo-nestrs" / "toolchain.json").read_text(encoding="utf-8"))
        version = invoke([args.rustc, "-vV"], environment, root, output / "rustc-version")
        require(version.returncode == 0, "Cannot query rustc; inspect rustc-version.stderr.txt")
        actual = dict(line.split(": ", 1) for line in version.stdout.splitlines() if ": " in line)
        report["toolchain"] = actual
        validate_compiler(expected, actual)
        sysroot_result = invoke([args.rustc, "--print", "sysroot"], environment, root, output / "sysroot")
        require(sysroot_result.returncode == 0, "Cannot determine compiler sysroot")
        sysroot = Path(sysroot_result.stdout.strip())
        rustc = sysroot / "bin" / executable_name("rustc")
        metadata = sysroot / "lib" / "rustlib" / actual["host"] / "lib"
        require(any(metadata.glob("librustc_middle-*.rmeta")), "Install matching rustc-dev before running this verifier")
        compiler_library_environment(environment, sysroot, actual["host"])

        # Build the production package, rather than a second copy of its driver.
        compile_environment = environment.copy()
        compile_environment["RUSTC"] = str(rustc)
        compile_environment["RUSTC_BOOTSTRAP"] = "nestrs_driver"
        compile_environment["CARGO_TARGET_DIR"] = str(root / "target")
        compile_environment["CARGO_NET_OFFLINE"] = "true"
        package = ["-p", "cargo-nestrs", "--features", "compiler-driver", "--locked", "--offline", "--target-dir", str(root / "target")]
        built = invoke([sys.executable, str(root / "tools/build-toolchain.py"), "--rustc", str(rustc)], compile_environment, root, output / "driver-build")
        require(built.returncode == 0, f"Production toolchain build failed; inspect {output / 'driver-build.stderr.txt'}")
        tested = invoke(["cargo", "test", *package, "--bin", "nestrs-driver"], compile_environment, root, output / "driver-tests")
        require(tested.returncode == 0, f"Production source-overlay/snapshot tests failed; inspect {output / 'driver-tests.stderr.txt'}")
        required_tests = {
            "autobind_codegen::tests::generated_projection_is_a_latent_capability_not_an_explicit_binding",
            "autobind_codegen::tests::overlays_multiple_offsets_without_editing_the_original_or_its_line_count",
            "autobind_codegen::tests::rejects_offsets_inside_a_multibyte_character_and_beyond_the_file",
            "autobind_codegen::tests::eof_insertions_cannot_be_swallowed_by_an_unterminated_line_comment",
            "autobind_codegen::tests::source_changes_are_rejected_before_generation_and_before_second_pass_reads",
            "snapshot_tests::repeated_reads_cannot_replace_the_original_snapshot",
            "snapshot_tests::second_pass_checks_unmodified_modules_too_and_rejects_new_sources",
        }
        passed_tests = {line.removeprefix("test ").removesuffix(" ... ok")
                        for line in tested.stdout.splitlines() if line.startswith("test ") and line.endswith(" ... ok")}
        require(required_tests <= passed_tests, f"Missing source safety tests: {required_tests - passed_tests}")
        report["overlay_tests"] = {"passed": True, "implementation": "cargo-nestrs/src/bin/nestrs-driver.rs", "output": tested.stdout}

        # A verifier run uses immutable copies of the just-built production tools,
        # so unrelated concurrent workspace rebuilds cannot change its driver hash.
        tools = output / "toolchain"
        tools.mkdir(exist_ok=True)
        cli = tools / executable_name("cargo-nestrs")
        driver = tools / executable_name("nestrs-driver")
        shutil.copy2(root / "target" / "debug" / cli.name, cli)
        shutil.copy2(root / "target" / "debug" / driver.name, driver)
        bridge = tools / bridge_name()
        shutil.copy2(root / "target" / "debug" / bridge.name, bridge)
        wrapped = environment.copy()
        wrapped["NESTRS_RUSTC"] = str(rustc)
        wrapped["NESTRS_DRIVER"] = str(driver)
        wrapped["NESTRS_MACRO_BRIDGE"] = str(bridge)
        doctor = invoke([str(cli), "doctor"], wrapped, root, output / "doctor")
        require(doctor.returncode == 0, f"Production toolchain doctor failed; inspect {output / 'doctor.stderr.txt'}")
        fingerprint = next((line.removeprefix("driver fingerprint: ") for line in doctor.stdout.splitlines() if line.startswith("driver fingerprint: ")), None)
        require(fingerprint is not None, "Doctor did not report the production driver fingerprint")
        report["driver_fingerprint"] = fingerprint
        report["driver_sha256"] = hashlib.sha256(driver.read_bytes()).hexdigest()
        report["macro_bridge_sha256"] = hashlib.sha256(bridge.read_bytes()).hexdigest()

        fixture = directory / "fixtures" / "auto-binding"
        before = source_hashes(fixture)
        manifest = fixture / "Cargo.toml"
        # Producers now publish latent projection capabilities even when an
        # interface is requested only downstream. Count actual DI demand, not
        # every precompiled Send/Sync spelling of those capabilities. The Rust
        # fixtures additionally check exact TypeId pairs and runtime behavior.
        expected_requests = {"positive": 6, "ambiguity": 1, "unsatisfied_bound": 1,
                             "explicit": 1, "duplicate_explicit": 0, "cfg_selected": 1,
                             "semantic_edges": 3, "unreferenced_generic": 0,
                             "factory_override": 0, "factory_other_key": 1, "source_forms": 3,
                             "explicit_generic_root": 1, "higher_ranked": 3}
        expected_explicit = {"explicit": 1, "duplicate_explicit": 1, "explicit_generic_root": 1}
        for phase, extra in (("default", ["--bins"]), ("alternate", ["--features", "alternate", "--bin", "cfg_selected"])):
            cargo_base = output / "cargo" / phase
            isolated = cache_directory(cargo_base, expected["release"], expected["commit_hash"], actual["host"], fingerprint)
            generated = isolated / "nestrs" / "compiler"
            # Recompile only this verifier's application package so every report
            # is freshly generated; retain the ordinary dependency build cache.
            cleaned = invoke(["cargo", "clean", "--manifest-path", str(manifest), "--package", "nestrs-auto-binding-fixture",
                              "--target-dir", str(isolated)], environment, root, output / f"fixture-clean-{phase}")
            require(cleaned.returncode == 0, "Cannot rebuild the isolated fixture package")
            if generated.exists():
                shutil.rmtree(generated)
            command = [str(cli), "build", "--manifest-path", str(manifest), "--locked", "--offline",
                       "--target-dir", str(cargo_base), "--message-format=json", *extra]
            compiled = invoke(command, wrapped, root, output / f"cargo-{phase}")
            require(compiled.returncode == 0, f"Fixture compilation failed; inspect {output / f'cargo-{phase}.stderr.txt'}")
            executables = {}
            for line in compiled.stdout.splitlines():
                message = json.loads(line)
                if message.get("reason") == "compiler-artifact" and message.get("executable"):
                    executables[message["target"]["name"]] = message["executable"]
            wanted = set(expected_requests) if phase == "default" else {"cfg_selected"}
            require(set(executables) == wanted, f"Unexpected fixture binary set: expected {wanted}, found {set(executables)}")
            analyses = {}
            for path in generated.rglob("analysis.json"):
                analysis = json.loads(path.read_text(encoding="utf-8"))
                if analysis["crate"] not in {name.replace("-", "_") for name in wanted}:
                    # All executable entries now get a registry, including Cargo
                    # build scripts. Their shared crate names are not fixture cases.
                    continue
                status = json.loads(path.with_name("compilation.json").read_text(encoding="utf-8"))
                require(status["passed"] and status["passes"] == 2, f"Final compilation did not pass for {path}")
                require(analysis["crate"] not in analyses, f"Duplicate fresh analysis for {analysis['crate']}")
                analyses[analysis["crate"]] = (analysis, path)
            for name, executable in sorted(executables.items()):
                analysis, analysis_file = analyses[name.replace("-", "_")]
                require(analysis["requests"] == expected_requests[name],
                        f"{name}: expected {expected_requests[name]} actual interface requests, got {analysis['requests']}")
                require(analysis["explicit_bindings"] == expected_explicit.get(name, 0),
                        f"{name}: explicit binding pairs changed unexpectedly")
                pairs = [(binding["concrete"], binding["interface"]) for binding in analysis["bindings"]]
                require(len(pairs) == analysis["generated_bindings"],
                        f"{name}: generated capability count and source records disagree")
                require(len(set(pairs)) == len(pairs), f"{name}: generated duplicate local capability pairs")
                executed = invoke([executable], environment, root, output / f"run-{phase}-{name}")
                require(executed.returncode == 0, f"Runtime assertions failed: {phase}/{name}; inspect its run log")
                report["cases"].append({"name": name, "phase": phase, "passed": True,
                                        "generated_bindings": analysis["generated_bindings"],
                                        "requests": analysis["requests"],
                                        "explicit_bindings": analysis["explicit_bindings"],
                                        "analysis": str(analysis_file), "stdout": executed.stdout})
                print(f"PASS {phase}/{name} ({analysis['requests']} interface requests; {analysis['generated_bindings']} projection capabilities)")
        require(source_hashes(fixture) == before, "Application sources changed during the two-pass build")
        require(len(report["cases"]) == 14, "Expected all 14 automatic-binding integration executions")
        report["application_sources_unchanged"] = True
        report["passed"] = True
    except (ProbeFailure, OSError, ValueError, KeyError, RuntimeError) as error:
        report["failure"] = str(error)
        print(f"FAIL {error}", file=sys.stderr)
    report_file = output / "report.json"
    report_file.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {report_file}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

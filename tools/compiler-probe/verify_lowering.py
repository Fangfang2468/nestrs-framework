#!/usr/bin/env python3
"""Verify early compiler callback coverage and the limits of inert attributes.

This miniature probe intentionally does not implement Nestrs lowering. It
proves why declaration rewriting still needs a real expansion frontend while
the shared code generator is extracted from the procedural macro crate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True

from verify import ProbeFailure, execute, require


def fingerprints(directory: Path) -> dict[str, str]:
    return {
        str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(directory.rglob("*.rs"))
    }


def records_for(records: list[dict], phase: str) -> dict[str, dict]:
    selected = [
        record for record in records
        if record["phase"] == phase and record["kind"] == "struct"
    ]
    result = {record["name"]: record for record in selected}
    require(len(result) == len(selected), f"Repeated struct records in {phase}")
    return result


def verify_coverage(records: list[dict], alternate: bool) -> list[str]:
    root = records_for(records, "root_parsed")
    expanded = records_for(records, "expanded")
    require(
        set(root) == {"Root", "Alternate", "DefaultBranch", "Disabled"},
        f"Unexpected root callback contents: {sorted(root)}",
    )
    active = "Alternate" if alternate else "DefaultBranch"
    require(
        set(expanded) == {"Root", "External", "Generated", active},
        f"Unexpected expanded callback contents: {sorted(expanded)}",
    )
    require(expanded["Generated"]["from_expansion"], "Macro origin was lost")
    require(not expanded["External"]["from_expansion"], "External source was marked generated")
    require("external.rs" in expanded["External"]["source"], "External source span was lost")
    require(
        all("nestrs::injectable" in record["declaration"] for record in expanded.values()),
        "Registered tool attributes should remain inert in the expanded AST",
    )
    require(
        "value: String" in expanded["Root"]["declaration"],
        "The probe must not rewrite DI field types",
    )
    require(
        any(record["phase"] == "analyzed" for record in records),
        "Positive fixture did not complete type checking",
    )
    return [
        "root_callback_excludes_external_and_generated_items",
        "root_callback_precedes_cfg_selection",
        "expanded_callback_includes_external_and_generated_items",
        "expanded_callback_follows_cfg_selection",
        "source_spans_and_macro_origin_preserved",
        "tool_attributes_are_legal_but_inert",
        "field_types_remain_unmodified",
        "normal_derive_and_macro_call_site_name_resolution",
    ]


def run_case(
    name: str,
    compiler: Path | str,
    fixture: Path,
    sysroot: Path,
    output: Path,
    environment: dict[str, str],
    root: Path,
    alternate: bool = False,
    expected_error: str | None = None,
) -> dict:
    binary = output / name
    binary.unlink(missing_ok=True)
    args = [
        str(compiler), str(fixture), "--crate-name", name,
        "--edition", "2024", "--sysroot", str(sysroot), "-o", str(binary),
    ]
    if alternate:
        args += ["--cfg", 'feature="alternate"']
    result = execute(args, environment, root)
    records_file = output / f"{name}.jsonl"
    diagnostics = output / f"{name}.diagnostics.txt"
    records_file.write_text(result.stdout, encoding="utf-8")
    diagnostics.write_text(result.stderr, encoding="utf-8")
    observed = {
        "name": name,
        "command": args,
        "exit_code": result.returncode,
        "records": str(records_file),
        "diagnostics": str(diagnostics),
        "expected_error": expected_error,
        "passed": False,
    }
    try:
        records = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        if expected_error:
            require(result.returncode != 0, f"{name}: invalid source was accepted")
            require(
                f"error[{expected_error}]" in result.stderr,
                f"{name}: expected {expected_error}; inspect {diagnostics}",
            )
            require(not binary.exists(), f"{name}: failed compilation left an executable")
            if expected_error == "E0277":
                require("Consumer" in records_for(records, "expanded"), "Expansion was not reached")
                require(
                    not any(record["phase"] == "analyzed" for record in records),
                    "Inert declarations unexpectedly passed type checking",
                )
                require(
                    "port: dyn Port" in result.stderr and "async fn make" in result.stderr,
                    "Both the class field and factory parameter must require lowering",
                )
            else:
                require(not records, "Plain rustc unexpectedly emitted probe records")
            observed["checks"] = [f"rustc_rejected_{expected_error}", "no_executable_emitted"]
        else:
            require(result.returncode == 0, f"{name}: compilation failed; inspect {diagnostics}")
            require(binary.exists(), f"{name}: successful compilation did not emit an executable")
            observed["checks"] = verify_coverage(records, alternate)
            ran = execute([str(binary)], environment, root)
            (output / f"{name}.runtime.txt").write_text(ran.stdout + ran.stderr, encoding="utf-8")
            require(ran.returncode == 0, f"{name}: normal Rust name resolution/derive checks failed")
            observed["checks"].append("executable_assertions_passed")
            observed["runtime_exit_code"] = ran.returncode
        observed["passed"] = True
    except (ProbeFailure, ValueError, KeyError, TypeError) as error:
        observed["failure"] = str(error)
    return observed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc", help="Must match toolchain.json")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    root = directory.parents[1]
    fixtures = directory / "fixtures" / "lowering"
    output = root / "target" / "nestrs-lowering"
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "scope": "Pinned compiler hook coverage; tool attributes are not DI lowering",
        "passed": False,
        "cases": [],
    }
    environment = os.environ.copy()
    environment.pop("RUSTC_BOOTSTRAP", None)
    before = fingerprints(fixtures)
    try:
        expected = json.loads((directory / "toolchain.json").read_text(encoding="utf-8"))
        version = execute([args.rustc, "-vV"], environment, root)
        require(version.returncode == 0, f"Cannot query compiler version: {version.stderr}")
        observed = dict(line.split(": ", 1) for line in version.stdout.splitlines() if ": " in line)
        report["toolchain"] = {"expected": expected, "observed": observed}
        for expected_key, actual_key in (("release", "release"), ("commit_hash", "commit-hash"), ("host", "host")):
            require(
                observed.get(actual_key) == expected[expected_key],
                f"Unsupported rustc {actual_key}: expected {expected[expected_key]!r}, "
                f"observed {observed.get(actual_key)!r}. No toolchain was changed.",
            )
        sysroot_result = execute([args.rustc, "--print", "sysroot"], environment, root)
        require(sysroot_result.returncode == 0, "Cannot determine compiler sysroot")
        sysroot = Path(sysroot_result.stdout.strip())
        libraries = sysroot / "lib" / "rustlib" / expected["host"] / "lib"
        require(
            any(libraries.glob("librustc_resolve-*.rmeta")),
            f"Matching rustc-dev component is required at {libraries}; this script does not install it",
        )
        driver = output / "driver"
        compile_args = [
            args.rustc, str(directory / "lowering_driver.rs"),
            "--crate-name", "nestrs_lowering_probe", "--edition", "2024",
            "-D", "warnings", "-C", "rpath", "-L", f"native={sysroot / 'lib'}",
            "-o", str(driver),
        ]
        compile_environment = environment.copy()
        compile_environment["RUSTC_BOOTSTRAP"] = "nestrs_lowering_probe"
        compiled = execute(compile_args, compile_environment, root)
        (output / "driver-build.diagnostics.txt").write_text(compiled.stderr, encoding="utf-8")
        report["driver_build"] = {"command": compile_args, "exit_code": compiled.returncode}
        require(compiled.returncode == 0, "Driver build failed; inspect driver-build.diagnostics.txt")
        runtime_environment = environment.copy()
        existing = runtime_environment.get("LD_LIBRARY_PATH", "")
        runtime_environment["LD_LIBRARY_PATH"] = str(sysroot / "lib") + (os.pathsep + existing if existing else "")
        cases = [
            ("coverage_default", driver, "coverage.rs", False, None),
            ("coverage_alternate", driver, "coverage.rs", True, None),
            ("inert_needs_lowering", driver, "inert_needs_lowering.rs", False, "E0277"),
            ("plain_rustc_unknown_tool", args.rustc, "coverage.rs", False, "E0433"),
        ]
        for name, compiler, fixture, alternate, expected_error in cases:
            case = run_case(
                name, compiler, fixtures / fixture, sysroot, output,
                runtime_environment, root, alternate, expected_error,
            )
            report["cases"].append(case)
            print(f"{'PASS' if case['passed'] else 'FAIL'} {name}")
        after = fingerprints(fixtures)
        report["source_snapshot"] = {"before": before, "after": after, "unchanged": before == after}
        require(before == after, "Probe modified fixture source files")
        report["passed"] = all(case["passed"] for case in report["cases"])
    except (ProbeFailure, OSError, ValueError) as error:
        report["failure"] = str(error)
        print(f"FAIL {error}", file=sys.stderr)
    report_file = output / "report.json"
    report_file.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {report_file}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

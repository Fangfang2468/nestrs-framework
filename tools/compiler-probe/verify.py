#!/usr/bin/env python3
"""Verify the pinned semantic probe without Cargo or changing any toolchain.

This is a limited experiment. The fixtures' safe trait projections are written
by hand; neither this script nor the driver implements automatic DI binding.
All compiler processes are invoked with argument arrays, never through a shell.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


class ProbeFailure(Exception):
    """An unmet precondition or an observed compiler result mismatch."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ProbeFailure(message)


def execute(args: list[str], env: dict[str, str], cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        args, cwd=cwd, env=env, text=True, capture_output=True, check=False
    )


def find_record(records: list[dict], kind: str, field: str, suffix: str) -> dict:
    matches = [
        record
        for record in records
        if record.get("kind") == kind and record.get(field, "").endswith(suffix)
    ]
    require(len(matches) == 1, f"Expected one {kind} with {field} ending {suffix!r}: {matches}")
    return matches[0]


def verify_positive(records: list[dict], enabled: bool) -> list[str]:
    private = find_record(records, "impl", "self_type", "PrivateStore")
    require("private" in private["impl_path"], "Private module implementation was not observed")
    require(not private["generic"], "PrivateStore must be a closed implementation")
    require(not private["from_expansion"], "PrivateStore must be a direct implementation")
    require(private["trait_path"].endswith("Store"), "PrivateStore must implement Store")
    require("semantic_positive.rs" in private["source"], "Source span was not preserved")

    generated = find_record(records, "impl", "self_type", "MacroStore")
    require(generated["from_expansion"], "macro_rules implementation must retain expansion origin")
    require(generated["trait_path"].endswith("Store"), "MacroStore must implement Store")

    generic = find_record(records, "impl", "self_type", "Repository<T>")
    require(generic["generic"], "Open Repository implementation must be marked generic")
    require(generic["trait_path"].endswith("Store"), "Repository<T> must implement Store")

    alias = find_record(records, "type_alias", "path", "KnownRepository")
    require(not alias["generic"], "KnownRepository must be a closed type alias")
    for field in ("resolved_type", "normalized_type"):
        require(
            "Repository<" in alias[field] and alias[field].endswith("User>"),
            f"KnownRepository {field} must resolve to Repository<User>: {alias[field]}",
        )
    open_alias = find_record(records, "type_alias", "path", "RepositoryAlias")
    require(open_alias["generic"], "RepositoryAlias<T> must remain generic")
    projected = find_record(records, "type_alias", "path", "ProjectedRepository")
    require(not projected["generic"], "ProjectedRepository must be closed")
    require(
        "Repository<" in projected["normalized_type"]
        and projected["normalized_type"].endswith("User>"),
        "Associated-type alias must normalize to Repository<User>",
    )
    require(
        projected["resolved_type"] != projected["normalized_type"],
        "Associated-type alias must demonstrate a semantic normalization change",
    )

    active = "EnabledStore" if enabled else "DisabledStore"
    inactive = "DisabledStore" if enabled else "EnabledStore"
    find_record(records, "impl", "self_type", active)
    require(
        not any(record.get("self_type", "").endswith(inactive) for record in records),
        f"Inactive cfg branch {inactive} leaked into semantic records",
    )
    return [
        "private_module_impl",
        "macro_generated_impl_with_expansion_origin",
        "open_generic_impl",
        "closed_alias_resolution",
        "generic_alias_remains_open",
        "associated_type_alias_normalization",
        f"cfg_only_{active}",
        "handwritten_safe_projections_type_checked",
    ]


def run_case(
    name: str,
    fixture: Path,
    expected_error: str | None,
    enabled: bool,
    driver: Path,
    sysroot: Path,
    output: Path,
    env: dict[str, str],
    root: Path,
) -> dict:
    metadata_file = output / f"{name}.rmeta"
    # An earlier experiment must not make the stop-before-codegen check pass
    # or fail for the wrong reason. This path is owned by this harness.
    metadata_file.unlink(missing_ok=True)
    args = [
        str(driver),
        str(fixture),
        "--crate-name", name,
        "--crate-type", "lib",
        "--edition", "2024",
        "--sysroot", str(sysroot),
        "--emit=metadata",
        "-o", str(metadata_file),
    ]
    if enabled:
        args.extend(["--cfg", 'feature="enabled"'])
    result = execute(args, env, root)
    records_file = output / f"{name}.jsonl"
    diagnostics_file = output / f"{name}.diagnostics.txt"
    records_file.write_text(result.stdout, encoding="utf-8")
    diagnostics_file.write_text(result.stderr, encoding="utf-8")
    observed = {
        "name": name,
        "fixture": str(fixture.relative_to(root)),
        "command": args,
        "exit_code": result.returncode,
        "records": str(records_file),
        "diagnostics": str(diagnostics_file),
        "expected_error": expected_error,
        "metadata_emitted": metadata_file.exists(),
        "passed": False,
    }
    try:
        if expected_error:
            require(result.returncode != 0, f"{name}: invalid projection was accepted")
            require(
                f"error[{expected_error}]" in result.stderr,
                f"{name}: expected rustc {expected_error}; inspect {diagnostics_file}",
            )
            require(not result.stdout.strip(), f"{name}: failed analysis emitted semantic records")
            observed["checks"] = [f"rustc_rejected_{expected_error}", "no_analysis_records"]
        else:
            require(result.returncode == 0, f"{name}: analysis failed; inspect {diagnostics_file}")
            require(
                not metadata_file.exists(),
                f"{name}: after_analysis must stop before metadata or executable emission",
            )
            records = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
            require(bool(records), f"{name}: driver emitted no semantic records")
            observed["record_count"] = len(records)
            observed["checks"] = verify_positive(records, enabled)
            observed["checks"].append("stopped_before_metadata_emission")
        observed["passed"] = True
    except (ProbeFailure, ValueError, KeyError, TypeError) as error:
        observed["failure"] = str(error)
    return observed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc", help="Compiler executable; version must match toolchain.json")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    root = directory.parents[1]
    output = root / "target" / "nestrs-compiler-probe"
    output.mkdir(parents=True, exist_ok=True)
    report_file = output / "report.json"
    report = {
        "scope": "Limited rustc semantic probe; no production CLI or automatic binding generation",
        "passed": False,
        "cases": [],
    }

    # Never enable unstable features in fixtures or mutate the calling shell.
    environment = os.environ.copy()
    environment.pop("RUSTC_BOOTSTRAP", None)
    try:
        expected = json.loads((directory / "toolchain.json").read_text(encoding="utf-8"))
        version = execute([args.rustc, "-vV"], environment, root)
        require(version.returncode == 0, f"Cannot query compiler version: {version.stderr}")
        observed = dict(
            line.split(": ", 1) for line in version.stdout.splitlines() if ": " in line
        )
        report["toolchain"] = {"expected": expected, "observed": observed, "rustc": args.rustc}
        (output / "rustc-version.txt").write_text(version.stdout, encoding="utf-8")
        for expected_key, actual_key in (("release", "release"), ("commit_hash", "commit-hash"), ("host", "host")):
            require(
                observed.get(actual_key) == expected[expected_key],
                f"Unsupported rustc {actual_key}: expected {expected[expected_key]!r}, "
                f"observed {observed.get(actual_key)!r}. No toolchain was installed or changed.",
            )

        sysroot_result = execute([args.rustc, "--print", "sysroot"], environment, root)
        require(sysroot_result.returncode == 0, f"Cannot determine sysroot: {sysroot_result.stderr}")
        sysroot = Path(sysroot_result.stdout.strip())
        compiler_libraries = sysroot / "lib" / "rustlib" / expected["host"] / "lib"
        for crate in ("rustc_hir", "rustc_interface", "rustc_middle"):
            require(
                any(compiler_libraries.glob(f"lib{crate}-*.rmeta")),
                f"rustc-dev metadata for {crate} is missing from {compiler_libraries}. "
                "Install the matching rustc-dev component explicitly; this probe will not install it.",
            )

        driver = output / "driver"
        compile_args = [
            args.rustc,
            str(directory / "driver.rs"),
            "--crate-name", "nestrs_compiler_probe",
            "--edition", "2024",
            "-D", "warnings",
            "-C", "rpath",
            "-L", f"native={sysroot / 'lib'}",
            "-o", str(driver),
        ]
        compile_environment = environment.copy()
        compile_environment["RUSTC_BOOTSTRAP"] = "nestrs_compiler_probe"
        compiled = execute(compile_args, compile_environment, root)
        (output / "driver-build.stdout.txt").write_text(compiled.stdout, encoding="utf-8")
        (output / "driver-build.diagnostics.txt").write_text(compiled.stderr, encoding="utf-8")
        report["driver_build"] = {"command": compile_args, "exit_code": compiled.returncode}
        require(compiled.returncode == 0, f"Driver compilation failed; inspect {output / 'driver-build.diagnostics.txt'}")

        runtime_environment = environment.copy()
        existing_libraries = runtime_environment.get("LD_LIBRARY_PATH", "")
        runtime_environment["LD_LIBRARY_PATH"] = str(sysroot / "lib") + (
            os.pathsep + existing_libraries if existing_libraries else ""
        )
        fixtures = directory / "fixtures"
        cases = [
            ("semantic_default", "semantic_positive.rs", None, False),
            ("semantic_enabled", "semantic_positive.rs", None, True),
            ("reject_missing_impl", "reject_missing_impl.rs", "E0277", False),
            ("reject_generic_bound", "reject_generic_bound.rs", "E0277", False),
            ("reject_dyn_incompatible", "reject_dyn_incompatible.rs", "E0038", False),
        ]
        for name, file, expected_error, enabled in cases:
            result = run_case(
                name, fixtures / file, expected_error, enabled, driver,
                sysroot, output, runtime_environment, root,
            )
            report["cases"].append(result)
            print(f"{'PASS' if result['passed'] else 'FAIL'} {name}")
        report["passed"] = all(case["passed"] for case in report["cases"])
    except (ProbeFailure, OSError, ValueError) as error:
        report["failure"] = str(error)
        print(f"FAIL {error}", file=sys.stderr)
    report_file.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {report_file}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

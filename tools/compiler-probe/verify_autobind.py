#!/usr/bin/env python3
"""Build the pinned tools and run the production automatic-binding contracts.

All fixture selection and AOT assertions belong to the Rust integration harness.
This script only prepares its toolchain environment and retains command logs.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from toolchain_support import compiler_command_environment, compiler_library_environment, executable_name, query_doctor


def invoke(command, environment, root, log):
    """Retain unmodified command output and propagate its actual failure status."""
    result = subprocess.run(
        [str(argument) for argument in command], cwd=root, env=environment,
        text=True, capture_output=True, check=False, encoding="utf-8",
    )
    log.with_suffix(".stdout.txt").write_text(result.stdout, encoding="utf-8")
    log.with_suffix(".stderr.txt").write_text(result.stderr, encoding="utf-8")
    log.with_suffix(".status.json").write_text(
        json.dumps({"command": result.args, "exit_code": result.returncode}, indent=2) + "\n",
        encoding="utf-8",
    )
    result.check_returncode()
    return result


def run_contracts(root, output, rustc, skip_build, report):
    environment = os.environ.copy()
    for name in [
        "RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTC",
        "NESTRS_GRAPH_TARGET", "NESTRS_DRIVER", "NESTRS_MACRO_BRIDGE", "NESTRS_RUSTC",
    ]:
        environment.pop(name, None)
    if rustc is not None:
        compiler_command_environment(environment, rustc)
    if not skip_build:
        command = [sys.executable, root / "tools/build-toolchain.py"]
        if rustc is not None:
            command.extend(["--rustc", rustc])
        invoke(
            command, environment, root, output / "toolchain-build",
        )

    if rustc is not None:
        # The build helper accepts a command or rustup proxy, while the CLI's
        # explicit override requires the actual compiler. Resolve only its path;
        # doctor remains responsible for validating the complete identity.
        selected = invoke(
            [rustc, "--print", "sysroot"], environment, root, output / "selected-sysroot",
        )
        sysroot = selected.stdout.strip()
        if not sysroot:
            raise ValueError("The selected compiler did not report a sysroot")
        environment["NESTRS_RUSTC"] = str(Path(sysroot) / "bin" / executable_name("rustc"))

    # Cargo chooses where the just-built CLI lives. The CLI owns all subsequent
    # compiler identity, tool selection and cache-layout decisions.
    metadata = invoke(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked", "--offline"],
        environment, root, output / "cargo-metadata",
    )
    target = Path(json.loads(metadata.stdout)["target_directory"])
    cli = target / "debug" / executable_name("cargo-nestrs")
    doctor = query_doctor(cli, cwd=root, environment=environment, log=output / "doctor")
    report["toolchain"] = doctor

    environment.update({
        "RUSTC": doctor["compiler"],
        "NESTRS_RUSTC": doctor["compiler"],
        "NESTRS_DRIVER": doctor["driver"],
        "NESTRS_MACRO_BRIDGE": doctor["macro_bridge"],
        "CARGO_TARGET_DIR": str(target),
        "CARGO_NET_OFFLINE": "true",
        # Only the Rust harness's production driver target needs bootstrap.
        # The application CLI removes this before launching fixture builds.
        "RUSTC_BOOTSTRAP": "nestrs_driver",
    })
    compiler_library_environment(environment, Path(doctor["sysroot"]), doctor["rustc"]["host"])
    invoke(
        ["cargo", "test", "-p", "cargo-nestrs", "--features", "compiler-driver",
         "--test", "autobind_contracts", "--locked", "--offline"],
        environment, root, output / "autobind-contracts",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default=os.environ.get("NESTRS_RUSTC"),
                        help="compiler selected by the production toolchain")
    parser.add_argument("--skip-build", action="store_true", help="use the existing debug toolchain")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    base = root / "target/nestrs-autobind"
    base.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="run-", dir=base))
    report = {"suite": "cargo-nestrs/tests/autobind_contracts.rs", "passed": False}
    exit_code = 0
    try:
        run_contracts(root, output, args.rustc, args.skip_build, report)
        report["passed"] = True
    except subprocess.CalledProcessError as error:
        report["failure"] = str(error)
        report["failed_command_exit_code"] = error.returncode
        exit_code = error.returncode if error.returncode > 0 else 128 - error.returncode
        print(f"FAIL {error}", file=sys.stderr)
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        report["failure"] = str(error)
        exit_code = 1
        print(f"FAIL {error}", file=sys.stderr)
    report["exit_code"] = exit_code
    report_file = output / "report.json"
    report_file.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {report_file}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())

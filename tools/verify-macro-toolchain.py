#!/usr/bin/env python3
"""Run tool-owned macro bridge and external generic metadata cases in debug and release."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from toolchain_support import bridge_name, executable_name


def hashes(directory):
    sources = list(directory.rglob("*.rs")) + [directory / "Cargo.toml", directory / "Cargo.lock"]
    return {
        str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(sources)
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true", help="use the already built debug CLI and driver")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    fixture = root / "cargo-nestrs/tests/fixtures/macro-cross-crate"
    output = root / "target/nestrs-macro-toolchain"
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    for key in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"]:
        environment.pop(key, None)
    if not args.skip_build:
        subprocess.run([sys.executable, str(root / "tools/build-toolchain.py")], cwd=root, env=environment, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=environment, text=True, encoding="utf-8",
    ))
    binary_directory = Path(metadata["target_directory"]) / "debug"
    # Other verifiers may rebuild the workspace; use immutable executable copies.
    toolchain = output / "toolchain"
    toolchain.mkdir(exist_ok=True)
    for name in [executable_name("cargo-nestrs"), executable_name("nestrs-driver")]:
        shutil.copy2(binary_directory / name, toolchain / name)
    bridge = Path(environment.get("NESTRS_MACRO_BRIDGE", str(binary_directory / bridge_name())))
    shutil.copy2(bridge, toolchain / bridge_name())
    cli = toolchain / executable_name("cargo-nestrs")
    environment["NESTRS_DRIVER"] = str(toolchain / executable_name("nestrs-driver"))
    environment["NESTRS_MACRO_BRIDGE"] = str(toolchain / bridge_name())
    environment["CARGO_TARGET_DIR"] = str(output / "cargo")
    before = hashes(fixture)
    report = {"passed": False, "cases": [], "source_hashes": before}
    try:
        checked = subprocess.run(
            [str(cli), "check", "--locked", "--all-targets", "--manifest-path", str(fixture / "Cargo.toml")],
            cwd=root, env=environment, text=True, capture_output=True, encoding="utf-8",
        )
        (output / "check-all-targets.stdout.txt").write_text(checked.stdout, encoding="utf-8")
        (output / "check-all-targets.stderr.txt").write_text(checked.stderr, encoding="utf-8")
        assert checked.returncode == 0, f"metadata-only cross-crate check failed; inspect {output / 'check-all-targets.stderr.txt'}"
        report["metadata_check_passed"] = True
        print("PASS check-all-targets (external generic MIR in rmeta)", flush=True)
        for profile in ["debug", "release"]:
            for binary, expected in [
                ("macro_frontend", "macro frontend+primary+cfg+external+macro+factory+trait binding passed"),
                ("external_blueprints", "external blueprints: chain, alias, shared trait identity and all keys passed"),
                ("inherited_pair", "inherited binding: upstream projection reused without duplicate registration"),
            ]:
                command = [str(cli), "run", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "--bin", binary]
                if profile == "release":
                    command.append("--release")
                result = subprocess.run(command, cwd=root, env=environment, text=True, capture_output=True, encoding="utf-8")
                name = f"{binary}-{profile}"
                (output / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
                (output / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
                assert result.returncode == 0, f"{name} failed; inspect {output / (name + '.stderr.txt')}"
                assert expected in result.stdout, f"{name} did not finish its runtime assertions"
                report["cases"].append({"binary": binary, "profile": profile, "passed": True})
                print(f"PASS {name}", flush=True)
        assert hashes(fixture) == before, "macro compilation modified original fixture source or manifest"
        report["sources_unchanged"] = True
        report["passed"] = True
    finally:
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Verified 6 executions; report: {output / 'report.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

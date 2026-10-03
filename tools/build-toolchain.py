#!/usr/bin/env python3
"""Bootstrap the CLI, driver and private bridge from this repository's sources.

This entry point works before a Nestrs CLI exists. It checks an already installed
compiler and prepares only the environment needed to build the three artifacts;
it does not install, upgrade or distribute a toolchain. Once built, use the CLI's
doctor command for artifact discovery and its structured toolchain report.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

from toolchain_support import bridge_name, compiler_command_environment, compiler_library_environment, executable_name, validate_compiler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--rustc", default=os.environ.get("NESTRS_RUSTC", "rustc"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    expected = json.loads((root / "cargo-nestrs/toolchain.json").read_text(encoding="utf-8"))
    environment = os.environ.copy()
    for name in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"]:
        environment.pop(name, None)
    compiler_command_environment(environment, args.rustc)
    version = subprocess.check_output([args.rustc, "-vV"], env=environment, text=True, encoding="utf-8")
    actual = dict(line.split(": ", 1) for line in version.splitlines() if ": " in line)
    validate_compiler(expected, actual)
    sysroot = Path(subprocess.check_output([args.rustc, "--print", "sysroot"], env=environment, text=True, encoding="utf-8").strip())
    libraries = sysroot / "lib/rustlib" / actual["host"] / "lib"
    if not list(libraries.glob("librustc_middle-*.rmeta")):
        raise RuntimeError("The matching rustc-dev component is required; no components were installed")
    # Only the driver and private bridge need unstable compiler APIs. The bridge
    # uses definition-site spans for generated bindings; application builds never
    # inherit this setting because the CLI explicitly removes it in child builds.
    environment["RUSTC_BOOTSTRAP"] = "nestrs_driver,nestrs_tool_bridge"
    environment["RUSTC"] = str(sysroot / "bin" / executable_name("rustc"))
    compiler_library_environment(environment, sysroot, actual["host"])
    cmd = ["cargo", "build", "-p", "cargo-nestrs", "-p", "nestrs-tool-bridge", "--features", "cargo-nestrs/compiler-driver"]
    if args.release:
        cmd.append("--release")
    result = subprocess.run(cmd, cwd=root, env=environment)
    if result.returncode:
        return result.returncode
    profile = "release" if args.release else "debug"
    metadata = subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=environment, text=True, encoding="utf-8")
    directory = Path(json.loads(metadata)["target_directory"]) / profile
    cli = directory / executable_name("cargo-nestrs")
    driver = directory / executable_name("nestrs-driver")
    bridge = directory / bridge_name()
    for path in [cli, driver, bridge]:
        if not path.is_file():
            raise RuntimeError(f"Build did not produce the expected tool: {path}")
    print(f"Built CLI: {cli}")
    print(f"Built driver: {driver}")
    print(f"Built private bridge: {bridge}")
    print(f"Use {cli} doctor, or add {directory} to PATH to use cargo nestrs.")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

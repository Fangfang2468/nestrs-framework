"""Host-specific paths shared by the Nestrs maintenance scripts.

This module is only a build/test helper. The installed CLI never imports Python.
"""

import os
from pathlib import Path
import shutil
import sys


def executable_name(name):
    return name + (".exe" if os.name == "nt" else "")


def bridge_name():
    if os.name == "nt":
        return "nestrs_tool_bridge.dll"
    return "libnestrs_tool_bridge" + (".dylib" if sys.platform == "darwin" else ".so")


def validate_compiler(expected, actual):
    for field, key in [("release", "release"), ("commit_hash", "commit-hash")]:
        if actual.get(key) != expected[field]:
            raise RuntimeError(f"Unsupported rustc {key}: expected {expected[field]}, found {actual.get(key)}")
    if actual.get("host") not in expected["hosts"]:
        raise RuntimeError(f"Unsupported rustc host: expected one of {expected['hosts']}, found {actual.get('host')}")


def cache_directory(target, release, commit, host, fingerprint):
    """Match Toolchain::cache_directory when inspecting generated artifacts."""
    base = target / "nestrs"
    if "windows" in host:
        digest = 0xCBF29CE484222325
        for field in [release, commit, host, fingerprint]:
            for byte in field.encode("utf-8") + b"\0":
                digest = ((digest ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        return base / f"{digest:016x}"
    return base / f"{release}-{commit}-{host}" / fingerprint


def compiler_command_environment(environment, compiler):
    """Make a directly selected Windows rustc load its adjacent runtime DLLs."""
    if os.name == "nt":
        path = Path(shutil.which(compiler, path=environment.get("PATH")) or compiler)
        if path.is_file():
            environment["PATH"] = str(path.resolve().parent) + os.pathsep + environment.get("PATH", "")


def compiler_library_environment(environment, sysroot, host):
    """Let directly executed driver/macro-server processes find compiler DLLs."""
    if os.name == "nt":
        key = "PATH"
        directories = [sysroot / "bin", sysroot / "lib" / "rustlib" / host / "lib", sysroot / "lib"]
    else:
        key = "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
        directories = [sysroot / "lib"]
    existing = environment.get(key, "")
    environment[key] = os.pathsep.join([*(str(path) for path in directories), *([existing] if existing else [])])


def macro_server(sysroot):
    name = executable_name("rust-analyzer-proc-macro-srv")
    candidates = [sysroot / "libexec" / name, sysroot / "bin" / name]
    return next((path for path in candidates if path.is_file()), candidates[0])

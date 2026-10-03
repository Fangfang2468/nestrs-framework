"""Host-specific paths shared by the Nestrs maintenance scripts.

This module is only a build/test helper. The installed CLI never imports Python.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def executable_name(name):
    """Return the host executable suffix without selecting an installed tool."""
    return name + (".exe" if os.name == "nt" else "")


def bridge_name():
    """Name the bootstrap bridge artifact using the current Python host convention."""
    if os.name == "nt":
        return "nestrs_tool_bridge.dll"
    return "libnestrs_tool_bridge" + (".dylib" if sys.platform == "darwin" else ".so")


def validate_compiler(expected, actual):
    """Check the bootstrap rustc -vV identity against the repository pin."""
    for field, key in [("release", "release"), ("commit_hash", "commit-hash")]:
        if actual.get(key) != expected[field]:
            raise RuntimeError(f"Unsupported rustc {key}: expected {expected[field]}, found {actual.get(key)}")
    if actual.get("host") not in expected["hosts"]:
        raise RuntimeError(f"Unsupported rustc host: expected one of {expected['hosts']}, found {actual.get('host')}")


def query_doctor(cli, *, environment=None, cwd=None, target_dir=None, log=None):
    """Read the installed CLI's versioned toolchain and optional cache paths.

    Paths and the fingerprint are opaque CLI results. Maintenance scripts must
    not reconstruct the production cache layout or parse human-facing output.
    When requested, raw stdout/stderr and the exact process status are retained
    even if the command fails or its JSON cannot be consumed.
    """
    command = [str(cli), "doctor", "--json"]
    if target_dir is not None:
        command.extend(["--target-dir", str(target_dir)])
    result = subprocess.run(
        command, cwd=cwd, env=environment, text=True, capture_output=True,
        check=False, encoding="utf-8",
    )
    if log is not None:
        log = Path(log)
        log.with_suffix(".stdout.txt").write_text(result.stdout, encoding="utf-8")
        log.with_suffix(".stderr.txt").write_text(result.stderr, encoding="utf-8")
        log.with_suffix(".status.json").write_text(
            json.dumps({"command": command, "exit_code": result.returncode}, indent=2) + "\n",
            encoding="utf-8",
        )
    result.check_returncode()
    return parse_doctor_output(result.stdout, require_directories=target_dir is not None)


def parse_doctor_output(output, *, require_directories=False):
    """Validate only the doctor JSON contract, never infer tool identity or paths."""
    try:
        value = json.loads(output)
    except json.JSONDecodeError as error:
        raise ValueError(f"Invalid doctor JSON: {error}") from error
    if not isinstance(value, dict):
        raise ValueError("Doctor JSON must be an object")
    if type(value.get("version")) is not int or value["version"] != 1:
        raise ValueError(f"Unsupported doctor JSON version: {value.get('version')!r}")

    def require_string(record, field, label):
        if not isinstance(record.get(field), str) or not record[field].strip():
            raise ValueError(f"Doctor JSON requires a nonempty string for {label}")

    identity = value.get("rustc")
    if not isinstance(identity, dict):
        raise ValueError("Doctor JSON requires a rustc identity object")
    for field in ["release", "commit_hash", "host"]:
        require_string(identity, field, f"rustc.{field}")
    for field in ["compiler", "sysroot", "driver", "macro_bridge", "fingerprint"]:
        require_string(value, field, field)
    for field in ["target_directory", "cache_directory", "compiler_output_directory"]:
        if field not in value:
            raise ValueError(f"Doctor JSON is missing {field}")
        if require_directories or value[field] is not None:
            require_string(value, field, field)
    return value


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
    """Find the probe server in an existing sysroot, leaving absence to the caller."""
    name = executable_name("rust-analyzer-proc-macro-srv")
    candidates = [sysroot / "libexec" / name, sysroot / "bin" / name]
    return next((path for path in candidates if path.is_file()), candidates[0])

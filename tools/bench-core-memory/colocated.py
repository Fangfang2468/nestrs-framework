#!/usr/bin/env python3
"""Run an already-built DI probe beside temporary PostgreSQL/Redis under one cgroup.

Only an explicit invocation creates resources. No image pull, host port, existing
volume, network, daemon setting, repository source, or compiler is modified.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
import uuid

MIB = 1024 * 1024
CGROUP = Path("/sys/fs/cgroup")
LABEL = "io.nestrs.capacity.run"
POSTGRES = "postgres:18.4-bookworm"
REDIS = "redis:8.6.3"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="a new report directory")
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--app-scenario", choices=["payload", "mixed", "scoped", "lazy4", "async4"], default="payload")
    parser.add_argument("--concurrency", type=int, default=128)
    parser.add_argument("--queries-per-scope", type=int, default=8)
    parser.add_argument("--payload-bytes", type=int, default=65536)
    parser.add_argument("--hold-ms", type=int, default=100)
    parser.add_argument("--sample-ms", type=int, default=250)
    args = parser.parse_args()
    if not 5 <= args.seconds <= 300:
        parser.error("--seconds must be between 5 and 300")
    if not 1 <= args.concurrency <= 8192 or not 1 <= args.queries_per_scope <= 1024:
        parser.error("concurrency or queries-per-scope is outside the bounded probe range")
    if not 1 <= args.payload_bytes <= 16 * MIB or not 10 <= args.hold_ms <= 1000:
        parser.error("payload-bytes or hold-ms is outside the bounded probe range")
    if not 50 <= args.sample_ms <= 1000:
        parser.error("sample-ms must be between 50 and 1000")
    binary = args.app_binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("app-binary must be an existing executable; this script never builds it")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_id = uuid.uuid4().hex
    prefix = "nc" + run_id
    slice_unit = prefix + ".slice"  # No hyphen: no extra automatically-created parent slices.
    app_unit = prefix + "app.scope"
    pg_name, redis_name, volume_name = prefix + "pg", prefix + "redis", prefix + "pgdata"
    handles, children, container_names = [], [], []
    slice_created = app_created = volume_created = False
    stop_sampling = threading.Event()
    sampler = None
    phase = "preflight"
    paths = {}
    sample_errors = []
    samples_summary = {}
    report = {"run_id": run_id, "passed": False, "started_unix": time.time(), "resources": {}, "commands": [], "cleanup": []}

    def save(name, value):
        (output / name).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")

    def execute(command, name=None, check=True, timeout=60, text_input=None):
        command = list(map(str, command))
        report["commands"].append(command)
        result = subprocess.run(command, input=text_input, capture_output=True, text=True, timeout=timeout)
        if name:
            (output / (name + ".stdout.txt")).write_text(result.stdout)
            (output / (name + ".stderr.txt")).write_text(result.stderr)
        if check and result.returncode:
            raise RuntimeError(f"command failed ({result.returncode}): {command!r}\n{result.stderr[-4000:]}")
        return result

    def launch(command, name):
        command = list(map(str, command))
        report["commands"].append(command)
        stdout = (output / (name + ".stdout.txt")).open("w")
        stderr = (output / (name + ".stderr.txt")).open("w")
        handles.extend([stdout, stderr])
        child = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        children.append(child)
        return child

    def unit_cgroup(unit):
        value = execute(["systemctl", "show", unit, "--property=ControlGroup", "--value"]).stdout.strip()
        if not value.startswith("/") or value == "/":
            raise RuntimeError(f"missing isolated cgroup for {unit}: {value!r}")
        return CGROUP / value.lstrip("/")

    def container_cgroup(name):
        info = json.loads(execute(["docker", "inspect", name]).stdout)[0]
        if info["Config"].get("Labels", {}).get(LABEL) != run_id:
            raise RuntimeError(f"container ownership mismatch: {name}")
        pid = info["State"]["Pid"]
        group = next(line.split("::", 1)[1] for line in Path(f"/proc/{pid}/cgroup").read_text().splitlines() if line.startswith("0::"))
        path = CGROUP / group.lstrip("/")
        if not path.is_relative_to(paths["slice"]):
            raise RuntimeError(f"container escaped the measured slice: {name}: {path}")
        return path

    def metrics(path):
        if not path.exists():
            return {"missing": True}
        result = {"path": str(path)}
        for name in ["memory.current", "memory.peak", "memory.max", "memory.events", "memory.events.local", "memory.swap.current", "memory.swap.peak", "memory.swap.max", "memory.stat", "cpu.stat", "cpu.max", "cpuset.cpus.effective", "pids.current"]:
            try:
                text = (path / name).read_text().strip()
            except FileNotFoundError:
                continue
            if name in ["memory.events", "memory.events.local", "memory.stat", "cpu.stat"]:
                result[name] = {key: int(value) for key, value in (line.split() for line in text.splitlines())}
            else:
                result[name] = int(text) if text.isdecimal() else text
        return result

    def check_memory(path, limit):
        data = metrics(path)
        if data.get("memory.max") != limit or data.get("memory.swap.max") != 0:
            raise RuntimeError(f"resource limits were not applied: {data}")
        if data.get("cpuset.cpus.effective") != "2,4":
            raise RuntimeError(f"unexpected effective CPUs: {data}")

    def sample_loop():
        with (output / "cgroups.jsonl").open("w") as log:
            while not stop_sampling.is_set():
                try:
                    snapshot = {"unix": time.time(), "monotonic": time.monotonic(), "phase": phase, "groups": {name: metrics(path) for name, path in list(paths.items())}}
                    log.write(json.dumps(snapshot) + "\n")
                    log.flush()
                    for name, value in snapshot["groups"].items():
                        item = samples_summary.setdefault(name, {"max_sampled_memory_current": 0, "max_kernel_memory_peak": 0, "max_swap_current": 0, "max_events": {}, "first_cpu_stat": None, "last_cpu_stat": None})
                        item["max_sampled_memory_current"] = max(item["max_sampled_memory_current"], value.get("memory.current", 0))
                        item["max_kernel_memory_peak"] = max(item["max_kernel_memory_peak"], value.get("memory.peak", 0))
                        item["max_swap_current"] = max(item["max_swap_current"], value.get("memory.swap.current", 0))
                        for key, count in value.get("memory.events", {}).items():
                            item["max_events"][key] = max(item["max_events"].get(key, 0), count)
                        if "cpu.stat" in value:
                            item["first_cpu_stat"] = item["first_cpu_stat"] or value["cpu.stat"]
                            item["last_cpu_stat"] = value["cpu.stat"]
                except Exception as error:
                    sample_errors.append(repr(error))
                stop_sampling.wait(args.sample_ms / 1000)

    def pg(command, name=None, timeout=60):
        return execute(["docker", "exec", "--user", "postgres", pg_name, *command], name=name, timeout=timeout)

    def redis(command, name=None, timeout=60):
        return execute(["docker", "exec", redis_name, *command], name=name, timeout=timeout)

    def wait_ready(command):
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if execute(command, check=False, timeout=10).returncode == 0:
                return
            time.sleep(0.25)
        raise RuntimeError(f"container readiness timeout: {command!r}")

    def capture_database_state(suffix):
        pg(["psql", "-U", "postgres", "-d", "capacity", "-At", "-F", "\t", "-c", "SELECT name, setting, unit FROM pg_settings WHERE name IN ('server_version','shared_buffers','max_connections','work_mem','maintenance_work_mem','effective_cache_size','max_wal_size','huge_pages','data_directory') ORDER BY name"], "postgres-config-" + suffix)
        pg(["psql", "-U", "postgres", "-d", "capacity", "-At", "-c", "SELECT pg_database_size(current_database()), (SELECT count(*) FROM pg_stat_activity), (SELECT count(*) FROM pgbench_accounts)"], "postgres-state-" + suffix)
        redis(["redis-cli", "INFO", "all"], "redis-info-" + suffix)
        redis(["redis-cli", "CONFIG", "GET", "maxmemory", "maxmemory-policy", "save", "appendonly", "io-threads"], "redis-config-" + suffix)
        redis(["redis-cli", "DBSIZE"], "redis-dbsize-" + suffix)

    def container_owned(name):
        found = execute(["docker", "inspect", name], check=False)
        return found.returncode == 0 and json.loads(found.stdout)[0]["Config"].get("Labels", {}).get(LABEL) == run_id

    def cleanup():
        nonlocal phase
        phase = "cleanup"
        if app_created:
            result = execute(["systemctl", "stop", app_unit], check=False, timeout=30)
            report["cleanup"].append({"app_scope": app_unit, "exit_code": result.returncode})
        for child in children:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
        for name in reversed(container_names):
            if container_owned(name):
                execute(["docker", "logs", name], "container-" + name[-5:], check=False)
                info = execute(["docker", "inspect", name], check=False)
                if info.returncode == 0:
                    save(name + "-final-inspect.json", json.loads(info.stdout))
                removed = execute(["docker", "rm", "--force", "--volumes", name], check=False, timeout=30)
                report["cleanup"].append({"container": name, "exit_code": removed.returncode})
                if removed.returncode:
                    report["passed"] = False
                    report["cleanup_failure"] = "a run-owned container could not be removed"
        if volume_created:
            found = execute(["docker", "volume", "inspect", volume_name], check=False)
            if found.returncode == 0 and json.loads(found.stdout)[0].get("Labels", {}).get(LABEL) == run_id:
                removed = execute(["docker", "volume", "rm", volume_name], check=False, timeout=30)
                report["cleanup"].append({"volume": volume_name, "exit_code": removed.returncode})
                if removed.returncode:
                    report["passed"] = False
                    report["cleanup_failure"] = "the run-owned PostgreSQL volume could not be removed"
        stop_sampling.set()
        if sampler:
            sampler.join(timeout=5)
        if slice_created:
            if "slice" in paths:
                save("slice-final.json", metrics(paths["slice"]))
            stopped = execute(["systemctl", "stop", slice_unit], check=False, timeout=30)
            report["cleanup"].append({"slice": slice_unit, "exit_code": stopped.returncode})
            execute(["systemctl", "reset-failed", app_unit, slice_unit], check=False)
        for handle in handles:
            handle.close()

    def interrupted(signum, _frame):
        raise RuntimeError(f"received signal {signum}; cleaning up this run's resources")

    for sig in [signal.SIGTERM, signal.SIGINT]:
        signal.signal(sig, interrupted)
    try:
        info = json.loads(execute(["docker", "info", "--format", "{{json .}}"]).stdout)
        if info.get("CgroupDriver") != "systemd" or str(info.get("CgroupVersion")) != "2":
            raise RuntimeError("Docker must already use the systemd cgroup driver and cgroup v2")
        images = {name: json.loads(execute(["docker", "image", "inspect", name]).stdout)[0] for name in [POSTGRES, REDIS]}
        save("images.json", images)
        report["app_binary"] = str(binary)
        report["app_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report["parameters"] = vars(args) | {"app_binary": str(binary), "output": str(output)}
        report["limits"] = {"slice_bytes": 1536 * MIB, "postgres_bytes": 640 * MIB, "redis_bytes": 256 * MIB, "app_bytes": 512 * MIB, "swap_bytes": 0, "cpus": [2, 4], "cpu_quota_percent": 200}
        report["resources"] = {"slice": slice_unit, "app_scope": app_unit, "postgres": pg_name, "redis": redis_name, "volume": volume_name, "network": "none"}
        save("manifest.json", report)
        execute(["uname", "-a"], "uname")
        execute(["systemctl", "--version"], "systemd-version")
        execute(["docker", "version"], "docker-version")
        save("host-memory-before.json", {"meminfo": Path("/proc/meminfo").read_text(), "cpus": (CGROUP / "cpuset.cpus.effective").read_text()})
        # Start a transient slice through systemd's native API. No unit file or
        # persistent property is written; bit mask 0x14 selects CPUs 2 and 4.
        execute(["busctl", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1", "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))", slice_unit, "fail", "7", "Description", "s", "Nestrs temporary capacity probe " + run_id, "MemoryAccounting", "b", "true", "CPUAccounting", "b", "true", "MemoryMax", "t", str(1536 * MIB), "MemorySwapMax", "t", "0", "AllowedCPUs", "ay", "1", "20", "CPUQuotaPerSecUSec", "t", "2000000", "0"], "slice-create")
        slice_created = True
        deadline = time.monotonic() + 10
        while execute(["systemctl", "is-active", "--quiet", slice_unit], check=False).returncode:
            if time.monotonic() >= deadline:
                raise RuntimeError("transient slice did not become active")
            time.sleep(0.1)
        paths["slice"] = unit_cgroup(slice_unit)
        check_memory(paths["slice"], 1536 * MIB)
        quota, period = metrics(paths["slice"])["cpu.max"].split()
        if quota == "max" or int(quota) != int(period) * 2:
            raise RuntimeError("aggregate CPU quota is not exactly two CPUs")
        execute(["systemctl", "show", slice_unit, "--property=MemoryMax,MemorySwapMax,AllowedCPUs,CPUQuotaPerSecUSec,ControlGroup"], "slice-properties")
        sampler = threading.Thread(target=sample_loop, daemon=True)
        sampler.start()
        phase = "database_startup"
        volume_created = True
        execute(["docker", "volume", "create", "--label", LABEL + "=" + run_id, volume_name], "volume-create")
        common = ["--detach", "--pull=never", "--network=none", "--cgroup-parent=" + slice_unit, "--label", LABEL + "=" + run_id, "--pids-limit=256"]
        container_names.append(pg_name)
        execute(["docker", "run", *common, "--name", pg_name, "--memory=640m", "--memory-swap=640m", "--mount", "type=volume,src=" + volume_name + ",dst=/var/lib/postgresql", "--env", "POSTGRES_HOST_AUTH_METHOD=trust", POSTGRES, "postgres", "-c", "shared_buffers=128MB", "-c", "max_connections=20", "-c", "work_mem=4MB", "-c", "maintenance_work_mem=32MB", "-c", "effective_cache_size=256MB", "-c", "max_wal_size=256MB", "-c", "huge_pages=off"], "postgres-start")
        paths["postgres"] = container_cgroup(pg_name)
        check_memory(paths["postgres"], 640 * MIB)
        container_names.append(redis_name)
        execute(["docker", "run", *common, "--name", redis_name, "--memory=256m", "--memory-swap=256m", REDIS, "redis-server", "--save", "", "--appendonly", "no", "--maxmemory", "128mb", "--maxmemory-policy", "allkeys-lru", "--io-threads", "1"], "redis-start")
        paths["redis"] = container_cgroup(redis_name)
        check_memory(paths["redis"], 256 * MIB)
        # The image's initdb bootstrap server accepts only Unix sockets; wait
        # for the final TCP listener so the initialization restart is complete.
        wait_ready(["docker", "exec", "--user", "postgres", pg_name, "pg_isready", "-h", "127.0.0.1", "-U", "postgres"])
        wait_ready(["docker", "exec", redis_name, "redis-cli", "PING"])
        phase = "dataset_initialization"
        pg(["createdb", "-U", "postgres", "capacity"], "postgres-createdb")
        pg(["pgbench", "-i", "-s", "10", "-U", "postgres", "capacity"], "pgbench-init", timeout=300)
        # Exactly 50,000 unique keys; do not approximate this with random SETs.
        seed_log = (output / "redis-seed.stdout.txt").open("w")
        seed_errors = (output / "redis-seed.stderr.txt").open("w")
        handles.extend([seed_log, seed_errors])
        seed_command = ["docker", "exec", "-i", redis_name, "redis-cli", "--pipe"]
        report["commands"].append(seed_command)
        seed = subprocess.Popen(seed_command, stdin=subprocess.PIPE, stdout=seed_log, stderr=seed_errors)
        children.append(seed)
        value = b"x" * 1024
        for index in range(50000):
            key = f"capacity:{index:012d}".encode()
            seed.stdin.write(b"*3\r\n$3\r\nSET\r\n$" + str(len(key)).encode() + b"\r\n" + key + b"\r\n$1024\r\n" + value + b"\r\n")
        seed.stdin.close()
        if seed.wait(timeout=90):
            raise RuntimeError("Redis dataset load failed")
        seed_log.flush()
        if int(redis(["redis-cli", "DBSIZE"]).stdout.strip()) != 50000:
            raise RuntimeError("Redis dataset does not contain exactly 50,000 keys")
        capture_database_state("before-load")
        save("cgroups-before-load.json", {name: metrics(path) for name, path in paths.items()})
        phase = "simultaneous_load"
        waves = math.ceil((args.seconds + 10) * 1000 / args.hold_ms)
        app_command = ["systemd-run", "--quiet", "--scope", "--unit=" + app_unit, "--slice=" + slice_unit, "--property=MemoryMax=512M", "--property=MemorySwapMax=0", str(binary), args.app_scenario, str(waves), str(args.concurrency), str(args.queries_per_scope), str(args.payload_bytes), str(args.hold_ms)]
        app = launch(app_command, "app")
        app_created = True
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            result = execute(["systemctl", "show", app_unit, "--property=ControlGroup", "--value"], check=False)
            if result.returncode == 0 and result.stdout.strip().startswith("/"):
                paths["app"] = CGROUP / result.stdout.strip().lstrip("/")
                break
            if app.poll() is not None:
                raise RuntimeError(f"application scope exited early: {app.returncode}")
            time.sleep(0.1)
        else:
            raise RuntimeError("application scope was not created")
        if not paths["app"].is_relative_to(paths["slice"]):
            raise RuntimeError("application escaped the measured slice")
        check_memory(paths["app"], 512 * MIB)
        pgbench = launch(["docker", "exec", "--user", "postgres", pg_name, "pgbench", "-U", "postgres", "-c", "8", "-j", "2", "-T", str(args.seconds), "-P", "5", "capacity"], "pgbench-load")
        load_start = time.monotonic()
        deadline = load_start + args.seconds
        redis_batches = []
        while time.monotonic() < deadline:
            for operation in ["GET", "SET"]:
                command = ["redis-benchmark", "-q", "-n", "100000", "-c", "16", "-P", "4", "-r", "50000", operation, "capacity:__rand_int__"]
                if operation == "SET":
                    command.append("x" * 1024)
                started = time.monotonic()
                result = redis(command, timeout=30)
                redis_batches.append({"operation": operation, "started_monotonic": started, "elapsed_seconds": time.monotonic() - started, "stdout": result.stdout, "stderr": result.stderr, "exit_code": result.returncode})
                save("redis-load.json", redis_batches)
                if app.poll() is not None and time.monotonic() < deadline:
                    raise RuntimeError(f"application finished before the bounded database load: {app.returncode}")
                if time.monotonic() >= deadline:
                    break
        report["load_seconds"] = time.monotonic() - load_start
        report["pgbench_exit_code"] = pgbench.wait(timeout=30)
        if report["pgbench_exit_code"]:
            raise RuntimeError("pgbench workload failed")
        phase = "app_drain"
        report["app_exit_code"] = app.wait(timeout=args.seconds + 90)
        if report["app_exit_code"]:
            raise RuntimeError("application assertions or process failed")
        phase = "post_load"
        capture_database_state("after-load")
        save("cgroups-after-load.json", {name: metrics(path) for name, path in paths.items()})
        for value in samples_summary.values():
            if value["max_events"].get("oom", 0) or value["max_events"].get("oom_kill", 0) or value["max_swap_current"]:
                raise RuntimeError("cgroup recorded OOM activity or swap usage")
        if sample_errors:
            raise RuntimeError(f"cgroup sampling failed: {sample_errors!r}")
        report["passed"] = True
    except Exception as error:
        report["failure"] = repr(error)
    finally:
        try:
            cleanup()
        except Exception as error:
            report["cleanup_failure"] = repr(error)
            report["passed"] = False
        report["finished_unix"] = time.time()
        report["cgroup_summary"] = samples_summary
        report["sample_errors"] = sample_errors
        save("report.json", report)
    print(json.dumps({"passed": report["passed"], "report": str(output / "report.json"), "failure": report.get("failure"), "cleanup_failure": report.get("cleanup_failure")}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

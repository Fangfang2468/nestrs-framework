#!/usr/bin/env python3
"""串行构建/交替采样 std 与 ahash，保留原始样本；只使用 Python 标准库。"""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import sys
import tomllib


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def environment():
    env = os.environ.copy()
    # 两个版本都用通用 Release 目标；不让外部 native/AES/包装器设置影响对照。
    for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_BOOTSTRAP",
                "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_TARGET_DIR", "RUSTC",
                "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_TARGET"]:
        env.pop(key, None)
    for key in list(env):
        if key.startswith("CARGO_PROFILE_RELEASE_") or (
                key.startswith("CARGO_TARGET_") and key.endswith("_RUSTFLAGS")):
            env.pop(key)
    return env


def build_binary(command, log, binary_name, env):
    result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True)
    log.write_text(result.stdout + "\n" + result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(f"构建失败，详见 {log}")
    paths = []
    for line in result.stdout.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (item.get("reason") == "compiler-artifact"
                and item.get("target", {}).get("name") == binary_name
                and item.get("executable")):
            paths.append(item["executable"])
    if not paths:
        raise RuntimeError(f"Cargo 未报告 {binary_name} 可执行文件，详见 {log}")
    return str(Path(paths[-1]).resolve())


def build(args):
    if args.baseline_root is None:
        raise ValueError("build/all 需要 --baseline-root，指向修改前的源码快照")
    env = environment()
    binaries = {}
    commands = []
    cores = {"before": args.baseline_root.resolve() / "nestrs-core",
             "after": ROOT / "nestrs-core"}
    for variant, core in cores.items():
        app = args.output / "apps" / variant
        subprocess.run([sys.executable, str(HERE / "prepare.py"), "--core", str(core),
                        "--output", str(app)], check=True, env=env)
        # 从对应快照继承解析结果；只允许 Cargo 补入基准 package 与 ahash 闭包。
        shutil.copyfile(core.parent / "Cargo.lock", app / "Cargo.lock")
        command = [str(args.cli.resolve()), "build", "--release", "--offline",
                   "--manifest-path", str(app / "Cargo.toml"), "--message-format=json"]
        commands.append(command)
        print(f"构建 {variant} DI 基准……", flush=True)
        binaries[variant] = build_binary(command, args.output / f"build-{variant}.log",
                                         "di-ahash-bench", env)

    locks = {variant: tomllib.loads((args.output / "apps" / variant / "Cargo.lock").read_text())
             for variant in cores}
    packages = {variant: {(p["name"], p["version"], p.get("source")): p.get("checksum")
                          for p in lock["package"] if p.get("source")}
                for variant, lock in locks.items()}
    if not packages["before"].items() <= packages["after"].items():
        raise RuntimeError("基准的既有依赖解析不同，不能把其影响算作 ahash 收益")

    micro = args.output / "apps" / "hash-tables"
    micro.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(HERE / "hash_tables.rs", micro / "main.rs")
    (micro / "Cargo.toml").write_text('''[package]
name = "di-hash-tables"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[[bin]]
name = "hash-tables"
path = "main.rs"
[dependencies]
ahash = "=0.8.12"
[profile.release]
opt-level = 3
debug = false
lto = false
codegen-units = 16
''', encoding="utf-8")
    command = ["cargo", "build", "--release", "--offline", "--manifest-path",
               str(micro / "Cargo.toml"), "--message-format=json"]
    commands.append(command)
    binaries["hash-tables"] = build_binary(command, args.output / "build-hash-tables.log",
                                           "hash-tables", env)
    save(args.output / "build.json", {
        "binaries": binaries, "commands": commands,
        "locks": locks,
        "binary_sha256": {name: digest(Path(path)) for name, path in binaries.items()},
        "sources": {variant: {str(path.relative_to(core)): digest(path)
                              for path in sorted(core.rglob("*.rs"))}
                    for variant, core in cores.items()},
        "fixture_sha256": {name: digest(HERE / name) for name in ["main.rs", "hash_tables.rs"]},
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "rustc_cfg": subprocess.check_output(["rustc", "--print", "cfg"], text=True),
        "toolchain": subprocess.check_output([str(args.cli.resolve()), "doctor"], env=env, text=True),
    })


def run_sample(command, cpus, env):
    # affinity 从子进程继承到 Tokio workers；不修改用户当前 shell 的 affinity。
    pin = None
    if cpus:
        if not hasattr(os, "sched_setaffinity"):
            raise RuntimeError("当前系统不支持 sched_setaffinity，请省略 CPU 参数")
        pin = lambda: os.sched_setaffinity(0, set(map(int, cpus.split(","))))
    output = subprocess.check_output(command, env=env, text=True, preexec_fn=pin)
    return json.loads(output.strip())


def measure(args):
    built = json.loads((args.output / "build.json").read_text())
    for name, path in built["binaries"].items():
        if digest(Path(path)) != built["binary_sha256"][name]:
            raise RuntimeError(f"{name} 二进制已改变，请重新 build")
    for name, expected in built["fixture_sha256"].items():
        if digest(HERE / name) != expected:
            raise RuntimeError(f"{name} 源码已改变，请重新 build")
    env = environment()
    binaries = built["binaries"]
    workloads = []
    for scenario in ["usize-lookup", "route-lookup", "parent-churn"]:
        commands = {variant: [binaries["hash-tables"], "--hasher", hasher,
                              "--scenario", scenario, "--iterations", str(args.hash_iterations)]
                    for variant, hasher in [("before", "std"), ("after", "ahash")]}
        workloads.append(("hash", "none", scenario, args.single_cpu, commands))
    for runtime in ["current-thread", "multi-thread"]:
        for scenario in ["warm-singleton", "keyed-trait", "scoped-cache", "transient-fanin"]:
            iterations = args.di_iterations if runtime == "current-thread" else args.multi_iterations
            if scenario == "transient-fanin":
                iterations = args.transient_iterations
            commands = {variant: [binaries[variant], "--scenario", scenario, "--runtime", runtime,
                                  "--iterations", str(iterations), "--warmup", "2000"]
                        for variant in ["before", "after"]}
            cpus = args.single_cpu if runtime == "current-thread" else args.multi_cpus
            workloads.append(("di", runtime, scenario, cpus, commands))
    save(args.output / "measurement.json", {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(), "cpu_count": os.cpu_count(),
        "cpu_info": Path("/proc/cpuinfo").read_text() if Path("/proc/cpuinfo").exists() else None,
        "rounds": args.rounds, "single_cpu": args.single_cpu, "multi_cpus": args.multi_cpus,
        "workloads": workloads, "runner_sha256": digest(Path(__file__)),
    })
    # 无剔除离群值。每轮恰好一对 A/B，偶数轮反转执行顺序，减小时间漂移偏差。
    with (args.output / "samples.jsonl").open("w", encoding="utf-8") as raw:
        for kind, runtime, scenario, cpus, commands in workloads:
            for round_index in range(args.rounds):
                order = ["before", "after"] if round_index % 2 == 0 else ["after", "before"]
                pair = {}
                for variant in order:
                    sample = run_sample(commands[variant], cpus, env)
                    sample.update(kind=kind, runtime=runtime, variant=variant, round=round_index)
                    raw.write(json.dumps(sample) + "\n")
                    raw.flush()
                    pair[variant] = sample
                if pair["before"]["checksum"] != pair["after"]["checksum"]:
                    raise RuntimeError(f"{scenario} 两版本 checksum 不同")
                print(f"{kind}/{runtime}/{scenario} {round_index + 1}/{args.rounds}: "
                      f'{pair["before"]["ns_per_op"]:.2f} → '
                      f'{pair["after"]["ns_per_op"]:.2f} ns/op', flush=True)


def distribution(values):
    quartiles = statistics.quantiles(values, n=4, method="inclusive")
    return {"median": statistics.median(values), "p25": quartiles[0], "p75": quartiles[2],
            "min": min(values), "max": max(values)}


def summarize(args):
    samples = [json.loads(line) for line in (args.output / "samples.jsonl").read_text().splitlines()]
    groups = {}
    for sample in samples:
        key = (sample["kind"], sample["runtime"], sample["scenario"])
        groups.setdefault(key, {}).setdefault(sample["variant"], {})[sample["round"]] = sample["ns_per_op"]
    summaries = []
    rng = random.Random(20261001)  # 仅统计重采样可复现；哈希器仍使用随机种子。
    for (kind, runtime, scenario), variants in groups.items():
        before, after = variants["before"], variants["after"]
        if before.keys() != after.keys() or len(before) < 2:
            raise RuntimeError(f"{scenario} 缺少完整配对样本")
        reductions = [100 * (1 - after[index] / before[index]) for index in before]
        boot = sorted(statistics.median(rng.choices(reductions, k=len(reductions))) for _ in range(10000))
        b, a = distribution(list(before.values())), distribution(list(after.values()))
        summaries.append({"kind": kind, "runtime": runtime, "scenario": scenario,
                          "samples_per_variant": len(before), "before_ns": b, "after_ns": a,
                          "median_latency_reduction_pct": 100 * (1 - a["median"] / b["median"]),
                          "paired_reduction_pct": statistics.median(reductions),
                          "paired_bootstrap_95pct": [boot[250], boot[9749]]})
    save(args.output / "summary.json", summaries)
    lines = ["| 场景 | std ns/op（中位数，P25–P75） | ahash ns/op（中位数，P25–P75） | 中位耗时下降 | 配对下降中位数 | 配对下降 95% bootstrap 区间 |",
             "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for row in summaries:
        b, a, ci = row["before_ns"], row["after_ns"], row["paired_bootstrap_95pct"]
        lines.append(f'| {row["kind"]}/{row["runtime"]}/{row["scenario"]} | '
                     f'{b["median"]:.2f} ({b["p25"]:.2f}–{b["p75"]:.2f}) | '
                     f'{a["median"]:.2f} ({a["p25"]:.2f}–{a["p75"]:.2f}) | '
                     f'{row["median_latency_reduction_pct"]:.2f}% | '
                     f'{row["paired_reduction_pct"]:.2f}% | {ci[0]:.2f}%–{ci[1]:.2f}% |')
    text = "\n".join(lines) + "\n"
    (args.output / "RESULTS.md").write_text(text, encoding="utf-8")
    print(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=["build", "measure", "summarize", "all"])
    parser.add_argument("--baseline-root", type=Path)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/cargo-nestrs")
    parser.add_argument("--output", type=Path, default=ROOT / "target/di-ahash/benchmark")
    parser.add_argument("--rounds", type=int, default=20)
    parser.add_argument("--hash-iterations", type=int, default=4_000_000)
    parser.add_argument("--di-iterations", type=int, default=100_000)
    parser.add_argument("--multi-iterations", type=int, default=20_000)
    parser.add_argument("--transient-iterations", type=int, default=4_096)
    parser.add_argument("--single-cpu", help="例如 2；Linux 可选 affinity")
    parser.add_argument("--multi-cpus", help="例如 2,4,6；Linux 可选 affinity")
    args = parser.parse_args()
    if args.rounds < 2:
        parser.error("至少需要两轮完整配对样本")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if args.phase in ["build", "all"]:
        build(args)
    if args.phase in ["measure", "all"]:
        measure(args)
    if args.phase in ["summarize", "all"]:
        summarize(args)


if __name__ == "__main__":
    main()

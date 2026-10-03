#!/usr/bin/env python3
"""Verify cross-crate automatic bindings through real check, run and graph commands."""

import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from toolchain_support import bridge_name, executable_name


BINARIES = {
    "transitive_visibility": "cross-crate visibility: inaccessible transitive capabilities skipped and reexported blanket trait preserved",
    "lazy_cross_crate": "cross-crate lazy: library field shares the upstream private async factory instance",
    "downstream_demand": "cross-crate downstream demand: public class and private factory share singleton projections",
    "sibling_selection": "cross-crate siblings: primary, exact key and present/absent optional injection passed",
    "transitive_reuse": "cross-crate transitive reuse: repeated automatic pairs share one logical route",
    "alias_identity": "cross-crate aliases: reexports, distinct crate identities and keyed projection reuse passed",
    "generic_capabilities": "cross-crate generics: private closed blueprint, nested private trait, associated type and supertrait passed",
    "higher_ranked_capabilities": "cross-crate higher-ranked traits: private provider, shared identity and bound associated types passed",
}


def source_hashes(directory):
    paths = list(directory.rglob("*.rs")) + list(directory.rglob("Cargo.toml")) + list(directory.rglob("Cargo.lock"))
    return {
        str(path.relative_to(directory)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(paths)
    }


class GraphData(HTMLParser):
    def __init__(self):
        super().__init__()
        self.collecting = False
        self.count = 0
        self.fragments = []

    def handle_starttag(self, tag, attrs):
        if tag == "script" and dict(attrs).get("id") == "graph-data":
            self.collecting = True
            self.count += 1

    def handle_endtag(self, tag):
        if tag == "script":
            self.collecting = False

    def handle_data(self, data):
        if self.collecting:
            self.fragments.append(data)


def verify_graph(path, binary):
    parser = GraphData()
    parser.feed(path.read_text(encoding="utf-8"))
    assert parser.count == 1, f"{binary}: expected one graph-data payload"
    graph = json.loads("".join(parser.fragments))
    assert graph["version"] == 1, f"{binary}: expected selected-binary graph"
    nodes = graph["nodes"]
    assert not any("DormantRepository" in node["name"] for node in nodes), "unused generic was materialized"
    repositories = [node for node in nodes if "::implementation::Repository<" in node["name"]]
    if binary == "generic_capabilities":
        assert len(repositories) == 1, f"expected one private closed generic provider: {repositories}"
    else:
        assert not repositories, "unrequested private closed generic was materialized"

    def select(suffix, key=None):
        found = [node for node in nodes if node["name"].endswith(suffix) and node["key"] == key]
        assert len(found) == 1, f"{binary}: expected exactly one {suffix} with key {key}: {found}"
        return found[0]

    if binary == "transitive_visibility":
        # 本入口只链接 contracts 和自己的服务；不为了复用其它入口断言引入无关 provider。
        service = select("transitive_visibility::Service")
        assert len(nodes) == 1 and not service["dependencies"]
        return {"nodes": len(nodes), "dormant_generic_absent": True}

    primary = select("nestrs_cross_primary_provider::implementation::Service")
    fallback = select("nestrs_cross_fallback_provider::implementation::Service")
    inventory = select("nestrs_cross_primary_provider::implementation::Inventory")
    assert primary["primary"] and not fallback["primary"]
    assert primary["id"] != fallback["id"], "same-named crate types were merged"
    audit = select(
        "nestrs_cross_primary_provider::implementation::Service",
        {"kind": "named", "value": "audit"},
    )
    assert primary["id"] != audit["id"], "keyed providers were merged"

    def inputs(node):
        return {dependency["label"]: dependency for dependency in node["dependencies"]}

    if binary == "lazy_cross_crate":
        consumer = select("nestrs_cross_upstream_consumer::LazyConnectionConsumer")
        connection = select("nestrs_cross_primary_provider::implementation::PrivateConnection")
        dependency = inputs(consumer)["connection"]
        assert dependency["lazy"] is True, "library metadata lost the deferred input edge"
        assert dependency["target"] == connection["id"]
        assert connection["kind"] == "async factory"

    if binary in {"downstream_demand", "transitive_reuse"}:
        checkout = select("nestrs_cross_upstream_consumer::Checkout")
        dependencies = inputs(checkout)
        connection = select("nestrs_cross_primary_provider::implementation::PrivateConnection")
        tracker = select("nestrs_cross_fallback_provider::implementation::Tracking")
        assert connection["kind"] == "async factory"
        assert dependencies["catalog"]["target"] == inventory["id"]
        assert dependencies["connection"]["target"] == connection["id"]
        assert dependencies["delivery"]["target"] == primary["id"]
        assert dependencies["tracking"]["target"] == tracker["id"], "upstream consumer lost sibling provider"
        assert dependencies["fraud"]["optional"] and dependencies["fraud"]["target"] is None
    if binary in {"sibling_selection", "transitive_reuse"}:
        dispatch = select("nestrs_cross_sibling_consumer::Dispatch")
        dependencies = inputs(dispatch)
        assert dependencies["catalog"]["target"] == inventory["id"]
        assert dependencies["preferred"]["target"] == primary["id"]
        assert dependencies["audit"]["target"] == audit["id"]
        assert dependencies["optional_preferred"]["optional"]
        assert dependencies["optional_preferred"]["target"] == primary["id"]
        assert dependencies["fraud"]["optional"] and dependencies["fraud"]["target"] is None
    if binary == "generic_capabilities":
        repository = repositories[0]
        connection = select("nestrs_cross_primary_provider::implementation::RepositoryConnection")
        dependencies = inputs(repository)
        assert dependencies["connection"]["target"] == connection["id"]
        assert "PrivateRepositoryDependency" in dependencies["connection"]["requested"]
    if binary == "higher_ranked_capabilities":
        service = select("nestrs_cross_primary_provider::implementation::PrivateHrtbService")
        dependencies = inputs(select("higher_ranked_capabilities::Reader"))
        for label in ["text", "count", "borrowed"]:
            assert dependencies[label]["target"] == service["id"]
        assert dependencies["incompatible"]["optional"]
        assert dependencies["incompatible"]["target"] is None
    return {"nodes": len(nodes), "dormant_generic_absent": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true", help="use the already built debug toolchain")
    parser.add_argument("--cli", type=Path, help="CLI path, defaults to Cargo target/debug/cargo-nestrs")
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    fixture = root / "cargo-nestrs/tests/fixtures/cross-crate-binding"
    output = root / "target/nestrs-cross-crate-binding"
    output.mkdir(parents=True, exist_ok=True)
    report_path = output / "report.json"
    report_path.write_text(
        json.dumps({"passed": False, "cases": [], "state": "initializing"}) + "\n",
        encoding="utf-8",
    )
    environment = os.environ.copy()
    for name in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "NESTRS_GRAPH_TARGET"]:
        environment.pop(name, None)
    if not arguments.skip_build:
        subprocess.run([sys.executable, str(root / "tools/build-toolchain.py")], cwd=root, env=environment, check=True)
    if arguments.cli is None:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=root, env=environment, text=True, encoding="utf-8",
        ))
        source_cli = Path(metadata["target_directory"]) / "debug" / executable_name("cargo-nestrs")
    else:
        source_cli = arguments.cli.resolve()
    source_driver = Path(environment.get("NESTRS_DRIVER", str(source_cli.with_name(executable_name("nestrs-driver")))))
    source_bridge = Path(environment.get("NESTRS_MACRO_BRIDGE", str(source_cli.with_name(bridge_name()))))
    toolchain = output / "toolchain"
    toolchain.mkdir(exist_ok=True)
    cli = toolchain / executable_name("cargo-nestrs")
    driver = toolchain / executable_name("nestrs-driver")
    bridge = toolchain / bridge_name()
    for source, destination in [(source_cli, cli), (source_driver, driver), (source_bridge, bridge)]:
        shutil.copy2(source, destination)
    environment["NESTRS_DRIVER"] = str(driver)
    environment["NESTRS_MACRO_BRIDGE"] = str(bridge)
    environment["CARGO_TARGET_DIR"] = str(output / "cargo")
    ambiguity_sentinel = output / "ambiguous-main-executed.txt"
    ambiguity_sentinel.unlink(missing_ok=True)
    environment["NESTRS_CROSS_AMBIGUITY_SENTINEL"] = str(ambiguity_sentinel)
    before = source_hashes(fixture)
    report = {
        "passed": False,
        "cases": [],
        "source_hashes": before,
        "toolchain_hashes": {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in [cli, driver, bridge]},
    }
    common = ["--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", "nestrs-cross-crate-binding"]

    def run(name, command, expected=None, success=True, diagnostic=None):
        result = subprocess.run(
            [str(cli), *command], cwd=root, env=environment, text=True,
            encoding="utf-8", capture_output=True, timeout=600,
        )
        (output / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
        error_log = output / f"{name}.stderr.txt"
        error_log.write_text(result.stderr, encoding="utf-8")
        assert (result.returncode == 0) == success, f"{name}: exit {result.returncode}; inspect {error_log}"
        if expected is not None:
            assert expected in result.stdout, f"{name}: runtime assertions did not finish"
        if diagnostic is not None:
            details = [diagnostic] if isinstance(diagnostic, str) else diagnostic
            for detail in details:
                assert detail in result.stderr, f"{name}: missing diagnostic {detail!r}; inspect {error_log}"
        assert "internal compiler error" not in result.stderr, f"{name}: compiler crashed; inspect {error_log}"
        assert not ambiguity_sentinel.exists(), f"{name}: invalid binary's business main executed"
        case = {"name": name, "passed": True, "command": command, "exit_code": result.returncode}
        report["cases"].append(case)
        print(f"PASS {name}", flush=True)
        return case

    try:
        # library 只贡献能力；需要兄弟 crate 补齐的依赖留给最终 binary 验证。
        run("check-libraries", ["check", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "--workspace", "--lib"])
        valid_targets = [argument for binary in BINARIES for argument in ["--bin", binary]]
        run("check-valid-targets", ["check", *common, *valid_targets])
        ambiguity = ["AmbiguousTrait", "AmbiguousPort", "conflict", "primary-provider", "fallback-provider"]
        run("check-all-targets-rejects-ambiguity", ["check", *common, "--all-targets"], success=False, diagnostic=ambiguity)
        for profile in ["debug", "release"]:
            for binary, expected in BINARIES.items():
                command = ["run", *common, "--bin", binary]
                if profile == "release":
                    command.append("--release")
                run(f"run-{profile}-{binary}", command, expected)
            # 不再运行一个捕获 build panic 后 exit(23) 的程序；两种构建入口都必须
            # 在编译时拒绝该未被查询的非法服务，且完整保留候选和源码诊断。
            for operation in ["check", "build"]:
                negative_command = [operation, *common, "--bin", "ambiguous_candidates"]
                if profile == "release":
                    negative_command.append("--release")
                run(
                    f"{operation}-{profile}-ambiguous_candidates", negative_command,
                    success=False, diagnostic=ambiguity,
                )
        for binary in BINARIES:
            graph = output / f"{binary}.html"
            case = run(f"graph-{binary}", ["graph", *common, "--bin", binary, "--output", str(graph)])
            try:
                case["graph"] = verify_graph(graph, binary)
            except (AssertionError, OSError, ValueError, KeyError):
                case["passed"] = False
                raise
        preserved = output / "ambiguous-preserved.html"
        preserved.write_text("existing graph must survive a failed export\n", encoding="utf-8")
        run(
            "graph-ambiguous_candidates",
            ["graph", *common, "--bin", "ambiguous_candidates", "--output", str(preserved)],
            success=False, diagnostic=ambiguity,
        )
        assert preserved.read_text(encoding="utf-8") == "existing graph must survive a failed export\n"
        assert source_hashes(fixture) == before, "toolchain changed application sources or manifests"
        report["sources_unchanged"] = True
        report["invalid_main_not_executed"] = not ambiguity_sentinel.exists()
        report["passed"] = True
    except (AssertionError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        report["failure"] = str(error)
        raise
    finally:
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Verified {len(report['cases'])} check/run/graph cases; report: {output / 'report.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

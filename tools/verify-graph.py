#!/usr/bin/env python3
"""Exercise the real graph CLI, side-effect boundaries and target cache isolation."""

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


class GraphData(HTMLParser):
    def __init__(self):
        super().__init__()
        self.collecting = False
        self.data = []
        self.matches = 0

    def handle_starttag(self, tag, attrs):
        if tag == "script" and dict(attrs).get("id") == "graph-data":
            self.collecting = True
            self.matches += 1

    def handle_endtag(self, tag):
        if tag == "script":
            self.collecting = False

    def handle_data(self, data):
        if self.collecting:
            self.data.append(data)


def graph_data(path):
    html = path.read_text(encoding="utf-8")
    parser = GraphData()
    parser.feed(html)
    assert parser.matches == 1, f"{path}: expected one graph-data script"
    assert "<script src=" not in html, f"{path}: graph must work offline"
    return json.loads("".join(parser.data))


def verify_graph(path, binary):
    graph = graph_data(path)
    return verify_graph_payload(graph, binary)


def verify_graph_payload(graph, binary):
    assert graph["version"] == 1, graph
    nodes = graph["nodes"]
    assert len(nodes) == 5, f"unexpected provider count: {nodes}"

    def node(suffix):
        found = [item for item in nodes if item["name"].endswith(suffix)]
        assert len(found) == 1, f"expected one {suffix}: {nodes}"
        return found[0]

    name = binary.capitalize()
    database = node("::shared::Database")
    client = node("::shared::Client")
    consumer = node("::shared::Consumer")
    cache = node(f"::shared::Cache<{binary}::{name}>")
    node(f"::{name}Only")
    assert all(item["name"].startswith(binary + "::") for item in nodes), nodes
    assert client["kind"] == "sync factory", client
    assert client["key"] == {"kind": "named", "value": "active"}, client
    assert consumer["lifetime"] == "Scoped" and consumer["requiresScope"], consumer
    inputs = consumer["dependencies"]
    assert [item["slot"] for item in inputs] == [1, 2, 3], inputs
    assert [item["label"] for item in inputs] == ["database", "port", "optional"], inputs
    assert inputs[0]["target"] == database["id"], inputs
    assert inputs[1]["target"] == client["id"], inputs
    assert inputs[1]["requested"] == f"dyn {binary}::shared::Port", inputs
    assert inputs[1]["key"] == client["key"], inputs
    assert inputs[2]["optional"] and inputs[2]["target"] is None, inputs
    assert inputs[2]["requested"] == f"dyn {binary}::shared::Missing", inputs
    assert len(cache["dependencies"]) == 1, cache
    assert cache["dependencies"][0]["target"] == database["id"], cache
    return graph


def project_data(path):
    project = graph_data(path)
    assert project["version"] == 2 and project["kind"] == "project", project
    entries = project["entries"]
    assert len({entry["id"] for entry in entries}) == len(entries), entries
    packages = {package["id"]: package for package in project["packages"]}
    for entry in entries:
        assert entry["packageId"] in packages, entry
        assert entry["package"] == packages[entry["packageId"]]["name"], entry
        assert entry["status"] in ["ok", "error", "skipped"], entry
        if entry["status"] == "ok":
            assert entry["graph"]["version"] == 1 and entry["diagnostic"] is None, entry
        else:
            assert entry["graph"] is None and entry["diagnostic"], entry
    return project


def fixture_entries(project):
    entries = {entry["binary"]: entry for entry in project["entries"] if entry["package"] == "nestrs-graph-fixture"}
    assert set(entries) == {"alpha", "beta", "build-script-build", "invalid_graph", "no_main", "cfg_no_main", "feature_app", "compile_error"}, entries
    for binary in ["alpha", "beta"]:
        assert entries[binary]["status"] == "ok", entries[binary]
        verify_graph_payload(entries[binary]["graph"], binary)
    build_named = entries["build-script-build"]
    assert build_named["status"] == "ok", build_named
    assert len(build_named["graph"]["nodes"]) == 1, build_named
    assert build_named["graph"]["nodes"][0]["name"].endswith("::BuildScriptNamedService"), build_named
    assert entries["invalid_graph"]["status"] == "error", entries
    assert "graph validation failed" in entries["invalid_graph"]["diagnostic"], entries
    for binary in ["no_main", "cfg_no_main"]:
        assert entries[binary]["status"] == "error", entries[binary]
        assert "no_main" in entries[binary]["diagnostic"], entries[binary]
    return entries


def source_hashes(fixture):
    paths = list(fixture.rglob("*.rs")) + list(fixture.rglob("Cargo.toml")) + list(fixture.rglob("Cargo.lock"))
    return {
        str(path.relative_to(fixture)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(paths)
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true", help="use the already built CLI and driver")
    parser.add_argument("--cli", type=Path, help="CLI path; defaults to Cargo target/debug/cargo-nestrs")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    fixture = root / "cargo-nestrs/tests/fixtures/graph"
    output = root / "target/nestrs-graph-verification"
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    for key in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "NESTRS_GRAPH_TARGET"]:
        environment.pop(key, None)
    if not args.skip_build:
        subprocess.run([sys.executable, str(root / "tools/build-toolchain.py")], cwd=root, env=environment, check=True)
    if args.cli is None:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=root, env=environment, text=True, encoding="utf-8",
        ))
        source_cli = Path(metadata["target_directory"]) / "debug" / executable_name("cargo-nestrs")
    else:
        source_cli = args.cli.resolve()
    source_driver = Path(environment.get("NESTRS_DRIVER", str(source_cli.with_name(executable_name("nestrs-driver")))))
    source_bridge = Path(environment.get("NESTRS_MACRO_BRIDGE", str(source_cli.with_name(bridge_name()))))
    # Each invocation tests one immutable pair even if another terminal rebuilds
    # the workspace toolchain while these integration commands are running.
    binaries = output / "toolchain"
    binaries.mkdir(exist_ok=True)
    cli = binaries / executable_name("cargo-nestrs")
    driver = binaries / executable_name("nestrs-driver")
    bridge = binaries / bridge_name()
    shutil.copy2(source_cli, cli)
    shutil.copy2(source_driver, driver)
    shutil.copy2(source_bridge, bridge)
    environment["NESTRS_DRIVER"] = str(driver)
    environment["NESTRS_MACRO_BRIDGE"] = str(bridge)
    target = output / "cargo"
    environment["CARGO_TARGET_DIR"] = str(target)
    sentinel = output / "unexpected-execution.txt"
    environment["NESTRS_GRAPH_SENTINEL"] = str(sentinel)
    if sentinel.exists():
        sentinel.unlink()
    before = source_hashes(fixture)
    report = {
        "passed": False,
        "cases": [],
        "source_hashes": before,
        "toolchain_hashes": {
            path.name: hashlib.sha256(path.read_bytes()).hexdigest()
            for path in [cli, driver, bridge]
        },
    }
    common = ["--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", "nestrs-graph-fixture"]

    def run(name, command, success=True, diagnostic=None, allow_sentinel=False, cwd=root):
        result = subprocess.run([str(cli), *command], cwd=cwd, env=environment, text=True, capture_output=True, timeout=600, encoding="utf-8")
        (output / f"{name}.stdout.txt").write_text(result.stdout, encoding="utf-8")
        error_log = output / f"{name}.stderr.txt"
        error_log.write_text(result.stderr, encoding="utf-8")
        assert (result.returncode == 0) == success, f"{name}: unexpected exit {result.returncode}; inspect {error_log}"
        if diagnostic is not None:
            assert diagnostic in result.stderr, f"{name}: missing {diagnostic!r}; inspect {error_log}"
        if not allow_sentinel:
            assert not sentinel.exists(), f"{name}: graph executed {sentinel.read_text(encoding="utf-8")}"
        report["cases"].append({"name": name, "exit_code": result.returncode, "passed": True})
        print(f"PASS {name}", flush=True)
        return result

    def render(name, binary, path):
        run(name, ["graph", *common, "--bin", binary, "--output", str(path)])
        return verify_graph(path, binary)

    try:
        first = render("alpha-first", "alpha", output / "alpha-first.html")
        second = render("beta", "beta", output / "beta.html")
        again = render("alpha-cached", "alpha", output / "alpha-cached.html")
        assert first == again and first != second, "binary graph cache was reused across targets"
        # A real ordinary execution is the positive control for the main trap.
        run("ordinary-main-control", ["run", *common, "--bin", "alpha"], success=False, allow_sentinel=True)
        assert sentinel.read_text(encoding="utf-8") == "business main alpha", "ordinary main did not execute the sentinel control"
        sentinel.unlink()
        after_run = render("alpha-after-normal-run", "alpha", output / "alpha-after-normal-run.html")
        assert first == after_run, "normal application compilation contaminated graph artifacts"

        run("default-output", ["graph", *common, "--bin", "alpha"])
        assert verify_graph(target / "nestrs-di.html", "alpha") == first
        run("relative-output", ["graph", *common, "--bin", "beta", "--output", "relative.html"], cwd=output)
        assert verify_graph(output / "relative.html", "beta") == second

        for name, package, binary, diagnostic in [
            ("invalid-graph", "nestrs-graph-fixture", "invalid_graph", "graph validation failed"),
            ("missing-core", "nestrs-graph-without-core", "nestrs-graph-without-core", "depend directly on nestrs-core"),
            ("no-main", "nestrs-graph-fixture", "no_main", "no_main"),
            ("cfg-no-main", "nestrs-graph-fixture", "cfg_no_main", "no_main"),
            ("missing-feature", "nestrs-graph-fixture", "feature_app", "extras"),
        ]:
            destination = output / f"{name}.html"
            previous = b"previous valid graph output\n"
            destination.write_bytes(previous)
            run(name, ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", package, "--bin", binary, "--output", str(destination)], success=False, diagnostic=diagnostic)
            assert destination.read_bytes() == previous, f"{name}: failed graph changed previous output"

        # Package selection exports every binary, even though default-run is alpha.
        # Failed or unsupported entries remain diagnostics alongside successful graphs.
        package_output = output / "package.html"
        package_output.write_text("old project report", encoding="utf-8")
        run("package-partial", ["graph", *common, "--color=always", "--output", str(package_output)], success=False)
        package = project_data(package_output)
        entries = fixture_entries(package)
        assert entries["alpha"]["graph"] == first and entries["beta"]["graph"] == second
        for binary, feature in [("feature_app", "extras"), ("compile_error", "broken")]:
            assert entries[binary]["status"] == "skipped", entries[binary]
            assert entries[binary]["requiredFeatures"] == [feature], entries[binary]
            assert feature in entries[binary]["diagnostic"], entries[binary]

        workspace_output = output / "workspace.html"
        run("workspace-partial", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "--workspace", "--output", str(workspace_output)], success=False)
        workspace = project_data(workspace_output)
        fixture_entries(workspace)
        assert {item["name"] for item in workspace["packages"]} == {
            "nestrs-graph-fixture", "nestrs-graph-without-core", "nestrs-graph-other-app", "nestrs-graph-library-only",
        }, workspace
        assert not any(entry["package"] == "nestrs-graph-library-only" for entry in workspace["entries"]), workspace
        same_names = [entry for entry in workspace["entries"] if entry["binary"] == "alpha"]
        assert len(same_names) == 2 and all(entry["status"] == "ok" for entry in same_names), same_names
        other = next(entry for entry in same_names if entry["package"] == "nestrs-graph-other-app")
        assert len(other["graph"]["nodes"]) == 1 and other["graph"]["nodes"][0]["name"].endswith("::OtherPackageOnly"), other
        missing_core = next(entry for entry in workspace["entries"] if entry["package"] == "nestrs-graph-without-core")
        assert missing_core["status"] == "error" and "depend directly on nestrs-core" in missing_core["diagnostic"], missing_core

        workspace_feature_output = output / "workspace-feature-rejected.html"
        workspace_feature_output.write_bytes(b"preserve report on unsupported workspace feature selection")
        run("workspace-feature-rejected", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "--workspace", "--features", "nestrs-graph-fixture/feature-alias", "--output", str(workspace_feature_output)], success=False, diagnostic="--features")
        assert workspace_feature_output.read_bytes() == b"preserve report on unsupported workspace feature selection"

        default_feature_output = output / "default-members-feature-rejected.html"
        default_feature_output.write_bytes(b"preserve report on unsupported default-members feature selection")
        run("default-members-feature-rejected", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "--features", "feature-alias", "--output", str(default_feature_output)], success=False, diagnostic="--features")
        assert default_feature_output.read_bytes() == b"preserve report on unsupported default-members feature selection"

        all_alias_output = output / "all-alias-rejected.html"
        all_alias_output.write_bytes(b"preserve report on rejected all selector")
        run("all-alias-rejected", ["graph", *common, "--all", "--bin", "alpha", "--output", str(all_alias_output)], success=False, diagnostic="--workspace")
        assert all_alias_output.read_bytes() == b"preserve report on rejected all selector"

        repeat_output = output / "package-repeat.html"
        run("package-cached", ["graph", *common, "--output", str(repeat_output)], success=False)
        assert project_data(repeat_output) == package, "project report ordering or per-binary caches are unstable"
        assert render("alpha-after-project", "alpha", output / "alpha-after-project.html") == first

        feature_output = output / "package-feature.html"
        run("package-feature-alias", ["graph", *common, "--features", "feature-alias", "--output", str(feature_output)], success=False)
        feature_entries = fixture_entries(project_data(feature_output))
        assert feature_entries["feature_app"]["status"] == "ok", feature_entries["feature_app"]
        assert feature_entries["feature_app"]["graph"]["nodes"][0]["name"].endswith("::FeatureOnly"), feature_entries["feature_app"]
        assert feature_entries["compile_error"]["status"] == "skipped", feature_entries["compile_error"]

        all_features_output = output / "package-all-features.html"
        run("package-build-failure", ["graph", *common, "--all-features", "--output", str(all_features_output)], success=False)
        all_features_entries = fixture_entries(project_data(all_features_output))
        assert all_features_entries["feature_app"]["status"] == "ok", all_features_entries["feature_app"]
        assert all_features_entries["compile_error"]["status"] == "error", all_features_entries["compile_error"]
        assert "intentional project graph compile failure" in all_features_entries["compile_error"]["diagnostic"], all_features_entries["compile_error"]

        all_failed_output = output / "all-failed.html"
        run("package-all-failed", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", "nestrs-graph-without-core", "--output", str(all_failed_output)], success=False)
        all_failed = project_data(all_failed_output)
        assert len(all_failed["entries"]) == 1 and all_failed["entries"][0]["status"] == "error", all_failed

        success_output = output / "package-success.html"
        run("package-success", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", "nestrs-graph-other-app", "--output", str(success_output)])
        assert project_data(success_output)["entries"][0]["graph"] == other["graph"]

        library_output = output / "library-only.html"
        library_output.write_bytes(b"preserve output without an executable target")
        run("package-no-binary", ["graph", "--locked", "--manifest-path", str(fixture / "Cargo.toml"), "-p", "nestrs-graph-library-only", "--output", str(library_output)], success=False)
        assert library_output.read_bytes() == b"preserve output without an executable target"
        blocked = output / "not-a-directory"
        blocked.write_text("a regular file blocks the export path\n", encoding="utf-8")
        run("export-error", ["graph", *common, "--bin", "alpha", "--output", str(blocked / "graph.html")], success=False)
        assert blocked.read_text(encoding="utf-8") == "a regular file blocks the export path\n"
        assert source_hashes(fixture) == before, "graph compilation modified original fixture sources or manifests"
        report["sources_unchanged"] = True
        report["passed"] = True
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Verified {len(report['cases'])} commands; report: {output / 'report.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

#!/usr/bin/env python3
"""Historical low-level probe for the installed rust-analyzer macro server.

This verifies macro loading and rewritten declarations, not the full editor UI,
completion, or automatic trait-binding diagnostics. No component is installed.
The small token codec implements the server's legacy JSON protocol versions 5/6
in ID span mode. It is used only for these fixed test inputs, never application
source preprocessing.

This legacy entry currently removes RUSTC_BOOTSTRAP and builds the private bridge
directly. The bridge now requires proc_macro_def_site, so that build lacks its
crate-scoped authorization and fails with E0554 on the pinned stable compiler;
the same feature gate was reproduced with the script's captured environment.
Use verify-ide.py for the maintained standard-LSP verification entry. This known
probe setup defect is not evidence of a product macro or LSP failure.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys

from toolchain_support import compiler_command_environment, compiler_library_environment, macro_server


def flat_tree(source):
    """Encode the limited identifier/punctuation syntax of our fixed probes."""
    tree = {name: [] for name in ["subtree", "literal", "punct", "ident", "token_tree", "text"]}
    words = re.findall(r"[A-Za-z_][A-Za-z_0-9]*|[^\s]", source)
    root = (0, [])
    stack = [root]
    pairs = {"(": (1, ")"), "{": (2, "}"), "[": (3, "]")}
    closing = {1: ")", 2: "}", 3: "]"}
    for index, word in enumerate(words):
        if word in pairs:
            group = (pairs[word][0], [])
            stack[-1][1].append(group)
            stack.append(group)
        elif word in closing.values():
            assert len(stack) > 1 and closing[stack[-1][0]] == word
            stack.pop()
        else:
            # Rust compound punctuation must preserve joint spacing (->, ::).
            joint = index + 1 < len(words) and word + words[index + 1] in {"->", "::"}
            stack[-1][1].append((word, joint))
    assert len(stack) == 1
    queue = [root]
    for kind, children in queue:
        first = len(tree["token_tree"])
        tree["subtree"].extend([0, 0, kind, first, first + len(children)])
        for value, detail in children:
            if isinstance(value, int):
                child_index = len(queue)
                queue.append((value, detail))
                tree["token_tree"].append(child_index << 2)
            elif value.isidentifier():
                child_index = len(tree["ident"]) // 3
                text_index = len(tree["text"])
                tree["text"].append(value)
                tree["ident"].extend([0, text_index, 0])
                tree["token_tree"].append(child_index << 2 | 3)
            else:
                assert len(value) == 1 and not value.isalnum(), value
                child_index = len(tree["punct"]) // 3
                tree["punct"].extend([0, ord(value), int(detail)])
                tree["token_tree"].append(child_index << 2 | 2)
    return tree


def tokens(tree, subtree=0):
    """Decode ordered tokens, including group delimiters and quoted literals."""
    _, _, kind, first, last = tree["subtree"][5 * subtree:5 * subtree + 5]
    delimiters = ["", "()", "{}", "[]"][kind]
    if delimiters:
        yield delimiters[0]
    for encoded in tree["token_tree"][first:last]:
        index, tag = encoded >> 2, encoded & 3
        if tag == 0:
            yield from tokens(tree, index)
        elif tag == 3:
            _, text, raw = tree["ident"][3 * index:3 * index + 3]
            yield ("r#" if raw else "") + tree["text"][text]
        elif tag == 2:
            yield chr(tree["punct"][3 * index + 1])
        else:
            _, text, kind, suffix = tree["literal"][4 * index:4 * index + 4]
            value = tree["text"][text]
            literal_kind, hashes = kind & 255, kind >> 8
            if literal_kind in [1, 2]:
                value = ("b" if literal_kind == 1 else "") + "'" + value + "'"
            elif literal_kind in [5, 7, 9]:
                value = {5: "", 7: "b", 9: "c"}[literal_kind] + '"' + value + '"'
            elif literal_kind in [6, 8, 10]:
                marker = "#" * hashes
                value = {6: "r", 8: "br", 10: "cr"}[literal_kind] + marker + '"' + value + '"' + marker
            if suffix != 0xFFFFFFFF:
                value += tree["text"][suffix]
            yield value
    if delimiters:
        yield delimiters[1]


def main():
    """Run the legacy macro-server probe and retain its protocol and expansion transcripts."""
    root = Path(__file__).resolve().parent.parent
    output = root / "target/nestrs-tool-bridge-probe"
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    for name in ["RUSTC_BOOTSTRAP", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTDOC"]:
        environment.pop(name, None)
    rustc = environment.get("NESTRS_RUSTC", "rustc")
    compiler_command_environment(environment, rustc)
    environment["RUSTC"] = rustc
    sysroot = Path(subprocess.check_output([rustc, "--print", "sysroot"], cwd=root, env=environment, text=True, encoding="utf-8").strip())
    version = subprocess.check_output([rustc, "-vV"], cwd=root, env=environment, text=True, encoding="utf-8")
    host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
    compiler_library_environment(environment, sysroot, host)
    server = macro_server(sysroot)
    assert server.is_file(), f"macro server is not installed in the active toolchain: {server}"
    # Known probe defect: this direct build has no nestrs_tool_bridge bootstrap
    # grant. The maintained build-toolchain.py scopes that grant to tool builds;
    # updating this legacy probe also requires checking its protocol assertions.
    build = subprocess.run(
        ["cargo", "build", "-p", "nestrs-tool-bridge", "--message-format=json"],
        cwd=root, env=environment, text=True, capture_output=True, encoding="utf-8",
    )
    (output / "build.stderr.txt").write_text(build.stderr, encoding="utf-8")
    assert build.returncode == 0, f"macro build failed; inspect {output / 'build.stderr.txt'}"
    libraries = []
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == "nestrs_tool_bridge":
            libraries.extend(Path(name) for name in message["filenames"] if Path(name).suffix in {".so", ".dylib", ".dll"})
    assert len(libraries) == 1, f"expected one proc-macro library, got {libraries}"
    library = libraries[0]
    environment["RUST_ANALYZER_INTERNALS_DO_NOT_USE"] = "1"
    report = {
        "passed": False,
        "scope": "macro server loading and declaration expansion; no full editor UI or completion validation",
        "server": str(server),
        "macro_dylib": str(library),
        "cases": [],
    }

    def exchange(name, requests):
        payload = "".join(json.dumps(request) + "\n" for request in requests)
        (output / f"{name}.requests.jsonl").write_text(payload, encoding="utf-8")
        result = subprocess.run(
            [str(server)], input=payload, cwd=root, env=environment,
            text=True, capture_output=True, timeout=30, encoding="utf-8",
        )
        (output / f"{name}.responses.jsonl").write_text(result.stdout, encoding="utf-8")
        (output / f"{name}.stderr.txt").write_text(result.stderr, encoding="utf-8")
        assert result.returncode == 0, f"macro server failed; inspect {output / (name + '.stderr.txt')}"
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        assert len(responses) == len(requests), responses
        return responses

    try:
        responses = exchange("load", [
            {"ApiVersionCheck": {}},
            {"ListMacros": {"dylib_path": str(library)}},
        ])
        version = responses[0]["ApiVersionCheck"]
        assert version in [5, 6], f"unsupported macro server protocol {version}; update the test codec"
        report["protocol_version"] = version
        macros = responses[1]["ListMacros"].get("Ok")
        assert macros is not None, responses[1]
        assert {"injectable", "factory", "primary"}.issubset({name for name, kind in macros if kind == "Attr"})
        report["macros"] = macros
        cases = [
            (
                "injectable",
                "struct Consumer { #[inject] port: dyn Port, #[inject] optional: Option<dyn Port> }",
                [
                    "structConsumer{port:::nestrs_core::Injection<dynPort>",
                    "optional:::core::option::Option<::nestrs_core::Injection<dynPort>>",
                    "compiler_dependency::<dynPort,0usize>",
                    "compiler_dependency::<dynPort,1usize>",
                ],
            ),
            (
                "factory",
                "async fn create(#[inject] port: dyn Port) -> Product { Product }",
                [
                    "asyncfncreate<'__nestrs_factory_frame>(port:&'__nestrs_factory_framedynPort)->Product",
                    "FactoryInputs<'frame>",
                    "FactoryFuture<'frame>",
                    "FactoryInvoker::Async(__nestrs_factory_construct)",
                ],
            ),
        ]
        for name, source, expected in cases:
            request = {"ExpandMacro": {
                "lib": str(library), "env": [], "current_dir": str(root),
                "macro_body": flat_tree(source), "macro_name": name,
                "attributes": flat_tree(""),
            }}
            response = exchange(name, [request])[0]
            expanded = response["ExpandMacro"].get("Ok")
            assert expanded is not None, response
            ordered = list(tokens(expanded))
            (output / f"{name}.expanded.txt").write_text(" ".join(ordered) + "\n", encoding="utf-8")
            compact = "".join(ordered)
            assert "compile_error!" not in compact, compact
            for fragment in expected:
                assert fragment in compact, f"{name} expansion missing {fragment}; inspect expanded tokens"
            assert "#[inject]" not in compact, f"{name} left helper markers unconsumed"
            report["cases"].append({"macro": name, "passed": True, "expected_fragments": expected})
            print(f"PASS macro server expansion: {name}", flush=True)
        report["passed"] = True
    finally:
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Verified macro loading and 2 declaration expansions; report: {output / 'report.json'}")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, subprocess.SubprocessError, KeyError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)

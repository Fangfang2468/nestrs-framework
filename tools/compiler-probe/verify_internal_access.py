#!/usr/bin/env python3
"""Exercise private runtime access, source auditing and metadata export filtering.

The harness imports the actual production query module. Generated fixture files
and compiler outputs live only under target; no global toolchain is changed.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from toolchain_support import (  # noqa: E402
    bridge_name, compiler_command_environment, compiler_library_environment,
    executable_name, validate_compiler,
)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rustc", default="rustc")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    root = directory.parents[1]
    output = root / "target" / "nestrs-internal-access"
    output.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    for name in ["RUSTC_BOOTSTRAP", "PROBE_TRUSTED_FILE", "PROBE_EXPECT_TRUSTED", "PROBE_BRIDGE_PATH"]:
        environment.pop(name, None)
    compiler_command_environment(environment, args.rustc)
    report = {"passed": False, "cases": []}

    def command(name, arguments, *, env=None, succeeds=True, diagnostic=None):
        completed = subprocess.run(
            [str(value) for value in arguments], cwd=root,
            env=env or environment, text=True, capture_output=True, check=False,
        )
        (output / f"{name}.log").write_text(completed.stdout + completed.stderr, encoding="utf-8")
        assert (completed.returncode == 0) == succeeds, f"{name}: inspect {output / (name + '.log')}"
        if diagnostic:
            assert diagnostic in completed.stderr, f"{name}: missing diagnostic {diagnostic}"
        report["cases"].append({"name": name, "passed": True, "exit_code": completed.returncode})
        return completed

    def source(name, contents):
        path = output / f"{name}.rs"
        path.write_text(contents, encoding="utf-8")
        return path

    try:
        version = command("version", [args.rustc, "-vV"])
        identity = dict(line.split(": ", 1) for line in version.stdout.splitlines() if ": " in line)
        validate_compiler(json.loads((root / "cargo-nestrs/toolchain.json").read_text()), identity)
        sysroot = Path(command("sysroot", [args.rustc, "--print", "sysroot"]).stdout.strip())
        compiler_library_environment(environment, sysroot, identity["host"])
        driver = output / executable_name("internal-access-probe")
        build_env = dict(environment, RUSTC_BOOTSTRAP="nestrs_internal_access_probe")
        command("driver", [args.rustc, directory / "internal_access_driver.rs",
                           "--crate-name", "nestrs_internal_access_probe", "--edition", "2024",
                           "-Dwarnings", "-L", f"native={sysroot / 'lib'}", "-o", driver], env=build_env)
        core = source("runtime", '''#![allow(dead_code)]
mod activation {
    pub struct Hidden(pub usize);
    impl Hidden { pub fn number(&self) -> usize { self.0 } }
    pub fn make() -> Hidden { Hidden(17) }
}
mod facade {
    // 诊断路径可能选用此别名；认证必须按定义身份找到 graph::plan。
    #[allow(unused_imports)] use crate::graph::plan;
    pub struct Public;
    impl Public { pub fn value() -> usize { 2 } }
}
pub use facade::Public;
// 来源认证锚定真实执行协议，不再要求 core 提供编译器专用的空 marker。
mod graph { pub(crate) mod plan {
    pub unsafe fn plan_set_options(_output: *mut (), _eager: bool, _concurrency: usize) {}
} }
#[macro_export] macro_rules! __nestrs_query {
    () => { $crate::activation::make().number() };
    ($input:expr) => { $input };
}
''')
        core_library = output / "libnestrs_core.rlib"
        command("runtime", [driver, core, "--edition", "2024", "--crate-name", "nestrs_core",
                            "--crate-type", "rlib", "-o", core_library])
        bridge = source("bridge", '''extern crate proc_macro;
#[proc_macro_attribute]
pub fn injectable(_: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut result = item;
    result.extend("fn generated_value() -> usize { nestrs_core::activation::make().number() }".parse::<proc_macro::TokenStream>().unwrap());
    result
}
''')
        bridge_library = output / bridge_name()
        command("bridge", [args.rustc, bridge, "--edition", "2024", "--crate-name", "nestrs_tool_bridge",
                           "--crate-type", "proc-macro", "-o", bridge_library])
        environment["PROBE_BRIDGE_PATH"] = str(bridge_library.resolve())
        externs = ["--extern", f"nestrs_core={core_library}", "--extern", f"nestrs={bridge_library}"]
        cases = {
            "public": ("fn main(){assert_eq!(nestrs_core::Public::value(),2);}", True),
            "ordinary_glob": ("use nestrs_core::*; fn main(){assert_eq!(Public::value(),2);}", True),
            "core_macro": ("fn main(){assert_eq!(nestrs_core::__nestrs_query!(),17);}", True),
            "bridge_macro": ("#[nestrs::injectable] struct A; fn main(){assert_eq!(generated_value(),17);}", True),
            "private_direct": ("fn main(){let _=nestrs_core::activation::make();}", False),
            "private_alias": ("use nestrs_core::activation as hidden; fn main(){let _=hidden::make();}", False),
            "private_glob": ("use nestrs_core::activation::*; fn main(){let _=make();}", False),
            "private_macro": ("macro_rules! bad{()=>{nestrs_core::activation::make()}} fn main(){let _=bad!();}", False),
            "private_macro_argument": ("fn main(){let _=nestrs_core::__nestrs_query!(nestrs_core::activation::make());}", False),
            "bridge_user_body": ("#[nestrs::injectable] fn bad(){let _=nestrs_core::activation::make();} fn main(){}", False),
            "explicit_reexport": ("pub use nestrs_core::activation; fn main(){}", False),
        }
        for name, (contents, succeeds) in cases.items():
            binary = output / executable_name(name)
            command(name, [driver, source(name, contents), "--edition", "2024", *externs, "-o", binary],
                    succeeds=succeeds, diagnostic=None if succeeds else "Nestrs 内部实现")
            if succeeds:
                command(name + "-run", [binary])
        # Standard rustc sees the runtime's original private metadata.
        command("ordinary_private", [args.rustc, output / "private_direct.rs", "--edition", "2024", *externs,
                                     "-o", output / executable_name("ordinary_private")],
                succeeds=False, diagnostic="E0603")
        reexport = output / "libreexport.rlib"
        command("reexport", [driver, source("reexport", "pub use nestrs_core::*;"),
                             "--crate-name", "reexport", "--crate-type", "rlib", "--edition", "2024",
                             *externs, "-o", reexport])
        dependencies = ["-L", f"dependency={output}", "--extern", f"reexport={reexport}"]
        command("reexport_public", [args.rustc, source("reexport_public", "fn main(){assert_eq!(reexport::Public::value(),2);}"),
                                    "--edition", "2024", *dependencies, "-o", output / executable_name("reexport_public")])
        command("reexport_private", [args.rustc, source("reexport_private", "fn main(){let _=reexport::activation::make();}"),
                                     "--edition", "2024", *dependencies,
                                     "-o", output / executable_name("reexport_private")], succeeds=False, diagnostic="E0433")
        # The same callback name has no authority unless the compiler marks its
        # exact source range and encodes that provenance in the producer rmeta.
        for trusted in [False, True]:
            name = "generated" if trusted else "spoofed"
            declaration = source(name, "use nestrs_core as _; pub fn __nestrs_probe_callback() -> usize { 17 }")
            library = output / f"lib{name}.rlib"
            env = dict(environment, PROBE_TRUSTED_FILE=str(declaration)) if trusted else environment
            command(name, [driver, declaration, "--edition", "2024", "--crate-type", "rlib",
                           "--crate-name", name, *externs, "-o", library], env=env)
            consumer = source("consume_" + name, f"fn main(){{assert_eq!({name}::__nestrs_probe_callback(),17);}}")
            command("consume_" + name, [driver, consumer, "--edition", "2024", *externs,
                                       "-L", f"dependency={output}", "--extern", f"{name}={library}",
                                       "-o", output / executable_name("consume_" + name)],
                    env=dict(environment, PROBE_EXPECT_TRUSTED="yes" if trusted else "no"))
        report["passed"] = True
    except (AssertionError, OSError, ValueError, RuntimeError) as error:
        report["failure"] = str(error)
        print(error, file=sys.stderr)
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"{'PASS' if report['passed'] else 'FAIL'} internal access: {output / 'report.json'}")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

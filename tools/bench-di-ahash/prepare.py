#!/usr/bin/env python3
"""为相同业务基准创建独立 Cargo 项目，不改工作区成员或生产 feature。"""

import argparse
import hashlib
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", required=True, type=Path, help="nestrs-core crate 目录")
    parser.add_argument("--output", required=True, type=Path, help="生成的独立项目目录")
    args = parser.parse_args()
    core = args.core.resolve()
    output = args.output.resolve()
    if not (core / "Cargo.toml").is_file():
        parser.error(f"core manifest 不存在：{core / 'Cargo.toml'}")
    output.mkdir(parents=True, exist_ok=True)
    (output / "src").mkdir(exist_ok=True)
    source = Path(__file__).with_name("main.rs").read_bytes()
    (output / "src" / "main.rs").write_bytes(source)
    manifest = f'''[package]
name = "di-ahash-bench"
version = "0.0.0"
edition = "2024"
publish = false

# 独立工作区，避免把基准混入框架/示例的生产成员。
[workspace]

[dependencies]
nestrs-core = {{ path = {json.dumps(str(core))} }}
tokio = {{ version = "=1.53.1", default-features = false, features = ["rt-multi-thread", "sync", "time"] }}

[profile.release]
opt-level = 3
debug = false
lto = false
codegen-units = 16
'''
    (output / "Cargo.toml").write_text(manifest, encoding="utf-8")
    print(json.dumps({
        "manifest": str(output / "Cargo.toml"),
        "core": str(core),
        "source_sha256": hashlib.sha256(source).hexdigest(),
    }))


if __name__ == "__main__":
    main()

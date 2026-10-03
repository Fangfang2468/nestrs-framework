#!/usr/bin/env python3
"""逐项目运行 cargo nestrs，验证它因预期诊断失败，并保存原始编译日志。"""

import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import sys

# 检查具体诊断和涉及的类型/输入，避免把缺工具链或普通 Rust 编译错误算作成功。
CASES = {
    '01-missing-concrete': ('MissingDependency', 'OrderService', 'Database', 'database'),
    '02-missing-trait': ('MissingDependency', 'Checkout', 'PaymentGateway', 'gateway'),
    '03-missing-default-key': ('MissingDependency', 'OrderService', 'Database', 'key=None'),
    '04-missing-named-key': ('MissingDependency', 'Checkout', 'PaymentGateway', 'live'),
    '05-key-kind-mismatch': ('MissingDependency', 'Worker', 'Queue', '7'),
    '06-ambiguous-trait': ('AmbiguousTrait', 'PaymentGateway', 'CardGateway', 'BankGateway', '没有唯一 primary'),
    '07-multiple-primary': ('AmbiguousTrait', 'PaymentGateway', 'CardGateway', 'BankGateway', '多个 primary'),
    '08-primary-other-key': ('AmbiguousTrait', 'PaymentGateway', 'CardGateway', 'BankGateway', '没有唯一 primary'),
    '09-optional-ambiguity': ('AmbiguousTrait', 'MessagePort', 'EmailSender', 'SmsSender'),
    '10-lazy-ambiguity': ('AmbiguousTrait', 'MessagePort', 'EmailSender', 'SmsSender'),
    '11-self-cycle': ('Cycle', 'Cache', 'parent'),
    '12-dependency-cycle': ('Cycle', 'OrderService', 'PaymentService', 'payment', 'order'),
    '13-optional-cycle': ('Cycle', 'Alpha', 'Beta', 'alpha', 'beta'),
    '14-lazy-cycle': ('Cycle', 'Alpha', 'Beta', 'alpha', 'beta'),
    '15-singleton-scoped': ('ScopeRequired', 'ApplicationCache', 'RequestSession'),
    '16-transitive-scoped': ('ScopeRequired', 'Application', 'Formatter', 'RequestSession'),
    '17-optional-scoped': ('ScopeRequired', 'Application', 'Session'),
    '18-lazy-scoped': ('ScopeRequired', 'Application', 'Intermediate', 'Session'),
    '19-factory-missing': ('MissingDependency', 'Application', 'Database', '_database'),
    '20-factory-cycle': ('Cycle', 'First', 'Second', '_first', '_second'),
    '21-factory-scoped': ('ScopeRequired', 'Application', 'RequestSession', '_session'),
    '22-factory-lazy-missing': ('MissingDependency', 'ReportService', 'Missing', 'missing'),
    '23-constructor-missing': ('MissingDependency', 'Application', 'Database', '_database'),
    '24-duplicate-provider': ('DuplicateProvider', 'Database', '重复 provider', 'key=None'),
    '25-generic-missing': ('MissingDependency', 'Repository', 'User', 'Database', 'database'),
    '26-generic-growth': ('DI 泛型类型不断增长或过于复杂', '1024', '递归泛型查询'),
    '27-duplicate-binding': ('DuplicateBinding', 'Service', 'Port'),
    '28-orphan-binding': ('OrphanBinding', 'Service', 'Port'),
    '29-provider-lazy-missing': ('MissingDependency', 'DeferredApplication', 'Missing', 'missing'),
    '30-multiple-errors': (
        'MissingDependency', 'AmbiguousTrait', 'OrderService', 'Database', 'Queue',
        'Mailer', 'EmailClient', 'live', 'sandbox', 'Checkout', 'CardGateway',
        'BankGateway', 'report_service', '_config', 'ReportConfig',
    ),
}


# 用户可识别的诊断编号；内部 kind 仍在 cause 中核对，不能只凭进程失败判定通过。
CODES = dict(zip(CASES, (
    "DI001", "DI001", "DI002", "DI002", "DI002",
    "DI003", "DI004", "DI003", "DI003", "DI003",
    "DI005", "DI005", "DI005", "DI005",
    "DI006", "DI006", "DI006", "DI006",
    "DI001", "DI005", "DI006", "DI001", "DI001", "DI007", "DI001", "DI008",
    "DI009", "DI010", "DI001",
    ("DI001", "DI001", "DI002", "DI003", "DI001"),
)))


# 相同编号也必须对应正确的消费位置；不能把两个 DI001 互换或重复报告同一字段。
TITLE_FRAGMENTS = {
    '30-multiple-errors': (
        ('OrderService.database', 'Database'),
        ('OrderService.queue', 'Queue'),
        ('Mailer.client', 'live', 'EmailClient'),
        ('Checkout.gateway',),
        ('report_service', '_config', 'ReportConfig'),
    ),
}


def diagnostic_codes(code):
    """单错误保留字符串配置，多错误按源码顺序列出每一条编号。"""
    return (code,) if isinstance(code, str) else tuple(code)


def source_first_failure(output, code, expected_titles=()):
    codes = diagnostic_codes(code)
    headers = list(re.finditer(r"^error(?:\[[^\]\n]+\])?: ([^\n]+)", output, re.MULTILINE))
    errors = []
    for index, header in enumerate(headers):
        title = header[1]
        end = headers[index + 1].start() if index + 1 < len(headers) else len(output)
        if title.startswith("[NESTRS-"):
            errors.append((title, output[header.end():end]))
        elif not title.startswith(("could not compile ", "aborting due to ")):
            # Cargo/rustc 的总结不是独立诊断；其他 Rust 错误不能混充预期 DI 失败。
            return False
    if len(errors) != len(codes) or (expected_titles and len(expected_titles) != len(codes)):
        return False
    for index, ((title, block), expected_code) in enumerate(zip(errors, codes)):
        if not title.startswith(f"[NESTRS-{expected_code}]"):
            return False
        if expected_titles and not all(fragment in title for fragment in expected_titles[index]):
            return False
        if any(token in title for token in ("__nestrs", "input_slot", "槽位", "ProviderDefinition")):
            return False
        location = re.search(r"^\s*--> (.+):(\d+):(\d+)\s*$", block, re.MULTILINE)
        if (not location or "target" in location[1].replace("\\", "/").split("/")
                or int(location[2]) == 0 or int(location[3]) == 0):
            return False
        helps = list(re.finditer(r"^\s*= help:", block, re.MULTILINE))
        causes = list(re.finditer(r"^\s*= note: cause:", block, re.MULTILINE))
        children = list(re.finditer(r"^\s*= (?:note|help|warning|error):", block, re.MULTILINE))
        if not helps or len(causes) != 1:
            return False
        if not (location.start() < helps[0].start() <= helps[-1].start() < causes[0].start()):
            return False
        if children[-1].start() != causes[0].start():
            return False
    return True


def is_expected_failure(returncode, output, expected):
    crashes = ("internal compiler error", "panicked at", "fatal runtime error")
    return (
        returncode > 0
        and all(fragment in output for fragment in expected)
        and not any(marker in output for marker in crashes)
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES, action="append", help="只验证指定项目，可重复")
    parser.add_argument("--operation", choices=("check", "build"), default="check")
    parser.add_argument("--offline", action="store_true", help="仅使用本机已经缓存的依赖")
    parser.add_argument("--logs-dir", type=Path, help="日志目录；默认在本组示例的 target/diagnostics 下")
    args = parser.parse_args()
    base = Path(__file__).resolve().parent
    logs = (args.logs_dir or base / "target" / "diagnostics" / args.operation).resolve()
    logs.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["CARGO_TERM_COLOR"] = "never"
    selected = args.case or list(CASES)
    results = []
    for directory in selected:
        command = [
            "cargo", "nestrs", args.operation, "--locked", "--manifest-path",
            str(base / directory / "Cargo.toml"),
        ]
        if args.offline:
            command.append("--offline")
        try:
            result = subprocess.run(
                command, cwd=base, env=environment, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
            )
        except OSError as error:
            print(f"无法启动 Cargo: {error}", file=sys.stderr)
            return 1
        output = result.stdout
        log = logs / f"{directory}.log"
        log.write_text(output, encoding="utf-8")
        missing = [text for text in CASES[directory] if text not in output]
        codes = [f"NESTRS-{code}" for code in diagnostic_codes(CODES[directory])]
        source_first = source_first_failure(output, CODES[directory], TITLE_FRAGMENTS.get(directory, ()))
        passed = (is_expected_failure(result.returncode, output, CASES[directory])
                  and source_first)
        results.append({
            "case": directory, "operation": args.operation, "command": command,
            "exit_code": result.returncode, "passed": passed,
            "diagnostic_code": codes[0] if len(codes) == 1 else codes,
            "diagnostic_codes": codes,
            "source_first": source_first,
            "missing_diagnostic_fragments": missing, "log": str(log),
        })
        status = "PASS" if passed else "FAIL"
        print(f"[{status}] {directory}: {', '.join(codes)} (cargo exit {result.returncode})", flush=True)
        if not passed:
            print(output, file=sys.stderr)
    (logs / "summary.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    passed = sum(item["passed"] for item in results)
    print(f"{passed}/{len(results)} 个项目按预期报错；原始日志：{logs}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())

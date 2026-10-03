#!/usr/bin/env python3
"""Build each frozen core with its matching toolchain; retain paired requested-heap/RSS samples."""
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
SCENARIOS = ['empty', 'concrete1', 'concrete4', 'trait1', 'trait4', 'optional1', 'optional4',
             'absent1', 'absent4', 'lazy1', 'lazy4', 'lazy-unused1', 'lazy-unused4',
             'sync0', 'sync1', 'sync4', 'async0', 'async1', 'async4', 'mixed', 'scoped', 'singleton']


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def environment():
    env = os.environ.copy()
    for key in list(env):
        if key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC_BOOTSTRAP', 'RUSTC_WRAPPER',
                   'RUSTC_WORKSPACE_WRAPPER', 'CARGO_TARGET_DIR', 'RUSTC', 'CARGO_BUILD_RUSTFLAGS',
                   'CARGO_BUILD_TARGET', 'NESTRS_DRIVER', 'NESTRS_MACRO_BRIDGE'] or key.startswith('CARGO_PROFILE_RELEASE_') or (
                       key.startswith('CARGO_TARGET_') and key.endswith('_RUSTFLAGS')):
            env.pop(key, None)
    env['CARGO_NET_OFFLINE'] = 'true'
    env['CARGO_BUILD_JOBS'] = '4'
    return env


def registry_packages(lock):
    return {f'{p["name"]}@{p["version"]}': [p['source'], p.get('checksum')]
            for p in tomllib.loads(Path(lock).read_text())['package'] if p.get('source')}


def build(args):
    old = args.output / 'build.json'
    manifest = json.loads(old.read_text()) if old.exists() else {}
    sources = {'before': args.baseline_root.resolve(), 'after': ROOT}
    clis = {'before': args.before_cli.resolve(), 'after': args.after_cli.resolve()}
    variants = ['before', 'after'] if args.variant == 'both' else [args.variant]
    for variant in variants:
        source = sources[variant]
        app = args.output / 'apps' / variant
        (app / 'src').mkdir(parents=True, exist_ok=True)
        shutil.copyfile(HERE / 'main.rs', app / 'src/main.rs')
        (app / 'Cargo.toml').write_text(f'''[package]
name = "direct-input-bench"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
nestrs-core = {{ path = {json.dumps(str(source / 'nestrs-core'))} }}
tokio = {{ version = "=1.53.1", default-features = false, features = ["rt", "sync", "time"] }}
[profile.release]
opt-level = 3
debug = false
lto = false
codegen-units = 16
''')
        env = environment()
        # Resolve the fixture package without changing registry versions; subsequent builds are locked.
        shutil.copyfile(source / 'Cargo.lock', app / 'Cargo.lock')
        generated = subprocess.run(['cargo', 'generate-lockfile', '--offline', '--manifest-path', str(app / 'Cargo.toml')],
                                   env=env, text=True, capture_output=True)
        if generated.returncode:
            raise RuntimeError(generated.stderr)
        registry = registry_packages(app / 'Cargo.lock')
        original = registry_packages(source / 'Cargo.lock')
        if not registry.items() <= original.items():
            raise RuntimeError(f'{variant}: fixture changed locked registry dependency versions')
        command = [str(clis[variant]), 'build', '--release', '--offline', '--locked',
                   '--manifest-path', str(app / 'Cargo.toml'), '--message-format=json']
        print(f'Building {variant} with {clis[variant]}', flush=True)
        result = subprocess.run(command, cwd=source, env=env, text=True, capture_output=True)
        log = args.output / f'build-{variant}.log'
        log.write_text(result.stdout + '\n' + result.stderr)
        if result.returncode:
            raise RuntimeError(f'build failed: {log}')
        artifacts = []
        for line in result.stdout.splitlines():
            try:
                item = json.loads(line)
            except json.JSONDecodeError:
                continue
            if item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == 'direct-input-bench' and item.get('executable'):
                artifacts.append(item['executable'])
        if not artifacts:
            raise RuntimeError(f'No executable in {log}')
        binary = Path(artifacts[-1]).resolve()
        tool_files = [clis[variant], clis[variant].with_name('nestrs-driver'), clis[variant].with_name('libnestrs_tool_bridge.so')]
        manifest[variant] = {
            'binary': str(binary), 'binary_sha256': digest(binary), 'command': command,
            'fixture_sha256': digest(app / 'src/main.rs'), 'registry_packages': registry,
            'lock_sha256': digest(app / 'Cargo.lock'),
            'tool_sha256': {str(p): digest(p) for p in tool_files},
            'core_sha256': {str(p.relative_to(source)): digest(p) for p in sorted((source / 'nestrs-core/src').rglob('*.rs'))},
            'compiler_sha256': {str(p.relative_to(source)): digest(p) for p in sorted((source / 'cargo-nestrs/src').rglob('*.rs'))},
            'rustc': subprocess.check_output(['rustc', '-Vv'], env=env, text=True),
            'doctor': subprocess.check_output([str(clis[variant]), 'doctor'], env=env, text=True),
        }
        save(old, manifest)
    if all(v in manifest for v in ['before', 'after']):
        if manifest['before']['registry_packages'] != manifest['after']['registry_packages']:
            raise RuntimeError('before/after registry versions differ')
        if manifest['before']['fixture_sha256'] != manifest['after']['fixture_sha256']:
            if args.variant == 'both':
                raise RuntimeError('before/after business sources differ')
            print('The other fixture variant is stale; rebuild it before measure', flush=True)


def run_sample(binary, scenario, args, variant):
    peak_file = args.output / f'peak-rss-{variant}.txt'
    command = ['/usr/bin/time', '-f', '%M', '-o', str(peak_file), binary, scenario, str(args.batches), str(args.batch_size)]
    pin = None
    if args.cpu is not None:
        pin = lambda: os.sched_setaffinity(0, {args.cpu})
    result = subprocess.run(command, env=environment(), text=True, capture_output=True, preexec_fn=pin)
    if result.returncode:
        raise RuntimeError(f'{scenario}/{variant} failed: {result.stderr}')
    value = json.loads(result.stdout)
    value['process_peak_rss_kib'] = int(peak_file.read_text().strip())
    return value


def validated_build(args):
    built = json.loads((args.output / 'build.json').read_text())
    for variant in ['before', 'after']:
        b = built[variant]
        if digest(b['binary']) != b['binary_sha256'] or digest(HERE / 'main.rs') != b['fixture_sha256']:
            raise RuntimeError(f'{variant}: binary or fixture has changed; rebuild')
    if built['before']['registry_packages'] != built['after']['registry_packages']:
        raise RuntimeError('registry dependency mismatch')
    return built


def measure(args):
    built = validated_build(args)
    scenarios = args.scenarios.split(',') if args.scenarios else SCENARIOS
    save(args.output / 'measurement.json', {
        'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'platform': platform.platform(),
        'cpu_count': os.cpu_count(), 'affinity': sorted(os.sched_getaffinity(0)), 'cpu': args.cpu,
        'cpu_info': Path('/proc/cpuinfo').read_text(), 'rounds': args.rounds,
        'batches': args.batches, 'batch_size': args.batch_size, 'scenarios': scenarios,
        'runner_sha256': digest(__file__),
        'note': 'Current-thread Tokio; scopes bounded per batch; query window excludes owner open/close and RSS reads. RSS includes runtime/code/allocator overhead; requested-byte accounting excludes allocator metadata.',
    })
    with (args.output / 'samples.jsonl').open('w') as raw:
        for scenario in scenarios:
            for index in range(args.rounds):
                pair = {}
                for variant in (['before', 'after'] if index % 2 == 0 else ['after', 'before']):
                    value = run_sample(built[variant]['binary'], scenario, args, variant)
                    value.update(variant=variant, round=index)
                    raw.write(json.dumps(value) + '\n'); raw.flush()
                    pair[variant] = value
                for key in ['checksum', 'created', 'dropped', 'queries']:
                    if pair['before'][key] != pair['after'][key]:
                        raise RuntimeError(f'{scenario}: before/after {key} differs')
                print(f'{scenario} {index + 1}/{args.rounds}: allocated/query '
                      f'{pair["before"]["allocated_bytes"] / pair["before"]["queries"]:.2f} -> '
                      f'{pair["after"]["allocated_bytes"] / pair["after"]["queries"]:.2f} bytes', flush=True)


def allocator_evidence(args, samples, filename):
    evidence = {
        'fixture_sha256': digest(HERE / 'main.rs'),
        'evidence': 'Every successful process executes allocator_self_test before runtime creation and asserts exact deltas; any failure exits nonzero and stops the runner.',
        'asserted_successful_alloc_or_zeroed': 2, 'asserted_successful_realloc': 1,
        'asserted_dealloc': 2, 'asserted_requested_bytes': 88, 'asserted_freed_bytes': 88,
        'asserted_live_change_bytes': 0, 'zero_initialization_checked': True,
        'realloc_contents_preserved_checked': True,
        'failure_allocation_injected': False,
        'successful_processes': {v: sum(s['variant'] == v for s in samples) for v in ['before', 'after']},
    }
    save(args.output / filename, evidence)


def stress(args):
    built = validated_build(args)
    scenarios = args.scenarios.split(',') if args.scenarios else ['mixed', 'lazy4', 'async4']
    save(args.output / 'retention-stress-config.json', {
        'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'scenarios': scenarios,
        'batches': args.batches, 'batch_size': args.batch_size, 'cpu': args.cpu,
        'runner_sha256': digest(__file__), 'note': 'One process per version/scenario; retention stress, not a latency comparison.'})
    values = []
    with (args.output / 'retention-stress.jsonl').open('w') as raw:
        for index, scenario in enumerate(scenarios):
            pair = {}
            for variant in (['before', 'after'] if index % 2 == 0 else ['after', 'before']):
                value = run_sample(built[variant]['binary'], scenario, args, variant)
                value.update(variant=variant, round=0)
                raw.write(json.dumps(value) + '\n'); raw.flush()
                values.append(value); pair[variant] = value
            for key in ['checksum', 'created', 'dropped', 'queries']:
                if pair['before'][key] != pair['after'][key]:
                    raise RuntimeError(f'{scenario}: stress {key} differs')
            print(f'{scenario}: {pair["before"]["queries"]} queries/version; '
                  f'live delta {pair["before"]["steady_end_live_bytes"] - pair["before"]["steady_start_live_bytes"]} -> '
                  f'{pair["after"]["steady_end_live_bytes"] - pair["after"]["steady_start_live_bytes"]}; '
                  f'drops {pair["before"]["dropped"]} -> {pair["after"]["dropped"]}', flush=True)
    allocator_evidence(args, values, 'retention-allocator-self-test.json')


def distribution(values):
    if len(values) == 1:
        q = [values[0]] * 3
    else:
        q = statistics.quantiles(values, n=4, method='inclusive')
    return {'median': statistics.median(values), 'p25': q[0], 'p75': q[2], 'min': min(values), 'max': max(values)}


def summarize(args):
    samples = [json.loads(line) for line in (args.output / 'samples.jsonl').read_text().splitlines()]
    allocator_evidence(args, samples, 'allocator-self-test.json')
    groups = {}
    for s in samples:
        groups.setdefault(s['scenario'], {}).setdefault(s['variant'], []).append(s)
    rows = []
    for scenario, variants in groups.items():
        row = {'scenario': scenario}
        for variant, values in variants.items():
            row[variant] = {
                'samples': len(values),
                **{metric + '_per_query': distribution([v[metric] / v['queries'] for v in values]) for metric in
                   ['allocation_calls', 'reallocation_calls', 'allocated_bytes', 'elapsed_ns']},
                **{metric: distribution([v[metric] for v in values]) for metric in
                   ['peak_live_bytes', 'peak_live_extra_bytes', 'steady_start_live_bytes', 'steady_end_live_bytes', 'root_closed_live_bytes',
                    'rss_start_kib', 'rss_end_kib', 'process_vm_hwm_kib', 'process_peak_rss_kib']},
                'steady_live_delta_bytes': distribution([v['steady_end_live_bytes'] - v['steady_start_live_bytes'] for v in values]),
                'checkpoint_live_spread_bytes': distribution([max(c['live_bytes'] for c in v['checkpoints']) - min(c['live_bytes'] for c in v['checkpoints']) for v in values]),
                'checkpoint_live_last_minus_first_bytes': distribution([v['checkpoints'][-1]['live_bytes'] - v['checkpoints'][0]['live_bytes'] for v in values]),
                'checkpoint_rss_last_minus_first_kib': distribution([v['checkpoints'][-1]['rss_kib'] - v['checkpoints'][0]['rss_kib'] for v in values]),
                'checkpoint_rss_spread_kib': distribution([max(c['rss_kib'] for c in v['checkpoints']) - min(c['rss_kib'] for c in v['checkpoints']) for v in values]),
                'created_equals_dropped': all(v['created'] == v['dropped'] for v in values),
            }
        pairs = {v['round']: v for v in variants['before']}
        reductions = [100 * (1 - v['elapsed_ns'] / pairs[v['round']]['elapsed_ns']) for v in variants['after']]
        rng = random.Random(20261002)
        bootstrap = sorted(statistics.median(rng.choices(reductions, k=len(reductions))) for _ in range(10000))
        row['paired_elapsed_reduction_pct'] = distribution(reductions)
        row['paired_elapsed_bootstrap_95pct'] = [bootstrap[250], bootstrap[9749]]
        rows.append(row)
    save(args.output / 'summary.json', rows)
    marginals = []
    for family in ['concrete', 'trait', 'optional', 'absent', 'lazy', 'lazy-unused', 'sync', 'async']:
        if family + '1' not in groups or family + '4' not in groups:
            continue
        marginal = {'family': family, 'note': 'Difference between 4-input and 1-input services divided by 3; includes all public query/runtime costs and object field sizes.'}
        for variant in ['before', 'after']:
            one = {v['round']: v for v in groups[family + '1'][variant]}
            four = groups[family + '4'][variant]
            marginal[variant] = {metric: distribution([(v[metric] / v['queries'] - one[v['round']][metric] / one[v['round']]['queries']) / 3 for v in four])
                                 for metric in ['allocation_calls', 'allocated_bytes']}
        marginals.append(marginal)
    save(args.output / 'marginal-inputs.json', marginals)
    lines = ['# Direct input memory comparison', '',
             'All byte figures below are allocator-requested sizes. They exclude allocator headers, rounding, stacks, executable mappings, and shared pages. RSS is a distinct process metric. Query measurements exclude scope setup/close and procfs reads; retained heap checkpoints are captured after each bounded scope is closed. No outliers removed.', '',
             '| Scenario | allocations/query before → after | requested bytes/query before → after | peak additional live bytes/batch before → after | post-close live delta bytes before → after | VmHWM KiB before → after |',
             '| --- | ---: | ---: | ---: | ---: | ---: |']
    for row in rows:
        cells = []
        for key in ['allocation_calls_per_query', 'allocated_bytes_per_query', 'peak_live_extra_bytes', 'steady_live_delta_bytes', 'process_vm_hwm_kib']:
            cells.append(f'{row["before"][key]["median"]:.2f} → {row["after"][key]["median"]:.2f}')
        lines.append('| ' + row['scenario'] + ' | ' + ' | '.join(cells) + ' |')
    lines += ['', '| Input kind | marginal allocations/input before → after | marginal requested bytes/input before → after |', '| --- | ---: | ---: |']
    for row in marginals:
        lines.append(f'| {row["family"]} | {row["before"]["allocation_calls"]["median"]:.2f} → {row["after"]["allocation_calls"]["median"]:.2f} | {row["before"]["allocated_bytes"]["median"]:.2f} → {row["after"]["allocated_bytes"]["median"]:.2f} |')
    lines += ['', 'Marginals use the 4-input minus 1-input service result divided by 3 and include payload field sizes and the complete runtime path, not only the input adapter.', '', 'Distributions (min, P25, median, P75, max), paired latency bootstrap intervals, all scope checkpoint samples, construction/drop assertions, binary/tool/source hashes and dependency locks are retained in summary.json, samples.jsonl and build.json.',
              'A flat requested-live checkpoint and matching drops provide bounded workload evidence; they are not proof that every workload is leak-free. Peak RSS includes process startup and does not equal the request-heap peak.', '']
    stress_file = args.output / 'retention-stress.jsonl'
    if stress_file.exists():
        stress_samples = [json.loads(line) for line in stress_file.read_text().splitlines()]
        retention = []
        lines += ['', '## Extended retention check', '',
                  '| Scenario/version | queries | created / dropped | post-close live delta bytes | checkpoint live min–max bytes | RSS first → last KiB |',
                  '| --- | ---: | ---: | ---: | ---: | ---: |']
        for v in stress_samples:
            live = [c['live_bytes'] for c in v['checkpoints']]
            rss_values = [c['rss_kib'] for c in v['checkpoints']]
            r = {'scenario': v['scenario'], 'variant': v['variant'], 'queries': v['queries'],
                 'created': v['created'], 'dropped': v['dropped'],
                 'steady_live_delta_bytes': v['steady_end_live_bytes'] - v['steady_start_live_bytes'],
                 'checkpoint_live_min_bytes': min(live), 'checkpoint_live_max_bytes': max(live),
                 'checkpoint_live_last_minus_first_bytes': live[-1] - live[0],
                 'checkpoint_rss_min_kib': min(rss_values), 'checkpoint_rss_max_kib': max(rss_values),
                 'checkpoint_rss_last_minus_first_kib': rss_values[-1] - rss_values[0],
                 'rss_start_kib': v['rss_start_kib'], 'rss_end_kib': v['rss_end_kib'],
                 'process_vm_hwm_kib': v['process_vm_hwm_kib'],
                 'root_closed_live_bytes': v['root_closed_live_bytes']}
            retention.append(r)
            lines.append(f'| {v["scenario"]}/{v["variant"]} | {v["queries"]} | {v["created"]} / {v["dropped"]} | {r["steady_live_delta_bytes"]} | {min(live)}–{max(live)} | {rss_values[0]} → {rss_values[-1]} |')
        save(args.output / 'retention-summary.json', retention)
    (args.output / 'RESULTS.md').write_text('\n'.join(lines))
    print('\n'.join(lines))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('phase', choices=['build', 'measure', 'stress', 'summarize'])
    p.add_argument('--output', type=Path, default=ROOT / 'target/direct-input-20261002/benchmark')
    p.add_argument('--baseline-root', type=Path, default=ROOT / 'target/direct-input-20261002/baseline')
    p.add_argument('--before-cli', type=Path, default=ROOT / 'target/direct-input-20261002/benchmark/baseline-toolchain/debug/cargo-nestrs')
    p.add_argument('--after-cli', type=Path, default=ROOT / 'target/debug/cargo-nestrs')
    p.add_argument('--variant', choices=['before', 'after', 'both'], default='both')
    p.add_argument('--rounds', type=int, default=8)
    p.add_argument('--batches', type=int, default=32)
    p.add_argument('--batch-size', type=int, default=32)
    p.add_argument('--cpu', type=int)
    p.add_argument('--scenarios')
    args = p.parse_args()
    args.output = args.output.resolve(); args.output.mkdir(parents=True, exist_ok=True)
    {'build': build, 'measure': measure, 'stress': stress, 'summarize': summarize}[args.phase](args)


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Frozen-source memory/CPU experiments; no network or production settings changes."""
import argparse, datetime, hashlib, json, os, pathlib, platform, random, resource, shutil, statistics, subprocess, tomllib
HERE=pathlib.Path(__file__).resolve().parent
ROOT=HERE.parent.parent

def digest(p): return hashlib.sha256(pathlib.Path(p).read_bytes()).hexdigest()
def save(p,v): pathlib.Path(p).write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n')
def env():
    result=os.environ.copy()
    for k in list(result):
        if k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_BOOTSTRAP','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','CARGO_TARGET_DIR','RUSTC','CARGO_BUILD_RUSTFLAGS','CARGO_BUILD_TARGET','NESTRS_DRIVER','NESTRS_MACRO_BRIDGE') or k.startswith('CARGO_PROFILE_RELEASE_') or (k.startswith('CARGO_TARGET_') and k.endswith('_RUSTFLAGS')): result.pop(k,None)
    result.update(CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4')
    return result
def registries(p): return {f'{x["name"]}@{x["version"]}':[x['source'],x.get('checksum')] for x in tomllib.loads(pathlib.Path(p).read_text())['package'] if x.get('source')}
def source_hashes(root):
    paths=[root/p for p in ('Cargo.toml','Cargo.lock','nestrs-core/Cargo.toml',
        'cargo-nestrs/Cargo.toml','cargo-nestrs/toolchain.json','cargo-nestrs/build.rs',
        'cargo-nestrs/internal/bridge/Cargo.toml')]
    paths+=sorted((root/'nestrs-core/src').rglob('*.rs'))
    paths+=sorted((root/'cargo-nestrs/src').rglob('*.rs'))
    paths+=sorted((root/'cargo-nestrs/internal/bridge/src').rglob('*.rs'))
    return {str(p.relative_to(root)):digest(p) for p in paths}

def build(a):
    out=a.output.resolve();out.mkdir(parents=True,exist_ok=True)
    frozen=out/'toolchain';frozen.mkdir(exist_ok=True)
    for name in ('cargo-nestrs','nestrs-driver','libnestrs_tool_bridge.so'):
        src=a.cli.resolve().with_name(name);dst=frozen/name
        if not dst.exists():shutil.copy2(src,dst)
        if digest(src)!=digest(dst):raise RuntimeError('Compiler tools changed; use the frozen CLI path or a new experiment directory')
    compiler=subprocess.check_output(['rustc','--print','sysroot'],text=True).strip()+'/bin/rustc'
    rustc=subprocess.check_output([compiler,'-Vv'],text=True)
    pin=json.loads((a.baseline_root/'cargo-nestrs/toolchain.json').read_text())
    assert f'commit-hash: {pin["commit_hash"]}' in rustc and f'release: {pin["release"]}' in rustc
    record=out/'build.json';built=json.loads(record.read_text()) if record.exists() else {}
    variant=a.variant
    source=a.baseline_root.resolve() if variant in ('before','manual') else a.after_root.resolve()
    app=out/'apps'/variant;(app/'src').mkdir(parents=True,exist_ok=True)
    for p in HERE.glob('*.rs'):shutil.copyfile(p,app/'src'/p.name)
    (app/'src/padding.rs').write_text('// Unrequested providers retained in the frozen execution graph.\n'+''.join(f'#[injectable]\nstruct Dormant{i};\n' for i in range(a.padding)))
    (app/'Cargo.toml').write_text(f'''[package]
name="nestrs-memory-bench"
version="0.0.0"
edition="2024"
publish=false
[workspace]
[features]
default=["di"]
di=["dep:nestrs-core"]
manual=[]
large-graph=[]
[dependencies]
nestrs-core={{path={json.dumps(str(source/'nestrs-core'))},optional=true}}
tokio={{version="=1.53.1",default-features=false,features=["rt-multi-thread","sync","time"]}}
[profile.release]
opt-level=3
debug=false
lto=false
codegen-units=16
''')
    e=env();e['RUSTC']=compiler;e['CARGO_TARGET_DIR']=str(out/'cargo-target'/variant)
    if not (app/'Cargo.lock').exists():
        shutil.copyfile(source/'Cargo.lock',app/'Cargo.lock')
        subprocess.run(['cargo','generate-lockfile','--offline','--manifest-path',str(app/'Cargo.toml')],env=e,check=True,capture_output=True)
    registry=registries(app/'Cargo.lock')
    assert registry.items() <= registries(source/'Cargo.lock').items(), 'registry versions changed'
    provenance=source_hashes(source)
    if variant in ('before','manual') and (out/'source-before.json').exists():
        original=json.loads((out/'source-before.json').read_text())
        assert all(original.get(k)==v for k,v in provenance.items()),'baseline differs from saved snapshot'
    (out/'bin').mkdir(exist_ok=True)
    for size in (['small'] if variant=='manual' else ['small','large']):
        command=(["cargo"] if variant=='manual' else [str(frozen/'cargo-nestrs')])+['build','--release','--offline','--locked','--manifest-path',str(app/'Cargo.toml'),'--message-format=json']
        if variant=='manual':command+=['--no-default-features','--features','manual']
        elif size=='large':command+=['--features','large-graph']
        print('Building',variant,size,flush=True)
        proc=subprocess.run(command,cwd=source,env=e,text=True,capture_output=True)
        (out/f'build-{variant}-{size}.log').write_text(proc.stdout+'\n'+proc.stderr)
        if proc.returncode:raise RuntimeError(f'{variant}/{size} build failed; inspect saved log')
        artifacts=[json.loads(x) for x in proc.stdout.splitlines() if x.startswith('{')]
        binary=[x['executable'] for x in artifacts if x.get('reason')=='compiler-artifact' and x.get('executable') and x.get('target',{}).get('name')=='nestrs-memory-bench'][-1]
        retained=out/'bin'/f'{variant}-{size}';shutil.copy2(binary,retained)
        assert provenance==source_hashes(source),'source changed during build'
        key=f'{variant}-{size}'
        reflection=None
        if variant!='manual':
            expected_nodes=9+(a.padding if size=='large' else 0)
            matches=[]
            for f in pathlib.Path(e['CARGO_TARGET_DIR']).rglob('*.nestrs-reflect.json'):
                value=json.loads(f.read_text())
                if len(value['nodes'])==expected_nodes:matches.append(f)
            assert len(matches)==1,('expected one matching reflection sidecar',matches)
            dest=out/f'reflect-{variant}-{size}.json';shutil.copyfile(matches[0],dest)
            reflection=dict(path=str(dest),sha256=digest(dest),nodes=expected_nodes)
        built[key]=dict(binary=str(retained),binary_sha256=digest(retained),source_root=str(source),source_sha256=provenance,
            fixture_sha256={p.name:digest(p) for p in (app/'src').glob('*.rs')},registry_packages=registry,
            lock_sha256=digest(app/'Cargo.lock'),command=command,rustc=rustc,
            tools_sha256={p.name:digest(p) for p in frozen.iterdir()},padding=a.padding,reflection=reflection,
            built_utc=datetime.datetime.now(datetime.timezone.utc).isoformat())
        save(record,built)
        print('Saved',retained,flush=True)

CASES={
 'mixed64':('mixed',32,64,8,65536,0,0,'small'),
 'mixed512':('mixed',24,512,8,65536,0,0,'small'),
 'scoped64':('scoped',48,64,8,65536,0,0,'small'),
 'lazy64':('lazy4',32,64,8,65536,0,0,'small'),
 'async64':('async4',32,64,8,65536,0,0,'small'),
 'payload256':('payload',24,256,8,1048576,0,0,'small'),
 'sparse-small':('sparse',48,64,8,65536,0,0,'small'),
 'sparse-large':('sparse',48,64,8,65536,0,0,'large'),
 'burst-recovery':('mixed',1200,1,8,65536,0,512,'small'),
}

def measure(a):
    out=a.output.resolve();built=json.loads((out/'build.json').read_text())
    variants=a.variants.split(',');cases=a.cases.split(',') if a.cases else list(CASES)
    used={f'{v}-'+('small' if v=='manual' else CASES[c][-1]) for v in variants for c in cases}
    reference=built[next(iter(used))]
    current_fixture={p.name:digest(p) for p in HERE.glob('*.rs')}
    for k in used:
        b=built[k]
        assert b['registry_packages']==reference['registry_packages'],'registry mismatch'
        assert b['fixture_sha256']==reference['fixture_sha256'],'probe sources differ'
        assert b['tools_sha256']==reference['tools_sha256'],'compiler tool identity mismatch'
        assert all(b['fixture_sha256'].get(k)==v for k,v in current_fixture.items()),'probe source edited after build'
        assert digest(b['binary'])==b['binary_sha256'],'binary changed after build'
        assert all(digest(out/'toolchain'/k)==v for k,v in b['tools_sha256'].items()),'frozen tool changed'
        assert source_hashes(pathlib.Path(b['source_root']))==b['source_sha256'],'source snapshot changed after build'
    d=out/a.measurement;d.mkdir(exist_ok=False)
    save(d/'config.json',dict(variants=variants,cases={k:CASES[k] for k in cases},rounds=a.rounds,cpus=a.cpus,
         platform=platform.platform(),cpuinfo=pathlib.Path('/proc/cpuinfo').read_text(),meminfo=pathlib.Path('/proc/meminfo').read_text(),
         runner_sha256=digest(__file__),utc=datetime.datetime.now(datetime.timezone.utc).isoformat()))
    raw=(d/'samples.jsonl').open('w')
    for case in cases:
      parameters=CASES[case]
      for round in range(a.rounds):
       ordered=variants[round%len(variants):]+variants[:round%len(variants)]
       identity=None
       for variant in ordered:
        size='small' if variant=='manual' else parameters[-1];b=built[f'{variant}-{size}']
        assert digest(b['binary'])==b['binary_sha256']
        base=d/f'{case}-{variant}-{round}';timefile=base.with_suffix('.time.json')
        cmd=['/usr/bin/timeout','--signal=TERM','--kill-after=5s','180s',
             '/usr/bin/time','-f','{"elapsed":%e,"user":%U,"system":%S,"peak_rss_kib":%M}', '-o',str(timefile),b['binary']]+list(map(str,parameters[:-1]))
        usage0=resource.getrusage(resource.RUSAGE_CHILDREN)
        proc=subprocess.run(cmd,env=env(),text=True,capture_output=True,preexec_fn=lambda:os.sched_setaffinity(0,set(map(int,a.cpus.split(',')))))
        usage1=resource.getrusage(resource.RUSAGE_CHILDREN)
        base.with_suffix('.jsonl').write_text(proc.stdout);base.with_suffix('.stderr').write_text(proc.stderr)
        if proc.returncode:raise RuntimeError(f'failed {base}')
        rows=[json.loads(x) for x in proc.stdout.splitlines()]
        header=rows[0];frames=rows[1:];closed=[r for r in frames if r['stage']=='closed'];held=[r for r in frames if r['stage']=='held']
        assert len(closed)==len(held)==parameters[1]
        assert all(r['created']==r['dropped'] for r in closed)
        queries=sum(r['concurrency']*parameters[3] for r in closed)
        metrics=dict(queries=queries,checksum=frames[-1]['checksum'],created=frames[-1]['created'],dropped=frames[-1]['dropped'],
          allocated_bytes=sum(r['allocated_bytes'] for r in closed),allocs=sum(r['allocs'] for r in closed),
          reallocs=sum(r['reallocs'] for r in closed),deallocs=sum(r['deallocs'] for r in closed),
          query_ns=sum(r['query_ns'] for r in closed),close_ns=sum(r['close_ns'] for r in closed),
          held_live_peak_bytes=max(r['live_bytes'] for r in held),held_rss_peak_kib=max(r['rss_kib'] for r in held),
          live_peak_bytes=max(r['peak_live_bytes'] for r in frames),root_closed=frames[-1],
          final_closed_live_bytes=closed[-1]['live_bytes'],final_closed_rss_kib=closed[-1]['rss_kib'],
          process=json.loads(timefile.read_text()))
        metrics['throughput_active_queries_per_second']=queries/((metrics['query_ns']+metrics['close_ns'])/1e9)
        metrics['process']['cpu_user_seconds']=usage1.ru_utime-usage0.ru_utime
        metrics['process']['cpu_system_seconds']=usage1.ru_stime-usage0.ru_stime
        workload_identity={k:metrics[k] for k in ('queries','checksum','created','dropped')}
        if identity is None:identity=workload_identity
        else:assert identity==workload_identity,('workload differs across variants',case,variant)
        result=dict(case=case,variant=variant,round=round,binary_sha256=b['binary_sha256'],header=header,metrics=metrics)
        raw.write(json.dumps(result)+'\n');raw.flush()
        print(case,variant,round,'alloc/query',round_float(metrics['allocated_bytes']/queries),'close/ms',round_float(metrics['close_ns']/1e6),flush=True)
    raw.close()

def round_float(n):return float(f'{n:.3f}')

def distribution(values):
    ordered=sorted(values)
    quartiles=statistics.quantiles(values,n=4,method='inclusive') if len(values)>1 else [values[0]]*3
    return dict(n=len(values),median=statistics.median(values),p25=quartiles[0],p75=quartiles[2],min=min(values),max=max(values))

def summarize(a):
    d=a.output.resolve()/a.measurement
    rows=[json.loads(x) for x in (d/'samples.jsonl').read_text().splitlines()]
    groups={}
    metrics=('allocated_bytes','allocs','reallocs','deallocs','query_ns','close_ns','held_live_peak_bytes','held_rss_peak_kib','live_peak_bytes','final_closed_live_bytes','final_closed_rss_kib','throughput_active_queries_per_second')
    for row in rows:groups.setdefault(row['case'],{}).setdefault(row['variant'],[]).append(row)
    result={}
    for case,variants in groups.items():
        item={}
        for variant,samples in variants.items():
            item[variant]={k:distribution([r['metrics'][k] for r in samples]) for k in metrics}
            for key in ('allocated_bytes','allocs','reallocs','deallocs'):
                item[variant][key+'_per_query']=distribution([r['metrics'][key]/r['metrics']['queries'] for r in samples])
            for key in ('rss_kib','live_bytes','close_ns'):
                item[variant]['root_closed_'+key]=distribution([r['metrics']['root_closed'][key] for r in samples])
            for key in ('elapsed','peak_rss_kib','cpu_user_seconds','cpu_system_seconds'):
                item[variant]['process_'+key]=distribution([r['metrics']['process'][key] for r in samples])
        if 'before' in variants and 'after' in variants:
            before={r['round']:r for r in variants['before']};after={r['round']:r for r in variants['after']}
            assert before.keys()==after.keys()
            for round in before:
                for k in ('queries','checksum','created','dropped'):
                    assert before[round]['metrics'][k]==after[round]['metrics'][k]
            changes={};rng=random.Random(20261002)
            for key in metrics:
                pairs=[(before[i]['metrics'][key],after[i]['metrics'][key]) for i in before]
                if any(b==0 for b,_ in pairs):continue
                improvements=[(a/b-1)*100 if key=='throughput_active_queries_per_second' else (1-a/b)*100 for b,a in pairs]
                boots=sorted(statistics.median(rng.choices(improvements,k=len(improvements))) for _ in range(10000))
                changes[key]=dict(median_percent=statistics.median(improvements),bootstrap_95_percent=[boots[249],boots[9749]],positive_is_improvement=True)
            item['before_after_paired_changes']=changes
        result[case]=item
    save(d/'summary.json',result)
    recovery={}
    for path in sorted(d.glob('burst-recovery-*.jsonl')):
        frames=[json.loads(x) for x in path.read_text().splitlines()]
        closed=[r for r in frames if r.get('stage')=='closed']
        chunks=[]
        for i in range(0,len(closed),64):
            block=closed[i:i+64]
            chunks.append(dict(first_wave=block[0]['wave'],last_wave=block[-1]['wave'],
                live_bytes=distribution([r['live_bytes'] for r in block]),
                rss_kib=distribution([r['rss_kib'] for r in block])))
        recovery[path.name]=dict(burst_closed=closed[0],last_closed=closed[-1],root_closed=frames[-1],blocks_of_64_waves=chunks)
    if recovery:save(d/'burst-recovery.json',recovery)
    lines=['# Core 内存实验数据汇总','',f'原始数据：`{d / "samples.jsonl"}`。所有数值由 retained 二进制测量；manual 是显式组装工程基线，不等价于完整 DI 契约。','',
        '| 场景 | 变体 | 请求分配 B/query | alloc/query | hold live MiB | hold RSS MiB | close 总 ms | 活跃区间 queries/s | 全进程 CPU s | 全进程 elapsed s |','| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for case,item in result.items():
        for variant,s in item.items():
            if variant=='before_after_paired_changes':continue
            samples=groups[case][variant]
            cpu=statistics.median(r['metrics']['process']['cpu_user_seconds']+r['metrics']['process']['cpu_system_seconds'] for r in samples)
            lines.append(f'| {case} | {variant} | {s["allocated_bytes_per_query"]["median"]:.2f} | {s["allocs_per_query"]["median"]:.2f} | {s["held_live_peak_bytes"]["median"]/2**20:.3f} | {s["held_rss_peak_kib"]["median"]/1024:.3f} | {s["close_ns"]["median"]/1e6:.3f} | {s["throughput_active_queries_per_second"]["median"]:.1f} | {cpu:.4f} | {s["process_elapsed"]["median"]:.2f} |')
    lines+=['','分布（中位数、四分位、范围）、root 关闭后 retained、CPU 和配对变化 bootstrap 区间详见同目录 `summary.json`。置信区间跨零的变化不能称为稳定改善；计数分配器增加原子操作开销，因此这里不是未插桩的云服务 QPS。']
    (d/'SUMMARY.md').write_text('\n'.join(lines)+'\n')
    print(d/'SUMMARY.md')
def main():
    p=argparse.ArgumentParser();p.add_argument('--output',type=pathlib.Path,default=ROOT/'target/core-memory-20261002')
    sub=p.add_subparsers(dest='command',required=True)
    b=sub.add_parser('build');b.add_argument('--variant',choices=['before','after','manual'],required=True)
    b.add_argument('--baseline-root',type=pathlib.Path,default=ROOT/'target/core-memory-20261002/baseline')
    b.add_argument('--after-root',type=pathlib.Path,default=ROOT);b.add_argument('--cli',type=pathlib.Path,default=ROOT/'target/debug/cargo-nestrs');b.add_argument('--padding',type=int,default=1024)
    m=sub.add_parser('measure');m.add_argument('--variants',default='before,after,manual');m.add_argument('--cases');m.add_argument('--rounds',type=int,default=7);m.add_argument('--cpus',default='2,4');m.add_argument('--measurement',default='paired')
    s=sub.add_parser('summarize');s.add_argument('--measurement',default='paired')
    a=p.parse_args();globals()[a.command](a)
if __name__=='__main__':main()

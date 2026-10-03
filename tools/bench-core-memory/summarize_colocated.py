#!/usr/bin/env python3
"""Audit one completed colocated.py run, including independent resource absence checks."""
import argparse, collections, datetime, hashlib, json, pathlib, re, subprocess

def read(p):return json.loads(p.read_text())
def check(command):
    p=subprocess.run(command,capture_output=True,text=True,timeout=20)
    return dict(command=command,exit_code=p.returncode,stdout=p.stdout.strip(),stderr=p.stderr.strip())
def info(p):
    return {k:int(v) if v.isdigit() else v for line in p.read_text().splitlines() if ':' in line for k,v in [line.split(':',1)]}

def main():
    parser=argparse.ArgumentParser();parser.add_argument('directory',type=pathlib.Path);a=parser.parse_args();d=a.directory.resolve()
    r=read(d/'report.json');assert r['passed'] and not r['sample_errors'] and r['app_exit_code']==r['pgbench_exit_code']==0
    assert hashlib.sha256(pathlib.Path(r['app_binary']).read_bytes()).hexdigest()==r['app_sha256']
    samples=[json.loads(x) for x in (d/'cgroups.jsonl').read_text().splitlines()]
    groups={}
    for name in ('slice','app','postgres','redis'):
        entries=[(s,s['groups'][name]) for s in samples if 'memory.current' in s['groups'].get(name,{})]
        snap,g=max(entries,key=lambda p:p[1]['memory.current'])
        events={k:{e:max(v.get(k,{}).get(e,0) for _,v in entries) for e in ('max','oom','oom_kill','oom_group_kill')} for k in ('memory.events','memory.events.local')}
        assert all(events['memory.events'][e]==0 for e in ('oom','oom_kill','oom_group_kill'))
        swap=max(v['memory.swap.current'] for _,v in entries);assert swap==0
        groups[name]=dict(kernel_peak_bytes=max(v['memory.peak'] for _,v in entries),
            max_sampled_current_bytes=g['memory.current'],peak_sample_phase=snap['phase'],peak_sample_unix=snap['unix'],
            peak_sample_stat={k:g['memory.stat'][k] for k in ('anon','file','shmem','kernel')},
            events=events,swap_peak_bytes=max(v.get('memory.swap.peak',0) for _,v in entries),
            cpu_final=entries[-1][1]['cpu.stat'])
        assert groups[name]['swap_peak_bytes']==0
    assert groups['slice']['events']['memory.events.local']['max']==0
    assert groups['slice']['events']['memory.events']['max']==groups['postgres']['events']['memory.events.local']['max']
    rows=[json.loads(x) for x in (d/'app.stdout.txt').read_text().splitlines()];header=rows[0];final=rows[-1]
    held=[x for x in rows if x.get('stage')=='held'];closed=[x for x in rows if x.get('stage')=='closed']
    assert len(held)==len(closed)==header['waves']
    n=header['concurrency'];q=header['queries_per_scope'];payload=header['payload_bytes']
    assert header['scenario']=='payload'
    for i,(h,c) in enumerate(zip(held,closed)):
        assert h['wave']==c['wave']==i and h['created']==c['created']==(i+4)*n
        assert h['created']-h['dropped']==n and c['created']==c['dropped']
        assert c['checksum']==n*q*payload
    assert final['stage']=='root_closed' and final['created']==final['dropped']==(header['waves']+3)*n
    assert final['checksum']==header['waves']*n*q*payload
    redislogs=read(d/'redis-load.json');assert all(x['exit_code']==0 and not x['stderr'].strip() for x in redislogs)
    requests={k:v*100000 for k,v in collections.Counter(x['operation'] for x in redislogs).items()}
    before=info(d/'redis-info-before-load.stdout.txt');after=info(d/'redis-info-after-load.stdout.txt')
    assert after['keyspace_hits']-before['keyspace_hits']==requests['GET'] and after['keyspace_misses']==after['evicted_keys']==0
    assert int((d/'redis-dbsize-before-load.stdout.txt').read_text())==int((d/'redis-dbsize-after-load.stdout.txt').read_text())==50000
    pg=(d/'pgbench-load.stdout.txt').read_text()
    pgbench=dict(transactions=int(re.search(r'actually processed: (\d+)',pg)[1]),failed=int(re.search(r'failed transactions: (\d+)',pg)[1]),tps=float(re.search(r'^tps = ([\d.]+)',pg,re.M)[1]),latency_average_ms=float(re.search(r'latency average = ([\d.]+)',pg)[1]))
    assert pgbench['failed']==0
    res=r['resources'];checks=[]
    for label in ('postgres','redis'):
        result=check(['docker','inspect','--type','container',res[label]]);checks.append(result)
        assert result['exit_code']!=0 and 'No such' in result['stderr']
        assert read(d/f'{res[label]}-final-inspect.json')[0]['State']['OOMKilled'] is False
    result=check(['docker','volume','inspect',res['volume']]);checks.append(result)
    assert result['exit_code']!=0 and 'no such' in result['stderr'].lower()
    for label in ('slice','app_scope'):
        result=check(['systemctl','show',res[label],'--property=LoadState,ActiveState,SubState,Transient,FragmentPath,ControlGroup']);checks.append(result)
        props=dict(x.split('=',1) for x in result['stdout'].splitlines())
        assert props['ActiveState']=='inactive' and props['SubState']=='dead' and props['Transient']=='no'
        assert not props['FragmentPath'] and not props['ControlGroup']
        assert not (pathlib.Path('/run/systemd/transient')/res[label]).exists()
    assert not (pathlib.Path('/sys/fs/cgroup')/res['slice']).exists()
    keys=('used_memory','used_memory_peak','used_memory_rss','evicted_keys','keyspace_hits','keyspace_misses','db0')
    result=dict(verified_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),all_checks_passed=True,
        report=str(d/'report.json'),binary_sha256=r['app_sha256'],limits=r['limits'],load_seconds=r['load_seconds'],groups=groups,
        app=dict(header=header,root_closed=final,queries=header['waves']*n*q,held_rss_peak_kib=max(x['rss_kib'] for x in held),peak_live_bytes=max(x['peak_live_bytes'] for x in rows[1:])),
        pgbench=pgbench,redis=dict(requests=requests,before={k:before[k] for k in keys},after={k:after[k] for k in keys}),
        cleanup_checks=checks,all_owned_resources_absent=True)
    (d/'summary.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    lines=['# 优化后同机容量复验','',f'同机数据库压力 {r["load_seconds"]:.3f} s，app 为 256 scopes × 1 MiB、每 scope 8 次查询、3 波预热+700 波测量、hold 100 ms。全部退出成功，查询校验和、工作载荷 Probe 构造/drop 计数及清理均核对通过。','',
      '| cgroup | 内核 peak MiB | 最大采样 current MiB | anon MiB | file MiB（含 shmem） | shmem MiB | local max |','| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for k,g in groups.items():
        s=g['peak_sample_stat'];lines.append(f'| {k} | {g["kernel_peak_bytes"]/2**20:.3f} | {g["max_sampled_current_bytes"]/2**20:.3f} | {s["anon"]/2**20:.3f} | {s["file"]/2**20:.3f} | {s["shmem"]/2**20:.3f} | {g["events"]["memory.events.local"]["max"]} |')
    lines+=['',f'pgbench 完成 {pgbench["transactions"]:,} 笔、0 失败，{pgbench["tps"]:.2f} TPS；Redis GET {requests["GET"]:,} 次、SET {requests["SET"]:,} 次，0 miss/eviction。受测 payload 构造/drop 均为 {final["created"]:,}，root_closed live {final["live_bytes"]:,} B，RSS {final["rss_kib"]/1024:.2f} MiB。','',
      '所有组 swap、OOM、OOM kill 均为 0；父 slice local max 为 0，父 events 的 max 来自 PG 子组回收压力。表中 anon/file/shmem 对应最大采样 current，不是内核峰值的瞬时分解；shmem 包含在 file 内。', '',
      '已独立确认本次两个容器、命名卷、活动 cgroup 和临时 systemd 配置均清理。scope 随应用正常退出可能先消失；slice 的默认 Loaded 状态可由 systemd 查询合成，清理以 inactive/dead、非 transient、无 ControlGroup/配置与 cgroup 路径不存在为准。','',
      '这是优化后版本的单次受限服务组适配验收，不是与旧探针的配对性能改善数据，也不是实际 2 GiB 云机、完整 HTTP/DB 业务调用、长期运行或 Redis 持久化验收。父组 1536 MiB/2 CPU，app/PG/Redis 512/640/256 MiB，均禁用 swap；宿主 daemon/系统等在组外。']
    (d/'SUMMARY.md').write_text('\n'.join(lines)+'\n');print(d/'SUMMARY.md')

if __name__=='__main__':main()

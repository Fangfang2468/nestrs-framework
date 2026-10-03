# DI 哈希表更换基准

这份业务源码用于对比修改前后 core 的真实服务查询。它不增加 workspace member、
生产 feature 或公开内部 API；每个版本通过 `prepare.py` 指向对应 core，生成独立项目。
务必使用同一源码、编译器、Release 配置和 Nestrs driver 构建两个版本。

当前 `runner.py` 只接受一个 `--cli`，after 固定使用脚本所在工作区的 core。
因此两版 core 必须与该 CLI 的执行协议兼容。2026-10-01 的历史基线使用旧 v1，
当前工作区使用 v2，不能直接用当前工具重建旧的 `target/di-ahash/baseline`。
复现原测量须恢复当时的 before/after 源码及匹配工具；跨 ABI 比较改用支持两套
工具的[直接输入基准](../bench-direct-input/README.md)。

准备新的同协议对照时，下面的 baseline 必须是本次优化前的完整快照，CLI 必须
同时适配两版 core；这些路径需替换成实际路径：

```bash
python3 tools/bench-di-ahash/runner.py build \
  --baseline-root /absolute/path/same-abi-baseline \
  --cli /absolute/path/matching-tools/debug/cargo-nestrs \
  --output target/di-ahash/new-comparison
```

runner 会清理 Rust flags、wrapper、target 与 Release 环境覆盖，但保留
`NESTRS_RUSTC`、`NESTRS_DRIVER` 和 `NESTRS_MACRO_BRIDGE`。复现时应清除不需要的
工具路径覆盖，或确保它们指向上述匹配组合；以 `build.json` 的 doctor 输出核实实际
使用的工具，不能仅凭 `--cli` 路径判断 driver/bridge 身份。

CLI 会把真实程序写入带 compiler/driver 指纹的 target 子目录。使用构建输出中的
`executable`（可加 Cargo 的 `--message-format=json`）定位二进制。不要计时
`cargo run`，因为构建和工具链启动不属于查询耗时。

每次运行只测一个场景，并输出一行 JSON：

```bash
"$BENCH_BINARY" --scenario warm-singleton --runtime current-thread \
  --iterations 100000 --warmup 2000
```

| 场景 | 计时中的工作 |
| --- | --- |
| `warm-singleton` | 反复调用公开具体类型查询方法，命中已经构造的 Singleton |
| `keyed-trait` | 带字符串 key 的 trait 查询，包含 key 创建、路由查找、缓存和真实类型投影 |
| `scoped-cache` | 同一个 scope 内查询已经构造的 Scoped 服务，依赖一个 Singleton |
| `transient-fanin` | 每次查询构造一个 Transient 消费者与三个独立 Transient 输入实例 |

`--runtime` 支持 `current-thread` 和 `multi-thread`；后者固定两个 Tokio worker。
`--iterations` 与 `--warmup` 必须为正整数。所有场景在计时之外构建 root、预热和关闭；
瞬时服务每 512 次查询完整关闭一个 scope，计时只累计各批的查询循环，scope 创建和
关闭不在计时范围。新 scope 会在计时前完成一次无关 Singleton 查询，以确认协调器
已处理其注册命令。这是有界的构造负载，不是容器完整生命周期基准。每次查询读取
业务值并经过 `black_box`，最终校验 checksum，防止把空循环当成服务查询。

建议先完成全部编译与测试，再单独测量。多个进程按交替顺序运行 before/after，
重复至少 15 次，记录原始 JSON、CPU/系统/工具链、CPU affinity、环境变量和源码 hash。
报告各版本的中位数及分布；哈希表微基准与上述端到端结果分开呈现。不要把一次结果、
编译时间或哈希算法局部加速直接描述为整个 DI 的加速。

## 统一执行与复现

`runner.py` 依赖 Python 3.11+ 标准库。基线必须包含 workspace Cargo.toml /
Cargo.lock，且只隔离待测优化，不能把其他架构变化算作哈希收益。固定 CPU 选项使用
Python 的 `os.sched_setaffinity`；以下命令的 CPU 编号需按本机拓扑调整，未配置 affinity 时省略选项。
现有性能证据来自 Linux/WSL2，脚本没有经过 Windows 原生验收。

```bash
# 与前面 build 使用同一个输出目录；构建和其他测试全部结束后再测量。
python3 tools/bench-di-ahash/runner.py measure \
  --output target/di-ahash/new-comparison \
  --rounds 20 --single-cpu 2 --multi-cpus 2,4,6
python3 tools/bench-di-ahash/runner.py summarize \
  --output target/di-ahash/new-comparison
```

只重新汇总本机原始 2026-10-01 样本时，执行
`python3 tools/bench-di-ahash/runner.py summarize --output target/di-ahash/benchmark`；
它读取已有样本并重写汇总，不重新编译或运行。原始快照和数据不随仓库分发。

runner 串行构建两个真实 DI 二进制和一个微基准二进制；两个 DI 项目继承各自快照
的 Cargo.lock，并检查旧 registry package 的版本、来源和 checksum 一致。保留
默认 Release（opt-level 3、16 codegen units、无 LTO），不启用 `target-cpu=native`
或额外 AES feature；清理可能改变目标/Release 的环境覆盖。Rust/Cargo 的配置文件
仍应由复测者核对。微基准在同一个二进制中切换标准库与 ahash 的 `RandomState`。

每轮依次运行一对 before/after，下一轮顺序反转；每个进程都预热并使用自己的
随机哈希种子。默认 20 轮，不剔除离群值。Linux affinity 由调用线程及 Tokio workers
继承，多线程场景共享指定 CPU mask，不逐线程绑定独立物理核。

微基准的 `usize-lookup` 和 `route-lookup` 使用 1024 项预建表，后者复制 core 的
真实键形状，但不直接调用 core 私有类型。`parent-churn` 保持 1024 项集合并反复
删除/插入，每次 iteration 计为两个操作；哈希器之外两边使用相同的标准库容器。
它们不包含字符串分配、建表或 Tokio 调度。

默认产物位于 `target/di-ahash/benchmark/`：

- `build.json`：构建命令、工具链、源码/二进制 hash、两个 DI 项目的 lock。
- `measurement.json`：机器、CPU affinity、迭代次数和实际测量命令。
- `samples.jsonl`：全部原始样本，每行一个进程的平均 ns/op。
- `summary.json` / `RESULTS.md`：两组中位数、P25–P75、下降比例与配对统计。
- `build-*.log` 与 `apps/`：完整编译输出和可独立复建的项目。

“中位耗时下降”计算为 `1 - median(after) / median(before)`；配对统计先对每一对
计算 `1 - after_i / before_i`，再求中位数并用 10,000 次重采样估计 95% bootstrap
区间。这是两个不同的统计量，表中分别列出。P25–P75 是进程平均值的分布，不是
单个请求尾延迟。区间跨过零表示这次采样不能稳定证明加速；单机 WSL 结果也不能
推导所有平台、业务负载或高并发吞吐。完整结果见
[性能与内存](../../docs/NESTRS_PERFORMANCE.md)。

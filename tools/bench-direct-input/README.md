# 直接构造输入的内存对比

这套独立业务 fixture 通过两版各自匹配的 `cargo-nestrs`、driver 和私有 bridge 编译。它只使用业务公开 API，不在生产 core 中增加测试开关，不把基准加入正常 workspace。

当前 runner 面向 Linux，需要 Python 3.11+、`/proc`、`/usr/bin/time`，指定 CPU
时通过 Python 的 `os.sched_setaffinity` 绑定；构建审计使用 Linux bridge 文件名 `.so`。框架支持 Windows host
不代表这个测量脚本已跨平台。已记录的数字与结论统一见
[性能与内存](../../docs/NESTRS_PERFORMANCE.md)。

## 复现

先保存**实际工作区**基线（包含开始任务前已有的未提交修改），不要拿 Git HEAD 代替。基线目录必须包含完整 workspace 和 Cargo.lock。分别在修改前与修改后的源码目录构建工具链；两边使用同一个受支持的 rustc。

```bash
# 在基线目录执行；目标目录应使用绝对路径。
CARGO_TARGET_DIR=/absolute/path/before-tools python3 tools/build-toolchain.py

# 在修改后的 workspace 执行。
python3 tools/build-toolchain.py
python3 tools/bench-direct-input/runner.py build \
  --baseline-root /absolute/path/baseline \
  --before-cli /absolute/path/before-tools/debug/cargo-nestrs \
  --after-cli /absolute/path/current/target/debug/cargo-nestrs \
  --output target/direct-input/new-comparison

# 关闭其他构建、测试和重负载，再采样；--cpu 必须属于当前进程允许的 CPU 集。
python3 tools/bench-direct-input/runner.py measure --output target/direct-input/new-comparison \
  --rounds 20 --batches 64 --batch-size 32 --cpu 2
# 额外的长期释放检查：每场景/每版本执行 65,536 次查询。
python3 tools/bench-direct-input/runner.py stress --output target/direct-input/new-comparison \
  --batches 128 --batch-size 512 --cpu 2
python3 tools/bench-direct-input/runner.py summarize --output target/direct-input/new-comparison
```

`--output` 可以指定独立输出目录，新的比较应显式指定，以保留已有样本。`build --variant before` 和 `build --variant after` 支持分阶段构建；`--scenarios concrete4,mixed` 可以缩小采样范围。默认输出位于 `target/direct-input-20261002/benchmark/`。Release 配置固定为 opt-level 3、无 LTO、16 codegen units；不使用 native CPU 优化。`--batches` 的有效范围是 1–128，`--batch-size` 必须大于零；runner 默认是 8 轮、32 批、每批 32 次查询，上面显式选取正式测量规模。

构建前按 workspace 锁定的版本生成 fixture 锁文件并验证全部 registry 包的 source/version/checksum 一致，实际构建使用 `--offline --locked`。两版业务源码 SHA-256 必须相同。`build.json` 保存工具、可执行程序、core、codegen/driver 源码的 SHA-256，以及完整构建命令、rustc 和 doctor 输出。

## 场景与测量边界

场景包含无输入、1/4 个 concrete、trait、已存在 optional、缺席 optional、已访问 lazy、未访问 lazy、0/1/4 参数同步/异步 factory，以及 8 输入的 Transient、Scoped 和已缓存 Singleton。异步 factory 的参数真实跨越一次 `yield_now().await`，验证输入引用在挂起之后仍然可用。不同输入场景可以通过 1 → 4 的差值观察边际成本；它包含整个容器执行路径，不是孤立的输入转换微基准。

所有 concrete/trait/已存在 optional 输入都指向默认 Singleton `Base`；lazy 输入指向
Singleton `LazyBase`。访问 lazy 的场景在预热阶段已经构造目标，所以查询窗口中的
“首次访问”是新消费者的 lazy 句柄第一次 `get`，不包含目标 Singleton 冷构造。
未访问场景则始终不通过这些句柄请求目标。

每个样本是一个新进程，使用 current-thread Tokio。先执行 8 个有界预热批次，再进行指定数量的测量批次。每批创建一个 scope，查询之后显式关闭，等待协调器处理完成；Transient 因此不会无限堆积。Scoped 在同一批次中复用，Singleton 对照则不重复构造。

计数 allocator 委托 `System`，正确处理 `alloc`、`alloc_zeroed`、`dealloc`、成功/失败 `realloc`。`realloc` 成功时累计请求大小计入新大小、累计释放大小计入旧大小，当前存活字节更新为两者之差；失败时保持旧分配。统计本身只使用原子计数，不分配内存。启动前自检验证零初始化、数据保留、realloc 以及分配/释放后的 live 数量恢复。

- `allocation_calls`：成功 alloc/alloc_zeroed 次数；realloc 单独列出。
- `allocated_bytes`：累计向 allocator 请求的字节数，**不是存活内存**。
- `peak_live_bytes`：查询窗口中全进程 allocator 存活请求字节峰值，包含测量前已存在的对象。
- `peak_live_extra_bytes`：单批查询期间，相对该批开始时的 allocator 存活请求字节峰值；跨批取最大值。
- `steady_start_live_bytes` / `steady_end_live_bytes`：整个测量区间前后仍存活的请求字节。
- `checkpoints`：每批 scope 关闭后记录 live 请求字节与 Linux VmRSS，用于检查重复循环后的稳定性。
- `root_closed_live_bytes`：最终 root 关闭后的请求字节；进程全局冻结计划、Tokio runtime、命令行参数等仍存在，不能要求该值为零。
- `rss_start_kib` / `rss_end_kib`：Linux `/proc/self/status` 中的进程常驻集。
- `process_vm_hwm_kib`：最终关闭后读取 `/proc/self/status` 的 VmHWM；报告主表使用此进程常驻集高水位。
- `process_peak_rss_kib`：`/usr/bin/time` 取得整个进程生命周期的峰值常驻集。

查询窗口不包含 scope 创建/关闭和读取 `/proc` 的开销；关闭后的存活检查另行记录。请求字节统计不包含分配器头部、块大小取整、线程栈、代码映射和共享库。RSS 包含这些影响，不能用其中一个指标代替另一个。Linux procfs 与 getrusage 的计数时点和近似记账可能产生小幅差异，因此 VmHWM 与外部 time 的峰值同时保留，不能把几十 KiB 的差值当成精确堆内存变化。

被查询的消费者或 factory 返回服务包含一个不分配的 `Probe`：构造与 Drop 分别递增计数。`Base` / `LazyBase` 不带 Probe，计数不代表容器内全部实例数。每个样本验证这些被查询服务的精确构造数量、关闭后的 Drop 数量，以及预期业务 checksum；两版同轮结果也必须一致。匹配的计数和稳定检查点只能支持这些有界工作负载没有观察到残留增长，不能证明全部路径无泄漏。

交替执行 before/after 与 after/before，不删除离群样本。`samples.jsonl` 保留每个进程的原始值和所有检查点；`summary.json` 保留最小值、P25、中位数、P75、最大值。计时是在启用原子计数 allocator 的条件下获得，**只作带仪器的耗时观察，不据此宣称生产吞吐量提升**。

## 保留的证据

- `build.json`：两版 fixture/binary/core/编译器源码/CLI/driver/bridge 哈希、registry 锁定版本、构建命令与工具链身份。
- `build-before.log`、`build-after.log`：真实工具链 Release 构建输出。
- `measurement.json`：正式采样环境、CPU affinity、轮数与批次参数。
- `samples.jsonl`：交替运行的全部进程原始结果，每个样本包含所有批次关闭检查点。
- `summary.json`、`marginal-inputs.json`、`RESULTS.md`：分布与 1 → 4 输入的边际成本。
- `retention-stress-config.json`、`retention-stress.jsonl`：独立长循环原始记录。默认 stress 场景为 mixed/lazy4/async4；上面的命令每种情况执行 128 个批次，每批 512 次查询。
- `allocator-self-test.json`、`retention-allocator-self-test.json`：成功进程数量、计数器启动自检断言与 fixture 哈希。每个成功进程都实际执行自检，任何断言失败会使采样终止。没有强行耗尽内存来注入 OOM。

2026-10-02 的本机测量另外保留了首次测量暴露容量问题的 `intermediate-before-capacity-fix/`、槽位精确容量修复后的 `intermediate-after-slot-capacity-fix/`，以及定位存活重叠的 `allocation-trace/`。这些是中间诊断证据，不参与最终统计。allocation-trace 在独立 fixture 副本里用固定数组记录分配/释放事件，正式 fixture 没有启用它。

# Core 内存与关闭开销基准

本工具把同一业务探针分别链接到 exact worktree 的 before/after core，另提供显式手动组装基线。临时应用、冻结编译器工件、原始采样和汇总写到 `target/core-memory-20261002/`；不修改生产 core、系统分配器配置或全局 Rust 工具链，不联网更新依赖。

## 测量口径

两个 Tokio worker，CPU affinity 默认 `2,4`，每轮新进程，release opt-level 3、无 LTO、16 codegen units。每个进程有 3 波预热；每波创建并发 scope、每 scope 查询 8 次，barrier 确保所有服务同时保活，然后采样，放行并等待全部 scope 关闭。Transient 实例保留到 scope 关闭，Scoped/payload 每 scope 复用一个实例。共同业务计数器检查构造/drop 数、每波与最终校验和；sparse 同时检查清理回调数。

构造/drop 计数只覆盖含 `Probe` 的业务工作载荷实例；根共享的 `Base` / `LazyBase` 不计入该计数，不能将它表述为所有 provider 的总构造数。

`GlobalAlloc` 包装 `System`，统计成功 alloc/alloc_zeroed、成功 realloc、dealloc、请求/释放字节和 live。realloc 按完整新请求/旧释放累计，live 使用一个原子增量。启动自检覆盖 zeroed、realloc 数据保持及准确计数。统计不是 allocator metadata、映射或 RSS；全局原子计数影响吞吐，不能把结果当作未插桩业务 QPS。

每波输出 held/closed JSON：分配计数累计自该波开始，关闭行覆盖该波完整周期；held live/RSS 在所有实例同时存在时读取；query_ns 从创建请求任务至全部查询结束，close_ns 从放行至全部关闭任务返回。关闭耗时包含调度与回调，不含 hold 和随后根查询屏障。root_closed 单独记录 root 关闭耗时及 20 ms 后 retained。`/proc/self/status` 用已打开文件和栈缓冲读取，暖机后采样不创建临时 heap buffer。Linux RSS 是近似采样，最高进程 RSS 另由 `/usr/bin/time` 记录。

进程 CPU 使用父进程 `getrusage(RUSAGE_CHILDREN)` 的 user/system 差值，包含本次程序启动、预热、测量、关闭及 `/usr/bin/time` / timeout 小量开销；是进程总量，不是只查询区间 CPU。每个样本用 timeout 管理自己的进程组，180 s 后停止，5 s 后仍未结束则强制终止该测试组。活跃 throughput 用查询数除以 query_ns+close_ns，排除显式 hold 与采样；报告中同时保留整个进程 elapsed，不能混作 HTTP 吞吐。

DI 的每波 settle 会额外查询一次根 Base，manual 为 black_box+yield；这段在完整分配统计与进程 CPU/elapsed 内，在 active query+close 计时外。容量维护也可能发生在该段，所以 active 吞吐必须联合完整进程 CPU/elapsed 阅读。不能把 B/query（每 scope 8 次）写成 B/构造实例；更不能与旧探针每 scope 16 次、mixed512 持有 8192 个实例的结果直接当作前后配对。

## 场景

| case | 同时 scope | 查询/范围 | 目的 |
| --- | ---: | --- | --- |
| mixed64 / mixed512 | 64 / 512 | Transient，concrete、trait、optional、absent、2 个 lazy 字段 | 小实例并发常态与突发 |
| scoped64 | 64 | 同构字段，但每 scope 复用 | Scoped 缓存与 owner |
| lazy64 | 64 | Transient，4 个 lazy 字段 | Lazy 分配与首次获取 |
| async64 | 64 | 异步 factory，4 个普通输入，yield 一次 | 工厂与任务展开 |
| payload256 | 256 | 每 scope 1 MiB，实际填充并检查首尾字节 | 256 MiB 业务载荷及峰后 RSS |
| sparse-small / sparse-large | 64 | 每 scope 只有 consumer+lazy leaf，均有 async cleanup | 9 / 1033 节点计划中的稀疏关闭 |
| burst-recovery | 首波 512，随后 1 | 测量首波大突发，随后 1199 波低负载 | 多个事件驱动容量维护窗口后的 retained |

`large-graph` 在同一源码中加入 1024 个未激活 provider，构建时保存真实 reflection sidecar 并断言 1033 个节点；不是在基准器内虚构大数组。sparse 每 scope 仍只创建两个被跟踪实例。该用例专门反映按全图关闭排序时共享工作区的分配差异。burst 的大波发生于预热后的第一个测量波；正式数据包括它，详细逐波轨迹用来观察峰值回落。

## Manual 基线边界

手动组装使用 `Arc`、`Option`、每字段 `OnceLock<Arc<_>>`、显式异步工厂和 scope 内的类型化保活容器；与 DI 使用相同业务常量、并发数、查询数、payload、构造/drop 数、同时保活时点与异步 cleanup 工作。Transient 保留到 scope 结束，不通过每次查询立即释放来偷减 held 内存。

它没有冻结图查路、中央构造名额、任务/等待订阅、失败缓存、取消后继续初始化、弱 owner Lazy 协议、精确投影与 lease、通用图关闭顺序等契约。字段布局也不同（Arc/OnceLock 与 Injection/LazyInjection）。因此差值是**完整 DI 实现与该显式组装方案的工程成本差异**，不能称为“纯 core 净开销”或语义等价替代品。手动 large 场景复用小二进制，因为手动组装没有运行期的未请求注册图；图成本主要应比较 DI before/after 的同一 large 用例。

它也是一份具体实现，scope 保留了不同场景的未用字段等统一驱动结构；不能称为数学意义上的最小内存下界。

## 构建与运行

先由任务保存实际工作区快照到 baseline；不要使用可能缺少现有修改的 Git HEAD。优化稳定后以同样方式保存 after。CLI/driver/bridge 必须属于相同固定 rustc，工具首次调用复制并固定它们，记录二进制 SHA-256、实际 rustc 完整身份、生产源码哈希、probe 源码哈希、lock 与 registry package version/checksum。

```bash
python3 tools/bench-core-memory/runner.py build --variant before
python3 tools/bench-core-memory/runner.py build --variant manual
python3 tools/bench-core-memory/runner.py build --variant after \
  --after-root target/core-memory-20261002/after \
  --cli target/core-memory-20261002/toolchain/cargo-nestrs
python3 tools/bench-core-memory/runner.py measure --rounds 7 --measurement paired
python3 tools/bench-core-memory/runner.py summarize --measurement paired
```

构建步骤顺序执行，避免多个构建同时写 `build.json`。每轮按变体轮换次序，配对 before/after 使用相同参数；正式计时期间不要并行运行编译、其他基准或数据库压力。`measure` 主动拒绝二进制、probe、registry 或固定工具不一致的结果。修改 probe（包括 rustfmt）后重编所有变体。

仅功能烟测可执行完整三变体检查，其计时不进入正式性能结论：

```bash
python3 tools/bench-core-memory/runner.py measure \
  --variants before,after,manual --rounds 1 --measurement smoke
```

`--measurement` 指定的采样子目录、colocated 的 `--output` 目录必须尚不存在；重复
实验使用新的名称，保留旧证据。runner 的全局 `--output` 应放在 `build`／`measure`／
`summarize` 子命令之前。正式 payload 至少 3 次独立进程；默认所有用例 7 次。
汇总给出中位数、四分位、范围和配对变化 bootstrap 95% 区间，跨零不得称为稳定改善。

## 同机容量复验

完成前后计时后，再单独使用容量脚本执行两核、1536 MiB 服务 slice，app/PG/Redis 为 512/640/256 MiB、swap 0。脚本要求 Linux cgroup v2、systemd 与 Docker，镜像必须已经存在；不会下载镜像、暴露宿主端口或改既有服务。复用 PG 18.4 和 Redis 8.6.3 缓存模式、数据集、并发/随机键配置，数据库压力 60 s、应用 3 波预热+700 波测量、每波 hold 100 ms：

```bash
python3 tools/bench-core-memory/colocated.py \
  --app-binary target/core-memory-20261002/bin/after-small \
  --output target/core-memory-20261002/colocated-after \
  --seconds 60 --app-scenario payload --concurrency 256 \
  --queries-per-scope 8 --payload-bytes 1048576 --hold-ms 100
python3 tools/bench-core-memory/summarize_colocated.py \
  target/core-memory-20261002/colocated-after
```

同机压力不得与正式 paired 计时并行。保留 `memory.events.local` 与层级 events、anon/file/shmem、所有工作量/退出结果和 owned 资源删除证据。它验证受限服务组，不等于真实 2 GiB 整机、业务端到端或 Redis 持久化验收；单次 after 适配复验也不是配对性能改善证据。

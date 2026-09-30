# Core 内部可读性重构验收

日期：2026-09-29。固定编译器为 Rust 1.98.0，commit
`88d9e12ae178fab0fb5cc050a94da85685d449ea`。

本记录对应图编译分阶段、共享缓存与活跃任务分离、显式关闭状态、输入协议整理、
迭代释放分层、错误模块提取及中文源码注释。阅读路线见
[core 内部阅读指南](NESTRS_CORE_INTERNALS.md)。此前移除 linkme / 公开内部 ABI 的
迁移证据保留在[编译器注册清单验收](NESTRS_COMPILER_REGISTRY_VALIDATION.md)，不以
旧测试结果替代本次重构验证。

## 1. 行为与测试边界

- 公开门面、查询宏、业务声明语法和 core/bootstrap 分层不变。
- 完成的任务移出活跃任务表；Singleton/Scoped 的成功 lease 或失败记录独立缓存。
  新增两项回归覆盖多等待者、跨 scope 合并/隔离、任务退役、journal 发布与失败不重试。
- 原 activation 的 28 个测试及 53 个断言调用保留；测试实现随后按维护者要求统一
  移入 crate 根 `tests/unit/`，源码中只保留测试模块挂载声明。
- 删除未被图读取的单值 BoundKeyPolicy。对应测试改为验证真实 named/indexed key
  路由、同一 Provider 投影和不跨 key 回退，不再只比较固定策略标签。
- 深链图、失败路径、构造、关闭、逃逸 lease 与最终释放仍使用非递归路径；既有小栈
  回归继续执行。本轮没有以吞吐量或延迟量测作性能提升结论。

## 2. Linux 已执行验证

| 范围 | 结果 |
| --- | --- |
| core 单元测试 | 91 项通过，含新增的两项共享缓存退役回归 |
| cargo nestrs check --workspace --all-targets --locked | 通过 |
| cargo nestrs test --workspace --locked | 通过；core 91、工具库 95、CLI 28、帮助 12、示例业务 3、示例 CLI 7 |
| cargo-nestrs compiler-driver --all-targets | 通过；driver 16、11 组内部 core 契约、native host、reachability、registry ABI 4 和 rustdoc 2 均执行 |
| 完整 DI fixture | 通过，包含 59 个宏/UI 编译契约及取消、关闭、初始化等运行期回归 |
| 跨 crate 验证脚本 | 22 条 check/run/graph 命令通过，覆盖 Debug、Release、key、泛型、私有实现及歧义 |
| 图导出验证脚本 | 25 条命令通过，覆盖输入边、无副作用、多入口、诊断、特性选择和输出保留 |
| 默认成员与 compiler-driver Clippy | -D warnings 通过 |
| core 私有文档构建 | RUSTDOCFLAGS=-D warnings，document-private-items 通过 |
| 格式与差异检查 | cargo fmt --check、git diff --check 通过 |

真实 checkout 分别运行 Lazy、Eager + scope warm-up + 构造并发上限 1，以及单笔
`place-order`。两种 sample 模式都得到 2 笔成功订单、剩余库存 2 件、597.00 元成交
金额和 4 条审计，完成 scope 与 root 关闭。项目 HTML 报告为 1 valid、0 errors、
0 skipped。

普通 Cargo 检查的是默认 core/工具成员；应用 workspace 验证使用 cargo nestrs。
文档中原有 ignore 示例不算作执行通过；可执行文档由独立 rustdoc fixture 验证。

## 3. 验收发现并修复的文档入口缺口

Windows 最初在间接依赖 core 的 consumer doctest 中出现 `__nestrs_registry_v1`
未解析符号。旧 driver 只为直接 --extern nestrs_core 的文档设置完整 builder，间接
依赖场景绕过了注册入口生成；core 模块拆分改变目标代码布局后暴露了这个既有缺口。

回归现已让 producer 封装真实的 build、服务查询和 dispose，再由没有 core 直接依赖
的 consumer 在普通 main 与 doctest 中调用。旧工具在 Linux 上也稳定链接失败，修复后
通过，因此不依赖特定平台的链接裁剪行为来判断正确性。

修复让所有 Nestrs doctest 使用 driver builder，并保证没有直接 core 依赖的文档源分析
也进入完整流程。文档载体按 lib 交给 rustdoc，真实源码仍按原 crate-type 检查；snippet
通过原 --extern 加载真实工件。过程宏 crate 不会因载体中的普通 module 导出而报错。

修复后 Linux 重跑 bridge_metadata、rustdoc 两个入口和整个 workspace 的文档检查。
工具 crate 的 266 个 ignore、core 的 1 个 ignore 仍保留，私有 bridge 的文档编译成功。
没有给 consumer 补上伪造的直接 core 依赖，没有导出新的 core 内部接口。

## 4. Windows 原生验证

使用固定 1.98.0 MSVC 工具链和隔离的源码副本执行，未修改全局默认 Rust、PATH 或
PATHEXT。core 的 91 项测试通过；11 组内部 core 契约、registry ABI 4 项、reachability、
driver 16 项、工具库 94 项均实际执行成功。

checkout 完成 Lazy、Eager、scope 预热与并发上限 1 的运行，3 项业务测试、7 项 CLI
测试和 HTML 导出通过。完整 DI fixture 的 13 个测试入口通过，其中包含 59 个 UI 契约。

初次 compiler-driver 全 targets 因上节的 bridge_metadata 链接错误退出 101，记录未
删除。随后先单独执行被 fail-fast 遮住的契约、ABI、reachability 和 rustdoc，再同步
8 个文档入口修复/fixture 文件并重建。修复后 bridge_metadata、driver、help 12 项、
native_host、rustdoc 2 项及 workspace --doc --locked 全部通过。

native_host 实际覆盖带空格路径的构建、运行、图导出和 IDE 项目记录刷新；它不等同于
交互式编辑器验收。Unix 专属的 tests/cli.rs 在 Windows 为 0 项，不计为 Windows
执行通过。最终没有重新执行整条 all-targets 命令，而是按受影响目标完成复验。
日志共记录 19 条命令：18 条退出 0，1 条为已修复并专项覆盖的原始失败。

## 5. 验证证据

Linux 日志与结构化报告在 `target/core-refactor-validation/`。其中 `toolchain-tests.log`
保留测试迁移中途的字段语法错误，修复后全套结果为 `toolchain-tests-final.log`。
`docfix-before.log` / `docfix-after.log` 保留文档入口问题的前后对照；
`docfix-workspace-doc-final.log` 与 `docfix-integration.log` 记录其最终专项复验。

Windows 原生证据独立放在 `target/windows-core-refactor-validation/`，不混用此前
registry 迁移的 Windows 日志。target 文件可能被清理，关键结果应以本文记录为准。
本轮未重复浏览器视觉或交互式 rust-analyzer 验收，没有扩大其既有支持边界。

后续按维护者要求整理测试目录：13 个测试文件逐字节迁移到 `nestrs-core/tests/unit/`，
另一个内联测试模块只移除外层包装与缩进；91 个测试名称和 11 组编译器契约正文保持。
目录迁移后在 Linux 重新通过 core 测试、all-targets check、严格 Clippy、格式检查和
全部编译器契约，并通过现有 init check 命令检查 workspace 所有目标、刷新当前 IDE
模型。该次目录整理没有额外重跑 Windows；证据在 `target/core-test-layout/`。

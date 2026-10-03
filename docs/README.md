# 文档导航

每个主题保留一个主要入口。初次使用从[项目 README](../README.md)和声明指南开始；
维护实现时按下面的编译期、运行期边界阅读。

用法与设计文档描述当前实现；[修复记录](NESTRS_FIXES.md)按轮次保存缺陷、原因、
修复方式及当时的验证范围；[性能与内存](NESTRS_PERFORMANCE.md)保存有基线的历史测量。
阅读历史结论时应同时核对对应快照、工具链和平台，不能将其视为后续版本的自动验收。

| 文档 | 回答的问题 |
| --- | --- |
| [服务声明与查询](NESTRS_MACROS.md) | injectable、constructor、factory、注入、key、trait、泛型、lazy 和跨 crate 怎么用？ |
| [core 设计与架构](../nestrs-core/README.md) | root / scope 如何按配置创建与初始化？冻结计划、构造输入、实例所有权、并发、取消与关闭如何协作？ |
| [Cargo 工具链](NESTRS_CARGO_TOOLCHAIN.md) | 如何构建、检查、运行、测试、导图？配置、缓存、平台和回归边界是什么？ |
| [IDE 接入](NESTRS_IDE.md) | 如何配置原版 rust-analyzer？项目模型、保存检查和 constructor 补全如何工作？ |
| [rustc 扩展与集成](NESTRS_RUSTC_EXTENSION_GUIDE.md) | 宏、HIR、Ty/DefId、MIR 和跨 crate 摘要怎样生成执行计划？reflect 产物是什么？ |
| [诊断格式](NESTRS_DIAGNOSTICS_DESIGN.md) | 编译错误如何指向用户源码？错误代码、依赖链与 cause 怎样阅读？ |
| [性能与内存](NESTRS_PERFORMANCE.md) | 哪些优化经过实际测量？基线、原始数据和结论的适用范围是什么？ |
| [修复记录](NESTRS_FIXES.md) | 各轮具体修复了什么？为何前一轮回归没有覆盖后续缺陷？正式回归在哪里？ |

## 示例与测试

- [示例目录](../example/README.md)：正常业务项目与预期失败项目的使用入口。
- [电商 DI 示例](../example/di-checkout/README.md)：完整业务、生命周期和关闭流程。
- [依赖错误示例](../example/di-errors/README.md)：每类错误的原因、修复方向与独立项目。
- [core 测试职责](../nestrs-core/tests/README.md)：运行期行为、内存所有权和内部执行边界。
- [工具 fixture 索引](../cargo-nestrs/tests/fixtures/README.md)：真实 driver、宏、查询根、跨 crate 和诊断契约。
- [rustc 语义探针](../tools/compiler-probe/README.md)：基础能力实验及历史脚本的适用限制。
- [哈希表基准](../tools/bench-di-ahash/README.md)与[构造输入内存基准](../tools/bench-direct-input/README.md)：测量工具的使用与指标定义。
- [core 内存与关闭基准](../tools/bench-core-memory/README.md)：存活堆、分配流量、关闭成本与受限同机实验的口径。

## 维护约定

[AGENTS.md](../AGENTS.md) 集中保存贡献规范、已确认的架构边界和后续范围。
实现变更同步更新对应主题文档；不再为每轮重构建立独立方案与验收主文档。
同一规则只在负责该主题的文档完整解释，其他入口用简述与链接引导；例如声明语法归声明
指南，运行期所有权归 core，查询根和计划生成归 rustc 指南，命令与配置归工具链指南。
多轮修复统一追加到修复记录，保留每轮事实及后续补充，不用最后一次成功覆盖之前的遗漏。
未来设计必须标明尚未实现，历史测量必须保留日期与基线，不能写成当前版本的普遍保证。
临时核查、构建日志和原始测量放在 `target/`；它们不随仓库分发。修复记录应在正文保存
可独立理解的原因、改动与回归入口，即使清理 target 也能追溯实现演进。

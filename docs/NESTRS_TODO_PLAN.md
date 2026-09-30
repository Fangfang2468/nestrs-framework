# Nestrs 后续待办与讨论决策

更新日期：2026-09-30。

本文固定本次关于编译器注册清单、两个工具二进制及工具链分发的讨论，并记录已确认的
bootstrap / create 后续方向。它是后续会话的待办入口，不是已交付功能的使用说明。
当前命令见 [Cargo 工具链说明](NESTRS_CARGO_TOOLCHAIN.md)，已完成迁移的证据见
[编译器注册清单迁移验收](NESTRS_COMPILER_REGISTRY_VALIDATION.md)。

## 1. 状态与使用规则

- `[x]`：已有实现与验收记录；后续仍应按所修改的范围做回归。
- `[ ] 待实施`：尚未实现；条目包含建议的完成标准，不表示已有可运行命令。
- `[ ] 待决策`：讨论中提出的方案，尚未由维护者最终选定。
- `[ ] 保留后续范围`：已知的后续能力或验证边界，尚未排入当前实施阶段。
- 分发与生态条目只固化计划，不启动下载器、安装器、发布流水线或 bootstrap 的实现。
  记录发布任务也不等于授权上传工件、发布版本或创建 Git commit。
- 完成一项后保留编号，勾选状态，补充实现位置与实际验收记录。失败、跳过和未验证
  的 host 必须单列，不能只因存在 CI 配置或本机能构建就标记分发完成。

## 2. 已确认的架构与体验

| 事项 | 结论 |
| --- | --- |
| core / bootstrap | 保持分层，不合并。core 负责 DI；未来 bootstrap 负责 Application、配置和生态组合，导出 NestrsFactory |
| 注册收集 | 已移除 linkme 与公开 __private，使用编译器生成的每入口注册清单；不恢复链接段扫描或公开内部 ABI 模块 |
| 用户依赖 | 应用只依赖 core 与业务库；声明工具由 cargo nestrs 提供，不恢复公开宏 package |
| 工具组件 | 当前保留 cargo-nestrs 用户入口、nestrs-driver 编译入口和私有 proc-macro bridge；使用者只操作 cargo nestrs |
| 安装体验 | 目标是一次完整安装，由工具管理配套组件，不要求使用者分别构建、寻找和安装两个 bin 与 bridge |
| 项目 init | 用于已有项目接入和开发配置刷新；当前不负责安装机器级工具链 |
| 未来 create | 在 bootstrap 完成后创建已初始化项目，复用 init 的能力；用户无需在 create 后再执行 init |

两个 bin 属于同一个 Cargo package，拆分源于 CLI 与 rustc 内部适配的职责、启动条件
不同，并非 Cargo 强制要求。保留拆分也可以实现一次安装，不应为了减少用户操作
直接合并编译器与 CLI 的入口。

## 3. 已完成的基线

- [x] **BASE-01：编译器注册清单。** 汇总本 crate 与上游 metadata，生成版本化
  registry 入口；core 消费本次构图的快照。保留 provider / trait binding 的职责区别。
- [x] **BASE-02：真实私有边界。** core 不再导出 __private；生成代码来源受编译器
  校验，普通源码不能访问内部接口或 registry 入口，Injection 字段保留原生隐私。
- [x] **BASE-03：行为与跨 crate 回归。** 保留图冻结、生命周期、Tokio 调度、lease、
  取消与关闭契约；跨 crate、私有状态、泛型、Debug / Release / fat LTO 已有验收。
- [x] **BASE-04：配套工具与示例。** 宏、完整 driver doctest、原版 rust-analyzer、
  HTML 图及真实 checkout 示例通过本轮记录的验收；Linux 与 Windows 范围分别记录。
- [x] **BASE-05：开发阶段整套构建。** tools/build-toolchain.py 能构建配套 CLI、
  driver 与 bridge。普通 CLI 构建默认不启用 compiler-driver feature。
- [x] **BASE-06：core 内部可读性。** 图编译按阶段组织，运行时分开活跃任务与共享
  缓存，关闭阶段显式化，实例/释放和错误职责分离；补充中文源码注释与
  [阅读指南](NESTRS_CORE_INTERNALS.md)。验证与关联文档入口修复见
  [重构验收记录](NESTRS_CORE_REFACTOR_VALIDATION.md)，公开 DI 契约保持。

- [x] **BASE-07：字段级延迟注入。** `#[inject] #[lazy]` 生成 `LazyInjection<T>`，
  通过 `get().await` 获取冻结图中的固定目标；保留 trait/key/optional/闭合泛型、取消后
  接续同一 occurrence、消费者优先 cleanup 及迭代内存释放。使用与限制见
  [宏使用说明](NESTRS_MACROS.md#33-用-lazy-推迟某个字段的依赖初始化)。Linux/WSL
  已验证 core 105 项、延迟字段与 cfg 8 项、编译器 11 组契约及 4 项私有 ABI 回归、
  原有 DI/UI、跨 crate 和 checkout 示例。该次验证不替代 Windows 实机验收。
  服务声明级 lazy override、factory 参数 lazy、透明同步代理和构造重入挂起协议
  尚未实现；task-local 只诊断当前构造 worker，不追踪业务派生任务的等待因果。

BASE-05 不是发行安装能力。当前仍要求先准备匹配编译器与组件，再从仓库构建；没有
已经交付的统一下载/安装入口，现有 native-host CI 也不是正式 release 发布流水线。
既有 tools/ 构建与验证脚本可继续用于框架开发和 CI，普通框架使用者不应被要求运行它们。

## 4. 分发方案：讨论建议与待定选择

推荐顺序为：**完整预编译工具包 → 统一安装入口 → 可选 Cargo 安装入口**。
这是本次提出的工程建议；安装入口形式、命令名称、目录布局和发布渠道尚未最终选定。

建议为当前两个已适配 host 分别产出压缩包：

```text
nestrs-<版本>-x86_64-unknown-linux-gnu.tar.gz
nestrs-<版本>-x86_64-pc-windows-msvc.zip
```

一个与现有查找逻辑兼容的候选布局为：

```text
nestrs/
├── cargo-nestrs                 # Windows 为 .exe
├── nestrs-driver                # Windows 为 .exe
├── libnestrs_tool_bridge.so     # Windows 为 nestrs_tool_bridge.dll
└── manifest.json               # 拟新增的发行清单
```

CLI、driver 和 bridge 应作为配套版本部署。固定 Rust sysroot 的获取另由安装流程管理；
上面的三份 Nestrs 工件本身不包含完整 Rust 工具链。

若未来以 Cargo 作为入口，候选体验是 `cargo install cargo-nestrs --locked` 安装 CLI，
再由拟议的 `cargo nestrs toolchain install` 获取配套工件和 Rust 组件。**这不是当前
可照抄执行的完整安装流程，toolchain install / setup 都尚未实现，名称也未定。**

Cargo 一次可以安装同一 package 的多个 bin，但它不会自动部署另一个 proc-macro
package 的动态库。当前默认 feature 只构建 CLI；即使启用 compiler-driver，也不能
把普通 cargo install 当成已覆盖 bridge、rustc-dev 与编辑器组件的完整安装。

## 5. 分发 TODO

下面的依赖与验收标准用于后续落地讨论，不额外改变当前 CLI 的安装行为。

| 编号 | 状态 | 工作 | 完成标准 / 依赖 |
| --- | --- | --- | --- |
| DIST-01 | [ ] 待决策 | 确定首版安装入口与工件契约 | 明确是否先交付平台安装器、是否首版包含 Cargo 入口；确定发布渠道、版本目录、清单字段及 Rust 组件获取方式 |
| DIST-02 | [ ] 待实施 | 为 Linux / Windows 生成完整预编译工具包 | 依赖 DIST-01；原生 release 构建包含 CLI、driver、bridge 和清单，记录实际支持的系统/链接器条件；搬离源码和构建机目录后可使用 |
| DIST-03 | [ ] 待实施 | 实现配套工件的识别与完整性校验 | 清单记录 Nestrs 版本、完整 rustc commit、host 和工件校验信息；缺失、损坏、不同 host 或不匹配组合明确失败；现有缓存指纹不充当发行完整性校验 |
| DIST-04 | [ ] 待实施 | 实现一次完整安装 | 依赖 DIST-02/03；自动部署工具与命令入口，检查并准备匹配的 Rust / rustc-dev，完整开发体验包含 rust-src 和过程宏服务器；保留全局默认 Rust，不让用户手动配对 driver 与 bridge |
| DIST-05 | [ ] 待实施 | 明确并实现升级、失败恢复和卸载规则 | 配套版本整体切换，失败不留下半套可用工具；说明既有项目/IDE 模型如何刷新；仅移除工具拥有的文件，保留共享 Rust 环境和无关配置 |
| DIST-06 | [ ] 待实施 | 从干净环境验收安装包 | 依赖安装实现；两个 host 分别安装并运行 doctor、check/build/run/test、graph、init；覆盖跨 crate、空格/中文路径、重复安装和不匹配工件；没有仓库 checkout 或开发机绝对路径依赖 |
| DIST-07 | [ ] 待实施 | 补齐正式发布流程和用户安装文档 | 基于 DIST-06 的证据产出版本包、清单、校验信息及升级说明；开发构建文档和使用者安装文档分开；真正发布另行按维护者授权执行 |
| DIST-08 | [ ] 待决策 | 增加轻量 Cargo 安装入口 | 可在完整工具包之后实现；评估 CLI 的独立发布、下载与版本选择，确定是否提供 toolchain install；不在 Cargo build.rs 中隐式安装整套开发环境 |

DIST-06 的开发环境验收应区分“init 成功生成项目模型”与“真实编辑器交互正常”。如
发行说明承诺 IDE 即装即用，需要用实际原版 rust-analyzer 验证宏展开、补全、诊断与
保存检查；必须记录各 host 的实际覆盖，不能沿用旧快照的结果冒充新安装包验收。

Windows 原生编译仍涉及 MSVC Build Tools / Windows SDK。安装流程应能发现缺失并
给出准确指引；是否自动安装系统级工具尚未讨论确定。Linux 的系统库兼容基线也需
随 release 工件明确。当前 host 列表不自动扩展为 macOS、ARM64 或 Windows GNU 支持。

## 6. 生态后续 TODO

| 编号 | 状态 | 工作 | 依赖和边界 |
| --- | --- | --- | --- |
| ECO-01 | [ ] 待实施 | 实现独立的 nestrs-bootstrap | 先确定 Application、配置、生态组合与 NestrsFactory 的门面；所有运行期生态库仍建立在 core 上，不能反向依赖 bootstrap |
| ECO-02 | [ ] 待实施 | 实现 cargo nestrs create | 依赖 ECO-01 的应用契约；生成贴合真实开发的项目并自动初始化；创建后可以直接开发/运行，无需再执行 init |
| ECO-03 | [ ] 保留后续范围 | 扩充工具体验与支持矩阵 | 应用级 Clippy、更多 graph 目标、其他 host/编辑器和更广泛的宏/泛型组合已有明确边界，另立实现与验收任务，不顺带纳入分发首版 |

普通手动注册门面、声明 DSL、core/bootstrap 合并及恢复公开宏库均不是本次排定的
下一阶段任务。若维护者以后改变方向，应显式更新架构共识，不能从早期讨论恢复旧方案。

## 7. 后续会话接续顺序

1. 先读 AGENTS.md 和本文，核对待办状态，再检查实际源码及 git status。
2. 以迁移验收文档确认已有能力，不重做 BASE 项；当前存在未提交改动，不代表它们
   自动属于下一项任务，也不自动授权合并提交。
3. 若继续分发工作，从 DIST-01 的具体方案进入实现；本会话没有最终选定安装器形式。
4. 每完成一项更新本文与相关使用说明，记录实际命令、工件和平台验收；target 中的
   临时日志可能被清理，关键结论应写入可版本管理的文档。
5. 项目初始化、机器级工具安装和未来项目脚手架保持明确职责，不把安装塞入现有 init，
   不让 create 的用户重复手动初始化。

## 8. 实施时的代码入口

| 位置 | 当前作用 |
| --- | --- |
| [Cargo.toml](../cargo-nestrs/Cargo.toml) | 两个 bin 与 compiler-driver feature |
| [build-toolchain.py](../tools/build-toolchain.py) | 开发/CI 整套构建，可供未来打包流程复用 |
| [toolchain.json](../cargo-nestrs/toolchain.json) | 固定编译器身份与 host 列表 |
| [toolchain.rs](../cargo-nestrs/src/toolchain.rs) | 工具发现、版本检查、wrapper 与动态库环境配置 |
| [bridge.rs](../cargo-nestrs/src/bridge.rs) | 私有 bridge 查找、extern 注入和缓存指纹 |
| [commands](../cargo-nestrs/src/commands) | 当前命令和 init；无正式安装子命令 |
| [native-host.yml](../.github/workflows/native-host.yml) | 当前双 host 回归 CI，尚未承担正式发行包发布 |

技术背景见 [工具链演进方案](NESTRS_COMPILER_TOOLCHAIN_PLAN.md)；当前功能和命令以
[Cargo 工具链说明](NESTRS_CARGO_TOOLCHAIN.md) 为准。本文不替代其运行期与隐私契约。

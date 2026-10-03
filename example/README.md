# Nestrs 示例项目

`example/` 是示例项目的父目录，不是一个应用 package。每个子目录包含一个独立的
示例项目根目录，包括自己的 `Cargo.toml`、`src/` 和 README。正常业务示例作为仓库
workspace 的成员复用框架依赖和工具链；故意编译失败的诊断观察项目放在独立 workspace 中。

| 示例目录 | Cargo package | 业务场景 |
| --- | --- | --- |
| [di-checkout](di-checkout/README.md) | `nestrs-di-example` | 电商结账：同步 constructor、异步 factory、延迟依赖、keyed 支付、泛型仓库及异步 DI 关闭 |
| [di-errors](di-errors/README.md) | 30 个 `nestrs-error-*` 独立 package | 观察依赖缺失、key 错配、歧义、循环、生命周期等真实编译错误；本组自成 workspace |

## 运行与依赖图

先按[仓库 README](../README.md)构建匹配的 `cargo-nestrs` 工具链，并将 CLI 加入 PATH。
在仓库根目录执行：

```bash
cargo nestrs run -p nestrs-di-example -- sample
cargo nestrs graph -p nestrs-di-example
```

也可以进入具体项目目录：

```bash
cd example/di-checkout
cargo nestrs run -- sample
cargo nestrs graph
cargo nestrs test --all-targets
```

`di-checkout` 是单 binary 应用，`main.rs` 直接组织私有业务模块，不额外导出应用库。
`sample` 运行四个业务场景；`place-order` 接收真实下单参数；无子命令时显示帮助。
它只提供正常的 `checkout` 程序入口。最终入口在 check/build 阶段验证完整 DI 结构；
graph 读取这份编译计划的图数据，不运行业务或服务构造。项目图导出在图校验和文件写入成功时
返回退出码 `0`，默认写入 workspace 的 `target/nestrs-di.html`；不会因为框架负例而失败。
进入 `example/` 父目录不会自动选择某个示例，应进入子目录或使用 `-p` 指定 package。

应用检查和运行使用 `cargo nestrs`；普通 Cargo 不提供服务声明桥接和自动绑定环境。
工具会自动生成私有适配代码与已验证计划，统称 `nestrs-reflect`。示例只依赖 core
和业务所需库，无需添加同名 Cargo package 或手工注册服务；详见
[工具链与编译产物](../docs/NESTRS_CARGO_TOOLCHAIN.md)。编译期图检查与运行期资源初始化
分别发生：通过 `check` 或导出 HTML 后，仍需运行示例或测试验证工厂和业务行为。
开发环境通过 `cargo nestrs init` 初始化或刷新，默认生成通用 rust-analyzer 项目
与设置；使用 VS Code 时加 `--vscode`，其他 rust-analyzer LSP 客户端需要自行加载
生成配置。该命令不创建项目或添加依赖，具体参数见各示例 README。

## 观察编译期依赖图错误

[`di-errors/`](di-errors/README.md) 为每种典型错误关系提供一个轻型项目；总索引统一维护源码链接、
复现命令、错误编号和修复方向。从仓库根目录查看必选依赖缺失：

```bash
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml
```

这里命令以非零状态结束才是预期效果；不需要运行容器或服务。也可进入对应项目直接执行
`cargo nestrs check`，或将 `check` 换成 `build`。这 30 个项目不属于仓库根 workspace，
不会被框架的 `--workspace` 构建或正常业务示例的图导出选中。

如果想看一个项目同时报告多个错误，运行
[`30-multiple-errors`](di-errors/30-multiple-errors/src/main.rs)：

```bash
cargo nestrs check --manifest-path example/di-errors/30-multiple-errors/Cargo.toml
```

它会在一次编译中依次报告 5 条错误，并在各自的源码位置给出说明。

## 添加正常业务示例

- 在 `example/<场景名>/` 下建立一个项目，目录名描述业务场景，不直接把源码放进 `example/`。
- 为项目设置唯一的 Cargo package 名称，并在仓库根 `Cargo.toml` 的 workspace members 中登记。
- 以可运行的正常业务流程展示框架能力；在项目 README 中说明运行命令、预期结果、模拟资源和阅读顺序。
- 业务与生命周期回归可放在应用内部的 `#[cfg(test)]` 模块；进程边界测试放在示例自己的
  `tests/`。不必为了访问内部代码而创建应用 library。工具回归的非法声明继续放在
  `cargo-nestrs/tests/fixtures/`；供读者手动观察错误的教学项目可放在独立的 `di-errors/`
  workspace，避免正常示例的默认运行与图导出失败。
- 在本索引中加入目录、package 名称与场景说明；示例中引用仓库 crate 的 path 相对于自己的 `Cargo.toml` 计算。

非法生命周期回归现位于
[`cargo-nestrs/tests/fixtures/aot-invalid/src/bin/scope.rs`](../cargo-nestrs/tests/fixtures/aot-invalid/src/bin/scope.rs)，
由 `aot_graph.rs` 验证编译期拒绝；即使入口没有调用容器构建也会失败。
项目图“部分入口失败仍保留有效图”的测试继续使用 graph fixture 中的独立负例，不属于业务示例。

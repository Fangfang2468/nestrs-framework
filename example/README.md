# Nestrs 示例项目

`example/` 是示例项目的父目录，不是一个应用 package。每个子目录包含一个独立的
示例项目根目录，包括自己的 `Cargo.toml`、`src/`、测试和 README；示例作为仓库
workspace 的成员复用框架依赖和工具链。

| 示例目录 | Cargo package | 业务场景 |
| --- | --- | --- |
| [di-checkout](di-checkout/README.md) | `nestrs-di-example` | 电商结账：并发请求、库存预留、keyed 支付、泛型仓库及异步 DI 关闭 |

## 运行与依赖图

先按[仓库 README](../README.md)构建匹配的 `cargo-nestrs` 工具链，并将 CLI 加入 PATH。
在仓库根目录执行：

```bash
cargo nestrs run -p nestrs-di-example
cargo nestrs graph -p nestrs-di-example
```

也可以进入具体项目目录：

```bash
cd example/di-checkout
cargo nestrs run
cargo nestrs graph
cargo nestrs test --tests
```

`di-checkout` 只提供正常的 `checkout` 程序入口。项目图导出在图校验和文件写入成功时
返回退出码 `0`，默认写入 workspace 的 `target/nestrs-di.html`；不会因为框架负例而失败。
进入 `example/` 父目录不会自动选择某个示例，应进入子目录或使用 `-p` 指定 package。

应用检查和运行使用 `cargo nestrs`；普通 Cargo 不提供服务声明桥接和自动绑定环境。
开发环境通过 `cargo nestrs init` 初始化或刷新，默认生成通用 rust-analyzer 项目
与设置；使用 VS Code 时加 `--vscode`，其他 rust-analyzer LSP 客户端需要自行加载
生成配置。该命令不创建项目或添加依赖，具体参数见各示例 README。

## 添加示例

- 在 `example/<场景名>/` 下建立一个项目，目录名描述业务场景，不直接把源码放进 `example/`。
- 为项目设置唯一的 Cargo package 名称，并在仓库根 `Cargo.toml` 的 workspace members 中登记。
- 以可运行的正常业务流程展示框架能力；在项目 README 中说明运行命令、预期结果、模拟资源和阅读顺序。
- 业务与流程回归放在示例自己的 `tests/`。故意违反 DI 声明或生命周期规则的用例放在
  `cargo-nestrs/tests/fixtures/`，由工具回归测试驱动，避免正常示例的默认运行与图导出失败。
- 在本索引中加入目录、package 名称与场景说明；示例中引用仓库 crate 的 path 相对于自己的 `Cargo.toml` 计算。

非法生命周期回归现位于
[`cargo-nestrs/tests/fixtures/di/src/bin/invalid_lifetime.rs`](../cargo-nestrs/tests/fixtures/di/src/bin/invalid_lifetime.rs)。
项目图“部分入口失败仍保留有效图”的测试继续使用 graph fixture 中的独立负例，不属于业务示例。

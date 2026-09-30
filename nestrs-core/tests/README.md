# nestrs-core 测试目录

所有测试实现统一保存在 crate 根目录的 `tests/`，`src/` 只保存生产实现及必要的
测试模块挂载声明。目录位置与 Rust 模块可见性分开处理，不为迁移测试开放内部 API。

```text
tests/
├── unit/                       # 不依赖声明工具链的内部单元测试
│   ├── activation/             # 稳定地址、注入令牌、强 lease、迭代释放
│   │   └── construction/       # 输入准备/消费、类型化投影、工厂借用
│   ├── graph/                  # 图编译及只读图诊断
│   ├── registration/           # 闭合类型描述探测
│   ├── runtime/                # 协调器、缓存、并发、失败传播与关闭
│   ├── contracts.rs            # 注册与构造 ABI 的内部契约
│   ├── error.rs                # 共享失败记录、深路径显示与释放
│   └── facade_api.rs           # 容器公开门面与 owner 借用期
└── compiler/                   # 真实 Nestrs 工具链生成声明的隔离契约
```

## 单元测试如何加载

生产模块只保留下面这样的声明，测试函数、辅助类型和断言均位于 `tests/unit/`：

```rust,ignore
#[cfg(test)]
#[path = "../../tests/unit/activation/instance.rs"]
mod tests;
```

这样测试仍是被测模块的子模块，能通过 Rust 原有的私有访问规则检查内部状态。
`super`、测试名称和运行方式保持一致，不需要增加 `pub`、测试专用公开 ABI 或额外
编译一套生产实现。`#[path]` 相对声明所在的源文件目录解析，不是相对 shell 工作目录。

`unit/` 与 `compiler/` 不设置 `main.rs`，因此不会被 Cargo 误当成独立集成测试目标。
普通单元测试通过已有 lib 测试目标执行：

```sh
cargo test -p nestrs-core
cargo test -p nestrs-core --lib -- --list
```

## 编译器契约如何加载

`compiler/` 的每组用例由 `cargo-nestrs/tests/compiler_contracts.rs` 创建隔离测试工程，
包含真实 core 源码，并通过 Nestrs driver 编译运行。它们测试真实生成描述、类型化
adapter、泛型、key 和图初始化，每个工程有独立的注册入口。

这些用例需要真实工具链，普通 core 测试不会自动运行它们。先按仓库说明构建配套工具，
在已配置 driver 编译环境的终端执行：

```sh
cargo test -p cargo-nestrs --features compiler-driver --test compiler_contracts
```

涉及业务宏的黑盒集成和 UI 编译失败用例仍属于 `cargo-nestrs/tests/fixtures/di/`，
不让 core 反向依赖工具 crate。新增 core 测试应继续放到本目录；不能重新在 `src/`
创建测试文件或内联测试主体，也不能通过削弱私有边界来迁就文件布局。

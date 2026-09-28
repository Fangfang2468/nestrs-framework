//! cargo nestrs 管理的内部过程宏桥接，不是应用的 Cargo 依赖。
//!
//! 编译器和编辑器均将此产物作为 `nestrs` 命名空间注入。应用直接使用
//! `#[nestrs::injectable]` 和 `#[nestrs::factory]`，无需了解本包名称。
//! 此处只转换 token；声明分析、字段改写和代码生成复用 cargo-nestrs 后端。

/// 声明可注入的结构体，消费其字段上的 `#[inject]` 与 `#[value(...)]`。
#[proc_macro_attribute]
pub fn injectable(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_injectable(args.into(), input.into()).into()
}

/// 声明同步或异步工厂，生成借用真实 activation frame 的参数签名。
#[proc_macro_attribute]
pub fn factory(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_factory(args.into(), input.into()).into()
}

/// 将服务声明标记为 primary；可以放在 `injectable` 或 `factory` 上下方。
///
/// 配合使用时保留属性的末段名称 `primary`、`injectable`、`factory`；支持
/// `nestrs::factory` 和 crate/module 别名路径。单独使用 provider 宏可以
/// 重命名导入，但叠加 primary 时不保证任意属性重命名或任意顺序：过程宏
/// 无法解析尚未展开的其他属性别名。
#[proc_macro_attribute]
pub fn primary(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_primary(args.into(), input.into()).into()
}

/// 显式绑定 ABI 的内部回归入口；应用使用普通 impl 和 CLI 自动绑定。
#[doc(hidden)]
#[proc_macro_attribute]
pub fn bind(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_bind(args.into(), input.into()).into()
}

#![feature(proc_macro_def_site)]

//! cargo nestrs 管理的内部过程宏桥接，不是应用的 Cargo 依赖。
//!
//! 编译器和编辑器均将此产物作为 `nestrs` 命名空间注入。应用直接使用
//! `#[nestrs::injectable]` 和 `#[nestrs::factory]`，无需了解本包名称。
//! 此处只转换 token；声明分析、字段改写和代码生成复用 cargo-nestrs 后端。
//! 生成局部绑定和内部辅助项的定义处 span 由本桥接显式传给后端；不改业务 token 的卫生上下文。

/// 声明可注入的结构体，消费其字段上的 `#[inject]` 与 `#[value(...)]`。
#[proc_macro_attribute]
pub fn injectable(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_injectable_with_binding_span(
        args.into(),
        input.into(),
        proc_macro::Span::def_site().into(),
    )
    .into()
}

/// 内部字段条件编译阶段；用户不直接调用。
#[doc(hidden)]
#[proc_macro_derive(
    __NestrsConfiguredInjectable,
    attributes(__nestrs_declaration, __nestrs_type, __nestrs_attribute)
)]
pub fn configured_injectable(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_configured_injectable_with_binding_span(
        input.into(),
        proc_macro::Span::def_site().into(),
    )
    .into()
}

/// 声明同步或异步工厂，生成借用真实 activation frame 的参数签名。
#[proc_macro_attribute]
pub fn factory(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_factory_with_binding_span(
        args.into(),
        input.into(),
        proc_macro::Span::def_site().into(),
    )
    .into()
}

/// 为 injectable 服务指定同步关联构造函数；参数声明依赖，函数体完成业务初始化。
#[proc_macro_attribute]
pub fn constructor(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_constructor_with_binding_span(
        args.into(),
        input.into(),
        proc_macro::Span::def_site().into(),
    )
    .into()
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

/// 设置声明本身的初始化策略：bare/true 延迟，false 提前，无标注则继承容器策略。
/// 必须与 injectable 结构体或 factory 函数配合；上下顺序均可，路径命名边界同 primary。
#[proc_macro_attribute]
pub fn lazy(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    cargo_nestrs::codegen::expand_lazy(args.into(), input.into()).into()
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

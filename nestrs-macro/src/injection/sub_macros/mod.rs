//! 由多个属性宏共用的子标注。
//!
//! 子标注不是独立的属性宏，而是外层宏在自身展开过程中消费的 marker（例如
//! `#[injectable]` 字段与 `#[factory]` 参数上的 `#[inject]`）。它们没有自己的
//! 展开入口，因此共享的语法与语义集中定义在这里，外层宏只负责各自 AST 改写。

pub mod inject;
pub mod value;

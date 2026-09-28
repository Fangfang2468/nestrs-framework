//! 三个属性宏入口。
//!
//! 每个子模块负责一个属性宏从语法分析到注册代码生成的全过程；多个宏共享的语法
//! （`sub_macros` 中的子标注）与注册 ABI 渲染（`render`）放在上一层，供它们共同引用。

pub mod bind;
pub mod factory;
pub mod injectable;

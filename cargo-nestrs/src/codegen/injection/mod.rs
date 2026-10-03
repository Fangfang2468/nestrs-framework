//! 服务声明的宏分析与生成组件。
//!
//! provider 入口组织展开流程，属性和 helper 模块只解析语法，render 统一生成执行适配协议。

pub mod macros;
pub mod macros_attrs;
pub mod render;
pub mod sub_macros;

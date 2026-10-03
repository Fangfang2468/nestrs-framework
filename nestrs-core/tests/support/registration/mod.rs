//! 旧声明模型仅作为隔离单测的参考输入，不进入生产库或生成代码协议。
//!
//! 实际项目的声明归工具链所有；此处保留旧算法的输入，比较完整图选择规则，避免
//! 迁移执行协议时删除原有错误与安全边界测试。

pub mod binding;
pub mod catalog;
pub mod dependency;
pub mod provider;
pub mod root;

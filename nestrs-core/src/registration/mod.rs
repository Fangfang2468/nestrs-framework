//! 工具生成代码与图编译器之间的描述协议。
//!
//! 该模块属于 core 的私有实现，生成代码的访问权由 driver 审核，不是业务注册 API。
//! `provider`/`dependency`/`binding` 描述要构造什么，`root` 记录具体类型的查询锚点，
//! `compiler` 为跨 crate 语义分析保留类型标记，`catalog` 接收最终入口汇总的本次快照。
//! 本层不会选择候选、调用构造、保存实例或执行生命周期状态机。

pub mod binding;
pub mod catalog;
pub mod compiler;
pub mod dependency;
pub mod provider;
pub mod root;

//! 本地基础设施适配：资源工厂、共享库存与内存存储，不负责应用 scope。

mod database;
mod inventory;
mod payment;
mod repository;

#[cfg(test)]
pub(crate) use database::Database;
pub(crate) use inventory::Inventory;
pub(crate) use repository::Repository;

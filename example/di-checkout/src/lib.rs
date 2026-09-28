//! 一个本地可运行的电商下单应用：业务服务通过字段/工厂参数注入依赖，
//! 只有应用入口负责容器查询、请求 scope 和关闭。

pub mod demo;
pub mod domain;
pub mod observe;
pub mod services;

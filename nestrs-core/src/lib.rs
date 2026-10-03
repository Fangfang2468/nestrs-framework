// 以静态依赖图为执行计划的异步 DI 容器。
//
// 阅读实现时可以沿以下顺序进入各模块：
// 1. 工具链收集类型化声明和查询摘要，在最终入口编译时验证完整 DI 图。
// 2. 工具链生成的 reflect 产物提供执行入口，`graph::plan` 一次装配并共享执行计划；
//    core 生产运行时不再选择候选、展开泛型或验证拓扑。
// 3. `facade` 将用户借用和查询请求交给 `runtime` 的单一协调器。
// 4. `activation` 准备输入、保存稳定实例和强 lease，并以迭代队列释放依赖。
//
// 生命周期缓存、实例所有权与公开借用分别承担不同职责：缓存合并初始化，owner 的
// journal 保活已发布实例，注入令牌和工厂 frame 保活各自依赖。它们共同保证取消查询、
// owner 关闭或 Tokio 停止时不会把仍可被安全代码访问的值提前释放。

// 这些真实私有模块同时服务于编译器生成的 adapter。普通 Cargo 的本地调用分析看不到
// 下游生成代码中的使用点；因此限定在相关模块允许这些编译期协议入口暂未被本地引用。
#[allow(dead_code, unused_imports)]
mod activation;
mod error;
#[allow(dead_code)]
mod facade;
#[allow(dead_code)]
mod graph;
mod lifetime;
mod options;
// 旧声明结构仅保留为隔离图算法的测试输入；生产构建与工具生成代码均不使用此模块。
#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/support/registration/mod.rs"]
mod registration;
mod runtime;
#[allow(dead_code)]
mod service;

pub use error::{BuildError, DisposeError, ResolveError};
pub use facade::{ServiceProvider, ServiceProviderRef, ServiceScope};
pub use lifetime::ServiceLifetime;
pub use options::{InitializationMode, ServiceProviderOptions};
pub use service::ServiceKey;

pub use activation::{Injection, LazyInjection};

// 测试实体统一放在 crate 根 tests/；这里只声明挂载点，使白盒测试继续受同一
// crate 的私有边界约束，不为测试暴露生产内部 API。
#[cfg(test)]
#[path = "../tests/unit/contracts.rs"]
mod contract_tests;

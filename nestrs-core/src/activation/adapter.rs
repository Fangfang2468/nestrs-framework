//! 工具链生成代码与执行引擎之间的类型化操作契约。
//!
//! 这里仅保存当前目标程序的真实类型身份和可调用函数地址。服务声明、候选选择、
//! key 匹配与泛型发现都属于工具链；执行计划已经选定具体节点与输入目标。
//! 适配器按业务类型所在 crate 生成，继续接受 rustc 的隐私、类型和借用检查。

use std::{future::Future, pin::Pin};

use super::{AsyncConstructor, ClassConstructor, FactoryConstructor, InputKind, ServiceProjector};
use crate::service::ServiceType;

/// cleanup 返回的独立 future；执行引擎按 owner 顺序等待它完成。
pub type CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// 一个已成功发布实例的清理入口；每个实际实例在逻辑关闭时调用一次。
pub type CleanupHook = fn() -> CleanupFuture;

/// 工厂对真实借用 frame 的调用方式，不携带服务声明或依赖查找规则。
#[derive(Debug, Clone, Copy)]
pub enum FactoryInvoker {
    Sync(FactoryConstructor),
    Async(AsyncConstructor),
}

/// 已经选定的业务构造入口。class 接收拥有 lease 的输入，factory 借用真实调用帧。
#[derive(Debug, Clone, Copy)]
pub enum Constructor {
    Class(ClassConstructor),
    Factory(FactoryInvoker),
}

/// 单个业务类型的执行能力。输入严格按槽位排列，不包含注册规则或可调用蓝图。
#[derive(Debug, Clone)]
pub struct ActivationAdapter {
    pub service_type: ServiceType,
    pub constructor: Constructor,
    pub inputs: Vec<InputAdapter>,
    pub cleanup: Option<CleanupHook>,
}

/// 一个输入的类型化操作；对应目标、可选性和诊断信息由最终计划另行固定。
#[derive(Debug, Clone, Copy)]
pub struct InputAdapter {
    pub service_type: ServiceType,
    /// 编译器固定的准确交付形态，缺席和延迟输入同样必须验证。
    pub kind: InputKind,
    /// 普通或延迟目标的直接类型化交付；接口输入使用已选投影的对应入口。
    pub project: Option<ServiceProjector>,
}

/// 真实 concrete 到 trait 的类型转换能力。它不注册服务、不选候选，也不物化类型。
#[derive(Debug, Clone, Copy)]
pub struct ProjectionAdapter {
    pub trait_type: ServiceType,
    pub concrete_type: ServiceType,
    pub project: ServiceProjector,
}

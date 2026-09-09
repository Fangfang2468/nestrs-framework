use crate::{
    __private::{Constructor, InputPosition, PrepareInput},
    lifetime::Lifetime,
    registration::{service_identifier::ServiceIdentifier, service_source::ServiceSource},
};

#[derive(Debug, Clone, Copy)]
pub struct ServiceDescriptor {
    /// 注册服务的生命周期
    pub lifetime: Lifetime,

    /// 是否为默认实现
    pub primary: bool,

    /// 注册服务的声明来源，用于诊断信息或提示信息
    pub source: ServiceSource,
}

/// 一次字段或工厂参数的依赖注入语义。
#[derive(Debug, Clone, Copy)]
pub struct InjectionSpec {
    /// 依赖在构造输入中的位置。
    pub position: InputPosition,

    /// 查找依赖服务的 token。
    pub identifier: ServiceIdentifier,

    /// 缺失依赖时是否允许交付 `None`。
    pub optional: bool,

    /// 依赖诊断或元数据使用的可读标签。
    pub label: Option<&'static str>,

    /// 依赖请求的注入目标类别。
    pub target: InjectionTarget,

    /// 将 Arena 已发布的稳定地址准备为对应 `Inject<T>` 的单态化函数。
    pub prepare_input: PrepareInput,
}

/// 依赖请求的目标类型形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InjectionTarget {
    /// 普通具体服务类型。
    Concrete,

    /// `dyn Trait`，需要 concrete-to-trait 投影。
    TraitObject,

    /// 当前稳定地址 ABI 尚不能表示的依赖类型。
    Unsupported,
}

/// 泛型 provider 家族的静态描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GenericFamily {
    /// 泛型定义名称，用于诊断，不用于类型解析。
    pub name: &'static str,

    /// 泛型参数数量。
    pub arity: usize,
}

/// 将请求的闭合 `ServiceIdentifier` 具体化并返回对应 provider。
pub type GenericCloser = fn(ServiceIdentifier) -> Option<Provider>;

#[derive(Debug, Clone, Copy)]
pub enum ServiceConstructor {

    /// 同步构造器
    Sync(Constructor),

    /// 暂时不设计异步的情况，先占位
    Async(fn()),
}

#[derive(Debug, Clone)]
pub enum Provider {
    /// 通过 [`#\[injectable`\]] 方式注册， 对标 `Nestjs` 中的 `useClass`，
    Class {
        /// 查找服务的 ID
        provide: ServiceIdentifier,

        /// 服务的共有属性描述
        descriptor: ServiceDescriptor,

        /// 结构体服务注入的字段
        dependencies: Vec<InjectionSpec>,

        /// 服务的构造器
        constructor: ServiceConstructor,
    },

    /// 通过 [`#\[factory`\]] 方式注册， 对标 `Nestjs` 中的 `useFactory`，
    Factory {
        /// 查找服务的 id
        provide: ServiceIdentifier,

        /// 服务的共有属性描述
        descriptor: ServiceDescriptor,

        /// 工厂函数注册服务需要的参数
        dependencies: Vec<InjectionSpec>,

        /// 工厂函数的构造调用器
        construct_invoker: ServiceConstructor,
    },

    /// 提供泛型服务注入的支持。
    Generic {
        /// 泛型家族描述。
        family: GenericFamily,

        /// 泛型 provider 的公共属性。
        descriptor: ServiceDescriptor,

        /// 将闭合请求具体化为 Provider 的编译期闭合函数。
        close: GenericCloser,
    },

    /// 提供 Trait 注入的支持。
    Bound {
        /// 请求到的 trait token。
        provide: ServiceIdentifier,

        /// 实际提供服务的 concrete token。
        concrete: ServiceIdentifier,

        /// concrete-to-trait 必选投影函数。
        prepare_required: PrepareInput,

        /// concrete-to-trait 可选投影函数。
        prepare_optional: PrepareInput,
    },

    // /// 通过 [`ServiceProvider::add_instance`] 方式注册， 对标 `Nestjs` 中的 `useValue`，
    // Instance {
    //     provide: ServiceIdentifier,
    //     instance:
    // },
    /// 对标 `Nestjs` 中的 `useExisting`，
    Alias {
        provide: ServiceIdentifier,
        target: ServiceIdentifier,
    },
}

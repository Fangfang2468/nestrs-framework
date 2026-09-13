//! 注册项的归一化查询模型。
//!
//! linkme 收集的是统一的 [`Provider`] 枚举，其中实例生产者（`Class` / `Factory`）与
//! trait 投影规则（`Bound`）角色并不相同。registry 在构建时一次性按角色分区，此后的
//! 解析路径只查询同质表，不再需要在每次查询时判断变体。

use std::{collections::HashMap, sync::OnceLock};

use crate::{
    construction::Constructor,
    registration::{
        dependency::DependencyRequest,
        provider::{
            AsyncConstructor, FactoryConstructor, FactoryInvoker, Provider, ProviderCommon,
            REFLECTED_PROVIDERS, TraitBinding,
        },
        service_identifier::ServiceIdentifier,
        service_type::ServiceType,
    },
};

/// 一个可激活节点的统一视图。
///
/// `#[injectable]` 与 `#[factory]` 的差异只剩 [`Activation`]：解析路径不需要知道实例
/// 来自结构体构造还是工厂函数，只在真正调用构造时才区分。
#[derive(Debug, Clone)]
pub struct InstanceProvider {
    /// provider 导出的 service token。
    pub provide: ServiceIdentifier,

    /// provider 的共享声明属性。
    pub common: ProviderCommon,

    /// 构造该实例所需的依赖请求。
    pub dependencies: Vec<DependencyRequest>,

    /// 实例的构造方式。
    pub activation: Activation,
}

/// 一个实例 provider 的构造方式。
#[derive(Debug, Clone, Copy)]
pub enum Activation {
    /// `#[injectable]` 生成的同步构造 adapter。
    Class(Constructor),

    /// 同步 `#[factory]`。
    SyncFactory(FactoryConstructor),

    /// 异步 `#[factory]`（`async fn` 或显式返回 `Future`）。
    AsyncFactory(AsyncConstructor),
}

/// 当前链接单元内所有注册项归一化后的查询视图。
///
/// 同一个 [`ServiceIdentifier`] 允许存在多个候选：候选选择（唯一 primary 或歧义诊断）
/// 属于编译阶段，因此这里保留 `Vec` 而不是提前覆盖。
#[derive(Debug, Default)]
pub struct Registry {
    providers: HashMap<ServiceIdentifier, Vec<InstanceProvider>>,
    bindings: HashMap<ServiceType, Vec<TraitBinding>>,
}

impl Registry {
    /// 由注册项构造函数列表构建 registry。
    ///
    /// 这里是全仓库唯一按 [`Provider`] 角色分派的位置：`Class` / `Factory` 归一化为
    /// [`InstanceProvider`]，`Bound` 进入按 trait 类型索引的投影规则表。
    pub fn from_entries(entries: impl IntoIterator<Item = fn() -> Provider>) -> Self {
        let mut registry = Self::default();

        for entry in entries {
            match entry() {
                Provider::Class(provider) => registry.insert_provider(InstanceProvider {
                    provide: provider.provide,
                    common: provider.common,
                    dependencies: provider.dependencies,
                    activation: Activation::Class(provider.constructor),
                }),
                Provider::Factory(provider) => registry.insert_provider(InstanceProvider {
                    provide: provider.provide,
                    common: provider.common,
                    dependencies: provider.dependencies,
                    activation: match provider.invoker {
                        FactoryInvoker::Sync(constructor) => Activation::SyncFactory(constructor),
                        FactoryInvoker::Async(constructor) => Activation::AsyncFactory(constructor),
                    },
                }),
                Provider::Bound(binding) => registry
                    .bindings
                    .entry(binding.trait_type)
                    .or_default()
                    .push(binding),
            }
        }

        registry
    }

    /// 由当前链接单元的 [`REFLECTED_PROVIDERS`] 构建 registry。
    pub fn from_reflected() -> Self {
        Self::from_entries(REFLECTED_PROVIDERS.iter().copied())
    }

    /// 当前链接单元内缓存的只读 registry。
    pub fn global() -> &'static Registry {
        static REGISTRY: OnceLock<Registry> = OnceLock::new();

        REGISTRY.get_or_init(Self::from_reflected)
    }

    /// 指定 service token 的全部实例候选。
    pub fn providers(&self, identifier: ServiceIdentifier) -> &[InstanceProvider] {
        self.providers
            .get(&identifier)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 遍历全部实例候选。
    pub fn all_providers(&self) -> impl Iterator<Item = &InstanceProvider> {
        self.providers.values().flatten()
    }

    /// 指定 trait 类型的全部投影规则。
    pub fn bindings(&self, trait_type: ServiceType) -> &[TraitBinding] {
        self.bindings
            .get(&trait_type)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 遍历全部投影规则。
    pub fn all_bindings(&self) -> impl Iterator<Item = &TraitBinding> {
        self.bindings.values().flatten()
    }

    /// registry 中是否没有任何注册项。
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty() && self.bindings.is_empty()
    }

    fn insert_provider(&mut self, provider: InstanceProvider) {
        self.providers
            .entry(provider.provide)
            .or_default()
            .push(provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        construction::{
            ActivationError, ConstructionContext, ErasedService, FactoryConstructionContext,
        },
        lifetime::Lifetime,
        registration::{
            provider::{BoundKeyPolicy, ClassProvider, FactoryProvider, FactoryFuture, ProviderKind},
            service_source::ServiceSource,
        },
    };

    struct Service;
    struct OtherService;

    trait Port: Send + Sync {}

    impl Port for Service {}

    fn source(line: u32) -> ServiceSource {
        ServiceSource::new("registry.rs", line, 1)
    }

    fn common(line: u32) -> ProviderCommon {
        ProviderCommon {
            lifetime: Lifetime::Singleton,
            primary: false,
            source: source(line),
            cleanup: None,
        }
    }

    fn construct_service(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Service))
    }

    fn construct_other<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(OtherService))
    }

    fn construct_other_async<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> FactoryFuture<'frame> {
        Box::pin(async { Ok(ErasedService::new(OtherService)) })
    }

    fn project_service(service: &Service) -> &(dyn Port + 'static) {
        service
    }

    fn prepare_required_port(
        context: &mut ConstructionContext,
        position: crate::construction::InputPosition,
        input: Option<crate::arena::ArenaServiceRef>,
    ) -> Result<(), ActivationError> {
        crate::construction::prepare_bound_required::<Service, dyn Port>(
            context,
            position,
            input,
            project_service,
        )
    }

    fn prepare_optional_port(
        context: &mut ConstructionContext,
        position: crate::construction::InputPosition,
        input: Option<crate::arena::ArenaServiceRef>,
    ) -> Result<(), ActivationError> {
        crate::construction::prepare_bound_optional::<Service, dyn Port>(
            context,
            position,
            input,
            project_service,
        )
    }

    fn class_entry() -> Provider {
        Provider::Class(ClassProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<Service>()),
            common: common(1),
            dependencies: vec![],
            constructor: construct_service,
        })
    }

    fn factory_entry() -> Provider {
        Provider::Factory(FactoryProvider {
            // 与 class entry 相同的 token：registry 必须保留两个候选。
            provide: ServiceIdentifier::from(ServiceType::create::<Service>()),
            common: common(2),
            dependencies: vec![],
            invoker: FactoryInvoker::Sync(construct_other),
        })
    }

    fn bound_entry() -> Provider {
        Provider::Bound(TraitBinding {
            trait_type: ServiceType::create::<dyn Port>(),
            concrete_type: ServiceType::create::<Service>(),
            key_policy: BoundKeyPolicy::InheritRequestedKey,
            prepare_required: prepare_required_port,
            prepare_optional: prepare_optional_port,
            source: source(3),
        })
    }

    fn async_factory_entry() -> Provider {
        Provider::Factory(FactoryProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<OtherService>()),
            common: common(4),
            dependencies: vec![],
            invoker: FactoryInvoker::Async(construct_other_async),
        })
    }

    #[test]
    fn from_entries_partitions_roles_and_keeps_every_candidate() {
        let entries: Vec<fn() -> Provider> = vec![class_entry, factory_entry, bound_entry];
        let registry = Registry::from_entries(entries);
        let identifier = ServiceIdentifier::from(ServiceType::create::<Service>());

        let candidates = registry.providers(identifier);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|candidate| matches!(
            candidate.activation,
            Activation::Class(_)
        )));
        assert!(candidates.iter().any(|candidate| matches!(
            candidate.activation,
            Activation::SyncFactory(_)
        )));

        let bindings = registry.bindings(ServiceType::create::<dyn Port>());
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0].concrete_type,
            ServiceType::create::<Service>()
        );

        assert_eq!(registry.all_providers().count(), 2);
        assert_eq!(registry.all_bindings().count(), 1);
        assert!(!registry.is_empty());
        assert!(
            registry
                .providers(ServiceIdentifier::from(ServiceType::create::<OtherService>()))
                .is_empty()
        );
    }

    #[test]
    fn factory_activation_keeps_the_sync_async_distinction() {
        let entries: Vec<fn() -> Provider> = vec![factory_entry, async_factory_entry];
        let registry = Registry::from_entries(entries);

        let synchronous = registry.providers(ServiceIdentifier::from(
            ServiceType::create::<Service>(),
        ));
        assert!(matches!(
            synchronous[0].activation,
            Activation::SyncFactory(_)
        ));

        let asynchronous = registry.providers(ServiceIdentifier::from(
            ServiceType::create::<OtherService>(),
        ));
        assert!(matches!(
            asynchronous[0].activation,
            Activation::AsyncFactory(_)
        ));
    }

    #[test]
    fn provider_accessors_describe_each_role() {
        let class = class_entry();
        assert_eq!(class.kind(), ProviderKind::Class);
        assert_eq!(class.source(), source(1));
        assert_eq!(
            class.exported_identifier(),
            Some(ServiceIdentifier::from(ServiceType::create::<Service>()))
        );
        assert!(class.common().is_some());

        let factory = factory_entry();
        assert_eq!(factory.kind(), ProviderKind::Factory);
        assert!(factory.common().is_some());

        // Bound 是投影规则：不导出自己的 token，也没有独立的 lifetime/primary。
        let binding = bound_entry();
        assert_eq!(binding.kind(), ProviderKind::Bound);
        assert_eq!(binding.source(), source(3));
        assert!(binding.exported_identifier().is_none());
        assert!(binding.common().is_none());
    }

    #[test]
    fn global_registry_is_cached_and_reflects_the_empty_core_slice() {
        assert!(Registry::from_reflected().is_empty());
        assert!(std::ptr::eq(Registry::global(), Registry::global()));
    }
}

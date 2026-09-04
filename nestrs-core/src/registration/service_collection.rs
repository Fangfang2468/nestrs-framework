//! 不依赖构件图的直接实例化入口。
//!
//! `ServiceCollection` 仅保存 linkme 收集到的完整 component metadata。调用
//! [`ServiceCollection::instantiate`] 时，它按字段实际需要递归构造服务，并立即把
//! 每个成功输出提交进 [`crate::arena::Arena`]。这里刻意不建立持久依赖图、不做全局
//! 可构造性验证，也不生成并发构建计划。

use thiserror::Error;

use crate::{
    arena::{Arena, ArenaError},
    construction::{ActivationError, ConstructionContext, InputPosition, PrepareInput},
    metadata::{
        impl_bind::{InterfaceBinding, REFLECT_METADATA_BIND},
        injectable::{
            FieldInjection, FieldInjectionTarget, REFLECT_METADATA_INJECTABLE, StructComponent,
        },
    },
    registration::{
        injectable::Injectable, service_identifier::ServiceIdentifier, service_type::ServiceType,
    },
};

/// 一次直接实例化失败的原因。
#[derive(Debug, Error)]
pub enum InstantiationError {
    #[error("未找到根服务组件 {identifier:?}")]
    RootComponentNotFound { identifier: ServiceIdentifier },

    #[error("构造服务 {provider:?} 时缺少必选依赖 {dependency:?}")]
    MissingDependency {
        provider: ServiceIdentifier,
        dependency: ServiceIdentifier,
    },

    #[error(
        "字段 {field_name:?} 请求 {requested:?}，但其泛型 component definition 返回 {actual:?}"
    )]
    ComponentDefinitionIdentityMismatch {
        field_name: Option<&'static str>,
        requested: ServiceIdentifier,
        actual: ServiceIdentifier,
    },

    #[error("服务 {provider:?} 的字段 {field_name:?} 尚不支持 Arena 输入准备")]
    UnsupportedFieldInput {
        provider: ServiceIdentifier,
        field_name: Option<&'static str>,
    },

    #[error("服务 {identifier:?} 在当前实例化过程中发生递归重入")]
    ReentrantInstantiation { identifier: ServiceIdentifier },

    #[error("服务 {provider:?} 的 trait 字段 {field_name:?} 存在多个候选绑定 {candidates:?}")]
    AmbiguousTraitBinding {
        provider: ServiceIdentifier,
        field_name: Option<&'static str>,
        candidates: Vec<ServiceIdentifier>,
    },

    #[error("为服务 {provider:?} 的字段 {field_name:?} 准备构造输入失败")]
    PrepareInput {
        provider: ServiceIdentifier,
        field_name: Option<&'static str>,
        #[source]
        source: ActivationError,
    },

    #[error("服务 {identifier:?} 的宏生成构造器执行失败")]
    Constructor {
        identifier: ServiceIdentifier,
        #[source]
        source: ActivationError,
    },

    #[error(transparent)]
    Arena(#[from] ArenaError),
}

/// 由 linkme 收集的 component 集合及其直接实例化入口。
pub struct ServiceCollection {
    components: Vec<StructComponent>,
    bindings: Vec<InterfaceBinding>,
}

/// 已解析字段在调用宏生成 constructor 时应从哪里取得输入。
///
/// 这里不保存依赖图或构建计划；它只在当前 component 的两次顺序循环之间短暂保存已选
/// 定的 Arena 地址来源及单态化输入准备函数。
#[derive(Clone, Copy)]
enum ResolvedFieldInput {
    Arena {
        prepare_input: PrepareInput,
        service_identifier: ServiceIdentifier,
    },
    Absent {
        prepare_input: PrepareInput,
    },
}

/// 某个 trait 注入请求在当前 collection 中选出的 concrete provider。
#[derive(Clone, Copy)]
struct TraitBindingCandidate {
    binding: InterfaceBinding,
    service_identifier: ServiceIdentifier,
    primary: bool,
}

impl ServiceCollection {
    /// 收集当前链接单元内所有闭合 `#[injectable]` component。
    ///
    /// 开放泛型 provider 不会直接出现在此集合中；当某字段需要
    /// `Repository<UserEntity>` 这类闭合实例时，会通过该字段的
    /// `component_definition` callback 按需取得 component。
    pub fn new() -> Self {
        Self {
            components: REFLECT_METADATA_INJECTABLE
                .iter()
                .map(|component| component())
                .collect(),
            bindings: REFLECT_METADATA_BIND
                .iter()
                .map(|binding| binding())
                .collect(),
        }
    }

    /// 为无 key 的根服务创建一个新的 Arena 并完成其所需实例化。
    ///
    /// 返回的 Arena 拥有所有服务；请通过 [`Arena::get`] 读取根实例。若本次构造失败，
    /// 临时 Arena 会在返回前逆序析构已成功构造的服务。
    pub fn instantiate<T>(&self) -> Result<Arena, InstantiationError>
    where
        T: Injectable,
    {
        let mut arena = Arena::new();
        self.instantiate_into::<T>(&mut arena)?;
        Ok(arena)
    }

    /// 将无 key 的根服务及其实际依赖构造到既有 Arena 中。
    ///
    /// 这是直接按需递归，不构建或缓存依赖图。重复请求已提交的同一服务身份时会复用
    /// Arena 中的实例；运行期重入会返回错误而不是递归至栈溢出。
    pub fn instantiate_into<T>(&self, arena: &mut Arena) -> Result<(), InstantiationError>
    where
        T: Injectable,
    {
        self.instantiate_identifier(ServiceIdentifier::from(ServiceType::create::<T>()), arena)
    }

    /// 返回当前链接单元中收集到的闭合 component 数量。
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    fn instantiate_identifier(
        &self,
        identifier: ServiceIdentifier,
        arena: &mut Arena,
    ) -> Result<(), InstantiationError> {
        if arena.contains(identifier) {
            return Ok(());
        }

        let component = self
            .find_component(identifier)
            .cloned()
            .ok_or(InstantiationError::RootComponentNotFound { identifier })?;
        self.instantiate_component(component, arena)
    }

    fn instantiate_component(
        &self,
        component: StructComponent,
        arena: &mut Arena,
    ) -> Result<(), InstantiationError> {
        let identifier = component.service_identifier;
        if arena.contains(identifier) {
            return Ok(());
        }

        if !arena.begin_building(identifier) {
            return Err(InstantiationError::ReentrantInstantiation { identifier });
        }

        let result = (|| {
            let mut resolved_fields = Vec::with_capacity(component.field_injections.len());
            for field in &component.field_injections {
                resolved_fields.push(self.resolve_field_input(identifier, field, arena)?);
            }

            let mut context = ConstructionContext::new();
            for (field, resolved) in component
                .field_injections
                .iter()
                .zip(resolved_fields.into_iter())
            {
                let prepared = match resolved {
                    ResolvedFieldInput::Arena {
                        prepare_input,
                        service_identifier,
                    } => prepare_input(
                        &mut context,
                        InputPosition(field.dependency_position),
                        arena.lookup(service_identifier),
                    ),
                    ResolvedFieldInput::Absent { prepare_input } => {
                        prepare_input(&mut context, InputPosition(field.dependency_position), None)
                    }
                };

                prepared.map_err(|source| InstantiationError::PrepareInput {
                    provider: identifier,
                    field_name: field.field_name,
                    source,
                })?;
            }

            let output = (component.constructor)(context)
                .map_err(|source| InstantiationError::Constructor { identifier, source })?;
            arena.commit(identifier, output)?;
            Ok(())
        })();

        arena.finish_building(identifier);
        result
    }

    fn resolve_field_input(
        &self,
        provider: ServiceIdentifier,
        field: &FieldInjection,
        arena: &mut Arena,
    ) -> Result<ResolvedFieldInput, InstantiationError> {
        match field.target {
            FieldInjectionTarget::Concrete => {
                let prepare_input =
                    field
                        .prepare_input
                        .ok_or(InstantiationError::UnsupportedFieldInput {
                            provider,
                            field_name: field.field_name,
                        })?;
                self.instantiate_concrete_field_dependency(provider, field, arena)?;
                Ok(ResolvedFieldInput::Arena {
                    prepare_input,
                    service_identifier: field.service_identifier,
                })
            }
            FieldInjectionTarget::TraitObject => {
                self.resolve_trait_field_input(provider, field, arena)
            }
            FieldInjectionTarget::Unsupported => Err(InstantiationError::UnsupportedFieldInput {
                provider,
                field_name: field.field_name,
            }),
        }
    }

    fn instantiate_concrete_field_dependency(
        &self,
        provider: ServiceIdentifier,
        field: &FieldInjection,
        arena: &mut Arena,
    ) -> Result<(), InstantiationError> {
        let dependency = field.service_identifier;
        if arena.contains(dependency) {
            return Ok(());
        }

        // 已收集的闭合服务优先于泛型 fallback；这为后续显式 provider 覆盖保留了
        // 自然的运行时顺序，而无需建立全局选择图。
        if let Some(component) = self.find_component(dependency).cloned() {
            return self.instantiate_component(component, arena);
        }

        if let Some(component_definition) = field.component_definition {
            let component = component_definition();
            if component.service_identifier != dependency {
                return Err(InstantiationError::ComponentDefinitionIdentityMismatch {
                    field_name: field.field_name,
                    requested: dependency,
                    actual: component.service_identifier,
                });
            }
            return self.instantiate_component(component, arena);
        }

        if field.optional {
            return Ok(());
        }

        Err(InstantiationError::MissingDependency {
            provider,
            dependency,
        })
    }

    fn resolve_trait_field_input(
        &self,
        provider: ServiceIdentifier,
        field: &FieldInjection,
        arena: &mut Arena,
    ) -> Result<ResolvedFieldInput, InstantiationError> {
        if let Some(candidate) = self.select_trait_binding(provider, field, arena)? {
            if !arena.contains(candidate.service_identifier) {
                let component = self
                    .find_component(candidate.service_identifier)
                    .cloned()
                    .ok_or(InstantiationError::MissingDependency {
                        provider,
                        dependency: field.service_identifier,
                    })?;
                self.instantiate_component(component, arena)?;
            }

            let prepare_input = if field.optional {
                candidate.binding.prepare_optional
            } else {
                candidate.binding.prepare_required
            };
            return Ok(ResolvedFieldInput::Arena {
                prepare_input,
                service_identifier: candidate.service_identifier,
            });
        }

        if field.optional {
            let prepare_input =
                field
                    .prepare_input
                    .ok_or(InstantiationError::UnsupportedFieldInput {
                        provider,
                        field_name: field.field_name,
                    })?;
            return Ok(ResolvedFieldInput::Absent { prepare_input });
        }

        Err(InstantiationError::MissingDependency {
            provider,
            dependency: field.service_identifier,
        })
    }

    fn select_trait_binding(
        &self,
        provider: ServiceIdentifier,
        field: &FieldInjection,
        arena: &Arena,
    ) -> Result<Option<TraitBindingCandidate>, InstantiationError> {
        let mut candidates = Vec::new();

        for binding in &self.bindings {
            if binding.trait_type != field.service_identifier.service_type {
                continue;
            }

            // `#[bind]` 不自行拥有 key；它继承其 concrete `#[injectable]` provider 的
            // key。因此 trait 字段请求的 key 同时决定要激活的 concrete service identity。
            let service_identifier =
                ServiceIdentifier::new(field.service_identifier.service_key, binding.service_type);
            let component = self.find_component(service_identifier);
            if component.is_none() && !arena.contains(service_identifier) {
                continue;
            }

            // 对同一 concrete provider 重复标注相同 bind 时无需制造假的二义性。
            if candidates.iter().any(|candidate: &TraitBindingCandidate| {
                candidate.service_identifier == service_identifier
            }) {
                continue;
            }

            candidates.push(TraitBindingCandidate {
                binding: *binding,
                service_identifier,
                primary: component
                    .map(|component| component.primary)
                    .unwrap_or(false),
            });
        }

        match candidates.as_slice() {
            [] => Ok(None),
            [candidate] => Ok(Some(*candidate)),
            _ => {
                let primary_candidates: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|candidate| candidate.primary)
                    .collect();
                if let [candidate] = primary_candidates.as_slice() {
                    return Ok(Some(*candidate));
                }

                Err(InstantiationError::AmbiguousTraitBinding {
                    provider,
                    field_name: field.field_name,
                    candidates: candidates
                        .into_iter()
                        .map(|candidate| candidate.service_identifier)
                        .collect(),
                })
            }
        }
    }

    fn find_component(&self, identifier: ServiceIdentifier) -> Option<&StructComponent> {
        // 不在此阶段做重复注册诊断；保持当前“按收集顺序最后一个闭合定义获选”的最小
        // 实例化语义，完整的候选选择与验证留给后续 composition 阶段。
        self.components
            .iter()
            .rev()
            .find(|component| component.service_identifier == identifier)
    }
}

impl Default for ServiceCollection {
    fn default() -> Self {
        Self::new()
    }
}

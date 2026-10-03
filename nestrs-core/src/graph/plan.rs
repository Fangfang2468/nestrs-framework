//! 装载工具链已经验证、已经完成选择的不可变 DI 执行计划。
//!
//! `cargo nestrs` 负责确定节点编号、输入目标、trait 投影、拓扑顺序与 Scope 能力。
//! 本模块只将这些固定编号与当前目标程序中的真实 typed adapter 地址接合；它不选择
//! 候选、不执行泛型物化、不分析依赖图，更不调用 constructor/factory/value/cleanup。
//! 生成的执行适配器回调只取得 Rust 类型身份和函数指针，每个程序入口装载一次。
//!
//! 编译器入口是私有、版本化的协议。所有写入只发生在 `CompiledApplication::load`
//! 的栈上装配器中，完成后整体消费并封存。后续每次 build 共享计划，各自建立独立的运行期 owner。

use ahash::AHashMap;
use std::sync::{Arc, OnceLock};

use super::{
    AbsentInput, CompiledDependency, CompiledNode, DependencyInput, NodePolicy, RootRoute,
    ValidatedGraph,
};
use crate::{
    InitializationMode, ServiceKey, ServiceLifetime, ServiceProviderOptions,
    activation::{
        InputSlot, LazyInputPlan,
        adapter::{ActivationAdapter, InputAdapter, ProjectionAdapter},
    },
    service::{ServiceIdentifier, ServiceSource},
};

/// 每个最终 binary/test 入口只有这一份不可变配置与计划；服务实例不存放在这里。
pub(crate) struct CompiledApplication {
    /// 编译器固化的入口启动默认设置。
    pub(crate) options: ServiceProviderOptions,

    /// 当前入口共享的不可变执行计划。
    pub(crate) graph: Arc<ValidatedGraph>,
}

impl CompiledApplication {
    /// OnceLock 只缓存不可变程序计划。第一次装载也不需要 Tokio，不持有任何实例或 owner。
    pub(crate) fn load() -> &'static Self {
        /// 当前最终入口共享的不可变装配结果，不缓存运行期实例。
        static APPLICATION: OnceLock<CompiledApplication> = OnceLock::new();
        APPLICATION.get_or_init(|| {
            #[allow(unused_mut)]
            let mut assembly = PlanAssembly::default();
            #[cfg(nestrs_compiler)]
            // SAFETY: driver 验证入口与下面全部 sink 的真实签名，只将当前 assembly 地址
            // 同步传给这些函数；业务源码不能引用入口，地址也不会被保存到程序全局状态。
            unsafe {
                __nestrs_reflect_v2((&mut assembly as *mut PlanAssembly).cast());
            }
            assembly.finish()
        })
    }
}

/// MIR 的普通 usize 标量表示缺席，避免编译器伪造 Option 的目标平台布局。
const ABSENT: usize = usize::MAX;

/// 装配期间暂存真实执行入口，等待编译计划给每个输入写入已确定的目标。
struct PendingNode {
    /// 已装配的节点基本信息，输入尚待编译计划填充。
    node: CompiledNode,

    /// 待按槽位消费的准确输入适配器，每项只能写入一次。
    adapters: Vec<Option<InputAdapter>>,

    /// 按固定输入编号保存的已写或尚缺输入描述。
    inputs: Vec<Option<CompiledDependency>>,
}

/// 接收编译器入口的顺序写入，核对执行协议后冻结为目标端计划。
#[derive(Default)]
struct PlanAssembly {
    /// 编译器固化的入口启动默认设置。
    options: ServiceProviderOptions,

    /// 按编译计划顺序追加的待完成节点。
    nodes: Vec<PendingNode>,

    /// 按计划编号保存的真实 concrete 到 trait 投影能力。
    bindings: Vec<ProjectionAdapter>,

    /// 完整类型和 key 到已选节点及投影的查询路由。
    routes: AHashMap<crate::service::ServiceIdentifier, RootRoute>,

    /// 编译期验证得到的依赖优先顺序。
    topological_order: Vec<usize>,

    /// 完整依赖图的去重反向邻接表，含延迟目标。
    dependents: Vec<Vec<usize>>,
}

impl PlanAssembly {
    /// # Safety
    /// output 必须唯一指向仍存活的本次装配器，返回借用不得超出当前 sink 调用。
    unsafe fn from_output<'a>(output: *mut ()) -> &'a mut Self {
        // SAFETY: 只有经过签名校验的编译器入口使用该协议，借用只覆盖当前 sink 调用。
        unsafe { &mut *output.cast::<Self>() }
    }

    /// 从固定标量解码查询限定符，不执行候选选择。
    fn key(kind: usize, name: &'static str, index: usize) -> Option<ServiceKey> {
        match kind {
            0 => None,
            1 => Some(ServiceKey::Named(name.to_owned())),
            2 => Some(ServiceKey::Indexed(index)),
            _ => panic!("Nestrs 编译计划包含无效 key 类别"),
        }
    }

    /// 从已验证标量装配入口默认设置，并拒绝零构造并发数。
    fn set_options(&mut self, eager: bool, concurrency: usize) {
        let options = ServiceProviderOptions {
            initialization: if eager {
                InitializationMode::Eager
            } else {
                InitializationMode::Lazy
            },
            max_concurrent_activations: std::num::NonZeroUsize::new(concurrency)
                .expect("Nestrs 编译计划的并发上限必须大于零"),
        };
        self.options = options;
    }

    /// 按编译期编号保存真实类型投影能力。
    fn push_binding(&mut self, binding: ProjectionAdapter) {
        self.bindings.push(binding);
    }

    /// 装配已选节点与 concrete 路由，并为其输入保留待写槽位。
    #[allow(clippy::too_many_arguments)]
    fn push_provider(
        &mut self,
        adapter: ActivationAdapter,
        lifetime: usize,
        key_kind: usize,
        key_name: &'static str,
        key_index: usize,
        initialization: usize,
        source_file: &'static str,
        source_line: usize,
        source_column: usize,
        requires_scope: bool,
    ) {
        let identifier = ServiceIdentifier::new(
            Self::key(key_kind, key_name, key_index),
            adapter.service_type,
        );
        let common = NodePolicy {
            lifetime: match lifetime {
                0 => ServiceLifetime::Singleton,
                1 => ServiceLifetime::Scoped,
                2 => ServiceLifetime::Transient,
                _ => panic!("Nestrs 编译计划包含无效生命周期"),
            },
            lazy: match initialization {
                0 => None,
                1 => Some(true),
                2 => Some(false),
                _ => panic!("Nestrs 编译计划包含无效初始化策略"),
            },
            source: ServiceSource::new(
                source_file,
                source_line.try_into().expect("Nestrs 编译计划源码行号越界"),
                source_column
                    .try_into()
                    .expect("Nestrs 编译计划源码列号越界"),
            ),
            cleanup: adapter.cleanup,
        };
        let count = adapter.inputs.len();
        let provider = self.nodes.len();
        assert!(
            self.routes
                .insert(
                    identifier.clone(),
                    RootRoute {
                        provider,
                        projection: None
                    }
                )
                .is_none(),
            "Nestrs 编译计划包含重复 concrete 路由"
        );
        self.nodes.push(PendingNode {
            node: CompiledNode {
                identifier,
                common,
                dependencies: Vec::new(),
                constructor: adapter.constructor,
                requires_scope,
            },
            adapters: adapter.inputs.into_iter().map(Some).collect(),
            inputs: (0..count).map(|_| None).collect(),
        });
        self.dependents.push(Vec::new());
    }

    /// 写入一个已选输入，核对目标、可选形态、准确类型与投影协议。
    #[allow(clippy::too_many_arguments)]
    fn set_input(
        &mut self,
        provider: usize,
        slot: usize,
        target: usize,
        binding: usize,
        optional: bool,
        key_kind: usize,
        key_name: &'static str,
        key_index: usize,
        label: &'static str,
    ) {
        let adapter = self.nodes[provider].adapters[slot]
            .take()
            .expect("Nestrs 编译计划重复写入输入");
        assert_eq!(
            optional,
            adapter.kind.is_optional(),
            "Nestrs 编译计划的可选性与输入适配器不一致"
        );
        let target = (target != ABSENT).then_some(target);
        if let Some(target) = target {
            assert!(target < self.nodes.len(), "Nestrs 编译计划的目标节点越界");
        } else {
            assert!(optional, "Nestrs 编译计划将必选输入标记为缺席");
        }

        // 此处没有候选选择：编译器已经用 binding 编号指定唯一投影。普通和延迟交付
        // 共用同一个经 Rust 检查的 projector；optional 不影响目标或投影选择。
        let project = if binding != ABSENT {
            let binding = self.bindings[binding];
            let target = target.expect("Nestrs 编译计划的投影输入没有目标");
            assert_eq!(
                binding.trait_type, adapter.service_type,
                "Nestrs 编译计划的接口类型不一致"
            );
            assert_eq!(
                binding.concrete_type, self.nodes[target].node.identifier.service_type,
                "Nestrs 编译计划的投影来源不一致"
            );
            Some(binding.project)
        } else {
            if let Some(target) = target {
                assert_eq!(
                    adapter.service_type, self.nodes[target].node.identifier.service_type,
                    "Nestrs 编译计划的直接输入类型不一致"
                );
            }
            adapter.project
        };

        // 描述在程序计划首次装载时创建一次。所有运行期 occurrence 只克隆 Arc，
        // 不再重复复制消费者名称、key、源码位置与投影协议；状态仍属于各自字段。
        let label = (!label.is_empty()).then_some(label);
        let input_slot = InputSlot::new(slot);
        let requested = ServiceIdentifier::new(
            Self::key(key_kind, key_name, key_index),
            adapter.service_type,
        );

        // 可选字段只存在于编译器装配协议。此处一次确定最终交付分支，worker 随后
        // 直接匹配该分支；不会再遇到“有延迟标记但没有对应计划”的半完成执行节点。
        // 缺席保留普通/延迟类别，准确 T 则由 requested 保留，读取 None 时仍核对两者。
        let input = match (target, adapter.kind.is_lazy()) {
            (None, false) => DependencyInput::Absent(AbsentInput::Immediate),
            (None, true) => DependencyInput::Absent(AbsentInput::Lazy),
            (Some(target), false) => DependencyInput::Immediate {
                target,
                project: project.expect("Nestrs 编译计划的普通输入缺少直接类型化投影"),
            },
            (Some(target), true) => {
                let consumer = &self.nodes[provider].node;
                DependencyInput::Lazy {
                    plan: Arc::new(LazyInputPlan {
                        provider: target,
                        consumer: consumer.identifier.clone(),
                        source: consumer.common.source,
                        label,
                        input: input_slot,
                        project: project.expect("Nestrs 编译计划的延迟输入缺少直接类型化投影"),
                    }),
                }
            }
        };
        self.nodes[provider].inputs[slot] = Some(CompiledDependency {
            slot: input_slot,
            requested,
            optional,
            input,
            label,
        });
    }

    /// 将已验证接口路由接到具体节点及真实投影能力。
    fn push_trait_route(&mut self, provider: usize, binding: usize) {
        let target = &self.nodes[provider].node;
        let binding = self.bindings[binding];
        assert_eq!(
            binding.concrete_type, target.identifier.service_type,
            "Nestrs 编译计划的根投影来源不一致"
        );
        let identifier = crate::service::ServiceIdentifier::new(
            target.identifier.service_key.clone(),
            binding.trait_type,
        );
        assert!(
            self.routes
                .insert(
                    identifier,
                    RootRoute {
                        provider,
                        projection: Some(binding.project)
                    }
                )
                .is_none(),
            "Nestrs 编译计划包含重复接口路由"
        );
    }

    /// 按编译计划追加依赖优先的节点次序。
    fn push_order(&mut self, provider: usize) {
        assert!(provider < self.nodes.len(), "Nestrs 编译计划的顺序节点越界");
        self.topological_order.push(provider);
    }

    /// 保存完整图的反向依赖关系，供关闭次序使用。
    fn push_dependent(&mut self, dependency: usize, consumer: usize) {
        assert!(consumer < self.nodes.len(), "Nestrs 编译计划的消费者越界");
        self.dependents[dependency].push(consumer);
    }

    /// 检查装配完整性并冻结计划；不执行候选选择或用户构造。
    fn finish(self) -> CompiledApplication {
        // 这里只核对协议完整性，不重新运行图验证。编译器必须写入每一个槽位；如果
        // 工具与 core 的协议不匹配，应在启动时明确失败，不能交付不完整的输入计划。
        assert_eq!(
            self.topological_order.len(),
            self.nodes.len(),
            "Nestrs 编译计划缺少节点顺序"
        );
        let nodes = self
            .nodes
            .into_iter()
            .map(|mut pending| {
                assert!(
                    pending.adapters.iter().all(Option::is_none),
                    "Nestrs 编译计划缺少输入选择"
                );
                pending.node.dependencies = pending
                    .inputs
                    .into_iter()
                    .map(|input| input.expect("Nestrs 编译计划缺少输入槽位"))
                    .collect();
                pending.node
            })
            .collect();
        CompiledApplication {
            options: self.options,
            graph: Arc::new(ValidatedGraph {
                nodes,
                routes: self.routes,
                topological_order: self.topological_order,
                dependents: self.dependents,
            }),
        }
    }
}

#[cfg(nestrs_compiler)]
unsafe extern "Rust" {
    /// driver 为当前最终入口生成的唯一写入函数，只同步填充栈上的装配器。
    fn __nestrs_reflect_v2(output: *mut ());
}

/// 将最终入口的编译时配置写入计划，不读取部署环境中的 Cargo.toml。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议。
pub unsafe fn plan_set_options_v2(output: *mut (), eager: bool, concurrency: usize) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.set_options(eager, concurrency);
}

/// 按编译器已经确定的顺序装载投影地址；这里的下标就是后续 sink 的 binding 编号。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议。
pub unsafe fn plan_push_binding(output: *mut (), binding: ProjectionAdapter) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.push_binding(binding);
}

/// 接合已选节点的执行能力和固定策略，不接收服务声明或泛型蓝图。
///
/// 生命周期与初始化策略采用标量协议，避免编译器伪造 Rust 枚举内存布局。
/// key/源码位置只服务于查询与诊断；primary 等编译选择事实不进入该入口。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议；全部标量来自同一完整执行计划。
#[allow(clippy::too_many_arguments)]
pub unsafe fn plan_push_provider(
    output: *mut (),
    adapter: ActivationAdapter,
    lifetime: usize,
    key_kind: usize,
    key_name: &'static str,
    key_index: usize,
    initialization: usize,
    source_file: &'static str,
    source_line: usize,
    source_column: usize,
    requires_scope: bool,
) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.push_provider(
        adapter,
        lifetime,
        key_kind,
        key_name,
        key_index,
        initialization,
        source_file,
        source_line,
        source_column,
        requires_scope,
    );
}

/// 固定一个输入。target 与 binding 使用独立编号，MAX 分别代表缺席和无需投影。
/// 重复依赖仍逐槽位写入；不能把拓扑去重后的边直接当作真实构造输入。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议，所有编号已经通过编译器的类型与图检查。
#[allow(clippy::too_many_arguments)]
pub unsafe fn plan_set_input(
    output: *mut (),
    provider: usize,
    slot: usize,
    target: usize,
    binding: usize,
    optional: bool,
    key_kind: usize,
    key_name: &'static str,
    key_index: usize,
    label: &'static str,
) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.set_input(
        provider, slot, target, binding, optional, key_kind, key_name, key_index, label,
    );
}

/// 直接装入编译器选择的 trait 根路由；key 沿用目标 Provider 的精确 key。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议，provider/binding 来自同一个完整计划。
pub unsafe fn plan_push_trait_route(output: *mut (), provider: usize, binding: usize) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.push_trait_route(provider, binding);
}

/// 写入依赖优先的拓扑顺序；运行期预热直接遍历该顺序。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议，provider 是当前计划的有效节点编号。
pub unsafe fn plan_push_order(output: *mut (), provider: usize) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.push_order(provider);
}

/// 写入去重且排序后的反向边。它只承担拓扑关系，不替代上面的全部输入槽位。
///
/// # Safety
/// output 必须满足本模块的编译器装配协议，两个编号都是当前计划的有效节点。
pub unsafe fn plan_push_dependent(output: *mut (), dependency: usize, consumer: usize) {
    // SAFETY: 入口同步调用，且 output 仍然唯一指向当前装配器。
    unsafe { PlanAssembly::from_output(output) }.push_dependent(dependency, consumer);
}

#[cfg(test)]
#[path = "../../tests/unit/graph/plan.rs"]
mod tests;

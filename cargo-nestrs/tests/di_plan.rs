//! 独立于 rustc 与运行时的语义计划回归，覆盖选择规则及深图的迭代算法。

use cargo_nestrs::di_plan::{
    Binding, DiagnosticKind as Kind, Input, Key, Lifetime, Plan, Provider, Type, compile,
};

fn types(count: usize) -> Vec<Type> {
    (0..count)
        .map(|id| Type {
            id,
            name: format!("Service{id}"),
        })
        .collect()
}

fn provider(type_id: usize, dependencies: &[usize]) -> Provider {
    Provider {
        type_id,
        key: Key::Default,
        lifetime: Lifetime::Transient,
        primary: false,
        lazy: None,
        source: format!("service_{type_id}.rs:17:3"),
        inputs: dependencies
            .iter()
            .enumerate()
            .map(|(slot, &type_id)| Input {
                type_id,
                key: Key::Default,
                slot,
                optional: false,
                lazy: false,
                label: format!("field{slot}"),
            })
            .collect(),
    }
}

fn binding(concrete: usize, interface: usize) -> Binding {
    Binding {
        concrete,
        interface,
        source: format!("binding_{concrete}_{interface}.rs:9:1"),
    }
}

fn route(plan: &Plan, type_id: usize, key: Key) -> (usize, Option<usize>) {
    let route = plan
        .routes
        .iter()
        .find(|route| route.type_id == type_id && route.key == key)
        .unwrap();
    (route.provider, route.binding)
}

fn fails(types: &[Type], providers: &[Provider], bindings: &[Binding], kind: Kind) -> String {
    compile(types, providers, bindings)
        .expect_err("模型应被拒绝")
        .into_iter()
        .find(|error| error.kind == kind)
        .expect("应保留指定诊断类型")
        .message
}

#[test]
fn empty_declarations_produce_an_empty_executable_plan() {
    let plan = compile(&[], &[], &[]).unwrap();
    assert!(plan.inputs.is_empty());
    assert!(plan.routes.is_empty());
    assert!(plan.order.is_empty());
}

#[test]
fn provider_initialization_policies_never_remove_nodes_routes_or_dependency_edges() {
    let baseline = [provider(0, &[1]), provider(1, &[]), provider(2, &[])];
    let expected = compile(&types(3), &baseline, &[]).unwrap();
    for policy in [None, Some(true), Some(false)] {
        let mut providers = baseline.clone();
        for provider in &mut providers {
            provider.lazy = policy;
        }
        assert_eq!(compile(&types(3), &providers, &[]).unwrap(), expected);
        assert!(!providers[0].inputs[0].lazy, "服务策略不能改写普通依赖边");
    }
}

#[test]
fn lazy_providers_still_report_missing_cycle_and_scope_errors() {
    let mut missing = provider(0, &[1]);
    missing.lazy = Some(true);
    fails(&types(2), &[missing], &[], Kind::MissingDependency);

    let mut cycle = [provider(0, &[1]), provider(1, &[0])];
    for provider in &mut cycle {
        provider.lazy = Some(true);
    }
    fails(&types(2), &cycle, &[], Kind::Cycle);

    let mut singleton = provider(0, &[1]);
    singleton.lifetime = Lifetime::Singleton;
    singleton.lazy = Some(true);
    let mut scoped = provider(1, &[]);
    scoped.lifetime = Lifetime::Scoped;
    scoped.lazy = Some(true);
    fails(&types(2), &[singleton, scoped], &[], Kind::ScopeRequired);
}

#[test]
fn concrete_dependencies_are_selected_before_consumers() {
    let providers = [provider(0, &[1]), provider(1, &[2]), provider(2, &[])];
    let plan = compile(&types(3), &providers, &[]).unwrap();
    assert_eq!(plan.order, [2, 1, 0]);
    assert_eq!(plan.dependents, [vec![], vec![0], vec![1]]);
    assert_eq!(plan.inputs[0][0].target, Some(1));
    assert_eq!(route(&plan, 2, Key::Default), (2, None));
}

#[test]
fn duplicate_transient_inputs_keep_independent_slots_but_one_topology_edge() {
    let providers = [provider(0, &[1, 1]), provider(1, &[])];
    let plan = compile(&types(2), &providers, &[]).unwrap();
    assert_eq!(plan.inputs[0].len(), 2);
    assert_eq!(plan.inputs[0][0].target, Some(1));
    assert_eq!(plan.inputs[0][1].target, Some(1));
    assert_eq!(plan.dependents[1], [0]);
    assert_eq!(plan.order, [1, 0]);
}

#[test]
fn unreferenced_provider_missing_dependency_is_still_invalid() {
    let providers = [provider(0, &[]), provider(1, &[2])];
    let message = fails(&types(3), &providers, &[], Kind::MissingDependency);
    assert!(message.contains("Service1") && message.contains("Service2"));
    assert!(message.contains("field0") && message.contains("service_1.rs:17:3"));
}

#[test]
fn primary_does_not_legalize_duplicate_concrete_provider() {
    let mut selected = provider(0, &[]);
    selected.primary = true;
    fails(
        &types(1),
        &[provider(0, &[]), selected],
        &[],
        Kind::DuplicateProvider,
    );
}

#[test]
fn same_type_with_distinct_keys_creates_distinct_routes() {
    let mut named = provider(0, &[]);
    named.key = Key::Named("7".into());
    let mut indexed = provider(0, &[]);
    indexed.key = Key::Indexed(7);
    let plan = compile(&types(1), &[provider(0, &[]), named, indexed], &[]).unwrap();
    assert_eq!(route(&plan, 0, Key::Default), (0, None));
    assert_eq!(route(&plan, 0, Key::Named("7".into())), (1, None));
    assert_eq!(route(&plan, 0, Key::Indexed(7)), (2, None));
}

#[test]
fn default_key_never_falls_back_to_a_named_provider() {
    let mut dependency = provider(1, &[]);
    dependency.key = Key::Named("only".into());
    fails(
        &types(2),
        &[provider(0, &[1]), dependency],
        &[],
        Kind::MissingDependency,
    );
}

#[test]
fn numeric_keys_preserve_all_u128_bits() {
    let mut consumer = provider(0, &[1]);
    consumer.inputs[0].key = Key::Indexed(u128::MAX);
    let mut target = provider(1, &[]);
    target.key = Key::Indexed(u128::MAX);
    let plan = compile(&types(2), &[consumer, target], &[]).unwrap();
    assert_eq!(route(&plan, 1, Key::Indexed(u128::MAX)), (1, None));
}

#[test]
fn one_trait_candidate_selects_its_real_binding_index() {
    let providers = [provider(0, &[3]), provider(1, &[]), provider(2, &[])];
    let bindings = [binding(2, 4), binding(1, 3)];
    let plan = compile(&types(5), &providers, &bindings).unwrap();
    assert_eq!(route(&plan, 3, Key::Default), (1, Some(1)));
    assert_eq!(plan.inputs[0][0].binding, Some(1));
}

#[test]
fn trait_ambiguity_is_an_error_even_without_consumers() {
    fails(
        &types(3),
        &[provider(0, &[]), provider(1, &[])],
        &[binding(0, 2), binding(1, 2)],
        Kind::AmbiguousTrait,
    );
}

#[test]
fn exactly_one_primary_selects_a_trait_candidate() {
    let mut primary = provider(1, &[]);
    primary.primary = true;
    let plan = compile(
        &types(3),
        &[provider(0, &[]), primary],
        &[binding(0, 2), binding(1, 2)],
    )
    .unwrap();
    assert_eq!(route(&plan, 2, Key::Default), (1, Some(1)));
    assert_eq!(route(&plan, 0, Key::Default), (0, None));
}

#[test]
fn multiple_primary_candidates_are_rejected() {
    let mut first = provider(0, &[]);
    let mut second = provider(1, &[]);
    first.primary = true;
    second.primary = true;
    let message = fails(
        &types(3),
        &[first, second],
        &[binding(0, 2), binding(1, 2)],
        Kind::AmbiguousTrait,
    );
    assert!(message.contains("多个 primary"));
}

#[test]
fn primary_is_selected_only_among_exact_key_candidates() {
    let mut named = provider(0, &[]);
    named.primary = true;
    named.key = Key::Named("other".into());
    let plan = compile(
        &types(3),
        &[named, provider(1, &[])],
        &[binding(0, 2), binding(1, 2)],
    )
    .unwrap();
    assert_eq!(route(&plan, 2, Key::Default), (1, Some(1)));
    assert_eq!(route(&plan, 2, Key::Named("other".into())), (0, Some(0)));
}

#[test]
fn duplicate_binding_reports_both_sources() {
    let first = binding(0, 1);
    let mut second = first.clone();
    second.source = "duplicate.rs:22:4".into();
    let message = fails(
        &types(2),
        &[provider(0, &[])],
        &[first, second],
        Kind::DuplicateBinding,
    );
    assert!(message.contains("binding_0_1.rs:9:1") && message.contains("duplicate.rs:22:4"));
}

#[test]
fn binding_with_no_concrete_provider_is_rejected() {
    fails(
        &types(3),
        &[provider(0, &[])],
        &[binding(1, 2)],
        Kind::OrphanBinding,
    );
}

#[test]
fn absent_optional_input_has_no_target_or_binding() {
    let mut consumer = provider(0, &[1]);
    consumer.inputs[0].optional = true;
    let plan = compile(&types(2), &[consumer], &[]).unwrap();
    assert_eq!(plan.inputs[0][0].target, None);
    assert_eq!(plan.inputs[0][0].binding, None);
    assert_eq!(plan.order, [0]);
}

#[test]
fn optional_input_does_not_suppress_trait_ambiguity() {
    let mut consumer = provider(0, &[3]);
    consumer.inputs[0].optional = true;
    fails(
        &types(4),
        &[consumer, provider(1, &[]), provider(2, &[])],
        &[binding(1, 3), binding(2, 3)],
        Kind::AmbiguousTrait,
    );
}

#[test]
fn absent_optional_lazy_input_does_not_require_a_scope() {
    let mut consumer = provider(0, &[1]);
    consumer.lifetime = Lifetime::Singleton;
    consumer.inputs[0].optional = true;
    consumer.inputs[0].lazy = true;
    let plan = compile(&types(2), &[consumer], &[]).unwrap();
    assert_eq!(plan.requires_scope, [false]);
}

#[test]
fn optional_existing_input_still_participates_in_a_cycle() {
    let mut first = provider(0, &[1]);
    first.inputs[0].optional = true;
    fails(&types(2), &[first, provider(1, &[0])], &[], Kind::Cycle);
}

#[test]
fn gaps_and_duplicate_input_slots_are_rejected() {
    let mut consumer = provider(0, &[1, 1]);
    consumer.inputs[0].slot = 4;
    consumer.inputs[1].slot = 4;
    let errors = compile(&types(2), &[consumer, provider(1, &[])], &[]).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.kind == Kind::InvalidMetadata)
            .count(),
        2
    );
}

#[test]
fn scope_capability_crosses_transient_and_lazy_dependencies() {
    let mut consumer = provider(0, &[1]);
    consumer.inputs[0].lazy = true;
    let mut scoped = provider(2, &[]);
    scoped.lifetime = Lifetime::Scoped;
    let plan = compile(&types(3), &[consumer, provider(1, &[2]), scoped], &[]).unwrap();
    assert_eq!(plan.requires_scope, [true, true, true]);
    assert_eq!(plan.order, [2, 1, 0]);
}

#[test]
fn singleton_can_depend_on_transients_without_scoped_descendants() {
    let mut singleton = provider(0, &[1]);
    singleton.lifetime = Lifetime::Singleton;
    let plan = compile(&types(2), &[singleton, provider(1, &[])], &[]).unwrap();
    assert_eq!(plan.requires_scope, [false, false]);
}

#[test]
fn singleton_scope_conflict_keeps_the_complete_input_path() {
    let mut singleton = provider(0, &[1]);
    singleton.lifetime = Lifetime::Singleton;
    singleton.inputs[0].lazy = true;
    let mut scoped = provider(2, &[]);
    scoped.lifetime = Lifetime::Scoped;
    let message = fails(
        &types(3),
        &[singleton, provider(1, &[2]), scoped],
        &[],
        Kind::ScopeRequired,
    );
    for expected in [
        "Service0",
        "Service1",
        "Service2",
        "field0",
        "[lazy]",
        "service_2.rs:17:3",
    ] {
        assert!(message.contains(expected), "诊断缺少 {expected}");
    }
}

#[test]
fn optional_existing_scoped_input_remains_a_singleton_error() {
    let mut singleton = provider(0, &[1]);
    singleton.lifetime = Lifetime::Singleton;
    singleton.inputs[0].optional = true;
    let mut scoped = provider(1, &[]);
    scoped.lifetime = Lifetime::Scoped;
    fails(&types(2), &[singleton, scoped], &[], Kind::ScopeRequired);
}

#[test]
fn cycle_report_finds_the_actual_ring_without_its_dependent_tail() {
    let providers = [provider(0, &[1]), provider(1, &[2]), provider(2, &[1])];
    let message = fails(&types(3), &providers, &[], Kind::Cycle);
    assert!(!message.contains("Service0"));
    assert!(message.contains("Service1") && message.contains("Service2"));
}

#[test]
fn keyed_lazy_self_cycle_reports_key_field_and_source() {
    let mut cyclic = provider(0, &[0]);
    cyclic.key = Key::Named("ring".into());
    cyclic.inputs[0].key = cyclic.key.clone();
    cyclic.inputs[0].lazy = true;
    let message = fails(&types(1), &[cyclic], &[], Kind::Cycle);
    for expected in ["ring", "field0", "service_0.rs:17:3", "[lazy]"] {
        assert!(message.contains(expected));
    }
}

#[test]
fn separate_cycles_are_both_reported() {
    let providers = [provider(0, &[0]), provider(1, &[2]), provider(2, &[1])];
    let errors = compile(&types(3), &providers, &[]).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .filter(|error| error.kind == Kind::Cycle)
            .count(),
        2
    );
}

#[test]
fn diamond_dependencies_have_stable_order_and_unique_reverse_edges() {
    let providers = [
        provider(0, &[1, 2]),
        provider(1, &[3]),
        provider(2, &[3]),
        provider(3, &[]),
    ];
    let plan = compile(&types(4), &providers, &[]).unwrap();
    assert_eq!(plan.order, [3, 1, 2, 0]);
    assert_eq!(plan.dependents[3], [1, 2]);
}

#[test]
fn successful_plans_repeat_with_frozen_absence_and_preserve_provider_indices() {
    let mut consumer = provider(0, &[1, 2]);
    consumer.inputs[1].optional = true;
    let target = provider(1, &[]);
    for providers in [[consumer.clone(), target.clone()], [target, consumer]] {
        let consumer = providers.iter().position(|node| node.type_id == 0).unwrap();
        let target = providers.iter().position(|node| node.type_id == 1).unwrap();
        let plan = compile(&types(3), &providers, &[]).unwrap();
        assert_eq!(compile(&types(3), &providers, &[]).unwrap(), plan);
        // 前端排序后的输入编号是 ABI；纯模型不得为了稳定性偷偷重新编号。
        assert_eq!(route(&plan, 0, Key::Default), (consumer, None));
        assert_eq!(route(&plan, 1, Key::Default), (target, None));
        assert_eq!(plan.inputs[consumer][0].target, Some(target));
        assert_eq!(plan.inputs[consumer][1].target, None);
        assert_eq!(plan.inputs[consumer][1].binding, None);
        assert_eq!(plan.dependents[target], [consumer]);
        assert_eq!(plan.order, [target, consumer]);
    }
}

#[test]
fn lazy_optional_traits_keep_ambiguity_and_scope_errors() {
    let mut consumer = provider(0, &[2]);
    consumer.inputs[0].optional = true;
    consumer.inputs[0].lazy = true;
    fails(
        &types(4),
        &[consumer.clone(), provider(1, &[]), provider(3, &[])],
        &[binding(1, 2), binding(3, 2)],
        Kind::AmbiguousTrait,
    );
    consumer.lifetime = Lifetime::Singleton;
    let mut scoped = provider(1, &[]);
    scoped.lifetime = Lifetime::Scoped;
    let message = fails(
        &types(3),
        &[consumer, scoped],
        &[binding(1, 2)],
        Kind::ScopeRequired,
    );
    assert!(message.contains("[lazy]"));
}

#[test]
fn malformed_lazy_slots_keep_missing_and_topological_diagnostics() {
    for cyclic in [false, true] {
        let mut consumer = provider(0, &[1, 2]);
        consumer.lifetime = Lifetime::Singleton;
        consumer.inputs[0].lazy = true;
        consumer.inputs[0].slot = 1;
        let (target, expected) = if cyclic {
            (provider(1, &[0]), Kind::Cycle)
        } else {
            let mut target = provider(1, &[]);
            target.lifetime = Lifetime::Scoped;
            (target, Kind::ScopeRequired)
        };
        let errors = compile(&types(3), &[consumer, target], &[]).unwrap_err();
        for kind in [Kind::InvalidMetadata, Kind::MissingDependency, expected] {
            assert!(
                errors.iter().any(|error| error.kind == kind),
                "缺少 {kind:?}: {errors:?}"
            );
        }
    }
}

#[test]
fn actual_ids_define_identity_even_for_equal_names_and_sparse_ids() {
    let types = [
        Type {
            id: 700,
            name: "Same".into(),
        },
        Type {
            id: 99,
            name: "Same".into(),
        },
    ];
    let providers = [provider(700, &[99]), provider(99, &[])];
    let plan = compile(&types, &providers, &[]).unwrap();
    assert_eq!(plan.inputs[0][0].target, Some(1));
    assert_eq!(plan.routes.len(), 2);
}

#[test]
fn unknown_types_and_duplicate_type_ids_are_invalid_metadata() {
    fails(&[], &[provider(4, &[])], &[], Kind::InvalidMetadata);
    fails(&types(1), &[provider(0, &[4])], &[], Kind::InvalidMetadata);
    fails(
        &types(1),
        &[provider(0, &[])],
        &[binding(0, 4)],
        Kind::InvalidMetadata,
    );
    fails(
        &[types(1)[0].clone(), types(1)[0].clone()],
        &[],
        &[],
        Kind::InvalidMetadata,
    );
}

#[test]
fn invalid_binding_type_roles_are_rejected() {
    fails(
        &types(1),
        &[provider(0, &[])],
        &[binding(0, 0)],
        Kind::InvalidMetadata,
    );
    fails(
        &types(2),
        &[provider(0, &[]), provider(1, &[])],
        &[binding(0, 1)],
        Kind::InvalidMetadata,
    );
}

#[test]
fn diagnostics_are_repeatable_and_keep_independent_errors() {
    let providers = [provider(0, &[1]), provider(0, &[])];
    let first = compile(&types(2), &providers, &[]).unwrap_err();
    assert!(
        first
            .iter()
            .any(|error| error.kind == Kind::DuplicateProvider)
    );
    assert!(
        first
            .iter()
            .any(|error| error.kind == Kind::MissingDependency)
    );
    assert_eq!(compile(&types(2), &providers, &[]).unwrap_err(), first);
}

#[test]
fn twenty_thousand_nodes_plan_and_scope_analysis_fit_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let count = 20_000;
            let mut providers: Vec<_> = (0..count)
                .map(|id| {
                    if id + 1 < count {
                        provider(id, &[id + 1])
                    } else {
                        provider(id, &[])
                    }
                })
                .collect();
            providers[count - 1].lifetime = Lifetime::Scoped;
            let plan = compile(&types(count), &providers, &[]).unwrap();
            assert_eq!(plan.order.len(), count);
            assert_eq!(plan.order[0], count - 1);
            assert!(plan.requires_scope.iter().all(|value| *value));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_cycle_diagnostics_do_not_recurse_or_report_only_a_residual_node() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let count = 12_000;
            let providers: Vec<_> = (0..count)
                .map(|id| provider(id, &[(id + 1) % count]))
                .collect();
            let message = fails(&types(count), &providers, &[], Kind::Cycle);
            assert!(message.contains("Service11999"));
            assert!(message.contains("Service0"));
            assert!(message.contains("field0"));
        })
        .unwrap()
        .join()
        .unwrap();
}

//! 工具链生成的目标端反射标记及其身份检查。
//!
//! 服务声明的标记由真实 bridge 宏放在各自匿名 const 的 `__nestrs_reflect` 模块内。
//! 查询摘要不能依赖服务宏：只通过上游重导出使用 core 的普通库同样会贡献查询根，
//! 因此 driver 为每个处理过的 crate 注入独立的查询标记模块。该模块只存在于编译
//! 输入，不改变用户文件，不需要另一个公开 crate，也不向 core 添加空标记定义。

extern crate rustc_abi;

use crate::protocol::{self, Inputs, Marker, Parameter};
use rustc_abi::ExternAbi;
use rustc_ast::{self as ast, token};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_interface::interface;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::FileName;

pub(crate) const SOURCE: &str = "nestrs reflection metadata";
const MODULE: &str = protocol::REFLECTION_MODULE;

/// 在标准展开前只追加普通 Rust 定义。下游通过编码 MIR 读取其真实 DefId 与类型实参。
/// 用户源码与宏展开都不引用这里的查询标记，故 stock rust-analyzer 无需模拟此注入。
pub(crate) fn prepare(compiler: &interface::Compiler, krate: &mut ast::Crate) {
    if compiler.sess.opts.test
        || compiler
            .sess
            .opts
            .crate_types
            .contains(&rustc_session::config::CrateType::Executable)
    {
        // 两阶段读取必须一致；manifest 在发现阶段就加入 SourceMap 快照及 dep-info。
        crate::registration_codegen::startup_options(&compiler.sess);
    }
    let root = Marker::QueryRoot.name();
    let call = Marker::QueryCall.name();
    let unsize = Marker::QueryUnsize.name();
    let source = format!(
        r#"
        #[allow(dead_code)]
        const _: () = {{
            mod {MODULE} {{
                #[inline(never)] pub const fn {root}<T: ?Sized>() {{}}
                #[inline(never)] pub const fn {call}<F: ?Sized>() {{}}
                #[inline(never)] pub const fn {unsize}<S: ?Sized, T: ?Sized>() {{}}
                #[inline(never)] pub const fn {}() {{}}
            }}
        }};
        "#,
        crate::query_roots::SUMMARY_NAME,
    );
    let mut parser = rustc_parse::new_parser_from_source_str(
        &compiler.sess.psess,
        FileName::Custom(SOURCE.into()),
        source,
        rustc_parse::lexer::StripTokens::Nothing,
    )
    .unwrap_or_else(|diagnostics| {
        for diagnostic in diagnostics {
            diagnostic.emit();
        }
        compiler.sess.dcx().fatal("无法生成 Nestrs 反射查询标记")
    });
    while parser.token != token::Eof {
        match parser.parse_item(
            rustc_parse::parser::ForceCollect::No,
            rustc_parse::parser::AllowConstBlockItems::No,
        ) {
            Ok(Some(item)) => krate.items.push(item),
            Ok(None) => break,
            Err(error) => {
                error.emit();
                break;
            }
        }
    }
}

/// 只找本次 AST 注入的查询函数；宏内同名标记不能替代 driver 自己的摘要目标。
pub(crate) fn local_marker(tcx: TyCtxt<'_>, name: &str) -> Option<DefId> {
    tcx.hir_body_owners()
        .map(LocalDefId::to_def_id)
        .find(|&definition| {
            virtual_definition(tcx, definition) && reflect_item(tcx, definition, name)
        })
}

pub(crate) fn summary_definition(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    reflect_item(tcx, definition, crate::query_roots::SUMMARY_NAME)
}

fn virtual_definition(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    let source = tcx
        .sess
        .source_map()
        .lookup_source_file(tcx.def_span(definition).lo());
    matches!(&source.name, FileName::Custom(name) if name == SOURCE)
}

/// 标记的身份由作用域、来源和 ABI 共同确认。源码中的同名函数或 enum 不具备权限。
/// 对跨 crate 项，trusted_definition 使用 metadata 中真实桥接卫生或认证生成卫生。
pub(crate) fn reflect_item(tcx: TyCtxt<'_>, definition: DefId, name: &str) -> bool {
    if tcx
        .opt_item_name(definition)
        .is_none_or(|item| item.as_str() != name)
    {
        return false;
    }
    let parent = tcx.parent(definition);
    if tcx.def_kind(parent) != DefKind::Mod
        || tcx.item_name(parent).as_str() != MODULE
        || !crate::internal_access::trusted_definition(tcx, definition)
    {
        return false;
    }
    match name {
        protocol::COMPILER_KEY => compiler_key(tcx, definition),
        protocol::PROVIDER_DEFINITION => tcx.def_kind(definition) == DefKind::Trait,
        protocol::PROVIDER_HELPER => provider_helper(tcx, definition),
        _ => Marker::from_name(name).is_some_and(|kind| {
            let (parameters, inputs) = kind.signature();
            marker(tcx, definition, parameters, inputs)
        }),
    }
}

fn marker(tcx: TyCtxt<'_>, definition: DefId, parameters: &[Parameter], inputs: Inputs) -> bool {
    if tcx.def_kind(definition) != DefKind::Fn || tcx.is_foreign_item(definition) {
        return false;
    }
    let generics = tcx.generics_of(definition);
    if generics.parent.is_some() || generics.own_params.len() != parameters.len() {
        return false;
    }
    for (parameter, expected) in generics.own_params.iter().zip(parameters) {
        let matches = match (&parameter.kind, expected) {
            (ty::GenericParamDefKind::Type { .. }, Parameter::Type) => true,
            (ty::GenericParamDefKind::Const { .. }, expected) => {
                let kind = tcx
                    .type_of(parameter.def_id)
                    .instantiate_identity()
                    .skip_normalization();
                match expected {
                    Parameter::Usize => kind == tcx.types.usize,
                    Parameter::U8 => kind == tcx.types.u8,
                    Parameter::Bool => kind == tcx.types.bool,
                    Parameter::Type => false,
                }
            }
            _ => false,
        };
        if !matches {
            return false;
        }
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    if signature.abi() != ExternAbi::Rust
        || !signature.safety().is_safe()
        || signature.c_variadic()
        || signature.output() != tcx.types.unit
    {
        return false;
    }
    match (inputs, signature.inputs()) {
        (Inputs::None, []) => true,
        (Inputs::Label, [label]) => static_str(*label),
        (Inputs::Key, [key]) => key_type(tcx, *key, tcx.parent(definition)),
        (Inputs::KeyLabel, [key, label]) => {
            key_type(tcx, *key, tcx.parent(definition)) && static_str(*label)
        }
        _ => false,
    }
}

fn key_type(tcx: TyCtxt<'_>, key: Ty<'_>, module: DefId) -> bool {
    matches!(key.kind(), ty::Adt(definition, _) if
        tcx.parent(definition.did()) == module && reflect_item(tcx, definition.did(), protocol::COMPILER_KEY))
}

fn static_str(value: Ty<'_>) -> bool {
    matches!(value.kind(), ty::Ref(region, element, mutability)
        if region.is_static() && element.is_str() && !mutability.is_mut())
}

fn compiler_key(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    if tcx.def_kind(definition) != DefKind::Enum || tcx.generics_of(definition).count() != 0 {
        return false;
    }
    let key = tcx.adt_def(definition);
    if key.variants().len() != 3 {
        return false;
    }
    key.variants()
        .iter()
        .all(|variant| match variant.name.as_str() {
            "Default" => variant.fields.is_empty(),
            "Named" => {
                variant.fields.len() == 1
                    && static_str(
                        tcx.type_of(variant.fields.iter().next().expect("one field").did)
                            .instantiate_identity()
                            .skip_normalization(),
                    )
            }
            "Indexed" => {
                variant.fields.len() == 1
                    && tcx
                        .type_of(variant.fields.iter().next().expect("one field").did)
                        .instantiate_identity()
                        .skip_normalization()
                        == tcx.types.usize
            }
            _ => false,
        })
}

/// 泛型 adapter helper 保留普通 trait 约束，让最终入口通过 FnDef 调用，而不是直接
/// 拼接 impl 方法的 MIR 身份。约束必须指向同一反射作用域自己的 ProviderDefinition。
fn provider_helper(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    if tcx.def_kind(definition) != DefKind::Fn || tcx.is_foreign_item(definition) {
        return false;
    }
    let generics = tcx.generics_of(definition);
    if generics.parent.is_some()
        || generics.own_params.len() != 1
        || !matches!(
            generics.own_params[0].kind,
            ty::GenericParamDefKind::Type { .. }
        )
    {
        return false;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    if signature.abi() != ExternAbi::Rust
        || !signature.safety().is_safe()
        || signature.c_variadic()
        || !signature.inputs().is_empty()
        || !matches!(signature.output().kind(), ty::Adt(adapter, _)
            if tcx.crate_name(adapter.did().krate).as_str() == "nestrs_core"
                && crate::registration_codegen::definition_path(tcx, adapter.did())
                    == protocol::ACTIVATION_ADAPTER)
    {
        return false;
    }
    tcx.predicates_of(definition)
        .predicates
        .iter()
        .any(|(clause, _)| {
            matches!(clause.kind().skip_binder(), ty::ClauseKind::Trait(predicate)
            if tcx.parent(predicate.trait_ref.def_id) == tcx.parent(definition)
                && reflect_item(tcx, predicate.trait_ref.def_id, protocol::PROVIDER_DEFINITION))
        })
}

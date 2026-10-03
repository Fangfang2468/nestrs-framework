//! `#[factory]` 函数签名的结构化分析。
//!
//! factory 的每个普通参数都是一个依赖请求：未标注参数与 `#[inject]` 都请求无 key
//! 的服务，只有 `#[inject(...)]` 改变请求 key。这里一次性完成属性消费、可选性、
//! 输入槽位、函数签名重写与成功输出类型判定；后续 codegen 只消费这些稳定事实，
//! 不重新解析已经从最终函数项移除的 marker。

use crate::codegen::injection::{
    macros_attrs::service_key::ServiceKeySpec,
    sub_macros::{
        inject::{
            self, DependencyRequest, FACTORY_MESSAGES, inject_key, split_optional,
            unparenthesized_type,
        },
        lazy, value,
    },
};

use zyn::syn::{
    self, Attribute, FnArg, GenericArgument, ItemFn, Pat, PathArguments, ReturnType, Type,
    spanned::Spanned,
};

/// factory adapter 应如何调用原始函数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryInvocation {
    /// 原始函数直接产生服务或 `Result<服务, 错误>`。
    Sync,

    /// 原始函数是 `async fn`，或直接返回显式 `Future<Output = ...>`。
    Async,
}

/// factory 调用结果是否需要把用户错误归一化为 core 内部的 `ConstructionError`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactoryResultKind {
    /// 直接成功输出。
    Direct,

    /// `Result<成功输出, E>`；`E` 不跨过 factory adapter ABI。
    Result,
}

/// factory 成功输出的静态形状。
#[derive(Clone, Debug)]
pub(crate) struct FactoryReturn {
    /// 要注册为 provider token 的实际成功 concrete 类型。
    pub(crate) success_type: Type,

    /// adapter 是同步还是异步调用器。
    pub(crate) invocation: FactoryInvocation,

    /// 是否需要把原始 `Err(E)` 映射到 `ConstructionError::FactoryFailed`。
    pub(crate) result_kind: FactoryResultKind,
}

/// 一项 factory 参数的注入事实。
#[derive(Clone, Debug)]
pub(crate) struct FactoryParameterSpec {
    /// 参数在原函数签名中的零基位置。
    /// 在 `FactoryInputs` 中的连续输入槽位。
    pub(crate) input_slot: usize,

    /// 参数的简单标识符，用于 adapter 调用、label 和诊断。
    pub(crate) ident: syn::Ident,

    /// 已剥离最外层 `Option` 的服务请求类型。
    pub(crate) service_type: Type,

    /// 参数请求的静态 key；`None` 即默认 key。
    pub(crate) key: Option<ServiceKeySpec>,

    /// 原参数是否为 `Option<T>`，即缺失时可以交付 `None`。
    pub(crate) optional: bool,

    /// 是否交付按值的延迟句柄；目标仍参与图校验，但不阻塞当前 factory 启动。
    pub(crate) lazy: bool,
}

impl FactoryParameterSpec {
    /// 将参数事实转换为共享依赖请求。
    ///
    /// factory 参数的声明位置、输入槽位与标签都固定来自签名本身，因此这里不需要
    /// 额外过滤：每个参数都是一项依赖请求。
    pub(crate) fn dependency_request(&self) -> DependencyRequest {
        DependencyRequest {
            input_slot: self.input_slot,
            service_type: self.service_type.clone(),
            key: self.key.clone(),
            optional: self.optional,
            lazy: self.lazy,
            label: Some(self.ident.clone()),
        }
    }
}

/// factory 宏的共享分析结果。
///
/// `item` 已经移除了参数上的 helper marker。普通参数改写为 `&'frame T` /
/// `Option<&'frame T>`，其隐藏生命周期由真实 activation frame 绑定，不能逃逸至
/// 长期服务或后台任务。延迟参数改写为按值的 `LazyInjection<T>` /
/// `Option<LazyInjection<T>>`，由句柄本身管理访问与保活，可以移动到返回服务。
/// 所有 provider metadata 与 adapter 取参继续读取 `parameters`，避免二次解析。
#[derive(Clone, Debug)]
pub(crate) struct FactoryAnalysis {
    /// helper 已消费且参数已按输入所有权规则改写的业务函数。
    pub(crate) item: ItemFn,

    /// 按签名顺序保存的全部依赖输入事实。
    pub(crate) parameters: Vec<FactoryParameterSpec>,

    /// 成功 concrete 类型、调用方式及错误转换策略。
    pub(crate) output: FactoryReturn,
}

/// 分析一个 factory 函数并准备最终用户函数签名。
pub(crate) fn analyze_factory(mut item: ItemFn) -> syn::Result<FactoryAnalysis> {
    if item.sig.receiver().is_some() {
        return Err(syn::Error::new(
            item.sig.ident.span(),
            "`#[factory]` 只能标注普通函数，不能用于带 `self` 的 impl 方法",
        ));
    }

    // 在分析返回类型前保留既有的安全契约诊断优先级。否则一个同时是 `unsafe` 和
    // unit-return 的非法函数会错误地先被报告为 unit factory，掩盖原有 API 约束。
    if let Some(unsafety) = &item.sig.unsafety {
        return Err(syn::Error::new(
            unsafety.span(),
            "`#[factory]` 不能标注 `unsafe` 函数",
        ));
    }

    if let Some(abi) = &item.sig.abi {
        return Err(syn::Error::new(
            abi.span(),
            "`#[factory]` 不能标注 `extern` 函数",
        ));
    }

    if !item.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &item.sig.generics,
            "`#[factory]` 不支持泛型函数；请返回具体服务类型",
        ));
    }

    let output = analyze_factory_return(&item)?;

    // 用户声明的泛型已经在上方拒绝，因此这个名字不会与用户 ABI 冲突。普通注入
    // 必须使用明确的 frame lifetime，不能让 `'_` 被输出类型反向推断成 `'static`。
    // 延迟句柄拥有输入槽位中的值，不借用 frame；全部参数都是 lazy 时不添加多余泛型。
    let parameter_lifetime: syn::Lifetime = syn::parse_quote!('__nestrs_factory_frame);
    let mut parameters = Vec::with_capacity(item.sig.inputs.len());

    for (position, argument) in item.sig.inputs.iter_mut().enumerate() {
        let FnArg::Typed(parameter) = argument else {
            // receiver 已在上面检查；这条分支保留防御性，以免 syn 将来加入另一种
            // FnArg 时在宏中静默生成错误 adapter。
            return Err(syn::Error::new_spanned(
                argument,
                "`#[factory]` 参数必须是普通具名参数",
            ));
        };

        let ident = simple_parameter_ident(&parameter.pat)?;
        let (key, lazy) = take_parameter_markers(&mut parameter.attrs)?;
        let original_type = (*parameter.ty).clone();
        let (service_type, optional) = split_optional(&original_type, FACTORY_MESSAGES)?;

        *parameter.ty = injected_parameter_type(&service_type, optional, lazy, &parameter_lifetime);
        parameters.push(FactoryParameterSpec {
            input_slot: position,
            ident,
            service_type,
            key,
            optional,
            lazy,
        });
    }

    if parameters.iter().any(|parameter| !parameter.lazy) {
        item.sig
            .generics
            .params
            .push(syn::parse_quote!('__nestrs_factory_frame));
    }

    Ok(FactoryAnalysis {
        item,
        parameters,
        output,
    })
}

/// 从 factory 参数的 pattern 取得唯一允许的简单标识符。
fn simple_parameter_ident(pattern: &Pat) -> syn::Result<syn::Ident> {
    let Pat::Ident(identifier) = pattern else {
        return Err(syn::Error::new_spanned(
            pattern,
            "`#[factory]` 参数只支持简单标识符模式，例如 `database: Database`",
        ));
    };

    if identifier.by_ref.is_some() || identifier.subpat.is_some() {
        return Err(syn::Error::new_spanned(
            pattern,
            "`#[factory]` 参数只支持简单标识符模式，例如 `database: Database`",
        ));
    }

    Ok(identifier.ident.clone())
}

/// 取出并消费一个 factory 参数可使用的 marker 属性。
///
/// `#[inject]` 的省略形式和没有属性的参数具有同一语义。`#[value]` 对结构体字段
/// 才有初始化意义，函数参数没有默认构造阶段，必须在这里明确拒绝。
fn take_parameter_markers(
    attributes: &mut Vec<Attribute>,
) -> syn::Result<(Option<ServiceKeySpec>, bool)> {
    for attribute in attributes.iter() {
        if inject::is_marker(attribute) || lazy::is_marker(attribute) {
            continue;
        }
        if value::is_marker(attribute) {
            return Err(syn::Error::new_spanned(
                attribute,
                "`#[factory]` 参数不支持 #[value(...)]；factory 参数只能通过注入取得",
            ));
        }
        return Err(syn::Error::new_spanned(
            attribute,
            "`#[factory]` 参数只支持 #[inject] 和 #[lazy] 属性",
        ));
    }

    let key = inject_key(attributes)?;
    let lazy = lazy::parse_parameter(attributes)?;

    // 所有允许的 marker 都已成为 `FactoryParameterSpec` 的事实；最终函数绝不能
    // 留下一个会被 rustc 当作未知属性的 `#[inject]` / `#[lazy]`。
    attributes.clear();
    Ok((key, lazy))
}

/// 普通参数借用真实 frame，lazy 参数拥有句柄；optional 包装保持在最外层。
fn injected_parameter_type(
    service_type: &Type,
    optional: bool,
    lazy: bool,
    lifetime: &syn::Lifetime,
) -> Type {
    let injected: Type = if lazy {
        syn::parse_quote!(::nestrs_core::LazyInjection<#service_type>)
    } else {
        syn::parse_quote!(& #lifetime #service_type)
    };
    if optional {
        syn::parse_quote!(
            ::core::option::Option<#injected>
        )
    } else {
        injected
    }
}

/// 判定 factory 实际成功输出、同步/异步调用方式及 Result 错误归一化需求。
fn analyze_factory_return(item: &ItemFn) -> syn::Result<FactoryReturn> {
    let return_type = match &item.sig.output {
        // unit 形态由入口保留的 RequireNonUnit* zyn elements 统一诊断。分析层仍
        // 需要一个可注册的语法类型，以便在那些 element 截断渲染之前完整描述输出。
        ReturnType::Default => {
            let invocation = if item.sig.asyncness.is_some() {
                FactoryInvocation::Async
            } else {
                FactoryInvocation::Sync
            };
            return analyze_success_type(&syn::parse_quote!(()), invocation);
        }
        ReturnType::Type(_, ty) => ty.as_ref(),
    };

    if item.sig.asyncness.is_some() {
        if explicit_future_output(return_type).is_some() {
            return Err(syn::Error::new_spanned(
                return_type,
                "`#[factory]` 的 async fn 应直接返回服务或 Result<服务, 错误>，不能再返回 Future",
            ));
        }

        return analyze_success_type(return_type, FactoryInvocation::Async);
    }

    if let Some(future_output) = explicit_future_output(return_type) {
        return analyze_success_type(future_output, FactoryInvocation::Async);
    }

    analyze_success_type(return_type, FactoryInvocation::Sync)
}

/// 从返回类型提取可注册的成功类型，同时确定是否需要转换 Result 错误。
fn analyze_success_type(
    candidate: &Type,
    invocation: FactoryInvocation,
) -> syn::Result<FactoryReturn> {
    reject_qualified_nonstandard_result_path(candidate)?;

    if let Some(success_type) = standard_result_success_type(candidate) {
        validate_factory_success_type(success_type)?;
        return Ok(FactoryReturn {
            success_type: success_type.clone(),
            invocation,
            result_kind: FactoryResultKind::Result,
        });
    }

    validate_factory_success_type(candidate)?;
    Ok(FactoryReturn {
        success_type: candidate.clone(),
        invocation,
        result_kind: FactoryResultKind::Direct,
    })
}

/// 明确拒绝非标准库限定路径的 `*::Result`。
///
/// 宏只能可靠识别裸 `Result`、`std::result::Result` 与
/// `core::result::Result`。若将 `anyhow::Result<T>` 或项目类型别名当作直接
/// 服务输出，生成的 provider token 会错误地指向整个 Result 容器而非成功值。裸
/// 类型别名在 proc macro 阶段无法解析，仍按普通直接输出处理；限定路径的 Result
/// 则具有足够的语法证据，应在这里给出确定诊断。
fn reject_qualified_nonstandard_result_path(ty: &Type) -> syn::Result<()> {
    let Type::Path(type_path) = unparenthesized_type(ty) else {
        return Ok(());
    };
    let Some(last_segment) = type_path.path.segments.last() else {
        return Ok(());
    };

    let is_qualified_result = last_segment.ident == "Result"
        && (type_path.qself.is_some() || type_path.path.segments.len() > 1);
    if is_qualified_result && !is_standard_library_type_path(&type_path.path, "result", "Result") {
        return Err(syn::Error::new_spanned(
            ty,
            "`#[factory]` 仅支持裸 `Result`、`std::result::Result` 或 `core::result::Result`；不支持限定路径的 `Result` 或类型别名，请改用 `std::result::Result<T, E>`",
        ));
    }

    Ok(())
}

/// `impl Trait` 和 `dyn Trait` 不能成为 factory provider 的 concrete token。
/// trait 请求由返回 concrete 服务的 factory 配合普通 impl 的自动绑定满足。
fn validate_factory_success_type(ty: &Type) -> syn::Result<()> {
    match unparenthesized_type(ty) {
        Type::ImplTrait(_) => Err(syn::Error::new_spanned(
            ty,
            "`#[factory]` 的成功输出不能是 impl Trait；请返回具体服务类型",
        )),
        Type::TraitObject(_) => Err(syn::Error::new_spanned(
            ty,
            "`#[factory]` 的成功输出不能是 dyn Trait；请返回具体服务类型并使用 #[bind] 导出 trait",
        )),
        _ => Ok(()),
    }
}

/// 解析标准库 `Result<T, E>` 的成功类型。
fn standard_result_success_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = unparenthesized_type(ty) else {
        return None;
    };
    if type_path.qself.is_some()
        || !is_standard_library_type_path(&type_path.path, "result", "Result")
    {
        return None;
    }

    let segment = type_path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let GenericArgument::Type(success_type) = arguments.args.first()? else {
        return None;
    };

    Some(success_type)
}

/// 识别 `impl Future<Output = T>` 与 `dyn Future<Output = T>` 的实际 output。
fn explicit_future_output(ty: &Type) -> Option<&Type> {
    match unparenthesized_type(ty) {
        Type::ImplTrait(impl_trait) => impl_trait.bounds.iter().find_map(future_output_in_bound),
        Type::TraitObject(trait_object) => {
            trait_object.bounds.iter().find_map(future_output_in_bound)
        }
        _ => None,
    }
}

/// 只在可识别的标准 Future 约束中读取 Output 关联类型。
fn future_output_in_bound(bound: &syn::TypeParamBound) -> Option<&Type> {
    let syn::TypeParamBound::Trait(trait_bound) = bound else {
        return None;
    };
    if !is_standard_library_type_path(&trait_bound.path, "future", "Future") {
        return None;
    }

    let segment = trait_bound.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| {
        let GenericArgument::AssocType(association) = argument else {
            return None;
        };
        (association.ident == "Output").then_some(&association.ty)
    })
}

/// 接受裸名称与完整 std/core 路径，不将任意同名限定路径当作标准类型。
fn is_standard_library_type_path(path: &syn::Path, module: &str, terminal: &str) -> bool {
    let mut segments = path.segments.iter();
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return first.ident == terminal;
    };
    let Some(third) = segments.next() else {
        return false;
    };

    segments.next().is_none()
        && matches!(first.ident.to_string().as_str(), "core" | "std")
        && second.ident == module
        && third.ident == terminal
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::{quote::ToTokens, syn};

    fn analyze(source: &str) -> syn::Result<FactoryAnalysis> {
        analyze_factory(syn::parse_str(source).expect("factory source should parse"))
    }

    #[test]
    fn rewrites_default_keyed_and_optional_parameters_once() {
        let analysis = analyze(
            r#"
            fn make(
                database: Database,
                #[inject]
                cache: Cache,
                #[inject("audit")]
                audit: Option<dyn Audit>,
            ) -> Service { todo!() }
            "#,
        )
        .expect("factory should analyze");

        assert_eq!(analysis.parameters.len(), 3);
        assert_eq!(analysis.parameters[0].input_slot, 0);
        assert_eq!(analysis.parameters[1].key, None);
        assert_eq!(
            analysis.parameters[2].key,
            Some(ServiceKeySpec::named("audit"))
        );
        assert!(analysis.parameters[2].optional);
        let rewritten = analysis.item.to_token_stream().to_string();
        assert!(rewritten.contains("fn make < '__nestrs_factory_frame >"));
        assert!(rewritten.contains("database : & '__nestrs_factory_frame Database"));
        assert!(rewritten.contains("cache : & '__nestrs_factory_frame Cache"));
        assert!(rewritten.contains(
            "audit : :: core :: option :: Option < & '__nestrs_factory_frame dyn Audit >"
        ));
        assert!(!rewritten.contains("# [ inject"));
    }

    #[test]
    fn owned_lazy_parameters_do_not_introduce_a_factory_frame_lifetime() {
        let analysis = analyze(
            r#"
            fn make(
                #[lazy] reports: Report<User>,
                #[inject(7)] #[nestrs::lazy] audit: Option<dyn Audit>,
            ) -> Service { todo!() }
            "#,
        )
        .expect("lazy parameters should analyze");

        assert!(analysis.item.sig.generics.params.is_empty());
        assert!(analysis.parameters.iter().all(|parameter| parameter.lazy));
        assert!(
            analysis
                .parameters
                .iter()
                .all(|parameter| parameter.dependency_request().lazy)
        );
        assert_eq!(analysis.parameters[1].key, Some(ServiceKeySpec::indexed(7)));
        assert!(analysis.parameters[1].optional);
        let rewritten = analysis.item.to_token_stream().to_string();
        assert!(!rewritten.contains("__nestrs_factory_frame"));
        assert!(
            rewritten.contains("reports : :: nestrs_core :: LazyInjection < Report < User > >")
        );
        assert!(rewritten.contains(
            "audit : :: core :: option :: Option < :: nestrs_core :: LazyInjection < dyn Audit > >"
        ));
        assert!(!rewritten.contains("# ["));
    }

    #[test]
    fn detects_direct_result_async_and_explicit_future_success_types() {
        let direct = analyze("fn make() -> Service { todo!() }").expect("direct output");
        assert_eq!(direct.output.invocation, FactoryInvocation::Sync);
        assert_eq!(direct.output.result_kind, FactoryResultKind::Direct);

        let result =
            analyze("fn make() -> Result<Service, Error> { todo!() }").expect("result output");
        assert_eq!(result.output.invocation, FactoryInvocation::Sync);
        assert_eq!(result.output.result_kind, FactoryResultKind::Result);

        let asynchronous = analyze("async fn make() -> Result<Service, Error> { todo!() }")
            .expect("async result output");
        assert_eq!(asynchronous.output.invocation, FactoryInvocation::Async);
        assert_eq!(asynchronous.output.result_kind, FactoryResultKind::Result);

        let future = analyze(
            "fn make() -> impl ::core::future::Future<Output = Result<Service, Error>> { todo!() }",
        )
        .expect("future output");
        assert_eq!(future.output.invocation, FactoryInvocation::Async);
        assert_eq!(future.output.result_kind, FactoryResultKind::Result);
    }

    #[test]
    fn accepts_standard_result_paths_and_rejects_qualified_nonstandard_result_paths() {
        for source in [
            "fn make() -> Result<Service, Error> { todo!() }",
            "fn make() -> ::std::result::Result<Service, Error> { todo!() }",
            "fn make() -> ::core::result::Result<Service, Error> { todo!() }",
        ] {
            let analysis = analyze(source).expect("standard Result path should be supported");
            assert_eq!(analysis.output.result_kind, FactoryResultKind::Result);
        }

        let error = analyze("fn make() -> anyhow::Result<Service> { todo!() }")
            .expect_err("qualified nonstandard Result must be rejected");
        assert!(error.to_string().contains("不支持限定路径的 `Result`"));
        assert!(error.to_string().contains("std::result::Result<T, E>"));
    }

    #[test]
    fn rejects_generic_functions_complex_patterns_and_invalid_parameter_attributes() {
        let generic =
            analyze("fn make<T>() -> Service { todo!() }").expect_err("generic factory must fail");
        assert!(generic.to_string().contains("不支持泛型函数"));

        let pattern = analyze("fn make((left, right): (Left, Right)) -> Service { todo!() }")
            .expect_err("complex parameter pattern must fail");
        assert!(pattern.to_string().contains("简单标识符模式"));

        let value = analyze("fn make(#[value(1)] value: Value) -> Service { todo!() }")
            .expect_err("value attribute must fail");
        assert!(value.to_string().contains("不支持 #[value"));

        let other = analyze("fn make(#[allow(unused)] value: Value) -> Service { todo!() }")
            .expect_err("arbitrary parameter attribute must fail");
        assert!(other.to_string().contains("只支持 #[inject]"));
    }

    #[test]
    fn leaves_unit_shape_diagnostics_to_the_shared_zyn_elements() {
        let direct = analyze("fn make() {}").expect("analysis only classifies the output");
        assert_eq!(
            direct.output.success_type.to_token_stream().to_string(),
            "()"
        );
        assert_eq!(direct.output.result_kind, FactoryResultKind::Direct);

        let async_direct = analyze("async fn make() {}").expect("analysis only classifies output");
        assert_eq!(async_direct.output.invocation, FactoryInvocation::Async);

        let result = analyze("fn make() -> Result<(), Error> { todo!() }")
            .expect("analysis only classifies the Result success output");
        assert_eq!(
            result.output.success_type.to_token_stream().to_string(),
            "()"
        );
        assert_eq!(result.output.result_kind, FactoryResultKind::Result);

        let future = analyze("fn make() -> impl ::core::future::Future<Output = ()> { todo!() }")
            .expect("analysis only classifies the Future output");
        assert_eq!(
            future.output.success_type.to_token_stream().to_string(),
            "()"
        );
        assert_eq!(future.output.invocation, FactoryInvocation::Async);
    }

    #[test]
    fn preserves_unsafe_and_extern_diagnostics_before_return_analysis() {
        let unsafe_factory = analyze("unsafe fn make() {}")
            .expect_err("unsafe factory must fail before unit return analysis");
        assert!(
            unsafe_factory
                .to_string()
                .contains("不能标注 `unsafe` 函数")
        );

        let extern_factory = analyze("extern \"C\" fn make() {}")
            .expect_err("extern factory must fail before unit return analysis");
        assert!(
            extern_factory
                .to_string()
                .contains("不能标注 `extern` 函数")
        );
    }
}

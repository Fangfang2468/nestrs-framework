//! `#[factory]` 函数签名的结构化分析。
//!
//! factory 的每个普通参数都是一个依赖请求：未标注参数与 `#[inject]` 都请求无 key
//! 的服务，只有 `#[inject(...)]` 改变请求 key。这里一次性完成属性消费、可选性、
//! 输入槽位、函数签名重写与成功输出类型判定；后续 codegen 只消费这些稳定事实，
//! 不重新解析已经从最终函数项移除的 marker。

use crate::injection::{
    attrs::service_key::ServiceKey,
    sub_macros::{
        inject::{
            self, DependencyRequest, FACTORY_MESSAGES, inject_key, split_optional,
            unparenthesized_type,
        },
        value,
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

/// factory 调用结果是否需要把用户错误归一化为 [`ActivationError`][1]。
///
/// [1]: ::nestrs_core::__private::ActivationError
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
    /// 是否需要把原始 `Err(E)` 映射到 `ActivationError::FactoryFailed`。
    pub(crate) result_kind: FactoryResultKind,
}

/// 一项 factory 参数的注入事实。
#[derive(Clone, Debug)]
pub(crate) struct FactoryParameterSpec {
    /// 参数在原函数签名中的零基位置。
    pub(crate) declaration_position: usize,
    /// 在 `ConstructionContext` 中的连续输入槽位。
    pub(crate) input_position: usize,
    /// 参数的简单标识符，用于 adapter 调用、label 和诊断。
    pub(crate) ident: syn::Ident,
    /// 已剥离最外层 `Option` 的服务请求类型。
    pub(crate) service_type: Type,
    /// 参数请求的静态 key；`None` 即默认 key。
    pub(crate) key: Option<ServiceKey>,
    /// 原参数是否为 `Option<T>`，即缺失时可以交付 `None`。
    pub(crate) optional: bool,
}

impl FactoryParameterSpec {
    /// 将参数事实转换为共享依赖请求。
    ///
    /// factory 参数的声明位置、输入槽位与标签都固定来自签名本身，因此这里不需要
    /// 额外过滤：每个参数都是一项依赖请求。
    pub(crate) fn dependency_request(&self) -> DependencyRequest {
        DependencyRequest {
            declaration_position: self.declaration_position,
            input_position: self.input_position,
            service_type: self.service_type.clone(),
            key: self.key.clone(),
            optional: self.optional,
            label: Some(self.ident.clone()),
        }
    }
}

/// factory 宏的共享分析结果。
///
/// `item` 已经移除了参数上的 `#[inject]` marker，并把参数类型改写为
/// `Inject<T, FactoryParameter<'frame>>` / `Option<...>`。`'frame` 是宏生成的隐藏
/// 生命周期，并由 factory adapter 的真实 activation frame 绑定；因此最终用户函数不能
/// 把参数安全地保存到长期服务或后台任务。所有 provider metadata 与 adapter 取参继续
/// 读取 `parameters`，避免二次解析。
#[derive(Clone, Debug)]
pub(crate) struct FactoryAnalysis {
    pub(crate) item: ItemFn,
    pub(crate) parameters: Vec<FactoryParameterSpec>,
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

    // 用户声明的泛型已经在上方拒绝，因此这个名字不会与用户 ABI 冲突。只要存在参数，
    // 就为重写后的函数添加一个由 adapter 推导的隐藏生命周期；不能使用一个无约束的
    // `'_` 占位 lifetime，否则返回服务的类型约束可能把它错误地推断为 `'static`。
    let parameter_lifetime = if item.sig.inputs.is_empty() {
        None
    } else {
        let lifetime: syn::Lifetime = syn::parse_quote!('__nestrs_factory_frame);
        item.sig
            .generics
            .params
            .push(syn::parse_quote!('__nestrs_factory_frame));
        Some(lifetime)
    };
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
        let key = take_parameter_key(&mut parameter.attrs)?;
        let original_type = (*parameter.ty).clone();
        let (service_type, optional) = split_optional(&original_type, FACTORY_MESSAGES)?;

        let parameter_lifetime = parameter_lifetime
            .as_ref()
            .expect("a factory parameter requires the generated activation lifetime");
        parameter.ty = Box::new(injected_parameter_type(
            &service_type,
            optional,
            parameter_lifetime,
        ));
        parameters.push(FactoryParameterSpec {
            declaration_position: position,
            input_position: position,
            ident,
            service_type,
            key,
            optional,
        });
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
fn take_parameter_key(attributes: &mut Vec<Attribute>) -> syn::Result<Option<ServiceKey>> {
    for attribute in attributes.iter() {
        if inject::is_marker(attribute) {
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
            "`#[factory]` 参数只支持 #[inject] 属性",
        ));
    }

    let key = inject_key(attributes)?;

    // 所有允许的 marker 都已成为 `FactoryParameterSpec` 的事实；最终函数绝不能
    // 留下一个会被 rustc 当作未知属性的 `#[inject]`。
    attributes.clear();
    Ok(key)
}

fn injected_parameter_type(service_type: &Type, optional: bool, lifetime: &syn::Lifetime) -> Type {
    if optional {
        syn::parse_quote!(
            ::core::option::Option<
                ::nestrs_core::__private::Inject<
                    #service_type,
                    ::nestrs_core::__private::FactoryParameter<#lifetime>
                >
            >
        )
    } else {
        syn::parse_quote!(
            ::nestrs_core::__private::Inject<
                #service_type,
                ::nestrs_core::__private::FactoryParameter<#lifetime>
            >
        )
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

fn analyze_success_type(
    candidate: &Type,
    invocation: FactoryInvocation,
) -> syn::Result<FactoryReturn> {
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

/// `impl Trait` 和 `dyn Trait` 不能成为 factory provider 的 concrete token。
/// trait 导出必须由返回 concrete 服务的 factory 再配合 `#[bind]` 表达。
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
                #[inject(key = "audit")]
                audit: Option<dyn Audit>,
            ) -> Service { todo!() }
            "#,
        )
        .expect("factory should analyze");

        assert_eq!(analysis.parameters.len(), 3);
        assert_eq!(analysis.parameters[0].input_position, 0);
        assert_eq!(analysis.parameters[1].key, None);
        assert_eq!(
            analysis.parameters[2].key,
            Some(ServiceKey::Named("audit".to_owned()))
        );
        assert!(analysis.parameters[2].optional);
        let rewritten = analysis.item.to_token_stream().to_string();
        assert!(rewritten.contains("fn make < '__nestrs_factory_frame >"));
        assert!(
            rewritten.contains(
                "database : :: nestrs_core :: __private :: Inject < Database , :: nestrs_core :: __private :: FactoryParameter < '__nestrs_factory_frame > >"
            )
        );
        assert!(rewritten.contains(
            "cache : :: nestrs_core :: __private :: Inject < Cache , :: nestrs_core :: __private :: FactoryParameter < '__nestrs_factory_frame > >"
        ));
        assert!(rewritten.contains(
            "audit : :: core :: option :: Option < :: nestrs_core :: __private :: Inject < dyn Audit , :: nestrs_core :: __private :: FactoryParameter < '__nestrs_factory_frame > > >"
        ));
        assert!(!rewritten.contains("# [ inject"));
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

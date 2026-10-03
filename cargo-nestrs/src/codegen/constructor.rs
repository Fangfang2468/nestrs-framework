//! 显式关联构造函数的签名分析。
//!
//! 宏只消费参数声明：普通输入生成拥有 lease 的 Injection，lazy 输入生成延迟句柄。
//! 函数体的 cfg、嵌套宏和局部变量
//! 卫生尚未由 rustc 解析，不能在此按字符串推断字段来源；该工作统一交给 compiler
//! 的 constructor hook，编辑器再重放同一份已解析的字段模型。

use zyn::syn::{
    self, Attribute, FnArg, GenericArgument, ImplItemFn, Pat, Path, PathArguments, ReturnType, Type,
};

use super::injection::{
    macros_attrs::service_key::ServiceKeySpec,
    sub_macros::{
        inject::{self, DependencyRequest, GrammarMessages, split_optional},
        lazy, value,
    },
};

/// 普通编译保留候选给 driver；编辑器直接渲染已验证的构造选择。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstructorMode {
    /// 保留自动与显式候选，等待 driver 在真实名称解析后选择。
    Deferred,

    /// IDE 模型已确认自动字段构造，只输出该候选。
    Automatic,

    /// IDE 模型已确认显式关联构造，只引用对应的输入和激活 helper。
    Explicit,
}

/// 一项构造参数对应一个图输入；即使参数只用于校验或计算普通字段，也不能删除此输入。
#[derive(Clone, Debug)]
pub(crate) struct ConstructorParameterSpec {
    /// 参数在完整构造输入中的连续槽位，按签名顺序分配。
    pub(crate) input_slot: usize,

    /// 保留原始卫生的业务参数名，用作依赖标签与诊断来源。
    pub(crate) ident: syn::Ident,

    /// 剥离外层 Option 后的真实请求语法类型。
    pub(crate) service_type: Type,

    /// 静态服务 key；缺省时选择默认 key。
    pub(crate) key: Option<ServiceKeySpec>,

    /// 缺少匹配服务时是否允许交付 None。
    pub(crate) optional: bool,

    /// 是否按值交付 LazyInjection，而不是已就绪的 Injection。
    pub(crate) lazy: bool,
}

impl ConstructorParameterSpec {
    /// 把参数事实交给字段和 factory 共用的输入描述生成器。
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

/// 关联构造返回值的语法形态，决定是否生成错误映射。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstructorResultKind {
    /// 关联构造直接返回 Self。
    Direct,

    /// 关联构造返回 Result<Self, E>，错误由 adapter 转为 ConstructionError。
    Result,
}

/// 显式构造的共享分析结果；业务函数体不在此阶段解释。
#[derive(Clone, Debug)]
pub(crate) struct ConstructorAnalysis {
    /// 参数 helper 已消费；输入类型已改为按值 Injection / LazyInjection。
    pub(crate) item: ImplItemFn,

    /// 按原签名顺序保存的输入事实，不因字段是否存储参数而裁剪。
    pub(crate) parameters: Vec<ConstructorParameterSpec>,

    /// 决定激活 adapter 的成功值提取与错误转换方式。
    pub(crate) result_kind: ConstructorResultKind,
}

/// 校验同步关联构造函数并改写参数；函数体保留完整的原始 token 与宏卫生。
pub(crate) fn analyze_constructor(mut item: ImplItemFn) -> syn::Result<ConstructorAnalysis> {
    validate_signature(&item)?;
    let result_kind = result_kind(&item.sig.output)?;
    let mut parameters = Vec::with_capacity(item.sig.inputs.len());
    for (position, argument) in item.sig.inputs.iter_mut().enumerate() {
        let FnArg::Typed(parameter) = argument else {
            unreachable!("receiver 已由 validate_signature 拒绝");
        };
        let ident = parameter_ident(&parameter.pat)?.clone();
        let (key, lazy) = take_markers(&mut parameter.attrs)?;
        let (service_type, optional) = split_optional(
            &parameter.ty,
            GrammarMessages {
                optional_shape: "`#[constructor]` 可选参数必须写为 Option<T>",
            },
        )?;
        let token: Type = if lazy {
            syn::parse_quote!(::nestrs_core::LazyInjection<#service_type>)
        } else {
            syn::parse_quote!(::nestrs_core::Injection<#service_type>)
        };
        *parameter.ty = if optional {
            syn::parse_quote!(::core::option::Option<#token>)
        } else {
            token
        };
        parameters.push(ConstructorParameterSpec {
            input_slot: position,
            ident,
            service_type,
            key,
            optional,
            lazy,
        });
    }

    Ok(ConstructorAnalysis {
        item,
        parameters,
        result_kind,
    })
}

/// 拒绝接收 self、异步、unsafe、extern 及方法自有泛型，保留 impl 泛型。
fn validate_signature(item: &ImplItemFn) -> syn::Result<()> {
    let signature = &item.sig;
    let unsupported = if signature.asyncness.is_some() {
        Some("`#[constructor]` 只支持同步关联函数；异步初始化请使用 #[factory]")
    } else if signature.unsafety.is_some() {
        Some("`#[constructor]` 不能标注 unsafe 函数")
    } else if signature.abi.is_some() {
        Some("`#[constructor]` 不能标注 extern 函数")
    } else if signature.receiver().is_some() {
        Some("`#[constructor]` 不能包含 self 参数")
    } else if !signature.generics.params.is_empty() {
        Some("`#[constructor]` 不支持方法自己的泛型参数；可使用所属 impl 的泛型参数")
    } else {
        None
    };
    if let Some(message) = unsupported {
        return Err(syn::Error::new_spanned(signature, message));
    }
    Ok(())
}

/// 识别 Self 或标准 Result<Self, E>，在宏期拒绝不支持的输出语法。
fn result_kind(output: &ReturnType) -> syn::Result<ConstructorResultKind> {
    if let ReturnType::Type(_, ty) = output {
        if is_self_type(ty) {
            return Ok(ConstructorResultKind::Direct);
        }
        if let Type::Path(path) = inject::unparenthesized_type(ty)
            && path.qself.is_none()
            && is_standard_path(&path.path, "result", "Result")
            && let PathArguments::AngleBracketed(arguments) =
                &path.path.segments.last().unwrap().arguments
            && arguments.args.len() == 2
            && let Some(GenericArgument::Type(success)) = arguments.args.first()
            && is_self_type(success)
            && matches!(arguments.args.last(), Some(GenericArgument::Type(_)))
        {
            return Ok(ConstructorResultKind::Result);
        }
    }
    Err(syn::Error::new_spanned(
        output,
        "`#[constructor]` 必须返回 Self 或 Result<Self, E>，不支持 Future 或其他成功类型",
    ))
}

/// 忽略括号和宏分组后判断是否为裸 Self。
fn is_self_type(ty: &Type) -> bool {
    matches!(inject::unparenthesized_type(ty), Type::Path(path) if path.qself.is_none() && path.path.is_ident("Self"))
}

/// 识别裸类型名或 std/core 的指定路径，不在宏阶段解析别名。
fn is_standard_path(path: &Path, module: &str, terminal: &str) -> bool {
    let segments: Vec<_> = path.segments.iter().collect();
    match segments.as_slice() {
        [only] => only.ident == terminal,
        [root, middle, last] => {
            (root.ident == "std" || root.ident == "core")
                && middle.ident == module
                && last.ident == terminal
        }
        _ => false,
    }
}

/// 只接受简单标识符模式，保留 mut 与原始标识符的业务身份。
fn parameter_ident(pattern: &Pat) -> syn::Result<&syn::Ident> {
    match pattern {
        Pat::Ident(identifier) if identifier.by_ref.is_none() && identifier.subpat.is_none() => {
            Ok(&identifier.ident)
        }
        _ => Err(syn::Error::new_spanned(
            pattern,
            "`#[constructor]` 参数只支持简单标识符模式，例如 database: Database",
        )),
    }
}

/// 校验并消费参数上的 inject/lazy helper；业务值必须在构造函数体内计算。
fn take_markers(attributes: &mut Vec<Attribute>) -> syn::Result<(Option<ServiceKeySpec>, bool)> {
    for attribute in attributes.iter() {
        if inject::is_marker(attribute) || lazy::is_marker(attribute) {
            continue;
        }
        return Err(syn::Error::new_spanned(
            attribute,
            if value::is_marker(attribute) {
                "`#[constructor]` 参数不支持 #[value]；请在函数体内初始化业务值"
            } else {
                "`#[constructor]` 参数只支持 #[inject] 和 #[lazy] 属性"
            },
        ));
    }
    let key = inject::inject_key(attributes)?;
    let lazy = lazy::parse_parameter(attributes)?;
    attributes.clear();
    Ok((key, lazy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze(source: &str) -> syn::Result<ConstructorAnalysis> {
        analyze_constructor(syn::parse_str(source).expect("关联函数语法合法"))
    }

    #[test]
    fn constructor_parameters_own_tokens_and_preserve_full_dependency_facts() {
        let analysis = analyze(
            r#"fn new(
            database: Database,
            #[inject("audit")] #[lazy] audit: Option<dyn Audit>,
            #[nestrs::inject(7)] cache: Cache<User>,
        ) -> Self { Self { database, audit, cache } }"#,
        )
        .unwrap();
        let types: Vec<_> = analysis
            .item
            .sig
            .inputs
            .iter()
            .map(|argument| match argument {
                FnArg::Typed(parameter) => {
                    assert!(parameter.attrs.is_empty());
                    parameter.ty.as_ref()
                }
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            *types[0],
            syn::parse_quote!(::nestrs_core::Injection<Database>)
        );
        assert_eq!(
            *types[1],
            syn::parse_quote!(::core::option::Option<::nestrs_core::LazyInjection<dyn Audit>>)
        );
        assert_eq!(
            *types[2],
            syn::parse_quote!(::nestrs_core::Injection<Cache<User>>)
        );
        assert_eq!(
            analysis.parameters[1].key,
            Some(ServiceKeySpec::named("audit"))
        );
        assert_eq!(analysis.parameters[2].key, Some(ServiceKeySpec::indexed(7)));
        let dependency = analysis.parameters[1].dependency_request();
        assert!(dependency.optional && dependency.lazy);
        assert_eq!(dependency.input_slot, 1);
    }

    #[test]
    fn rejects_unsupported_signatures_and_parameter_markers() {
        for source in [
            "async fn new() -> Self { Self {} }",
            "unsafe fn new() -> Self { Self {} }",
            "extern \"C\" fn new() -> Self { Self {} }",
            "fn new(&self) -> Self { Self {} }",
            "fn new<T>() -> Self { Self {} }",
            "fn new() -> impl Future<Output = Self> { todo!() }",
            "fn new() -> Service { Self {} }",
            "fn new(#[lazy(false)] db: Database) -> Self { Self { db } }",
            "fn new(#[lazy] #[lazy] db: Database) -> Self { Self { db } }",
            "fn new(#[value(1)] db: Database) -> Self { Self { db } }",
            "fn new((db, other): (Database, Other)) -> Self { Self { db } }",
        ] {
            assert!(analyze(source).is_err(), "应拒绝不支持的声明：{source}");
        }
    }

    #[test]
    fn leaves_body_provenance_to_the_resolved_compiler_ast() {
        // 宏输入仍含 cfg 和宏卫生信息，此时不能按名字猜字段或主动解释函数体。
        // 字段来源的成功、失败回归在 constructor_contracts 的真实 compiler fixture 中。
        let source = "fn new(db: Database) -> Self { Self { #[cfg(any())] db } }";
        let analysis = analyze(source).unwrap();
        assert_eq!(analysis.parameters.len(), 1);
        assert_eq!(
            analysis.item.block,
            syn::parse_str::<ImplItemFn>(source).unwrap().block
        );
    }
}

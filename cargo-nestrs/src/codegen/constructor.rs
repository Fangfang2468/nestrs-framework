//! 显式关联构造函数的签名分析。
//!
//! 宏只消费参数声明并生成拥有 lease 的输入类型。函数体的 cfg、嵌套宏和局部变量
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
    Deferred,
    Automatic,
    Explicit,
}

/// 一项构造参数对应一个图输入；即使参数只用于校验或计算普通字段，也不能删除此输入。
#[derive(Clone, Debug)]
pub(crate) struct ConstructorParameterSpec {
    pub(crate) input_slot: usize,
    pub(crate) ident: syn::Ident,
    pub(crate) service_type: Type,
    pub(crate) key: Option<ServiceKeySpec>,
    pub(crate) optional: bool,
    pub(crate) lazy: bool,
}

impl ConstructorParameterSpec {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstructorResultKind {
    Direct,
    Result,
}

#[derive(Clone, Debug)]
pub(crate) struct ConstructorAnalysis {
    /// 参数 helper 已消费；输入类型已改为按值 Injection / LazyInjection。
    pub(crate) item: ImplItemFn,
    pub(crate) parameters: Vec<ConstructorParameterSpec>,
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

fn is_self_type(ty: &Type) -> bool {
    matches!(inject::unparenthesized_type(ty), Type::Path(path) if path.qself.is_none() && path.path.is_ident("Self"))
}

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

//! 可注入服务请求类型的唯一分类与校验。
//!
//! 分类结果决定依赖请求的交付方式与 provider 来源，因此这里也是「trait 注入」与
//! 「闭合泛型服务」两条路径唯一的静态分流点。

use zyn::syn::{self, GenericArgument, PathArguments, Type};

/// 共享语法在两个宏入口中的措辞差异。
///
/// 语法规则本身完全一致，只有面向用户的文案需要区分「字段」与「参数」。
#[derive(Clone, Copy, Debug)]
pub(crate) struct GrammarMessages {
    /// 可选依赖没有写成 `Option<T>` 时的错误文案。
    pub(crate) optional_shape: &'static str,
}

/// `#[injectable]` 字段使用的措辞。
pub(crate) const INJECTABLE_MESSAGES: GrammarMessages = GrammarMessages {
    optional_shape: "#[inject] 可选字段必须写为 Option<T>",
};

/// `#[factory]` 参数使用的措辞。
pub(crate) const FACTORY_MESSAGES: GrammarMessages = GrammarMessages {
    optional_shape: "`#[factory]` 可选参数必须写为 Option<T>",
};

/// 一个已通过校验的服务请求的静态形状。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyShape {
    /// 普通 concrete 服务类型。
    Concrete,

    /// 带实参的 concrete 类型路径。
    ///
    /// `TypeId` 无法从它反推泛型实参，因此缺少显式注册时需要由
    /// `ProviderDefinition` 在消费点按需物化 provider。
    ClosedGeneric,

    /// `dyn Trait`：交付依赖 `#[bind]` 在解析期提供的 typed projector。
    TraitObject,
}

/// 分类一个服务请求类型。
///
/// 调用方必须先用 [`validate_service_type`] 校验同一类型；两者共同保证返回值只会是
/// `Concrete`、`ClosedGeneric` 或 `TraitObject`。
pub(crate) fn classify(service_type: &Type) -> DependencyShape {
    match unparenthesized_type(service_type) {
        Type::TraitObject(_) => DependencyShape::TraitObject,
        Type::Path(type_path) if has_angle_bracketed_arguments(type_path) => {
            DependencyShape::ClosedGeneric
        }
        _ => DependencyShape::Concrete,
    }
}

/// 该请求是否需要在缺少显式注册时按需物化 provider。
pub(crate) fn requires_materialization(service_type: &Type) -> bool {
    classify(service_type) == DependencyShape::ClosedGeneric
}

fn has_angle_bracketed_arguments(type_path: &syn::TypePath) -> bool {
    type_path.qself.is_none()
        && type_path.path.segments.iter().any(|segment| {
            matches!(
                &segment.arguments,
                PathArguments::AngleBracketed(arguments) if !arguments.args.is_empty()
            )
        })
}

/// 剥离唯一允许的最外层 `Option<T>`，返回实际服务请求类型与可选性。
///
/// 括号与 macro 分组会在返回前被剥离，因此 `#[inject] field: (Service)` 与
/// `#[inject] field: Service` 在所有宏入口下都具有相同语义。
pub(crate) fn split_optional(
    service_type: &Type,
    messages: GrammarMessages,
) -> syn::Result<(Type, bool)> {
    if let Some(inner_type) = option_inner(service_type, messages)? {
        validate_service_type(&inner_type, true)?;
        return Ok((unparenthesized_type(&inner_type).clone(), true));
    }

    validate_service_type(service_type, true)?;
    Ok((unparenthesized_type(service_type).clone(), false))
}

fn option_inner(ty: &Type, messages: GrammarMessages) -> syn::Result<Option<Type>> {
    let Type::Path(type_path) = unparenthesized_type(ty) else {
        return Ok(None);
    };

    if type_path.qself.is_some()
        || !type_path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Option")
    {
        return Ok(None);
    }

    let segment = type_path
        .path
        .segments
        .last()
        .expect("an Option path always has a final segment");
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(syn::Error::new_spanned(ty, messages.optional_shape));
    };

    if arguments.args.len() != 1 {
        return Err(syn::Error::new_spanned(arguments, messages.optional_shape));
    }

    let Some(GenericArgument::Type(inner_type)) = arguments.args.first() else {
        return Err(syn::Error::new_spanned(arguments, messages.optional_shape));
    };

    Ok(Some(inner_type.clone()))
}

/// 校验一个注入目标是否是可以被容器表达的精确服务类型。
pub(crate) fn validate_service_type(ty: &Type, is_top_level: bool) -> syn::Result<()> {
    match unparenthesized_type(ty) {
        Type::TraitObject(_) => Ok(()),
        Type::Path(type_path) => {
            if type_path.qself.is_some() {
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[inject] 不接受带限定的关联类型；请注入精确服务类型",
                ));
            }

            if type_path.path.is_ident("Self") {
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[inject] 不接受 Self；请注入精确服务类型",
                ));
            }

            if is_top_level && is_disallowed_top_level_wrapper(&type_path.path) {
                return Err(syn::Error::new_spanned(
                    ty,
                    "#[inject] 不接受最外层 Arc<T> 或嵌套 Option<T>",
                ));
            }

            for segment in &type_path.path.segments {
                match &segment.arguments {
                    PathArguments::None => {}
                    PathArguments::AngleBracketed(arguments) => {
                        validate_generic_arguments(arguments.args.iter())?;
                    }
                    PathArguments::Parenthesized(_) => {
                        return Err(syn::Error::new_spanned(
                            ty,
                            "#[inject] 不接受函数式类型实参；请注入精确服务类型",
                        ));
                    }
                }
            }

            Ok(())
        }
        Type::Reference(_) => Err(syn::Error::new_spanned(
            ty,
            "#[inject] 不接受引用；请注入精确服务类型",
        )),
        Type::ImplTrait(_) => Err(syn::Error::new_spanned(
            ty,
            "#[inject] 不接受 impl Trait；请注入精确服务类型",
        )),
        _ => Err(syn::Error::new_spanned(
            ty,
            "#[inject] 仅接受精确类型路径或 dyn Trait",
        )),
    }
}

fn validate_generic_arguments<'a>(
    arguments: impl Iterator<Item = &'a GenericArgument>,
) -> syn::Result<()> {
    for argument in arguments {
        match argument {
            GenericArgument::Type(ty) => validate_service_type(ty, false)?,
            GenericArgument::AssocType(association) => {
                validate_service_type(&association.ty, false)?;
            }
            GenericArgument::Lifetime(_)
            | GenericArgument::Const(_)
            | GenericArgument::AssocConst(_) => {}
            GenericArgument::Constraint(_) => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "#[inject] 不接受关联类型约束；请注入精确服务类型",
                ));
            }
            _ => {
                return Err(syn::Error::new_spanned(
                    argument,
                    "#[inject] 包含不支持的泛型实参；请注入精确服务类型",
                ));
            }
        }
    }

    Ok(())
}

fn is_disallowed_top_level_wrapper(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == "Arc" || segment.ident == "Option")
}

/// 递归剥离 `(T)` 与 macro 分组，保留内部类型。
pub(crate) fn unparenthesized_type(ty: &Type) -> &Type {
    match ty {
        Type::Paren(parenthesized) => unparenthesized_type(&parenthesized.elem),
        Type::Group(grouped) => unparenthesized_type(&grouped.elem),
        _ => ty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::syn::parse_quote;

    #[test]
    fn classifies_concrete_closed_generic_and_trait_requests() {
        assert_eq!(
            classify(&parse_quote!(Database)),
            DependencyShape::Concrete
        );
        assert_eq!(
            classify(&parse_quote!((Database))),
            DependencyShape::Concrete
        );
        assert_eq!(
            classify(&parse_quote!(Repository<User>)),
            DependencyShape::ClosedGeneric
        );
        assert_eq!(
            classify(&parse_quote!(dyn Audit)),
            DependencyShape::TraitObject
        );
        assert_eq!(
            classify(&parse_quote!(dyn Repository<User>)),
            DependencyShape::TraitObject
        );

        assert!(requires_materialization(&parse_quote!(Repository<User>)));
        assert!(!requires_materialization(&parse_quote!(Database)));
        assert!(!requires_materialization(&parse_quote!(dyn Audit)));
    }

    #[test]
    fn splits_the_only_supported_optional_form() {
        let (required, optional) =
            split_optional(&parse_quote!(Database), INJECTABLE_MESSAGES).expect("required form");
        assert_eq!(required, parse_quote!(Database));
        assert!(!optional);

        let (inner, optional) =
            split_optional(&parse_quote!(Option<Database>), INJECTABLE_MESSAGES).expect("optional");
        assert_eq!(inner, parse_quote!(Database));
        assert!(optional);

        // 括号与 `Option` 的组合在抽取后与未加括号写法等价。
        let (inner, optional) =
            split_optional(&parse_quote!((Option<Database>)), INJECTABLE_MESSAGES)
                .expect("parenthesized optional");
        assert_eq!(inner, parse_quote!(Database));
        assert!(optional);
    }

    #[test]
    fn rejects_invalid_optional_and_service_shapes() {
        let nested = split_optional(&parse_quote!(Option<Option<Database>>), INJECTABLE_MESSAGES)
            .expect_err("nested Option must fail");
        assert!(nested.to_string().contains("不接受最外层 Arc<T> 或嵌套 Option<T>"));

        let bare = split_optional(&parse_quote!(Option), FACTORY_MESSAGES)
            .expect_err("bare Option must fail");
        assert!(bare.to_string().contains("`#[factory]` 可选参数必须写为 Option<T>"));

        for rejected in [
            parse_quote!(&Database),
            parse_quote!(impl Audit),
            parse_quote!(Self),
            parse_quote!(Arc<Database>),
        ] {
            assert!(
                validate_service_type(&rejected, true).is_err(),
                "type should be rejected: {rejected:?}"
            );
        }

        assert!(validate_service_type(&parse_quote!(dyn Audit), true).is_ok());
        assert!(validate_service_type(&parse_quote!(Repository<User>), true).is_ok());
    }
}

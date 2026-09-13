//! `#[inject]` 子标注的唯一实现。
//!
//! `#[injectable]` 字段与 `#[factory]` 参数共用这个子标注：两者对 `#[inject]`、
//! `#[inject("name")]`、`#[inject(1)]`、`#[inject(key = ...)]` 的接受范围，以及
//! `Option<T>` 可选形态与可注入服务类型的规则完全一致。
//!
//! 这里只做「源码语法 → 宏期事实」：不生成 token、不改写 AST、不依赖 provider 注册
//! ABI。key 值的字面量规则定义在 [`crate::injection::attrs::service_key`]，注册 ABI
//! 的渲染在 `crate::injection::render`。

use crate::injection::attrs::service_key::{self, ServiceKey};
use zyn::syn::{
    self, Attribute, GenericArgument, Lit, Meta, PathArguments, Type, parse::Parser,
    punctuated::Punctuated,
};

// ---------------------------------------------------------------------------
// 依赖请求事实
// ---------------------------------------------------------------------------

/// 一个依赖请求的宏期事实。
///
/// 它与 `nestrs_core::registration::dependency::DependencyRequest` 一一对应：这里是
/// 语法层事实，后者是写进 provider 注册 ABI 的运行时描述。`#[inject]` 字段与 factory
/// 参数都先归一到这个形状，再共享同一套渲染逻辑。
#[derive(Clone, Debug)]
pub(crate) struct DependencyRequest {
    /// 依赖在字段或参数声明中的零基位置。
    ///
    /// 对结构体字段，该位置包含 `#[value]` 与默认字段；它只服务于稳定诊断，不等同
    /// 于构造 ABI 的输入槽位。
    pub(crate) declaration_position: usize,

    /// 依赖在构造输入中的位置。
    pub(crate) input_position: usize,

    /// 请求的服务类型；已剥离最外层 `Option` 与多余括号。
    pub(crate) service_type: Type,

    /// 静态服务限定符。
    pub(crate) key: Option<ServiceKey>,

    /// 缺失依赖时是否允许交付 `None`。
    pub(crate) optional: bool,

    /// 具名字段或参数的名称；元组字段为 `None`。
    pub(crate) label: Option<syn::Ident>,
}

// ---------------------------------------------------------------------------
// 属性语法
// ---------------------------------------------------------------------------

/// 从属性列表中取出唯一的 `#[inject(...)]` 并返回它声明的 key。
///
/// 未标注 `#[inject]` 与裸 `#[inject]` 都返回 `Ok(None)`（默认 key）；重复标注与
/// 非法参数形态返回错误。属性本身不由这里移除，调用方按自己的 AST 改写职责处理。
pub(crate) fn inject_key(attributes: &[Attribute]) -> syn::Result<Option<ServiceKey>> {
    let mut found: Option<&Attribute> = None;

    for attribute in attributes {
        if !attribute.path().is_ident("inject") {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(attribute, "重复的 #[inject] 属性"));
        }
        found = Some(attribute);
    }

    found
        .map(parse_inject_attribute)
        .transpose()
        .map(Option::flatten)
}

/// 解析单个 `#[inject]` / `#[inject(...)]` 属性。
fn parse_inject_attribute(attribute: &Attribute) -> syn::Result<Option<ServiceKey>> {
    match &attribute.meta {
        Meta::Path(_) => Ok(None),
        Meta::List(list) => {
            if list.tokens.is_empty() {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 不接受空参数；请使用 #[inject]、#[inject(\"key\")] 或 #[inject(key = \"key\")]",
                ));
            }

            if let Ok(literal) = syn::parse2::<Lit>(list.tokens.clone()) {
                return service_key::from_literal(&literal).map(Some);
            }

            let metas = Punctuated::<Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .map_err(|_| {
                    syn::Error::new_spanned(attribute, "#[inject] 只接受一个字符串或整数 key")
                })?;

            if metas.len() != 1 {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 只接受一个 key 参数",
                ));
            }

            let Some(Meta::NameValue(value)) = metas.first() else {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 只接受字符串或整数 key；命名形式请写为 key = ...",
                ));
            };

            if !value.path.is_ident("key") {
                return Err(syn::Error::new_spanned(
                    &value.path,
                    "#[inject] 只支持 key 参数",
                ));
            }

            service_key::from_expression(&value.value).map(Some)
        }
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attribute,
            "#[inject] 参数必须写在括号中",
        )),
    }
}

// ---------------------------------------------------------------------------
// 服务类型形状
// ---------------------------------------------------------------------------

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

    fn key_of(attribute: Attribute) -> syn::Result<Option<ServiceKey>> {
        inject_key(&[attribute])
    }

    #[test]
    fn parses_every_supported_key_form() {
        assert_eq!(key_of(parse_quote!(#[inject])).expect("bare"), None);
        assert_eq!(
            key_of(parse_quote!(#[inject("named")])).expect("positional string"),
            Some(ServiceKey::Named("named".to_owned()))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(7)])).expect("positional integer"),
            Some(ServiceKey::Indexed(7))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(key = "named")])).expect("named string"),
            Some(ServiceKey::Named("named".to_owned()))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(key = 3)])).expect("named integer"),
            Some(ServiceKey::Indexed(3))
        );
    }

    #[test]
    fn ignores_attributes_that_are_not_inject_markers() {
        let attributes: Vec<Attribute> = vec![parse_quote!(#[value(1)])];

        assert_eq!(
            inject_key(&attributes).expect("non-inject attributes are ignored"),
            None
        );
    }

    #[test]
    fn rejects_duplicate_empty_and_invalid_keys() {
        let duplicate = inject_key(&[parse_quote!(#[inject]), parse_quote!(#[inject])])
            .expect_err("duplicate markers must fail");
        assert!(duplicate.to_string().contains("重复的 #[inject] 属性"));

        let empty = key_of(parse_quote!(#[inject()])).expect_err("empty list must fail");
        assert!(empty.to_string().contains("不接受空参数"));

        let empty_name = key_of(parse_quote!(#[inject(key = "")])).expect_err("empty key");
        assert!(empty_name.to_string().contains("key 字符串不可为空"));

        let float = key_of(parse_quote!(#[inject(key = 1.5)])).expect_err("float key");
        assert!(float.to_string().contains("key 必须是字符串或非负整数值字面量"));

        let unknown = key_of(parse_quote!(#[inject(name = "x")])).expect_err("unknown key name");
        assert!(unknown.to_string().contains("只支持 key 参数"));

        let two = key_of(parse_quote!(#[inject(a = 1, b = 2)])).expect_err("two keys");
        assert!(two.to_string().contains("只接受一个 key 参数"));

        let name_value = key_of(parse_quote!(#[inject = 1])).expect_err("name-value form");
        assert!(name_value.to_string().contains("参数必须写在括号中"));
    }

    #[test]
    fn classifies_concrete_closed_generic_and_trait_requests() {
        assert_eq!(classify(&parse_quote!(Database)), DependencyShape::Concrete);
        assert_eq!(classify(&parse_quote!((Database))), DependencyShape::Concrete);
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
        assert!(
            nested
                .to_string()
                .contains("不接受最外层 Arc<T> 或嵌套 Option<T>")
        );

        let bare = split_optional(&parse_quote!(Option), FACTORY_MESSAGES)
            .expect_err("bare Option must fail");
        assert!(
            bare.to_string()
                .contains("`#[factory]` 可选参数必须写为 Option<T>")
        );

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

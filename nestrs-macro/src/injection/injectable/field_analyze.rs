//! `#[injectable]` 字段的语法分析。
//!
//! 这里刻意只描述字段的构造策略，不生成激活运行时 ABI。这样同一份
//! [`FieldSpec`] 后续可以同时驱动字段改写、依赖元数据和构造 adapter，避免
//! 多处重新解析 `#[inject]` 而产生漂移。

use crate::injection::attrs::service_key::ServiceKey;

use zyn::syn::{
    self, Attribute, Expr, ExprLit, Field, Fields, GenericArgument, Meta, PathArguments, Type,
    parse::Parser, punctuated::Punctuated, spanned::Spanned,
};

/// 一个字段在自动构造时的来源。
///
/// `Value` 会在宏生成的词法隔离构造 adapter 被调用时求值；`Default` 则由同一
/// adapter 调用 `Default::default()`。注入令牌按 `dependency_position` 从 activation
/// context 取得，三种策略始终复用这份分析结果。
#[derive(Clone, Debug)]
pub(crate) enum FieldStrategy {
    /// 从容器输入槽取得依赖。
    Inject {
        /// 请求服务的实际类型；对 `Option<T>` 字段已经剥离最外层 `Option`。
        service_type: Type,
        /// 可选的静态服务限定符。
        key: Option<ServiceKey>,
        /// 缺失服务时是否允许交付 `None`。
        optional: bool,
    },
    /// 在宏生成的构造 adapter 中原样求值的字段表达式。
    Value { expression: Expr },
    /// 未标注字段的默认构造策略。
    Default,
}

/// 一个字段的稳定宏期事实。
///
/// `dependency_position` 只为 `Inject` 分配，因而 `#[value(...)]` 与未标注字段
/// 不会影响容器输入的顺序。
#[derive(Clone, Debug)]
pub(crate) struct FieldSpec {
    /// 字段在声明中的零基位置。
    pub index: usize,
    /// 具名字段的名称；元组字段为 `None`。
    pub field_name: Option<syn::Ident>,
    /// 已解析的构造策略。
    pub strategy: FieldStrategy,
    /// 在生成构造 adapter 及 activation context 中的注入输入位置。
    pub dependency_position: Option<usize>,
}

impl FieldSpec {
    /// 此字段是否由 activation context 提供。
    ///
    /// 输出 element 可以直接用这个语义谓词组织 `@if`，无需各自重复解构
    /// `FieldStrategy`；实际的类型、key 和可选性仍只在消费该字段的 element 中
    /// 提取。
    pub(crate) fn is_injected(&self) -> bool {
        matches!(&self.strategy, FieldStrategy::Inject { .. })
    }
}

/// `#[injectable]` 的字段分析结果。
///
/// 这是宏入口的纯数据阶段，而不是 zyn token element：它把分析结果与清除 marker
/// 后的结构体一起交给三个平级的输出 element。这样“分析”不再承担编排职责，且
/// 三个 consumer 始终共享同一份 [`FieldSpec`]。
#[derive(Clone, Debug)]
pub(crate) struct AnalyzedFields {
    /// 已移除 `#[inject]` / `#[value]` marker 的原始结构体。
    pub item: syn::ItemStruct,
    /// 所有字段的稳定分析事实。
    pub specs: Vec<FieldSpec>,
}

impl AnalyzedFields {
    /// 是否存在需要从 [`ConstructionContext`](::nestrs_core::__private::ConstructionContext)
    /// 消费的字段。
    pub(crate) fn has_injected_fields(&self) -> bool {
        self.specs.iter().any(FieldSpec::is_injected)
    }
}

/// 判断一个注入目标是否是带实参的 concrete type path。
///
/// `Repository<User>` 与 `Repository<T>` 都属于这类路径：前者可直接产生已闭合
/// callback，后者则由开放 provider 的 `ComponentDefinition` impl 额外施加
/// `Repository<T>: ComponentDefinition` 约束后再单态化。`dyn Trait` 不属于
/// `Type::Path`，因此始终保留现有的 bind 解析路径。
pub(crate) fn is_generic_concrete_type_path(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path.qself.is_none()
        && type_path.path.segments.iter().any(|segment| {
            matches!(
                &segment.arguments,
                PathArguments::AngleBracketed(arguments) if !arguments.args.is_empty()
            )
        })
}

/// 分析字段并消费所有字段策略 marker。
///
/// 这一步只准备共享的宏期事实，不生成 token，也不调度任何输出 element。
pub(crate) fn analyze_fields(mut item: syn::ItemStruct) -> syn::Result<AnalyzedFields> {
    let specs = collect_field_specs(&item.fields)?;
    remove_field_strategy_attributes(&mut item.fields);

    Ok(AnalyzedFields { item, specs })
}

/// 分析一个 `#[injectable]` 的所有字段。
///
/// 接受具名、元组和单元结构体；元组字段保留位置语义，不杜撰会泄漏到运行时
/// metadata 的伪字段名。
pub(crate) fn collect_field_specs(fields: &Fields) -> syn::Result<Vec<FieldSpec>> {
    let mut specs = Vec::with_capacity(fields.len());

    for (index, field) in fields.iter().enumerate() {
        let inject_attributes = marker_attributes(&field.attrs, "inject");
        let value_attributes = marker_attributes(&field.attrs, "value");

        if !inject_attributes.is_empty() && !value_attributes.is_empty() {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "字段 `{}` 不能同时标注 #[inject] 和 #[value(...)]",
                    field_label(field, index)
                ),
            ));
        }

        let strategy = if !inject_attributes.is_empty() {
            let key = parse_inject_attribute(&inject_attributes)?;
            let (service_type, optional) = parse_inject_service_type(&field.ty)?;

            FieldStrategy::Inject {
                service_type,
                key,
                optional,
            }
        } else if !value_attributes.is_empty() {
            FieldStrategy::Value {
                expression: parse_value_attribute(&value_attributes)?,
            }
        } else {
            FieldStrategy::Default
        };

        specs.push(FieldSpec {
            index,
            field_name: field.ident.clone(),
            strategy,
            dependency_position: None,
        });
    }

    let mut next_dependency_position = 0usize;
    for spec in &mut specs {
        if matches!(spec.strategy, FieldStrategy::Inject { .. }) {
            spec.dependency_position = Some(next_dependency_position);
            next_dependency_position =
                next_dependency_position.checked_add(1).ok_or_else(|| {
                    syn::Error::new(
                        spec.field_name
                            .as_ref()
                            .map(Spanned::span)
                            .unwrap_or_else(zyn::proc_macro2::Span::call_site),
                        "单个 #[injectable] 的 #[inject] 字段数量过多",
                    )
                })?;
        }
    }

    Ok(specs)
}

/// 删除已经由 [`analyze_fields`] 消费的字段策略 marker。
///
/// `#[value(...)]` 的表达式已经被 [`FieldSpec`] 保存，后续阶段绝不再解析原始
/// 属性，因此这里清除它们可避免 Rust 报出未知属性，同时保证属性不会泄漏到最终
/// 结构体。
fn remove_field_strategy_attributes(fields: &mut Fields) {
    for field in fields.iter_mut() {
        field.attrs.retain(|attribute| {
            !attribute.path().is_ident("inject") && !attribute.path().is_ident("value")
        });
    }
}

fn marker_attributes<'a>(attributes: &'a [Attribute], name: &str) -> Vec<&'a Attribute> {
    attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident(name))
        .collect()
}

fn field_label(field: &Field, index: usize) -> String {
    field
        .ident
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| index.to_string())
}

/// 解析 `#[inject]`、`#[inject("name")]`、`#[inject(1)]` 以及与服务配置一致的
/// `#[inject(key = "name")]` / `#[inject(key = 1)]`。
fn parse_inject_attribute(attributes: &[&Attribute]) -> syn::Result<Option<ServiceKey>> {
    let attribute = exactly_one_attribute(attributes, "inject")?;

    match &attribute.meta {
        Meta::Path(_) => Ok(None),
        Meta::List(list) => {
            if list.tokens.is_empty() {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 不接受空参数；请使用 #[inject]、#[inject(\"key\")] 或 #[inject(key = \"key\")]",
                ));
            }

            if let Ok(literal) = syn::parse2::<syn::Lit>(list.tokens.clone()) {
                return parse_service_key_literal(&literal).map(Some);
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

            parse_service_key_expression(&value.value).map(Some)
        }
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attribute,
            "#[inject] 参数必须写在括号中",
        )),
    }
}

/// 严格解析 `#[value(<Rust expression>)]`，保留表达式 AST 给构造 adapter 使用。
fn parse_value_attribute(attributes: &[&Attribute]) -> syn::Result<Expr> {
    let attribute = exactly_one_attribute(attributes, "value")?;

    let Meta::List(list) = &attribute.meta else {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value] 必须写为 #[value(<Rust expression>)]",
        ));
    };

    if list.tokens.is_empty() {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value] 必须包含一个 Rust 表达式，例如 #[value(1)]",
        ));
    }

    let expression = syn::parse2::<Expr>(list.tokens.clone())?;
    if let Expr::Assign(assign) = &expression {
        if let Expr::Path(path) = assign.left.as_ref() {
            if path.path.is_ident("expr") {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[value(expr = ...)] 已移除；请改用 #[value(...)]",
                ));
            }
            if path.path.is_ident("func") {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[value(func = ...)] 已移除；请改用 #[value(path::to::function())]",
                ));
            }
        }
    }

    Ok(expression)
}

fn exactly_one_attribute<'a>(
    attributes: &[&'a Attribute],
    name: &str,
) -> syn::Result<&'a Attribute> {
    match attributes {
        [] => unreachable!("marker attribute was checked before parsing"),
        [attribute] => Ok(*attribute),
        [_, duplicate, ..] => Err(syn::Error::new_spanned(
            duplicate,
            format!("重复的 #[{name}] 属性"),
        )),
    }
}

fn parse_service_key_expression(expression: &Expr) -> syn::Result<ServiceKey> {
    let Expr::Lit(ExprLit { lit, .. }) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "key 必须是字符串或非负整数值字面量",
        ));
    };

    parse_service_key_literal(lit)
}

fn parse_service_key_literal(literal: &syn::Lit) -> syn::Result<ServiceKey> {
    match literal {
        syn::Lit::Str(value) if value.value().is_empty() => {
            Err(syn::Error::new_spanned(value, "key 字符串不可为空"))
        }
        syn::Lit::Str(value) => Ok(ServiceKey::Named(value.value())),
        syn::Lit::Int(value) => value
            .base10_parse::<usize>()
            .map(ServiceKey::Indexed)
            .map_err(|_| {
                syn::Error::new_spanned(value, "key 整数必须是可表示为 usize 的非负字面量")
            }),
        _ => Err(syn::Error::new_spanned(
            literal,
            "key 必须是字符串或非负整数值字面量",
        )),
    }
}

/// 分离唯一允许的可选注入形态 `Option<T>`。
fn parse_inject_service_type(ty: &Type) -> syn::Result<(Type, bool)> {
    if let Some(inner_type) = option_inner(ty)? {
        validate_inject_service_type(&inner_type, true)?;
        return Ok((inner_type, true));
    }

    validate_inject_service_type(ty, true)?;
    Ok((ty.clone(), false))
}

fn option_inner(ty: &Type) -> syn::Result<Option<Type>> {
    let Type::Path(type_path) = ty else {
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
        return Err(syn::Error::new_spanned(
            ty,
            "#[inject] 可选字段必须写为 Option<T>",
        ));
    };

    if arguments.args.len() != 1 {
        return Err(syn::Error::new_spanned(
            arguments,
            "#[inject] 可选字段必须写为 Option<T>",
        ));
    }

    let Some(GenericArgument::Type(inner_type)) = arguments.args.first() else {
        return Err(syn::Error::new_spanned(
            arguments,
            "#[inject] 可选字段必须写为 Option<T>",
        ));
    };

    Ok(Some(inner_type.clone()))
}

fn validate_inject_service_type(ty: &Type, is_top_level: bool) -> syn::Result<()> {
    match ty {
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
            GenericArgument::Type(ty) => validate_inject_service_type(ty, false)?,
            GenericArgument::AssocType(association) => {
                validate_inject_service_type(&association.ty, false)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::quote::ToTokens;

    fn fields(source: &str) -> Fields {
        syn::parse_str::<syn::ItemStruct>(source)
            .expect("test input should parse")
            .fields
    }

    #[test]
    fn analyzes_inject_value_and_default_once() {
        let specs = collect_field_specs(&fields(
            r#"
            struct Consumer {
                #[inject]
                database: Database,
                #[inject(key = "audit")]
                audit: Option<dyn Audit>,
                #[value(make_label("primary"))]
                label: String,
                retries: usize,
            }
            "#,
        ))
        .expect("fields should be valid");

        assert_eq!(specs.len(), 4);
        assert_eq!(specs[0].dependency_position, Some(0));
        assert_eq!(specs[1].dependency_position, Some(1));
        assert_eq!(specs[2].dependency_position, None);
        assert_eq!(specs[3].dependency_position, None);

        match &specs[0].strategy {
            FieldStrategy::Inject {
                service_type,
                key,
                optional,
            } => {
                assert_eq!(service_type.to_token_stream().to_string(), "Database");
                assert_eq!(key, &None);
                assert!(!optional);
            }
            strategy => panic!("unexpected strategy: {strategy:?}"),
        }

        match &specs[1].strategy {
            FieldStrategy::Inject {
                service_type,
                key: Some(ServiceKey::Named(key)),
                optional,
            } => {
                assert_eq!(service_type.to_token_stream().to_string(), "dyn Audit");
                assert_eq!(key, "audit");
                assert!(*optional);
            }
            strategy => panic!("unexpected strategy: {strategy:?}"),
        }

        match &specs[2].strategy {
            FieldStrategy::Value { expression } => {
                assert_eq!(
                    expression.to_token_stream().to_string(),
                    "make_label (\"primary\")"
                );
            }
            strategy => panic!("unexpected strategy: {strategy:?}"),
        }
        assert!(matches!(specs[3].strategy, FieldStrategy::Default));
    }

    #[test]
    fn supports_positional_key_forms() {
        let specs = collect_field_specs(&fields(
            r#"
            struct Consumer(
                #[inject("named")] Named,
                #[inject(7)] Indexed,
            );
            "#,
        ))
        .expect("fields should be valid");

        assert_eq!(specs[0].field_name, None);
        assert!(matches!(
            specs[0].strategy,
            FieldStrategy::Inject {
                key: Some(ServiceKey::Named(ref key)),
                optional: false,
                ..
            } if key == "named"
        ));
        assert!(matches!(
            specs[1].strategy,
            FieldStrategy::Inject {
                key: Some(ServiceKey::Indexed(7)),
                optional: false,
                ..
            }
        ));
    }

    #[test]
    fn rejects_conflicting_or_duplicate_markers() {
        let conflict = collect_field_specs(&fields(
            r#"
            struct Consumer {
                #[inject]
                #[value(1)]
                field: Service,
            }
            "#,
        ))
        .expect_err("markers must be mutually exclusive");
        assert!(conflict.to_string().contains("不能同时标注"));

        let duplicate = collect_field_specs(&fields(
            r#"
            struct Consumer {
                #[inject]
                #[inject]
                field: Service,
            }
            "#,
        ))
        .expect_err("duplicate inject markers must fail");
        assert!(duplicate.to_string().contains("重复的 #[inject] 属性"));
    }

    #[test]
    fn rejects_removed_value_named_syntax() {
        let error = collect_field_specs(&fields(
            r#"
            struct Consumer {
                #[value(expr = 1)]
                field: usize,
            }
            "#,
        ))
        .expect_err("legacy syntax must fail");

        assert!(error.to_string().contains("#[value(expr = ...)] 已移除"));
    }

    #[test]
    fn consumes_field_strategy_markers_after_analysis() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Consumer {
                #[inject]
                database: Database,
                #[doc = "kept"]
                #[value(1)]
                retries: usize,
            }
            "#,
        )
        .expect("test input should parse");

        let analyzed = analyze_fields(item).expect("fields should be valid");

        let fields: Vec<_> = analyzed.item.fields.iter().collect();
        assert!(fields[0].attrs.is_empty());
        assert_eq!(fields[1].attrs.len(), 1);
        assert!(fields[1].attrs[0].path().is_ident("doc"));
    }

    #[test]
    fn preserves_generic_parameters_for_component_definition_generation() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Repository<Entity>
            where
                Entity: Send + Sync + 'static,
            {
                marker: std::marker::PhantomData<Entity>,
            }
            "#,
        )
        .expect("test input should parse");

        let analyzed = analyze_fields(item).expect("generic injectable fields should be valid");

        assert_eq!(analyzed.item.generics.params.len(), 1);
        assert!(analyzed.item.generics.where_clause.is_some());
    }
}

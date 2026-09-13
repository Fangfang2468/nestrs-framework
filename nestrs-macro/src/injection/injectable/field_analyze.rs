//! `#[injectable]` 字段的语法分析。
//!
//! 这里刻意只描述字段的构造策略，不生成激活运行时 ABI。这样同一份
//! [`FieldSpec`] 后续可以同时驱动字段改写、依赖元数据和构造 adapter，避免
//! 多处重新解析 `#[inject]` 而产生漂移。

use crate::injection::{
    attrs::service_key::ServiceKey,
    request::{DependencyRequest, INJECTABLE_MESSAGES, inject_key, split_optional},
};

use zyn::syn::{self, Attribute, Expr, Field, Fields, Meta, Type, spanned::Spanned};

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

    /// 将注入字段的宏期事实转换为共享依赖请求。
    ///
    /// 只有 `#[inject]` 字段会产生依赖请求；`#[value]` 与默认字段既没有输入槽位，
    /// 也不参与 provider 的依赖描述。
    pub(crate) fn dependency_request(&self) -> DependencyRequest {
        let FieldStrategy::Inject {
            service_type,
            key,
            optional,
        } = &self.strategy
        else {
            unreachable!("only injected fields have a dependency request");
        };

        DependencyRequest {
            declaration_position: self.index,
            input_position: self
                .dependency_position
                .expect("inject field must have a dependency position"),
            service_type: service_type.clone(),
            key: key.clone(),
            optional: *optional,
            label: self.field_name.clone(),
        }
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
/// provider 依赖描述的伪字段名。
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
            let key = inject_key(&field.attrs)?;
            let (service_type, optional) = split_optional(&field.ty, INJECTABLE_MESSAGES)?;

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
    fn preserves_generic_parameters_for_provider_definition_generation() {
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

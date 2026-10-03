//! 服务策略在标准属性展开顺序下的契约；不需要 proc_macro 宿主或运行时容器。

use super::{
    conditional_fields, expand_configured_injectable, expand_factory, expand_injectable,
    expand_lazy, expand_primary,
};
use zyn::{TokenStream, quote::quote, syn};

/// rustc 每次只移除正在执行的属性。这里模拟相同顺序，保留后续属性给下一入口。
fn expand_stack(mut item: syn::Item) -> TokenStream {
    loop {
        let attrs = match &mut item {
            syn::Item::Struct(item) => &mut item.attrs,
            syn::Item::Fn(item) => &mut item.attrs,
            _ => panic!("测试声明只使用结构体和工厂"),
        };
        let current = attrs.remove(0);
        let name = current.path().segments.last().unwrap().ident.to_string();
        let args = match current.meta {
            syn::Meta::Path(_) => TokenStream::new(),
            syn::Meta::List(list) => list.tokens,
            syn::Meta::NameValue(_) => panic!("此辅助函数只接收合法属性外壳"),
        };
        let expanded = match name.as_str() {
            "injectable" => return expand_injectable(args, quote!(#item)),
            "factory" => return expand_factory(args, quote!(#item)),
            "primary" => expand_primary(args, quote!(#item)),
            "lazy" => expand_lazy(args, quote!(#item)),
            _ => panic!("非测试属性 {name}"),
        };
        let file = syn::parse2::<syn::File>(expanded).unwrap();
        item = file
            .items
            .into_iter()
            .find(|item| matches!(item, syn::Item::Struct(_) | syn::Item::Fn(_)))
            .expect("lazy/primary 必须保留原始声明");
    }
}

fn assert_policy(tokens: TokenStream, service: &str, value: Option<bool>, primary: bool) {
    syn::parse2::<syn::File>(tokens.clone()).expect("生成结果必须仍是合法 Rust 语法");
    let rendered = tokens.to_string();
    let code = match value {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    assert!(!rendered.contains("ProviderCommon"), "{rendered}");
    let marker = format!("compiler_plan_provider :: < {service} , 0u8 , {primary} , {code}u8 >");
    assert!(rendered.contains(&marker), "{rendered}");
    assert!(!rendered.contains("__nestrs_service_lazy"), "{rendered}");
}

#[test]
fn provider_policy_agrees_for_every_lazy_primary_provider_order() {
    // 同时覆盖服务路径、开放泛型与同步/异步工厂，三种配置都有一致的 metadata 编码。
    for (provider, body, service) in [
        (
            "injectable",
            quote!(
                struct Service;
            ),
            "Service",
        ),
        (
            "injectable",
            quote!(
                struct Generic<T> {
                    marker: std::marker::PhantomData<T>,
                }
            ),
            "Self",
        ),
        (
            "factory",
            quote!(
                fn make() -> Service {
                    Service
                }
            ),
            "Service",
        ),
        (
            "factory",
            quote!(
                async fn make() -> Result<Service, String> {
                    Ok(Service)
                }
            ),
            "Service",
        ),
    ] {
        for namespace in ["", "nestrs::", "declarations::"] {
            for (arguments, expected) in [
                (TokenStream::new(), true),
                (quote!(true), true),
                (quote!(false), false),
            ] {
                let provider: syn::Path =
                    syn::parse_str(&format!("{namespace}{provider}")).unwrap();
                let lazy: syn::Path = syn::parse_str(&format!("{namespace}lazy")).unwrap();
                let primary: syn::Path = syn::parse_str(&format!("{namespace}primary")).unwrap();
                let markers = [
                    quote!(#[#provider]),
                    quote!(#[#lazy(#arguments)]),
                    quote!(#[#primary]),
                ];
                for order in [
                    [0, 1, 2],
                    [0, 2, 1],
                    [1, 0, 2],
                    [1, 2, 0],
                    [2, 0, 1],
                    [2, 1, 0],
                ] {
                    let attributes = order.iter().map(|index| &markers[*index]);
                    let item = syn::parse2(quote!(#(#attributes)* #body)).unwrap();
                    assert_policy(expand_stack(item), service, Some(expected), true);
                }
            }
        }
    }
}

#[test]
fn unannotated_providers_inherit_and_bare_lazy_matches_empty_parentheses() {
    assert_policy(
        expand_injectable(
            TokenStream::new(),
            quote!(
                struct Service;
            ),
        ),
        "Service",
        None,
        false,
    );
    assert_policy(
        expand_factory(
            TokenStream::new(),
            quote!(
                fn make() -> Service {
                    Service
                }
            ),
        ),
        "Service",
        None,
        false,
    );
    for item in [
        syn::parse_quote!(
            #[lazy]
            #[injectable]
            struct Service;
        ),
        syn::parse_quote!(
            #[injectable]
            #[lazy]
            struct Service;
        ),
        syn::parse_quote!(
            #[lazy()]
            #[injectable]
            struct Service;
        ),
        syn::parse_quote!(
            #[injectable]
            #[lazy()]
            struct Service;
        ),
    ] {
        assert_policy(expand_stack(item), "Service", Some(true), false);
    }
}

#[test]
fn consuming_lower_lazy_retains_macro_import_usage() {
    for path in ["lazy", "nestrs::lazy", "declarations::lazy"] {
        let path: syn::Path = syn::parse_str(path).unwrap();
        let expected = quote!(use #path as _;).to_string();
        for body in [
            quote!(
                struct Service;
            ),
            quote!(
                struct Service<T>(std::marker::PhantomData<T>);
            ),
        ] {
            let output = expand_injectable(TokenStream::new(), quote!(#[#path(false)] #body));
            assert!(output.to_string().contains(&expected));
        }
        let output = expand_factory(
            TokenStream::new(),
            quote!(#[#path(false)] fn make() -> Service { Service }),
        );
        assert!(output.to_string().contains(&expected));
    }
}

#[test]
fn lazy_rejects_non_boolean_arguments_in_both_attribute_orders() {
    for args in [
        quote!(1),
        quote!("true"),
        quote!(!false),
        quote!(true, false),
        quote!(true,),
        quote!(enabled = true),
    ] {
        for output in [
            expand_lazy(
                args.clone(),
                quote!(
                    #[injectable]
                    struct Service;
                ),
            ),
            expand_injectable(
                TokenStream::new(),
                quote!(
                    #[lazy(#args)]
                    struct Service;
                ),
            ),
            expand_factory(
                TokenStream::new(),
                quote!(
                    #[lazy(#args)]
                    fn make() -> Service {
                        Service
                    }
                ),
            ),
        ] {
            let message = output.to_string();
            assert!(message.contains("compile_error"), "{message}");
            assert!(
                message.contains("只接受空参数或单个布尔字面量"),
                "{message}"
            );
        }
    }
    let output = expand_injectable(
        TokenStream::new(),
        quote!(
            #[lazy = false]
            struct Service;
        ),
    );
    assert!(output.to_string().contains("只接受空参数或单个布尔字面量"));
}

#[test]
fn lazy_rejects_duplicate_and_misplaced_service_markers() {
    for output in [
        expand_lazy(
            quote!(true),
            quote!(
                #[lazy(false)]
                #[injectable]
                struct Service;
            ),
        ),
        expand_lazy(
            quote!(true),
            quote!(
                #[injectable]
                #[lazy(false)]
                struct Service;
            ),
        ),
        expand_injectable(
            TokenStream::new(),
            quote!(
                #[lazy]
                #[nestrs::lazy(false)]
                struct Service;
            ),
        ),
        expand_factory(
            TokenStream::new(),
            quote!(
                #[lazy]
                #[lazy(false)]
                fn make() -> Service {
                    Service
                }
            ),
        ),
        expand_injectable(
            TokenStream::new(),
            quote!(
                #[__nestrs_service_lazy(true)]
                #[lazy]
                struct Service;
            ),
        ),
    ] {
        assert!(
            output
                .to_string()
                .contains("同一个服务声明不能重复标注 #[lazy]")
        );
    }
    for (item, message) in [
        (
            quote!(
                struct Service;
            ),
            "结构体上的 #[lazy] 必须与 #[injectable] 配合使用",
        ),
        (
            quote!(
                fn make() {}
            ),
            "函数上的 #[lazy] 必须与 #[factory] 配合使用",
        ),
        (
            quote!(
                enum Kind {}
            ),
            "#[lazy] 只能标注 #[injectable] 结构体或 #[factory] 函数",
        ),
        (
            quote!(impl Service {}),
            "#[lazy] 只能标注 #[injectable] 结构体或 #[factory] 函数",
        ),
    ] {
        let output = expand_lazy(TokenStream::new(), item).to_string();
        assert!(output.contains(message), "{output}");
    }
}

#[test]
fn conditional_field_carrier_preserves_service_policy_and_field_helper_boundaries() {
    // 模拟 derive 收到 rustc 已完成 cfg/cfg_attr 筛选的字段；服务属性在声明载荷中
    // 来回传递，不能误用字段 helper 的裸标记规则去解析服务级 lazy(false)。
    for marker in [
        quote!(#[lazy(false)]),
        quote!(#[__nestrs_service_lazy(false)]),
    ] {
        let original = syn::parse2(quote! {
            #marker
            struct Service {
                #[cfg(feature = "reports")]
                #[inject]
                #[lazy]
                reports: Reports,
            }
        })
        .unwrap();
        let carrier = conditional_fields::defer(TokenStream::new(), original).unwrap();
        let mut carrier: syn::ItemStruct = syn::parse2(carrier).unwrap();
        for field in &mut carrier.fields {
            field
                .attrs
                .retain(|attribute| !attribute.path().is_ident("cfg"));
        }
        let output = expand_configured_injectable(quote!(#carrier));
        assert_policy(output.clone(), "Service", Some(false), false);
        assert!(output.to_string().contains("LazyInjection < Reports >"));
    }
    for marker in [
        quote!(#[lazy()]),
        quote!(#[lazy(true)]),
        quote!(#[lazy(false)]),
    ] {
        let output = expand_injectable(
            TokenStream::new(),
            quote! {
                #[lazy(false)]
                struct Service { #[inject] #marker reports: Reports }
            },
        );
        assert!(
            output
                .to_string()
                .contains("延迟注入字段只接受无参数的 #[lazy] 属性")
        );
    }
}

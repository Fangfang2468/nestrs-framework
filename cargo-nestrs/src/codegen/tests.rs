//! These tests call the shared entry points in a normal test process. They catch
//! accidental dependence on the compiler's proc_macro execution context.

use super::{expand_bind, expand_factory, expand_injectable, expand_primary};
use zyn::{TokenStream, quote::quote, syn};

fn file(tokens: TokenStream) -> syn::File {
    syn::parse2(tokens).expect("declaration expansion should produce valid Rust items")
}

#[test]
fn injectable_entrypoint_preserves_user_items_and_lowers_injection_fields() {
    let expanded = file(expand_injectable(
        quote!(lifetime = Scoped, key = "requests"),
        quote! {
            #[derive(Debug)]
            pub struct Handler {
                #[inject]
                database: Database,
                #[inject]
                audit: Option<dyn Audit>,
                #[value("request")]
                label: String,
            }
        },
    ));
    let item = expanded
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) => Some(item),
            _ => None,
        })
        .expect("user struct is retained");
    assert_eq!(item.ident, "Handler");
    assert!(matches!(item.vis, syn::Visibility::Public(_)));
    assert!(item.attrs.iter().any(|attr| attr.path().is_ident("derive")));
    let fields: Vec<_> = item.fields.iter().collect();
    assert_eq!(
        fields[0].ty,
        syn::parse_quote!(::nestrs_core::__private::Injection<Database>)
    );
    assert_eq!(
        fields[1].ty,
        syn::parse_quote!(::core::option::Option<::nestrs_core::__private::Injection<dyn Audit>>)
    );
    assert_eq!(fields[2].ty, syn::parse_quote!(String));
    assert!(fields.iter().all(|field| {
        field
            .attrs
            .iter()
            .all(|attr| !attr.path().is_ident("inject") && !attr.path().is_ident("value"))
    }));
}

#[test]
fn factory_entrypoint_keeps_body_and_binds_parameters_to_activation_frame() {
    let original: syn::ItemFn = syn::parse_quote! {
        async fn connection(database: Database, #[inject(key = "audit")] audit: Option<dyn Audit>)
            -> Result<Connection, ConnectionError>
        {
            database.connect(audit).await
        }
    };
    let expanded = file(expand_factory(
        quote!(lifetime = Singleton),
        quote!(#original),
    ));
    let item = expanded
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item) => Some(item),
            _ => None,
        })
        .expect("user function is retained");
    assert_eq!(item.block, original.block);
    assert!(item.sig.asyncness.is_some());
    assert_eq!(item.sig.generics.lifetimes().count(), 1);
    let parameters: Vec<_> = item
        .sig
        .inputs
        .iter()
        .map(|parameter| match parameter {
            syn::FnArg::Typed(parameter) => parameter,
            _ => panic!("factory parameters must be ordinary arguments"),
        })
        .collect();
    assert_eq!(
        *parameters[0].ty,
        syn::parse_quote!(&'__nestrs_factory_frame Database)
    );
    assert_eq!(
        *parameters[1].ty,
        syn::parse_quote!(::core::option::Option<&'__nestrs_factory_frame dyn Audit>)
    );
    assert!(
        parameters
            .iter()
            .all(|parameter| parameter.attrs.is_empty())
    );
}

#[test]
fn namespaced_helpers_are_consumed_by_class_and_factory_declarations() {
    let expanded = file(expand_injectable(
        TokenStream::new(),
        quote! {
            struct Consumer {
                #[nestrs::inject]
                database: Database,
                #[nestrs::value(7)]
                retries: usize,
            }
        },
    ));
    let fields = expanded
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) => Some(&item.fields),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        fields.iter().next().unwrap().ty,
        syn::parse_quote!(::nestrs_core::__private::Injection<Database>)
    );
    assert!(fields.iter().all(|field| field.attrs.is_empty()));

    let expanded = file(expand_factory(
        TokenStream::new(),
        quote! {
            async fn connection(#[nestrs::inject(key = "main")] database: Database) -> Connection { Connection }
        },
    ));
    let argument = expanded
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(item) => item.sig.inputs.first(),
            _ => None,
        })
        .unwrap();
    let syn::FnArg::Typed(argument) = argument else {
        panic!("expected rewritten factory parameter")
    };
    assert_eq!(
        *argument.ty,
        syn::parse_quote!(&'__nestrs_factory_frame Database)
    );
    assert!(argument.attrs.is_empty());
}

#[test]
fn primary_handoff_can_be_expanded_without_a_proc_macro_host() {
    let expanded = file(expand_primary(
        TokenStream::new(),
        quote! {
            #[injectable]
            struct Service;
        },
    ));
    let mut item = expanded
        .items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) => Some(item),
            _ => None,
        })
        .expect("primary retains the declaration");
    assert!(
        item.attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("__nestrs_injectable_primary"))
    );
    // rustc removes the invoked attribute before passing the item to its macro.
    item.attrs
        .retain(|attribute| !attribute.path().is_ident("injectable"));
    let result = expand_injectable(TokenStream::new(), quote!(#item));
    file(result.clone());
    let tokens = result.to_string();
    assert!(tokens.contains("primary : true"));
    assert!(!tokens.contains("__nestrs_injectable_primary"));
}

#[test]
fn primary_before_factory_preserves_bare_qualified_and_crate_alias_paths() {
    for path in ["factory", "nestrs::factory", "declarations::factory"] {
        let attribute_path: syn::Path = syn::parse_str(path).unwrap();
        let expanded = file(expand_primary(
            TokenStream::new(),
            quote! {
                #[#attribute_path]
                async fn create(database: Database) -> Service { Service }
            },
        ));
        let mut function = expanded
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Fn(function) => Some(function),
                _ => None,
            })
            .expect("primary must retain the factory function");
        assert!(
            function
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("__nestrs_factory_primary")),
            "{path} must receive the primary marker before expansion",
        );
        // rustc removes the invoked factory attribute before calling its entry.
        function
            .attrs
            .retain(|attribute| attribute.path() != &attribute_path);
        let expanded = expand_factory(TokenStream::new(), quote!(#function));
        file(expanded.clone());
        let rendered = expanded.to_string();
        assert_eq!(rendered.matches("Provider :: Factory").count(), 1);
        assert_eq!(rendered.matches("primary : true").count(), 1);
        assert!(!rendered.contains("__nestrs_factory_primary"));
        assert!(rendered.contains("& '__nestrs_factory_frame Database"));
    }
}

#[test]
fn bind_entrypoint_preserves_the_impl_and_only_adds_projection_registration() {
    let original: syn::ItemImpl = syn::parse_quote! {
        impl Port for Service {
            fn name(&self) -> &str { "service" }
        }
    };
    let result = expand_bind(TokenStream::new(), quote!(#original));
    let expanded = file(result.clone());
    assert!(
        expanded
            .items
            .iter()
            .any(|item| { matches!(item, syn::Item::Impl(item) if item == &original) })
    );
    let tokens = result.to_string();
    assert!(tokens.contains("REFLECTED_BINDINGS"));
    assert!(!tokens.contains("REFLECTED_PROVIDERS"));
}

#[test]
fn invalid_inputs_keep_extractor_and_configuration_diagnostics() {
    let cases = [
        (
            expand_injectable(
                TokenStream::new(),
                quote!(
                    enum Wrong {}
                ),
            ),
            "expected struct input",
        ),
        (
            expand_factory(
                TokenStream::new(),
                quote!(
                    struct Wrong;
                ),
            ),
            "expected fn item input",
        ),
        (
            expand_bind(
                TokenStream::new(),
                quote!(
                    struct Wrong;
                ),
            ),
            "expected impl item input",
        ),
        (
            expand_injectable(
                quote!(42),
                quote!(
                    struct Service;
                ),
            ),
            "`#[injectable]` 参数填写格式错误",
        ),
        (
            expand_injectable(
                quote!(lifetime = Scoped, lifetime = Singleton),
                quote!(
                    struct Service;
                ),
            ),
            "`#[injectable]` 参数 `lifetime` 重复声明",
        ),
        (
            expand_primary(
                quote!(true),
                quote!(
                    struct Service;
                ),
            ),
            "`#[primary]` 不接受参数",
        ),
    ];
    for (tokens, diagnostic) in cases {
        let rendered = tokens.to_string();
        assert!(rendered.contains(diagnostic), "{rendered}");
        assert!(rendered.contains("compile_error"), "{rendered}");
    }
}

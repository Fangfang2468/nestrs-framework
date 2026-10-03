//! 将用户声明的真实 token 位置编码为工具私有的诊断来源标记。
//!
//! 图语义仍来自类型化策略 marker；这些空调用只为诊断保存名字和来源，不能作为
//! provider 注册或类型身份的证据。编译器按 marker 的真实 DefId、签名和生成来源认证。

use crate::protocol::{self, Marker, OriginKind};
use zyn::{
    TokenStream,
    meta::Args,
    proc_macro2::{Delimiter, Span, TokenTree},
    quote::{ToTokens, quote_spanned},
    syn::{self, spanned::Spanned},
};

/// 服务声明及策略的业务源码位置，仅供诊断定位，不参与类型匹配。
#[derive(Clone, Debug)]
pub(crate) struct ProviderOrigin {
    /// 业务结构体或工厂名，保留标识符的真实 span。
    pub(crate) name: syn::Ident,

    /// 显式生命周期值的来源位置；缺省策略没有独立位置。
    pub(crate) lifetime: Option<Span>,

    /// primary 属性或交接 marker 的来源位置。
    pub(crate) primary: Option<Span>,
}

impl ProviderOrigin {
    /// 从尚未消费的 provider 参数中提取显式策略来源。
    pub(crate) fn from_args(name: syn::Ident, args: &Args, primary: Option<Span>) -> Self {
        let lifetime = args.iter().find_map(|arg| match arg {
            zyn::Arg::Expr(name, expression) if name == "lifetime" => Some(expression.span()),
            _ => None,
        });
        Self {
            name,
            lifetime,
            primary,
        }
    }

    /// 输出声明与已配置策略的位置 marker，复用生成项的卫生上下文。
    pub(crate) fn render(&self, binding_span: Span) -> TokenStream {
        let mut output = origin(
            OriginKind::Declaration,
            0,
            &self.name.to_string(),
            self.name.span(),
            binding_span,
        );
        if let Some(span) = self.lifetime {
            output.extend(origin(OriginKind::Lifetime, 0, "", span, binding_span));
        }
        if let Some(span) = self.primary {
            output.extend(origin(OriginKind::Primary, 0, "", span, binding_span));
        }
        output
    }
}

/// 单独保存类型最后一个真实 token 的位置。原生 proc_macro 环境不能稳定 join
/// 多个 token 的 span，`Type::span()` 可能仅指向 `dyn` / 路径首段。driver 在自己的
/// SourceMap 中核验同文件及顺序后合并两个端点；这里不搜索源码或重建行列。
/// 无分隔分组由宏卫生产生，递归到其内部；有分隔分组以原来的闭合符号为终点。
pub(crate) fn type_end(ty: &impl ToTokens) -> Option<Span> {
    /// 取得最后一个实际 token 的端点，递归穿过没有分隔符的卫生分组。
    fn last(tokens: TokenStream) -> Option<Span> {
        match tokens.into_iter().last()? {
            TokenTree::Group(group) if group.delimiter() == Delimiter::None => {
                last(group.stream()).or_else(|| Some(group.span()))
            }
            TokenTree::Group(group) => Some(group.span_close()),
            token => Some(token.span()),
        }
    }
    last(ty.to_token_stream())
}

/// call 本身和 callee 均使用源位置，MIR source_info 因而不退化为整个属性调用。
/// 保留 label token 的位置，让 Cargo JSON 与跨 crate metadata 使用相同来源。
pub(crate) fn origin(
    kind: OriginKind,
    slot: usize,
    label: &str,
    span: Span,
    binding_span: Span,
) -> TokenStream {
    let module = syn::Ident::new(protocol::REFLECTION_MODULE, binding_span.located_at(span));
    let marker = syn::Ident::new(Marker::PlanOrigin.name(), binding_span.located_at(span));
    let kind = kind as u8;
    let label = syn::LitStr::new(label, span);
    quote_spanned!(span=> #module::#marker::<#kind, #slot>(#label);)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::{expand_constructor, expand_factory, expand_injectable};
    use zyn::syn::fold::Fold;

    #[derive(Default)]
    struct Calls(Vec<syn::ExprCall>);

    impl Fold for Calls {
        fn fold_macro(&mut self, mac: syn::Macro) -> syn::Macro {
            if mac
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "vec")
            {
                use syn::parse::Parser;
                let expressions =
                    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                        .parse2(mac.tokens.clone())
                        .expect("generated input vector");
                for expression in expressions {
                    self.fold_expr(expression);
                }
            }
            mac
        }
        fn fold_expr_call(&mut self, call: syn::ExprCall) -> syn::ExprCall {
            self.0.push(call.clone());
            syn::fold::fold_expr_call(self, call)
        }
    }

    fn calls(tokens: TokenStream, marker: Marker) -> Vec<syn::ExprCall> {
        let mut calls = Calls::default();
        calls.fold_file(syn::parse2(tokens).expect("valid expanded declaration"));
        calls.0.into_iter().filter(|call| matches!(&*call.func,
            syn::Expr::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == marker.name())
        )).collect()
    }

    fn assert_span(actual: Span, expected: Span) {
        assert_eq!(actual.start(), expected.start());
        assert_eq!(actual.end(), expected.end());
    }

    fn literal_arg(call: &syn::ExprCall, index: usize) -> &syn::Lit {
        let syn::Expr::Lit(expr) = &call.args[index] else {
            panic!("literal argument")
        };
        &expr.lit
    }

    #[test]
    fn field_input_keeps_type_name_and_key_locations_separate() {
        let item: syn::ItemStruct = syn::parse_str(
            "struct Checkout {\n    #[inject(\"live\")]\n    gateway: dyn PaymentGateway,\n}",
        )
        .unwrap();
        let field = item.fields.iter().next().unwrap();
        let syn::Meta::List(inject) = &field.attrs[0].meta else {
            panic!("inject arguments")
        };
        let key: syn::LitStr = syn::parse2(inject.tokens.clone()).unwrap();
        let expanded = expand_injectable(TokenStream::new(), item.to_token_stream());
        let inputs = calls(expanded.clone(), Marker::PlanInput);
        assert_eq!(inputs.len(), 1);
        assert_span(inputs[0].span(), field.ty.span());
        assert_span(
            literal_arg(&inputs[0], 1).span(),
            field.ident.as_ref().unwrap().span(),
        );
        let origins = calls(expanded, Marker::PlanOrigin);
        assert!(
            origins
                .iter()
                .any(|origin| origin.span().start() == key.span().start()
                    && origin.span().end() == key.span().end())
        );
    }

    #[test]
    fn provider_origin_keeps_policy_and_generic_declaration_locations() {
        let item: syn::ItemStruct =
            syn::parse_str("#[primary]\nstruct Repository<T> {\n    #[inject]\n    storage: T,\n}")
                .unwrap();
        let args: TokenStream = "lifetime = Scoped, key = \"db\"".parse().unwrap();
        let parsed: Args = syn::parse2(args.clone()).unwrap();
        let lifetime = parsed.iter().next().unwrap().as_expr().span();
        let expanded = expand_injectable(args, item.to_token_stream());
        let providers = calls(expanded.clone(), Marker::PlanProvider);
        assert_eq!(providers.len(), 1);
        assert_span(providers[0].span(), item.ident.span());
        let origins = calls(expanded, Marker::PlanOrigin);
        assert_eq!(origins.len(), 6);
        assert_span(origins[0].span(), item.ident.span());
        assert_span(origins[1].span(), lifetime);
        assert_span(origins[2].span(), item.attrs[0].path().span());
    }

    #[test]
    fn factory_keeps_user_function_parameter_and_success_type_locations() {
        let item: syn::ItemFn = syn::parse_str(
            "fn connect(\n    config: Config,\n) -> Result<Database, Error> { todo!() }",
        )
        .unwrap();
        let syn::FnArg::Typed(parameter) = &item.sig.inputs[0] else {
            panic!("typed parameter")
        };
        let expanded = expand_factory(TokenStream::new(), item.to_token_stream());
        let inputs = calls(expanded.clone(), Marker::PlanInput);
        assert_span(inputs[0].span(), parameter.ty.span());
        let providers = calls(expanded.clone(), Marker::PlanProvider);
        // The successful output alone is highlighted, rather than the Result wrapper.
        assert_eq!(providers[0].span().start().line, 3);
        assert_eq!(providers[0].span().start().column, 12);
        assert_eq!(providers[0].span().end().column, 20);
        let origins = calls(expanded, Marker::PlanOrigin);
        assert_span(origins[0].span(), item.sig.ident.span());
        assert!(
            matches!(literal_arg(&origins[0], 0), syn::Lit::Str(name) if name.value() == "connect")
        );
    }

    #[test]
    fn constructor_helper_keeps_parameter_and_user_method_identity() {
        let item: syn::ImplItemFn =
            syn::parse_str("fn create(\n    database: Database,\n) -> Self { Self {} }").unwrap();
        let syn::FnArg::Typed(parameter) = &item.sig.inputs[0] else {
            panic!("typed parameter")
        };
        let expanded = expand_constructor(TokenStream::new(), item.to_token_stream());
        let inputs = calls(expanded.clone(), Marker::PlanInput);
        assert_span(inputs[0].span(), parameter.ty.span());
        let origins = calls(expanded, Marker::PlanOrigin);
        assert_eq!(origins.len(), 2);
        assert_span(origins[0].span(), item.sig.ident.span());
        assert_eq!(origin_kind(&origins[0]), OriginKind::Constructor as u8);
        assert!(
            matches!(literal_arg(&origins[0], 0), syn::Lit::Str(name) if name.value() == "create")
        );
    }
    fn origin_kind(call: &syn::ExprCall) -> u8 {
        let syn::Expr::Path(path) = &*call.func else {
            panic!("marker path")
        };
        let syn::PathArguments::AngleBracketed(arguments) =
            &path.path.segments.last().unwrap().arguments
        else {
            panic!("marker arguments")
        };
        let syn::GenericArgument::Const(syn::Expr::Lit(literal)) = &arguments.args[0] else {
            panic!("kind constant")
        };
        let syn::Lit::Int(kind) = &literal.lit else {
            panic!("kind integer")
        };
        kind.base10_parse().unwrap()
    }

    #[test]
    fn type_endpoints_keep_trait_and_nested_generic_terminal_tokens() {
        for input in [
            "dyn Port",
            "dyn Port<User> + Send + Sync",
            "Repository<Vec<User>>",
        ] {
            let declaration: syn::ItemFn = syn::parse_str(&format!(
                "fn make(#[lazy] value: {input}) -> Repository<Vec<User>> {{ todo!() }}"
            ))
            .unwrap();
            let syn::FnArg::Typed(parameter) = &declaration.sig.inputs[0] else {
                panic!("typed parameter")
            };
            let syn::ReturnType::Type(_, output) = &declaration.sig.output else {
                panic!("output type")
            };
            let expanded = expand_factory(TokenStream::new(), declaration.to_token_stream());
            let origins = calls(expanded, Marker::PlanOrigin);
            let input_end = origins
                .iter()
                .find(|call| origin_kind(call) == OriginKind::InputTypeEnd as u8)
                .unwrap();
            let provider_end = origins
                .iter()
                .find(|call| origin_kind(call) == OriginKind::ProviderTypeEnd as u8)
                .unwrap();
            assert_eq!(input_end.span().end(), parameter.ty.span().end());
            assert_eq!(provider_end.span().end(), output.span().end());
            // Provider ending is the outermost `>` token itself, not the beginning of its path.
            assert_eq!(
                provider_end.span().end().column - provider_end.span().start().column,
                1
            );
        }
    }

    #[test]
    fn type_endpoints_descend_hygiene_groups_and_keep_explicit_closing_delimiters() {
        let tuple: TokenStream = "(User, Database)".parse().unwrap();
        let end = type_end(&tuple).unwrap();
        assert_eq!(end.start().column, 15);
        assert_eq!(end.end().column, 16);
        let grouped: TokenStream =
            TokenTree::Group(zyn::proc_macro2::Group::new(Delimiter::None, tuple)).into();
        assert_span(type_end(&grouped).unwrap(), end);
        let nested: TokenStream =
            TokenTree::Group(zyn::proc_macro2::Group::new(Delimiter::None, grouped)).into();
        assert_span(type_end(&nested).unwrap(), end);
    }
}

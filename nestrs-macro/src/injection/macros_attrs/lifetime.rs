use zyn::{
    syn::{spanned::Spanned, Expr, ExprLit, Lit},
    Arg, FromArg,
};

/// 属性宏在编译期使用的生命周期配置。
///
/// 它不复用 `nestrs-core` 的运行时枚举，确保过程宏实现本身不依赖 DI runtime。
/// 宏仅在生成的 token 中引用 `::nestrs_core::Lifetime`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceLifetime {
    /// Root frame 拥有且只激活一次的服务。
    Singleton,
    /// 每个 scope 独立拥有一次的服务。
    Scoped,
    /// 每个消费位置独立激活的服务。
    Transient,
}

impl FromArg for ServiceLifetime {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {
        // 提取出原始写法，例如 "scoped" / Scoped / scoped / ServiceLifetime::Scoped
        let raw: String = match arg {
            // lifetime = "scoped"
            Arg::Expr(
                _,
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }),
            ) => s.value(),
            Arg::Lit(Lit::Str(s)) => s.value(),

            // lifetime = Scoped / lifetime = scoped / lifetime = ServiceLifetime::Scoped
            Arg::Expr(_, Expr::Path(p)) => p.path.segments.last().unwrap().ident.to_string(),

            _ => {
                return Err(
                    zyn::mark::error("期望的格式 lifetime = \"scoped\" 或 lifetime = Scoped 或 lifetime = Lifetime::Scoped")
                        .span(arg.span())
                        .build(),
                );
            }
        };

        // 统一转 snake_case：Scoped / SCOPED / scoped → "scoped"
        let key = zyn::case::to_snake(&raw);

        match key.as_str() {
            "singleton" => Ok(Self::Singleton),
            "scoped" => Ok(Self::Scoped),
            "transient" => Ok(Self::Transient),
            other => Err(zyn::mark::error(format!("未知的生命周期 `{other}`"))
                .span(arg.span())
                .build()),
        }
    }
}

use zyn::{Arg, FromArg, syn::{Expr, Lit, ExprLit, spanned::Spanned}};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceKey {
    /// 没有 Key 值
    Nil,

    /// 同一服务类型命名空间中的静态名称。
    Named(String),
    
    /// 同一服务类型命名空间中的稳定编号。
    Indexed(usize),
}


impl FromArg for ServiceKey {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {

        // 提取出原始写法
        let raw = match arg {
            // key = "xxx" 或是 key = 12
            Arg::Expr(_, Expr::Lit(ExprLit { lit, .. })) => match lit {
                Lit::Str(str) => {
                    if str.value().is_empty() {
                        return Err(
                            zyn::mark::error("key = ... 字符串不可为空")
                                .span(str.span())
                                .build()
                        )
                    }

                    Self::Named(str.value())
                }
                Lit::Int(num) => num
                    .base10_parse::<usize>()
                    .map(Self::Indexed)
                    .map_err(|_| {
                        zyn::mark::error("key = ... 整数必须是可表示为 usize 的非负字面量")
                            .span(num.span())
                            .build()
                    })
                    .unwrap(),
                _ => {
                    return Err(
                        zyn::mark::error("key = ... 期望一个字符串或整数值字面量")
                            .span(lit.span())
                            .build(),
                    );
                }
            },

            _ => {
                return Err(
                    zyn::mark::error("key = ... 期望一个字符串或整数值字面量")
                        .span(arg.span())
                        .build(),
                );
            }
        };

        Ok(raw)
    }
}
use zyn::{
    syn::{spanned::Spanned, Expr, ExprLit, Lit, Path},
    Arg,
};

#[derive(Debug, Clone)]
pub struct CleanupPath {
    pub func_path: Path,
}

impl zyn::FromArg for CleanupPath {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {
        if let Arg::Expr(_, expr) = arg {
            match expr {
                Expr::Lit(ExprLit { lit, .. }) => match lit {
                    Lit::Str(func_path_litstr) => {
                        zyn::syn::parse_str::<Path>(&func_path_litstr.value())
                            .map(|func_path| Self { func_path })
                            .map_err(|error| {
                                zyn::mark::error(format!(
                                    "cleanup = ... 必须是合法函数路径：{error}"
                                ))
                                .span(func_path_litstr.span())
                                .build()
                            })
                    }
                    _ => {
                        return Err(zyn::mark::error("cleanup = ... 期望一个函数路径字面量")
                            .span(lit.span())
                            .build());
                    }
                },
                _ => {
                    return Err(zyn::mark::error("cleanup = ... 期望一个函数路径字面量")
                        .span(expr.span())
                        .build());
                }
            }
        } else {
            Err(zyn::mark::error("cleanup 参数格式填写错误")
                .span(arg.span())
                .build())
        }
    }
}

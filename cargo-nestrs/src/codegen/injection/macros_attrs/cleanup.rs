//! 解析服务关闭时调用的 cleanup 函数路径；签名由生成适配器的 Rust 类型检查验证。

use zyn::{
    Arg,
    syn::{Expr, ExprLit, Lit, Path, spanned::Spanned},
};

/// 用户声明的异步关闭回调路径，宏期不调用或求值该函数。
#[derive(Debug, Clone)]
pub struct CleanupPath {
    /// 从字符串字面量解析出的 Rust 函数路径。
    pub func_path: Path,
}

impl zyn::FromArg for CleanupPath {
    /// 解析 cleanup 字符串路径，并把格式错误定位到原属性值。
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
                    _ => Err(zyn::mark::error("cleanup = ... 期望一个函数路径字面量")
                        .span(lit.span())
                        .build()),
                },
                _ => Err(zyn::mark::error("cleanup = ... 期望一个函数路径字面量")
                    .span(expr.span())
                    .build()),
            }
        } else {
            Err(zyn::mark::error("cleanup 参数格式填写错误")
                .span(arg.span())
                .build())
        }
    }
}

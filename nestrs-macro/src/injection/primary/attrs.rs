use zyn::{
    Arg, FromArg,
    syn::{Path, spanned::Spanned},
};

#[derive(Debug, Clone, Default)]
pub struct PrimaryInterface {
    /// 手动指定的Trait名称
    pub trait_names: Vec<Path>,
}

impl FromArg for PrimaryInterface {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {
        let trait_names = match arg {
            // `#[primary]` 与 `#[primary()]` 会由 PrimaryConfig 包装为空列表。
            Arg::List(_, args) => args
                .iter()
                .map(|arg| match arg {
                    // `#[primary(TraitTest1, TraitTest2)]`
                    Arg::Flag(trait_name) => Ok(Path::from(trait_name.clone())),
                    _ => Err(
                        zyn::mark::error("`#[primary(...)]` 仅接受 Trait 标识符，例如 `#[primary(Trait1, Trait2)]`")
                            .span(arg.span())
                            .build()
                    ),
                })
                .collect::<zyn::Result<Vec<_>>>()?,

            // 允许该类型单独作为属性配置字段解析。
            Arg::Flag(trait_name) => vec![Path::from(trait_name.clone())],

            _ => {
                return Err(zyn::mark::error(
                    "`#[primary(...)]` 仅接受 Trait 标识符，例如 `#[primary(TraitTest1, TraitTest2)]`",
                )
                .span(arg.span())
                .build());
            }
        };

        Ok(Self { trait_names })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_primary(args: &str) -> PrimaryInterface {
        let args: zyn::Args = zyn::syn::parse_str(args).unwrap();
        let arg = Arg::List(zyn::format_ident!("primary"), args);

        PrimaryInterface::from_arg(&arg).unwrap()
    }

    #[test]
    fn supports_empty_primary_arguments() {
        assert!(parse_primary("").trait_names.is_empty());
    }

    #[test]
    fn supports_multiple_trait_identifiers() {
        let primary = parse_primary("TraitTest1, TraitTest2");
        let names: Vec<_> = primary
            .trait_names
            .iter()
            .map(|path| path.segments.last().unwrap().ident.to_string())
            .collect();

        assert_eq!(names, ["TraitTest1", "TraitTest2"]);
    }
}

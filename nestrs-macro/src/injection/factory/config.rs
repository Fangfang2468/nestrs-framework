//! `#[factory(...)]` 的 provider 配置解析。
//!
//! factory 与 `#[injectable]` 导出的 provider 共享 lifetime、key 与 cleanup
//! 语义，但它们是两个独立的宏入口。单独保留此配置类型可以避免 factory 的调用
//! 约束依附到结构体字段分析上，也让入口在处理 `#[primary]` 的属性顺序时只需要
//! 传入已经确定的布尔值。

use crate::injection::attrs::{
    cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKey,
};

use zyn::{Attribute, meta::Args, syn::spanned::Spanned};

/// `#[factory]` 的静态 provider 配置。
#[derive(Attribute, Clone, Debug)]
#[zyn("factory")]
pub(crate) struct FactoryConfig {
    /// 未声明时保持 singleton，和 `#[injectable]` 一致。
    #[zyn(default = ServiceLifetime::Singleton)]
    pub(crate) lifetime: ServiceLifetime,

    /// factory 成功输出使用的可选静态 key。
    #[zyn(default)]
    pub(crate) key: Option<ServiceKey>,

    /// provider 生命周期结束时的可选异步 cleanup 函数路径。
    #[zyn(default)]
    pub(crate) cleanup: Option<CleanupPath>,
}

/// 解析 `#[factory(...)]` 参数并在进入 derive 解析前统一检查命名和重复项。
///
/// `zyn::Attribute` 已负责各个值的强类型解析；这里保留与 injectable 入口相同的
/// 形状约束，确保 `#[factory("name")]` 这类位置参数不会悄悄获得另一套语义。
pub(crate) fn parse_factory_config(args: &Args) -> zyn::Result<FactoryConfig> {
    let mut seen = Vec::new();

    for argument in args.iter() {
        let Some(name) = argument.name() else {
            return Err(zyn::mark::error("`#[factory]` 参数填写格式错误")
                .span(argument.span())
                .build());
        };

        let name = name.to_string();
        if seen.contains(&name) {
            return Err(
                zyn::mark::error(format!("`#[factory]` 参数 `{name}` 重复声明"))
                    .span(argument.span())
                    .build(),
            );
        }
        seen.push(name);
    }

    FactoryConfig::from_args(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::{meta::Args, syn};

    #[test]
    fn accepts_the_same_provider_options_as_injectable() {
        let args: Args =
            syn::parse_str(r#"lifetime = "scoped", key = "replica", cleanup = "cleanup_replica""#)
                .expect("factory arguments should parse");

        let config = parse_factory_config(&args).expect("factory configuration should parse");

        assert_eq!(config.lifetime, ServiceLifetime::Scoped);
        assert_eq!(config.key, Some(ServiceKey::Named("replica".to_owned())));
        assert_eq!(
            config
                .cleanup
                .expect("cleanup should be configured")
                .func_path,
            syn::parse_str::<syn::Path>("cleanup_replica").expect("path should parse")
        );
    }

    #[test]
    fn rejects_duplicate_configuration_names() {
        let args: Args =
            syn::parse_str(r#"key = "a", key = "b""#).expect("factory arguments should parse");

        let error = parse_factory_config(&args).expect_err("duplicate keys must fail");

        assert!(error.to_string().contains("参数 `key` 重复声明"));
    }
}

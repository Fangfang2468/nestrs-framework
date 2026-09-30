//! 读取当前 package 的 Cargo.toml 顶层 `[nestrs-cli]` 启动配置。
//!
//! 配置只属于正在编译的应用 package。这里不合并 workspace 或依赖 crate 的配置，
//! 避免同一可执行入口的启动策略被依赖关系或 workspace 布局隐式改变。

use std::{fs, path::Path};

use toml::Table;

/// 经过验证的 DI 启动配置，供编译入口嵌入运行期默认值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiConfig {
    /// 是否在容器构建时预热 Singleton 及其必要依赖。
    pub eager: bool,
    /// 同一个 root 及全部 scope 共享的构造任务上限，始终大于零。
    pub max_concurrent_activations: usize,
}

impl Default for DiConfig {
    fn default() -> Self {
        Self {
            eager: false,
            max_concurrent_activations: 32,
        }
    }
}

/// 从指定 Cargo.toml 读取配置；读取和解析失败都保留实际文件路径。
pub fn read_manifest(path: &Path) -> Result<DiConfig, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("无法读取项目配置 {}：{error}", path.display()))?;
    parse_manifest(&text).map_err(|error| format!("项目配置 {} 无效：{error}", path.display()))
}

/// 只解析 `[nestrs-cli]`，未配置的字段使用 Lazy / 32。
pub fn parse_manifest(text: &str) -> Result<DiConfig, String> {
    let manifest = text
        .parse::<Table>()
        .map_err(|error| format!("Cargo.toml 的 TOML 语法错误：{error}"))?;
    let Some(value) = manifest.get("nestrs-cli") else {
        return Ok(DiConfig::default());
    };
    let table = value
        .as_table()
        .ok_or_else(|| "nestrs-cli 必须是 TOML 表".to_owned())?;

    // 配置键严格检查，避免拼写错误被当成默认值而静默改变启动行为。
    for key in table.keys() {
        if key != "initialization" && key != "max-concurrent-activations" {
            return Err(format!(
                "未知 nestrs-cli 配置项 nestrs-cli.{key}；支持 initialization 和 max-concurrent-activations"
            ));
        }
    }

    let mut config = DiConfig::default();
    if let Some(value) = table.get("initialization") {
        config.eager = match value.as_str() {
            Some("lazy") => false,
            Some("eager") => true,
            _ => {
                return Err(
                    "nestrs-cli.initialization 必须是字符串 \"lazy\" 或 \"eager\"".to_owned(),
                );
            }
        };
    }
    if let Some(value) = table.get("max-concurrent-activations") {
        config.max_concurrent_activations = value
            .as_integer()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                "nestrs-cli.max-concurrent-activations 必须是大于零且不超出 usize 范围的整数"
                    .to_owned()
            })?;
    }
    Ok(config)
}

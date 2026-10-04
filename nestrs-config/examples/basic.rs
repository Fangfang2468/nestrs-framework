//! 最小独立使用示例：内存默认值与环境式文本覆盖共同绑定一个普通 Serde 结构体。
//! 运行 `cargo run -p nestrs-config --no-default-features --example basic`，
//! 即使关闭所有文件格式 feature，也不需要 Nestrs 工具链或异步运行时。

use nestrs_config::sources::{Environment, Memory};
use nestrs_config::{ConfigError, ConfigLayer, ConfigPath, Configuration, LayerValue, Origin};
use serde::Deserialize;

// 模型的字段规则由普通 Serde 声明；本例不是配置宏或自动业务校验的演示。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpOptions {
    port: u16,
    // 输入缺失时使用 Vec 的默认空值；默认值只进入目标对象，不回写配置快照。
    #[serde(default)]
    allowed_origins: Vec<String>,
}

fn main() -> Result<(), ConfigError> {
    // 先构造低优先级默认层，来源名称用于安全定位，不参与值转换。
    let mut defaults = ConfigLayer::builder(Origin::named("defaults"));
    defaults.insert(
        ConfigPath::parse("app.http.port")?,
        LayerValue::unsigned(3000),
    )?;
    // 后添加的环境式来源覆盖端口。from_iter 使用显式输入，运行示例不会修改进程环境。
    // 去掉 APP__ 前缀后再按 __ 分段，因此剩余 APP__HTTP__PORT 映射为 app.http.port。
    let config = Configuration::builder()
        .add_source(Memory::new(defaults.build()?))
        .add_source(Environment::from_iter(
            "APP__",
            [("APP__APP__HTTP__PORT", "8080")],
        ))
        .build()?;
    // 环境文本在 u16 目标请求下转换；绑定得到独立拥有所有权的 HttpOptions。
    let http: HttpOptions = config.get_required("app.http")?;
    assert_eq!(http.port, 8080);
    assert!(http.allowed_origins.is_empty());
    println!("HTTP port: {}", http.port);
    Ok(())
}

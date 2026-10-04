//! 普通 Cargo 消费者验收：本项目有独立 workspace，仅依赖配置库与 Serde。
//! Cargo.toml 将 nestrs-config 重命名为 settings，并关闭默认 features，
//! 用于验证公开 API 不依赖原始依赖名、workspace 隐式依赖或 Nestrs 工具链。

use serde::Deserialize;
use settings::sources::Environment;
use settings::{ConfigError, ConfigService, Configuration};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct HttpOptions {
    port: u16,
    host: String,
    #[serde(default)]
    tls: bool,
}

// 泛型代码只依赖两方法协议；读取缺失值与必需值都不需要知道客户端内部模型。
fn port<C: ConfigService>(config: &C) -> Result<u16, ConfigError> {
    assert_eq!(config.get::<u16>("missing")?, None);
    config.get_required("http.port")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 使用确定的环境式输入，不要求运行机器存在特定配置文件或全局环境变量。
    let config = Configuration::builder()
        .add_source(Environment::from_iter(
            "DEMO__",
            [
                ("DEMO__HTTP__PORT", "8080"),
                ("DEMO__HTTP__HOST", "127.0.0.1"),
            ],
        ))
        .build()?;
    // 先验证静态泛型调用，再验证普通 Serde 对象绑定和相对 section 读取。
    assert_eq!(port(&config)?, 8080);
    assert_eq!(
        config.get_required::<HttpOptions>("http")?,
        HttpOptions {
            port: 8080,
            host: "127.0.0.1".into(),
            tls: false,
        }
    );
    assert_eq!(config.section("http")?.get_required::<u16>("port")?, 8080);
    // 外层集成测试使用固定成功标记确认此可执行程序确实运行完毕。
    println!("standalone config acceptance passed");
    Ok(())
}

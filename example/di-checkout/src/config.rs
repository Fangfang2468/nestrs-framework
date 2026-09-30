use nestrs::factory;
/// 默认本地配置是单例；异步工厂仅在构造期间借用它并复制需要的配置值。
#[derive(Debug)]
pub(crate) struct AppConfig {
    pub(crate) database_name: String,
    pub(crate) merchant_name: String,
    pub(crate) fail_payment_initialization: bool,
}

#[factory]
fn app_config() -> AppConfig {
    crate::observe::created("AppConfig");
    AppConfig {
        database_name: "local-memory-commerce".to_owned(),
        merchant_name: "Nestrs Demo Shop".to_owned(),
        fail_payment_initialization: std::env::var("NESTRS_EXAMPLE_FAIL_PAYMENT")
            .is_ok_and(|value| value == "1"),
    }
}

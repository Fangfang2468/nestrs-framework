#[nestrs::injectable]
pub struct Exported {
    #[nestrs::value(42)]
    pub number: u32,
}

pub fn answer() -> u32 {
    42
}

/// 对下游隐藏容器类型，但实际执行构图、服务查询和异步关闭。
/// 这会要求最终可执行入口拥有 registry，不能依赖链接器恰好裁掉未调用的 core 代码。
pub fn resolve_number() -> u32 {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(async {
            let provider = nestrs_core::ServiceProvider::build().await.unwrap();
            let number = nestrs_core::get_required_service!(provider, Exported)
                .await
                .unwrap()
                .number;
            provider.dispose_async().await.unwrap();
            number
        })
}

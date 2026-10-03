//! 仅间接依赖 core 的业务库仍须经过查询摘要管线。
#![allow(dead_code)]
use query_library::{Repository, ServiceProvider};
pub struct IndirectOnly;

async fn not_called(provider: &ServiceProvider) {
    if false {
        let _ = provider
            .get_required_service::<Repository<IndirectOnly>>()
            .await;
    }
}

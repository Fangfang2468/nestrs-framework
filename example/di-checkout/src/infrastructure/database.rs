use nestrs::factory;
use std::time::Duration;

use crate::config::AppConfig;

/// 模拟异步打开的数据库连接。真正的示例数据保存在 Repository 的内存表中。
pub(crate) struct Database {
    id: usize,
    name: String,
}

impl Database {
    #[cfg(test)]
    pub(crate) fn id(&self) -> usize {
        self.id
    }
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

#[factory(cleanup = "cleanup_database")]
async fn database(config: AppConfig) -> Database {
    // 宏将这个参数改写为借用；输出复制配置，不保留构造 frame 中的引用。
    crate::observe::event("factory start Database (160 ms)");
    tokio::time::sleep(Duration::from_millis(160)).await;
    let database = Database {
        id: crate::observe::created("Database"),
        name: config.database_name.clone(),
    };
    crate::observe::event(format!("factory end Database #{}", database.id));
    database
}

async fn cleanup_database() {
    crate::observe::event("cleanup hook Database（零参数，仅记录钩子调用）");
}

impl Drop for Database {
    fn drop(&mut self) {
        crate::observe::event(format!("drop Database #{} ({})", self.id, self.name));
    }
}

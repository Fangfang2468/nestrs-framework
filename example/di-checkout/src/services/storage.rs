use nestrs::{factory, injectable};
use std::{sync::Mutex, time::Duration};

use crate::domain::Order;

use super::AppConfig;

/// 模拟异步打开的数据库连接。真正的示例数据保存在 Repository 的内存表中。
pub struct Database {
    id: usize,
    name: String,
}

impl Database {
    pub fn id(&self) -> usize {
        self.id
    }
    pub fn name(&self) -> &str {
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

/// 单例泛型仓储：不同闭合类型各有一张内存表，但共享同一个 Database。
///
/// 编译器从 OrderStore 的普通 impl 确认 `Repository<Order>`；其它类型可由查询宏发现。
#[injectable]
pub struct Repository<T: Send + Sync + 'static> {
    #[inject]
    database: Database,
    records: Mutex<Vec<T>>,
    #[value(crate::observe::created(std::any::type_name::<Repository<T>>()))]
    id: usize,
}

impl<T: Send + Sync + 'static> Repository<T> {
    pub fn insert(&self, value: T) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(value);
    }

    pub fn all(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn id(&self) -> usize {
        self.id
    }
    pub fn database_id(&self) -> usize {
        self.database.id()
    }
}

impl<T: Send + Sync + 'static> Drop for Repository<T> {
    fn drop(&mut self) {
        crate::observe::event(format!(
            "drop {} #{}",
            std::any::type_name::<Self>(),
            self.id
        ));
    }
}

/// 结算用例依赖业务接口，不依赖某个仓储实现或容器查询方法。
pub trait OrderStore: Send + Sync {
    fn save(&self, order: Order);
    fn all(&self) -> Vec<Order>;
    fn instance_id(&self) -> usize;
    fn database_id(&self) -> usize;
}

impl OrderStore for Repository<Order> {
    fn save(&self, order: Order) {
        self.insert(order);
    }
    fn all(&self) -> Vec<Order> {
        Repository::all(self)
    }
    fn instance_id(&self) -> usize {
        self.id()
    }
    fn database_id(&self) -> usize {
        Repository::database_id(self)
    }
}

use nestrs::injectable;
use std::sync::Mutex;

use crate::domain::{Order, OrderStore};

use super::database::Database;

/// 单例泛型仓储：不同闭合类型各有一张内存表，但共享同一个 Database。
///
/// 编译器从 OrderStore 的普通 impl 确认 `Repository<Order>`；其它闭合类型可由查询方法的编译期分析发现。
#[injectable]
pub(crate) struct Repository<T: Send + Sync + 'static> {
    #[inject]
    database: Database,
    records: Mutex<Vec<T>>,
    #[value(crate::observe::created(std::any::type_name::<Repository<T>>()))]
    id: usize,
}

impl<T: Send + Sync + 'static> Repository<T> {
    pub(crate) fn insert(&self, value: T) {
        crate::observe::event(format!(
            "业务记录写入 {} / {}",
            self.database.name(),
            std::any::type_name::<T>(),
        ));
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(value);
    }

    pub(crate) fn all(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn id(&self) -> usize {
        self.id
    }
    #[cfg(test)]
    pub(crate) fn database_id(&self) -> usize {
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

impl OrderStore for Repository<Order> {
    fn save(&self, order: Order) {
        self.insert(order);
    }
    fn all(&self) -> Vec<Order> {
        Repository::all(self)
    }
    #[cfg(test)]
    fn instance_id(&self) -> usize {
        self.id()
    }
    #[cfg(test)]
    fn database_id(&self) -> usize {
        Repository::database_id(self)
    }
}

//! 为任务记录中的反向依赖、查询等待者和 lazy watch 提供紧凑存储。
//!
//! 零项和单项内联保存；多个不同 key 使用哈希映射，多个 lazy watch 使用顺序列表。
//! 映射移除成员后保留已有哈希表，不反复降级、搬迁或扫描剩余订阅。

use std::{collections::hash_map, hash::Hash};

use ahash::AHashMap;

/// 按 key 保存等待者；零或一个键值对内联，第二个不同 key 才分配哈希表。
pub(super) enum CompactMap<K, V> {
    /// 没有键值对，也没有堆分配。
    Empty,

    /// 唯一的 key 和 value 直接保存在枚举内。
    One(K, V),

    /// 使用装箱哈希表；移除至零或一项后也保留该形态。
    /// 单独装箱表头，避免撑大常见的单等待者任务记录。
    #[allow(clippy::box_collection)]
    Many(Box<AHashMap<K, V>>),
}

impl<K: Eq + Hash, V> CompactMap<K, V> {
    /// 创建不分配堆存储的空映射。
    pub(super) fn new() -> Self {
        Self::Empty
    }

    /// 插入键值对；同 key 替换并返回旧值，新 key 返回 None。
    /// 内联形态只有遇到第二个不同 key 时才升级为哈希映射。
    pub(super) fn insert(&mut self, key: K, value: V) -> Option<V> {
        match self {
            Self::Empty => {
                *self = Self::One(key, value);
                None
            }
            Self::One(previous, slot) if *previous == key => Some(std::mem::replace(slot, value)),
            Self::One(..) => {
                let Self::One(previous, slot) = std::mem::replace(self, Self::Empty) else {
                    unreachable!();
                };
                let mut values = AHashMap::with_capacity(2);
                values.insert(previous, slot);
                values.insert(key, value);
                *self = Self::Many(Box::new(values));
                None
            }
            Self::Many(values) => values.insert(key, value),
        }
    }

    /// 按 key 移出旧值，不存在时返回 None；已有哈希表不缩回内联形态。
    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        match self {
            Self::Empty => None,
            Self::One(previous, _) if previous != key => None,
            Self::One(..) => {
                let Self::One(_, value) = std::mem::replace(self, Self::Empty) else {
                    unreachable!();
                };
                Some(value)
            }
            Self::Many(values) => values.remove(key),
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(..) => 1,
            Self::Many(values) => values.len(),
        }
    }
}

/// 消费映射并逐项移交键值对，不为遍历建立中转集合。
pub(super) enum MapIntoIter<K, V> {
    /// 保存至多一个尚未移出的键值对；None 同时表示原映射为空或已取完。
    One(Option<(K, V)>),

    /// 接管原哈希表的拥有型迭代器；遍历顺序不作保证。
    Many(hash_map::IntoIter<K, V>),
}

impl<K, V> Iterator for MapIntoIter<K, V> {
    /// 移交所有权的一个键值对。
    type Item = (K, V);

    /// 取走唯一的内联键值对，或继续消费原哈希表迭代器。
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(value) => value.take(),
            Self::Many(values) => values.next(),
        }
    }
}

impl<K, V> IntoIterator for CompactMap<K, V> {
    /// 消费映射时移交的键值对。
    type Item = (K, V);

    /// 对应内联形态或原哈希表的拥有型迭代器。
    type IntoIter = MapIntoIter<K, V>;

    /// 消费映射；空和单项转为 Option，哈希形态直接接管原表的迭代器。
    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Empty => MapIntoIter::One(None),
            Self::One(key, value) => MapIntoIter::One(Some((key, value))),
            Self::Many(values) => MapIntoIter::Many((*values).into_iter()),
        }
    }
}

/// 使用成员作为映射 key 保存唯一反向依赖；单个成员无需堆分配。
pub(super) struct CompactSet<T>(CompactMap<T, ()>);

impl<T: Eq + Hash> CompactSet<T> {
    /// 创建不分配堆存储的空集合。
    pub(super) fn new() -> Self {
        Self(CompactMap::new())
    }

    /// 插入成员，仅首次插入返回 true；重复成员不增加数量。
    pub(super) fn insert(&mut self, value: T) -> bool {
        self.0.insert(value, ()).is_none()
    }

    /// 移除相等成员，返回是否原本存在；沿用底层映射的存储形态。
    pub(super) fn remove(&mut self, value: &T) -> bool {
        self.0.remove(value).is_some()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(test)]
    pub(super) fn contains(&self, value: &T) -> bool {
        match &self.0 {
            CompactMap::Empty => false,
            CompactMap::One(previous, ()) => previous == value,
            CompactMap::Many(values) => values.contains_key(value),
        }
    }
}

impl<T> IntoIterator for CompactSet<T> {
    /// 从集合移出的唯一成员。
    type Item = T;

    /// 在底层映射迭代器上丢弃单元值，只保留成员。
    type IntoIter = std::iter::Map<MapIntoIter<T, ()>, fn((T, ())) -> T>;

    /// 消费底层映射并移交其 key，不额外收集成员，也不保证遍历顺序。
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().map(|(value, ())| value)
    }
}

/// 按加入顺序保留 lazy watch 发送者直至构造结束；不按 key 去重或移除。
pub(super) enum CompactList<T> {
    /// 没有列表元素，也没有堆分配。
    Empty,

    /// 唯一元素直接保存在枚举内。
    One(T),

    /// 按加入顺序保存多个元素；装箱 Vec 头以缩小单 watch 任务记录。
    #[allow(clippy::box_collection)]
    Many(Box<Vec<T>>),
}

impl<T> CompactList<T> {
    /// 创建不分配堆存储的空列表。
    pub(super) fn new() -> Self {
        Self::Empty
    }

    /// 在末尾追加元素；第二次追加时把内联元素和新元素一同移入 Vec。
    pub(super) fn push(&mut self, value: T) {
        match self {
            Self::Empty => *self = Self::One(value),
            Self::One(..) => {
                let Self::One(previous) = std::mem::replace(self, Self::Empty) else {
                    unreachable!();
                };
                *self = Self::Many(Box::new(vec![previous, value]));
            }
            Self::Many(values) => values.push(value),
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(_) => 1,
            Self::Many(values) => values.len(),
        }
    }
}

/// 消费紧凑列表的拥有型迭代器，保持元素的加入顺序。
pub(super) enum ListIntoIter<T> {
    /// 保存至多一个尚未移出的元素；None 表示原列表为空或已取完。
    One(Option<T>),

    /// 接管原 Vec 的拥有型迭代器，继续按顺序移交元素。
    Many(std::vec::IntoIter<T>),
}

impl<T> Iterator for ListIntoIter<T> {
    /// 从列表移出并交给调用者的元素。
    type Item = T;

    /// 取走唯一的内联元素，或返回原 Vec 中下一个未消费元素。
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(value) => value.take(),
            Self::Many(values) => values.next(),
        }
    }
}

impl<T> IntoIterator for CompactList<T> {
    /// 按加入顺序移交的列表元素。
    type Item = T;

    /// 对应内联形态或原 Vec 的拥有型迭代器。
    type IntoIter = ListIntoIter<T>;

    /// 消费列表；空和单项转为 Option，多项直接接管原 Vec 的迭代器。
    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Empty => ListIntoIter::One(None),
            Self::One(value) => ListIntoIter::One(Some(value)),
            Self::Many(values) => ListIntoIter::Many((*values).into_iter()),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/compact.rs"]
mod tests;

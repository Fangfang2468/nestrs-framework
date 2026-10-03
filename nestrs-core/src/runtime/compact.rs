//! 单订阅直接存在任务记录中；共享任务才分配哈希表，保留按标识退订的复杂度。
//!
//! 多项形态不在逐个取消时降级，否则高扇出任务会反复搬迁或扫描剩余订阅。

use std::{collections::hash_map, hash::Hash};

use ahash::AHashMap;

pub(super) enum CompactMap<K, V> {
    Empty,
    One(K, V),
    // 装箱只发生在共享路径，避免哈希表头撑大每个单订阅任务记录。
    #[allow(clippy::box_collection)]
    Many(Box<AHashMap<K, V>>),
}

impl<K: Eq + Hash, V> CompactMap<K, V> {
    pub(super) fn new() -> Self {
        Self::Empty
    }

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

pub(super) enum MapIntoIter<K, V> {
    One(Option<(K, V)>),
    Many(hash_map::IntoIter<K, V>),
}

impl<K, V> Iterator for MapIntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(value) => value.take(),
            Self::Many(values) => values.next(),
        }
    }
}

impl<K, V> IntoIterator for CompactMap<K, V> {
    type Item = (K, V);
    type IntoIter = MapIntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Empty => MapIntoIter::One(None),
            Self::One(key, value) => MapIntoIter::One(Some((key, value))),
            Self::Many(values) => MapIntoIter::Many((*values).into_iter()),
        }
    }
}

pub(super) struct CompactSet<T>(CompactMap<T, ()>);

impl<T: Eq + Hash> CompactSet<T> {
    pub(super) fn new() -> Self {
        Self(CompactMap::new())
    }

    pub(super) fn insert(&mut self, value: T) -> bool {
        self.0.insert(value, ()).is_none()
    }

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
    type Item = T;
    type IntoIter = std::iter::Map<MapIntoIter<T, ()>, fn((T, ())) -> T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().map(|(value, ())| value)
    }
}

/// Lazy watch 不按 QueryId 退订，只需保留接收端对应的发送者直至构造结束。
pub(super) enum CompactList<T> {
    Empty,
    One(T),
    // 两级存储换取单订阅记录小于 Vec；只有多 watch 的共享构造付出额外分配。
    #[allow(clippy::box_collection)]
    Many(Box<Vec<T>>),
}

impl<T> CompactList<T> {
    pub(super) fn new() -> Self {
        Self::Empty
    }

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

pub(super) enum ListIntoIter<T> {
    One(Option<T>),
    Many(std::vec::IntoIter<T>),
}

impl<T> Iterator for ListIntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(value) => value.take(),
            Self::Many(values) => values.next(),
        }
    }
}

impl<T> IntoIterator for CompactList<T> {
    type Item = T;
    type IntoIter = ListIntoIter<T>;

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

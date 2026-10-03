//! 协调器复用的关闭排序工作空间，不持有实例或用户回调。
//!
//! 完整冻结 DAG 仍参与反向 Kahn；节点桶只保存 journal 下标，实例始终留在原 journal。
//! 同一协调器同步完成排序后才启动 worker，因此所有 owner 可共用这些临时数组。

use std::collections::BinaryHeap;

use crate::graph::ValidatedGraph;

use super::owner::Published;

const NONE: usize = usize::MAX;

#[derive(Default)]
pub(super) struct CleanupOrder {
    heads: Vec<usize>,
    remaining: Vec<usize>,
    next: Vec<usize>,
    destinations: Vec<usize>,
    ready: BinaryHeap<(usize, usize)>,
    targets: Vec<usize>,
    peak_entries: usize,
}

impl CleanupOrder {
    pub(super) fn order(&mut self, journal: &mut [Published], graph: &ValidatedGraph) {
        if journal.len() < 2 {
            return;
        }
        self.peak_entries = self.peak_entries.max(journal.len());
        self.heads.clear();
        self.heads.resize(graph.nodes.len(), NONE);
        self.remaining.clear();
        self.remaining.extend(graph.dependents.iter().map(Vec::len));
        self.next.clear();
        self.destinations.resize(journal.len(), NONE);
        for (sequence, entry) in journal.iter().enumerate() {
            self.next.push(self.heads[entry.provider]);
            self.heads[entry.provider] = sequence;
        }
        self.ready.clear();
        for (provider, &count) in self.remaining.iter().enumerate() {
            if count == 0 {
                self.ready.push((self.heads[provider], provider));
            }
        }

        let mut destination = journal.len();
        while let Some((sequence, provider)) = self.ready.pop() {
            if sequence != NONE {
                // cleanup 从 journal 尾部取出，消费者优先的次序直接映射到反向下标。
                destination -= 1;
                self.destinations[sequence] = destination;
                self.heads[provider] = self.next[sequence];
                if self.heads[provider] != NONE {
                    self.ready.push((self.heads[provider], provider));
                    continue;
                }
            }
            // 使用同一个小工作数组去重槽位；未实例化 provider 也执行这一传播。
            self.targets.clear();
            self.targets.extend(
                graph.nodes[provider]
                    .dependencies
                    .iter()
                    .filter_map(|dependency| dependency.input.target()),
            );
            self.targets.sort_unstable();
            self.targets.dedup();
            for &target in &self.targets {
                self.remaining[target] -= 1;
                if self.remaining[target] == 0 {
                    self.ready.push((self.heads[target], target));
                }
            }
        }
        assert_eq!(destination, 0, "已验证 DAG 必须可完整排序");
        // 置换只交换 journal 内的 lease，不克隆、不释放、不构造第二份实例容器。
        for index in 0..journal.len() {
            while self.destinations[index] != index {
                let target = self.destinations[index];
                journal.swap(index, target);
                self.destinations.swap(index, target);
            }
        }
    }

    /// 以一整个调度窗口内实际需要的最大 journal 为基准，仅回收显著过剩的容量。
    pub(super) fn reclaim(&mut self) {
        let target = self.peak_entries.max(64).saturating_mul(2);
        for buffer in [&mut self.next, &mut self.destinations] {
            buffer.clear();
            if buffer.capacity() > target.saturating_mul(2) {
                buffer.shrink_to(target);
            }
        }
        self.peak_entries = 0;
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/cleanup.rs"]
mod tests;

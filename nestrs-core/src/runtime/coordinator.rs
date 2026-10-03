//! 唯一持有可变调度状态的协调器。
//!
//! 外部命令、worker 完成与关闭推进都在同一事件循环处理，因此缓存合并和任务依赖
//! 不需要额外锁。任务表只保存未结束的 occurrence；已完成的共享结果放在 owner 缓存。
//! 展开和失败传播都使用显式工作队列，深层服务图不会变成 Rust 调用栈。

use ahash::{AHashMap, AHashSet};
use std::{collections::VecDeque, sync::Arc};

use tokio::{
    sync::mpsc,
    task::{Id, JoinError, JoinSet},
};

use crate::{
    activation::ReleaseDomain,
    error::{DisposeError, ResolveError},
    graph::{DependencyInput, ValidatedGraph},
    lifetime::ServiceLifetime,
    panic_payload::PanicPayload,
};

use super::{
    CloseWaiter, OwnerId, QueryId, Resolution, TaskId,
    handle::Command,
    owner::{CacheEntry, OwnerData, OwnerPhase, OwnerState, ROOT},
    task::{Activation, ResolutionWaiter, TaskRequest, TaskState},
    worker::{ActivationWorker, CleanupWorker},
};

/// Tokio 的 JoinError 只携带任务 ID，必须独立记录所属激活/cleanup，才能把 panic
/// 转回对应服务的诊断。这里的映射不参与服务缓存，也不能替代活跃构造任务表。
enum JobKind {
    Activation(TaskId),
    Cleanup { owner: OwnerId, provider: usize },
}

enum JobCompletion {
    Activation(Resolution),
    Cleanup(Vec<String>),
}

pub(super) struct Coordinator {
    graph: Arc<ValidatedGraph>,
    domain: Arc<ReleaseDomain>,
    commands: mpsc::UnboundedReceiver<Command>,
    commands_open: bool,
    // 调度索引采用随机种子的 aHash；依赖推进与关闭次序仍由显式队列和图顺序决定。
    owners: AHashMap<OwnerId, OwnerState>,
    tasks: AHashMap<TaskId, Activation>,
    // 只定位尚未完成的普通查询。退订直接找到所属任务，不扫描其他等待者或任务。
    query_tasks: AHashMap<QueryId, TaskId>,
    next_task: TaskId,
    ready: VecDeque<TaskId>,
    jobs: JoinSet<JobCompletion>,
    job_kinds: AHashMap<Id, JobKind>,
    running_activations: usize,
    max_activations: usize,
    closed_scope_errors: Vec<String>,
}

impl Coordinator {
    pub(super) fn new(
        graph: Arc<ValidatedGraph>,
        root: Arc<OwnerData>,
        commands: mpsc::UnboundedReceiver<Command>,
        max_activations: usize,
    ) -> Self {
        Self {
            graph,
            domain: ReleaseDomain::new(),
            commands,
            commands_open: true,
            owners: AHashMap::from([(ROOT, OwnerState::new(root))]),
            tasks: AHashMap::new(),
            query_tasks: AHashMap::new(),
            next_task: 0,
            ready: VecDeque::new(),
            jobs: JoinSet::new(),
            job_kinds: AHashMap::new(),
            running_activations: 0,
            max_activations,
            closed_scope_errors: Vec::new(),
        }
    }

    pub(super) async fn run(mut self) {
        loop {
            // 依赖一满足就入队并争取可用构造名额，不设置“整层完成”屏障。
            self.launch_ready();
            self.advance_closures();
            if self.owners[&ROOT].phase == OwnerPhase::Closed {
                break;
            }
            tokio::select! {
                command = self.commands.recv(), if self.commands_open => {
                    match command {
                        Some(command) => self.handle_command(command),
                        None => {
                            self.commands_open = false;
                            let root = self.owners[&ROOT].data.clone();
                            self.begin_close(root, None);
                        }
                    }
                }
                completion = self.jobs.join_next_with_id(), if !self.jobs.is_empty() => {
                    if let Some(completion) = completion {
                        self.handle_completion(completion);
                    }
                }
            }
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Register(data) => {
                let mut state = OwnerState::new(data);
                if self.owners[&ROOT].phase != OwnerPhase::Open {
                    state.begin_close();
                }
                self.owners.insert(state.data.id, state);
            }
            Command::Resolve {
                query,
                owner,
                provider,
                waiter,
            } => {
                self.accept_resolution(owner, provider, (query, waiter));
            }
            Command::CancelQuery(query) => self.cancel_query(query),
            Command::ResolveLazy {
                owner,
                provider,
                waiter,
            } => {
                self.accept_resolution(owner, provider, ResolutionWaiter::Lazy(waiter));
            }
            Command::Close { owner, waiter } => self.begin_close(owner, waiter),
        }
    }

    fn accept_resolution(
        &mut self,
        owner: OwnerId,
        provider: usize,
        waiter: impl Into<ResolutionWaiter>,
    ) {
        let waiter = waiter.into();
        // 句柄提交前的原子检查只是快速失败；命令在排队期间可能发生关闭，
        // 因此真正“接受工作”的决定仍由协调器在这里作出。
        let open = self
            .owners
            .get(&owner)
            .is_some_and(|owner| owner.phase == OwnerPhase::Open);
        if !open || self.owners[&ROOT].phase != OwnerPhase::Open {
            waiter.send(Err(ResolveError::closed()));
            return;
        }
        let Some(node) = self.graph.nodes.get(provider) else {
            waiter.send(Err(ResolveError::new("无效的 provider 计划编号".into())));
            return;
        };
        if owner == ROOT && node.requires_scope {
            waiter.send(Err(ResolveError::construction(
                &node.identifier,
                node.common.source,
                "此服务的依赖闭包需要 scope，不能从 root provider 获取".into(),
            )));
            return;
        }
        let mut expansion = Vec::new();
        match self.ensure_task(owner, provider, &mut expansion) {
            TaskRequest::Cached(result) => {
                waiter.send(result);
            }
            TaskRequest::Pending(task) => {
                let activation = self.tasks.get_mut(&task).unwrap();
                match waiter {
                    ResolutionWaiter::Query(query, waiter) => {
                        // 即使接收端已经取消，也保留本次已接受的初始化；只是无需安装
                        // 无人等待的订阅。已安装的订阅随后由 CancelQuery 定位移除。
                        if !waiter.is_closed() {
                            activation.query_waiters.insert(query, waiter);
                            self.query_tasks.insert(query, task);
                        }
                    }
                    ResolutionWaiter::Lazy(waiter) => activation.lazy_waiters.push(waiter),
                }
            }
        }
        self.expand(&mut expansion);
    }

    fn cancel_query(&mut self, query: QueryId) {
        if let Some(task) = self.query_tasks.remove(&query) {
            // 完成路径会同时删除索引；因此存在索引时任务和等待者都必须仍然存在。
            self.tasks
                .get_mut(&task)
                .unwrap()
                .query_waiters
                .remove(&query);
        }
    }

    /// 先按生命周期确定真实 owner，再查共享缓存。Transient 永远创建新 occurrence。
    /// 返回缓存结果时不会创建任务；Building 则让所有并发消费者共用已有的 TaskId。
    fn ensure_task(
        &mut self,
        requested_owner: OwnerId,
        provider: usize,
        expansion: &mut Vec<TaskId>,
    ) -> TaskRequest {
        let lifetime = self.graph.nodes[provider].common.lifetime;
        let owner = if lifetime == ServiceLifetime::Singleton {
            ROOT
        } else {
            requested_owner
        };
        let cached = lifetime != ServiceLifetime::Transient;
        if cached && let Some(entry) = self.owners[&owner].cache.get(&provider) {
            return match entry {
                CacheEntry::Building(task) => TaskRequest::Pending(*task),
                CacheEntry::Ready(lease) => TaskRequest::Cached(Ok(lease.clone())),
                CacheEntry::Failed(error) => TaskRequest::Cached(Err(error.clone())),
            };
        }
        let task = self.next_task;
        self.next_task += 1;
        self.tasks.insert(
            task,
            Activation {
                owner,
                provider,
                state: TaskState::Unexpanded,
                parents: AHashSet::new(),
                query_waiters: AHashMap::new(),
                lazy_waiters: Vec::new(),
            },
        );
        let state = self.owners.get_mut(&owner).unwrap();
        state.active_tasks.insert(task);
        if cached {
            state.cache.insert(provider, CacheEntry::Building(task));
        }
        expansion.push(task);
        TaskRequest::Pending(task)
    }

    /// 只展开必要子图。缓存命中截断展开，缺席 optional 保留空槽位。
    ///
    /// 即使遇到已缓存失败，也完成本次已选节点的输入展开。由此接受的其他子任务
    /// 仍须排空并交由 owner 收纳，不能因为某个父任务失败而取消共享工作。
    fn expand(&mut self, expansion: &mut Vec<TaskId>) {
        while let Some(task) = expansion.pop() {
            let Some(activation) = self.tasks.get_mut(&task) else {
                // 之前传播的依赖失败已经使该任务退役；它此前接受的孩子仍独立留在队列中。
                continue;
            };
            debug_assert!(matches!(activation.state, TaskState::Unexpanded));
            let (owner, provider) = (activation.owner, activation.provider);
            activation.state = TaskState::Waiting {
                inputs: vec![None; self.graph.nodes[provider].dependencies.len()],
                children: vec![None; self.graph.nodes[provider].dependencies.len()],
                remaining: 0,
            };
            let targets: Vec<_> = self.graph.nodes[provider]
                .dependencies
                .iter()
                .map(|dependency| match &dependency.input {
                    DependencyInput::Immediate { target, .. } => Some(*target),
                    // 缺席输入直接交付 None，延迟输入只交付句柄，都不占前置任务。
                    DependencyInput::Absent(_) | DependencyInput::Lazy { .. } => None,
                })
                .collect();
            let mut failure = None;
            for (index, target) in targets.into_iter().enumerate() {
                let Some(target) = target else { continue };
                debug_assert!(
                    self.graph.dependents[target]
                        .binary_search(&provider)
                        .is_ok()
                );
                let request = self.ensure_task(owner, target, expansion);
                let activation = self.tasks.get_mut(&task).unwrap();
                let TaskState::Waiting {
                    inputs,
                    children,
                    remaining,
                } = &mut activation.state
                else {
                    unreachable!("正在展开的任务必须处于等待输入阶段");
                };
                match request {
                    TaskRequest::Cached(Ok(lease)) => inputs[index] = Some(lease),
                    TaskRequest::Cached(Err(error)) => {
                        failure.get_or_insert(error);
                    }
                    TaskRequest::Pending(child) => {
                        *remaining += 1;
                        children[index] = Some(child);
                        self.tasks
                            .get_mut(&child)
                            .unwrap()
                            .parents
                            .insert((task, index));
                    }
                }
            }
            if let Some(error) = failure {
                let node = &self.graph.nodes[provider];
                self.settle(
                    task,
                    Err(ResolveError::dependency(
                        &node.identifier,
                        node.common.source,
                        error,
                    )),
                );
            } else {
                self.queue_if_ready(task);
            }
        }
    }

    fn queue_if_ready(&mut self, task: TaskId) {
        let activation = self.tasks.get_mut(&task).unwrap();
        if let TaskState::Waiting {
            remaining: 0,
            inputs,
            ..
        } = &mut activation.state
        {
            let inputs = std::mem::take(inputs);
            activation.state = TaskState::Queued { inputs };
            self.ready.push_back(task);
        }
    }

    /// 结束一次 occurrence，并用显式队列传播失败/推进消费者。
    ///
    /// 顺序不可颠倒：发布 journal → 更新共享缓存 → 通知等待者/消费者。
    /// 所有完成任务立刻离开 tasks；失败缓存和成功 lease 独立留在 owner 中。
    fn settle(&mut self, task: TaskId, result: Resolution) {
        let mut completed = VecDeque::from([(task, result)]);
        while let Some((task, result)) = completed.pop_front() {
            let Some(activation) = self.tasks.remove(&task) else {
                // 多条失败边可以到达同一个消费者，但它只完成一次。
                continue;
            };
            // 提前失败只结束当前消费者，已接受的孩子仍独立排空。按仍未就绪的槽位
            // 移除子任务的反向订阅，避免长时间 Pending 的共享孩子保存历史失败父节点。
            if let TaskState::Waiting { children, .. } = &activation.state {
                for (input, child) in children.iter().enumerate() {
                    if let Some(child) = child.and_then(|child| self.tasks.get_mut(&child)) {
                        child.parents.remove(&(task, input));
                    }
                }
            }
            let owner = self.owners.get_mut(&activation.owner).unwrap();
            let removed = owner.active_tasks.remove(&task);
            debug_assert!(removed, "活跃任务表与 owner 活跃集合必须一致");
            if let Ok(lease) = &result {
                owner.data.publish(activation.provider, lease.clone());
            }
            if self.graph.nodes[activation.provider].common.lifetime != ServiceLifetime::Transient {
                owner
                    .cache
                    .insert(activation.provider, CacheEntry::completed(&result));
            }
            for (query, waiter) in activation.query_waiters {
                self.query_tasks.remove(&query);
                let _ = waiter.send(result.clone());
            }
            for waiter in activation.lazy_waiters {
                let _ = waiter.send(Some(result.clone()));
            }
            for (parent, input) in activation.parents {
                let Some(parent_state) = self.tasks.get_mut(&parent) else {
                    continue;
                };
                let TaskState::Waiting {
                    inputs,
                    children,
                    remaining,
                } = &mut parent_state.state
                else {
                    unreachable!("仍在等待依赖的消费者必须处于 Waiting 阶段");
                };
                let child = children[input].take();
                debug_assert_eq!(child, Some(task), "输入槽位必须仍订阅当前子任务");
                match &result {
                    Ok(lease) => {
                        inputs[input] = Some(lease.clone());
                        *remaining -= 1;
                        self.queue_if_ready(parent);
                    }
                    Err(error) => {
                        let node = &self.graph.nodes[parent_state.provider];
                        completed.push_back((
                            parent,
                            Err(ResolveError::dependency(
                                &node.identifier,
                                node.common.source,
                                error.clone(),
                            )),
                        ));
                    }
                }
            }
        }
    }

    fn launch_ready(&mut self) {
        while self.running_activations < self.max_activations {
            let Some(task) = self.ready.pop_front() else {
                break;
            };
            let Some(activation) = self.tasks.get_mut(&task) else {
                continue;
            };
            let TaskState::Queued { inputs } = &mut activation.state else {
                continue;
            };
            let inputs = std::mem::take(inputs);
            activation.state = TaskState::Running;
            let provider = activation.provider;
            let graph = self.graph.clone();
            let domain = self.domain.clone();
            // 只传实际 owner 的弱请求能力，Singleton 始终绑定 root。worker 按计划的
            // Lazy 分支现场包装字段句柄；调度器无需另建逐槽位的延迟输入数组。
            let resolver = Arc::downgrade(&self.owners[&activation.owner].data);
            let worker = ActivationWorker::new(graph, provider, inputs, resolver, domain);
            let handle = self
                .jobs
                .spawn(async move { JobCompletion::Activation(worker.run().await) });
            self.job_kinds
                .insert(handle.id(), JobKind::Activation(task));
            self.running_activations += 1;
        }
    }

    fn handle_completion(&mut self, completion: Result<(Id, JobCompletion), JoinError>) {
        let id = match &completion {
            Ok((id, _)) => *id,
            Err(error) => error.id(),
        };
        let kind = self
            .job_kinds
            .remove(&id)
            .expect("每个 worker 必须有归属记录");
        // JoinError 拥有用户 panic 载荷；必须在保护范围内消费它，不能让它在
        // 协调器的 match 分支末尾隐式析构。取消错误没有需要回收的 panic 载荷。
        let completion = completion.map_err(|error| {
            let message = error.to_string();
            match error.try_into_panic() {
                Ok(payload) => PanicPayload::new(payload).finish_message(message),
                Err(_) => message,
            }
        });
        match kind {
            JobKind::Activation(task) => {
                self.running_activations -= 1;
                let activation = &self.tasks[&task];
                let node = &self.graph.nodes[activation.provider];
                let result = match completion {
                    Ok((_, JobCompletion::Activation(result))) => result,
                    Err(error) => Err(ResolveError::construction(
                        &node.identifier,
                        node.common.source,
                        format!("构造任务终止：{error}"),
                    )),
                    _ => unreachable!("构造 worker 只能返回构造结果"),
                };
                self.settle(task, result);
            }
            JobKind::Cleanup { owner, provider } => {
                let errors = match completion {
                    Ok((_, JobCompletion::Cleanup(errors))) => errors,
                    Err(error) => vec![format!(
                        "{:?} cleanup 任务终止：{error}",
                        self.graph.nodes[provider].identifier
                    )],
                    _ => unreachable!("cleanup worker 只能返回清理结果"),
                };
                let owner = self.owners.get_mut(&owner).unwrap();
                debug_assert_eq!(owner.phase, OwnerPhase::Cleaning { running: true });
                owner.phase = OwnerPhase::Cleaning { running: false };
                owner.errors.extend(errors);
            }
        }
    }

    fn begin_close(&mut self, owner: Arc<OwnerData>, waiter: Option<CloseWaiter>) {
        if let Some(result) = owner.completed_close() {
            if let Some(waiter) = waiter {
                let _ = waiter.send(result);
            }
            return;
        }
        let Some(state) = self.owners.get_mut(&owner.id) else {
            if let Some(waiter) = waiter {
                let _ = waiter.send(Err(DisposeError::coordinator_stopped()));
            }
            return;
        };
        if let Some(waiter) = waiter {
            state.close_waiters.push(waiter);
        }
        state.begin_close();
        if owner.id == ROOT {
            for state in self.owners.values_mut() {
                state.begin_close();
            }
        }
    }

    fn advance_closures(&mut self) {
        // 扫描全部 owner 使排空和 root 等待 scope 的顺序保持直观。
        // 如果大量 scope 的量测证明这一步是瓶颈，再考虑按状态变化入队推进关闭。
        let mut owners: Vec<_> = self.owners.keys().copied().collect();
        // root 最后检查，否则关闭多个空 scope 后可能再无事件唤醒 root。
        owners.sort_unstable_by_key(|owner| *owner == ROOT);
        for owner in owners {
            let state = &self.owners[&owner];
            if state.phase == OwnerPhase::Draining {
                if !state.active_tasks.is_empty() || (owner == ROOT && self.owners.len() != 1) {
                    continue;
                }
                let state = self.owners.get_mut(&owner).unwrap();
                // 此时活跃任务全部退役；去掉缓存强 lease，journal 独立持有待清理实例。
                state.cache.clear();
                state.data.order_cleanup(&self.graph);
                state.phase = OwnerPhase::Cleaning { running: false };
            }
            if self.owners[&owner].phase != (OwnerPhase::Cleaning { running: false }) {
                continue;
            }
            if let Some(entry) = self.owners[&owner].data.next_cleanup() {
                let provider = entry.provider;
                let graph = self.graph.clone();
                self.owners.get_mut(&owner).unwrap().phase = OwnerPhase::Cleaning { running: true };
                // 只有上一项 cleanup/释放完成，才会把 running 改回 false 并取下一项。
                // 仅保证启动次序是不够的：依赖不能先于消费者的异步 hook 完成清理。
                let worker = CleanupWorker::new(entry, graph);
                let handle = self
                    .jobs
                    .spawn(async move { JobCompletion::Cleanup(worker.run().await) });
                self.job_kinds
                    .insert(handle.id(), JobKind::Cleanup { owner, provider });
            } else {
                let state = self.owners.get_mut(&owner).unwrap();
                if owner == ROOT {
                    state.errors.append(&mut self.closed_scope_errors);
                }
                let result = if state.errors.is_empty() {
                    Ok(())
                } else {
                    Err(DisposeError::new(state.errors.clone()))
                };
                state.data.complete_close(result.clone());
                state.phase = OwnerPhase::Closed;
                for waiter in std::mem::take(&mut state.close_waiters) {
                    let _ = waiter.send(result.clone());
                }
                if owner != ROOT {
                    let state = self.owners.remove(&owner).unwrap();
                    self.closed_scope_errors.extend(state.errors);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/coordinator.rs"]
mod tests;

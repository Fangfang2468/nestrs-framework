//! A single, iterative Tokio coordinator for every owner in a provider.
//!
//! The coordinator alone mutates activation state. Workers receive already prepared dependency
//! leases, and never recursively resolve services. Owner journals live independently of the
//! coordinator so a stopped Tokio runtime cannot invalidate a reference returned to its owner.

use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    task::Poll,
};

use tokio::{
    sync::{mpsc, oneshot},
    task::{Id, JoinError, JoinSet},
};

use crate::{
    activation::{ActivationPreparation, DependencyLease, ReleaseDomain},
    facade::{DisposeError, ResolveError},
    graph::{Constructor, ValidatedGraph},
    lifetime::ServiceLifetime,
    registration::provider::FactoryInvoker,
};

type OwnerId = u64;
type TaskId = u64;
type Resolution = Result<DependencyLease, ResolveError>;
type ResolveWaiter = oneshot::Sender<Resolution>;
type CloseWaiter = oneshot::Sender<Result<(), DisposeError>>;

const ROOT: OwnerId = 0;
const OPEN: u8 = 0;
const CLOSING: u8 = 1;
const CLOSED: u8 = 2;

/// An API lifetime boundary. The coordinator retains its data, but never this handle itself.
pub(crate) struct Owner {
    data: Arc<OwnerData>,
    commands: mpsc::UnboundedSender<Command>,
}

impl Owner {
    pub(crate) fn is_closed(&self) -> bool {
        self.data.status.load(Ordering::Acquire) != OPEN
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if self.data.status.load(Ordering::Acquire) != CLOSED {
            let _ = self.commands.send(Command::Close {
                owner: self.data.clone(),
                waiter: None,
            });
        }
    }
}

struct OwnerData {
    id: OwnerId,
    status: AtomicU8,
    // This must be owned by the public lifetime handle, not only by the coordinator.
    journal: Mutex<Vec<Published>>,
    close_result: Mutex<Option<Result<(), DisposeError>>>,
}

impl OwnerData {
    fn new(id: OwnerId) -> Arc<Self> {
        Arc::new(Self {
            id,
            status: AtomicU8::new(OPEN),
            journal: Mutex::new(Vec::new()),
            close_result: Mutex::new(None),
        })
    }

    fn completed_close(&self) -> Option<Result<(), DisposeError>> {
        self.close_result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

struct Published {
    provider: usize,
    lease: DependencyLease,
}

/// A command handle. Background work is deliberately independent of any individual get future.
pub(crate) struct Runtime {
    graph: Arc<ValidatedGraph>,
    commands: mpsc::UnboundedSender<Command>,
    next_owner: AtomicU64,
}

impl Runtime {
    pub(crate) fn start(graph: Arc<ValidatedGraph>, max: usize) -> (Arc<Self>, Arc<Owner>) {
        assert!(max > 0, "activation concurrency must be nonzero");
        let (commands, receiver) = mpsc::unbounded_channel();
        let root = OwnerData::new(ROOT);
        let owner = Arc::new(Owner {
            data: root.clone(),
            commands: commands.clone(),
        });
        let runtime = Arc::new(Self {
            graph: graph.clone(),
            commands,
            next_owner: AtomicU64::new(1),
        });
        tokio::spawn(Coordinator::new(graph, root, receiver, max).run());
        (runtime, owner)
    }

    pub(crate) fn create_scope(self: &Arc<Self>) -> Arc<Owner> {
        let data = OwnerData::new(self.next_owner.fetch_add(1, Ordering::Relaxed));
        if self.commands.send(Command::Register(data.clone())).is_err() {
            data.status.store(CLOSED, Ordering::Release);
            *data
                .close_result
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Err(coordinator_stopped()));
        }
        Arc::new(Owner {
            data,
            commands: self.commands.clone(),
        })
    }

    pub(crate) async fn resolve(&self, owner: &Arc<Owner>, provider: usize) -> Resolution {
        let receiver = self.request_resolution(owner, provider)?;
        receiver
            .await
            .unwrap_or_else(|_| Err(ResolveError::closed()))
    }

    fn request_resolution(
        &self,
        owner: &Arc<Owner>,
        provider: usize,
    ) -> Result<oneshot::Receiver<Resolution>, ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        let (waiter, receiver) = oneshot::channel();
        self.commands
            .send(Command::Resolve {
                owner: owner.data.id,
                provider,
                waiter,
            })
            .map_err(|_| ResolveError::closed())?;
        Ok(receiver)
    }

    pub(crate) async fn warm_up(
        &self,
        owner: &Arc<Owner>,
        lifetime: ServiceLifetime,
    ) -> Result<(), ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        // Submit the complete warm-up before awaiting any node, allowing independent work to run.
        let mut receivers = Vec::new();
        for &provider in &self.graph.topological_order {
            let node = &self.graph.nodes[provider];
            if node.common.lifetime == lifetime {
                receivers.push(self.request_resolution(owner, provider)?);
            }
        }
        let mut first_error = None;
        for receiver in receivers {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(ResolveError::closed()));
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) async fn close(&self, owner: &Arc<Owner>) -> Result<(), DisposeError> {
        if let Some(result) = owner.data.completed_close() {
            return result;
        }
        let (waiter, receiver) = oneshot::channel();
        if self
            .commands
            .send(Command::Close {
                owner: owner.data.clone(),
                waiter: Some(waiter),
            })
            .is_err()
        {
            return owner
                .data
                .completed_close()
                .unwrap_or_else(|| Err(coordinator_stopped()));
        }
        receiver
            .await
            .unwrap_or_else(|_| Err(coordinator_stopped()))
    }

    pub(crate) fn request_close(&self, owner: &Arc<Owner>) {
        let _ = self.commands.send(Command::Close {
            owner: owner.data.clone(),
            waiter: None,
        });
    }
}

fn coordinator_stopped() -> DisposeError {
    DisposeError::new(vec![
        "Tokio 协调任务已经停止；异步 cleanup 未确认完成".to_owned(),
    ])
}

enum Command {
    Register(Arc<OwnerData>),
    Resolve {
        owner: OwnerId,
        provider: usize,
        waiter: ResolveWaiter,
    },
    Close {
        owner: Arc<OwnerData>,
        waiter: Option<CloseWaiter>,
    },
}

struct OwnerState {
    data: Arc<OwnerData>,
    cache: HashMap<usize, TaskId>,
    task_ids: Vec<TaskId>,
    pending: usize,
    cleaning: bool,
    cleanup_running: bool,
    errors: Vec<String>,
    close_waiters: Vec<CloseWaiter>,
}

impl OwnerState {
    fn new(data: Arc<OwnerData>) -> Self {
        Self {
            data,
            cache: HashMap::new(),
            task_ids: Vec::new(),
            pending: 0,
            cleaning: false,
            cleanup_running: false,
            errors: Vec::new(),
            close_waiters: Vec::new(),
        }
    }
}

enum TaskState {
    Waiting,
    Queued,
    Running,
    Complete(Resolution),
}

struct Activation {
    owner: OwnerId,
    provider: usize,
    state: TaskState,
    expanded: bool,
    pending: usize,
    inputs: Vec<Option<DependencyLease>>,
    parents: Vec<(TaskId, usize)>,
    waiters: Vec<ResolveWaiter>,
}

enum JobKind {
    Activation(TaskId),
    Cleanup { owner: OwnerId, provider: usize },
}

enum JobCompletion {
    Activation(Resolution),
    Cleanup(Vec<String>),
}

struct Coordinator {
    graph: Arc<ValidatedGraph>,
    domain: Arc<ReleaseDomain>,
    commands: mpsc::UnboundedReceiver<Command>,
    commands_open: bool,
    owners: HashMap<OwnerId, OwnerState>,
    tasks: HashMap<TaskId, Activation>,
    next_task: TaskId,
    ready: VecDeque<TaskId>,
    jobs: JoinSet<JobCompletion>,
    job_kinds: HashMap<Id, JobKind>,
    running_activations: usize,
    max_activations: usize,
    closed_scope_errors: Vec<String>,
}

impl Coordinator {
    fn new(
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
            owners: HashMap::from([(ROOT, OwnerState::new(root))]),
            tasks: HashMap::new(),
            next_task: 0,
            ready: VecDeque::new(),
            jobs: JoinSet::new(),
            job_kinds: HashMap::new(),
            running_activations: 0,
            max_activations,
            closed_scope_errors: Vec::new(),
        }
    }

    async fn run(mut self) {
        loop {
            self.launch_ready();
            self.advance_closures();
            if self.owners[&ROOT].data.status.load(Ordering::Acquire) == CLOSED {
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
                let closing = self.owners[&ROOT].data.status.load(Ordering::Acquire) != OPEN;
                if closing {
                    data.status.store(CLOSING, Ordering::Release);
                }
                self.owners.insert(data.id, OwnerState::new(data));
            }
            Command::Resolve {
                owner,
                provider,
                waiter,
            } => {
                self.accept_resolution(owner, provider, waiter);
            }
            Command::Close { owner, waiter } => self.begin_close(owner, waiter),
        }
    }

    fn accept_resolution(&mut self, owner: OwnerId, provider: usize, waiter: ResolveWaiter) {
        let open = self
            .owners
            .get(&owner)
            .is_some_and(|owner| owner.data.status.load(Ordering::Acquire) == OPEN);
        if !open || self.owners[&ROOT].data.status.load(Ordering::Acquire) != OPEN {
            let _ = waiter.send(Err(ResolveError::closed()));
            return;
        }
        let Some(node) = self.graph.nodes.get(provider) else {
            let _ = waiter.send(Err(ResolveError::new("无效的 provider 计划编号".into())));
            return;
        };
        if owner == ROOT && node.requires_scope {
            let _ = waiter.send(Err(ResolveError::construction(
                &node.identifier,
                node.common.source,
                "此服务的依赖闭包需要 scope，不能从 root provider 获取".into(),
            )));
            return;
        }
        let mut expansion = Vec::new();
        let task = self.ensure_task(owner, provider, &mut expansion);
        match &self.tasks[&task].state {
            TaskState::Complete(result) => {
                let _ = waiter.send(result.clone());
            }
            _ => self.tasks.get_mut(&task).unwrap().waiters.push(waiter),
        }
        self.expand(&mut expansion);
    }

    fn ensure_task(
        &mut self,
        requested_owner: OwnerId,
        provider: usize,
        expansion: &mut Vec<TaskId>,
    ) -> TaskId {
        let node = &self.graph.nodes[provider];
        let owner = if node.common.lifetime == ServiceLifetime::Singleton {
            ROOT
        } else {
            requested_owner
        };
        let cached = node.common.lifetime != ServiceLifetime::Transient;
        if cached && let Some(task) = self.owners[&owner].cache.get(&provider) {
            return *task;
        }
        let task = self.next_task;
        self.next_task += 1;
        self.tasks.insert(
            task,
            Activation {
                owner,
                provider,
                state: TaskState::Waiting,
                expanded: false,
                pending: 0,
                inputs: vec![None; node.dependencies.len()],
                parents: Vec::new(),
                waiters: Vec::new(),
            },
        );
        let state = self.owners.get_mut(&owner).unwrap();
        state.pending += 1;
        state.task_ids.push(task);
        if cached {
            state.cache.insert(provider, task);
        }
        expansion.push(task);
        task
    }

    fn expand(&mut self, expansion: &mut Vec<TaskId>) {
        while let Some(task) = expansion.pop() {
            let (owner, provider) = {
                let task = &self.tasks[&task];
                (task.owner, task.provider)
            };
            let targets: Vec<_> = self.graph.nodes[provider]
                .dependencies
                .iter()
                .map(|dependency| dependency.target)
                .collect();
            let mut failure = None;
            for (index, target) in targets.into_iter().enumerate() {
                let Some(target) = target else { continue };
                debug_assert!(
                    self.graph.dependents[target]
                        .binary_search(&provider)
                        .is_ok()
                );
                let child = self.ensure_task(owner, target, expansion);
                match &self.tasks[&child].state {
                    TaskState::Complete(Ok(lease)) => {
                        let lease = lease.clone();
                        self.tasks.get_mut(&task).unwrap().inputs[index] = Some(lease);
                    }
                    TaskState::Complete(Err(error)) => {
                        failure.get_or_insert_with(|| error.clone());
                    }
                    _ => {
                        self.tasks
                            .get_mut(&child)
                            .unwrap()
                            .parents
                            .push((task, index));
                        self.tasks.get_mut(&task).unwrap().pending += 1;
                    }
                }
            }
            self.tasks.get_mut(&task).unwrap().expanded = true;
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
        let task_state = self.tasks.get_mut(&task).unwrap();
        if task_state.expanded
            && task_state.pending == 0
            && matches!(task_state.state, TaskState::Waiting)
        {
            task_state.state = TaskState::Queued;
            self.ready.push_back(task);
        }
    }

    fn settle(&mut self, task: TaskId, result: Resolution) {
        let mut completed = VecDeque::from([(task, result)]);
        while let Some((task, result)) = completed.pop_front() {
            let Some(activation) = self.tasks.get_mut(&task) else {
                continue;
            };
            if matches!(activation.state, TaskState::Complete(_)) {
                continue;
            }
            activation.state = TaskState::Complete(result.clone());
            self.owners.get_mut(&activation.owner).unwrap().pending -= 1;
            let parents = std::mem::take(&mut activation.parents);
            for waiter in std::mem::take(&mut activation.waiters) {
                let _ = waiter.send(result.clone());
            }
            for (parent, input) in parents {
                let Some(parent_state) = self.tasks.get_mut(&parent) else {
                    continue;
                };
                if matches!(parent_state.state, TaskState::Complete(_)) {
                    continue;
                }
                match &result {
                    Ok(lease) => {
                        parent_state.inputs[input] = Some(lease.clone());
                        parent_state.pending -= 1;
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
            if !matches!(activation.state, TaskState::Queued) {
                continue;
            }
            activation.state = TaskState::Running;
            let inputs = std::mem::take(&mut activation.inputs);
            let provider = activation.provider;
            let graph = self.graph.clone();
            let domain = self.domain.clone();
            let handle = self.jobs.spawn(async move {
                JobCompletion::Activation(activate(graph, provider, inputs, domain).await)
            });
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
        let kind = self.job_kinds.remove(&id).expect("every worker is tracked");
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
                    _ => unreachable!("activation jobs return activation results"),
                };
                if let Ok(lease) = &result {
                    self.owners[&activation.owner]
                        .data
                        .journal
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(Published {
                            provider: activation.provider,
                            lease: lease.clone(),
                        });
                }
                self.settle(task, result);
            }
            JobKind::Cleanup { owner, provider } => {
                let errors = match completion {
                    Ok((_, JobCompletion::Cleanup(errors))) => errors,
                    Err(error) => vec![format!(
                        "{:?} cleanup 任务终止：{error}",
                        self.graph.nodes[provider].identifier,
                    )],
                    _ => unreachable!("cleanup jobs return cleanup results"),
                };
                let owner = self.owners.get_mut(&owner).unwrap();
                owner.cleanup_running = false;
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
                let _ = waiter.send(Err(coordinator_stopped()));
            }
            return;
        };
        if let Some(waiter) = waiter {
            state.close_waiters.push(waiter);
        }
        state.data.status.store(CLOSING, Ordering::Release);
        if owner.id == ROOT {
            for state in self.owners.values() {
                state.data.status.store(CLOSING, Ordering::Release);
            }
        }
    }

    fn advance_closures(&mut self) {
        // Each owner completes one cleanup before starting the next. Reversing only launch order
        // would allow a dependency to close while a consumer's asynchronous hook is still using it.
        let mut owners: Vec<_> = self.owners.keys().copied().collect();
        // Root must be visited after the last scope is removed, even when no worker event remains
        // to wake the loop (for example, closing several empty scopes).
        owners.sort_unstable_by_key(|owner| *owner == ROOT);
        for owner in owners {
            let state = &self.owners[&owner];
            if state.data.status.load(Ordering::Acquire) != CLOSING
                || state.pending != 0
                || state.cleanup_running
                || (owner == ROOT && self.owners.len() != 1)
            {
                continue;
            }
            if !state.cleaning {
                let state = self.owners.get_mut(&owner).unwrap();
                state.cleaning = true;
                state.cache.clear();
                for task in std::mem::take(&mut state.task_ids) {
                    self.tasks.remove(&task);
                }
            }
            let entry = self.owners[&owner]
                .data
                .journal
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop();
            if let Some(entry) = entry {
                let provider = entry.provider;
                let graph = self.graph.clone();
                self.owners.get_mut(&owner).unwrap().cleanup_running = true;
                let handle = self
                    .jobs
                    .spawn(async move { JobCompletion::Cleanup(cleanup(entry, graph).await) });
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
                *state
                    .data
                    .close_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(result.clone());
                state.data.status.store(CLOSED, Ordering::Release);
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

async fn activate(
    graph: Arc<ValidatedGraph>,
    provider: usize,
    inputs: Vec<Option<DependencyLease>>,
    domain: Arc<ReleaseDomain>,
) -> Resolution {
    let node = &graph.nodes[provider];
    let convert = |error: crate::activation::ConstructionError| {
        ResolveError::construction(&node.identifier, node.common.source, error.to_string())
    };
    let mut preparation = ActivationPreparation::new(node.dependencies.len());
    for (dependency, input) in node.dependencies.iter().zip(inputs) {
        preparation
            .prepare(dependency.slot, dependency.prepare, input)
            .map_err(convert)?;
    }
    let (service, dependencies) = match node.constructor {
        Constructor::Class(constructor) => {
            let (inputs, dependencies) = preparation.finish_class().map_err(convert)?;
            (constructor(inputs).map_err(convert)?, dependencies)
        }
        Constructor::Factory(invoker) => {
            let mut frame = preparation.finish_factory().map_err(convert)?;
            let service = match invoker {
                FactoryInvoker::Sync(constructor) => {
                    constructor(frame.inputs()).map_err(convert)?
                }
                FactoryInvoker::Async(constructor) => {
                    constructor(frame.inputs()).await.map_err(convert)?
                }
            };
            (service, frame.into_dependencies())
        }
    };
    if service.service_type() != node.identifier.service_type {
        return Err(ResolveError::construction(
            &node.identifier,
            node.common.source,
            format!("构造器返回了不匹配的服务类型 {:?}", service.service_type()),
        ));
    }
    Ok(DependencyLease::new(service, dependencies, domain))
}

async fn cleanup(entry: Published, graph: Arc<ValidatedGraph>) -> Vec<String> {
    let node = &graph.nodes[entry.provider];
    let mut errors = Vec::new();
    let describe = |stage: &str, detail: String| {
        format!(
            "{:?}（{:?}）{stage}：{detail}",
            node.identifier, node.common.source,
        )
    };
    if let Some(hook) = node.common.cleanup {
        match catch_unwind(AssertUnwindSafe(hook)) {
            Err(payload) => errors.push(describe("cleanup panic", panic_message(payload.as_ref()))),
            Ok(mut future) => {
                let outcome = poll_fn(|cx| {
                    match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
                        Ok(Poll::Pending) => Poll::Pending,
                        Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
                        Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
                    }
                })
                .await;
                if let Err(detail) = outcome {
                    errors.push(describe("cleanup panic", detail));
                }
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(future))) {
                    errors.push(describe(
                        "cleanup future Drop panic",
                        panic_message(payload.as_ref()),
                    ));
                }
            }
        }
    }
    if let Some(completion) = entry.lease.release_tracked() {
        for detail in completion.await {
            errors.push(describe("service Drop panic", detail));
        }
    }
    errors
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "未提供字符串 panic 信息".to_owned())
}

#[cfg(test)]
mod tests;

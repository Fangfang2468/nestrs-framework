//! 用户 panic 的载荷仍是用户值，其 Drop 不能逃逸到协调器或跳过清理。

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use tokio::sync::{mpsc, oneshot};

use super::{graph, node};
use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, DependencyLease, ErasedService,
        adapter::{CleanupFuture, CleanupHook},
    },
    graph::Constructor,
    runtime::{
        Runtime,
        coordinator::{Coordinator, JobKind},
        owner::{CLOSED, OwnerData, OwnerPhase, ROOT},
    },
};

const TIMEOUT: Duration = Duration::from_secs(2);

struct PanicOnDrop;

impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("panic payload destructor panicked");
    }
}

fn panicking_constructor(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    std::panic::panic_any(PanicOnDrop);
}

fn healthy_constructor(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(42_u32))
}

#[tokio::test]
async fn constructor_payload_drop_panic_does_not_stop_unrelated_queries_or_close() {
    let graph = graph(vec![
        node::<u32>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(panicking_constructor),
            vec![],
        ),
        node::<u32>(
            1,
            ServiceLifetime::Singleton,
            Constructor::Class(healthy_constructor),
            vec![],
        ),
    ]);
    let (runtime, owner) = Runtime::start(graph, 1);
    let failure = tokio::time::timeout(TIMEOUT, runtime.resolve(&owner, 0))
        .await
        .expect("构造失败必须完成查询")
        .err()
        .expect("发生 panic 的构造不能成功");
    assert!(
        failure.to_string().contains("构造任务终止"),
        "必须保留构造诊断，不能因协调器退出退化为 closed：{failure}"
    );
    assert!(
        failure
            .to_string()
            .contains("panic payload destructor panicked"),
        "必须同时保留 payload 析构失败，而不是跳过其析构：{failure}"
    );
    let healthy = tokio::time::timeout(TIMEOUT, runtime.resolve(&owner, 1))
        .await
        .expect("无关服务查询不能挂起")
        .expect("无关服务仍应成功");
    // SAFETY: healthy 在整个读取期间强持有这个已核对类型的实例。
    assert_eq!(unsafe { *healthy.pointer::<u32>().unwrap().as_ref() }, 42);
    drop(healthy);
    tokio::time::timeout(TIMEOUT, runtime.close(&owner))
        .await
        .expect("构造失败后关闭不能挂起")
        .expect("未发布的失败构造不应破坏 owner 关闭");
}

fn panic_creating_cleanup() -> CleanupFuture {
    std::panic::panic_any(PanicOnDrop);
}

fn panic_polling_cleanup() -> CleanupFuture {
    Box::pin(async {
        tokio::task::yield_now().await;
        std::panic::panic_any(PanicOnDrop);
    })
}

struct PanicDroppingCleanup;

impl Future for PanicDroppingCleanup {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

impl Drop for PanicDroppingCleanup {
    fn drop(&mut self) {
        std::panic::panic_any(PanicOnDrop);
    }
}

fn panic_dropping_cleanup() -> CleanupFuture {
    Box::pin(PanicDroppingCleanup)
}

// 后续 hook 用独立诊断作为到达证据，不依赖进程级计数器或测试执行顺序。
fn later_cleanup() -> CleanupFuture {
    Box::pin(async {
        tokio::task::yield_now().await;
        panic!("later cleanup reached");
    })
}

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn already_published(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    unreachable!("关闭回归只使用测试直接发布的实例，不重新调用构造器")
}

async fn check_cleanup_failure(hook: CleanupHook, stage: &str, escaped_job: bool) {
    let mut later = node::<DropProbe>(
        0,
        ServiceLifetime::Singleton,
        Constructor::Class(already_published),
        vec![],
    );
    later.common.cleanup = Some(later_cleanup);
    let mut failing = node::<DropProbe>(
        1,
        ServiceLifetime::Singleton,
        Constructor::Class(already_published),
        vec![],
    );
    failing.common.cleanup = Some(hook);
    let (commands, receiver) = mpsc::unbounded_channel();
    let owner = OwnerData::new(ROOT, commands.downgrade());
    let mut coordinator = Coordinator::new(graph(vec![later, failing]), owner.clone(), receiver, 1);
    let drops = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    // 此回归从已发布实例开始，只测试关闭。每个实例使用本次测试独有的计数器。
    for (provider, count) in drops.iter().enumerate() {
        owner.publish(
            provider,
            DependencyLease::new(
                ErasedService::new(DropProbe(count.clone())),
                vec![],
                coordinator.domain.clone(),
            ),
        );
    }
    let (waiter, closed) = oneshot::channel();
    coordinator.begin_close(owner.clone(), Some(waiter));
    let completion = if escaped_job {
        // 模拟 cleanup worker 在自身保护之外终止，直接覆盖 JoinError 的兜底分支。
        // 被该 worker 取走的实例先正常释放，剩余 journal 仍由协调器继续清理。
        drop(owner.next_cleanup().unwrap());
        coordinator.owners.get_mut(&ROOT).unwrap().phase = OwnerPhase::Cleaning { running: true };
        let handle = coordinator.jobs.spawn(async {
            std::panic::panic_any(PanicOnDrop);
        });
        coordinator.job_kinds.insert(
            handle.id(),
            JobKind::Cleanup {
                owner: ROOT,
                provider: 1,
            },
        );
        Some(
            tokio::time::timeout(TIMEOUT, coordinator.jobs.join_next_with_id())
                .await
                .expect("发生 panic 的 cleanup worker 必须结束")
                .unwrap(),
        )
    } else {
        None
    };
    let running = tokio::spawn(async move {
        if let Some(completion) = completion {
            coordinator.handle_completion(completion);
        }
        coordinator.run().await;
    });
    tokio::time::timeout(TIMEOUT, running)
        .await
        .expect("payload 析构 panic 不能使关闭挂起")
        .expect("payload 析构 panic 不能终止协调器");
    let failure = closed
        .await
        .expect("关闭等待者必须收到结果")
        .expect_err("两个有意 panic 的 cleanup 应报告失败");
    let messages = failure.failures().join("\n");
    let first_failure = failure
        .failures()
        .iter()
        .find(|message| !message.contains("later cleanup reached"))
        .expect("必须报告第一个 cleanup 的失败");
    assert!(
        first_failure.contains(stage),
        "必须保留原 cleanup 阶段：{messages}"
    );
    assert!(
        first_failure.contains("panic payload destructor panicked"),
        "必须保留 payload 析构失败：{messages}"
    );
    assert!(
        messages.contains("later cleanup reached"),
        "失败后必须继续执行后续 hook：{messages}"
    );
    if !escaped_job {
        assert!(
            !messages.contains("cleanup 任务终止"),
            "已受保护的 hook panic 不应逃逸为 worker JoinError：{messages}"
        );
    }
    assert_eq!(owner.status.load(Ordering::Acquire), CLOSED);
    assert!(owner.journal.lock().unwrap().is_empty());
    assert!(drops.iter().all(|count| count.load(Ordering::SeqCst) == 1));
}

#[tokio::test]
async fn cleanup_creation_payload_drop_panic_preserves_remaining_cleanup_and_release() {
    check_cleanup_failure(panic_creating_cleanup, "cleanup panic", false).await;
}

#[tokio::test]
async fn cleanup_poll_payload_drop_panic_preserves_remaining_cleanup_and_release() {
    check_cleanup_failure(panic_polling_cleanup, "cleanup panic", false).await;
}

#[tokio::test]
async fn cleanup_future_drop_payload_panic_preserves_remaining_cleanup_and_release() {
    check_cleanup_failure(panic_dropping_cleanup, "cleanup future Drop panic", false).await;
}

#[tokio::test]
async fn cleanup_job_payload_drop_panic_does_not_stop_the_coordinator() {
    check_cleanup_failure(panic_creating_cleanup, "cleanup 任务终止", true).await;
}

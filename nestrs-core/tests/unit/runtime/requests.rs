//! 查询等待端的所有权回归：RAII 退订覆盖取消、完成竞争和预热中的全部请求。

use std::{
    collections::HashMap,
    future::poll_fn,
    sync::{Arc, atomic::AtomicU64},
    task::Poll,
};

use super::{Command, Runtime};
use crate::{
    InitializationMode, ResolveError, ServiceLifetime,
    activation::{ConstructionInputs, ErasedService},
    graph::NodePolicy,
    graph::{CompiledNode, Constructor, ValidatedGraph},
    runtime::owner::{Owner, OwnerData, ROOT},
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};
use tokio::sync::mpsc;

fn fixture() -> (Runtime, Arc<Owner>, mpsc::UnboundedReceiver<Command>) {
    let (commands, requests) = mpsc::unbounded_channel();
    let owner = Arc::new(Owner {
        data: OwnerData::new(ROOT, commands.downgrade()),
        commands: commands.clone(),
    });
    let nodes = (0..3)
        .map(|index| CompiledNode {
            identifier: ServiceIdentifier::new(
                Some(ServiceKey::Indexed(index)),
                ServiceType::create::<u32>(),
            ),
            common: NodePolicy {
                lifetime: if index == 2 {
                    ServiceLifetime::Scoped
                } else {
                    ServiceLifetime::Singleton
                },

                lazy: None,
                source: ServiceSource::new(file!(), line!(), 0),
                cleanup: None,
            },
            dependencies: vec![],
            constructor: Constructor::Class(|inputs: ConstructionInputs| {
                inputs.ensure_all_consumed()?;
                Ok(ErasedService::new(42_u32))
            }),
            requires_scope: index == 2,
        })
        .collect();
    let runtime = Runtime {
        graph: Arc::new(ValidatedGraph {
            nodes,
            dependents: vec![vec![], vec![], vec![]],
            topological_order: vec![0, 1, 2],
            routes: HashMap::new(),
        }),
        commands,
        next_owner: AtomicU64::new(1),
        next_query: AtomicU64::new(0),
    };
    (runtime, owner, requests)
}

#[tokio::test]
async fn dropping_query_wait_unregisters_even_when_a_result_has_already_arrived() {
    for completed in [false, true] {
        let (runtime, owner, mut requests) = fixture();
        let mut query = Box::pin(runtime.resolve(&owner, 0));
        poll_fn(|cx| {
            assert!(query.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let Command::Resolve {
            query: id, waiter, ..
        } = requests.try_recv().unwrap()
        else {
            panic!("必须先提交查询")
        };
        if completed {
            assert!(
                waiter
                    .send(Err(ResolveError::new(
                        "completed before cancellation".into(),
                    )))
                    .is_ok()
            );
        }
        drop(query);
        let Command::CancelQuery(cancelled) = requests.try_recv().unwrap() else {
            panic!("取消必须退订")
        };
        assert_eq!(id, cancelled);
        assert!(requests.try_recv().is_err());
    }
}

#[tokio::test]
async fn an_observed_result_does_not_send_a_redundant_cancellation() {
    let (runtime, owner, mut requests) = fixture();
    let mut query = Box::pin(runtime.resolve(&owner, 0));
    poll_fn(|cx| {
        assert!(query.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let Command::Resolve { waiter, .. } = requests.try_recv().unwrap() else {
        panic!("必须先提交查询")
    };
    assert!(
        waiter
            .send(Err(ResolveError::new("observed".into())))
            .is_ok()
    );
    assert!(query.await.is_err());
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn cancelling_warmup_unregisters_waited_and_not_yet_waited_requests() {
    for first_completed in [false, true] {
        let (runtime, owner, mut requests) = fixture();
        let mut warmup = Box::pin(runtime.warm_up(
            &owner,
            ServiceLifetime::Singleton,
            InitializationMode::Eager,
        ));
        poll_fn(|cx| {
            assert!(warmup.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let Command::Resolve {
            query: first,
            provider: 0,
            waiter,
            ..
        } = requests.try_recv().unwrap()
        else {
            panic!("第一个 Singleton 应已提交")
        };
        let Command::Resolve {
            query: second,
            provider: 1,
            waiter: second_waiter,
            ..
        } = requests.try_recv().unwrap()
        else {
            panic!("等待第一个结果前就应提交第二个 Singleton")
        };
        assert_ne!(first, second);
        assert!(
            requests.try_recv().is_err(),
            "Singleton 预热不提交 Scoped 根"
        );
        if first_completed {
            assert!(
                waiter
                    .send(Err(ResolveError::new("first finished".into())))
                    .is_ok()
            );
            // 失败不能跳过其他已提交初始化；现在 warmup 应等待第二个结果。
            poll_fn(|cx| {
                assert!(warmup.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        drop(warmup);
        let mut cancelled = Vec::new();
        while let Ok(command) = requests.try_recv() {
            let Command::CancelQuery(query) = command else {
                panic!("只应发送退订命令")
            };
            cancelled.push(query);
        }
        cancelled.sort_unstable();
        assert_eq!(
            cancelled,
            if first_completed {
                vec![second]
            } else {
                vec![first, second]
            }
        );
        assert!(second_waiter.is_closed());
    }
}

#[test]
fn a_submission_error_releases_previously_submitted_request_guards() {
    let (runtime, owner, mut commands) = fixture();
    // 预热提交阶段没有 await；用同一 request API 确定性触发 owner 在两次提交间关闭，
    // 验证 ? 提前返回时，已经进入集合但尚未 wait 的凭证也会退订。
    let submitted = (|| -> Result<(), ResolveError> {
        let mut requests = vec![runtime.request_resolution(&owner, 0)?];
        owner.data.mark_closing();
        requests.push(runtime.request_resolution(&owner, 1)?);
        Ok(())
    })();
    assert!(submitted.is_err());
    let Command::Resolve { query, waiter, .. } = commands.try_recv().unwrap() else {
        panic!("第一个查询必须已提交")
    };
    let Command::CancelQuery(cancelled) = commands.try_recv().unwrap() else {
        panic!("错误返回必须退订先前请求")
    };
    assert_eq!(query, cancelled);
    assert!(waiter.is_closed());
    assert!(commands.try_recv().is_err());
}

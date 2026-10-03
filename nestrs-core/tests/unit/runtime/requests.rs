//! 创建与查询等待端的所有权回归：登记确认、取消关闭以及全部初始化请求的 RAII 退订。

use ahash::AHashMap;
use std::{
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

#[tokio::test]
async fn lazy_scope_waits_for_registration_before_delivery() {
    let (runtime, _root, mut requests) = fixture();
    let mut creating = Box::pin(runtime.create_scope(InitializationMode::Lazy));
    poll_fn(|cx| {
        assert!(creating.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let Command::Register { data, ready } = requests.try_recv().unwrap() else {
        panic!("必须先登记 scope")
    };
    assert_ne!(data.id, ROOT);
    assert!(requests.try_recv().is_err());
    poll_fn(|cx| {
        assert!(creating.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    ready.send(Ok(())).unwrap();
    let scope = creating.await.unwrap();
    assert!(Arc::ptr_eq(&scope.data, &data));
    assert!(
        requests.try_recv().is_err(),
        "Lazy 创建不提交普通 Scoped 初始化"
    );
    drop(scope);
    let Command::Close {
        owner,
        waiter: None,
    } = requests.try_recv().unwrap()
    else {
        panic!("交付后的 scope 仍使用相同关闭协议")
    };
    assert!(Arc::ptr_eq(&owner, &data));
}

#[tokio::test]
async fn cancelling_scope_registration_closes_the_undelivered_owner() {
    // 同时覆盖协调器尚未确认与已经确认、创建 future 尚未恢复这两个取消窗口。
    for registered in [false, true] {
        let (runtime, _root, mut requests) = fixture();
        let mut creating = Box::pin(runtime.create_scope(InitializationMode::Eager));
        poll_fn(|cx| {
            assert!(creating.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        let Command::Register { data, ready } = requests.try_recv().unwrap() else {
            panic!("必须先登记 scope")
        };
        if registered {
            ready.send(Ok(())).unwrap();
        }
        drop(creating);
        let Command::Close {
            owner,
            waiter: None,
        } = requests.try_recv().unwrap()
        else {
            panic!("取消登记等待也必须关闭未交付的 scope")
        };
        assert!(Arc::ptr_eq(&owner, &data));
        assert!(requests.try_recv().is_err(), "登记完成前不提交初始化请求");
    }
}

#[tokio::test]
async fn closed_root_rejects_lazy_scope_creation_without_hanging() {
    let (fixture, _root, _requests) = fixture();
    let (runtime, root) = Runtime::start(fixture.graph, 1, InitializationMode::Lazy)
        .await
        .unwrap();
    runtime.close(&root).await.unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        runtime.create_scope(InitializationMode::Lazy),
    )
    .await
    .expect("已关闭 root 必须及时拒绝新 scope");
    assert!(result.is_err());
}

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
            routes: AHashMap::new(),
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
async fn cancelling_creation_unregisters_queries_and_closes_the_undelivered_owner() {
    for first_completed in [false, true] {
        let (runtime, owner, mut requests) = fixture();
        let mut creating = Box::pin(runtime.initialize_owner(
            owner,
            ServiceLifetime::Singleton,
            InitializationMode::Eager,
        ));
        poll_fn(|cx| {
            assert!(creating.as_mut().poll(cx).is_pending());
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
            // 失败不能跳过其他已提交初始化；现在创建过程应等待第二个结果。
            poll_fn(|cx| {
                assert!(creating.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        drop(creating);
        let mut cancelled = Vec::new();
        let mut closed = Vec::new();
        while let Ok(command) = requests.try_recv() {
            match command {
                Command::CancelQuery(query) => cancelled.push(query),
                Command::Close {
                    owner,
                    waiter: None,
                } => closed.push(owner.id),
                _ => panic!("取消创建应退订请求并关闭未交付 owner"),
            }
        }
        assert_eq!(closed, [ROOT]);
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

//! 高级示例：Tokio 异步激活、静态多层 Scope、factory、trait、key、optional、
//! 泛型 provider 与显式 cleanup。
//!
//! 日常应用从 `src/main.rs` 的 `ServiceProvider` 同步入口开始即可；本示例刻意集中展示
//! 需要显式选择的高级能力。

use std::marker::PhantomData;

use nestrs_core::scope::{ScopeLayer, ScopeProvider};
use nestrs_macro::{bind, factory, injectable};

#[derive(Debug, Clone)]
struct User {
    id: u32,
    name: &'static str,
}

#[injectable]
struct Repository<Entity> {
    _marker: PhantomData<Entity>,
}

impl Repository<User> {
    fn find_all(&self) -> Vec<User> {
        vec![
            User {
                id: 1,
                name: "Ada Lovelace",
            },
            User {
                id: 2,
                name: "Grace Hopper",
            },
        ]
    }
}

trait UserCollection: Send + Sync {
    fn get_users(&self) -> Vec<User>;
}

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}

impl UserService {
    fn total_users(&self) -> usize {
        self.repository.find_all().len()
    }
}

#[bind]
impl UserCollection for UserService {
    fn get_users(&self) -> Vec<User> {
        self.repository.find_all()
    }
}

/// 一个 keyed provider；在 consumer 端也明确写出相同的 key。
#[injectable(key = "audit")]
struct AuditTrail;

impl AuditTrail {
    fn record(&self, event: &str) {
        println!("audit: {event}");
    }
}

struct StartupSummary {
    user_count: usize,
    has_audit: bool,
}

async fn cleanup_startup_summary() {
    println!("cleanup singleton startup summary");
}

/// factory 参数同样来自编译好的依赖图；它不是运行时 resolver。
#[factory(cleanup = "cleanup_startup_summary")]
fn startup_summary(
    #[inject] users: UserService,
    #[inject(key = "audit")] audit: Option<AuditTrail>,
) -> StartupSummary {
    if let Some(audit) = audit.as_ref() {
        audit.record("creating startup summary");
    }

    StartupSummary {
        user_count: users.total_users(),
        has_audit: audit.is_some(),
    }
}

async fn cleanup_application() {
    println!("cleanup application");
}

#[injectable(cleanup = "cleanup_application")]
struct ApplicationRoot {
    #[inject]
    users: UserService,
    #[inject]
    startup: StartupSummary,
}

async fn cleanup_request() {
    println!("cleanup request");
}

/// 链头 ScopeRoot：每次 request scope 都有独立的实例。
#[injectable(lifetime = "scoped", cleanup = "cleanup_request")]
struct UserController {
    #[inject]
    users: UserService,
    #[inject]
    collection: dyn UserCollection,
    #[inject(key = "audit")]
    audit: Option<AuditTrail>,
}

impl UserController {
    fn list_users(&self) -> Vec<User> {
        if let Some(audit) = self.audit.as_ref() {
            audit.record("listing users");
        }

        self.collection.get_users()
    }
}

/// child scope 可以读取精确祖先 request scope 的服务。
#[injectable(lifetime = "scoped")]
struct TransactionContext {
    #[inject]
    request: UserController,
}

/// 最内层 scope 同时读取两个祖先层的服务。
#[injectable(lifetime = "scoped")]
struct CommandContext {
    #[inject]
    request: UserController,
    #[inject]
    transaction: TransactionContext,
}

type ApplicationScopes =
    ScopeLayer<UserController, ScopeLayer<TransactionContext, ScopeLayer<CommandContext>>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()?
        .block_on(async {
            let provider =
                ScopeProvider::<ApplicationRoot, ApplicationScopes>::build_async().await?;
            assert_eq!(provider.root().users.total_users(), 2);
            assert_eq!(provider.root().startup.user_count, 2);
            assert!(provider.root().startup.has_audit);

            let request = provider.create_scope_async().await?;
            let users = request.root().list_users();
            assert_eq!(users.len(), 2);
            assert_eq!(users[0].id, 1);
            assert_eq!(users[1].name, "Grace Hopper");

            let transaction = request.create_child_scope_async().await?;
            let command = transaction.create_child_scope_async().await?;
            assert_eq!(command.root().request.users.total_users(), 2);
            assert_eq!(command.root().transaction.request.list_users().len(), 2);

            // cleanup 只由显式、消费式 shutdown 驱动，且必须由内向外结束。
            command.shutdown().await;
            transaction.shutdown().await;
            request.shutdown().await;
            provider.shutdown().await;
            Ok(())
        })
}

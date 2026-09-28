//! Kept in a real external module to catch frontends that only lower the crate root.
use nestrs::factory;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
pub(super) static DROPS: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Database {
    name: String,
}

impl Drop for Database {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) type DatabaseAlias = Database;

#[factory]
fn database() -> Database {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    Database {
        name: "orders-primary".to_owned(),
    }
}

struct Missing;

pub(super) struct SyncSummary {
    pub(super) text: String,
}

#[factory]
fn sync_summary(
    database: DatabaseAlias,
    present: Option<DatabaseAlias>,
    missing: Option<Missing>,
) -> SyncSummary {
    let borrowed: &Database = database;
    assert!(std::ptr::eq(borrowed, present.unwrap()));
    assert!(missing.is_none());
    SyncSummary {
        text: format!("{}:sync", borrowed.name),
    }
}

pub(super) struct AsyncSummary {
    pub(super) text: String,
}

#[factory]
async fn async_summary(
    #[nestrs::inject] database: DatabaseAlias,
    present: Option<DatabaseAlias>,
    missing: Option<Missing>,
) -> AsyncSummary {
    let borrowed = database.name.as_str();
    let optional_borrow = present.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    assert!(std::ptr::eq(database, optional_borrow));
    assert!(missing.is_none());
    AsyncSummary {
        text: format!("{borrowed}:async"),
    }
}

pub(super) struct FutureSummary {
    pub(super) text: String,
}

#[factory]
fn future_summary(database: DatabaseAlias) -> impl Future<Output = FutureSummary> {
    let borrowed = database.name.as_str();
    async move {
        tokio::task::yield_now().await;
        assert_eq!(DROPS.load(Ordering::SeqCst), 0);
        FutureSummary {
            text: format!("{borrowed}:future"),
        }
    }
}

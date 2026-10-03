use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::Injection;
use crate::activation::{
    DependencyLease, ErasedService, InputSlot, ProjectionTarget, ReleaseDomain, project_required,
};

struct Service;
trait Port: Send + Sync {}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn injection_is_send_and_sync_for_an_injectable_service() {
    assert_send_sync::<Injection<Service>>();
    assert_send_sync::<Injection<dyn Port>>();
}

#[test]
fn a_token_moved_out_by_consumer_drop_keeps_the_dependency_alive() {
    struct Dependency {
        value: u32,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Dependency {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Consumer {
        dependency: Option<Injection<Dependency>>,
        escaped: Arc<Mutex<Option<Injection<Dependency>>>>,
    }
    impl Drop for Consumer {
        fn drop(&mut self) {
            *self.escaped.lock().unwrap() = self.dependency.take();
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let escaped = Arc::new(Mutex::new(None));
    let domain = ReleaseDomain::new();
    let dependency = DependencyLease::new(
        ErasedService::new(Dependency {
            value: 42,
            drops: drops.clone(),
        }),
        vec![],
        domain.clone(),
    );
    let token = ProjectionTarget::project::<Dependency>(
        InputSlot::new(0),
        dependency.clone(),
        project_required::<Dependency>,
    )
    .unwrap();
    let consumer = DependencyLease::new(
        ErasedService::new(Consumer {
            dependency: Some(token),
            escaped: escaped.clone(),
        }),
        vec![dependency],
        domain,
    );
    drop(consumer);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let token = escaped.lock().unwrap().take().unwrap();
    assert_eq!(token.value, 42);
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

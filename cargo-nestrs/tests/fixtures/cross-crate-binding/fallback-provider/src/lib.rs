//! A sibling crate with the same local concrete name, but a different Rust type.

use std::sync::atomic::{AtomicUsize, Ordering};

static CONFLICT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

mod implementation {
    use contracts::{AmbiguousPort, DeliveryPort, DormantPort, TrackingPort};
    use nestrs::{factory, injectable};

    #[injectable]
    pub struct Service {
        #[value(99)]
        id: usize,
    }

    impl Service {
        pub fn id(&self) -> usize {
            self.id
        }
    }

    impl DeliveryPort for Service {
        fn source(&self) -> &'static str {
            "fallback"
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    #[injectable]
    struct Tracking {
        #[value(101)]
        _id: usize,
    }

    impl TrackingPort for Tracking {
        fn carrier(&self) -> &'static str {
            "fallback-tracker"
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    #[injectable]
    struct Unrequested;

    impl DormantPort for Unrequested {}

    struct Conflict;

    impl AmbiguousPort for Conflict {}

    #[factory(key = "conflict")]
    fn conflict() -> Conflict {
        super::CONFLICT_CONSTRUCTIONS.fetch_add(1, super::Ordering::SeqCst);
        Conflict
    }
}

pub use implementation::Service;

pub fn conflict_constructions() -> usize {
    CONFLICT_CONSTRUCTIONS.load(Ordering::SeqCst)
}

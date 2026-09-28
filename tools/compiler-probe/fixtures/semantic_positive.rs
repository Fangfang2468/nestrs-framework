//! Semantic probe input; projection functions are deliberately handwritten.
//! They establish what rustc checks, not that Nestrs can generate them yet.
#![allow(dead_code)]

trait Store: Send + Sync {
    fn count(&self) -> usize;
}

trait Entity: Send + Sync {}

mod private {
    use super::{Entity, Store};
    use std::marker::PhantomData;

    struct PrivateStore;

    impl Store for PrivateStore {
        fn count(&self) -> usize {
            1
        }
    }

    fn project_private(value: &PrivateStore) -> &dyn Store {
        value
    }

    struct MacroStore;

    macro_rules! implement_store {
        ($service:ty) => {
            impl Store for $service {
                fn count(&self) -> usize {
                    2
                }
            }
        };
    }

    implement_store!(MacroStore);

    fn project_macro(value: &MacroStore) -> &dyn Store {
        value
    }

    struct User;

    impl Entity for User {}

    struct Repository<T>(PhantomData<T>);

    impl<T: Entity> Store for Repository<T> {
        fn count(&self) -> usize {
            3
        }
    }

    type KnownRepository = Repository<User>;
    type RepositoryAlias<T> = Repository<T>;

    trait Holder {
        type Service;
    }

    struct Choices;

    impl Holder for Choices {
        type Service = Repository<User>;
    }

    type ProjectedRepository = <Choices as Holder>::Service;

    fn project_closed_generic(value: &KnownRepository) -> &dyn Store {
        value
    }

    fn project_associated_type(value: &ProjectedRepository) -> &dyn Store {
        value
    }

    #[cfg(feature = "enabled")]
    struct EnabledStore;

    #[cfg(feature = "enabled")]
    impl Store for EnabledStore {
        fn count(&self) -> usize {
            4
        }
    }

    #[cfg(not(feature = "enabled"))]
    struct DisabledStore;

    #[cfg(not(feature = "enabled"))]
    impl Store for DisabledStore {
        fn count(&self) -> usize {
            5
        }
    }
}

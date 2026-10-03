//! Raw names are source identifiers, including associated type bindings.
#![allow(non_camel_case_types)]

use nestrs_core::ServiceProvider;

mod r#mod {
    use super::ServiceProvider;

    trait r#trait<T = u16, const N: usize = 3>: Send + Sync {
        type r#type;
        type r#async;
        type r#gen;
        fn value(&self) -> Self::r#type;
        fn paired(&self) -> Self::r#async;
        fn marker(&self) -> Self::r#gen;
    }
    use self::r#trait as Renamed;

    #[nestrs::injectable]
    struct r#struct<const N: usize> {
        #[value([47; N])]
        value: [u8; N],
    }
    impl<const N: usize> r#trait<u16, N> for r#struct<N> {
        type r#type = [u8; N];
        type r#async = (u16, [u8; N]);
        type r#gen = bool;
        fn value(&self) -> Self::r#type {
            self.value
        }
        fn paired(&self) -> Self::r#async {
            (N as u16, self.value)
        }
        fn marker(&self) -> Self::r#gen {
            true
        }
    }

    type Three = r#struct<3>;
    type Five = r#struct<5>;
    type Interface = dyn Renamed<r#type = [u8; 3], r#async = (u16, [u8; 3]), r#gen = bool>;
    type Other = dyn Renamed<u16, 5, r#type = [u8; 5], r#async = (u16, [u8; 5]), r#gen = bool>;

    // Equalities inherited from a principal trait must not be printed twice.
    trait r#enum: Renamed<r#type = [u8; 3], r#async = (u16, [u8; 3]), r#gen = bool> {}
    impl r#enum for Three {}

    trait Borrowed<'a> {
        type r#await;
        fn borrowed(&'a self) -> Self::r#await;
    }
    impl<'a, const N: usize> Borrowed<'a> for r#struct<N> {
        type r#await = &'a [u8; N];
        fn borrowed(&'a self) -> Self::r#await {
            &self.value
        }
    }
    // The raw equality belongs to the supertrait's binder, not the dyn type.
    trait BorrowedPort: for<'a> Borrowed<'a, r#await = &'a [u8; 3]> + Send + Sync {}
    impl BorrowedPort for Three {}

    #[nestrs::injectable]
    struct Consumer {
        #[inject]
        direct: Interface,
        #[inject]
        other: Other,
        #[inject]
        inherited: dyn r#enum,
        #[inject]
        borrowed: dyn BorrowedPort,
    }

    pub async fn verify(root: &ServiceProvider) {
        let three = root.get_required_service::<Three>().await.unwrap();
        let five = root.get_required_service::<Five>().await.unwrap();
        let bound = root.get_required_service::<Interface>().await.unwrap();
        let other = root.get_required_service::<Other>().await.unwrap();
        let inherited = root.get_required_service::<dyn r#enum>().await.unwrap();
        let borrowed = root
            .get_required_service::<dyn BorrowedPort>()
            .await
            .unwrap();
        let consumer = root.get_required_service::<Consumer>().await.unwrap();
        assert_eq!(bound.value(), [47; 3]);
        assert_eq!(bound.paired(), (3, [47; 3]));
        assert!(bound.marker());
        assert_eq!(other.value(), [47; 5]);
        assert_eq!(other.paired(), (5, [47; 5]));
        assert_eq!(inherited.value(), [47; 3]);
        assert_eq!(borrowed.borrowed(), &[47; 3]);
        assert_eq!(consumer.direct.value(), [47; 3]);
        assert_eq!(consumer.other.value(), [47; 5]);
        assert_eq!(consumer.inherited.value(), [47; 3]);
        assert_eq!(consumer.borrowed.borrowed(), &[47; 3]);
        for projected in [
            bound as *const Interface as *const (),
            inherited as *const dyn r#enum as *const (),
            borrowed as *const dyn BorrowedPort as *const (),
            &*consumer.direct as *const Interface as *const (),
            &*consumer.inherited as *const dyn r#enum as *const (),
            &*consumer.borrowed as *const dyn BorrowedPort as *const (),
        ] {
            assert_eq!(three as *const Three as *const (), projected);
        }
        assert!(std::ptr::addr_eq(five, other));
        assert!(std::ptr::addr_eq(five, &*consumer.other));
    }
}

// A normal associated name exercises the unchanged diagnostic-printer path.
mod ordinary {
    use super::ServiceProvider;
    trait Port: Send + Sync {
        type Output;
        fn value(&self) -> Self::Output;
    }
    #[nestrs::injectable]
    struct Service;
    impl Port for Service {
        type Output = usize;
        fn value(&self) -> usize {
            53
        }
    }
    pub async fn verify(root: &ServiceProvider) {
        let concrete = root.get_required_service::<Service>().await.unwrap();
        let bound = root
            .get_required_service::<dyn Port<Output = usize>>()
            .await
            .unwrap();
        assert_eq!(bound.value(), 53);
        assert!(std::ptr::addr_eq(concrete, bound));
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = ServiceProvider::build(None).await.unwrap();
    r#mod::verify(&root).await;
    ordinary::verify(&root).await;
    root.dispose_async().await.unwrap();
    println!("raw associated type source paths passed");
}

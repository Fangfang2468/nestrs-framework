//! 括号只改变类型语法，不改变 constructor 槽位、可选性或真实实例 lease。
#![allow(unused_parens)]

use nestrs::{constructor, injectable};
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Dependency {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    sequence: usize,
}

struct Missing<T>(PhantomData<T>);

#[injectable]
struct LazyOnly<T: Send + Sync + 'static> {
    present: (((::std::option::Option<(T)>))),
    absent: ((::core::option::Option<((Missing<T>))>)),
}

impl<T: Send + Sync + 'static> LazyOnly<T> {
    #[constructor]
    fn new(
        #[lazy] present: ((Option<T>)),
        #[lazy] absent: (::std::option::Option<Missing<T>>),
    ) -> Self {
        Self { present, absent }
    }
}

#[injectable]
struct Matrix<T: Send + Sync + 'static> {
    plain: Option<T>,
    plain_absent: Option<Missing<T>>,
    lazy: Option<T>,
    lazy_absent: Option<Missing<T>>,
    nested: (((Option<((T))>))),
    nested_absent: ((::core::option::Option<Missing<T>>)),
    nested_lazy: ((::std::option::Option<(T)>)),
    nested_lazy_absent: (((Option<Missing<T>>))),
    required: ((T)),
}

impl<T: Send + Sync + 'static> Matrix<T> {
    #[constructor]
    fn new(
        plain: ((Option<T>)),
        plain_absent: (::core::option::Option<Missing<T>>),
        #[lazy] lazy: (Option<T>),
        #[lazy] lazy_absent: ((Option<Missing<T>>)),
        nested: Option<T>,
        nested_absent: ((Option<Missing<T>>)),
        #[lazy] nested_lazy: (::core::option::Option<T>),
        #[lazy] nested_lazy_absent: ::std::option::Option<Missing<T>>,
        required: (T),
    ) -> Self {
        Self {
            plain,
            plain_absent,
            lazy,
            lazy_absent,
            nested,
            nested_absent,
            nested_lazy,
            nested_lazy_absent,
            required,
        }
    }
}

#[tokio::main]
async fn main() {
    let root = ServiceProvider::build(None).await.unwrap();
    let delayed = root
        .get_required_service::<LazyOnly<Dependency>>()
        .await
        .unwrap();
    assert!(delayed.absent.is_none());
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    let delayed_value = delayed.present.as_ref().unwrap().get().await.unwrap();
    assert_eq!(delayed_value.sequence, 0);
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);

    let matrix = root
        .get_required_service::<Matrix<Dependency>>()
        .await
        .unwrap();
    assert!(matrix.plain_absent.is_none());
    assert!(matrix.lazy_absent.is_none());
    assert!(matrix.nested_absent.is_none());
    assert!(matrix.nested_lazy_absent.is_none());
    let plain = matrix.plain.as_ref().unwrap();
    let nested = matrix.nested.as_ref().unwrap();
    let lazy = matrix.lazy.as_ref().unwrap().get().await.unwrap();
    let nested_lazy = matrix.nested_lazy.as_ref().unwrap().get().await.unwrap();
    for value in [
        &**plain,
        &**nested,
        &*lazy,
        &*nested_lazy,
        &*matrix.required,
    ] {
        assert!(std::ptr::eq(value, &*delayed_value));
    }
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    root.dispose_async().await.unwrap();
    println!("constructor parenthesized optional contracts passed");
}

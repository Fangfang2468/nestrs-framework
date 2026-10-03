#![allow(dead_code)]
use implicit_library::{self as library, InvalidDrop, OnDrop, Outer, Read, Write};
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use std::mem::ManuallyDrop;

struct Coerce;
struct MultiCoerce;
struct MethodReceiver;
struct MutableCoerce;
struct MutableMethod;
struct MultiMutable;
struct LocalDrop;
struct ExplicitDrop;
struct GenericDrop;
struct ArgumentDrop;
struct OptionalDrop;
struct VectorDrop;
struct BoxedDrop;
struct ArrayDrop;
struct TupleLeft;
struct TupleRight;
struct EnumDrop;
struct ClosureDrop;
struct FutureDrop;
struct RecursiveDrop;
struct DeadDeref;
struct DeadDrop;
struct SuppressedManual;
struct SuppressedForget;
struct SuppressedReference;
#[cfg(feature = "extra-root")]
struct FeatureDrop;

enum Aggregate<'a> {
    Value(OnDrop<'a, EnumDrop>),
    Empty,
}

// 粗筛只能遍历有限名义类型图，不能物化这个与查询无关的增长类型族。
struct Growing<T> {
    next: Option<Box<Growing<Vec<T>>>>,
    marker: std::marker::PhantomData<T>,
}
fn unrelated_growth<T>() {
    std::hint::black_box(std::marker::PhantomData::<T>);
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if false {
        unrelated_growth::<Growing<u8>>();
    }
    assert_eq!(library::builds(), 0);
    let provider = ServiceProvider::build(Some(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    }))
    .await
    .unwrap();
    // 六条deref路径、十八种drop类型（含不透明与关联字段）、两端各两个死分支。
    // feature额外加入上游deref与下游drop，cfg排除时不能贡献根。
    assert_eq!(
        library::builds(),
        28 + 2 * usize::from(cfg!(feature = "extra-root"))
    );
    assert_eq!(library::queries(), 0);

    let read = Read::<Coerce>::new(&provider);
    let value: &usize = &read;
    assert_eq!(*value, 17);
    let outer = Outer::<MultiCoerce>::new(&provider);
    let value: &usize = &outer;
    assert_eq!(*value, 17);
    assert_eq!(Read::<MethodReceiver>::new(&provider).saturating_add(1), 18);
    let mut write = Write::<MutableCoerce>::new(&provider);
    let value: &mut usize = &mut write;
    *value = 3;
    Write::<MutableMethod>::new(&provider).clone_from(&7);
    let mut outer = library::OuterMut::<MultiMutable>::new(&provider);
    let value: &mut usize = &mut outer;
    *value = 9;
    {
        let _value = OnDrop::<LocalDrop>::new(&provider);
    }
    drop(OnDrop::<ExplicitDrop>::new(&provider));
    library::generic_drop(OnDrop::<GenericDrop>::new(&provider));
    library::argument_drop(OnDrop::<ArgumentDrop>::new(&provider));
    drop(Some(OnDrop::<OptionalDrop>::new(&provider)));
    drop(vec![OnDrop::<VectorDrop>::new(&provider)]);
    drop(Box::new(OnDrop::<BoxedDrop>::new(&provider)));
    drop([
        OnDrop::<ArrayDrop>::new(&provider),
        OnDrop::<ArrayDrop>::new(&provider),
    ]);
    drop((
        OnDrop::<TupleLeft>::new(&provider),
        OnDrop::<TupleRight>::new(&provider),
    ));
    drop(Aggregate::Value(OnDrop::new(&provider)));
    drop(library::opaque(&provider));
    drop(library::associated_pair(&provider));
    drop(library::fixed_associated(&provider));
    let captured = OnDrop::<ClosureDrop>::new(&provider);
    drop(move || {
        std::hint::black_box(&captured);
    });
    let captured = OnDrop::<FutureDrop>::new(&provider);
    drop(async move {
        std::hint::black_box(&captured);
    });
    drop(library::Recursive::<RecursiveDrop>::new(&provider, 3));

    // 这些类型的Drop会要求非法图；抑制析构或只持有引用不能激活它。
    {
        let held = ManuallyDrop::new(InvalidDrop::<SuppressedManual>::new(&provider));
        std::mem::forget(InvalidDrop::<SuppressedForget>::new(&provider));
        let _reference: Option<&InvalidDrop<'_, SuppressedReference>> = None;
        std::hint::black_box(&held);
    }
    if false {
        let read = Read::<DeadDeref>::new(&provider);
        let _: &usize = &read;
        let _value = OnDrop::<DeadDrop>::new(&provider);
    }
    #[cfg(feature = "extra-root")]
    if false {
        let _value = OnDrop::<FeatureDrop>::new(&provider);
    }
    #[cfg(feature = "invalid-deref")]
    {
        let invalid = library::InvalidDeref::<u32>::new(&provider);
        let _: &usize = &invalid;
    }
    #[cfg(feature = "invalid-drop")]
    drop(Some(Box::new(InvalidDrop::<u64>::new(&provider))));
    assert_eq!(
        library::builds(),
        28 + 2 * usize::from(cfg!(feature = "extra-root"))
    );
    #[cfg(feature = "invalid-associated")]
    drop(library::invalid_associated(&provider));
    assert_eq!(
        library::queries(),
        28,
        "6 deref calls plus 22 destructor calls including repeated array/recursive instances"
    );
    provider.dispose_async().await.unwrap();
}

#![allow(dead_code)]
use dynamic_library::{self as library, Run, RunMut, Runner};
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use std::{pin::Pin, rc::Rc, sync::Arc};
struct Boxed;
struct Shared;
struct Mutable;
struct Atomic;
struct Counted;
struct Pinned;
struct Cast;
struct TailTag;
struct Generic;
struct Super;
struct Dead;
struct LibraryDead;
struct Dormant;
struct Feature;
struct ShapeGood;
struct ShapeBad;
struct ShapeSend;
struct Tail<T: ?Sized> {
    prefix: usize,
    value: T,
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(
        library::builds(),
        14 + usize::from(cfg!(feature = "extra-root"))
    );
    assert_eq!(library::queries(), 0);
    let runner: Box<dyn Run> = Box::new(Runner::<Boxed>::new());
    runner.run(&provider).await;
    let value = Runner::<Shared>::new();
    let runner: &dyn Run = &value;
    library::invoke(runner, &provider).await;
    let mut value = Runner::<Mutable>::new();
    let runner: &mut dyn RunMut = &mut value;
    runner.run_mut(&provider).await;
    let runner: Arc<dyn Run> = Arc::new(Runner::<Atomic>::new());
    runner.run(&provider).await;
    let runner: Rc<dyn Run> = Rc::new(Runner::<Counted>::new());
    runner.run(&provider).await;
    let runner: Pin<Box<dyn Run>> = Box::pin(Runner::<Pinned>::new());
    runner.run(&provider).await;
    let value = Runner::<Cast>::new();
    (&value as &dyn Run).run(&provider).await;
    let value = Tail {
        prefix: 0,
        value: Runner::<TailTag>::new(),
    };
    let runner: &Tail<dyn Run> = &value;
    runner.value.run(&provider).await;
    library::erase(Runner::<Generic>::new())
        .run(&provider)
        .await;
    let runner: Arc<dyn library::Base<Tag = Super> + Send + Sync> = library::erase_child::<Super>();
    runner.execute(&provider).await;
    if false {
        let runner: Box<dyn Run> = Box::new(Runner::<Dead>::new());
        runner.run(&provider).await;
        library::dead::<LibraryDead>(&provider);
    }
    #[cfg(feature = "extra-root")]
    if false {
        library::erase(Runner::<Feature>::new())
            .run(&provider)
            .await;
    }
    // Only erased; no Dormant virtual call may seed its invalid method.
    let _dormant: Box<dyn library::Dormant> = Box::new(Runner::<Dormant>::new());
    #[cfg(feature = "invalid-dynamic")]
    if false {
        runner_bad(&provider).await;
    }
    #[cfg(feature = "invalid-default")]
    if false {
        let runner: Box<dyn library::InvalidDefault<Tag = u16>> = Box::new(Runner::<u16>::new());
        runner.invalid_default(&provider).await;
    }
    let selected: Box<dyn library::Select<Tag = ShapeGood>> =
        Box::new(library::Selected::<ShapeGood>::new());
    let _rejected: Box<dyn library::Select<Tag = ShapeBad>> =
        Box::new(library::Rejected::<ShapeBad>::new());
    selected.select(&provider).await;
    let selected: Box<dyn library::Select<Tag = ShapeSend> + Send> =
        Box::new(library::Selected::<ShapeSend>::new());
    let _rejected: Box<dyn library::Select<Tag = ShapeSend>> =
        Box::new(library::Rejected::<ShapeSend>::new());
    selected.select(&provider).await;
    assert_eq!(library::queries(), 12);
    provider.dispose_async().await.unwrap();
}
#[cfg(feature = "invalid-dynamic")]
async fn runner_bad(provider: &ServiceProvider) {
    let runner: Box<dyn library::InvalidRun> = Box::new(Runner::<u8>::new());
    runner.fail(provider).await;
}

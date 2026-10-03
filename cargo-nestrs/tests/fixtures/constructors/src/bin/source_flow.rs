//! 真实名称解析后的来源分析：cfg、宏卫生、遮蔽和控制流均不得按名字猜测。
#![allow(dead_code)]
use nestrs::{constructor, injectable};
use nestrs_core::{Injection, ServiceProvider};

#[injectable]
struct Dependency;

#[injectable]
struct Conditional {
    #[cfg(feature = "alternate")]
    dependency: Dependency,
    #[cfg_attr(not(feature = "alternate"), cfg(any()))]
    another: Dependency,
    value: usize,
}
impl Conditional {
    #[constructor]
    fn new(dependency: Dependency, another: Dependency) -> Self {
        let _ = &dependency;
        let _ = &another;
        Self {
            #[cfg(feature = "alternate")]
            dependency,
            #[cfg_attr(not(feature = "alternate"), cfg(any()))]
            another,
            value: 7,
        }
    }
}

#[injectable]
struct CapturedParameter {
    dependency: Dependency,
}
macro_rules! captured_parameter {
    ($argument:ident) => {
        #[constructor]
        fn new($argument: Dependency) -> Self {
            // 定义端的局部变量不能遮蔽调用端捕获的参数，虽然拼写都是 dependency。
            let dependency = 11usize;
            let _ = dependency;
            Self {
                dependency: $argument,
            }
        }
    };
}
impl CapturedParameter {
    captured_parameter!(dependency);
}

#[injectable]
struct DefinedParameter {
    dependency: Dependency,
}
macro_rules! defined_parameter {
    ($local:ident) => {
        #[constructor]
        fn new(dependency: Dependency) -> Self {
            // 反向验证：调用端捕获的局部变量不覆盖定义端参数。
            let $local = 12usize;
            let _ = $local;
            Self { dependency }
        }
    };
}
impl DefinedParameter {
    defined_parameter!(dependency);
}

#[injectable]
struct Shadowed {
    dependency: usize,
    other: usize,
}
impl Shadowed {
    #[constructor]
    fn new(dependency: Dependency) -> Self {
        let _ = &dependency;
        let dependency = 42usize;
        let (dependency, other) = (dependency, 43usize);
        Self { dependency, other }
    }
}

#[injectable]
struct Unit;
impl Unit {
    #[constructor]
    fn new(dependency: Dependency) -> Self {
        let _ = dependency;
        Self
    }
}

#[injectable]
struct Renamed {
    second: Dependency,
    first: Dependency,
    value: usize,
}
impl Renamed {
    #[constructor]
    fn new(first: Dependency, second: Dependency, unused: Dependency) -> Self {
        let renamed = first;
        let actual: Injection<Dependency> = renamed;
        let _ = unused;
        Self {
            second: actual,
            first: second,
            value: 1,
        }
    }
}

#[injectable]
struct Branches {
    dependency: Dependency,
}
impl Branches {
    #[constructor]
    fn new(dependency: Dependency) -> Result<Self, &'static str> {
        if false {
            return Err("early failure");
        }
        match 1usize {
            0 => return Err("failure"),
            1 => {
                let alias = dependency;
                if true {
                    Ok(Self { dependency: alias })
                } else {
                    let second = alias;
                    Ok(Self { dependency: second })
                }
            }
            _ => {
                let result = Self { dependency };
                Ok(result)
            }
        }
    }
}

#[injectable]
struct EarlyReturn {
    dependency: Dependency,
}
impl EarlyReturn {
    #[constructor]
    fn new(dependency: Dependency) -> Self {
        if true {
            return Self { dependency };
        }
        Self { dependency }
    }
}

#[injectable]
struct Failing {
    value: usize,
}
impl Failing {
    #[constructor]
    fn new(dependency: Dependency) -> Result<Self, &'static str> {
        let _ = dependency;
        if true {
            Err("left")
        } else {
            return Err("right");
        }
    }
}

#[injectable]
struct ExpandedAlias {
    dependency: Dependency,
}
macro_rules! identity {
    ($value:expr) => {
        $value
    };
}
impl ExpandedAlias {
    #[constructor]
    fn new(dependency: Dependency) -> Self {
        // 宏已经展开，解析后的参数身份可证明，因此无需保留原先宏阶段的盲目拒绝。
        let alias = identity!(dependency);
        Self { dependency: alias }
    }
}

mod exact {
    use super::*;
    #[injectable]
    pub struct Service {
        pub dependency: Dependency,
    }
    impl Service {
        #[constructor]
        fn new(dependency: Dependency) -> Self {
            crate::exact::Service { dependency }
        }
    }
}

fn injected(_: &Injection<Dependency>) {}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = ServiceProvider::build(None).await.unwrap();
    let conditional = root.get_required_service::<Conditional>().await.unwrap();
    assert_eq!(conditional.value, 7);
    #[cfg(feature = "alternate")]
    {
        injected(&conditional.dependency);
        injected(&conditional.another);
    }
    injected(
        &root
            .get_required_service::<CapturedParameter>()
            .await
            .unwrap()
            .dependency,
    );
    injected(
        &root
            .get_required_service::<DefinedParameter>()
            .await
            .unwrap()
            .dependency,
    );
    let shadowed = root.get_required_service::<Shadowed>().await.unwrap();
    assert_eq!((shadowed.dependency, shadowed.other), (42, 43));
    root.get_required_service::<Unit>().await.unwrap();
    let renamed = root.get_required_service::<Renamed>().await.unwrap();
    injected(&renamed.first);
    injected(&renamed.second);
    injected(
        &root
            .get_required_service::<Branches>()
            .await
            .unwrap()
            .dependency,
    );
    injected(
        &root
            .get_required_service::<EarlyReturn>()
            .await
            .unwrap()
            .dependency,
    );
    injected(
        &root
            .get_required_service::<ExpandedAlias>()
            .await
            .unwrap()
            .dependency,
    );
    injected(
        &root
            .get_required_service::<exact::Service>()
            .await
            .unwrap()
            .dependency,
    );
    assert!(root.get_required_service::<Failing>().await.is_err());
    root.dispose_async().await.unwrap();
    println!("constructor source flow passed");
}

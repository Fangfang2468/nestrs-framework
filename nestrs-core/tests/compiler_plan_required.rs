//! 集成测试链接未启用 cfg(test) 的真实 core，分别验证普通 Cargo 和 Nestrs 工具链。
//! nestrs_compiler cfg 只标记 core 本体，不能用它推断当前集成测试的编译方式；
//! CLI 提供的编译期工具链身份用于选择严格对应的断言，两条路径都实际运行测试。

use nestrs_core::{BuildError, ServiceProvider, ServiceProviderOptions};

const HAS_TOOLCHAIN_PLAN: bool = option_env!("NESTRS_TOOLCHAIN_ID").is_some();

#[tokio::test]
async fn build_entries_enforce_the_plan_contract_for_the_current_compiler() {
    for result in [
        ServiceProvider::build().await,
        ServiceProvider::build_with_options(ServiceProviderOptions::default()).await,
    ] {
        if HAS_TOOLCHAIN_PLAN {
            let provider = result.expect("Nestrs 工具链编译后两个 build 入口都应装载有效计划");
            provider.dispose_async().await.unwrap();
            continue;
        }
        let Err(error) = result else {
            panic!("缺少 Nestrs 编译计划时不能构建空容器");
        };
        assert!(matches!(error, BuildError::CompilerPlanUnavailable));
        assert!(error.to_string().contains("cargo nestrs"));
    }
}

#[test]
fn startup_errors_distinguish_missing_plan_from_missing_runtime() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    let mut future = std::pin::pin!(ServiceProvider::build());
    let mut context = Context::from_waker(Waker::noop());
    let result = future.as_mut().poll(&mut context);
    if HAS_TOOLCHAIN_PLAN {
        assert!(matches!(
            result,
            Poll::Ready(Err(BuildError::RuntimeUnavailable))
        ));
    } else {
        assert!(matches!(
            result,
            Poll::Ready(Err(BuildError::CompilerPlanUnavailable))
        ));
    }
}

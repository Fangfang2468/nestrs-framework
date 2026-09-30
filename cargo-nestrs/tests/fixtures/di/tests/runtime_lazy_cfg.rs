//! 真实 rustc 条件编译、泛型单态化和延迟字段协议共同工作的黑盒回归。
//!
//! 禁用字段故意引用不存在的类型，证明它们在字段分析、槽位分配和静态图物化前
//! 已经被 rustc 移除。没有通过手写内部描述绕过用户实际使用的宏生成路径。

use nestrs::injectable;
use nestrs_core::{Injection, LazyInjection, ServiceProvider, get_required_service};
use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Report {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1)]
    id: usize,
}

trait ReportPort: Send + Sync {
    fn id(&self) -> usize;
}

impl ReportPort for Report {
    fn id(&self) -> usize {
        self.id
    }
}

type ConcreteAlias = Report;
type InterfaceAlias = dyn ReportPort;

#[injectable]
struct ConfiguredFields {
    #[cfg(any())]
    #[inject]
    #[lazy]
    disabled: MissingReport,
    #[cfg_attr(all(), cfg(any()))]
    #[inject]
    #[lazy(false)]
    disabled_invalid_marker: MissingDisabledReport,
    #[cfg_attr(all(), inject)]
    #[cfg_attr(all(), nestrs::lazy)]
    report: ConcreteAlias,
    #[cfg_attr(all(), cfg_attr(all(), nestrs::inject("absent"), lazy))]
    absent: Option<InterfaceAlias>,
    #[cfg_attr(any(), inject, lazy)]
    default_value: usize,
}

#[injectable]
struct ConfiguredTuple(
    #[cfg(any())]
    #[inject]
    #[lazy]
    MissingTupleReport,
    #[value(7)] usize,
    #[cfg_attr(all(), inject, lazy)] ConcreteAlias,
    #[cfg_attr(all(), value(19))] usize,
);

/// 原始 T 可以是 trait object。宏必须包装 T 本身，不能把 LazyInjection<T>
/// 当作依赖请求或把 Sized 约束错误附加给 T。
#[injectable]
struct GenericDeferred<T: ?Sized> {
    #[cfg(any())]
    #[inject]
    #[lazy]
    disabled: MissingGenericReport<T>,
    #[cfg_attr(all(), nestrs::inject)]
    #[cfg_attr(all(), cfg_attr(all(), lazy))]
    target: T,
}

type ClosedInterfaceAlias = GenericDeferred<InterfaceAlias>;
type ClosedConcreteAlias = GenericDeferred<ConcreteAlias>;

#[injectable]
struct ImmediateWhenLazyIsDisabled {
    #[inject]
    #[cfg_attr(any(), lazy)]
    target: ConcreteAlias,
}

fn expects_lazy<T: ?Sized>(_: &LazyInjection<T>) {}
fn expects_immediate<T: ?Sized>(_: &Injection<T>) {}

#[tokio::test(flavor = "current_thread")]
async fn conditional_fields_and_unsized_aliases_preserve_lazy_input_semantics() {
    let provider = ServiceProvider::build().await.unwrap();
    let configured = get_required_service!(provider, ConfiguredFields)
        .await
        .unwrap();
    let tuple = get_required_service!(provider, ConfiguredTuple)
        .await
        .unwrap();
    let interface = get_required_service!(provider, ClosedInterfaceAlias)
        .await
        .unwrap();
    let concrete = get_required_service!(provider, ClosedConcreteAlias)
        .await
        .unwrap();

    // 同时校验生成后的 Rust 字段形态和运行期行为；仅能编译并不证明真正延迟。
    expects_lazy::<ConcreteAlias>(&configured.report);
    expects_lazy::<ConcreteAlias>(&tuple.1);
    expects_lazy::<InterfaceAlias>(&interface.target);
    expects_lazy::<ConcreteAlias>(&concrete.target);
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    assert!(configured.absent.is_none());
    assert_eq!(configured.default_value, 0);
    assert_eq!(tuple.0, 7);
    assert_eq!(tuple.2, 19);

    let report = configured.report.get().await.unwrap();
    assert_eq!(report.id, 1);
    assert_eq!(interface.target.get().await.unwrap().id(), report.id);
    assert!(std::ptr::eq(tuple.1.get().await.unwrap(), report));
    assert!(std::ptr::eq(concrete.target.get().await.unwrap(), report));
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);

    let immediate = get_required_service!(provider, ImmediateWhenLazyIsDisabled)
        .await
        .unwrap();
    expects_immediate::<ConcreteAlias>(&immediate.target);
    assert!(std::ptr::eq(&*immediate.target, report));
    provider.dispose_async().await.unwrap();
}

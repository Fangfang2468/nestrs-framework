#![allow(dead_code)]

use nestrs::{constructor, factory, injectable};
use nestrs_core::ServiceProvider;

mod external;
use external::External;

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

trait Port: Send + Sync {
    fn label(&self) -> &'static str;
}

#[injectable]
struct Repository {
    #[value(GENERATED_COUNT)]
    count: usize,
}

impl Port for Repository {
    fn label(&self) -> &'static str {
        env!("NESTRS_FIXTURE_LABEL")
    }
}

#[injectable]
struct Service {
    #[inject]
    port: dyn Port,
    #[inject]
    external: External,
}

impl Service {
    fn describe(&self) -> &'static str {
        self.port.label()
    }
}

// 标准 rust-analyzer 必须应用编译器选定的 constructor 模式，不能把 raw dyn 字段
// 留作未定长业务字段，也不能把未选中的自动 Default 候选交给类型检查。
#[injectable]
struct ConstructorService {
    constructor_port: dyn Port,
    later: External,
}

trait LocalConstructorNames {
    fn __nestrs_ide_constructor_1_activate() -> usize {
        47
    }

    fn __nestrs_ide_constructor_1_dependencies() -> usize {
        53
    }
}

impl LocalConstructorNames for ConstructorService {}

use nestrs_ide_contracts::ExternalConstructorNames;
impl ExternalConstructorNames for ConstructorService {}

impl ConstructorService {
    // 这些是合法业务成员。生成 adapter 必须与它们隔离，编辑器也应解析到业务定义。
    const __NESTRS_CONSTRUCTOR: usize = 31;

    fn __nestrs_constructor_activate() -> usize {
        29
    }

    fn __nestrs_constructor_dependencies() -> usize {
        37
    }

    // 编辑器连接名也必须依据本编译单元分配，不能换成长一点的固定保留名。
    fn __nestrs_ide_constructor_0_activate() -> usize {
        41
    }

    fn __nestrs_ide_constructor_0_dependencies() -> usize {
        43
    }

    #[constructor]
    fn new(input: dyn Port, #[lazy] delayed: External) -> Self {
        let actual_port = input;
        Self {
            constructor_port: actual_port,
            later: delayed,
        }
    }

    fn describe(&self) -> &'static str {
        self.constructor_port.label()
    }

    async fn delayed_number(&self) -> usize {
        self.later.get().await.unwrap().number
    }
}

struct MissingConstructorDependency;

#[allow(unused_parens)]
#[injectable]
struct ParenthesizedConstructor<T: Send + Sync + 'static> {
    present: (((::std::option::Option<(T)>))),
    absent: ((Option<MissingConstructorDependency>)),
    delayed_present: ((::core::option::Option<((T))>)),
    delayed_absent: (((Option<MissingConstructorDependency>))),
}

#[allow(unused_parens)]
impl<T: Send + Sync + 'static> ParenthesizedConstructor<T> {
    #[constructor]
    fn new(
        present: ((Option<T>)),
        absent: (::std::option::Option<MissingConstructorDependency>),
        #[lazy] delayed_present: (::core::option::Option<T>),
        #[lazy] delayed_absent: ((Option<MissingConstructorDependency>)),
    ) -> Self {
        Self {
            present,
            absent,
            delayed_present,
            delayed_absent,
        }
    }
}

struct Client(&'static str);

#[factory]
async fn create_client(port: dyn Port) -> Client {
    tokio::task::yield_now().await;
    Client(port.label())
}

macro_rules! generated_provider {
    () => {
        #[nestrs::injectable]
        struct MacroValue {
            #[nestrs::value(5)]
            number: usize,
        }
    };
}
generated_provider!();

#[cfg(feature = "alternate")]
fn feature_value() -> usize {
    2
}

#[cfg(not(feature = "alternate"))]
fn feature_value() -> usize {
    1
}

#[cfg(nestrs_fixture_generated)]
fn generated_value() -> usize {
    GENERATED_COUNT
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let service = provider.get_required_service::<Service>().await.unwrap();
    assert_eq!(service.describe(), "generated-by-build-script");
    assert_eq!(service.external.number, 23);
    assert_eq!(generated_value(), 17);
    assert!(feature_value() > 0);
    let _ = provider.get_required_service::<Client>().await.unwrap();
    let _ = provider.get_required_service::<MacroValue>().await.unwrap();
    let constructed = provider
        .get_required_service::<ConstructorService>()
        .await
        .unwrap();
    assert_eq!(constructed.describe(), "generated-by-build-script");
    assert_eq!(constructed.delayed_number().await, 23);
    assert_eq!(ConstructorService::__nestrs_constructor_activate(), 29);
    assert_eq!(ConstructorService::__nestrs_constructor_dependencies(), 37);
    assert_eq!(ConstructorService::__NESTRS_CONSTRUCTOR, 31);
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_0_activate(),
        41
    );
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_0_dependencies(),
        43
    );
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_1_activate(),
        47
    );
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_1_dependencies(),
        53
    );
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_2_activate(),
        59
    );
    assert_eq!(
        ConstructorService::__nestrs_ide_constructor_2_dependencies(),
        61
    );
    let optional = provider
        .get_required_service::<ParenthesizedConstructor<External>>()
        .await
        .unwrap();
    assert_eq!(optional.present.as_ref().unwrap().number, 23);
    assert!(optional.absent.is_none());
    assert_eq!(
        optional
            .delayed_present
            .as_ref()
            .unwrap()
            .get()
            .await
            .unwrap()
            .number,
        23
    );
    assert!(optional.delayed_absent.is_none());
    provider.dispose_async().await.unwrap();
}

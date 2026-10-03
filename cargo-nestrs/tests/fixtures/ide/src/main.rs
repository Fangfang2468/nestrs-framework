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

impl ConstructorService {
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
    let provider = ServiceProvider::build().await.unwrap();
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
    provider.dispose_async().await.unwrap();
}
